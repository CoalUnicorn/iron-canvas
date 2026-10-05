//! Build with --release --target wasm32-unknown-unknown, then wasm-bindgen --target web.
#[cfg(target_arch = "wasm32")]
#[path = "../../iron-canvas/examples/support/allocator.rs"]
mod allocator;
#[cfg(target_arch = "wasm32")]
#[path = "../../iron-canvas/examples/support/measurement.rs"]
mod measurement;
#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace=performance,js_name=now)]
        fn now() -> f64;
    }
    #[wasm_bindgen]
    pub fn measure_scene() -> String {
        let canvas = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("canvas")
            .unwrap()
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .unwrap();
        let metrics = iron_canvas_core::CanvasMetrics::new(
            iron_canvas_core::CanvasSize {
                w: 1100.0,
                h: 700.0,
            },
            1.25,
        )
        .unwrap();
        super::measurement::run(
            iron_canvas_canvas2d::Canvas2dSceneBackend::new(canvas, metrics).unwrap(),
            || now() * 1000.0,
        )
    }
}
fn main() {}
