//! P4 Task 1: test the visibility contract at the workspace IronCalc pin.
//!
//! The root tests check supported behavior. `pinned_engine_gaps` reproduces
//! known defects. A passing test in that module does not satisfy a P4 gate.
//! Replace each defect assertion with the required behavior after E3 lands.

use ironcalc_base::{Model, UserModel, expressions::types::Area};

const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;

#[derive(Clone, Copy, Debug)]
enum Axis {
    Row,
    Column,
}

const AXES: [Axis; 2] = [Axis::Row, Axis::Column];

impl Axis {
    fn last(self) -> i32 {
        match self {
            Self::Row => LAST_ROW,
            Self::Column => LAST_COLUMN,
        }
    }

    fn hide(
        self,
        model: &mut UserModel<'_>,
        sheet: u32,
        first: i32,
        last: i32,
        hidden: bool,
    ) -> Result<(), String> {
        match self {
            Self::Row => model.set_rows_hidden(sheet, first, last, hidden),
            Self::Column => model.set_columns_hidden(sheet, first, last, hidden),
        }
    }

    fn set_size(self, model: &mut UserModel<'_>, index: i32, pixels: f64) {
        match self {
            Self::Row => model.set_rows_height(0, index, index, pixels),
            Self::Column => model.set_columns_width(0, index, index, pixels),
        }
        .unwrap();
    }

    fn size(self, model: &UserModel<'_>, index: i32) -> f64 {
        match self {
            Self::Row => model.get_row_height(0, index),
            Self::Column => model.get_column_width(0, index),
        }
        .unwrap()
    }

    fn hidden(self, model: &UserModel<'_>, index: i32) -> bool {
        let sheet = model.get_model().workbook.worksheet(0).unwrap();
        match self {
            Self::Row => sheet.is_row_hidden(index),
            Self::Column => sheet.is_column_hidden(index),
        }
        .unwrap()
    }

    fn cell(self, index: i32) -> (i32, i32) {
        match self {
            Self::Row => (index, 1),
            Self::Column => (1, index),
        }
    }

    fn select(self, model: &mut UserModel<'_>, index: i32) {
        let (row, column) = self.cell(index);
        model.set_selected_cell(row, column).unwrap();
        // Keep terminal tests local. Engine scroll calculations otherwise walk
        // every preceding row when movement approaches the sheet limit.
        model.set_top_left_visible_cell(row, column).unwrap();
    }

    fn step(self, model: &mut UserModel<'_>, forward: bool) {
        match (self, forward) {
            (Self::Row, true) => model.on_arrow_down(),
            (Self::Row, false) => model.on_arrow_up(),
            (Self::Column, true) => model.on_arrow_right(),
            (Self::Column, false) => model.on_arrow_left(),
        }
        .unwrap();
    }

    fn key(self, forward: bool) -> &'static str {
        match (self, forward) {
            (Self::Row, true) => "ArrowDown",
            (Self::Row, false) => "ArrowUp",
            (Self::Column, true) => "ArrowRight",
            (Self::Column, false) => "ArrowLeft",
        }
    }
}

fn workbook() -> UserModel<'static> {
    UserModel::new_empty("visibility", "en", "UTC", "en").unwrap()
}

fn reload(model: &UserModel<'_>) -> UserModel<'static> {
    // Load fixture state without its setup history. This exposes extra undo
    // entries and permits direct checks of the first visibility action.
    UserModel::from_bytes(&model.to_bytes(), "en").unwrap()
}

fn imported_hidden(axis: Axis, index: i32) -> UserModel<'static> {
    // Construct imported state below UserModel. This avoids using the broken
    // terminal hide action to test independent navigation and Show behavior.
    let mut model = Model::new_empty("imported", "en", "UTC", "en").unwrap();
    match axis {
        Axis::Row => model.set_row_hidden(0, index, true),
        Axis::Column => model.set_column_hidden(0, index, true),
    }
    .unwrap();
    UserModel::from_model(model)
}

