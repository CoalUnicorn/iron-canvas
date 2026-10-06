//! Multi-format export backend for `iron-canvas`.
//!
//! Each format lives behind its own feature flag and contributes a
//! `Painter + BlitPainter + TextMetrics` adapter plus a scene backend that
//! stages a resolved `GridScene` into a document. The workbook export helpers
//! compose the existing IronCalc compiler with those backends.

pub mod common;
pub mod workbook;

pub use workbook::{CellRange, WorkbookExportError, export_xlsx};

#[cfg(feature = "svg")]
pub mod svg;

#[cfg(feature = "svg")]
pub use svg::{SvgPainter, SvgSceneBackend};

#[cfg(feature = "pdf")]
pub mod pdf;

#[cfg(feature = "pdf")]
pub use pdf::{PdfDocument, PdfPainter, PdfSceneBackend};
