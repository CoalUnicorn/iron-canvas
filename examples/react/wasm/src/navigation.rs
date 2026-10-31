//! Host-owned command order and engine movement. Canvas never applies writes.

use super::{LAST_COLUMN, LAST_ROW, Workbook};
use iron_canvas::{
    BindingProfile, InteractionFrame, NavigationIntent, NavigationKey, NavigationModifiers,
    NavigationReadiness, VisibilityAdjustment, classify_navigation,
};
use iron_canvas_core::{CellCoord, Point};
use iron_canvas_navigation::{
    ControllerCommand, NavigationController as SharedController, NavigationScope, QueueError,
};
use ironcalc_base::UserModel;
use wasm_bindgen::prelude::*;

#[cfg(all(test, target_arch = "wasm32"))]
const QUEUE_LIMIT: usize = 64;

#[wasm_bindgen]
pub struct NavigationCommand {
    intent: NavigationIntent,
}

#[wasm_bindgen(js_name = classifyKey)]
pub fn classify_key(
    key: &str,
    primary: bool,
    shift: bool,
    alt: bool,
    alt_graph: bool,
) -> Option<NavigationCommand> {
    let intent = classify_navigation(
        NavigationKey::from_key(key)?,
        NavigationModifiers {
            primary,
            shift,
            alt,
            alt_graph,
        },
        BindingProfile::Basic,
    )?;
    Some(NavigationCommand { intent })
}

#[derive(Default)]
pub(super) struct NavigationController {
    pub commands: SharedController,
    pub presented: Option<(InteractionFrame, u64, Option<u64>)>,
}

impl NavigationController {
    pub fn cancel(&mut self) -> Result<(), JsValue> {
        self.commands
            .cancel()
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }
}

impl Workbook {
    pub(super) fn navigation_scope(&self) -> Result<NavigationScope, JsValue> {
        let sheet = self.workbook.get_selected_sheet();
        let sheet_id = self
            .workbook
            .get_model()
            .workbook
            .worksheet(sheet)
            .map_err(|error| JsValue::from_str(&error))?
            .sheet_id;
        Ok(NavigationScope {
            workbook_id: self.workbook_id,
            view_id: 0,
            sheet_id,
        })
    }
}

#[wasm_bindgen]
impl Workbook {
    /// The DOM route consumes the key before it calls this method.
    #[wasm_bindgen(js_name = enqueueNavigation)]
    pub fn enqueue_navigation(
        &mut self,
        command: NavigationCommand,
        repeat: bool,
    ) -> Result<(), JsValue> {
        if command.intent == NavigationIntent::Cancel {
            return self.cancel_navigation();
        }
        let scope = self.navigation_scope()?;
        match self
            .navigation
            .commands
            .enqueue(command.intent, scope, repeat)
        {
            Ok(()) => Ok(()),
            Err(QueueError::RepeatDropped) if repeat => Ok(()),
            Err(error) => Err(JsValue::from_str(&error.to_string())),
        }
    }

    #[wasm_bindgen(js_name = cancelNavigation)]
    pub fn cancel_navigation(&mut self) -> Result<(), JsValue> {
        self.navigation.cancel()
    }

