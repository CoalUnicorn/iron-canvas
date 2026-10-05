//! Pixel- and cell-space primitives.
//!
//! Every visible artifact composes from [`pixel_rect::PixelRect`] and
//! [`prim::Line`]. The cell-address <-> pixel-rect mapping lives in
//! `slot` (legacy); layout values in `constants` (legacy); the canvas extent in
//! [`extent`]; the Excel-style column label helper in `labels` (legacy).

#[cfg(feature = "legacy")]
pub mod constants;
pub mod extent;
#[cfg(feature = "legacy")]
pub mod labels;
pub mod path;
pub mod pixel_rect;
pub mod prim;
#[cfg(feature = "legacy")]
pub mod slot;

pub use extent::{CanvasMetricError, CanvasMetrics, CanvasSize};
