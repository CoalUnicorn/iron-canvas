//! Single-page PDF assembly from a populated content stream.
//!
//! The page-open CTM `1 0 0 -1 0 H cm` flips the painter's Y-down coords
//! into PDF Y-up user space. It is emitted here, once per document, rather
//! than once per paint call.

use super::doc::{ContentStream, PdfDocument, font, object, page};

/// PDF document assembly. A unit type: [`Self::build_document`] is the
/// only entry point, and it works from a stream the caller already owns.
pub(crate) struct PdfSurface;

impl PdfSurface {
    /// Wrap an already-populated `ContentStream` in the page-open CTM and
    /// emit a complete single-page PDF.
    ///
    /// `shadings` are the distinct horizontal-gradient endpoint color pairs
    /// the painter registered, inlined into the `/Resources` object.
    pub fn build_document(
        stream: &ContentStream,
        shadings: &[(String, String)],
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let mut page_stream = ContentStream::new();
        page_stream.write(format!("1 0 0 -1 0 {height} cm\n").as_bytes());
        page_stream.write(stream.bytes());

        // Object numbering matches the smoke-test layout:
        // 1=Catalog, 2=Pages, 3=Page, 4=Contents, 5=Resources.
        let mut doc = PdfDocument::new();
        doc.add_object(1, 0, object::catalog_object(2));
        doc.add_object(2, 0, object::pages_object(3));
        doc.add_object(3, 0, page::page_object(2, 4, 5, width, height));
        doc.add_object(4, 0, page_stream.into_object());
        doc.add_object(5, 0, font::resources_object_with_helvetica(shadings));
        doc.finish()
    }
}
