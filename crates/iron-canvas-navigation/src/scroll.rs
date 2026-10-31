//! Pixel scroll state and checked view changes for one host view.
//!
//! The controller owns the accepted scroll position. It reads rounded axis
//! runs, normalizes the position, and mirrors changed engine indices through
//! a short mutable model borrow.

use crate::{ControllerCommand, NavigationController, NavigationScope, QueueError};
use iron_canvas::PageDirection;
use iron_canvas_core::{
    AxisExtents, AxisScroll, CellCoord, ScrollAxisWindow, ScrollPosition, move_position,
    normalize_position,
};
use iron_canvas_ironcalc::{WorksheetAxes, WorksheetAxis};
use ironcalc_base::UserModel;
use std::collections::HashMap;

/// Pane and freeze inputs for one scroll attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollViewport {
    pub pane_width: i32,
    pub pane_height: i32,
    pub frozen_rows: i32,
    pub frozen_columns: i32,
}

/// One scroll request in the shared command queue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollCommand {
    /// Logical pixels. Each axis keeps a signed fractional remainder.
    Pixels { dx: f64, dy: f64 },
    /// One scrolling pane extent on the vertical axis.
    Page(PageDirection),
    /// Set the origin and clear both offsets, even at unchanged indices.
    SetOrigin(CellCoord),
    /// Move just enough to expose a range.
    RevealRange { first: CellCoord, last: CellCoord },
}

/// Result of one accepted or unchanged scroll request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollUpdate {
    pub position: ScrollPosition,
    /// Checked generation for accepted view changes.
    pub generation: u64,
    pub mirrored: bool,
    pub changed: bool,
}

/// Failure to apply or import a scroll position.
#[derive(Debug, Clone, PartialEq)]
pub enum ScrollError {
    /// The view has no stored position. Import it first.
    UnknownView,
    /// The selected engine sheet does not match this view's sheet.
    SheetMismatch,
    /// The view generation is exhausted. It never wraps.
    GenerationExhausted,
    InvalidDelta,
    /// A zero-extent reveal target has no rectangle.
    NotVisible,
    Engine(String),
}

impl std::fmt::Display for ScrollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownView => f.write_str("view scroll state is not imported"),
            Self::SheetMismatch => f.write_str("selected engine sheet does not match the view"),
            Self::GenerationExhausted => f.write_str("view generation is exhausted"),
            Self::InvalidDelta => f.write_str("scroll delta is not finite"),
            Self::NotVisible => f.write_str("reveal target has no visible extent"),
            Self::Engine(error) => write!(f, "engine scroll update failed: {error}"),
        }
    }
}

impl std::error::Error for ScrollError {}

