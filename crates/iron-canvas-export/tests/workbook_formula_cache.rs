//! Finding 14: export the original XLSX bytes with absent or stale formula caches.
#![cfg(any(feature = "svg", feature = "pdf"))]

use iron_canvas_core::{CanvasTheme, CellCoord};
use iron_canvas_export::workbook::{CellRange, export_xlsx};
use std::{fs, path::Path};

fn assert_evaluated_export(extension: &str, expected_text: &str) {
    for name in [
        "formula_missing_cache.xlsx",
        "formula_missing_numeric_cache.xlsx",
        "formula_stale_cache.xlsx",
    ] {
        let input = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let output = std::env::temp_dir().join(format!(
            "iron-canvas-formula-fixture-{}-{name}.{extension}",
            std::process::id()
        ));
        let cell = CellCoord { row: 1, col: 1 };
        export_xlsx(
            input,
            &output,
            None,
            CellRange::new(cell, cell).unwrap(),
            &CanvasTheme::light(),
        )
        .unwrap();
        let bytes = fs::read(&output).unwrap();
        fs::remove_file(output).unwrap();
        let document = String::from_utf8_lossy(&bytes);
        assert!(
            document.contains(expected_text),
            "{name}: expected 1+1 = 2 in {extension}"
        );
    }
}

#[cfg(feature = "svg")]
#[test]
fn svg_export_evaluates_missing_and_stale_caches() {
    assert_evaluated_export("svg", ">2</text>");
}

#[cfg(feature = "pdf")]
#[test]
fn pdf_export_evaluates_missing_and_stale_caches() {
    assert_evaluated_export("pdf", "(2) Tj");
}
