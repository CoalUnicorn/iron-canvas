//! Browser proof for the shared Canvas2D binding.
//!
//! Run with:
//! `WASM_BINDGEN_USE_BROWSER=1 WASM_BINDGEN_USE_CHROME=1`
//! `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`
//! `cargo test -p iron-canvas-wasm --target wasm32-unknown-unknown --test browser`
#![cfg(target_arch = "wasm32")]

use iron_canvas_wasm::{
    CanvasRenderer, ScrollAxis, ScrollPosition, ScrollWindow, classify_navigation_js, move_axis_js,
    move_scroll_js, theme_from_element_value,
};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

wasm_bindgen_test_configure!(run_in_browser);

const WIDTH: u32 = 10;
const HEIGHT: u32 = 96;

fn canvas() -> HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    document
        .create_element("canvas")
        .unwrap()
        .dyn_into::<HtmlCanvasElement>()
        .unwrap()
}

fn context(canvas: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<CanvasRenderingContext2d>()
        .unwrap()
}

fn pixel(canvas: &HtmlCanvasElement, x: u32, y: u32) -> [u8; 4] {
    let data = context(canvas)
        .get_image_data(f64::from(x), f64::from(y), 1.0, 1.0)
        .unwrap()
        .data();
    [data[0], data[1], data[2], data[3]]
}

fn band_scene(middle: &str) -> serde_json::Value {
    json!({
        "clip": { "left": 0, "top": 0, "width": WIDTH, "height": HEIGHT },
        "passes": [[
            { "type": "rectFill",
              "rect": { "left": 0, "top": 0, "width": WIDTH, "height": 32 },
              "color": "#ff0000" },
            { "type": "rectFill",
              "rect": { "left": 0, "top": 32, "width": WIDTH, "height": 32 },
              "color": middle },
            { "type": "rectFill",
              "rect": { "left": 0, "top": 64, "width": WIDTH, "height": 32 },
              "color": "#0000ff" }
        ]]
    })
}

fn render(renderer: &mut CanvasRenderer, scene: serde_json::Value) {
    let value = js_sys::JSON::parse(&scene.to_string()).unwrap();
    renderer.render_scene(value).unwrap();
}

#[wasm_bindgen_test]
fn retained_scene_repaints_the_damaged_band() {
    let canvas = canvas();
    let mut renderer = CanvasRenderer::new(canvas.clone()).unwrap();
    renderer
        .resize(f64::from(WIDTH), f64::from(HEIGHT), 1.0)
        .unwrap();

    render(&mut renderer, band_scene("#00ff00"));
    assert_eq!(pixel(&canvas, 4, 16), [255, 0, 0, 255]);
    assert_eq!(pixel(&canvas, 4, 48), [0, 255, 0, 255]);
    assert_eq!(pixel(&canvas, 4, 80), [0, 0, 255, 255]);

    // Only the middle 32-pixel band changes. A damage update repaints it and
    // must keep the other two bands intact.
    render(&mut renderer, band_scene("#ffff00"));
    assert_eq!(pixel(&canvas, 4, 16), [255, 0, 0, 255]);
    assert_eq!(pixel(&canvas, 4, 48), [255, 255, 0, 255]);
    assert_eq!(pixel(&canvas, 4, 80), [0, 0, 255, 255]);

    // The same scene must reuse the committed pixels.
    render(&mut renderer, band_scene("#ffff00"));
    assert_eq!(pixel(&canvas, 4, 48), [255, 255, 0, 255]);

    // A fresh render must still produce the same pixels.
    let value = js_sys::JSON::parse(&band_scene("#ffff00").to_string()).unwrap();
    renderer.render_scene_fresh(value).unwrap();
    assert_eq!(pixel(&canvas, 4, 48), [255, 255, 0, 255]);

    // A metric invalidation forces the next scene to repaint every band.
    renderer.invalidate_metrics();
    render(&mut renderer, band_scene("#ffff00"));
    assert_eq!(pixel(&canvas, 4, 16), [255, 0, 0, 255]);
    assert_eq!(pixel(&canvas, 4, 48), [255, 255, 0, 255]);
    assert_eq!(pixel(&canvas, 4, 80), [0, 0, 255, 255]);
}

#[wasm_bindgen_test]
fn rect_fill_run_keeps_the_internal_edge() {
    let canvas = canvas();
    let mut renderer = CanvasRenderer::new(canvas.clone()).unwrap();
    renderer.resize(10.0, 10.0, 1.0).unwrap();

    render(
        &mut renderer,
        json!({
            "clip": { "left": 0, "top": 0, "width": 10, "height": 10 },
            "passes": [[{
                "type": "rectFillRun",
                "rect": { "left": 0, "top": 0, "width": 10, "height": 10 },
                "edges": [0, 5, 10],
                "color": "#ff0000"
            }]]
        }),
    );

    // The DTO maps a run of N edges to N - 1 rectangles. Both halves share
    // the color, so the internal edge is visible only as the same paint.
    assert_eq!(pixel(&canvas, 2, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(&canvas, 7, 5), [255, 0, 0, 255]);
}

#[wasm_bindgen_test]
fn theme_from_element_returns_the_palette() {
    let document = web_sys::window().unwrap().document().unwrap();
    let element = document.document_element().unwrap();
    let value = theme_from_element_value(&element).unwrap();
    let theme: serde_json::Value = serde_wasm_bindgen::from_value(value).unwrap();

    assert!(theme["gridColor"].is_string());
    assert!(theme["selectionFill"].is_string());
    assert!(theme["pointingTint"].is_string());
}

#[wasm_bindgen_test]
fn classifies_navigation_and_rejects_unknown_keys() {
    let modifiers = js_sys::JSON::parse(r#"{"shift":true}"#).unwrap();
    let value = classify_navigation_js("ArrowUp", modifiers, None).unwrap();
    let intent: serde_json::Value = serde_wasm_bindgen::from_value(value).unwrap();
    assert_eq!(intent, json!({ "type": "extendStep", "direction": "up" }));

    let value = classify_navigation_js("F9", wasm_bindgen::JsValue::UNDEFINED, None).unwrap();
    assert!(value.is_null());
}

#[wasm_bindgen_test]
fn scroll_math_uses_run_axes() {
    // 25-pixel defaults with one 50-pixel run from index 3 through 4.
    let runs = js_sys::JSON::parse(r#"[{"first":3,"last":4,"extent":50}]"#).unwrap();
    let axis = ScrollAxis::new(25, 6, runs).unwrap();
    assert_eq!(axis.total_extent(), 200.0);

    let window = ScrollWindow::new(1, 6);
    let moved = move_axis_js(1, 0, 30, &axis, &window, 100);
    assert_eq!((moved.first(), moved.offset()), (2, 5));

    // A pane wider than the content keeps the first scrolling cell.
    let columns = ScrollAxis::new(10, 5, wasm_bindgen::JsValue::UNDEFINED).unwrap();
    let column_window = ScrollWindow::new(1, 5);
    let position = move_scroll_js(
        &ScrollPosition::new(1, 1, 0, 0),
        30,
        30,
        &axis,
        &window,
        &columns,
        &column_window,
        100,
        100,
    );
    assert_eq!(position.first_row(), 2);
    assert_eq!(position.first_col(), 1);
    assert_eq!(position.offset_y(), 5);
    assert_eq!(position.offset_x(), 0);
}

#[wasm_bindgen_test]
fn scroll_axis_rejects_unordered_runs() {
    let runs = js_sys::JSON::parse(r#"[{"first":4,"last":3,"extent":50}]"#).unwrap();
    assert!(ScrollAxis::new(25, 10, runs).is_err());
}
