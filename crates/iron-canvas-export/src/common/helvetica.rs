//! Helvetica faces and WinAnsi advances used by PDF measurement and painting.
//!
//! The tables derive from Adobe's Core 14 AFMs. Oblique advances equal regular
//! advances; bold-oblique advances equal bold advances. Source hashes, license,
//! and conversion details are in `assets/HELVETICA-NOTICE.txt`.

use super::escape::pdf_text_byte;
#[cfg(feature = "pdf")]
use super::font::bold_italic;

#[cfg(feature = "pdf")]
#[derive(Clone, Copy)]
pub(crate) enum HelveticaFace {
    Regular,
    Bold,
    Oblique,
    BoldOblique,
}

#[cfg(feature = "pdf")]
impl HelveticaFace {
    pub(crate) fn from_css(font_css: &str) -> Self {
        match bold_italic(font_css) {
            (false, false) => Self::Regular,
            (true, false) => Self::Bold,
            (false, true) => Self::Oblique,
            (true, true) => Self::BoldOblique,
        }
    }

    pub(crate) fn resource_name(self) -> &'static str {
        match self {
            Self::Regular => "F1",
            Self::Bold => "F2",
            Self::Oblique => "F3",
            Self::BoldOblique => "F4",
        }
    }

    /// AFM font bounds include accents and descenders outside Ascender/Descender.
    /// Use these bounds so a fitted row contains the selected face's glyphs.
    pub(crate) fn font_metrics(self, size: f64) -> iron_canvas_core::painter::FontMetrics {
        let (top, bottom) = match self {
            Self::Regular | Self::Oblique => (931.0, 225.0),
            Self::Bold | Self::BoldOblique => (962.0, 228.0),
        };
        iron_canvas_core::painter::FontMetrics {
            ascent: top / 1000.0 * size,
            descent: bottom / 1000.0 * size,
            line_gap: 0.0,
        }
    }

    pub(crate) fn advance_width(self, text: &str, size_px: f64) -> f64 {
        let widths = match self {
            Self::Regular | Self::Oblique => &REGULAR_WIDTHS,
            Self::Bold | Self::BoldOblique => &BOLD_WIDTHS,
        };
        advance_width(text, size_px, widths)
    }
}

pub(crate) fn regular_advance_width(text: &str, size_px: f64) -> f64 {
    advance_width(text, size_px, &REGULAR_WIDTHS)
}

fn advance_width(text: &str, size_px: f64, widths: &[u16; 256]) -> f64 {
    text.chars()
        .map(|ch| f64::from(widths[usize::from(pdf_text_byte(ch))]) / 1000.0 * size_px)
        .sum()
}

// Adobe AFM advances in 1000-unit ems, indexed by WinAnsi byte.
// Unassigned codes and control codes have zero width; byte 0x7f is bullet.
#[rustfmt::skip]
const REGULAR_WIDTHS: [u16; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, 350,
    556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0,
    0, 222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667,
    278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333,
    400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611,
    667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
    556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];

#[rustfmt::skip]
#[cfg(feature = "pdf")]
const BOLD_WIDTHS: [u16; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584, 350,
    556, 0, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0,
    0, 278, 278, 500, 500, 350, 556, 1000, 333, 1000, 556, 333, 944, 0, 500, 667,
    278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333,
    400, 584, 333, 333, 333, 611, 556, 278, 333, 333, 365, 556, 834, 834, 834, 611,
    722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
    556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278, 278,
    611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];
