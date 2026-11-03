//! Pixel scroll state and checked view changes for one host view.
//!
//! The controller owns the accepted scroll position. It reads rounded axis
//! runs, normalizes the position, and mirrors changed engine indices through
//! a short mutable model borrow.

use crate::{
    GridCommand, GridError, LAST_COLUMN, LAST_ROW, NavigationController, Outcome, Pane,
    RevealOutcome, RevealTarget, SkipReason, ViewIdentity, apply_navigation,
};
use iron_canvas_core::scroll::scrolling_extent;
use iron_canvas_core::{
    AxisExtents, AxisScroll, CellCoord, RunAxis, ScrollAxisWindow, ScrollPosition, move_position,
    normalize_position,
};
use iron_canvas_ironcalc::WorksheetAxes;
use ironcalc_base::UserModel;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewState {
    position: ScrollPosition,
    fractional_x: f64,
    fractional_y: f64,
    generation: u64,
    /// Content revision of the newest axes accepted for this view.
    axes_revision: u64,
}

impl ViewState {
    fn new(position: ScrollPosition) -> Self {
        Self {
            position,
            fractional_x: 0.0,
            fractional_y: 0.0,
            axes_revision: 0,
            generation: 0,
        }
    }
}

pub(crate) type ViewMap = HashMap<ViewIdentity, ViewState>;

/// Reject cached axes with a different source identity or an older revision.
/// Unscoped axes stay available for tests and exports.
///
/// Return the newest revision accepted for the view. A newer revision comes
/// from a rebuilt axis cache after a content or dimension change. An older
/// revision is a stale cache and would move the view against old extents.
fn validate_axes(
    axes: &WorksheetAxes,
    identity: ViewIdentity,
    accepted_revision: u64,
) -> Result<u64, GridError> {
    let Some(axes_scope) = axes.scope else {
        return Ok(accepted_revision);
    };
    if axes_scope.workbook_id != identity.workbook_id
        || axes_scope.sheet_id != identity.sheet_id
        || axes_scope.revision < accepted_revision
    {
        return Err(GridError::AxesMismatch);
    }
    Ok(axes_scope.revision)
}

fn row_window(axes: &WorksheetAxes, frozen: i32) -> ScrollAxisWindow {
    ScrollAxisWindow::after_frozen(frozen, axes.rows.last_index())
}

fn column_window(axes: &WorksheetAxes, frozen: i32) -> ScrollAxisWindow {
    ScrollAxisWindow::after_frozen(frozen, axes.cols.last_index())
}

fn distance(axis: &RunAxis, window: ScrollAxisWindow, first: i32, offset: i32) -> i64 {
    if window.is_empty() {
        return 0;
    }
    let first = first.clamp(window.first, window.last);
    (axis.extent_before(first) - axis.extent_before(window.first) + i64::from(offset)).max(0)
}

