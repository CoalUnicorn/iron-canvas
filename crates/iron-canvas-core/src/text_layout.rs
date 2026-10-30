//! Backend-independent text wrapping and placement for grid paint.
//!
//! The caller resolves spreadsheet-specific font and alignment rules before
//! building [`TextLayoutOptions`]. This module only consumes resolved paint
//! inputs and backend text metrics; it has no workbook or cell-style concepts.

use crate::geometry::pixel_rect::PixelRect;
use crate::painter::{TextAlign, TextBaseline, TextMetrics, approx_text_width};
use unicode_linebreak::{BreakClass, break_property, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

/// Single-line spacing relative to the font's em size.
pub const LINE_HEIGHT_FACTOR: f64 = 1.2;

/// Horizontal inset applied at each edge of the available text rectangle.
pub const CELL_PADDING: f64 = 4.0;

/// Vertical inset between the available rectangle edge and a top/bottom line.
pub const TEXT_V_INSET_PX: f64 = 4.0;

/// Shared line box for placement, baseline selection, and auto-fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLineMetrics {
    pub height: f64,
    pub advance: f64,
    pub baseline: TextBaseline,
    pub baseline_offset: f64,
}

impl TextLineMetrics {
    pub fn resolve(metrics: &dyn TextMetrics, font: &str, size: f64, factor: f64) -> Self {
        if let Some(font) = metrics.font_metrics(font).filter(|font| font.is_valid()) {
            let height = font.ascent + font.descent;
            Self {
                height,
                advance: height + font.line_gap,
                baseline: TextBaseline::Alphabetic,
                baseline_offset: (font.ascent - font.descent) / 2.0,
            }
        } else {
            Self {
                height: size,
                advance: size * factor,
                baseline: TextBaseline::Middle,
                baseline_offset: 0.0,
            }
        }
    }

    /// Include the font box once and the baseline spacing between lines.
    pub fn block_height(self, lines: usize, inset: f64) -> f64 {
        if lines == 0 {
            0.0
        } else {
            self.height + (lines - 1) as f64 * self.advance + 2.0 * inset
        }
    }
}

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
    /// Advance per character when the metrics backend reports an invalid width, in
    /// logical pixels: the font size [`approx_text_width`] takes.
    pub approx_char_width: f64,
}

/// Lay out text into reusable storage and report whether clipping is required.
///
/// Newline-separated paragraphs are always retained. When wrapping is enabled,
/// Unicode break positions take priority. Long tokens break between grapheme
/// clusters. Spaces remain in the output. Protected punctuation and nonbreaking
/// glue can form a group wider than the available width; the caller clips it.
///
/// Existing line strings reuse their storage across cells and frames.
/// `scratch` remains available for source compatibility.
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

    position_text_lines(
        out,
        options,
        TextLineMetrics::resolve(
            metrics,
            font_css,
            options.font_size_px,
            options.line_height_factor,
        ),
    )
}

/// Apply placement after line breaking, including after a cache lookup.
pub(crate) fn position_text_lines(
    out: &mut [TextLine],
    options: TextLayoutOptions,
    metrics: TextLineMetrics,
) -> bool {
    let usable_w = f64::from(options.bounds.width) - 2.0 * options.horizontal_padding;
    position_lines(out, options, metrics);
    lines_escape_clip(out, usable_w, options.clip, metrics.height)
}

/// Split and wrap text while reusing both output line strings and scratch
/// storage. This lower-level entry point is useful to callers that need line
/// counts or widths without positioning paint output.
// Keep this public scratch input until the next API change.
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
    // Preserve the public API's existing effect while the private line breaker
    // no longer needs this string for storage.
    wrap_buf.clear();
    layout_lines_into(metrics, font_css, text, wrap, usable_w, approx_char_w, out);
}

