//! Shared IronCalc navigation for hosts of the read-only canvas session.
//!
//! A host grants a short mutable model borrow for each command. The engine
//! resolves merges and hidden axes. The caller owns frame presentation.

mod scroll;

use iron_canvas_core::CellCoord;
use iron_canvas_core::ScrollPosition;
use iron_canvas_core::navigation::{
    Direction, NavigationIntent, PageDirection, TraversalAxis, TraversalDirection,
};
use ironcalc_base::{UserModel, worksheet::NavigationDirection};
use scroll::ViewMap;

pub(crate) const LAST_ROW: i32 = 1_048_576;
pub(crate) const LAST_COLUMN: i32 = 16_384;

/// Stable identity of one host view. The sheet ID survives sheet reordering.
/// A host view identity separates two windows that show the same sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewIdentity {
    pub workbook_id: u64,
    pub view_id: u64,
    pub sheet_id: u32,
}

/// Pane and freeze inputs for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pane {
    pub width: i32,
    pub height: i32,
    pub frozen_rows: i32,
    pub frozen_columns: i32,
}

/// Bounds of the target. `focus` is the moving end for an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevealTarget {
    pub first: CellCoord,
    pub last: CellCoord,
    pub focus: Option<CellCoord>,
}

/// One grid input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridCommand {
    /// Logical pixels. Each axis keeps a signed fractional remainder.
    Scroll { dx: f64, dy: f64 },
    /// One classified key command. Applies the engine and reveals inline.
    Navigate(NavigationIntent),
    /// Explicit reveal. Used for a pointer jump, a link, or a name box.
    Reveal(RevealTarget),
}

/// Why the viewport did not move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The selection spans every column.
    FullRow,
    /// The selection spans every row.
    FullColumn,
    /// The pane has no extent on one axis.
    EmptyPane,
}

/// Viewport result of one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealOutcome {
    NotRequested,
    Moved,
    AlreadyVisible,
    Skipped(SkipReason),
}

/// One command result. `changed` covers the accepted position only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub position: ScrollPosition,
    pub generation: u64,
    pub mirrored: bool,
    pub changed: bool,
    pub reveal: RevealOutcome,
}

/// Typed result of the engine adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationResult {
    pub target: RevealTarget,
}

/// Typed adapter error. IronCalc returns strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationError {
    /// The target is outside the worksheet.
    InvalidTarget,
    /// The engine rejected a step. The view can be partly changed.
    Engine(String),
}

impl std::fmt::Display for NavigationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTarget => f.write_str("selection target is outside the worksheet"),
            Self::Engine(error) => write!(f, "engine navigation failed: {error}"),
        }
    }
}

impl std::error::Error for NavigationError {}

/// One view-state error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridError {
    UnknownView(ViewIdentity),
    IdentityMismatch,
    AxesMismatch,
    GenerationExhausted,
    InvalidDelta,
    /// The target is outside the sheet or has no visible extent.
    NotVisible,
    Navigation(NavigationError),
    Engine(String),
}

impl std::fmt::Display for GridError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownView(identity) => write!(
                f,
                "view is not attached: workbook {}, view {}, sheet {}",
                identity.workbook_id, identity.view_id, identity.sheet_id
            ),
            Self::IdentityMismatch => f.write_str("view identity does not match the engine sheet"),
            Self::AxesMismatch => f.write_str("axis sizes do not match the view identity"),
            Self::GenerationExhausted => f.write_str("view generation is exhausted"),
            Self::InvalidDelta => f.write_str("scroll delta is not finite"),
            Self::NotVisible => {
                f.write_str("reveal target is outside the sheet or has no visible extent")
            }
            Self::Navigation(error) => write!(f, "{error}"),
            Self::Engine(error) => write!(f, "engine scroll update failed: {error}"),
        }
    }
}

impl std::error::Error for GridError {}

/// Owns the accepted scroll position for each view.
#[derive(Default)]
pub struct NavigationController {
    views: ViewMap,
    primary: Option<ViewIdentity>,
    #[cfg(test)]
    fail_next_engine_write: bool,
}

impl NavigationController {
    /// Select the view that mirrors engine indices.
    pub fn activate(&mut self, identity: ViewIdentity) {
        self.primary = Some(identity);
    }
}

