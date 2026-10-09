use iron_canvas_core::painter::{TextAlign, TextMetrics, approx_text_width};
use iron_canvas_core::{
    PixelRect, Point, TextLayoutOptions, TextVerticalAlign, layout_text_into,
    layout_text_lines_into,
};

struct FixedMetrics;

impl TextMetrics for FixedMetrics {
    fn measure_text_width(&self, text: &str, _font_css: &str) -> f64 {
        text.chars().count() as f64 * 8.0
    }
}

/// A backend with no host text metric at all, so every width falls back to the
/// estimate.
struct MeasurelessMetrics;

impl TextMetrics for MeasurelessMetrics {
    fn measure_text_width(&self, _text: &str, _font_css: &str) -> f64 {
        0.0
    }
}

/// A backend with no width must fall back to the estimate `approx_text_width`
/// gives. `"aa éé"` is 5 characters and 7 bytes: the estimate is 60px and fits
/// the 70px available, a byte count gives 84px and wraps.
#[test]
fn measureless_fallback_uses_the_shared_character_estimate() {
    let mut lines = Vec::new();
    layout_text_lines_into(
        &MeasurelessMetrics,
        "12px sans-serif",
        "aa éé",
        true,
        70.0,
        12.0,
        &mut lines,
        &mut String::new(),
    );

    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["aa éé"],
        "a byte-based fallback would wrap this line"
    );
    assert_eq!(lines[0].width, approx_text_width(12.0, "aa éé"));
}

#[test]
fn public_layout_api_wraps_and_positions_resolved_text() {
    let bounds = PixelRect {
        top_left: Point { x: 10, y: 20 },
        width: 50,
        height: 50,
    };
    let mut lines = Vec::new();
    let needs_clip = layout_text_into(
        &FixedMetrics,
        "12px sans-serif",
        "one two",
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
        },
        &mut lines,
        &mut String::new(),
    );

    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["one ", "two"]
    );
    assert_eq!(lines[0].center_x, 30.0);
    assert!(!needs_clip);
}
