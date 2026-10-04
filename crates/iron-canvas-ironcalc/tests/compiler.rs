use iron_canvas_core::CellCoord;
use iron_canvas_ironcalc::WorksheetCompiler;
use ironcalc_base::UserModel;

#[test]
fn compiler_borrows_the_workbook_and_prepares_duplicate_cells_once() {
    let workbook =
        UserModel::new_empty("workbook", "en", "UTC", "en").expect("empty workbook constructs");
    let selected_view = workbook.get_selected_view();
    let coordinate = CellCoord { row: 1, col: 1 };

    let prepared = WorksheetCompiler::new()
        .prepare_cells(&workbook, 0, [coordinate, coordinate])
        .expect("requested cells compile");

    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].coordinate, coordinate);
    let view_after = workbook.get_selected_view();
    assert_eq!(
        (
            view_after.sheet,
            view_after.row,
            view_after.column,
            view_after.range,
            view_after.top_row,
            view_after.left_column,
        ),
        (
            selected_view.sheet,
            selected_view.row,
            selected_view.column,
            selected_view.range,
            selected_view.top_row,
            selected_view.left_column,
        )
    );
}
