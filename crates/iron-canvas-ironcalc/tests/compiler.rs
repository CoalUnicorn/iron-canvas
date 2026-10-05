use iron_canvas_core::{
    CanvasTheme, CellCoord, PixelRect, Point, painter::TextMetrics, scene::Primitive,
};
use iron_canvas_ironcalc::{CellPlacement, WorksheetCompiler, WorksheetViewport};
use ironcalc_base::UserModel;
struct Metrics;
impl TextMetrics for Metrics {
    fn measure_text_width(&self, text: &str, _: &str) -> f64 {
        text.chars().count() as f64 * 8.0
    }
}
fn rect() -> PixelRect {
    PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: 100,
        height: 30,
    }
}

#[test]
fn compiler_reads_each_distinct_cell_once_without_changing_workbook_or_view() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook.set_user_input(0, 1, 1, "42").unwrap();
    let before = workbook.to_bytes();
    let placement = CellPlacement {
        coordinate: CellCoord { row: 1, col: 1 },
        bounds: rect(),
    };
    let prepared = WorksheetCompiler::new()
        .prepare_cells(
            &workbook,
            0,
            rect(),
            [placement, placement],
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap();
    assert_eq!(prepared.extended_style_reads, 1);
    assert_eq!(prepared.cells.len(), 2);
    assert_eq!(workbook.to_bytes(), before);
    assert!(
        prepared.scene.passes[4]
            .iter()
            .any(|p| matches!(p, Primitive::Text {text, ..} if text == "42"))
    );
}

#[test]
fn invalid_sheet_is_rejected_even_without_cells() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    assert!(
        WorksheetCompiler::new()
            .prepare_cells(
                &workbook,
                u32::MAX,
                rect(),
                [],
                &Metrics,
                &CanvasTheme::light()
            )
            .is_err()
    );
    assert!(
        WorksheetCompiler::new()
            .prepare(
                &workbook,
                u32::MAX,
                WorksheetViewport {
                    first_cell: CellCoord { row: 1, col: 1 },
                    width: 0,
                    height: 0
                },
                &Metrics,
                &CanvasTheme::light()
            )
            .is_err()
    );
}

#[test]
fn invalid_late_coordinate_returns_an_error_instead_of_partial_scene() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    let cells = [
        CellPlacement {
            coordinate: CellCoord { row: 1, col: 1 },
            bounds: rect(),
        },
        CellPlacement {
            coordinate: CellCoord { row: 0, col: 1 },
            bounds: rect(),
        },
    ];
    let error = WorksheetCompiler::new()
        .prepare_cells(&workbook, 0, rect(), cells, &Metrics, &CanvasTheme::light())
        .unwrap_err();
    assert_eq!(error.cell.row, 0);
}

#[test]
fn explicit_sheet_does_not_change_selection() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook.new_sheet().unwrap();
    workbook.set_user_input(0, 1, 1, "first sheet").unwrap();
    workbook.set_user_input(1, 1, 1, "second sheet").unwrap();
    let before = workbook.to_bytes();
    let result = WorksheetCompiler::new()
        .prepare(
            &workbook,
            0,
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 100,
                height: 30,
            },
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap();
    assert!(
        result.scene.passes[4]
            .iter()
            .any(|p| matches!(p, Primitive::Text {text, ..} if text == "first sheet"))
    );
    assert_eq!(workbook.to_bytes(), before);
}
