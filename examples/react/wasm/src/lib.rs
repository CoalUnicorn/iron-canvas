//! React example adapter that connects its workbook UI to Iron Canvas.
//!
//! This app-specific glue lives under `examples/react`; the reusable
//! WebAssembly binding in `bindings/wasm` only accepts Iron Canvas scenes.

use iron_canvas::{
    CanvasSession, OverlayState, RenderRequest, RevisionToken, TextMetricsKey, WorksheetCache,
};
use iron_canvas_canvas2d::CanvasPainter;
use iron_canvas_core::{
    CanvasMetrics, CanvasSize, CanvasTheme, CellCoord,
    painter::{Painter, TextMetrics},
    scene::{GridScene, RenderBackend},
};
use iron_canvas_ironcalc::WorksheetViewport;
use iron_canvas_wasm::CanvasRenderer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::{Model, UserModel};
use std::convert::Infallible;
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

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
    metrics: CanvasMetrics,
    renderer: CanvasRenderer,
    session: CanvasSession<SceneCompilerBackend>,
    workbook_id: u64,
    revision: u64,
    cache: WorksheetCache,
    metrics_generation: u64,
}

#[wasm_bindgen]
impl Workbook {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<Workbook, JsValue> {
        let metrics = metrics_for(0.0, 0.0, 1.0)?;
        let renderer = CanvasRenderer::new(canvas.clone())?;
        let session = new_session(&canvas)?;
        let workbook = sample_workbook()?;
        let cache = WorksheetCache::new(visible_sheet_count(&workbook));
        Ok(Workbook {
            workbook,
            cache,
            metrics_generation: 0,
            metrics,
            renderer,
            session,
            workbook_id: 1,
            revision: 0,
        })
    }

    #[wasm_bindgen(js_name = resize)]
    pub fn resize(&mut self, width: f64, height: f64, dpr: f64) -> Result<(), JsValue> {
        let metrics = metrics_for(width, height, dpr)?;
        if metrics != self.metrics {
            let generation = self
                .metrics_generation
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("text metric generation exhausted"))?;
            self.renderer.resize(width, height, dpr)?;
            self.metrics = metrics;
            self.metrics_generation = generation;
            self.cache.clear();
            self.session.invalidate();
        }
        self.render()
    }

    /// Refresh metric-dependent content after browser fonts change.
    #[wasm_bindgen(js_name = invalidateFonts)]
    pub fn invalidate_fonts(&mut self) -> Result<(), JsValue> {
        self.metrics_generation = self
            .metrics_generation
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("text metric generation exhausted"))?;
        self.cache.clear();
        self.session.invalidate();
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
        self.render()
    }

    #[wasm_bindgen(js_name = addSheet)]
    pub fn add_sheet(&mut self) -> Result<(), JsValue> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("workbook revision exhausted"))?;
        self.workbook
            .new_sheet()
            .map_err(|error| JsValue::from_str(&error))?;
        self.revision = revision;
        self.cache.set_capacity(visible_sheet_count(&self.workbook));
        self.render()
    }

    #[wasm_bindgen(js_name = loadXlsx)]
    pub fn load_xlsx(&mut self, bytes: &[u8], file_name: &str) -> Result<(), JsValue> {
        let workbook = load_from_xlsx_bytes(bytes, file_name, "en", "UTC")
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let model =
            Model::from_workbook(workbook, "en").map_err(|error| JsValue::from_str(&error))?;

        let workbook_id = self
            .workbook_id
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("workbook identity exhausted"))?;
        self.workbook = UserModel::from_model(model);
        self.workbook_id = workbook_id;
        self.cache.clear();
        self.cache.set_capacity(visible_sheet_count(&self.workbook));
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
            .render_cached(
                &mut self.cache,
                &self.workbook,
                &request,
                TextMetricsKey::new(1, self.metrics_generation),
            )
            .map_err(|error| JsValue::from_str(&format!("{error:?}")))?;
        let scene = &self
            .session
            .committed_frame()
            .expect("a successful render commits a frame")
            .worksheet
            .scene;
        self.renderer.render_grid_scene(scene)
    }
}

fn visible_sheet_count(workbook: &UserModel<'_>) -> usize {
    workbook
        .get_worksheets_properties()
        .iter()
        .filter(|sheet| sheet.state == "visible")
        .count()
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

fn new_session(canvas: &HtmlCanvasElement) -> Result<CanvasSession<SceneCompilerBackend>, JsValue> {
    let context = canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("Canvas2D unavailable"))?
        .dyn_into::<CanvasRenderingContext2d>()?;
    Ok(CanvasSession::new(SceneCompilerBackend {
        metrics: CanvasPainter::new(context),
    }))
}

