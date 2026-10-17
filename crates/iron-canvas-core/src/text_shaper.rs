//! Cached line layout with the existing break policy.
//!
//! A shaper borrows one backend's metrics for an operation. The cache can live
//! longer, but its metric key changes whenever measurement results can change.
//! Width prediction and font shaping remain outside this stage.

use std::{collections::VecDeque, rc::Rc};

use crate::{
    painter::TextMetrics,
    text_layout::{TextLayoutOptions, TextLine, layout_text_lines_into, position_text_lines},
};

/// Host identity for a text metric environment and its font generation.
///
/// Equal keys promise equal text measurements and font resolution across all
/// sessions that share a cache. Use separate profiles for different backends
/// unless their measurements are equal. Advance the generation after a font
/// change. Use checked counters; never wrap or saturate either value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextMetricsKey {
    profile: u64,
    generation: u64,
}

impl TextMetricsKey {
    pub const fn new(profile: u64, generation: u64) -> Self {
        Self {
            profile,
            generation,
        }
    }
}

#[derive(Debug)]
struct Entry {
    font: String,
    text: String,
    width: u64,
    fallback: u64,
    wrap: bool,
    lines: Rc<[TextLine]>,
    bytes: usize,
}

/// Least recently used (LRU) storage for lines before placement.
///
/// Limits apply to entry count and retained entry data. The byte count includes
/// strings, line records, entry records, and reference counters. It excludes
/// allocator overhead and spare queue capacity. Returned lines can outlive the
/// cache; the caller owns that storage after removal.
#[derive(Debug)]
pub struct TextLayoutCache {
    capacity: usize,
    byte_capacity: usize,
    bytes: usize,
    metrics_key: Option<TextMetricsKey>,
    entries: VecDeque<Entry>,
}

impl Default for TextLayoutCache {
    fn default() -> Self {
        Self::new(500, 1024 * 1024)
    }
}

impl TextLayoutCache {
    pub fn new(capacity: usize, byte_capacity: usize) -> Self {
        Self {
            capacity,
            byte_capacity,
            bytes: 0,
            metrics_key: None,
            entries: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.metrics_key = None;
    }
}

/// Measure and cache lines with the current backend.
///
/// This stage preserves the existing wrapping algorithm, including its summed
/// piece widths. It does not claim shaping or kerning equivalence with paint.
pub struct TextShaper<'a> {
    metrics: &'a dyn TextMetrics,
    cache: &'a mut TextLayoutCache,
}

impl<'a> TextShaper<'a> {
    /// Bind storage to measurements. A different key removes all old entries.
    pub fn new(
        metrics: &'a dyn TextMetrics,
        key: TextMetricsKey,
        cache: &'a mut TextLayoutCache,
    ) -> Self {
        if cache.metrics_key != Some(key) {
            cache.clear();
            cache.metrics_key = Some(key);
        }
        Self { metrics, cache }
    }

    pub fn metrics(&self) -> &dyn TextMetrics {
        self.metrics
    }

    /// Return immutable lines with zero positions. Widths use logical pixels.
    pub fn break_lines(
        &mut self,
        font_css: &str,
        text: &str,
        width: f64,
        wrap: bool,
        fallback_width: f64,
    ) -> Rc<[TextLine]> {
        // Compare the exact inputs. No float rounding can change a wrap limit.
        let hit = self.cache.entries.iter().rposition(|entry| {
            entry.width == width.to_bits()
                && entry.fallback == fallback_width.to_bits()
                && entry.wrap == wrap
                && entry.font == font_css
                && entry.text == text
        });
        if let Some(index) = hit {
            let entry = self.cache.entries.remove(index).expect("entry exists");
            let lines = Rc::clone(&entry.lines);
            self.cache.entries.push_back(entry);
            return lines;
        }

        // Use one break algorithm for cached and uncached callers. A miss does
        // not train a predictor or change the result of a later operation.
        let mut lines = Vec::new();
        layout_text_lines_into(
            self.metrics,
            font_css,
            text,
            wrap,
            width,
            fallback_width,
            &mut lines,
            &mut String::new(),
        );
        let lines: Rc<[TextLine]> = lines.into();
        if self.cache.capacity == 0 || self.cache.byte_capacity == 0 {
            return lines;
        }
        let font = font_css.to_owned();
        let text = text.to_owned();
        let bytes = size_of::<Entry>()
            + 2 * size_of::<usize>()
            + font.capacity()
            + text.capacity()
            + size_of_val(lines.as_ref())
            + lines.iter().map(|line| line.text.capacity()).sum::<usize>();
        // Large text still produces a complete result. It does not displace
        // useful small entries when it cannot fit within the byte limit.
        if bytes > self.cache.byte_capacity {
            return lines;
        }
        while self.cache.entries.len() >= self.cache.capacity
            || self.cache.bytes > self.cache.byte_capacity - bytes
        {
            if let Some(entry) = self.cache.entries.pop_front() {
                self.cache.bytes -= entry.bytes;
            }
        }
        self.cache.entries.push_back(Entry {
            font,
            text,
            width: width.to_bits(),
            fallback: fallback_width.to_bits(),
            wrap,
            lines: Rc::clone(&lines),
            bytes,
        });
        self.cache.bytes += bytes;
        lines
    }

