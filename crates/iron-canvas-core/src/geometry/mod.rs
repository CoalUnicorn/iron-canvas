//! Pixel- and cell-space primitives.
//!
//! Every visible artifact composes from [`pixel_rect::PixelRect`] and
//! [`prim::Line`]. The canvas extent lives in [`extent`], backend-shared
//! layout values in [`constants`], and stroke geometry in [`path`].

pub mod constants;
pub mod extent;
pub mod path;
pub mod pixel_rect;
pub mod prim;

pub use extent::{CanvasMetricError, CanvasMetrics, CanvasSize};
