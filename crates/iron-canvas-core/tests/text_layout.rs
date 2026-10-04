use iron_canvas_core::painter::{TextAlign, TextMetrics};
use iron_canvas_core::{PixelRect, Point, TextLayoutOptions, TextVerticalAlign, layout_text_into};

struct FixedMetrics;

impl TextMetrics for FixedMetrics {
    fn measure_text_width(&self, text: &str, _font_css: &str) -> f64 {
        text.chars().count() as f64 * 8.0
    }
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
        ["one", "two"]
    );
    assert_eq!(lines[0].center_x, 26.0);
    assert!(!needs_clip);
}