    /// Apply the current position, alignment, and clip after cache lookup.
    pub fn layout_text_into(
        &mut self,
        font_css: &str,
        text: &str,
        options: TextLayoutOptions,
        out: &mut Vec<TextLine>,
    ) -> bool {
        let lines = self.break_lines(
            font_css,
            text,
            f64::from(options.bounds.width) - 2.0 * options.horizontal_padding,
            options.wrap,
            options.approx_char_width,
        );
        // Keep caller allocations while protecting cached lines from placement.
        for (index, line) in lines.iter().enumerate() {
            if let Some(slot) = out.get_mut(index) {
                slot.text.clone_from(&line.text);
                slot.width = line.width;
            } else {
                out.push(line.clone());
            }
        }
        out.truncate(lines.len());
        position_text_lines(out, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PixelRect, Point,
        painter::TextAlign,
        text_layout::{TextVerticalAlign, layout_text_into},
    };
    use std::cell::Cell;

    struct Metrics {
        width: f64,
        calls: Cell<usize>,
    }

    impl TextMetrics for Metrics {
        fn measure_text_width(&self, text: &str, _: &str) -> f64 {
            self.calls.set(self.calls.get() + 1);
            text.chars().count() as f64 * self.width
        }
    }

    const KEY: TextMetricsKey = TextMetricsKey::new(1, 0);

    fn options() -> TextLayoutOptions {
        let bounds = PixelRect {
            top_left: Point { x: 0, y: 0 },
            width: 61,
            height: 81,
        };
        TextLayoutOptions {
            bounds,
            clip: bounds,
            font_size_px: 12.0,
            horizontal: TextAlign::Start,
            vertical: TextVerticalAlign::Top,
            wrap: true,
            horizontal_padding: 4.0,
            vertical_inset: 4.0,
            line_height_factor: 1.2,
            approx_char_width: 12.0,
        }
    }

    #[test]
    fn cold_and_warm_layout_match_the_existing_break_policy() {
        for width in [0.0, 10.0] {
            let metrics = Metrics {
                width,
                calls: Cell::new(0),
            };
            let mut cache = TextLayoutCache::default();
            let mut shaper = TextShaper::new(&metrics, KEY, &mut cache);
            for text in [
                "",
                "one two\nthree\n",
                "  a  b  ",
                "a\u{a0}b",
                "長い文字列",
                "a\u{301} 👩‍💻",
                "https://example.com/long",
            ] {
                for wrap in [false, true] {
                    let mut options = options();
                    options.wrap = wrap;
                    let mut expected = Vec::new();
                    let overflow = layout_text_into(
                        &metrics,
                        "12px test",
                        text,
                        options,
                        &mut expected,
                        &mut String::new(),
                    );
                    let mut actual = Vec::new();
                    assert_eq!(
                        shaper.layout_text_into("12px test", text, options, &mut actual),
                        overflow
                    );
                    assert_eq!(actual, expected);
                    let calls = metrics.calls.get();
                    // Placement from a previous call cannot alter cached lines.
                    assert_eq!(
                        shaper.layout_text_into("12px test", text, options, &mut actual),
                        overflow
                    );
                    assert_eq!(actual, expected);
                    assert_eq!(metrics.calls.get(), calls);
                }
            }
        }
    }

    #[test]
    fn placement_and_clip_apply_after_lookup() {
        let metrics = Metrics {
            width: 10.0,
            calls: Cell::new(0),
        };
        let mut cache = TextLayoutCache::default();
        let mut shaper = TextShaper::new(&metrics, KEY, &mut cache);
        let mut out = Vec::new();
        shaper.layout_text_into("12px test", "one two", options(), &mut out);
        for horizontal in [TextAlign::Start, TextAlign::Center, TextAlign::End] {
            for vertical in [
                TextVerticalAlign::Top,
                TextVerticalAlign::Center,
                TextVerticalAlign::Bottom,
            ] {
                let mut options = options();
                options.bounds.top_left = Point { x: 13, y: 25 };
                options.bounds.height = 103;
                // Equal usable width with a different inset still reuses lines.
                options.bounds.width += 2;
                options.horizontal_padding += 1.0;
                options.horizontal = horizontal;
                options.vertical = vertical;
                options.font_size_px = 18.0;
                options.line_height_factor = 1.5;
                options.vertical_inset = 2.0;
                let mut expected = Vec::new();
                let overflow = layout_text_into(
                    &metrics,
                    "12px test",
                    "one two",
                    options,
                    &mut expected,
                    &mut String::new(),
                );
                let calls = metrics.calls.get();
                assert_eq!(
                    shaper.layout_text_into("12px test", "one two", options, &mut out),
                    overflow
                );
                assert_eq!(out, expected);
                assert_eq!(metrics.calls.get(), calls);
            }
        }
    }

    #[test]
    fn every_break_input_separates_entries() {
        let metrics = Metrics {
            width: 10.0,
            calls: Cell::new(0),
        };
        for (font, text, width, wrap, fallback) in [
            ("bold 12px test", "one two", 53.0, true, 12.0),
            ("12px test", "one three", 53.0, true, 12.0),
            ("12px test", "one two", 53.00000000000001, true, 12.0),
            ("12px test", "one two", 53.0, false, 12.0),
            ("12px test", "one two", 53.0, true, 13.0),
        ] {
            let mut cache = TextLayoutCache::default();
            let mut shaper = TextShaper::new(&metrics, KEY, &mut cache);
            shaper.break_lines("12px test", "one two", 53.0, true, 12.0);
            let calls = metrics.calls.get();
            shaper.break_lines(font, text, width, wrap, fallback);
            assert!(metrics.calls.get() > calls);
            let calls = metrics.calls.get();
            shaper.break_lines(font, text, width, wrap, fallback);
            assert_eq!(metrics.calls.get(), calls);
        }
    }

    #[test]
    fn changed_metric_profile_or_generation_removes_old_lines() {
        let mut cache = TextLayoutCache::default();
        let narrow = Metrics {
            width: 4.0,
            calls: Cell::new(0),
        };
        let wide = Metrics {
            width: 10.0,
            calls: Cell::new(0),
        };
        for key in [TextMetricsKey::new(2, 0), TextMetricsKey::new(1, 1)] {
            let a = TextShaper::new(&narrow, KEY, &mut cache)
                .break_lines("font", "one two", 35.0, true, 12.0);
            let b = TextShaper::new(&wide, key, &mut cache)
                .break_lines("font", "one two", 35.0, true, 12.0);
            assert_ne!(a, b);
            assert_eq!(cache.len(), 1);
        }
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.retained_bytes(), 0);
    }

