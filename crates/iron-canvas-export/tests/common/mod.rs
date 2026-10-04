use iron_canvas_core::{
    CanvasModel, CanvasView, CellContentQuery, CellKind, CellLink, CellStyle, Fetched, RCRange,
};

/// Small worksheet model shared by the export smoke tests. It deliberately
/// implements only the engine contracts these tests exercise.
pub struct TestModel {
    value: String,
    links: Vec<CellLink>,
    show_selection: bool,
}

impl TestModel {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            links: Vec::new(),
            show_selection: true,
        }
    }

    #[allow(dead_code)]
    pub fn with_links(mut self, links: Vec<CellLink>) -> Self {
        self.links = links;
        self
    }

    #[allow(dead_code)]
    pub fn with_selection(mut self, show_selection: bool) -> Self {
        self.show_selection = show_selection;
        self
    }
}

impl CellContentQuery for TestModel {
    fn get_cell_style(&self, _sheet: u32, _row: i32, _column: i32) -> Fetched<CellStyle> {
        Fetched::Value(CellStyle::default())
    }

    fn get_cell_type(&self, _sheet: u32, _row: i32, _column: i32) -> Fetched<CellKind> {
        Fetched::Value(CellKind::Text)
    }

    fn get_formatted_cell_value(&self, _sheet: u32, row: i32, column: i32) -> Fetched<String> {
        if (row, column) == (1, 1) {
            Fetched::Value(self.value.clone())
        } else {
            Fetched::Absent
        }
    }
}

impl CanvasModel for TestModel {
    fn get_selected_sheet(&self) -> Option<u32> {
        Some(0)
    }

    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange::from_cell(1, 1),
            top_row: 1,
            left_column: 1,
        })
    }

    fn get_frozen_rows_count(&self, _sheet: u32) -> Option<i32> {
        Some(0)
    }

    fn get_frozen_columns_count(&self, _sheet: u32) -> Option<i32> {
        Some(0)
    }

    fn get_row_height(&self, _sheet: u32, _row: i32) -> Fetched<f64> {
        Fetched::Absent
    }

    fn get_column_width(&self, _sheet: u32, _column: i32) -> Fetched<f64> {
        Fetched::Absent
    }

    fn get_show_grid_lines(&self, _sheet: u32) -> Fetched<bool> {
        Fetched::Absent
    }

    fn get_sheet_links(&self, _sheet: u32) -> Option<Vec<CellLink>> {
        Some(self.links.clone())
    }

    fn get_show_selection(&self) -> bool {
        self.show_selection
    }

    fn last_row(&self, _sheet: u32) -> i32 {
        5
    }

    fn last_column(&self, _sheet: u32) -> i32 {
        5
    }
}
