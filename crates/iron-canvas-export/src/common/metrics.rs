//! Real font metrics for the SVG/PDF export backends.
//!
//! The core crate's `approx_text_width` (flat `CHAR_WIDTH_FACTOR = 1.0` per
//! char) is ~2x wider than any real proportional font, so wrap math built on
//! it disagrees badly with what's actually painted — that mismatch is what
//! clips exported text (see `docs/reviews/2026-07-01-export-font-metrics-plan.md`).
//!
//! SVG measures the bundled Inter Regular face. PDF uses the selected
//! Helvetica face and the WinAnsi bytes written to the content stream.
//! Unmapped SVG glyphs use the shared width estimate. Unsupported PDF
//! characters use the advance of the emitted `?` replacement.

use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use iron_canvas_core::painter::approx_text_width;

//  Inter (SVG)

/// Inter Regular (v13, Latin subset) — see `assets/FONT-NOTICE.txt` for
/// source, submodule commit, and license (`assets/OFL.txt`).
const INTER_TTF: &[u8] = include_bytes!("../../assets/inter-regular.ttf");

fn inter_face() -> &'static ttf_parser::Face<'static> {
    static FACE: LazyLock<ttf_parser::Face<'static>> = LazyLock::new(|| {
        ttf_parser::Face::parse(INTER_TTF, 0).expect("bundled Inter TTF must parse")
    });
    &FACE
}

/// Sum per-character advances for `text`.
///
/// `advance` returns the real advance for a character, or `None` when that
/// font's table has no entry for it — such a character alone falls back to the
/// flat [`approx_text_width`] estimate, so every mapped script keeps working
/// exactly as before. Each backend passes its own per-char lookup, so the
/// fallback and the summing live here once.
fn sum_advances(text: &str, size_px: f64, advance: impl Fn(char) -> Option<f64>) -> f64 {
    text.chars()
        .map(|c| {
            advance(c).unwrap_or_else(|| approx_text_width(size_px, c.encode_utf8(&mut [0u8; 4])))
        })
        .sum()
}

/// Sum of real Inter glyph advances for `text` at `size_px`. Characters with
/// no glyph in the embedded subset fall back to the flat estimate for that
/// character alone.
pub fn inter_advance_width(text: &str, size_px: f64) -> f64 {
    let face = inter_face();
    let units_per_em = f64::from(face.units_per_em());
    sum_advances(text, size_px, |c| {
        face.glyph_index(c)
            .and_then(|id| face.glyph_hor_advance(id))
            .map(|adv| f64::from(adv) / units_per_em * size_px)
    })
}

/// Base64 (standard alphabet, padded) of the embedded Inter TTF, for the SVG
/// `@font-face` data URI. Encoded once per process — the TTF is a build-time
/// constant, and every document that drew text carries the same data URI.
pub fn inter_base64() -> &'static str {
    static ENCODED: LazyLock<String> = LazyLock::new(|| STANDARD.encode(INTER_TTF));
    &ENCODED
}

//  Helvetica (PDF)

/// Sum of regular Helvetica advances for the emitted WinAnsi text.
/// Unsupported characters use the width of `?`, matching PDF output.
/// The PDF painter selects style-specific advances through `HelveticaFace`.
pub fn helvetica_advance_width(text: &str, size_px: f64) -> f64 {
    super::helvetica::regular_advance_width(text, size_px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_word_measures_below_flat_estimate() {
        let flat = approx_text_width(13.0, "mortgage");
        let real = inter_advance_width("mortgage", 13.0);
        assert!(
            real < flat * 0.8,
            "real Inter advance ({real}) should be well below the flat 1.0-factor estimate ({flat})"
        );
        assert!(real > 0.0);
    }

    #[test]
    fn inter_unmapped_glyph_falls_back_instead_of_zero() {
        // Not in the Latin subset — must not silently measure 0.
        let w = inter_advance_width("📋", 13.0);
        assert_eq!(w, approx_text_width(13.0, "📋"));
    }

    #[test]
    fn inter_euro_sign_is_covered_by_the_embedded_subset() {
        // The Google Fonts "latin" subset explicitly includes U+20AC.
        let flat = approx_text_width(13.0, "€");
        let real = inter_advance_width("€", 13.0);
        assert!(real > 0.0);
        assert_ne!(
            real, flat,
            "€ should resolve to a real glyph advance, not the flat fallback"
        );
    }

    #[test]
    fn helvetica_ascii_word_measures_below_flat_estimate() {
        let flat = approx_text_width(13.0, "mortgage");
        let real = helvetica_advance_width("mortgage", 13.0);
        assert!(real < flat * 0.8);
        assert!(real > 0.0);
    }

    #[test]
    fn helvetica_measures_the_encoded_euro_glyph() {
        let w = helvetica_advance_width("€", 13.0);
        assert!((w - 0.556 * 13.0).abs() < 1e-9);
    }

    #[test]
    fn inter_base64_decodes_to_the_embedded_ttf() {
        use base64::Engine as _;

        let decoded = STANDARD
            .decode(inter_base64())
            .expect("inter_base64 must be standard-alphabet base64");
        assert_eq!(decoded, INTER_TTF);
    }
}