    /// Wait for valid geometry, then consume each command once in FIFO order.
    #[wasm_bindgen(js_name = flushNavigation)]
    pub fn flush_navigation(&mut self) -> Result<(), JsValue> {
        if self.navigation.commands.is_empty() {
            return Ok(());
        }
        let Some(expected) = self.render_request() else {
            return Ok(());
        };
        if matches!(
            self.session.navigation_geometry(&expected),
            NavigationReadiness::Pending(_)
        ) {
            self.render()?;
        }
        while !self.navigation.commands.is_empty() {
            let Some(expected) = self.render_request() else {
                break;
            };
            let (pane, frozen) = match self.session.navigation_geometry(&expected) {
                NavigationReadiness::Ready(ready) => {
                    (ready.geometry.scroll_pane_rect(), ready.frozen)
                }
                NavigationReadiness::Pending(_) => break,
            };
            let scope = self.navigation_scope()?;
            let Some(command) = self.navigation.commands.pop_ready(scope) else {
                break;
            };
            // The host scroll path consumes pixel commands in a later stage.
            let ControllerCommand::Navigation(intent) = command else {
                return Err(JsValue::from_str(
                    "scroll commands are not accepted by the navigation route",
                ));
            };
            if matches!(intent, NavigationIntent::Page(_)) && pane.height == 0 {
                continue;
            }
            let next_revision = self
                .navigation
                .commands
                .next_revision()
                .ok_or_else(|| JsValue::from_str("selection revision exhausted"))?;
            // Engine movement consumes logical pane dimensions. Release the
            // geometry borrow before the engine changes its accepted view.
            self.workbook.set_window_width(f64::from(pane.width));
            self.workbook.set_window_height(f64::from(pane.height));
            let origin = expected.viewport.scroll.first_cell;
            let origin = CellCoord {
                row: origin.row.max((frozen.rows + 1).min(LAST_ROW)),
                col: origin.col.max((frozen.cols + 1).min(LAST_COLUMN)),
            };
            let execution = self
                .workbook
                .set_top_left_visible_cell(origin.row, origin.col)
                .and_then(|()| execute(&mut self.workbook, intent, pane.height));
            self.navigation.commands.accept_revision(next_revision);
            if let Err(error) = execution {
                // A multi-step engine operation can change the view before an
                // error. Present the actual view and never replay that command.
                let presentation = self.render();
                return Err(presentation
                    .err()
                    .unwrap_or_else(|| JsValue::from_str(&error)));
            }
            let current = self.workbook.get_selected_view();
            let live = self
                .render_request()
                .ok_or_else(|| JsValue::from_str("viewport unavailable"))?;
            match self.session.scroll_to_show_checked(
                &self.workbook,
                &live,
                CellCoord {
                    row: current.row,
                    col: current.column,
                },
            ) {
                Ok(VisibilityAdjustment::Unchanged) => {}
                Ok(VisibilityAdjustment::ScrollTo(origin)) => {
                    if let Err(error) = self
                        .workbook
                        .set_top_left_visible_cell(origin.row, origin.col)
                    {
                        let _ = self.render();
                        return Err(JsValue::from_str(&error));
                    }
                }
                Err(error) => {
                    let _ = self.render();
                    return Err(JsValue::from_str(&error.to_string()));
                }
            }
        }
        self.render()
    }

    #[wasm_bindgen(js_name = activeCell)]
    pub fn active_cell(&self) -> String {
        let view = self.workbook.get_selected_view();
        let column =
            ironcalc_base::expressions::utils::number_to_column(view.column).unwrap_or_default();
        format!("{column}{}", view.row)
    }

    /// Accept a basic click only against the frame the host actually presents.
    #[wasm_bindgen(js_name = selectAt)]
    pub fn select_at(&mut self, x: i32, y: i32) -> Result<(), JsValue> {
        let expected = self
            .render_request()
            .ok_or_else(|| JsValue::from_str("viewport unavailable"))?;
        let stamp = self.session.interaction_frame();
        let scope = self.navigation_scope()?;
        let generation = self.navigation.commands.view_generation(scope);
        if !self.navigation.commands.is_empty()
            || self.navigation.presented
                != stamp.map(|stamp| {
                    (
                        stamp,
                        self.navigation.commands.selection_revision(),
                        generation,
                    )
                })
            || stamp.is_none_or(|stamp| {
                stamp.viewport != expected.viewport
                    || stamp.revision != expected.revision
                    || stamp.sheet != expected.sheet
            })
            || matches!(
                self.session.navigation_geometry(&expected),
                NavigationReadiness::Pending(_)
            )
        {
            return Err(JsValue::from_str("wait for worksheet presentation"));
        }
        if let Some(iron_canvas::HostIntent::Select { anchor, .. }) = self
            .session
            .pointer_intent(Point { x, y }, iron_canvas::PointerAction::Select)
        {
            let revision = self
                .navigation
                .commands
                .next_revision()
                .ok_or_else(|| JsValue::from_str("selection revision exhausted"))?;
            let result = iron_canvas_navigation::select_anchor_and_focus(
                &mut self.workbook,
                anchor.row,
                anchor.col,
                anchor.row,
                anchor.col,
            );
            self.navigation.commands.accept_revision(revision);
            let presentation = self.render();
            result.map_err(|error| JsValue::from_str(&error))?;
            presentation?;
        }
        Ok(())
    }
}

