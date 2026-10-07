//! `iron-canvas-core` — the pure-Rust application layer of the iron-canvas
//! grid renderer. Browser (wasm) backends and probes live in sibling crates.
//!
//! The crate exposes generic geometry, layout, text-wrapping, theme, and
//! resolved-scene types; spreadsheet semantics live in `iron-canvas-ironcalc`.

pub mod geometry;
pub mod painter;
pub mod scene;
pub mod scene_geometry;
pub mod shape;
pub mod text_layout;
pub mod theme;

pub use geometry::{
    CanvasMetricError, CanvasMetrics, CanvasSize,
    pixel_rect::PixelRect,
    prim::{Axis, Line, Point, Side, Span},
};
pub use text_layout::{
    TextLayoutOptions, TextLine, TextVerticalAlign, layout_text_into, layout_text_lines_into,
};
pub use theme::{CanvasTheme, ThemeVariables};

pub use scene_geometry::CellCoord;