/// Apply one classified command to the host model.
///
/// The caller validates workbook and sheet identity before this call. An
/// engine error can follow a partial view change, so the caller reads and
/// presents the actual view even when this function returns an error.
pub fn apply_navigation(
    model: &mut UserModel<'_>,
    intent: NavigationIntent,
    pane_height: i32,
) -> Result<NavigationResult, NavigationError> {
    let before = model.get_selected_view();
    let before_selection = (before.row, before.column, before.range);
    apply_intent(model, intent, pane_height)?;
    let after = model.get_selected_view();
    Ok(NavigationResult {
        target: navigation_target(before_selection, (after.row, after.column, after.range)),
    })
}

fn apply_intent(
    model: &mut UserModel<'_>,
    intent: NavigationIntent,
    pane_height: i32,
) -> Result<(), NavigationError> {
    use NavigationIntent as I;
    match intent {
        I::Step(direction) => {
            // Plain movement starts from the selected cell, not the old range.
            let (_, row, col) = model.get_selected_cell();
            model
                .set_selected_cell(row, col)
                .map_err(NavigationError::Engine)?;
            match direction {
                Direction::Up => model.on_arrow_up(),
                Direction::Down => model.on_arrow_down(),
                Direction::Left => model.on_arrow_left(),
                Direction::Right => model.on_arrow_right(),
            }
            .map_err(NavigationError::Engine)
        }
        I::ExtendStep(direction) => model
            .on_expand_selected_range(match direction {
                Direction::Up => "ArrowUp",
                Direction::Down => "ArrowDown",
                Direction::Left => "ArrowLeft",
                Direction::Right => "ArrowRight",
            })
            .map_err(NavigationError::Engine),
        I::DataEdge(direction) => {
            let (_, row, col) = model.get_selected_cell();
            model
                .set_selected_cell(row, col)
                .map_err(NavigationError::Engine)?;
            model
                .on_navigate_to_edge_in_direction(match direction {
                    Direction::Up => NavigationDirection::Up,
                    Direction::Down => NavigationDirection::Down,
                    Direction::Left => NavigationDirection::Left,
                    Direction::Right => NavigationDirection::Right,
                })
                .map_err(NavigationError::Engine)
        }
        I::Traverse { axis, direction } => apply_intent(
            model,
            I::Step(match (axis, direction) {
                (TraversalAxis::Rows, TraversalDirection::Forward) => Direction::Right,
                (TraversalAxis::Rows, TraversalDirection::Backward) => Direction::Left,
                (TraversalAxis::Columns, TraversalDirection::Forward) => Direction::Down,
                (TraversalAxis::Columns, TraversalDirection::Backward) => Direction::Up,
            }),
            pane_height,
        ),
        I::Page(_) if pane_height == 0 => Ok(()),
        I::Page(direction) => {
            let (_, row, col) = model.get_selected_cell();
            model
                .set_selected_cell(row, col)
                .map_err(NavigationError::Engine)?;
            match direction {
                PageDirection::Up => model.on_page_up(),
                PageDirection::Down => model.on_page_down(),
            }
            .map_err(NavigationError::Engine)
        }
        I::RowStart => {
            let (_, row, _) = model.get_selected_cell();
            model
                .set_selected_cell(row, 1)
                .map_err(NavigationError::Engine)
        }
        I::RowEnd => apply_intent(model, I::DataEdge(Direction::Right), pane_height),
        I::SheetStart => model
            .set_selected_cell(1, 1)
            .map_err(NavigationError::Engine),
        I::UsedEnd | I::SelectUsedRange => {
            let sheet = model.get_selected_sheet();
            let dimension = model
                .get_model()
                .workbook
                .worksheet(sheet)
                .map_err(NavigationError::Engine)?
                .dimension();
            if intent == I::UsedEnd {
                model
                    .set_selected_cell(dimension.max_row, dimension.max_column)
                    .map_err(NavigationError::Engine)
            } else {
                model
                    .set_selected_cell(dimension.min_row, dimension.min_column)
                    .map_err(NavigationError::Engine)?;
                model
                    .set_selected_range(
                        dimension.min_row,
                        dimension.min_column,
                        dimension.max_row,
                        dimension.max_column,
                    )
                    .map_err(NavigationError::Engine)
            }
        }
        I::Cancel => Ok(()),
    }
}

