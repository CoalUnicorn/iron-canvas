//! Hand-rolled PDF backend: writer (`doc`) + `Painter` adapter (`painter`)
//! and the scene-assembly helper (`surface`).
//!
//! `PdfPainter` translates `Painter`/`BlitPainter`/`TextMetrics` calls
//! into PDF content-stream ops and accumulates them in a shared
//! `Rc<RefCell<ContentStream>>`. `PdfSurface::build_document` wraps a
//! populated stream in the page-open CTM and emits a complete single-page
//! document.

pub mod doc;
pub mod painter;

pub use doc::PdfDocument;
pub use painter::PdfPainter;

mod surface;

mod scene;
pub use scene::PdfSceneBackend;