impl From<QueueError> for ScrollError {
    fn from(_: QueueError) -> Self {
        Self::GenerationExhausted
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewState {
    position: ScrollPosition,
    fractional_x: f64,
    fractional_y: f64,
    generation: u64,
    viewport: ScrollViewport,
}

impl ViewState {
    fn new(position: ScrollPosition, viewport: ScrollViewport) -> Self {
        Self {
            position,
            fractional_x: 0.0,
            fractional_y: 0.0,
            generation: 0,
            viewport,
        }
    }
}

pub(crate) type ViewMap = HashMap<NavigationScope, ViewState>;

fn row_window(axes: &WorksheetAxes, frozen: i32) -> ScrollAxisWindow {
    ScrollAxisWindow::after_frozen(frozen, axes.rows.last_index())
}

fn column_window(axes: &WorksheetAxes, frozen: i32) -> ScrollAxisWindow {
    ScrollAxisWindow::after_frozen(frozen, axes.cols.last_index())
}

fn scrolling_extent(axis: &WorksheetAxis, window: ScrollAxisWindow) -> i64 {
    if window.is_empty() {
        return 0;
    }
    (axis.total_extent() - axis.extent_before(window.first)).max(0)
}

fn distance(axis: &WorksheetAxis, window: ScrollAxisWindow, first: i32, offset: i32) -> i64 {
    if window.is_empty() {
        return 0;
    }
    let first = first.clamp(window.first, window.last);
    (axis.extent_before(first) - axis.extent_before(window.first) + i64::from(offset)).max(0)
}

fn located(axis: &WorksheetAxis, window: ScrollAxisWindow, distance: i64) -> AxisScroll {
    match axis.locate(distance, window) {
        Some((first, offset)) => AxisScroll { first, offset },
        None => AxisScroll {
            first: window.first,
            offset: 0,
        },
    }
}

/// Split a fractional delta into a whole-pixel part and a signed remainder.
/// An extreme finite value clamps to the reachable distance and clears its
/// remainder so reverse movement responds at once.
fn split_delta(total: f64, limit: i64) -> (i32, f64) {
    let whole = total.trunc();
    let bound = limit.clamp(0, i64::from(i32::MAX)) as f64;
    let clamped = whole.clamp(-bound, bound);
    let remainder = if clamped == whole { total - whole } else { 0.0 };
    (clamped as i32, remainder)
}

impl NavigationController {
    /// Import the engine origin for a view, or restore its saved position.
    pub fn import_view(
        &mut self,
        model: &mut UserModel<'_>,
        scope: NavigationScope,
        axes: &WorksheetAxes,
        viewport: ScrollViewport,
    ) -> Result<ScrollUpdate, ScrollError> {
        if let Some(state) = self.views.get(&scope).copied() {
            let position = self.normalize(state.position, axes, viewport, true);
            let changed = position != state.position;
            return self.install_or_update(model, scope, position, viewport, changed);
        }
        let sheet_id = selected_sheet_id(model)?;
        if sheet_id != scope.sheet_id {
            return Err(ScrollError::SheetMismatch);
        }
        let view = model.get_selected_view();
        let position = normalize_position(
            ScrollPosition::aligned(CellCoord {
                row: view.top_row.max(1),
                col: view.left_column.max(1),
            }),
            &axes.rows,
            row_window(axes, viewport.frozen_rows),
            &axes.cols,
            column_window(axes, viewport.frozen_columns),
        );
        self.views.insert(scope, ViewState::new(position, viewport));
        let mirrored = self.mirror(model, scope, position)?;
        Ok(ScrollUpdate {
            position,
            generation: 0,
            mirrored,
            changed: false,
        })
    }

    /// Apply one scroll command through the shared entry point.
    pub fn apply_scroll(
        &mut self,
        model: &mut UserModel<'_>,
        scope: NavigationScope,
        command: ScrollCommand,
        axes: &WorksheetAxes,
        viewport: ScrollViewport,
    ) -> Result<ScrollUpdate, ScrollError> {
        let Some(mut state) = self.views.get(&scope).copied() else {
            return Err(ScrollError::UnknownView);
        };
        state.viewport = viewport;
        let mut position = state.position;
        let changed;
        match command {
            ScrollCommand::Pixels { dx, dy } => {
                if !dx.is_finite() || !dy.is_finite() {
                    return Err(ScrollError::InvalidDelta);
                }
                let row_window = row_window(axes, viewport.frozen_rows);
                let column_window = column_window(axes, viewport.frozen_columns);
                let row_limit = scrolling_extent(&axes.rows, row_window)
                    + i64::from(viewport.pane_height.max(0));
                let column_limit = scrolling_extent(&axes.cols, column_window)
                    + i64::from(viewport.pane_width.max(0));
                let (whole_y, remainder_y) = split_delta(state.fractional_y + dy, row_limit);
                let (whole_x, remainder_x) = split_delta(state.fractional_x + dx, column_limit);
                state.fractional_y = remainder_y;
                state.fractional_x = remainder_x;
                let next = move_position(
                    position,
                    whole_x,
                    whole_y,
                    &axes.rows,
                    row_window,
                    &axes.cols,
                    column_window,
                    viewport.pane_width,
                    viewport.pane_height,
                );
                // Input beyond a clamped boundary cannot accumulate.
                if at_boundary(
                    &axes.rows,
                    row_window,
                    next.first_cell.row,
                    next.offset_y,
                    viewport.pane_height,
                ) {
                    state.fractional_y = 0.0;
                }
                if at_boundary(
                    &axes.cols,
                    column_window,
                    next.first_cell.col,
                    next.offset_x,
                    viewport.pane_width,
                ) {
                    state.fractional_x = 0.0;
                }
                changed = next != position;
                position = next;
            }
            ScrollCommand::Page(direction) => {
                let delta = match direction {
                    PageDirection::Up => -viewport.pane_height,
                    PageDirection::Down => viewport.pane_height,
                };
                let row_window = row_window(axes, viewport.frozen_rows);
                let next = move_position(
                    position,
                    0,
                    delta,
                    &axes.rows,
                    row_window,
                    &axes.cols,
                    column_window(axes, viewport.frozen_columns),
                    viewport.pane_width,
                    viewport.pane_height,
                );
                changed = next != position;
                position = next;
            }
            ScrollCommand::SetOrigin(first_cell) => {
                let next = normalize_position(
                    ScrollPosition::aligned(first_cell),
                    &axes.rows,
                    row_window(axes, viewport.frozen_rows),
                    &axes.cols,
                    column_window(axes, viewport.frozen_columns),
                );
                changed = next != position;
                position = next;
            }
            ScrollCommand::RevealRange { first, last } => {
                let (first, last) = merged_range(model, scope, first, last)?;
                let row = reveal_axis(
                    position.first_cell.row,
                    position.offset_y,
                    first.row,
                    last.row,
                    &axes.rows,
                    row_window(axes, viewport.frozen_rows),
                    viewport.pane_height,
                )?;
                let column = reveal_axis(
                    position.first_cell.col,
                    position.offset_x,
                    first.col,
                    last.col,
                    &axes.cols,
                    column_window(axes, viewport.frozen_columns),
                    viewport.pane_width,
                )?;
                let next = ScrollPosition {
                    first_cell: CellCoord {
                        row: row.first,
                        col: column.first,
                    },
                    offset_x: column.offset,
                    offset_y: row.offset,
                };
                changed = next != position;
                position = next;
            }
        }
        state.position = position;
        if changed {
            let generation = state
                .generation
                .checked_add(1)
                .ok_or(ScrollError::GenerationExhausted)?;
            let mirrored = self.mirror(model, scope, position)?;
            state.generation = generation;
            self.views.insert(scope, state);
            Ok(ScrollUpdate {
                position,
                generation,
                mirrored,
                changed: true,
            })
        } else {
            self.views.insert(scope, state);
            Ok(ScrollUpdate {
                position,
                generation: state.generation,
                mirrored: false,
                changed: false,
            })
        }
    }

    /// Reconcile after an engine navigation command.
    ///
    /// A changed engine origin resets offsets on the changed axes. Otherwise
    /// the stored offsets stay and the engine selection target is revealed.
    pub fn navigation_applied(
        &mut self,
        model: &mut UserModel<'_>,
        scope: NavigationScope,
        axes: &WorksheetAxes,
        viewport: ScrollViewport,
    ) -> Result<ScrollUpdate, ScrollError> {
        let Some(mut state) = self.views.get(&scope).copied() else {
            return Err(ScrollError::UnknownView);
        };
        state.viewport = viewport;
        if selected_sheet_id(model)? != scope.sheet_id {
            return Err(ScrollError::SheetMismatch);
        }
        let view = model.get_selected_view();
        let mut position = state.position;
        if view.top_row != position.first_cell.row {
            position.first_cell.row = view.top_row.max(1);
            position.offset_y = 0;
        }
        if view.left_column != position.first_cell.col {
            position.first_cell.col = view.left_column.max(1);
            position.offset_x = 0;
        }
        position = self.normalize(position, axes, viewport, true);
        let target = CellCoord {
            row: view.range[0].min(view.range[2]),
            col: view.range[1].min(view.range[3]),
        };
        let end = CellCoord {
            row: view.range[0].max(view.range[2]),
            col: view.range[1].max(view.range[3]),
        };
        let row = reveal_axis(
            position.first_cell.row,
            position.offset_y,
            target.row,
            end.row,
            &axes.rows,
            row_window(axes, viewport.frozen_rows),
            viewport.pane_height,
        )
        .unwrap_or(AxisScroll {
            first: position.first_cell.row,
            offset: position.offset_y,
        });
        let column = reveal_axis(
            position.first_cell.col,
            position.offset_x,
            target.col,
            end.col,
            &axes.cols,
            column_window(axes, viewport.frozen_columns),
            viewport.pane_width,
        )
        .unwrap_or(AxisScroll {
            first: position.first_cell.col,
            offset: position.offset_x,
        });
        position = ScrollPosition {
            first_cell: CellCoord {
                row: row.first,
                col: column.first,
            },
            offset_x: column.offset,
            offset_y: row.offset,
        };
        let changed = position != state.position;
        state.position = position;
        if changed {
            let generation = state
                .generation
                .checked_add(1)
                .ok_or(ScrollError::GenerationExhausted)?;
            let mirrored = self.mirror(model, scope, position)?;
            state.generation = generation;
            self.views.insert(scope, state);
            Ok(ScrollUpdate {
                position,
                generation,
                mirrored,
                changed: true,
            })
        } else {
            self.views.insert(scope, state);
            Ok(ScrollUpdate {
                position,
                generation: state.generation,
                mirrored: false,
                changed: false,
            })
        }
    }

    /// Normalize and end-clamp a saved position after a viewport resize.
    pub fn resize_view(
        &mut self,
        model: &mut UserModel<'_>,
        scope: NavigationScope,
        axes: &WorksheetAxes,
        viewport: ScrollViewport,
    ) -> Result<ScrollUpdate, ScrollError> {
        let Some(mut state) = self.views.get(&scope).copied() else {
            return Err(ScrollError::UnknownView);
        };
        state.viewport = viewport;
        let position = self.normalize(state.position, axes, viewport, true);
        let changed = position != state.position;
        state.position = position;
        if changed {
            let generation = state
                .generation
                .checked_add(1)
                .ok_or(ScrollError::GenerationExhausted)?;
            let mirrored = self.mirror(model, scope, position)?;
            state.generation = generation;
            self.views.insert(scope, state);
            Ok(ScrollUpdate {
                position,
                generation,
                mirrored,
                changed: true,
            })
        } else {
            self.views.insert(scope, state);
            Ok(ScrollUpdate {
                position,
                generation: state.generation,
                mirrored: false,
                changed: false,
            })
        }
    }

    /// Current accepted position for a view.
    pub fn view_position(&self, scope: NavigationScope) -> Option<ScrollPosition> {
        self.views.get(&scope).map(|state| state.position)
    }

    /// Checked view generation for presentation checks.
    pub fn view_generation(&self, scope: NavigationScope) -> Option<u64> {
        self.views.get(&scope).map(|state| state.generation)
    }

    /// Drop all positions for a deleted sheet.
    pub fn remove_sheet(&mut self, workbook_id: u64, sheet_id: u32) {
        self.views
            .retain(|scope, _| scope.workbook_id != workbook_id || scope.sheet_id != sheet_id);
    }

    /// Clear pending input and all saved positions on workbook replacement.
    pub fn reset(&mut self) -> Result<(), QueueError> {
        let next = self
            .interaction
            .checked_add(1)
            .ok_or(QueueError::InteractionExhausted)?;
        self.pending.clear();
        self.views.clear();
        self.primary = None;
        self.interaction = next;
        Ok(())
    }

    /// Enqueue one scroll command in input order.
    pub fn enqueue_scroll(
        &mut self,
        command: ScrollCommand,
        scope: NavigationScope,
        repeat: bool,
    ) -> Result<(), QueueError> {
        self.enqueue_queued(ControllerCommand::Scroll(command), scope, repeat)
    }

    fn install_or_update(
        &mut self,
        model: &mut UserModel<'_>,
        scope: NavigationScope,
        position: ScrollPosition,
        viewport: ScrollViewport,
        changed: bool,
    ) -> Result<ScrollUpdate, ScrollError> {
        let Some(mut state) = self.views.get(&scope).copied() else {
            return Err(ScrollError::UnknownView);
        };
        state.viewport = viewport;
        let previous = state.position;
        state.position = position;
        let generation = if changed {
            let generation = state
                .generation
                .checked_add(1)
                .ok_or(ScrollError::GenerationExhausted)?;
            state.generation = generation;
            generation
        } else {
            state.generation
        };
        let mirrored = if changed {
            self.mirror(model, scope, position)?
        } else {
            false
        };
        self.views.insert(scope, state);
        Ok(ScrollUpdate {
            position,
            generation,
            mirrored,
            changed: position != previous,
        })
    }

    fn normalize(
        &self,
        position: ScrollPosition,
        axes: &WorksheetAxes,
        viewport: ScrollViewport,
        clamp_end: bool,
    ) -> ScrollPosition {
        let row_window = row_window(axes, viewport.frozen_rows);
        let column_window = column_window(axes, viewport.frozen_columns);
        let normalized =
            normalize_position(position, &axes.rows, row_window, &axes.cols, column_window);
        if clamp_end {
            clamp_end_position(
                normalized,
                axes,
                row_window,
                column_window,
                viewport.pane_width,
                viewport.pane_height,
            )
        } else {
            normalized
        }
    }

    fn mirror(
        &mut self,
        model: &mut UserModel<'_>,
        scope: NavigationScope,
        position: ScrollPosition,
    ) -> Result<bool, ScrollError> {
        if self.primary != Some(scope) {
            // A secondary view keeps its own position and never changes the
            // selected engine sheet.
            return Ok(false);
        }
        let sheet_id = selected_sheet_id(model)?;
        if sheet_id != scope.sheet_id {
            return Ok(false);
        }
        let view = model.get_selected_view();
        if view.top_row == position.first_cell.row && view.left_column == position.first_cell.col {
            return Ok(false);
        }
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_engine_write) {
            return Err(ScrollError::Engine("injected setter failure".into()));
        }
        model
            .set_top_left_visible_cell(position.first_cell.row, position.first_cell.col)
            .map_err(ScrollError::Engine)?;
        Ok(true)
    }
}

fn selected_sheet_id(model: &UserModel<'_>) -> Result<u32, ScrollError> {
    let sheet = model.get_selected_sheet();
    model
        .get_model()
        .workbook
        .worksheet(sheet)
        .map(|worksheet| worksheet.sheet_id)
        .map_err(ScrollError::Engine)
}

fn sheet_index(model: &UserModel<'_>, sheet_id: u32) -> Result<u32, ScrollError> {
    model
        .get_model()
        .workbook
        .worksheets
        .iter()
        .position(|worksheet| worksheet.sheet_id == sheet_id)
        .map(|index| index as u32)
        .ok_or(ScrollError::SheetMismatch)
}

fn at_boundary(
    axis: &WorksheetAxis,
    window: ScrollAxisWindow,
    first: i32,
    offset: i32,
    pane: i32,
) -> bool {
    if pane <= 0 || window.is_empty() {
        return true;
    }
    let total = scrolling_extent(axis, window);
    let max = (total - i64::from(pane)).max(0);
    let current = distance(axis, window, first, offset);
    current <= 0 || current >= max
}

fn clamp_end_position(
    position: ScrollPosition,
    axes: &WorksheetAxes,
    row_window: ScrollAxisWindow,
    column_window: ScrollAxisWindow,
    pane_width: i32,
    pane_height: i32,
) -> ScrollPosition {
    let row = clamp_axis(
        position.first_cell.row,
        position.offset_y,
        &axes.rows,
        row_window,
        pane_height,
    );
    let column = clamp_axis(
        position.first_cell.col,
        position.offset_x,
        &axes.cols,
        column_window,
        pane_width,
    );
    ScrollPosition {
        first_cell: CellCoord {
            row: row.first,
            col: column.first,
        },
        offset_x: column.offset,
        offset_y: row.offset,
    }
}

fn clamp_axis(
    first: i32,
    offset: i32,
    axis: &WorksheetAxis,
    window: ScrollAxisWindow,
    pane: i32,
) -> AxisScroll {
    if pane <= 0 || window.is_empty() {
        return AxisScroll { first, offset };
    }
    let total = scrolling_extent(axis, window);
    if total <= 0 {
        return AxisScroll {
            first: window.first,
            offset: 0,
        };
    }
    let max = (total - i64::from(pane)).max(0);
    let current = distance(axis, window, first, offset);
    if current <= max {
        return AxisScroll { first, offset };
    }
    located(axis, window, max)
}

/// Reveal a target interval on one axis. `pane <= 0` keeps the position.
fn reveal_axis(
    first: i32,
    offset: i32,
    target_first: i32,
    target_last: i32,
    axis: &WorksheetAxis,
    window: ScrollAxisWindow,
    pane: i32,
) -> Result<AxisScroll, ScrollError> {
    let current = AxisScroll {
        first: first.clamp(window.first, window.last),
        offset,
    };
    if pane <= 0 || window.is_empty() {
        return Ok(current);
    }
    // A frozen target needs no movement on this axis.
    if target_last < window.first {
        return Ok(current);
    }
    let target_first = target_first.max(window.first);
    let target_last = target_last.min(window.last);
    if target_first > target_last {
        return Ok(current);
    }
    let base = axis.extent_before(window.first);
    let start = distance(axis, window, current.first, current.offset);
    let target_start = axis.extent_before(target_first) - base;
    let target_end = axis.extent_before(target_last.saturating_add(1)) - base;
    let span = target_end - target_start;
    if span <= 0 {
        return Err(ScrollError::NotVisible);
    }
    let pane = i64::from(pane);
    let total = scrolling_extent(axis, window);
    let max = (total - pane).max(0);
    let fully_visible = target_start >= start && target_end <= start + pane;
    if fully_visible {
        return Ok(current);
    }
    let next = if span > pane {
        // An oversized target that already covers the pane keeps position.
        if target_start <= start && target_end >= start + pane {
            return Ok(current);
        }
        let leading = (start - target_start).unsigned_abs();
        let trailing = (start + pane - target_end).unsigned_abs();
        if leading <= trailing {
            target_start
        } else {
            target_end - pane
        }
    } else if target_start < start {
        target_start
    } else {
        target_end - pane
    };
    Ok(located(axis, window, next.clamp(0, max)))
}

/// Expand a single-cell target to its merged bounds.
fn merged_range(
    model: &UserModel<'_>,
    scope: NavigationScope,
    first: CellCoord,
    last: CellCoord,
) -> Result<(CellCoord, CellCoord), ScrollError> {
    if first != last {
        return Ok((first, last));
    }
    let sheet = sheet_index(model, scope.sheet_id)?;
    let merges = model.get_merged_cells(sheet).map_err(ScrollError::Engine)?;
    for merge in merges {
        if (merge.row..merge.row + merge.height).contains(&first.row)
            && (merge.column..merge.column + merge.width).contains(&first.col)
        {
            return Ok((
                CellCoord {
                    row: merge.row,
                    col: merge.column,
                },
                CellCoord {
                    row: merge.last_row(),
                    col: merge.last_column(),
                },
            ));
        }
    }
    Ok((first, last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ControllerCommand;
    use iron_canvas_ironcalc::{
        AxisSizes, SizeRun, WorksheetAxis, get_column_sizes, get_row_sizes,
    };
    use ironcalc_base::expressions::types::Area;

    fn model() -> UserModel<'static> {
        let mut model = UserModel::new_empty("scroll", "en", "UTC", "en").unwrap();
        model.set_rows_height(0, 1, 200, 20.0).unwrap();
        model.set_columns_width(0, 1, 200, 30.0).unwrap();
        model
    }

    fn axes(model: &UserModel<'_>) -> WorksheetAxes {
        WorksheetAxes::from_sizes(
            &get_row_sizes(model, 0).unwrap(),
            &get_column_sizes(model, 0).unwrap(),
        )
        .unwrap()
    }

    fn scope(model: &UserModel<'_>, view_id: u64) -> NavigationScope {
        NavigationScope {
            workbook_id: 7,
            view_id,
            sheet_id: model.get_model().workbook.worksheet(0).unwrap().sheet_id,
        }
    }

    fn pane(pane_width: i32, pane_height: i32) -> ScrollViewport {
        ScrollViewport {
            pane_width,
            pane_height,
            frozen_rows: 0,
            frozen_columns: 0,
        }
    }

    #[test]
    fn fractional_pixels_accumulate_and_keep_signed_remainders() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.set_primary_view(Some(scope));
        controller
            .import_view(&mut model, scope, &axes, viewport)
            .unwrap();
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::Pixels { dx: 0.0, dy: 7.6 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.offset_y, 7);
        assert!(!update.mirrored);
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::Pixels { dx: 0.0, dy: 7.6 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.offset_y, 15);
        // Crossing a cell boundary mirrors the new engine row.
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::Pixels { dx: 0.0, dy: 6.0 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.first_cell.row, 2);
        assert_eq!(update.position.offset_y, 1);
        assert!(update.mirrored);
        assert_eq!(model.get_selected_view().top_row, 2);
        // A negative remainder is retained across events.
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::Pixels { dx: 0.0, dy: -7.4 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.first_cell.row, 1);
        assert_eq!(update.position.offset_y, 14);
    }

    #[test]
    fn pixels_cross_long_hidden_intervals_without_per_row_work() {
        let mut model = model();
        let hidden = AxisSizes {
            default_size: 25.0,
            runs: vec![SizeRun {
                first: 2,
                last: 1_000_000,
                size: 0.0,
            }],
        };
        let rows = WorksheetAxis::from_sizes(&hidden, 1_048_576).unwrap();
        let columns = WorksheetAxis::from_sizes(
            &AxisSizes {
                default_size: 90.0,
                runs: Vec::new(),
            },
            16_384,
        )
        .unwrap();
        let axes = WorksheetAxes {
            rows,
            cols: columns,
        };
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .import_view(&mut model, scope, &axes, viewport)
            .unwrap();
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::SetOrigin(CellCoord {
                    row: 1_000_000,
                    col: 1,
                }),
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.first_cell.row, 1_000_001);
        assert_eq!(update.position.offset_y, 0);
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::Pixels { dx: 0.0, dy: 10.0 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.offset_y, 10);
        assert!(update.position.first_cell.row > 1_000_000);
    }