fn located(axis: &RunAxis, window: ScrollAxisWindow, distance: i64) -> AxisScroll {
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
    /// Import a view, or restore its saved position.
    pub fn attach(
        &mut self,
        model: &mut UserModel<'_>,
        identity: ViewIdentity,
        axes: &WorksheetAxes,
        pane: Pane,
        origin: Option<CellCoord>,
    ) -> Result<Outcome, GridError> {
        let existing = self.views.get(&identity).copied();
        let axes_revision = validate_axes(
            axes,
            identity,
            existing.map_or(0, |state| state.axes_revision),
        )?;
        if let Some(state) = existing {
            let position = self.normalize(state.position, axes, pane, true);
            let changed = position != state.position;
            let mut state = ViewState {
                axes_revision,
                ..state
            };
            state.position = position;
            if changed {
                state.generation = state
                    .generation
                    .checked_add(1)
                    .ok_or(GridError::GenerationExhausted)?;
            }
            // Restore the engine indices when this view is primary, even when
            // the saved position did not change.
            let mirrored = self.mirror(model, identity, position)?;
            self.views.insert(identity, state);
            return Ok(Outcome {
                position,
                generation: state.generation,
                mirrored,
                changed,
                reveal: RevealOutcome::NotRequested,
            });
        }
        if selected_sheet_id(model)? != identity.sheet_id {
            return Err(GridError::IdentityMismatch);
        }
        let engine_position = origin.unwrap_or_else(|| {
            let view = model.get_selected_view();
            CellCoord {
                row: view.top_row.max(1),
                col: view.left_column.max(1),
            }
        });
        let aligned = ScrollPosition::aligned(engine_position);
        let position = normalize_position(
            aligned,
            &axes.rows,
            row_window(axes, pane.frozen_rows),
            &axes.cols,
            column_window(axes, pane.frozen_columns),
        );
        let state = ViewState {
            axes_revision,
            ..ViewState::new(position)
        };
        let mirrored = self.mirror(model, identity, position)?;
        let changed = position != aligned;
        self.views.insert(identity, state);
        Ok(Outcome {
            position,
            generation: 0,
            mirrored,
            changed,
            reveal: RevealOutcome::NotRequested,
        })
    }

    /// Validate the view and the selected sheet before an engine read or write.
    ///
    /// The engine holds one selected sheet. A command for another view must not
    /// change the visible selection, so validate first and return the state.
    fn validated_state(
        &self,
        model: &UserModel<'_>,
        identity: ViewIdentity,
        axes: &WorksheetAxes,
    ) -> Result<ViewState, GridError> {
        let Some(state) = self.views.get(&identity).copied() else {
            return Err(GridError::UnknownView(identity));
        };
        let axes_revision = validate_axes(axes, identity, state.axes_revision)?;
        if selected_sheet_id(model)? != identity.sheet_id {
            return Err(GridError::IdentityMismatch);
        }
        Ok(ViewState {
            axes_revision,
            ..state
        })
    }

    fn scroll_pixels(
        &mut self,
        model: &mut UserModel<'_>,
        identity: ViewIdentity,
        dx: f64,
        dy: f64,
        axes: &WorksheetAxes,
        pane: Pane,
    ) -> Result<Outcome, GridError> {
        let Some(mut state) = self.views.get(&identity).copied() else {
            return Err(GridError::UnknownView(identity));
        };
        state.axes_revision = validate_axes(axes, identity, state.axes_revision)?;
        if !dx.is_finite() || !dy.is_finite() {
            return Err(GridError::InvalidDelta);
        }
        let mut position = state.position;
        let row_window = row_window(axes, pane.frozen_rows);
        let column_window = column_window(axes, pane.frozen_columns);
        let row_limit = scrolling_extent(&axes.rows, row_window) + i64::from(pane.height.max(0));
        let column_limit =
            scrolling_extent(&axes.cols, column_window) + i64::from(pane.width.max(0));
        let total_y = state.fractional_y + dy;
        let total_x = state.fractional_x + dx;
        let (whole_y, remainder_y) = split_delta(total_y, row_limit);
        let (whole_x, remainder_x) = split_delta(total_x, column_limit);
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
            pane.width,
            pane.height,
        );
        // Clear only input that pushes outward from a boundary. A half-pixel
        // event that moves away from the boundary must stay for the next event.
        if clears_fraction(
            &axes.rows,
            row_window,
            next.first_cell.row,
            next.offset_y,
            pane.height,
            total_y,
        ) {
            state.fractional_y = 0.0;
        }
        if clears_fraction(
            &axes.cols,
            column_window,
            next.first_cell.col,
            next.offset_x,
            pane.width,
            total_x,
        ) {
            state.fractional_x = 0.0;
        }
        let changed = next != position;
        position = next;
        state.position = position;
        let generation = if changed {
            state
                .generation
                .checked_add(1)
                .ok_or(GridError::GenerationExhausted)?
        } else {
            state.generation
        };
        state.generation = generation;
        let mirrored = if changed {
            self.mirror(model, identity, position)?
        } else {
            false
        };
        self.views.insert(identity, state);
        Ok(Outcome {
            position,
            generation,
            mirrored,
            changed,
            reveal: RevealOutcome::NotRequested,
        })
    }

    /// Apply one grid input in order. Reveal inline for key and explicit input.
    pub fn command(
        &mut self,
        model: &mut UserModel<'_>,
        identity: ViewIdentity,
        command: GridCommand,
        axes: &WorksheetAxes,
        pane: Pane,
    ) -> Result<Outcome, GridError> {
        match command {
            GridCommand::Scroll { dx, dy } => {
                self.scroll_pixels(model, identity, dx, dy, axes, pane)
            }
            GridCommand::Navigate(intent) => {
                // Validate before the engine write. The engine can move a
                // selection even when the caller supplied a stale view.
                self.validated_state(model, identity, axes)?;
                let result = match apply_navigation(model, intent, pane.height) {
                    Ok(result) => result,
                    Err(error) => {
                        // The engine can move before it reports an error. Follow
                        // the view it now shows, then report the failure.
                        let _ = self.sync(model, identity, axes, pane);
                        return Err(GridError::Navigation(error));
                    }
                };
                let (state, mut position) = self.reconcile(model, identity, axes, pane)?;
                let reveal =
                    self.reveal_target(model, identity, &mut position, &result.target, axes, pane)?;
                self.commit(model, identity, state, position, reveal)
            }
            GridCommand::Reveal(target) => {
                validate_reveal_target(&target)?;
                let state = self.validated_state(model, identity, axes)?;
                let mut position = state.position;
                let reveal =
                    self.reveal_target(model, identity, &mut position, &target, axes, pane)?;
                self.commit(model, identity, state, position, reveal)
            }
        }
    }

    /// Reconcile an engine origin that moved outside the command path.
    pub fn sync(
        &mut self,
        model: &UserModel<'_>,
        identity: ViewIdentity,
        axes: &WorksheetAxes,
        pane: Pane,
    ) -> Result<Outcome, GridError> {
        let (state, position) = self.reconcile(model, identity, axes, pane)?;
        let changed = position != state.position;
        let mut state = state;
        state.position = position;
        if changed {
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or(GridError::GenerationExhausted)?;
        }
        self.views.insert(identity, state);
        Ok(Outcome {
            position,
            generation: state.generation,
            mirrored: false,
            changed,
            reveal: RevealOutcome::NotRequested,
        })
    }

    /// Normalize and end-clamp before the next frame.
    pub fn resize(
        &mut self,
        model: &mut UserModel<'_>,
        identity: ViewIdentity,
        axes: &WorksheetAxes,
        pane: Pane,
    ) -> Result<Outcome, GridError> {
        let Some(state) = self.views.get(&identity).copied() else {
            return Err(GridError::UnknownView(identity));
        };
        let axes_revision = validate_axes(axes, identity, state.axes_revision)?;
        let state = ViewState {
            axes_revision,
            ..state
        };
        let position = self.normalize(state.position, axes, pane, true);
        self.commit(
            model,
            identity,
            state,
            position,
            RevealOutcome::NotRequested,
        )
    }

    fn reconcile(
        &self,
        model: &UserModel<'_>,
        identity: ViewIdentity,
        axes: &WorksheetAxes,
        pane: Pane,
    ) -> Result<(ViewState, ScrollPosition), GridError> {
        let state = self.validated_state(model, identity, axes)?;
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
        position = self.normalize(position, axes, pane, true);
        Ok((state, position))
    }

    fn commit(
        &mut self,
        model: &mut UserModel<'_>,
        identity: ViewIdentity,
        mut state: ViewState,
        position: ScrollPosition,
        reveal: RevealOutcome,
    ) -> Result<Outcome, GridError> {
        let changed = position != state.position;
        state.position = position;
        if changed {
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or(GridError::GenerationExhausted)?;
        }
        let generation = state.generation;
        let mirrored = if changed {
            self.mirror(model, identity, position)?
        } else {
            false
        };
        self.views.insert(identity, state);
        Ok(Outcome {
            position,
            generation,
            mirrored,
            changed,
            reveal,
        })
    }

    fn reveal_target(
        &self,
        model: &UserModel<'_>,
        identity: ViewIdentity,
        position: &mut ScrollPosition,
        target: &RevealTarget,
        axes: &WorksheetAxes,
        pane: Pane,
    ) -> Result<RevealOutcome, GridError> {
        if let Some(reason) = skip_reason(target, pane) {
            return Ok(RevealOutcome::Skipped(reason));
        }
        let (first, last) = resolve_reveal(model, identity, target)?;
        let row = reveal_axis(
            position.first_cell.row,
            position.offset_y,
            first.row,
            last.row,
            &axes.rows,
            row_window(axes, pane.frozen_rows),
            pane.height,
        )?;
        let column = reveal_axis(
            position.first_cell.col,
            position.offset_x,
            first.col,
            last.col,
            &axes.cols,
            column_window(axes, pane.frozen_columns),
            pane.width,
        )?;
        let next = ScrollPosition {
            first_cell: CellCoord {
                row: row.first,
                col: column.first,
            },
            offset_x: column.offset,
            offset_y: row.offset,
        };
        let moved = next != *position;
        *position = next;
        Ok(if moved {
            RevealOutcome::Moved
        } else {
            RevealOutcome::AlreadyVisible
        })
    }
}

