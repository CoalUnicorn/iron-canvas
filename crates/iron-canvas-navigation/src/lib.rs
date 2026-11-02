//! Shared IronCalc navigation for hosts of the read-only canvas session.
//!
//! A host grants a short mutable model borrow for each command. The engine
//! resolves merges and hidden axes. The caller owns frame presentation.

mod scroll;
pub use scroll::{ScrollCommand, ScrollError, ScrollUpdate, ScrollViewport};

use iron_canvas_core::navigation::{
    Direction, NavigationIntent, PageDirection, TraversalAxis, TraversalDirection,
};
use ironcalc_base::{UserModel, worksheet::NavigationDirection};
use scroll::ViewMap;

/// Stable identity of one host view. The sheet ID survives sheet reordering.
/// A host view identity separates two windows that show the same sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavigationScope {
    pub workbook_id: u64,
    pub view_id: u64,
    pub sheet_id: u32,
}

/// Owns the accepted scroll position for each view.
#[derive(Default)]
pub struct NavigationController {
    views: ViewMap,
    primary: Option<NavigationScope>,
    #[cfg(test)]
    fail_next_engine_write: bool,
}

impl NavigationController {
    /// Select the view that mirrors engine indices. Other views keep their
    /// positions locally and never move the selected engine sheet.
    pub fn set_primary_view(&mut self, scope: Option<NavigationScope>) {
        self.primary = scope;
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
) -> Result<(), String> {
    use NavigationIntent as I;
    match intent {
        I::Step(direction) => {
            // Plain movement starts from the selected cell, not the old range.
            let (_, row, col) = model.get_selected_cell();
            model.set_selected_cell(row, col)?;
            match direction {
                Direction::Up => model.on_arrow_up(),
                Direction::Down => model.on_arrow_down(),
                Direction::Left => model.on_arrow_left(),
                Direction::Right => model.on_arrow_right(),
            }
        }
        I::ExtendStep(direction) => model.on_expand_selected_range(match direction {
            Direction::Up => "ArrowUp",
            Direction::Down => "ArrowDown",
            Direction::Left => "ArrowLeft",
            Direction::Right => "ArrowRight",
        }),
        I::DataEdge(direction) => {
            let (_, row, col) = model.get_selected_cell();
            model.set_selected_cell(row, col)?;
            model.on_navigate_to_edge_in_direction(match direction {
                Direction::Up => NavigationDirection::Up,
                Direction::Down => NavigationDirection::Down,
                Direction::Left => NavigationDirection::Left,
                Direction::Right => NavigationDirection::Right,
            })
        }
        I::Traverse { axis, direction } => apply_navigation(
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
            model.set_selected_cell(row, col)?;
            match direction {
                PageDirection::Up => model.on_page_up(),
                PageDirection::Down => model.on_page_down(),
            }
        }
        I::RowStart => {
            let (_, row, _) = model.get_selected_cell();
            model.set_selected_cell(row, 1)
        }
        I::RowEnd => apply_navigation(model, I::DataEdge(Direction::Right), pane_height),
        I::SheetStart => model.set_selected_cell(1, 1),
        I::UsedEnd | I::SelectUsedRange => {
            let sheet = model.get_selected_sheet();
            let dimension = model.get_model().workbook.worksheet(sheet)?.dimension();
            if intent == I::UsedEnd {
                model.set_selected_cell(dimension.max_row, dimension.max_column)
            } else {
                model.set_selected_cell(dimension.min_row, dimension.min_column)?;
                model.set_selected_range(
                    dimension.min_row,
                    dimension.min_column,
                    dimension.max_row,
                    dimension.max_column,
                )
            }
        }
        I::Cancel => Ok(()),
    }
}

/// Set an explicit anchor and focus with the existing engine operation.
///
/// This also lets IronCalc adjust the scroll origin. The two engine calls
/// are not atomic. Use this only after the host validates the target.
pub fn select_anchor_and_focus(
    model: &mut UserModel<'_>,
    anchor_row: i32,
    anchor_column: i32,
    focus_row: i32,
    focus_column: i32,
) -> Result<(), String> {
    if !(1..=1_048_576).contains(&anchor_row)
        || !(1..=16_384).contains(&anchor_column)
        || !(1..=1_048_576).contains(&focus_row)
        || !(1..=16_384).contains(&focus_column)
    {
        return Err("selection target is outside the worksheet".to_owned());
    }
    model.set_selected_cell(anchor_row, anchor_column)?;
    model.on_area_selecting(focus_row, focus_column)
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
        select_anchor_and_focus(&mut model, 1, 1, 1, 4).unwrap();
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
        select_anchor_and_focus(&mut model, 1, 1, 3, 1).unwrap();
        apply_navigation(&mut model, NavigationIntent::ExtendStep(Direction::Up), 200).unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 1, 1]);
    }

    #[test]
    fn invalid_focus_does_not_change_the_model_view() {
        let mut model = UserModel::new_empty("focus", "en", "UTC", "en").unwrap();
        let before = model.get_selected_view();
        assert!(select_anchor_and_focus(&mut model, 2, 2, 0, 2).is_err());
        let after = model.get_selected_view();
        assert_eq!(
            (after.row, after.column, after.range),
            (before.row, before.column, before.range)
        );
    }
}
