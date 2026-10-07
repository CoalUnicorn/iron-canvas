//! Backend-independent text wrapping and placement for grid paint.
//!
//! The caller resolves spreadsheet-specific font and alignment rules before
//! building [`TextLayoutOptions`]. This module only consumes resolved paint
//! inputs and backend text metrics; it has no workbook or cell-style concepts.

use crate::geometry::pixel_rect::PixelRect;
use crate::painter::{TextAlign, TextMetrics, approx_text_width};

/// Single-line spacing relative to the font's em size.
pub const LINE_HEIGHT_FACTOR: f64 = 1.2;

/// Horizontal inset applied at each edge of the available text rectangle.
pub const CELL_PADDING: f64 = 4.0;

/// Vertical inset between the available rectangle edge and a top/bottom line.
pub const TEXT_V_INSET_PX: f64 = 4.0;

/// Resolved vertical direction for generic text placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextVerticalAlign {
    Top,
    Center,
    Bottom,
}

/// A measured and positioned line, ready for a drawing backend.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    pub text: String,
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
}

/// Resolved inputs for wrapping, positioning, and overflow detection.
///
/// `bounds` controls wrapping and alignment. `clip` may be a smaller visible
/// fragment (for example when a spanning cell is partly outside the viewport).
/// The font, alignments, and rectangles are all supplied by the caller so the
/// helper does not decide workbook policy.
#[derive(Debug, Clone, Copy)]
pub struct TextLayoutOptions {
    pub bounds: PixelRect,
    pub clip: PixelRect,
    pub font_size_px: f64,
    pub horizontal: TextAlign,
    pub vertical: TextVerticalAlign,
    pub wrap: bool,
    pub horizontal_padding: f64,
    pub vertical_inset: f64,
    pub line_height_factor: f64,
    /// Advance per character when the metrics backend reports no width, in
    /// logical pixels: the font size [`approx_text_width`] takes.
    pub approx_char_width: f64,
}

/// Lay out text into reusable storage and report whether clipping is required.
///
/// Newline-separated paragraphs are always retained. When wrapping is enabled,
/// words are moved to the next line as needed; an individual word wider than
/// the available width remains intact and is clipped by the caller.
///
/// Existing line strings are overwritten in place so callers can reuse their
/// allocations across cells and frames. `scratch` is likewise reused while
/// assembling wrapped lines.
pub fn layout_text_into(
    metrics: &dyn TextMetrics,
    font_css: &str,
    text: &str,
    options: TextLayoutOptions,
    out: &mut Vec<TextLine>,
    scratch: &mut String,
) -> bool {
    let usable_w = f64::from(options.bounds.width) - 2.0 * options.horizontal_padding;
    layout_text_lines_into(
        metrics,
        font_css,
        text,
        options.wrap,
        usable_w,
        options.approx_char_width,
        out,
        scratch,
    );

    let line_height = options.font_size_px * options.line_height_factor;
    position_lines(
        out,
        options.horizontal,
        options.vertical,
        options.bounds,
        options.font_size_px,
        line_height,
        options.horizontal_padding,
        options.vertical_inset,
    );
    lines_escape_clip(out, usable_w, options.clip, line_height)
}

/// Split and wrap text while reusing both output line strings and scratch
/// storage. This lower-level entry point is useful to callers that need line
/// counts or widths without positioning paint output.
#[allow(clippy::too_many_arguments)]
pub fn layout_text_lines_into(
    metrics: &dyn TextMetrics,
    font_css: &str,
    text: &str,
    wrap: bool,
    usable_w: f64,
    approx_char_w: f64,
    out: &mut Vec<TextLine>,
    wrap_buf: &mut String,
) {
    layout_lines_into(
        metrics,
        font_css,
        text,
        wrap,
        usable_w,
        approx_char_w,
        out,
        wrap_buf,
    );
}

#[allow(clippy::too_many_arguments)]
fn position_lines(
    lines: &mut [TextLine],
    h_align: TextAlign,
    v_align: TextVerticalAlign,
    rect: PixelRect,
    size_px: f64,
    line_height: f64,
    horizontal_padding: f64,
    vertical_inset: f64,
) {
    let right = rect.right();
    let bottom = rect.bottom();
    let center = rect.center();
    let line_count = lines.len() as f64;
    for (i, line) in lines.iter_mut().enumerate() {
        let i_f = i as f64;
        let tw = line.width;
        line.center_x = match h_align {
            TextAlign::End => f64::from(right) - horizontal_padding - tw / 2.0,
            TextAlign::Center => f64::from(center.x),
            TextAlign::Start => f64::from(rect.left()) + horizontal_padding + tw / 2.0,
        };
        line.center_y = match v_align {
            TextVerticalAlign::Bottom => {
                f64::from(bottom) - size_px / 2.0 - vertical_inset
                    + (i_f - line_count + 1.0) * line_height
            }
            TextVerticalAlign::Center => {
                f64::from(center.y) + (i_f + (1.0 - line_count) / 2.0) * line_height
            }
            TextVerticalAlign::Top => {
                f64::from(rect.top()) + size_px / 2.0 + vertical_inset + i_f * line_height
            }
        };
    }
}

fn lines_escape_clip(lines: &[TextLine], usable_w: f64, rect: PixelRect, line_height: f64) -> bool {
    let half_line = line_height / 2.0;
    let left = f64::from(rect.left());
    let right = f64::from(rect.right());
    let top = f64::from(rect.top());
    let bottom = f64::from(rect.bottom());
    lines.iter().any(|line| {
        line.width > usable_w
            || line.center_x - line.width / 2.0 < left
            || line.center_x + line.width / 2.0 > right
            || line.center_y - half_line < top
            || line.center_y + half_line > bottom
    })
}