fn position_lines(lines: &mut [TextLine], options: TextLayoutOptions, metrics: TextLineMetrics) {
    let rect = options.bounds;
    let font_box_height = metrics.height;
    let line_spacing = metrics.advance;
    let right = rect.right();
    let bottom = rect.bottom();
    let center_x = f64::from(rect.left()) + f64::from(rect.width) / 2.0;
    let center_y = f64::from(rect.top()) + f64::from(rect.height) / 2.0;
    let line_count = lines.len() as f64;
    for (i, line) in lines.iter_mut().enumerate() {
        let i_f = i as f64;
        let tw = line.width;
        line.center_x = match options.horizontal {
            TextAlign::End => f64::from(right) - options.horizontal_padding - tw / 2.0,
            TextAlign::Center => center_x,
            TextAlign::Start => f64::from(rect.left()) + options.horizontal_padding + tw / 2.0,
        };
        line.center_y = match options.vertical {
            TextVerticalAlign::Bottom => {
                f64::from(bottom) - font_box_height / 2.0 - options.vertical_inset
                    + (i_f - line_count + 1.0) * line_spacing
            }
            TextVerticalAlign::Center => center_y + (i_f + (1.0 - line_count) / 2.0) * line_spacing,
            TextVerticalAlign::Top => {
                f64::from(rect.top())
                    + font_box_height / 2.0
                    + options.vertical_inset
                    + i_f * line_spacing
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

/// Split mandatory separators, including CRLF as one separator. Keep empty lines.
pub fn text_paragraphs(text: &str) -> impl Iterator<Item = &str> {
    let mut remaining = Some(text);
    std::iter::from_fn(move || {
        let text = remaining.take()?;
        for (index, ch) in text.char_indices() {
            if matches!(
                break_property(ch as u32),
                BreakClass::Mandatory
                    | BreakClass::CarriageReturn
                    | BreakClass::LineFeed
                    | BreakClass::NextLine
            ) {
                let mut end = index + ch.len_utf8();
                if ch == '\r' && text[end..].starts_with('\n') {
                    end += 1;
                }
                remaining = Some(&text[end..]);
                return Some(&text[..index]);
            }
        }
        Some(text)
    })
}

// Emergency breaks relax word boundaries, but keep punctuation and glue intact.
// Inspect whole graphemes so a combining mark cannot hide the base character.
fn emergency_break(left: &str, right: &str) -> bool {
    use BreakClass::*;
    let glued = |ch: char| matches!(break_property(ch as u32), NonBreakingGlue | WordJoiner);
    !left.chars().chain(right.chars()).any(glued)
        && !left
            .chars()
            .any(|ch| matches!(break_property(ch as u32), OpenPunctuation | Quotation))
        && !right.chars().any(|ch| {
            matches!(
                break_property(ch as u32),
                ClosePunctuation
                    | CloseParenthesis
                    | Exclamation
                    | Inseparable
                    | NonStarter
                    | ConditionalJapaneseStarter
                    | InfixSeparator
                    | Symbol
                    | Hyphen
                    | After
                    | Quotation
            )
        })
}

fn layout_lines_into(
    metrics: &dyn TextMetrics,
    font_css: &str,
    text: &str,
    wrap: bool,
    usable_w: f64,
    approx_char_w: f64,
    out: &mut Vec<TextLine>,
) {
    let mut idx = 0usize;
    let measure = |s: &str| -> f64 {
        let width = metrics.measure_text_width(s, font_css);
        if width.is_finite() && width >= 0.0 {
            width
        } else {
            approx_text_width(approx_char_w, s)
        }
    };

    for paragraph in text_paragraphs(text) {
        let width = measure(paragraph);
        if !wrap || usable_w <= 0.0 || width <= usable_w || paragraph.is_empty() {
            write_line(out, &mut idx, paragraph, width);
            continue;
        }

        // Intersect Unicode breaks with grapheme boundaries. Keep emergency
        // boundaries too, so an overlong URL does not require a glyph split.
        let graphemes: Vec<_> = paragraph.grapheme_indices(true).collect();
        let mut normal = linebreaks(paragraph).peekable();
        let mut boundaries = Vec::new();
        for (i, &(start, grapheme)) in graphemes.iter().enumerate() {
            let end = start + grapheme.len();
            while normal.peek().is_some_and(|&(index, _)| index < end) {
                normal.next();
            }
            let allowed = normal.peek().is_some_and(|&(index, _)| index == end);
            let emergency = graphemes
                .get(i + 1)
                .is_none_or(|&(_, next)| emergency_break(grapheme, next));
            if allowed || emergency {
                boundaries.push((end, allowed));
            }
        }

        let mut start = 0;
        let mut first_boundary = 0;
        let mut fit: Option<(usize, f64)> = None;
        for normal_index in (0..boundaries.len()).filter(|&i| boundaries[i].1) {
            let end = boundaries[normal_index].0;
            let mut width = measure(&paragraph[start..end]);
            if width > usable_w
                && let Some((index, previous_width)) = fit.take()
            {
                let previous_end = boundaries[index].0;
                write_line(
                    out,
                    &mut idx,
                    &paragraph[start..previous_end],
                    previous_width,
                );
                start = previous_end;
                first_boundary = index + 1;
                width = measure(&paragraph[start..end]);
            }
            if width <= usable_w {
                fit = Some((normal_index, width));
                continue;
            }

            // The complete token does not fit. Only now inspect its emergency
            // breaks. Measuring a partial word first could miss a narrower
            // complete run after kerning or ligature shaping.
            let mut cursor = first_boundary;
            while cursor <= normal_index {
                let mut emergency_fit = None;
                let chosen = loop {
                    let end = boundaries[cursor].0;
                    let width = measure(&paragraph[start..end]);
                    if width > usable_w {
                        break emergency_fit.unwrap_or((cursor, width));
                    }
                    emergency_fit = Some((cursor, width));
                    if cursor == normal_index {
                        break (cursor, width);
                    }
                    cursor += 1;
                };
                if chosen.0 == normal_index {
                    fit = Some(chosen);
                    break;
                }
                let end = boundaries[chosen.0].0;
                write_line(out, &mut idx, &paragraph[start..end], chosen.1);
                start = end;
                first_boundary = chosen.0 + 1;
                cursor = first_boundary;
            }
        }
        if let Some((index, width)) = fit {
            write_line(out, &mut idx, &paragraph[start..boundaries[index].0], width);
        }
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
            ["one ", "two", "three"]
        );
        assert!(!clipped);
    }

    #[test]
    fn wrapping_preserves_leading_repeated_and_trailing_whitespace() {
        let mut lines = Vec::new();
        layout_text_lines_into(
            &FixedMetrics,
            "12px sans-serif",
            "  a  b  ",
            true,
            52.0,
            12.0,
            &mut lines,
            &mut String::new(),
        );

        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["  a  ", "b  "]
        );
    }

    #[test]
    fn wrapping_keeps_nonbreaking_spaces_within_words() {
        let mut lines = Vec::new();
        layout_text_lines_into(
            &FixedMetrics,
            "12px sans-serif",
            "a\u{00a0}b",
            true,
            20.0,
            12.0,
            &mut lines,
            &mut String::new(),
        );

        assert_eq!(lines[0].text, "a\u{00a0}b");
    }

    #[test]
    fn wraps_a_long_word_between_graphemes() {
        let mut lines = Vec::new();
        let clipped = layout_text_into(
            &FixedMetrics,
            "12px sans-serif",
            "unbroken",
            options(TextVerticalAlign::Top),
            &mut lines,
            &mut String::new(),
        );

        assert!(!clipped);
        assert_eq!(lines[0].text, "unbro");
        assert_eq!(lines[1].text, "ken");
    }

    fn wrapped(metrics: &dyn TextMetrics, text: &str, width: f64) -> Vec<TextLine> {
        let mut lines = Vec::new();
        layout_text_lines_into(
            metrics,
            "12px test",
            text,
            true,
            width,
            10.0,
            &mut lines,
            &mut String::new(),
        );
        lines
    }

    #[test]
    fn hard_separators_keep_empty_paragraphs_and_crlf_is_one_break() {
        assert_eq!(
            text_paragraphs("\r\na\r\nb\rc\u{85}d\u{2028}e\u{2029}\n").collect::<Vec<_>>(),
            ["", "a", "b", "c", "d", "e", "", ""]
        );
    }

    #[test]
    fn complete_run_measurements_control_breaks_and_stored_widths() {
        struct Shaped;
        impl TextMetrics for Shaped {
            fn measure_text_width(&self, text: &str, _: &str) -> f64 {
                // The full run is narrower than its independently shaped words.
                if text == "AV fi " {
                    35.0
                } else {
                    text.len() as f64 * 10.0
                }
            }
        }
        let lines = wrapped(&Shaped, "AV fi zzzzz", 40.0);
        assert_eq!(lines[0].text, "AV fi ");
        assert_eq!(lines[0].width, 35.0);
        for line in lines {
            assert_eq!(
                line.width,
                Shaped.measure_text_width(&line.text, "12px test")
            );
        }
    }

    #[test]
    fn long_urls_hyphens_and_cjk_use_safe_breaks() {
        for (text, width, expected) in [
            ("abcd-efgh", 50.0, vec!["abcd-", "efgh"]),
            ("漢字、漢字。", 20.0, vec!["漢", "字、", "漢", "字。"]),
            ("（漢字）", 20.0, vec!["（漢", "字）"]),
        ] {
            let lines = wrapped(&FixedMetrics, text, width);
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
        let url = "https://example.com/verylongpath?query=abcdef";
        let lines = wrapped(&FixedMetrics, url, 80.0);
        assert!(lines.len() > 1);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            url
        );
        assert!(lines.iter().all(|line| line.width <= 80.0));
    }

    #[test]
    fn emergency_breaks_preserve_combining_marks_emoji_and_joiners() {
        struct GraphemeMetrics;
        impl TextMetrics for GraphemeMetrics {
            fn measure_text_width(&self, text: &str, _: &str) -> f64 {
                text.graphemes(true).count() as f64 * 10.0
            }
        }
        for text in ["a\u{301}b\u{301}", "👩‍💻👨‍👩‍👧‍👦", "🇬🇧🇫🇷", "👍🏽👍🏻", "क्षक्ष"]
        {
            let lines = wrapped(&GraphemeMetrics, text, 10.0);
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                text.graphemes(true).collect::<Vec<_>>()
            );
        }
        for text in [
            "a\u{a0}b",
            "a\u{2007}b",
            "a\u{202f}b",
            "a\u{2060}b",
            "（字）",
        ] {
            let lines = wrapped(&FixedMetrics, text, 1.0);
            assert_eq!(lines.len(), 1, "{text}");
            assert_eq!(lines[0].text, text);
            assert!(lines[0].width > 1.0);
        }
    }

    #[test]
    fn zero_width_is_valid_but_invalid_widths_use_the_estimate() {
        struct Width(f64);
        impl TextMetrics for Width {
            fn measure_text_width(&self, _: &str, _: &str) -> f64 {
                self.0
            }
        }
        assert_eq!(wrapped(&Width(0.0), "\u{200b}", 10.0)[0].width, 0.0);
        for width in [-1.0, f64::NAN, f64::INFINITY] {
            let lines = wrapped(&Width(width), "ab", 10.0);
            assert_eq!(lines.len(), 2);
            assert!(lines.iter().all(|line| line.width == 10.0));
        }
    }

    #[test]
    fn font_extents_control_alignment_baselines_and_fitted_height() {
        use crate::painter::FontMetrics;
        struct Vertical(Option<FontMetrics>);
        impl TextMetrics for Vertical {
            fn measure_text_width(&self, _: &str, _: &str) -> f64 {
                10.0
            }
            fn font_metrics(&self, _: &str) -> Option<FontMetrics> {
                self.0
            }
        }
        let metrics = Vertical(Some(FontMetrics {
            ascent: 13.0,
            descent: 5.0,
            line_gap: 3.0,
        }));
        let line = TextLineMetrics::resolve(&metrics, "12px test", 12.0, 1.2);
        assert_eq!(line.block_height(2, 4.0), 47.0);
        assert_eq!(line.baseline_offset, 4.0);
        assert_eq!(line.baseline, TextBaseline::Alphabetic);
        for (align, first) in [
            (TextVerticalAlign::Top, 13.0),
            (TextVerticalAlign::Center, 19.5),
            (TextVerticalAlign::Bottom, 26.0),
        ] {
            let mut lines = Vec::new();
            assert!(!layout_text_into(
                &metrics,
                "12px test",
                "a\nb",
                options(align),
                &mut lines,
                &mut String::new()
            ));
            assert_eq!(lines[0].center_y, first);
            assert_eq!(lines[1].center_y, first + 21.0);
        }
        for invalid in [
            None,
            Some(FontMetrics {
                ascent: f64::NAN,
                descent: 5.0,
                line_gap: 0.0,
            }),
            Some(FontMetrics {
                ascent: 13.0,
                descent: -1.0,
                line_gap: 0.0,
            }),
        ] {
            let fallback = TextLineMetrics::resolve(&Vertical(invalid), "12px test", 12.0, 1.2);
            assert_eq!(fallback.baseline, TextBaseline::Middle);
            assert_eq!(fallback.height, 12.0);
            assert_eq!(fallback.advance, 12.0 * 1.2);
        }
    }

    #[test]
    fn positions_centered_text_from_resolved_alignment() {
        let mut layout = options(TextVerticalAlign::Center);
        layout.bounds.width = 101;
        layout.bounds.height = 61;
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

        assert_eq!(lines[0].center_x, 50.5);
        assert_eq!(lines[0].center_y, 30.5);
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
