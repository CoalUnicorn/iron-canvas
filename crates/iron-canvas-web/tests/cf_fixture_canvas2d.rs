#![cfg(all(target_arch = "wasm32", feature = "js-model"))]

//! Browser proof that the conditional-formatting workbook fixture reaches the
//! real Canvas2D backend through the same IronCalc model adapter as SVG.

use std::rc::Rc;

use iron_canvas_canvas2d::WebSurface;
use iron_canvas_core::geometry::{CanvasMetrics, CanvasSize};
use iron_canvas_core::{CanvasTheme, Orchestrator, PaintResult};
use iron_canvas_ironcalc::IronCalcModel;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::{Model, UserModel};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const FIXTURE: &[u8] = include_bytes!("../../iron-canvas-ironcalc/tests/fixtures/cf_tests.xlsx");

fn make_canvas() -> HtmlCanvasElement {
    let document = web_sys::window()
        .expect("browser window")
        .document()
        .expect("browser document");
    document
        .create_element("canvas")
        .expect("canvas element creates")
        .dyn_into::<HtmlCanvasElement>()
        .expect("element is a canvas")
}

#[wasm_bindgen_test]
fn cf_fixture_renders_to_canvas2d() {
    let workbook =
        load_from_xlsx_bytes(FIXTURE, "cf_tests", "en", "UTC").expect("fixture workbook loads");
    let workbook = UserModel::from_model(
        Model::from_workbook(workbook, "en").expect("fixture model constructs"),
    );
    let grid_canvas = make_canvas();
    let overlay_canvas = make_canvas();
    let grid = WebSurface::grid(grid_canvas.clone()).expect("grid surface creates");
    let overlay = WebSurface::overlay(overlay_canvas).expect("overlay surface creates");

    let mut orchestrator = Orchestrator::new(grid, overlay);
    orchestrator.set_theme(CanvasTheme::light());
    orchestrator.set_model(Rc::new(IronCalcModel(workbook)));
    orchestrator.resize(
        CanvasMetrics::new(
            CanvasSize {
                w: 1100.0,
                h: 700.0,
            },
            1.0,
        )
        .expect("canvas metrics are valid"),
    );

    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    let context = grid_canvas
        .get_context("2d")
        .expect("canvas context lookup succeeds")
        .expect("2d context exists")
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .expect("context is Canvas2D");
    let pixels = context
        .get_image_data(0.0, 0.0, 1100.0, 700.0)
        .expect("painted pixels are readable")
        .data();
    let painted = pixels
        .0
        .as_slice()
        .chunks_exact(4)
        .filter(|rgba| rgba[3] != 0)
        .count();

    assert!(
        painted > 1_000,
        "rendered worksheet should contain visible Canvas2D pixels, got {painted}"
    );
}