#[test]
fn hide_and_show_preserve_unequal_sizes_in_one_undo_step_each() {
    for axis in AXES {
        let mut setup = workbook();
        let sizes = [24.0, 36.0, 72.0];
        for (index, size) in (10..=12).zip(sizes) {
            axis.set_size(&mut setup, index, size);
        }
        let mut model = reload(&setup);
        for hidden in [true, false] {
            axis.hide(&mut model, 0, 10, 12, hidden).unwrap();
            for (index, size) in (10..=12).zip(sizes) {
                assert_eq!(axis.hidden(&model, index), hidden);
                assert_eq!(axis.size(&model, index), if hidden { 0.0 } else { size });
            }
            model.undo().unwrap();
            for (index, size) in (10..=12).zip(sizes) {
                assert_eq!(axis.hidden(&model, index), !hidden);
                assert_eq!(axis.size(&model, index), if hidden { size } else { 0.0 });
            }
            assert_eq!(model.can_undo(), !hidden);
            model.redo().unwrap();
            assert!(!model.can_redo());
            for (index, size) in (10..=12).zip(sizes) {
                assert_eq!(axis.hidden(&model, index), hidden);
                assert_eq!(axis.size(&model, index), if hidden { 0.0 } else { size });
            }
        }
    }
}

#[test]
fn show_reveals_only_hidden_members_in_the_inclusive_interval() {
    for axis in AXES {
        let mut setup = workbook();
        axis.hide(&mut setup, 0, 9, 10, true).unwrap();
        axis.hide(&mut setup, 0, 12, 13, true).unwrap();
        let mut model = reload(&setup);
        let before = (9..=13).map(|i| axis.hidden(&model, i)).collect::<Vec<_>>();
        axis.hide(&mut model, 0, 10, 12, false).unwrap();
        assert_eq!(
            (9..=13).map(|i| axis.hidden(&model, i)).collect::<Vec<_>>(),
            [true, false, false, false, true]
        );
        model.undo().unwrap();
        assert_eq!(
            (9..=13).map(|i| axis.hidden(&model, i)).collect::<Vec<_>>(),
            before
        );
        assert!(!model.can_undo());
    }
}

#[test]
fn hide_selects_the_next_full_axis_and_show_preserves_that_selection() {
    for axis in AXES {
        let mut model = workbook();
        axis.select(&mut model, 10);
        axis.hide(&mut model, 0, 10, 13, true).unwrap();
        let expected = match axis {
            Axis::Row => [14, 1, 14, LAST_COLUMN],
            Axis::Column => [1, 14, LAST_ROW, 14],
        };
        assert_eq!(model.get_selected_view().range, expected);
        axis.hide(&mut model, 0, 10, 13, false).unwrap();
        assert_eq!(model.get_selected_view().range, expected);
    }
}

#[test]
fn hide_on_another_sheet_preserves_the_active_selection() {
    for axis in AXES {
        let mut model = workbook();
        model.new_sheet().unwrap();
        model.set_selected_sheet(0).unwrap();
        model.set_selected_cell(4, 5).unwrap();
        axis.hide(&mut model, 1, 10, 13, true).unwrap();
        assert_eq!(model.get_selected_cell(), (0, 4, 5));
        assert_eq!(model.get_selected_view().range, [4, 5, 4, 5]);
    }
}

#[test]
fn show_imported_terminal_axes_is_undoable() {
    for axis in AXES {
        let mut model = imported_hidden(axis, axis.last());
        axis.hide(&mut model, 0, axis.last(), axis.last(), false)
            .unwrap();
        assert!(!axis.hidden(&model, axis.last()));
        assert!(axis.size(&model, axis.last()) > 0.0);
        model.undo().unwrap();
        assert!(axis.hidden(&model, axis.last()));
        assert!(!model.can_undo());
        model.redo().unwrap();
        assert!(!axis.hidden(&model, axis.last()));
    }
}