struct SceneCompilerBackend {
    metrics: CanvasPainter,
}

impl RenderBackend for SceneCompilerBackend {
    type StagedFrame = ();
    type Error = Infallible;

    fn metrics(&self) -> &dyn TextMetrics {
        &self.metrics
    }

    fn stage_frame(&mut self, _scene: &GridScene) -> Result<Self::StagedFrame, Self::Error> {
        Ok(())
    }

    fn invalidate_metrics(&mut self) {
        self.metrics.clear_measure_cache();
        self.metrics.invalidate_cache();
    }

    fn commit_frame(&mut self, _staged: Self::StagedFrame) -> Result<(), Self::Error> {
        Ok(())
    }
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

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn host() -> Workbook {
        let canvas = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        let mut host = Workbook::new(canvas).unwrap();
        host.resize(640.0, 400.0, 1.0).unwrap();
        host
    }

    #[wasm_bindgen_test]
    fn view_changes_keep_revision_and_tab_return_hits_cache() {
        let mut host = host();
        let revision = host.revision;
        host.select_sheet(1).unwrap();
        host.select_sheet(0).unwrap();
        assert_eq!(host.revision, revision);
        assert_eq!(
            host.session
                .committed_frame()
                .unwrap()
                .worksheet
                .extended_style_reads,
            0
        );
        host.scroll_by(2, 1).unwrap();
        assert_eq!(host.revision, revision);
        assert!(
            host.session
                .committed_frame()
                .unwrap()
                .worksheet
                .extended_style_reads
                > 0
        );
        assert_eq!(host.cache.len(), 2);
        host.select_sheet(1).unwrap();
        host.select_sheet(0).unwrap();
        assert_eq!(
            host.session
                .committed_frame()
                .unwrap()
                .worksheet
                .extended_style_reads,
            0
        );
        host.add_sheet().unwrap();
        assert_eq!(host.revision, revision + 1);
        assert_eq!(host.cache.capacity(), 3);
        host.select_sheet(0).unwrap();
        assert!(
            host.session
                .committed_frame()
                .unwrap()
                .worksheet
                .extended_style_reads
                > 0
        );
    }

    #[wasm_bindgen_test]
    fn font_changes_refresh_generation_and_recompile_content() {
        let mut host = host();
        host.select_sheet(1).unwrap();
        let revision = host.revision;
        host.invalidate_fonts().unwrap();
        assert_eq!(host.metrics_generation, 2);
        assert_eq!(host.revision, revision);
        assert_eq!(host.cache.len(), 1);
        assert!(
            host.session
                .committed_frame()
                .unwrap()
                .worksheet
                .extended_style_reads
                > 0
        );
        host.select_sheet(0).unwrap();
        assert!(
            host.session
                .committed_frame()
                .unwrap()
                .worksheet
                .extended_style_reads
                > 0
        );
    }

    #[wasm_bindgen_test]
    fn exhausted_counters_reject_changes_before_model_or_metric_state_changes() {
        let mut host = host();
        host.revision = u64::MAX;
        let sheets = host.get_sheets().len();
        assert!(host.add_sheet().is_err());
        assert_eq!(host.get_sheets().len(), sheets);
        host.metrics_generation = u64::MAX;
        let frame = host.session.committed_frame().cloned();
        assert!(host.invalidate_fonts().is_err());
        assert!(host.resize(800.0, 600.0, 1.0).is_err());
        assert_eq!(host.session.committed_frame(), frame.as_ref());
    }

    #[wasm_bindgen_test]
    fn workbook_replacement_changes_identity_and_clears_old_entries() {
        const XLSX: &[u8] =
            include_bytes!("../../../../crates/iron-canvas-ironcalc/tests/fixtures/cf_tests.xlsx");
        let mut host = host();
        host.select_sheet(1).unwrap();
        assert_eq!(host.cache.len(), 2);
        host.load_xlsx(XLSX, "replacement").unwrap();
        assert_eq!(host.workbook_id, 2);
        assert_eq!(host.revision, 0);
        assert_eq!(host.cache.len(), 1);
        assert!(
            host.session
                .committed_frame()
                .unwrap()
                .worksheet
                .extended_style_reads
                > 0
        );

        host.workbook_id = u64::MAX;
        let before = host.workbook.to_bytes();
        let frame = host.session.committed_frame().cloned();
        assert!(host.load_xlsx(XLSX, "rejected").is_err());
        assert_eq!(host.workbook.to_bytes(), before);
        assert_eq!(host.session.committed_frame(), frame.as_ref());
        assert_eq!(host.cache.len(), 1);
    }
}