/// Reject a target outside the sheet or with an inconsistent focus.
///
/// A clamped reveal would report `AlreadyVisible` for a cell that does not
/// exist. Return `NotVisible` instead so the host sees the bad target.
fn validate_reveal_target(target: &RevealTarget) -> Result<(), GridError> {
    if !crate::in_bounds(target.first)
        || !crate::in_bounds(target.last)
        || target.first.row > target.last.row
        || target.first.col > target.last.col
    {
        return Err(GridError::NotVisible);
    }
    let focus_ok = match target.focus {
        None => true,
        Some(focus) => {
            crate::in_bounds(focus)
                && focus.row >= target.first.row
                && focus.row <= target.last.row
                && focus.col >= target.first.col
                && focus.col <= target.last.col
        }
    };
    if !focus_ok {
        return Err(GridError::NotVisible);
    }
    Ok(())
}

fn skip_reason(target: &RevealTarget, pane: Pane) -> Option<SkipReason> {
    if pane.width <= 0 || pane.height <= 0 {
        return Some(SkipReason::EmptyPane);
    }
    if target.first.col == 1 && target.last.col == LAST_COLUMN {
        return Some(SkipReason::FullRow);
    }
    if target.first.row == 1 && target.last.row == LAST_ROW {
        return Some(SkipReason::FullColumn);
    }
    None
}

fn resolve_reveal(
    model: &UserModel<'_>,
    identity: ViewIdentity,
    target: &RevealTarget,
) -> Result<(CellCoord, CellCoord), GridError> {
    if let Some(focus) = target.focus {
        return merged_range(model, identity, focus, focus);
    }
    if target.first == target.last {
        merged_range(model, identity, target.first, target.last)
    } else {
        Ok((target.first, target.last))
    }
}

