#![cfg(target_arch = "wasm32")]

//! Check cached setters against browser state and pixels after restoration.

use iron_canvas_canvas2d::CanvasPainter;
use iron_canvas_core::{
    PixelRect, Point,
    painter::{PaintColor, Painter, TextAlign, TextBaseline},
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

wasm_bindgen_test_configure!(run_in_browser);

fn context() -> CanvasRenderingContext2d {
    let canvas = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into::<HtmlCanvasElement>()
        .unwrap();
    canvas.set_width(8);
    canvas.set_height(8);
    canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap()
}

fn rect() -> PixelRect {
    PixelRect {
        top_left: Point::default(),
        width: 8,
        height: 8,
    }
}

fn pixel(ctx: &CanvasRenderingContext2d) -> Vec<u8> {
    ctx.get_image_data(2.0, 2.0, 1.0, 1.0).unwrap().data().0
}

#[wasm_bindgen_test]
fn colors_distinguish_static_prefixes_and_changed_borrowed_values() {
    let ctx = context();
    let painter = CanvasPainter::new(ctx.clone());
    let full: &'static str = "#123456";
    let short: &'static str = &full[..4];
    for (color, expected) in [
        (full, [0x12, 0x34, 0x56, 255]),
        (short, [0x11, 0x22, 0x33, 255]),
        (full, [0x12, 0x34, 0x56, 255]),
    ] {
        painter.rect_fill(rect(), PaintColor::Static(color));
        assert_eq!(pixel(&ctx), expected);
    }
    let mut color = String::from(full);
    painter.rect_fill(rect(), PaintColor::Borrowed(&color));
    assert_eq!(pixel(&ctx), [0x12, 0x34, 0x56, 255]);
    color.clear();
    color.push_str("#abcdef");
    painter.rect_fill(rect(), PaintColor::Borrowed(&color));
    assert_eq!(pixel(&ctx), [0xab, 0xcd, 0xef, 255]);

    // A gradient replaces the context fill without storing a string.
    // The same color after the gradient needs a new context binding.
    painter.rect_fill_hgradient(
        rect(),
        PaintColor::Static("red"),
        PaintColor::Static("blue"),
    );
    painter.rect_fill(rect(), PaintColor::Borrowed(&color));
    assert_eq!(pixel(&ctx), [0xab, 0xcd, 0xef, 255]);
}

fn paint_state(painter: &CanvasPainter, color: &str, font: &str) {
    painter.rect_fill(rect(), PaintColor::Borrowed(color));
    painter.rect_stroke(rect(), PaintColor::Borrowed(color), 1.0);
    painter.fill_text(
        "",
        0.0,
        0.0,
        PaintColor::Borrowed(font),
        PaintColor::Borrowed(color),
        TextAlign::Start,
        TextBaseline::Top,
    );
}

fn assert_green_state(ctx: &CanvasRenderingContext2d) {
    assert_eq!(ctx.fill_style().as_string().unwrap(), "#00ff00");
    assert_eq!(ctx.stroke_style().as_string().unwrap(), "#00ff00");
    assert_eq!(ctx.font(), "30px serif");
    assert_eq!(pixel(ctx), [0, 255, 0, 255]);
}

#[wasm_bindgen_test]
fn restored_context_rebinds_fill_stroke_and_font() {
    let ctx = context();
    let painter = CanvasPainter::new(ctx.clone());
    paint_state(&painter, "#ff0000", "10px serif");
    painter.push_clip(rect());
    paint_state(&painter, "#0000ff", "20px serif");
    painter.push_clip(rect());
    paint_state(&painter, "#00ff00", "30px serif");

    // Each restore changes browser state behind all three cached strings.
    // Request the last strings again to detect a missed invalidation.
    for _ in 0..2 {
        painter.pop_clip();
        paint_state(&painter, "#00ff00", "30px serif");
        assert_green_state(&ctx);
    }

    ctx.set_fill_style_str("red");
    ctx.set_stroke_style_str("red");
    ctx.set_font("10px serif");
    painter.invalidate_cache();
    paint_state(&painter, "#00ff00", "30px serif");
    assert_green_state(&ctx);
}
