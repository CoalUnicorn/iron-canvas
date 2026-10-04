//! Stage-2 smoke test: `SvgSurface` satisfies the `Surface` trait bound
//! so `Orchestrator<SvgSurface>` can be constructed. End-to-end paint
//! verification lives in the browser test (Stage 5); building a stub
//! `CanvasModel` here would be ~200 LOC for marginal extra coverage over
//! the existing `MemSurface`-driven integration tests.

#![cfg(feature = "svg")]

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{CanvasTheme, CellLink, LinkTarget, Orchestrator, RCRange};
use iron_canvas_export::SvgSurface;

mod common;
use common::TestModel;

#[test]
fn orchestrator_accepts_svg_surface() {
    let grid = SvgSurface::new(100, 50);
    let overlay = SvgSurface::new(100, 50);
    // The whole point of Stage 2: this type resolves.
    let _orch: Orchestrator<SvgSurface> = Orchestrator::new(grid, overlay);
}

#[test]
fn empty_surface_finishes_to_bare_svg() {
    let surface = SvgSurface::new(100, 50);
    let svg = surface.finish();
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    assert!(svg.contains("viewBox=\"0 0 100 50\""));
}

#[test]
fn svg_render_discards_overlay() {
    // A grid with content and an explicit selection. The selection paints only
    // through the *overlay* surface, which `render` drops.
    let model = Rc::new(TestModel::new("hello"));

    let svg = SvgSurface::render(
        model,
        &CanvasTheme::light(),
        CanvasSize { w: 300.0, h: 200.0 },
    )
    .expect("a one-shot export of a readable model commits a frame");

    assert!(
        svg.starts_with("<svg "),
        "render did not return an svg document"
    );
    assert!(
        svg.contains("<g class=\"cells\">"),
        "grid cell group missing from render output"
    );
    assert!(
        svg.contains(">hello</text>"),
        "cell content missing from render output"
    );
    assert!(
        !svg.contains("class=\"overlay\""),
        "overlay group leaked into the grid-only SVG export"
    );
}

/// Acceptance 5 (visual half): a link's resolved color and underline must
/// reach the SVG. PDF replays the same `Painter::fill_text` and
/// `stroke_text_hline` calls, so the shared rule covers both; the PDF
/// primitives themselves are pinned in `pdf_painter_smoke`.
#[test]
fn svg_export_passes_link_color_and_underline_to_the_painter() {
    let model = TestModel::new("link").with_links(vec![CellLink::new(
        RCRange::from_cell(1, 1),
        LinkTarget::External("https://example.com".to_string()),
        None,
        true,
        Some("#0563c1".to_string()),
    )]);

    let svg = SvgSurface::render(
        Rc::new(model),
        &CanvasTheme::light(),
        CanvasSize { w: 300.0, h: 200.0 },
    )
    .expect("a one-shot export of a readable model commits a frame");

    assert!(
        svg.contains("fill=\"#0563c1\""),
        "the link's resolved color must reach the SVG text: {svg}"
    );
    assert!(
        svg.contains("stroke=\"#0563c1\""),
        "the link underline must reach the SVG: {svg}"
    );
}