fn execute(
    model: &mut UserModel<'_>,
    intent: NavigationIntent,
    pane_height: i32,
) -> Result<(), String> {
    iron_canvas_navigation::apply_navigation(model, intent, pane_height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas::{Direction, PageDirection, TraversalAxis, TraversalDirection};
    use ironcalc_base::expressions::types::Area;

    #[test]
    fn basic_shift_arrows_extend_and_contract_over_merges_with_a_fixed_anchor() {
        let mut model = UserModel::new_empty("anchor", "en", "UTC", "en").unwrap();
        model
            .merge_cells(&Area {
                sheet: 0,
                row: 1,
                column: 2,
                width: 2,
                height: 1,
            })
            .unwrap();
        model.set_selected_cell(1, 1).unwrap();
        execute(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Right),
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 1, 3]);
        execute(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Left),
            200,
        )
        .unwrap();
        let view = model.get_selected_view();
        assert_eq!((view.row, view.column), (1, 1));
        assert_eq!(view.range, [1, 1, 1, 1]);
        model.set_selected_cell(1, 4).unwrap();
        execute(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Left),
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 2, 1, 4]);
        execute(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Right),
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 4, 1, 4]);
        model.set_rows_hidden(0, 2, 2, true).unwrap();
        model.set_selected_cell(1, 1).unwrap();
        execute(
            &mut model,
            NavigationIntent::ExtendStep(Direction::Down),
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 3, 1]);
        execute(&mut model, NavigationIntent::ExtendStep(Direction::Up), 200).unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 1, 1]);
    }

    #[test]
    fn real_engine_basic_targets_and_reverse_enter_add_no_content_undo_entry() {
        let mut model = UserModel::new_empty("basic", "en", "UTC", "en").unwrap();
        for col in 1..=4 {
            model.set_user_input(0, 1, col, "value").unwrap();
        }
        model.set_user_input(0, 10, 10, "history marker").unwrap();
        execute(
            &mut model,
            NavigationIntent::DataEdge(Direction::Right),
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_cell(), (0, 1, 4));
        execute(&mut model, NavigationIntent::RowStart, 200).unwrap();
        assert_eq!(model.get_selected_cell(), (0, 1, 1));
        execute(&mut model, NavigationIntent::RowEnd, 200).unwrap();
        assert_eq!(model.get_selected_cell(), (0, 1, 4));
        execute(&mut model, NavigationIntent::UsedEnd, 200).unwrap();
        assert_eq!(model.get_selected_cell(), (0, 10, 10));
        execute(
            &mut model,
            NavigationIntent::Traverse {
                axis: TraversalAxis::Columns,
                direction: TraversalDirection::Backward,
            },
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_cell(), (0, 9, 10));
        model.set_window_height(200.0);
        execute(&mut model, NavigationIntent::Page(PageDirection::Down), 200).unwrap();
        assert!(model.get_selected_view().row > 9);
        execute(&mut model, NavigationIntent::SheetStart, 200).unwrap();
        execute(&mut model, NavigationIntent::SelectUsedRange, 200).unwrap();
        assert_eq!(model.get_selected_view().range, [1, 1, 10, 10]);
        execute(
            &mut model,
            NavigationIntent::Traverse {
                axis: TraversalAxis::Rows,
                direction: TraversalDirection::Forward,
            },
            200,
        )
        .unwrap();
        assert_eq!(model.get_selected_view().range, [1, 2, 1, 2]);
        model.undo().unwrap();
        assert_eq!(model.get_cell_content(0, 10, 10).unwrap(), "");
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn queue(host: &mut Workbook, key: &str, shift: bool) {
        host.enqueue_navigation(
            classify_key(key, false, shift, false, false).unwrap(),
            false,
        )
        .unwrap();
    }

    #[wasm_bindgen_test]
    fn queued_keys_execute_in_order_from_the_live_view_before_one_presentation() {
        let mut host = crate::tests::host();
        queue(&mut host, "ArrowRight", false);
        queue(&mut host, "ArrowRight", false);
        assert_eq!(host.active_cell(), "A1");
        host.flush_navigation().unwrap();
        assert_eq!(host.active_cell(), "C1");
        assert_eq!(host.revision, 0);
        assert!(host.navigation.commands.is_empty());
        assert_eq!(
            host.navigation.presented,
            host.session.interaction_frame().map(|stamp| (stamp, 2))
        );
        queue(&mut host, "ArrowDown", false);
        queue(&mut host, "Enter", true);
        host.flush_navigation().unwrap();
        assert_eq!(host.active_cell(), "C1");
    }

    #[wasm_bindgen_test]
    fn queue_limits_cancellation_and_geometry_wait_do_not_reorder_commands() {
        let mut host = crate::tests::host();
        host.metrics = super::super::metrics_for(0.0, 0.0, 1.0).unwrap();
        for _ in 0..QUEUE_LIMIT {
            queue(&mut host, "ArrowRight", false);
        }
        host.enqueue_navigation(
            classify_key("ArrowRight", false, false, false, false).unwrap(),
            true,
        )
        .unwrap();
        assert!(
            host.enqueue_navigation(
                classify_key("ArrowRight", false, false, false, false).unwrap(),
                false
            )
            .is_err()
        );
        host.flush_navigation().unwrap();
        assert_eq!(host.navigation.commands.len(), QUEUE_LIMIT);
        host.cancel_navigation().unwrap();
        assert!(host.navigation.commands.is_empty());
        host.resize(640.0, 400.0, 1.0).unwrap();
        queue(&mut host, "ArrowDown", false);
        host.select_sheet(1).unwrap();
        host.flush_navigation().unwrap();
        assert_eq!(host.active_cell(), "A1");
    }

    #[wasm_bindgen_test]
    fn failed_presentation_blocks_hits_and_never_replays_executed_commands() {
        let mut host = crate::tests::host();
        host.fail_presentation = true;
        queue(&mut host, "ArrowRight", false);
        assert!(host.flush_navigation().is_err());
        assert_eq!(host.active_cell(), "B1");
        assert!(host.navigation.commands.is_empty());
        assert!(host.navigation.presented.is_none());
        assert!(host.select_at(100, 50).is_err());
        host.fail_presentation = false;
        host.render().unwrap();
        host.flush_navigation().unwrap();
        assert_eq!(host.active_cell(), "B1");
        let rect = host.session.cell_rect(1, 1).unwrap();
        host.select_at(rect.center().x, rect.center().y).unwrap();
        assert_eq!(host.active_cell(), "A1");
    }

    #[wasm_bindgen_test]
    fn failed_preparation_keeps_unstarted_commands_for_one_later_execution() {
        let mut host = crate::tests::host();
        host.workbook.set_rows_height(0, 1, 1, f64::MAX).unwrap();
        host.revision += 1;
        host.session.invalidate();
        queue(&mut host, "ArrowRight", false);
        assert!(host.flush_navigation().is_err());
        assert_eq!(host.navigation.commands.len(), 1);
        assert_eq!(host.active_cell(), "A1");
        host.workbook.set_rows_height(0, 1, 1, 24.0).unwrap();
        host.revision += 1;
        host.flush_navigation().unwrap();
        assert_eq!(host.active_cell(), "B1");
        assert!(host.navigation.commands.is_empty());
    }

    #[wasm_bindgen_test]
    fn execution_errors_publish_actual_view_and_zero_panes_consume_page_as_a_noop() {
        let mut host = crate::tests::host();
        host.workbook.set_selected_cell(LAST_ROW, 1).unwrap();
        host.workbook
            .set_selected_range(LAST_ROW, 1, LAST_ROW, 2)
            .unwrap();
        host.workbook
            .set_top_left_visible_cell(LAST_ROW, 1)
            .unwrap();
        host.render().unwrap();
        queue(&mut host, "PageDown", false);
        assert!(host.flush_navigation().is_err());
        assert_eq!(
            host.workbook.get_selected_view().range,
            [LAST_ROW, 1, LAST_ROW, 1]
        );
        assert!(host.navigation.presented.is_some());
        assert!(host.navigation.commands.is_empty());

        host.workbook.set_selected_cell(1, 1).unwrap();
        host.workbook.set_top_left_visible_cell(1, 1).unwrap();
        host.workbook.set_frozen_rows_count(0, 100).unwrap();
        host.revision += 1;
        host.render().unwrap();
        let before = host.workbook.to_bytes();
        queue(&mut host, "PageDown", false);
        host.flush_navigation().unwrap();
        assert_eq!(host.workbook.to_bytes(), before);
    }
}