    #[test]
    fn hits_refresh_use_order_and_eviction_preserves_results() {
        let metrics = Metrics {
            width: 10.0,
            calls: Cell::new(0),
        };
        let mut cache = TextLayoutCache::new(2, 4096);
        let mut shaper = TextShaper::new(&metrics, KEY, &mut cache);
        let a = shaper.break_lines("font", "a", 50.0, true, 12.0);
        let b = shaper.break_lines("font", "b", 50.0, true, 12.0);
        assert!(Rc::ptr_eq(
            &a,
            &shaper.break_lines("font", "a", 50.0, true, 12.0)
        ));
        shaper.break_lines("font", "c", 50.0, true, 12.0);
        assert!(Rc::ptr_eq(
            &a,
            &shaper.break_lines("font", "a", 50.0, true, 12.0)
        ));
        let calls = metrics.calls.get();
        let again = shaper.break_lines("font", "b", 50.0, true, 12.0);
        assert!(metrics.calls.get() > calls);
        assert_eq!(again, b);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn byte_limit_evicts_entries_and_bypasses_large_results() {
        let metrics = Metrics {
            width: 10.0,
            calls: Cell::new(0),
        };
        let mut cache = TextLayoutCache::default();
        TextShaper::new(&metrics, KEY, &mut cache).break_lines("font", "a", 50.0, true, 12.0);
        let entry_bytes = cache.retained_bytes();
        let mut cache = TextLayoutCache::new(10, entry_bytes);
        for text in ["a", "b", "c"] {
            TextShaper::new(&metrics, KEY, &mut cache).break_lines("font", text, 50.0, true, 12.0);
            assert_eq!(cache.len(), 1);
            assert_eq!(cache.retained_bytes(), entry_bytes);
        }
        let mut shaper = TextShaper::new(&metrics, KEY, &mut cache);
        let c = shaper.break_lines("font", "c", 50.0, true, 12.0);
        let large = "long".repeat(1024);
        let result = shaper.break_lines("font", &large, 50.0, true, 12.0);
        assert_eq!(result[0].text, large);
        assert!(Rc::ptr_eq(
            &c,
            &shaper.break_lines("font", "c", 50.0, true, 12.0)
        ));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.retained_bytes(), entry_bytes);
    }

    #[test]
    fn zero_limits_disable_retention() {
        let metrics = Metrics {
            width: 10.0,
            calls: Cell::new(0),
        };
        for (count, bytes) in [(0, 4096), (10, 0)] {
            let mut cache = TextLayoutCache::new(count, bytes);
            let mut shaper = TextShaper::new(&metrics, KEY, &mut cache);
            let a = shaper.break_lines("font", "text", 50.0, true, 12.0);
            let calls = metrics.calls.get();
            let b = shaper.break_lines("font", "text", 50.0, true, 12.0);
            assert_eq!(a, b);
            assert!(metrics.calls.get() > calls);
            assert!(cache.is_empty());
            assert_eq!(cache.retained_bytes(), 0);
        }
    }
}
