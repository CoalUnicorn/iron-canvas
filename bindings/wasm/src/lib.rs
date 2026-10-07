use iron_canvas::{CanvasSession, OverlayState, RenderRequest, RevisionToken};
use iron_canvas_canvas2d::Canvas2dSceneBackend;
use iron_canvas_core::{CanvasMetrics, CanvasSize, CanvasTheme, CellCoord};
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::{Model, UserModel};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;

#[wasm_bindgen]
pub struct SheetTab {
    index: u32,
    name: String,
}

#[wasm_bindgen]
impl SheetTab {
    #[wasm_bindgen(getter)]
    pub fn index(&self) -> u32 {
        self.index
    }

    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }
}

#[wasm_bindgen]
pub struct Workbook {
    workbook: UserModel<'static>,
    canvas: HtmlCanvasElement,
    metrics: CanvasMetrics,
    session: CanvasSession<Canvas2dSceneBackend>,
    workbook_id: u64,
    revision: u64,
}

#[wasm_bindgen]
impl Workbook {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<Workbook, JsValue> {
        let metrics = metrics_for(0.0, 0.0, 1.0)?;
        let session = new_session(&canvas, metrics)?;
        Ok(Workbook {
            workbook: sample_workbook()?,
            canvas,
            metrics,
            session,
            workbook_id: 1,
            revision: 0,
        })
    }

    #[wasm_bindgen(js_name = resize)]
    pub fn resize(&mut self, width: f64, height: f64, dpr: f64) -> Result<(), JsValue> {
        let metrics = metrics_for(width, height, dpr)?;
        if metrics != self.metrics {
            self.session = new_session(&self.canvas, metrics)?;
            self.metrics = metrics;
        }
        self.render()
    }

    #[wasm_bindgen(js_name = getSheets)]
    pub fn get_sheets(&self) -> Vec<SheetTab> {
        self.workbook
            .get_worksheets_properties()
            .into_iter()
            .enumerate()
            .filter(|(_, sheet)| sheet.state == "visible")
            .map(|(index, sheet)| SheetTab {
                index: index as u32,
                name: sheet.name,
            })
            .collect()
    }

    #[wasm_bindgen(js_name = selectedSheet)]
    pub fn selected_sheet(&self) -> u32 {
        self.workbook.get_selected_sheet()
    }

    #[wasm_bindgen(js_name = workbookName)]
    pub fn workbook_name(&self) -> String {
        self.workbook.get_name()
    }

    #[wasm_bindgen(js_name = selectSheet)]
    pub fn select_sheet(&mut self, index: u32) -> Result<(), JsValue> {
        self.workbook
            .set_selected_sheet(index)
            .map_err(|error| JsValue::from_str(&error))?;
        self.revision = self.revision.saturating_add(1);
        self.render()
    }

    #[wasm_bindgen(js_name = scrollBy)]
    pub fn scroll_by(&mut self, row_delta: i32, column_delta: i32) -> Result<(), JsValue> {
        let view = self.workbook.get_selected_view();
        let top_row = view.top_row.saturating_add(row_delta).clamp(1, LAST_ROW);
        let left_column = view
            .left_column
            .saturating_add(column_delta)
            .clamp(1, LAST_COLUMN);
        if top_row == view.top_row && left_column == view.left_column {
            return Ok(());
        }

        self.workbook
            .set_top_left_visible_cell(top_row, left_column)
            .map_err(|error| JsValue::from_str(&error))?;
        self.revision = self.revision.saturating_add(1);
        self.render()
    }

    #[wasm_bindgen(js_name = addSheet)]
    pub fn add_sheet(&mut self) -> Result<(), JsValue> {
        self.workbook
            .new_sheet()
            .map_err(|error| JsValue::from_str(&error))?;
        self.revision = self.revision.saturating_add(1);
        self.render()
    }

    #[wasm_bindgen(js_name = loadXlsx)]
    pub fn load_xlsx(&mut self, bytes: &[u8], file_name: &str) -> Result<(), JsValue> {
        let workbook = load_from_xlsx_bytes(bytes, file_name, "en", "UTC")
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let model =
            Model::from_workbook(workbook, "en").map_err(|error| JsValue::from_str(&error))?;

        self.workbook = UserModel::from_model(model);
        self.workbook_id = self.workbook_id.saturating_add(1);
        self.revision = 0;
        self.render()
    }
}

impl Workbook {
    fn render(&mut self) -> Result<(), JsValue> {
        let (width, height) = self.metrics.size().to_logical_extent();
        if width == 0 || height == 0 {
            return Ok(());
        }

        let view = self.workbook.get_selected_view();
        let request = RenderRequest {
            sheet: view.sheet,
            revision: RevisionToken {
                workbook_id: self.workbook_id,
                revision: self.revision,
            },
            viewport: WorksheetViewport {
                first_cell: CellCoord {
                    row: view.top_row.max(1),
                    col: view.left_column.max(1),
                },
                width,
                height,
            },
            theme: CanvasTheme::light(),
            overlays: OverlayState {
                active_cell: Some(CellCoord {
                    row: view.row.max(1),
                    col: view.column.max(1),
                }),
                ..OverlayState::default()
            },
        };

        self.session
            .render(&self.workbook, &request)
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&format!("{error:?}")))
    }
}

fn metrics_for(width: f64, height: f64, dpr: f64) -> Result<CanvasMetrics, JsValue> {
    CanvasMetrics::new(
        CanvasSize {
            w: width,
            h: height,
        },
        dpr,
    )
    .map_err(|error| JsValue::from_str(&error.to_string()))
}

fn new_session(
    canvas: &HtmlCanvasElement,
    metrics: CanvasMetrics,
) -> Result<CanvasSession<Canvas2dSceneBackend>, JsValue> {
    let backend = Canvas2dSceneBackend::new(canvas.clone(), metrics)?;
    Ok(CanvasSession::new(backend))
}

fn sample_workbook() -> Result<UserModel<'static>, JsValue> {
    let mut workbook = UserModel::new_empty("Example workbook", "en", "UTC", "en")
        .map_err(|error| JsValue::from_str(&error))?;
    let first_sheet = [
        (1, 1, "Quarter"),
        (1, 2, "Revenue"),
        (1, 3, "Status"),
        (2, 1, "Q1"),
        (2, 2, "128"),
        (2, 3, "On track"),
        (3, 1, "Q2"),
        (3, 2, "146"),
        (3, 3, "On track"),
        (4, 1, "Q3"),
        (4, 2, "139"),
        (4, 3, "Review"),
        (5, 1, "Q4"),
        (5, 2, "171"),
        (5, 3, "On track"),
    ];
    for (row, column, value) in first_sheet {
        workbook
            .set_user_input(0, row, column, value)
            .map_err(|error| JsValue::from_str(&error))?;
    }

    workbook
        .new_sheet()
        .map_err(|error| JsValue::from_str(&error))?;
    workbook
        .set_user_input(1, 1, 1, "Load an .xlsx workbook to view its sheets.")
        .map_err(|error| JsValue::from_str(&error))?;
    workbook
        .set_selected_sheet(0)
        .map_err(|error| JsValue::from_str(&error))?;
    Ok(workbook)
}
