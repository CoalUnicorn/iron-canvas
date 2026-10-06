//! IronCalc-free Canvas2D surface for the iron-canvas grid renderer.
//!
//! Hosts the Canvas2D `Painter` impl (`CanvasPainter`), the atomic/retained
//! scene backend (`Canvas2dSceneBackend`), and the CSS-var theme bridge
//! (`theme_from_element`). Every component here touches `web-sys`,
//! `wasm-bindgen`, or `js-sys`, but none depends on `ironcalc_base` — so this
//! crate is reusable by a pure data-grid facade with no spreadsheet semantics.

mod canvas_painter;
mod measure_cache;
pub mod theme_from_element;

pub use canvas_painter::CanvasPainter;

mod scene;
pub use scene::Canvas2dSceneBackend;