    #[test]
    fn exhausted_generation_rejects_the_change_before_the_engine_write() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .import_view(&mut model, scope, &axes, viewport)
            .unwrap();
        controller.views.get_mut(&scope).unwrap().generation = u64::MAX;
        let before = controller.view_position(scope).unwrap();
        assert_eq!(
            controller.apply_scroll(
                &mut model,
                scope,
                ScrollCommand::SetOrigin(CellCoord { row: 5, col: 5 }),
                &axes,
                viewport,
            ),
            Err(ScrollError::GenerationExhausted)
        );
        assert_eq!(controller.view_position(scope), Some(before));
        assert_eq!(
            (
                model.get_selected_view().top_row,
                model.get_selected_view().left_column
            ),
            (1, 1)
        );
    }

    #[test]
    fn engine_setter_failure_keeps_the_accepted_position() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.set_primary_view(Some(scope));
        controller
            .import_view(&mut model, scope, &axes, viewport)
            .unwrap();
        controller.fail_next_engine_write = true;
        let before = controller.view_position(scope).unwrap();
        let result = controller.apply_scroll(
            &mut model,
            scope,
            ScrollCommand::SetOrigin(CellCoord { row: 6, col: 6 }),
            &axes,
            viewport,
        );
        assert!(matches!(result, Err(ScrollError::Engine(_))));
        assert_eq!(controller.view_position(scope), Some(before));
        assert_eq!(model.get_selected_view().top_row, 1);
        assert_eq!(model.get_selected_view().left_column, 1);
    }

    #[test]
    fn views_keep_separate_positions_and_sheet_deletion_clears_them() {
        let mut model = model();
        let axes = axes(&model);
        let first = scope(&model, 0);
        let second = scope(&model, 1);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.set_primary_view(Some(first));
        controller
            .import_view(&mut model, first, &axes, viewport)
            .unwrap();
        controller
            .import_view(&mut model, second, &axes, viewport)
            .unwrap();
        controller
            .apply_scroll(
                &mut model,
                first,
                ScrollCommand::SetOrigin(CellCoord { row: 4, col: 3 }),
                &axes,
                viewport,
            )
            .unwrap();
        // The primary view mirrors the engine; the secondary view keeps its
        // own origin and never moves the selected engine sheet.
        assert_eq!(controller.view_position(first).unwrap().first_cell.row, 4);
        assert_eq!(
            controller.view_position(second).unwrap().first_cell,
            CellCoord { row: 1, col: 1 }
        );
        assert_eq!(model.get_selected_view().top_row, 4);
        controller
            .apply_scroll(
                &mut model,
                second,
                ScrollCommand::SetOrigin(CellCoord { row: 7, col: 2 }),
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(
            controller.view_position(second).unwrap().first_cell,
            CellCoord { row: 7, col: 2 }
        );
        assert_eq!(controller.view_position(first).unwrap().first_cell.row, 4);
        assert_eq!(model.get_selected_view().top_row, 4);
        // Re-import restores the saved position, not the engine origin.
        let restored = controller
            .import_view(&mut model, first, &axes, viewport)
            .unwrap();
        assert_eq!(restored.position.first_cell.row, 4);
        controller.remove_sheet(first.workbook_id, first.sheet_id);
        assert!(controller.view_position(first).is_none());
        assert!(controller.view_position(second).is_none());
        let reimported = controller
            .import_view(&mut model, first, &axes, viewport)
            .unwrap();
        assert_eq!(reimported.position.first_cell.row, 4);
        assert_eq!(reimported.generation, 0);
        controller.reset().unwrap();
        assert!(controller.view_position(first).is_none());
    }

    #[test]
    fn reveal_exposes_normal_and_oversized_targets() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(90, 100);
        let mut controller = NavigationController::default();
        controller
            .import_view(&mut model, scope, &axes, viewport)
            .unwrap();
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::RevealRange {
                    first: CellCoord { row: 10, col: 10 },
                    last: CellCoord { row: 10, col: 10 },
                },
                &axes,
                viewport,
            )
            .unwrap();
        // Rows: 10 * 20 = 200, pane 100 -> origin at row 6. Columns:
        // 10 * 30 = 300, pane 90 -> origin at column 8.
        assert_eq!(update.position.first_cell, CellCoord { row: 6, col: 8 });
        assert_eq!((update.position.offset_y, update.position.offset_x), (0, 0));
        let oversized = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::RevealRange {
                    first: CellCoord { row: 1, col: 1 },
                    last: CellCoord {
                        row: 1000,
                        col: 1000,
                    },
                },
                &axes,
                viewport,
            )
            .unwrap();
        assert!(!oversized.changed);
        assert_eq!(oversized.position, update.position);
    }

    #[test]
    fn reveal_rejects_zero_extent_targets_and_expands_merges() {
        let mut model = model();
        model.set_rows_hidden(0, 10, 10, true).unwrap();
        model
            .merge_cells(&Area {
                sheet: 0,
                row: 30,
                column: 30,
                width: 2,
                height: 2,
            })
            .unwrap();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(90, 100);
        let mut controller = NavigationController::default();
        controller
            .import_view(&mut model, scope, &axes, viewport)
            .unwrap();
        assert_eq!(
            controller.apply_scroll(
                &mut model,
                scope,
                ScrollCommand::RevealRange {
                    first: CellCoord { row: 10, col: 1 },
                    last: CellCoord { row: 10, col: 1 },
                },
                &axes,
                viewport,
            ),
            Err(ScrollError::NotVisible)
        );
        let update = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::RevealRange {
                    first: CellCoord { row: 31, col: 31 },
                    last: CellCoord { row: 31, col: 31 },
                },
                &axes,
                viewport,
            )
            .unwrap();
        assert!(update.position.first_cell.row <= 30);
        assert!(update.position.first_cell.col <= 30);
    }

    #[test]
    fn page_moves_one_pane_and_resize_clamps_the_saved_position() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(90, 60);
        let mut controller = NavigationController::default();
        controller
            .import_view(&mut model, scope, &axes, viewport)
            .unwrap();
        let down = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::Page(PageDirection::Down),
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(down.position.first_cell.row, 4);
        let up = controller
            .apply_scroll(
                &mut model,
                scope,
                ScrollCommand::Page(PageDirection::Up),
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(up.position.first_cell.row, 1);
        let resized = controller
            .resize_view(&mut model, scope, &axes, pane(90, 40))
            .unwrap();
        assert_eq!(resized.position.first_cell.row, 1);
        assert_eq!(resized.position.offset_y, 0);
    }

    #[test]
    fn scroll_commands_share_the_ordered_queue() {
        let mut controller = NavigationController::default();
        let scope = NavigationScope {
            workbook_id: 1,
            view_id: 3,
            sheet_id: 9,
        };
        controller
            .enqueue_scroll(ScrollCommand::Pixels { dx: 1.0, dy: 2.0 }, scope, false)
            .unwrap();
        controller
            .enqueue(
                iron_canvas::NavigationIntent::Step(iron_canvas::Direction::Right),
                scope,
                false,
            )
            .unwrap();
        assert_eq!(
            controller.pop_ready(scope),
            Some(ControllerCommand::Scroll(ScrollCommand::Pixels {
                dx: 1.0,
                dy: 2.0
            }))
        );
        assert!(matches!(
            controller.pop_ready(scope),
            Some(ControllerCommand::Navigation(_))
        ));
    }
}
