//! Shared IronCalc navigation for hosts of the read-only canvas session.
//!
//! A host grants a short mutable model borrow for each command. The engine
//! resolves merges and hidden axes. The caller owns frame presentation.

mod scroll;
pub use scroll::{ScrollCommand, ScrollError, ScrollUpdate, ScrollViewport};

use iron_canvas::{Direction, NavigationIntent, PageDirection, TraversalAxis, TraversalDirection};
use ironcalc_base::{UserModel, worksheet::NavigationDirection};
use scroll::ViewMap;
use std::collections::VecDeque;

const QUEUE_LIMIT: usize = 64;

/// Identity of a queued command. The sheet ID survives sheet reordering.
/// A host view identity separates two windows that show the same sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavigationScope {
    pub workbook_id: u64,
    pub view_id: u64,
    pub sheet_id: u32,
}

/// One entry in the shared navigation and scroll queue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControllerCommand {
    Navigation(NavigationIntent),
    Scroll(ScrollCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    Full,
    /// A repeated input arrived at capacity and was discarded.
    RepeatDropped,
    InteractionExhausted,
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => f.write_str("navigation queue is full"),
            Self::RepeatDropped => f.write_str("repeated input was dropped at capacity"),
            Self::InteractionExhausted => f.write_str("grid interaction identity exhausted"),
        }
    }
}

impl std::error::Error for QueueError {}

#[derive(Debug, Clone, Copy)]
struct PendingCommand {
    command: ControllerCommand,
    scope: NavigationScope,
    interaction: u64,
}

/// Keep commands in input order until geometry is ready.
#[derive(Default)]
pub struct NavigationController {
    pending: VecDeque<PendingCommand>,
    interaction: u64,
    selection_revision: u64,
    views: ViewMap,
    primary: Option<NavigationScope>,
    #[cfg(test)]
    fail_next_engine_write: bool,
}

impl NavigationController {
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn selection_revision(&self) -> u64 {
        self.selection_revision
    }

    pub fn next_revision(&self) -> Option<u64> {
        self.selection_revision.checked_add(1)
    }

    pub fn accept_revision(&mut self, revision: u64) {
        self.selection_revision = revision;
    }

    pub fn enqueue(
        &mut self,
        intent: NavigationIntent,
        scope: NavigationScope,
        repeat: bool,
    ) -> Result<(), QueueError> {
        self.enqueue_queued(ControllerCommand::Navigation(intent), scope, repeat)
    }

    pub(crate) fn enqueue_queued(
        &mut self,
        command: ControllerCommand,
        scope: NavigationScope,
        repeat: bool,
    ) -> Result<(), QueueError> {
        if self.interaction == u64::MAX {
            return Err(QueueError::InteractionExhausted);
        }
        if self.pending.len() == QUEUE_LIMIT {
            return Err(if repeat {
                QueueError::RepeatDropped
            } else {
                QueueError::Full
            });
        }
        self.pending.push_back(PendingCommand {
            command,
            scope,
            interaction: self.interaction,
        });
        Ok(())
    }

    /// Remove one command only when the host is ready to execute it.
    pub fn pop_ready(&mut self, scope: NavigationScope) -> Option<ControllerCommand> {
        while let Some(command) = self.pending.pop_front() {
            if command.scope == scope && command.interaction == self.interaction {
                return Some(command.command);
            }
        }
        None
    }

    /// Select the view that mirrors engine indices. Other views keep their
    /// positions locally and never move the selected engine sheet.
    pub fn set_primary_view(&mut self, scope: Option<NavigationScope>) {
        self.primary = scope;
    }

    pub fn cancel(&mut self) -> Result<(), QueueError> {
        let next = self
            .interaction
            .checked_add(1)
            .ok_or(QueueError::InteractionExhausted)?;
        self.pending.clear();
        self.interaction = next;
        Ok(())
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
    fn queue_keeps_order_and_rejects_old_sheet_identity() {
        let mut controller = NavigationController::default();
        let a = NavigationScope {
            workbook_id: 1,
            view_id: 0,
            sheet_id: 7,
        };
        let b = NavigationScope {
            workbook_id: 1,
            view_id: 0,
            sheet_id: 8,
        };
        controller
            .enqueue(NavigationIntent::Step(Direction::Right), a, false)
            .unwrap();
        controller
            .enqueue(NavigationIntent::Step(Direction::Down), b, false)
            .unwrap();
        controller
            .enqueue(NavigationIntent::Step(Direction::Left), b, false)
            .unwrap();
        assert_eq!(
            controller.pop_ready(b),
            Some(ControllerCommand::Navigation(NavigationIntent::Step(
                Direction::Down
            )))
        );
        assert_eq!(
            controller.pop_ready(b),
            Some(ControllerCommand::Navigation(NavigationIntent::Step(
                Direction::Left
            )))
        );
        assert_eq!(controller.pop_ready(b), None);
        controller
            .enqueue(NavigationIntent::Step(Direction::Up), b, false)
            .unwrap();
        controller.cancel().unwrap();
        assert_eq!(controller.pop_ready(b), None);
    }

    #[test]
    fn repeat_overflow_drops_only_the_repeat() {
        let mut controller = NavigationController::default();
        let scope = NavigationScope {
            workbook_id: 1,
            view_id: 0,
            sheet_id: 1,
        };
        for _ in 0..QUEUE_LIMIT {
            controller
                .enqueue(NavigationIntent::Step(Direction::Right), scope, false)
                .unwrap();
        }
        assert_eq!(
            controller.enqueue(NavigationIntent::Step(Direction::Down), scope, true),
            Err(QueueError::RepeatDropped)
        );
        assert_eq!(controller.len(), QUEUE_LIMIT);
        assert_eq!(
            controller.enqueue(NavigationIntent::Step(Direction::Down), scope, false),
            Err(QueueError::Full)
        );
        for _ in 0..QUEUE_LIMIT {
            assert_eq!(
                controller.pop_ready(scope),
                Some(ControllerCommand::Navigation(NavigationIntent::Step(
                    Direction::Right
                )))
            );
        }
        assert_eq!(controller.pop_ready(scope), None);
    }

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