/// Compute one reveal target from the selection before and after the command.
///
/// A moved active cell wins. An extension targets the corner opposite the old
/// active cell. A full row or full column keeps the range so the reveal can
/// skip it.
fn navigation_target(before: (i32, i32, [i32; 4]), after: (i32, i32, [i32; 4])) -> RevealTarget {
    let (before_row, before_column, before_range) = before;
    let (after_row, after_column, after_range) = after;
    let [first_row, first_column, last_row, last_column] = after_range;
    let first = CellCoord {
        row: first_row,
        col: first_column,
    };
    let last = CellCoord {
        row: last_row,
        col: last_column,
    };
    let full_row = first_column == 1 && last_column == LAST_COLUMN;
    let full_column = first_row == 1 && last_row == LAST_ROW;
    if full_row || full_column {
        return RevealTarget {
            first,
            last,
            focus: None,
        };
    }
    let active = CellCoord {
        row: after_row,
        col: after_column,
    };
    if after_row != before_row || after_column != before_column {
        return RevealTarget {
            first: active,
            last: active,
            focus: None,
        };
    }
    if after_range != before_range {
        let anchor = (before_row, before_column);
        let corners = [
            (first_row, first_column),
            (first_row, last_column),
            (last_row, first_column),
            (last_row, last_column),
        ];
        let opposite = corners
            .into_iter()
            .max_by_key(|(row, column)| (row - anchor.0).abs() + (column - anchor.1).abs())
            .unwrap_or((active.row, active.col));
        return RevealTarget {
            first,
            last,
            focus: Some(CellCoord {
                row: opposite.0,
                col: opposite.1,
            }),
        };
    }
    RevealTarget {
        first: active,
        last: active,
        focus: None,
    }
}

/// Set an explicit anchor and focus with the existing engine operation.
///
/// This also lets IronCalc adjust the scroll origin. The two engine calls
/// are not atomic. Use this only after the host validates the target.
pub fn select_anchor_and_focus(
    model: &mut UserModel<'_>,
    anchor: CellCoord,
    focus: CellCoord,
) -> Result<(), NavigationError> {
    if !in_bounds(anchor) || !in_bounds(focus) {
        return Err(NavigationError::InvalidTarget);
    }
    model
        .set_selected_cell(anchor.row, anchor.col)
        .map_err(NavigationError::Engine)?;
    model
        .on_area_selecting(focus.row, focus.col)
        .map_err(NavigationError::Engine)
}

fn in_bounds(coord: CellCoord) -> bool {
    (1..=LAST_ROW).contains(&coord.row) && (1..=LAST_COLUMN).contains(&coord.col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_focus_restores_merge_contraction() {
        use ironcalc_base::expressions::types::Area;

        let mut model = UserModel::new_empty("focus", "en", "UTC", "en").unwrap();
        model
            .merge_cells(&Area {
                sheet: 0,
                row: 1,
                column: 2,
                width: 2,
                height: 1,
            })
            .unwrap();
        select_anchor_and_focus(
            &mut model,
            CellCoord { row: 1, col: 1 },
            CellCoord { row: 1, col: 4 },
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 1, 4]);
        apply_navigation(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Left),
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 1, 3]);
        apply_navigation(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Left),
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 1, 1]);
    }

    #[test]
    fn explicit_focus_skips_hidden_rows_on_next_step() {
        let mut model = UserModel::new_empty("focus", "en", "UTC", "en").unwrap();
        model.set_rows_hidden(0, 2, 2, true).unwrap();
        select_anchor_and_focus(
            &mut model,
            CellCoord { row: 1, col: 1 },
            CellCoord { row: 3, col: 1 },
        )
        .unwrap();
        apply_navigation(&mut model, NavigationIntent::ExtendStep(Direction::Up), 200).unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 1, 1]);
    }

    #[test]
    fn navigation_result_targets_the_active_cell_and_extension_corner() {
        let mut model = UserModel::new_empty("target", "en", "UTC", "en").unwrap();
        let stepped =
            apply_navigation(&mut model, NavigationIntent::Step(Direction::Right), 200).unwrap();
        assert_eq!(
            stepped.target,
            RevealTarget {
                first: CellCoord { row: 1, col: 2 },
                last: CellCoord { row: 1, col: 2 },
                focus: None,
            }
        );
        let extended = apply_navigation(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Down),
            200,
        )
        .unwrap();
        assert_eq!(extended.target.focus, Some(CellCoord { row: 2, col: 2 }));
    }

    #[test]
    fn invalid_focus_does_not_change_the_model_view() {
        let mut model = UserModel::new_empty("focus", "en", "UTC", "en").unwrap();
        let before = model.get_selected_view();
        assert!(
            select_anchor_and_focus(
                &mut model,
                CellCoord { row: 2, col: 2 },
                CellCoord { row: 0, col: 2 },
            )
            .is_err()
        );
        let after = model.get_selected_view();
        assert_eq!(
            (after.row, after.column, after.range),
            (before.row, before.column, before.range)
        );
    }
}