#[test]
fn rejection_before_the_first_member_preserves_bytes_history_and_send_queue() {
    for axis in AXES {
        for (sheet, first, last) in [(99, 1, 1), (0, 0, 1), (0, axis.last() + 1, axis.last() + 1)] {
            for hidden in [false, true] {
                let mut model = workbook();
                model.set_name("history marker");
                model.undo().unwrap();
                model.flush_send_queue();
                let empty_queue = model.flush_send_queue();
                let before = model.to_bytes();
                assert!(axis.hide(&mut model, sheet, first, last, hidden).is_err());
                assert!(
                    model.to_bytes() == before,
                    "{axis:?}: rejected action writes data"
                );
                assert!(!model.can_undo());
                assert!(model.can_redo());
                assert_eq!(model.flush_send_queue(), empty_queue);
                model.redo().unwrap();
                assert_eq!(model.get_model().workbook.name, "history marker");
            }
        }
    }
}

#[test]
fn arrows_skip_hidden_runs_in_both_directions_without_undo_entries() {
    for axis in AXES {
        let mut setup = workbook();
        axis.hide(&mut setup, 0, 10, 13, true).unwrap();
        let mut model = reload(&setup);
        axis.select(&mut model, 9);
        axis.step(&mut model, true);
        let (row, column) = axis.cell(14);
        assert_eq!(model.get_selected_cell(), (0, row, column));
        axis.step(&mut model, false);
        let (row, column) = axis.cell(9);
        assert_eq!(model.get_selected_cell(), (0, row, column));
        assert!(!model.can_undo());
    }
}

#[test]
fn arrows_stay_on_the_visible_cell_at_hidden_sheet_limits() {
    for axis in AXES {
        for (hidden, start, forward) in [(1, 2, false), (axis.last(), axis.last() - 1, true)] {
            let mut model = imported_hidden(axis, hidden);
            axis.select(&mut model, start);
            let before = model.to_bytes();
            axis.step(&mut model, forward);
            assert!(
                model.to_bytes() == before,
                "{axis:?}: movement leaves the visible cell"
            );
        }
    }
}

#[test]
fn shift_arrows_keep_hidden_indices_inside_the_range_and_contract_to_the_anchor() {
    for axis in AXES {
        let mut model = workbook();
        axis.hide(&mut model, 0, 10, 13, true).unwrap();
        axis.select(&mut model, 9);
        model.on_expand_selected_range(axis.key(true)).unwrap();
        let (first_row, first_column) = axis.cell(9);
        let (last_row, last_column) = axis.cell(14);
        assert_eq!(
            model.get_selected_view().range,
            [first_row, first_column, last_row, last_column]
        );
        model.on_expand_selected_range(axis.key(false)).unwrap();
        assert_eq!(
            model.get_selected_view().range,
            [first_row, first_column, first_row, first_column]
        );
        assert_eq!(model.get_selected_cell(), (0, first_row, first_column));
    }
}

#[test]
fn movement_keeps_a_hidden_merge_anchor_when_the_merge_has_visible_members() {
    for axis in AXES {
        let mut model = workbook();
        model
            .merge_cells(&Area {
                sheet: 0,
                row: 2,
                column: 2,
                width: 2,
                height: 2,
            })
            .unwrap();
        axis.hide(&mut model, 0, 2, 2, true).unwrap();
        let (row, column) = match axis {
            Axis::Row => (1, 2),
            Axis::Column => (2, 1),
        };
        model.set_selected_cell(row, column).unwrap();
        axis.step(&mut model, true);
        assert_eq!(model.get_selected_cell(), (0, 2, 2));
        assert_eq!(model.get_selected_view().range, [2, 2, 3, 3]);
    }
}

mod pinned_engine_gaps {
    //! These tests prove defects at 7e6ae21. They assert the observed failure,
    //! not the required result. Keep host visibility actions disabled until
    //! E3 replaces these assertions with passing acceptance tests.

    use super::*;