#[allow(clippy::too_many_arguments)]
fn layout_lines_into(
    metrics: &dyn TextMetrics,
    font_css: &str,
    text: &str,
    wrap: bool,
    usable_w: f64,
    approx_char_w: f64,
    out: &mut Vec<TextLine>,
    wrap_buf: &mut String,
) {
    // Keep each line's String allocation when a later cell has fewer lines.
    let mut idx = 0usize;
    // Fall back to the estimate the paint backends use, so wrap matches paint.
    let measure = |s: &str| -> f64 {
        let width = metrics.measure_text_width(s, font_css);
        if width > 0.0 {
            width
        } else {
            approx_text_width(approx_char_w, s)
        }
    };

    if !wrap || usable_w <= 0.0 {
        for line in text.split('\n') {
            let width = measure(line);
            write_line(out, &mut idx, line, width);
        }
        out.truncate(idx);
        return;
    }

    let space_w = measure(" ");
    for raw_line in text.split('\n') {
        wrap_buf.clear();
        let mut current_w = 0.0;
        for word in raw_line.split_whitespace() {
            let word_w = measure(word);
            let separator_w = if wrap_buf.is_empty() { 0.0 } else { space_w };
            let tentative_w = current_w + separator_w + word_w;

            if tentative_w > usable_w && !wrap_buf.is_empty() {
                write_line(out, &mut idx, wrap_buf, current_w);
                wrap_buf.clear();
                wrap_buf.push_str(word);
                current_w = word_w;
            } else {
                if !wrap_buf.is_empty() {
                    wrap_buf.push(' ');
                }
                wrap_buf.push_str(word);
                current_w = tentative_w;
            }
        }
        write_line(out, &mut idx, wrap_buf, current_w);
    }
    out.truncate(idx);
}

fn write_line(out: &mut Vec<TextLine>, idx: &mut usize, text: &str, width: f64) {
    if let Some(slot) = out.get_mut(*idx) {
        slot.text.clear();
        slot.text.push_str(text);
        slot.center_x = 0.0;
        slot.center_y = 0.0;
        slot.width = width;
    } else {
        out.push(TextLine {
            text: text.to_owned(),
            center_x: 0.0,
            center_y: 0.0,
            width,
        });
    }
    *idx += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedMetrics;

    impl TextMetrics for FixedMetrics {
        fn measure_text_width(&self, text: &str, _font_css: &str) -> f64 {
            text.chars().count() as f64 * 10.0
        }
    }

    fn options(vertical: TextVerticalAlign) -> TextLayoutOptions {
        TextLayoutOptions {
            bounds: PixelRect {
                top_left: crate::geometry::prim::Point { x: 0, y: 0 },
                width: 60,
                height: 60,
            },
            clip: PixelRect {
                top_left: crate::geometry::prim::Point { x: 0, y: 0 },
                width: 60,
                height: 60,
            },
            font_size_px: 12.0,
            horizontal: TextAlign::Start,
            vertical,
            wrap: true,
            horizontal_padding: CELL_PADDING,
            vertical_inset: TEXT_V_INSET_PX,
            line_height_factor: LINE_HEIGHT_FACTOR,
            approx_char_width: 12.0,
        }
    }

    #[test]
    fn wraps_words_and_preserves_explicit_newlines() {
        let mut lines = Vec::new();
        let clipped = layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "one two\nthree",
            options(TextVerticalAlign::Top),
            &mut lines,
            &mut String::new(),
        );

        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
        assert!(!clipped);
    }

    #[test]
    fn reports_clip_when_one_word_exceeds_the_available_width() {
        let mut lines = Vec::new();
        let clipped = layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "unbroken",
            options(TextVerticalAlign::Top),
            &mut lines,
            &mut String::new(),
        );

        assert!(clipped);
        assert_eq!(lines[0].text, "unbroken");
    }

    #[test]
    fn positions_centered_text_from_resolved_alignment() {
        let mut layout = options(TextVerticalAlign::Center);
        layout.bounds.top_left = crate::geometry::prim::Point { x: 10, y: 20 };
        layout.bounds.width = 100;
        layout.clip = layout.bounds;
        layout.horizontal = TextAlign::Center;
        let mut lines = Vec::new();
        layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "one",
            layout,
            &mut lines,
            &mut String::new(),
        );

        assert_eq!(lines[0].center_x, 60.0);
        assert_eq!(lines[0].center_y, 50.0);
    }

    #[test]
    fn clips_text_that_fits_bounds_but_escapes_a_visible_fragment() {
        let mut layout = options(TextVerticalAlign::Top);
        layout.bounds.width = 100;
        layout.clip.top_left.x = 30;
        layout.clip.width = 20;
        let clipped = layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "one",
            layout,
            &mut Vec::new(),
            &mut String::new(),
        );

        assert!(clipped);
    }

    #[test]
    fn reuses_line_string_allocations_when_line_count_shrinks() {
        let mut lines = Vec::new();
        let mut scratch = String::new();
        let options = options(TextVerticalAlign::Top);
        layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "one two three",
            options,
            &mut lines,
            &mut scratch,
        );
        let capacity = lines[0].text.capacity();
        layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "x",
            options,
            &mut lines,
            &mut scratch,
        );

        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.capacity() >= capacity);
    }

    #[test]
    fn bottom_aligned_block_that_crosses_clip_boundary_requires_clipping() {
        let mut layout = options(TextVerticalAlign::Bottom);
        layout.bounds.height = 20;
        layout.clip.height = 10;
        let clipped = layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "one\ntwo",
            layout,
            &mut Vec::new(),
            &mut String::new(),
        );

        assert!(clipped);
    }
}