impl NavigationController {
    /// Current accepted position for a view.
    pub fn view_position(&self, identity: ViewIdentity) -> Option<ScrollPosition> {
        self.views.get(&identity).map(|state| state.position)
    }

    /// Drop all positions for a deleted sheet.
    pub fn remove_sheet(&mut self, workbook_id: u64, sheet_id: u32) {
        self.views
            .retain(|scope, _| scope.workbook_id != workbook_id || scope.sheet_id != sheet_id);
    }

    /// Clear all saved positions on workbook replacement.
    pub fn reset(&mut self) {
        self.views.clear();
        self.primary = None;
    }

    fn normalize(
        &self,
        position: ScrollPosition,
        axes: &WorksheetAxes,
        pane: Pane,
        clamp_end: bool,
    ) -> ScrollPosition {
        let row_window = row_window(axes, pane.frozen_rows);
        let column_window = column_window(axes, pane.frozen_columns);
        let normalized =
            normalize_position(position, &axes.rows, row_window, &axes.cols, column_window);
        if clamp_end {
            clamp_end_position(
                normalized,
                axes,
                row_window,
                column_window,
                pane.width,
                pane.height,
            )
        } else {
            normalized
        }
    }

    fn mirror(
        &mut self,
        model: &mut UserModel<'_>,
        identity: ViewIdentity,
        position: ScrollPosition,
    ) -> Result<bool, GridError> {
        if self.primary != Some(identity) {
            // A secondary view keeps its own position and never changes the
            // selected engine sheet.
            return Ok(false);
        }
        let sheet_id = selected_sheet_id(model)?;
        if sheet_id != identity.sheet_id {
            return Ok(false);
        }
        let view = model.get_selected_view();
        if view.top_row == position.first_cell.row && view.left_column == position.first_cell.col {
            return Ok(false);
        }
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_engine_write) {
            return Err(GridError::Engine("injected setter failure".into()));
        }
        model
            .set_top_left_visible_cell(position.first_cell.row, position.first_cell.col)
            .map_err(|error| GridError::Engine(error.to_string()))?;
        Ok(true)
    }
}

fn selected_sheet_id(model: &UserModel<'_>) -> Result<u32, GridError> {
    let sheet = model.get_selected_sheet();
    model
        .get_model()
        .workbook
        .worksheet(sheet)
        .map(|worksheet| worksheet.sheet_id)
        .map_err(|error| GridError::Engine(error.to_string()))
}

fn sheet_index(model: &UserModel<'_>, sheet_id: u32) -> Result<u32, GridError> {
    model
        .get_model()
        .workbook
        .worksheets
        .iter()
        .position(|worksheet| worksheet.sheet_id == sheet_id)
        .map(|index| index as u32)
        .ok_or(GridError::IdentityMismatch)
}