    #[test]
    fn terminal_hide_returns_an_error_after_a_write_without_history_or_diffs() {
        for axis in AXES {
            let mut model = workbook();
            let before = model.to_bytes();
            let empty_queue = model.flush_send_queue();
            let error = axis
                .hide(&mut model, 0, axis.last(), axis.last(), true)
                .unwrap_err();
            assert!(error.contains(&(axis.last() + 1).to_string()));
            assert!(axis.hidden(&model, axis.last()));
            assert!(model.to_bytes() != before);
            assert!(!model.can_undo());
            assert_eq!(model.flush_send_queue(), empty_queue);
            model.undo().unwrap();
            assert!(axis.hidden(&model, axis.last()));
        }
    }

    #[test]
    fn invalid_range_end_leaves_a_partial_hide_or_show_without_history() {
        for axis in AXES {
            for hidden in [false, true] {
                let mut model = if hidden {
                    workbook()
                } else {
                    imported_hidden(axis, axis.last())
                };
                let before = model.to_bytes();
                assert!(
                    axis.hide(&mut model, 0, axis.last(), axis.last() + 1, hidden)
                        .is_err()
                );
                assert_eq!(axis.hidden(&model, axis.last()), hidden);
                assert!(model.to_bytes() != before);
                assert!(!model.can_undo());
            }
        }
    }

    #[test]
    fn reversed_range_succeeds_and_adds_an_empty_undo_entry() {
        for axis in AXES {
            let mut model = workbook();
            axis.hide(&mut model, 0, 12, 10, false).unwrap();
            assert!(model.can_undo());
        }
    }

    #[test]
    fn show_does_not_recover_dimensions_that_round_to_zero() {
        for axis in AXES {
            for size in [0.0, 0.25] {
                let mut setup = workbook();
                axis.set_size(&mut setup, 10, size);
                axis.hide(&mut setup, 0, 10, 10, true).unwrap();
                let mut model = reload(&setup);
                axis.hide(&mut model, 0, 10, 10, false).unwrap();
                assert!(!axis.hidden(&model, 10));
                assert_eq!(axis.size(&model, 10).round(), 0.0);
            }
        }
    }

    #[test]
    fn arrows_land_on_unflagged_dimensions_that_round_to_zero() {
        for axis in AXES {
            for size in [0.0, 0.25] {
                let mut model = workbook();
                axis.set_size(&mut model, 10, size);
                for (start, forward) in [(9, true), (11, false)] {
                    axis.select(&mut model, start);
                    axis.step(&mut model, forward);
                    let (row, column) = axis.cell(10);
                    assert_eq!(model.get_selected_cell(), (0, row, column));
                    assert_eq!(axis.size(&model, 10).round(), 0.0);
                }
            }
        }
    }

    #[test]
    fn shift_arrow_reaches_a_hidden_leading_axis() {
        for axis in AXES {
            let mut model = imported_hidden(axis, 1);
            axis.select(&mut model, 2);
            model.on_expand_selected_range(axis.key(false)).unwrap();
            let (row, column) = axis.cell(2);
            assert_eq!(model.get_selected_view().range, [1, 1, row, column]);
            assert!(axis.hidden(&model, 1));
        }
    }

    #[test]
    fn shift_right_reaches_a_hidden_terminal_column() {
        let mut model = imported_hidden(Axis::Column, LAST_COLUMN);
        Axis::Column.select(&mut model, LAST_COLUMN - 1);
        model.on_expand_selected_range("ArrowRight").unwrap();
        assert_eq!(
            model.get_selected_view().range,
            [1, LAST_COLUMN - 1, 1, LAST_COLUMN]
        );
    }

    #[test]
    fn shift_down_reads_past_the_hidden_terminal_row() {
        let mut model = imported_hidden(Axis::Row, LAST_ROW);
        Axis::Row.select(&mut model, LAST_ROW - 1);
        let before = model.to_bytes();
        let error = model.on_expand_selected_range("ArrowDown").unwrap_err();
        assert!(error.contains(&(LAST_ROW + 1).to_string()));
        assert!(model.to_bytes() == before);
    }
}
