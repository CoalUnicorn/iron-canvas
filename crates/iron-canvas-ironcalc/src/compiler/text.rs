//! Spreadsheet text policy. Generic layout stays in core.
use iron_canvas_core::painter::{PaintColor, TextAlign, TextBaseline};
use iron_canvas_core::scene::SceneBuilder;
use iron_canvas_core::text_shaper::TextShaper;
use iron_canvas_core::{CanvasTheme, PixelRect, text_layout::*};
use ironcalc_base::{
    UserModel,
    links::CellLinkView,
    types::{CellType, HorizontalAlignment as H, Style, VerticalAlignment as V},
};

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_text(
    out: &mut SceneBuilder,
    workbook: &UserModel<'_>,
    style: &Style,
    kind: &CellType,
    text: &str,
    rect: PixelRect,
    reserved: i32,
    spill_clip: Option<PixelRect>,
    shaper: &mut TextShaper<'_>,
    theme: &CanvasTheme,
    link: Option<&CellLinkView>,
    lines: &mut Vec<TextLine>,
) {
    // A narrow source can still paint into a resolved spill clip. Keep the
    // minimum width only when the text stays inside its source cell.
    if text.is_empty() || (rect.width < 10 && spill_clip.is_none()) || rect.height < 10 {
        return;
    }
    let reserved = reserved.clamp(0, rect.width);
    let rect = PixelRect {
        top_left: iron_canvas_core::Point {
            x: rect.left() + reserved,
            y: rect.top(),
        },
        width: rect.width - reserved,
        height: rect.height,
    };
    if f64::from(rect.width) <= 2.0 * CELL_PADDING && spill_clip.is_none() {
        return;
    }
    let Layout {
        css,
        horizontal,
        size,
        metrics,
        overflows,
    } = layout(shaper, style, kind, text, rect, lines);
    let font = &style.font;
    let resolved = workbook.resolve_color(&font.color);
    // IronCalc keeps the hyperlink theme slot private. It is OOXML slot 10.
    let link_color =
        link.map(|_| workbook.resolve_color(&ironcalc_base::types::Color::Theme(10, 0.0)));
    let color = if matches!(kind, CellType::ErrorValue) {
        &theme.error_text_color
    } else if let Some(color) = link_color.as_deref().filter(|color| !color.is_empty()) {
        color
    } else if font.color.is_none() {
        &theme.default_text_color
    } else {
        resolved.as_str()
    };
    // Keep the source anchor. Only the resolved clip can extend into neighbors.
    // Retained rendering and backend culling use this clip as the ink bound.
    out.push_clip(if overflows {
        spill_clip.unwrap_or(rect)
    } else {
        rect
    });
    for line in lines {
        let x = match horizontal {
            TextAlign::Start => f64::from(rect.left()) + CELL_PADDING,
            TextAlign::End => f64::from(rect.right()) - CELL_PADDING,
            TextAlign::Center => line.center_x,
        };
        out.fill_text(
            &line.text,
            x,
            line.center_y + metrics.baseline_offset,
            PaintColor::Borrowed(&css),
            PaintColor::Borrowed(color),
            horizontal,
            metrics.baseline,
        );
        let x1 = line.center_x - line.width / 2.0;
        let x2 = line.center_x + line.width / 2.0;
        if font.u || link.is_some() {
            // The measured font box does not share the old middle baseline.
            // Place the underline halfway into the descent. The metric
            // interface has no font-specific underline position.
            let underline_y = if metrics.baseline == TextBaseline::Alphabetic {
                let descent = metrics.height / 2.0 - metrics.baseline_offset;
                line.center_y + metrics.baseline_offset + (descent / 2.0).max(1.0)
            } else {
                line.center_y + (size * 0.35).max(2.0)
            };
            out.stroke_text_hline(x1, x2, underline_y, PaintColor::Borrowed(color), 1.0);
        }
        if font.strike {
            out.stroke_text_hline(x1, x2, line.center_y, PaintColor::Borrowed(color), 1.0);
        }
    }
    out.pop_clip();
}

pub(super) struct Layout {
    css: String,
    horizontal: TextAlign,
    size: f64,
    metrics: TextLineMetrics,
    pub overflows: bool,
}

/// Use identical placement for spill planning and scene emission.
pub(super) fn layout(
    shaper: &mut TextShaper<'_>,
    style: &Style,
    kind: &CellType,
    text: &str,
    rect: PixelRect,
    lines: &mut Vec<TextLine>,
) -> Layout {
    let alignment = style.alignment.as_ref();
    let horizontal = match alignment.map(|a| &a.horizontal) {
        Some(H::Right) => TextAlign::End,
        Some(H::Center | H::CenterContinuous) => TextAlign::Center,
        None | Some(H::General) => {
            if matches!(kind, CellType::Number) {
                TextAlign::End
            } else {
                TextAlign::Start
            }
        }
        Some(H::Left | H::Fill | H::Justify | H::Distributed) => TextAlign::Start,
    };
    let vertical = match alignment.map(|a| &a.vertical) {
        None | Some(V::Bottom) => TextVerticalAlign::Bottom,
        Some(V::Center) => TextVerticalAlign::Center,
        Some(V::Top | V::Justify | V::Distributed) => TextVerticalAlign::Top,
    };
    let font = &style.font;
    let size = f64::from(font.sz);
    let css = font_css(style);
    let metrics = TextLineMetrics::resolve(shaper.metrics(), &css, size, LINE_HEIGHT_FACTOR);
    let overflows = shaper.layout_text_into(
        &css,
        text,
        TextLayoutOptions {
            bounds: rect,
            clip: rect,
            font_size_px: size,
            horizontal,
            vertical,
            wrap: alignment.is_some_and(|a| a.wrap_text),
            horizontal_padding: CELL_PADDING,
            vertical_inset: TEXT_V_INSET_PX,
            line_height_factor: LINE_HEIGHT_FACTOR,
            approx_char_width: size,
        },
        lines,
    );
    Layout {
        css,
        horizontal,
        size,
        metrics,
        overflows,
    }
}

/// `ctx.font` string for a cell style, matching the text pass.
pub(crate) fn font_css(style: &Style) -> String {
    let font = &style.font;
    let size = f64::from(font.sz);
    let family = font.name.trim();
    let family = if family.is_empty() { "Calibri" } else { family };
    let family = family.replace(['"', '\\', '\n', '\r'], "");
    format!(
        "{}{}{size}px \"{family}\"",
        if font.b { "bold " } else { "" },
        if font.i { "italic " } else { "" }
    )
}
