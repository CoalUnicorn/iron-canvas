//! Hand-rolled PDF backend: writer (`doc`) + `Painter` adapter (`painter`)
//! and the scene-assembly helper (`surface`).
//!
//! `PdfPainter` translates `Painter`/`BlitPainter`/`TextMetrics` calls
//! into PDF content-stream ops and accumulates them in an owned buffer.
//! `PdfPainter::finish` consumes the painter and assembles its stream and
//! gradient registry into a complete single-page document.

pub mod doc;
pub mod painter;

pub use doc::PdfDocument;
pub use painter::PdfPainter;

mod surface;

mod scene;
pub use scene::PdfSceneBackend;