/// Expand a single-cell target to its merged bounds.
fn merged_range(
    model: &UserModel<'_>,
    identity: ViewIdentity,
    first: CellCoord,
    last: CellCoord,
) -> Result<(CellCoord, CellCoord), GridError> {
    if first != last {
        return Ok((first, last));
    }
    let sheet = sheet_index(model, identity.sheet_id)?;
    let merges = model
        .get_merged_cells(sheet)
        .map_err(|error| GridError::Engine(error.to_string()))?;
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

/// True when the axis cannot move in the input direction.
///
/// The fraction is discarded only when the input pushes past the start or end.
/// A zero delta keeps an existing remainder. A pane that cannot scroll has no
/// reachable boundary, so its fraction is always discarded.
fn clears_fraction(
    axis: &RunAxis,
    window: ScrollAxisWindow,
    first: i32,
    offset: i32,
    pane: i32,
    delta: f64,
) -> bool {
    if pane <= 0 || window.is_empty() {
        return true;
    }
    let total = scrolling_extent(axis, window);
    let max = (total - i64::from(pane)).max(0);
    let current = distance(axis, window, first, offset);
    (current <= 0 && delta < 0.0) || (current >= max && delta > 0.0)
}

fn clamp_end_position(
    position: ScrollPosition,
    axes: &WorksheetAxes,
    row_window: ScrollAxisWindow,
    column_window: ScrollAxisWindow,
    width: i32,
    height: i32,
) -> ScrollPosition {
    let row = clamp_axis(
        position.first_cell.row,
        position.offset_y,
        &axes.rows,
        row_window,
        height,
    );
    let column = clamp_axis(
        position.first_cell.col,
        position.offset_x,
        &axes.cols,
        column_window,
        width,
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
    axis: &RunAxis,
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
    axis: &RunAxis,
    window: ScrollAxisWindow,
    pane: i32,
) -> Result<AxisScroll, GridError> {
    // An empty window has no scrolling cell. Every index is frozen. Represent
    // the axis with the last legal index and zero offset, as normalize_axis does.
    // Do not clamp here: clamp panics when the window bounds are inverted.
    if window.is_empty() {
        return Ok(AxisScroll {
            first: window.last,
            offset: 0,
        });
    }
    let current = AxisScroll {
        first: first.clamp(window.first, window.last),
        offset,
    };
    if pane <= 0 {
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
        return Err(GridError::NotVisible);
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

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::navigation::{Direction, NavigationIntent};
    use iron_canvas_ironcalc::{AxesScope, AxisSizes, SizeRun, get_column_sizes, get_row_sizes};

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

    fn scope(model: &UserModel<'_>, view_id: u64) -> ViewIdentity {
        ViewIdentity {
            workbook_id: 7,
            view_id,
            sheet_id: model.get_model().workbook.worksheet(0).unwrap().sheet_id,
        }
    }

    fn pane(width: i32, height: i32) -> Pane {
        Pane {
            width,
            height,
            frozen_rows: 0,
            frozen_columns: 0,
        }
    }

    /// Move one view to a cell with pixel input. Test rows are 20 pixels and
    /// test columns are 30 pixels.
    fn move_to(
        controller: &mut NavigationController,
        model: &mut UserModel<'_>,
        scope: ViewIdentity,
        axes: &WorksheetAxes,
        viewport: Pane,
        row: i32,
        col: i32,
    ) {
        let position = controller.view_position(scope).unwrap();
        let dx = (col - position.first_cell.col) * 30 - position.offset_x;
        let dy = (row - position.first_cell.row) * 20 - position.offset_y;
        if dx == 0 && dy == 0 {
            return;
        }
        controller
            .command(
                model,
                scope,
                GridCommand::Scroll {
                    dx: f64::from(dx),
                    dy: f64::from(dy),
                },
                axes,
                viewport,
            )
            .unwrap();
    }

    #[test]
    fn fractional_pixels_accumulate_and_keep_signed_remainders() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(scope);
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        let update = controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 7.6 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.offset_y, 7);
        assert!(!update.mirrored);
        let update = controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 7.6 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(update.position.offset_y, 15);
        // Crossing a cell boundary mirrors the new engine row.
        let update = controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 6.0 },
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
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: -7.4 },
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
        let rows = hidden.to_run_axis(1_048_576).unwrap();
        let columns = AxisSizes {
            default_size: 90.0,
            runs: Vec::new(),
        }
        .to_run_axis(16_384)
        .unwrap();
        let axes = WorksheetAxes {
            rows,
            cols: columns,
            scope: None,
        };
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        model.set_top_left_visible_cell(1_000_000, 1).unwrap();
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        assert_eq!(
            controller.view_position(scope).unwrap().first_cell.row,
            1_000_001
        );
        let update = controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 10.0 },
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
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        controller.views.get_mut(&scope).unwrap().generation = u64::MAX;
        let before = controller.view_position(scope).unwrap();
        assert_eq!(
            controller.command(
                &mut model,
                scope,
                GridCommand::Scroll {
                    dx: 120.0,
                    dy: 80.0
                },
                &axes,
                viewport,
            ),
            Err(GridError::GenerationExhausted)
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
    fn navigate_validates_the_view_and_sheet_before_the_engine_write() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        model.set_selected_cell(5, 5).unwrap();
        let before = model.get_selected_cell();

        // An unknown view must not move the selected sheet.
        let unknown = ViewIdentity {
            view_id: 99,
            ..scope
        };
        assert_eq!(
            controller.command(
                &mut model,
                unknown,
                GridCommand::Navigate(NavigationIntent::Step(Direction::Down)),
                &axes,
                viewport,
            ),
            Err(GridError::UnknownView(unknown))
        );
        assert_eq!(model.get_selected_cell(), before);

        // A view for another sheet must not move this sheet either.
        let other_sheet = ViewIdentity {
            sheet_id: scope.sheet_id + 1,
            view_id: 3,
            ..scope
        };
        controller.views.insert(
            other_sheet,
            ViewState::new(ScrollPosition::aligned(CellCoord { row: 1, col: 1 })),
        );
        assert_eq!(
            controller.command(
                &mut model,
                other_sheet,
                GridCommand::Navigate(NavigationIntent::Step(Direction::Down)),
                &axes,
                viewport,
            ),
            Err(GridError::IdentityMismatch)
        );
        assert_eq!(model.get_selected_cell(), before);
    }

    #[test]
    fn engine_setter_failure_keeps_the_accepted_position() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(scope);
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        controller.fail_next_engine_write = true;
        let before = controller.view_position(scope).unwrap();
        let result = controller.command(
            &mut model,
            scope,
            GridCommand::Scroll {
                dx: 150.0,
                dy: 100.0,
            },
            &axes,
            viewport,
        );
        assert!(matches!(result, Err(GridError::Engine(_))));
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
        controller.activate(first);
        controller
            .attach(&mut model, first, &axes, viewport, None)
            .unwrap();
        controller
            .attach(&mut model, second, &axes, viewport, None)
            .unwrap();
        move_to(&mut controller, &mut model, first, &axes, viewport, 4, 3);
        // The primary view mirrors the engine; the secondary view keeps its
        // own origin and never moves the selected engine sheet.
        assert_eq!(controller.view_position(first).unwrap().first_cell.row, 4);
        assert_eq!(
            controller.view_position(second).unwrap().first_cell,
            CellCoord { row: 1, col: 1 }
        );
        assert_eq!(model.get_selected_view().top_row, 4);
        move_to(&mut controller, &mut model, second, &axes, viewport, 7, 2);
        assert_eq!(
            controller.view_position(second).unwrap().first_cell,
            CellCoord { row: 7, col: 2 }
        );
        assert_eq!(controller.view_position(first).unwrap().first_cell.row, 4);
        assert_eq!(model.get_selected_view().top_row, 4);
        // Re-import restores the saved position, not the engine origin.
        let restored = controller
            .attach(&mut model, first, &axes, viewport, None)
            .unwrap();
        assert_eq!(restored.position.first_cell.row, 4);
        controller.remove_sheet(first.workbook_id, first.sheet_id);
        assert!(controller.view_position(first).is_none());
        assert!(controller.view_position(second).is_none());
        let reimported = controller
            .attach(&mut model, first, &axes, viewport, None)
            .unwrap();
        assert_eq!(reimported.position.first_cell.row, 4);
        assert_eq!(reimported.generation, 0);
        controller.reset();
        assert!(controller.view_position(first).is_none());
    }

    #[test]
    fn restoring_a_primary_view_restores_the_engine_origin() {
        let mut model = model();
        let axes = axes(&model);
        let first = scope(&model, 0);
        let second = scope(&model, 1);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();

        controller.activate(first);
        controller
            .attach(&mut model, first, &axes, viewport, None)
            .unwrap();
        move_to(&mut controller, &mut model, first, &axes, viewport, 10, 1);
        assert_eq!(model.get_selected_view().top_row, 10);

        controller
            .attach(&mut model, second, &axes, viewport, None)
            .unwrap();
        controller.activate(second);
        move_to(&mut controller, &mut model, second, &axes, viewport, 20, 1);
        assert_eq!(model.get_selected_view().top_row, 20);

        // The saved position stays equal, but the engine origin still belongs
        // to the other view. Restoring the primary view must move the engine.
        controller.activate(first);
        let restored = controller
            .attach(&mut model, first, &axes, viewport, None)
            .unwrap();
        assert_eq!(restored.position.first_cell.row, 10);
        assert!(!restored.changed);
        assert!(restored.mirrored);
        assert_eq!(model.get_selected_view().top_row, 10);
    }

    #[test]
    fn attach_reports_a_normalized_origin_change() {
        let mut model = model();
        model.set_rows_hidden(0, 2, 2, true).unwrap();
        model.set_top_left_visible_cell(2, 1).unwrap();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(scope);

        let update = controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();

        assert_eq!(update.position.first_cell.row, 3);
        assert!(update.mirrored);
        assert!(update.changed);
        assert_eq!(model.get_selected_view().top_row, 3);
    }

    #[test]
    fn scoped_axes_reject_another_workbook_or_sheet() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let wrong = axes.clone().with_scope(AxesScope {
            workbook_id: scope.workbook_id + 1,
            sheet_id: scope.sheet_id + 1,
            revision: 0,
        });
        let mut controller = NavigationController::default();
        controller.activate(scope);

        assert!(matches!(
            controller.attach(&mut model, scope, &wrong, viewport, None),
            Err(GridError::AxesMismatch)
        ));
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        assert!(matches!(
            controller.command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 1.0 },
                &wrong,
                viewport,
            ),
            Err(GridError::AxesMismatch)
        ));
    }

    #[test]
    fn stale_axes_are_rejected_after_a_newer_revision_is_accepted() {
        let mut model = model();
        let plain = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let older = plain.clone().with_scope(AxesScope {
            workbook_id: scope.workbook_id,
            sheet_id: scope.sheet_id,
            revision: 4,
        });
        let newer = plain.with_scope(AxesScope {
            workbook_id: scope.workbook_id,
            sheet_id: scope.sheet_id,
            revision: 5,
        });
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &older, viewport, None)
            .unwrap();
        // A dimension change rebuilds the axes at the new revision.
        controller
            .resize(&mut model, scope, &newer, viewport)
            .unwrap();
        // A cached old revision must not move the view against old extents.
        assert_eq!(
            controller.command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 5.0 },
                &older,
                viewport,
            ),
            Err(GridError::AxesMismatch)
        );
        assert!(matches!(
            controller.attach(&mut model, scope, &older, viewport, None),
            Err(GridError::AxesMismatch)
        ));
        assert!(
            controller
                .command(
                    &mut model,
                    scope,
                    GridCommand::Scroll { dx: 0.0, dy: 5.0 },
                    &newer,
                    viewport,
                )
                .is_ok()
        );
    }

    #[test]
    fn sync_handles_a_frozen_column_band_that_consumes_the_axis() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = Pane {
            width: 90,
            height: 100,
            frozen_rows: 0,
            frozen_columns: 16_384,
        };
        let mut controller = NavigationController::default();
        controller.activate(scope);
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        let update = controller.sync(&model, scope, &axes, viewport).unwrap();
        assert_eq!(update.position.first_cell.col, 16_384);
        assert_eq!(update.position.offset_x, 0);
    }

    #[test]
    fn fractional_pixels_accumulate_away_from_the_top_boundary() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(scope);
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();

        for _ in 0..2 {
            let update = controller
                .command(
                    &mut model,
                    scope,
                    GridCommand::Scroll { dx: 0.0, dy: 0.5 },
                    &axes,
                    viewport,
                )
                .unwrap();
            assert_eq!(update.position.first_cell.row, 1);
        }

        assert_eq!(
            controller.view_position(scope).unwrap().offset_y,
            1,
            "two half-pixel events at the top must move one pixel"
        );
    }

    #[test]
    fn fractional_pixels_clear_only_the_outward_direction() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(scope);
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();

        // A negative fraction at the top is outward and is discarded.
        controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: -0.5 },
                &axes,
                viewport,
            )
            .unwrap();
        let down = controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 1.0 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(down.position.offset_y, 1);

        // The same rule applies at the end of the axis. Use zero default
        // extents so row 196 is the last reachable cell.
        let rows = AxisSizes {
            default_size: 0.0,
            runs: vec![SizeRun {
                first: 1,
                last: 200,
                size: 20.0,
            }],
        };
        let columns = AxisSizes {
            default_size: 0.0,
            runs: vec![SizeRun {
                first: 1,
                last: 200,
                size: 30.0,
            }],
        };
        let axes = WorksheetAxes::from_sizes(&rows, &columns).unwrap();
        move_to(&mut controller, &mut model, scope, &axes, viewport, 196, 1);
        controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 0.5 },
                &axes,
                viewport,
            )
            .unwrap();
        let up = controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: -1.0 },
                &axes,
                viewport,
            )
            .unwrap();
        assert_eq!(up.position.first_cell.row, 195);
        assert_eq!(up.position.offset_y, 19);
    }

    #[test]
    fn resize_clamps_the_saved_position() {
        let mut model = model();
        // Bound the axes so the end clamp has a reachable edge.
        let rows = AxisSizes {
            default_size: 0.0,
            runs: vec![SizeRun {
                first: 1,
                last: 200,
                size: 20.0,
            }],
        };
        let columns = AxisSizes {
            default_size: 0.0,
            runs: vec![SizeRun {
                first: 1,
                last: 200,
                size: 30.0,
            }],
        };
        let axes = WorksheetAxes::from_sizes(&rows, &columns).unwrap();
        let scope = scope(&model, 0);
        let viewport = pane(90, 60);
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        move_to(&mut controller, &mut model, scope, &axes, viewport, 196, 1);
        let resized = controller
            .resize(&mut model, scope, &axes, pane(90, 200))
            .unwrap();
        assert_eq!(resized.position.first_cell.row, 191);
        assert_eq!(resized.position.offset_y, 0);
    }

    #[test]
    fn command_reveals_a_same_origin_target_inline() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let pane = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(scope);
        controller
            .attach(&mut model, scope, &axes, pane, None)
            .unwrap();
        let scrolled = controller
            .command(
                &mut model,
                scope,
                GridCommand::Scroll { dx: 0.0, dy: 5.0 },
                &axes,
                pane,
            )
            .unwrap();
        assert_eq!(scrolled.position.first_cell.row, 1);
        assert_eq!(scrolled.position.offset_y, 5);
        // SheetStart selects A1. The first cell does not change, but the
        // inline reveal must remove the offset.
        let update = controller
            .command(
                &mut model,
                scope,
                GridCommand::Navigate(NavigationIntent::SheetStart),
                &axes,
                pane,
            )
            .unwrap();
        assert_eq!(update.position.offset_y, 0);
        assert_eq!(update.reveal, RevealOutcome::Moved);
    }

    #[test]
    fn command_reveals_an_active_cell_target() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let pane = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &axes, pane, None)
            .unwrap();
        let update = controller
            .command(
                &mut model,
                scope,
                GridCommand::Reveal(RevealTarget {
                    first: CellCoord { row: 21, col: 1 },
                    last: CellCoord { row: 21, col: 1 },
                    focus: None,
                }),
                &axes,
                pane,
            )
            .unwrap();
        // Row 21 ends at 420. A 100-pixel pane puts the origin at row 17.
        assert_eq!(update.position.first_cell.row, 17);
        assert_eq!(update.reveal, RevealOutcome::Moved);
    }

    #[test]
    fn command_reveals_the_opposite_corner_target() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let pane = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &axes, pane, None)
            .unwrap();
        let update = controller
            .command(
                &mut model,
                scope,
                GridCommand::Reveal(RevealTarget {
                    first: CellCoord { row: 1, col: 1 },
                    last: CellCoord { row: 11, col: 1 },
                    focus: Some(CellCoord { row: 11, col: 1 }),
                }),
                &axes,
                pane,
            )
            .unwrap();
        // The focus at row 11 ends at 220. The pane starts at row 7.
        assert_eq!(update.position.first_cell.row, 7);
        assert_eq!(update.reveal, RevealOutcome::Moved);
    }

    #[test]
    fn reveal_rejects_targets_outside_the_sheet() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let viewport = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &axes, viewport, None)
            .unwrap();
        let before = controller.view_position(scope).unwrap();
        let target = |row, col| RevealTarget {
            first: CellCoord { row, col },
            last: CellCoord { row, col },
            focus: None,
        };
        for invalid in [
            target(0, 1),
            target(LAST_ROW + 1, 1),
            target(1, 0),
            target(1, LAST_COLUMN + 1),
            RevealTarget {
                first: CellCoord { row: 4, col: 1 },
                last: CellCoord { row: 3, col: 1 },
                focus: None,
            },
            RevealTarget {
                first: CellCoord { row: 1, col: 1 },
                last: CellCoord { row: 3, col: 1 },
                focus: Some(CellCoord { row: 4, col: 1 }),
            },
        ] {
            assert_eq!(
                controller.command(
                    &mut model,
                    scope,
                    GridCommand::Reveal(invalid),
                    &axes,
                    viewport,
                ),
                Err(GridError::NotVisible)
            );
            assert_eq!(controller.view_position(scope), Some(before));
        }
        assert!(
            controller
                .command(
                    &mut model,
                    scope,
                    GridCommand::Reveal(target(80, 1)),
                    &axes,
                    viewport,
                )
                .is_ok()
        );
    }

    #[test]
    fn command_skips_full_row_and_full_column_targets() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let pane = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &axes, pane, None)
            .unwrap();
        let full_row = controller
            .command(
                &mut model,
                scope,
                GridCommand::Reveal(RevealTarget {
                    first: CellCoord { row: 5, col: 1 },
                    last: CellCoord {
                        row: 5,
                        col: LAST_COLUMN,
                    },
                    focus: None,
                }),
                &axes,
                pane,
            )
            .unwrap();
        assert_eq!(full_row.reveal, RevealOutcome::Skipped(SkipReason::FullRow));
        let full_column = controller
            .command(
                &mut model,
                scope,
                GridCommand::Reveal(RevealTarget {
                    first: CellCoord { row: 1, col: 3 },
                    last: CellCoord {
                        row: LAST_ROW,
                        col: 3,
                    },
                    focus: None,
                }),
                &axes,
                pane,
            )
            .unwrap();
        assert_eq!(
            full_column.reveal,
            RevealOutcome::Skipped(SkipReason::FullColumn)
        );
    }

    #[test]
    fn sync_follows_the_engine_origin_without_revealing() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let pane = pane(100, 100);
        let mut controller = NavigationController::default();
        controller
            .attach(&mut model, scope, &axes, pane, None)
            .unwrap();
        model.set_top_left_visible_cell(5, 1).unwrap();
        let update = controller.sync(&model, scope, &axes, pane).unwrap();
        assert_eq!(update.position.first_cell.row, 5);
        assert_eq!(update.position.offset_y, 0);
        assert_eq!(update.reveal, RevealOutcome::NotRequested);
    }

    #[test]
    fn attach_restores_the_saved_view_and_engine_origin() {
        let mut model = model();
        let axes = axes(&model);
        let first = scope(&model, 0);
        let second = scope(&model, 1);
        let pane = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(first);
        controller
            .attach(&mut model, first, &axes, pane, None)
            .unwrap();
        controller
            .command(
                &mut model,
                first,
                GridCommand::Scroll { dx: 0.0, dy: 180.0 },
                &axes,
                pane,
            )
            .unwrap();
        assert_eq!(model.get_selected_view().top_row, 10);

        controller
            .attach(&mut model, second, &axes, pane, None)
            .unwrap();
        controller.activate(second);
        controller
            .command(
                &mut model,
                second,
                GridCommand::Scroll { dx: 0.0, dy: 200.0 },
                &axes,
                pane,
            )
            .unwrap();
        assert_eq!(model.get_selected_view().top_row, 20);

        // The saved position survives the other primary view. Activation
        // restores it and mirrors the engine origin.
        controller.activate(first);
        let restored = controller
            .attach(&mut model, first, &axes, pane, None)
            .unwrap();
        assert_eq!(restored.position.first_cell.row, 10);
        assert!(restored.mirrored);
        assert_eq!(model.get_selected_view().top_row, 10);
    }

    #[test]
    fn command_keeps_the_accepted_state_after_an_engine_failure() {
        let mut model = model();
        let axes = axes(&model);
        let scope = scope(&model, 0);
        let pane = pane(100, 100);
        let mut controller = NavigationController::default();
        controller.activate(scope);
        controller
            .attach(&mut model, scope, &axes, pane, None)
            .unwrap();
        controller.fail_next_engine_write = true;
        let before = controller.view_position(scope).unwrap();
        let result = controller.command(
            &mut model,
            scope,
            GridCommand::Scroll { dx: 0.0, dy: 80.0 },
            &axes,
            pane,
        );
        assert!(matches!(result, Err(GridError::Engine(_))));
        assert_eq!(controller.view_position(scope), Some(before));
        assert_eq!(model.get_selected_view().top_row, 1);
    }
}
