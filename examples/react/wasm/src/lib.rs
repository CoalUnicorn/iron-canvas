use iron_canvas::{CanvasSession, OverlayState, RenderRequest, RevisionToken};
use iron_canvas_canvas2d::Canvas2dSceneBackend;
use iron_canvas_core::{CanvasMetrics, CanvasSize, CanvasTheme, CellCoord};
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc_base::UserModel;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

#[wasm_bindgen]
pub struct ReactExample {
    canvas: HtmlCanvasElement,
    workbook: Vec<u8>,
    metrics: Option<CanvasMetrics>,
    session: Option<CanvasSession<Canvas2dSceneBackend>>,
}

#[wasm_bindgen]
impl ReactExample {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<ReactExample, JsValue> {
        Ok(Self {
            canvas,
            workbook: sample_workbook()?,
            metrics: None,
            session: None,
        })
    }

    /// Render the example sheet at its current CSS-pixel size and device ratio.
    pub fn render(&mut self, width: f64, height: f64, dpr: f64) -> Result<(), JsValue> {
        let metrics = CanvasMetrics::new(
            CanvasSize {
                w: width,
                h: height,
            },
            dpr,
        )
        .map_err(js_error)?;
        let (logical_width, logical_height) = metrics.logical_extent();
        if logical_width == 0 || logical_height == 0 {
            return Ok(());
        }

        if self.metrics != Some(metrics) {
            let backend = Canvas2dSceneBackend::new(self.canvas.clone(), metrics)?;
            self.session = Some(CanvasSession::new(backend));
            self.metrics = Some(metrics);
        }

        let workbook = UserModel::from_bytes(&self.workbook, "en").map_err(js_error)?;
        let request = RenderRequest {
            sheet: 0,
            revision: RevisionToken {
                workbook_id: 1,
                revision: 1,
            },
            viewport: WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: logical_width,
                height: logical_height,
            },
            theme: CanvasTheme::light(),
            overlays: OverlayState::default(),
        };

        self.session
            .as_mut()
            .expect("the session is initialized for non-empty canvas metrics")
            .render(&workbook, &request)
            .map_err(js_error)?;
        Ok(())
    }
}

fn sample_workbook() -> Result<Vec<u8>, JsValue> {
    let mut workbook =
        UserModel::new_empty("React example", "en", "UTC", "en").map_err(js_error)?;
    workbook
        .set_columns_width(0, 1, 4, 132.0)
        .map_err(js_error)?;
    workbook.set_rows_height(0, 1, 6, 30.0).map_err(js_error)?;

    let rows = [
        ["IronCanvas", "React + WebAssembly", "", ""],
        ["Quarter", "Revenue", "Costs", "Profit"],
        ["Q1", "12500", "8400", "=B3-C3"],
        ["Q2", "14800", "9100", "=B4-C4"],
        ["Q3", "13750", "8750", "=B5-C5"],
        ["Q4", "16200", "10200", "=B6-C6"],
    ];
    for (row, values) in rows.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            if !value.is_empty() {
                workbook
                    .set_user_input(0, row as i32 + 1, column as i32 + 1, value)
                    .map_err(js_error)?;
            }
        }
    }
    Ok(workbook.to_bytes())
}

fn js_error(error: impl std::fmt::Debug) -> JsValue {
    JsValue::from_str(&format!("{error:?}"))
}
