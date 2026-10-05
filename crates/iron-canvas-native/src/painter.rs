use cairo::{Context, ImageSurface};
use iron_canvas_core::{
    Line, PixelRect, Span,
    geometry::path::{Path, PathCmd},
    painter::*,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

pub struct NativePainter {
    context: Context,
    error: Cell<Option<cairo::Error>>,
    widths: RefCell<HashMap<String, HashMap<String, f64>>>,
    width_count: Cell<usize>,
}
impl NativePainter {
    pub fn new(surface: &ImageSurface) -> Result<Self, cairo::Error> {
        Ok(Self {
            context: Context::new(surface)?,
            error: Cell::new(None),
            widths: RefCell::default(),
            width_count: Cell::new(0),
        })
    }
    pub(crate) fn context(&self) -> &Context {
        &self.context
    }
    pub(crate) fn clear_measure_cache(&self) {
        self.widths.borrow_mut().clear();
        self.width_count.set(0);
    }
    pub fn status(&self) -> Result<(), cairo::Error> {
        self.error.get().map_or_else(|| self.context.status(), Err)
    }
    fn record(&self, result: Result<(), cairo::Error>) {
        if let Err(e) = result
            && self.error.get().is_none()
        {
            self.error.set(Some(e));
        }
    }
    fn source(&self, color: PaintColor<'_>) {
        let (r, g, b, a) = rgba(color.as_str());
        self.context.set_source_rgba(r, g, b, a);
    }
    fn rectangle(&self, rect: PixelRect) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.context.rectangle(x, y, w, h);
    }
    fn layout(&self, text: &str, css: &str) -> pango::Layout {
        let layout = pangocairo::functions::create_layout(&self.context);
        let mut font = pango::FontDescription::new();
        let size = parse_font_size_px(css);
        let family = css
            .find("px")
            .map(|i| css[i + 2..].trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
            .unwrap_or("sans-serif");
        font.set_family(family);
        font.set_absolute_size(size * f64::from(pango::SCALE));
        if css.split_whitespace().any(|v| v == "bold") {
            font.set_weight(pango::Weight::Bold);
        }
        if css.split_whitespace().any(|v| v == "italic") {
            font.set_style(pango::Style::Italic);
        }
        layout.set_font_description(Some(&font));
        layout.set_text(text);
        layout
    }
    fn path(&self, path: &Path<'_>) {
        self.context.new_path();
        for cmd in path.cmds() {
            match *cmd {
                PathCmd::Move(p) => self.context.move_to(p.x, p.y),
                PathCmd::Line(p) => self.context.line_to(p.x, p.y),
                PathCmd::Quad(c, p) => {
                    if let Ok((x, y)) = self.context.current_point() {
                        self.context.curve_to(
                            x + 2.0 * (c.x - x) / 3.0,
                            y + 2.0 * (c.y - y) / 3.0,
                            p.x + 2.0 * (c.x - p.x) / 3.0,
                            p.y + 2.0 * (c.y - p.y) / 3.0,
                            p.x,
                            p.y,
                        );
                    }
                }
                PathCmd::Cubic(a, b, p) => self.context.curve_to(a.x, a.y, b.x, b.y, p.x, p.y),
                PathCmd::Arc(a) => {
                    if a.sweep_angle >= 0.0 {
                        self.context.arc(
                            a.center.x,
                            a.center.y,
                            a.radius,
                            a.start_angle,
                            a.start_angle + a.sweep_angle,
                        );
                    } else {
                        self.context.arc_negative(
                            a.center.x,
                            a.center.y,
                            a.radius,
                            a.start_angle,
                            a.start_angle + a.sweep_angle,
                        );
                    }
                }
                PathCmd::Close => self.context.close_path(),
            }
        }
    }
    fn line(&self, x1: f64, y1: f64, x2: f64, y2: f64, color: PaintColor<'_>, width: f64) {
        self.context.new_path();
        self.source(color);
        self.context.set_line_width(width);
        self.context.set_dash(&[], 0.0);
        self.context.set_line_cap(cairo::LineCap::Butt);
        self.context.move_to(x1, y1);
        self.context.line_to(x2, y2);
        self.record(self.context.stroke());
    }
}
impl TextMetrics for NativePainter {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        if let Some(width) = self
            .widths
            .borrow()
            .get(font_css)
            .and_then(|font| font.get(text))
        {
            return *width;
        }
        let width = f64::from(self.layout(text, font_css).size().0) / f64::from(pango::SCALE);
        if self.width_count.get() >= 4096 {
            self.clear_measure_cache();
        }
        self.widths
            .borrow_mut()
            .entry(font_css.to_owned())
            .or_default()
            .insert(text.to_owned(), width);
        self.width_count.set(self.width_count.get() + 1);
        width
    }
}
impl Painter for NativePainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.context.new_path();
        self.rectangle(rect);
        self.source(color);
        self.record(self.context.fill());
    }
    fn rect_fill_hgradient(&self, rect: PixelRect, from: PaintColor, to: PaintColor) {
        if rect.width <= 0 || rect.height <= 0 {
            return;
        }
        let gradient =
            cairo::LinearGradient::new(f64::from(rect.left()), 0.0, f64::from(rect.right()), 0.0);
        for (offset, color) in [(0.0, from), (1.0, to)] {
            let (r, g, b, a) = rgba(color.as_str());
            gradient.add_color_stop_rgba(offset, r, g, b, a);
        }
        self.record(self.context.set_source(&gradient));
        self.context.new_path();
        self.rectangle(rect);
        self.record(self.context.fill());
    }
    fn fill_path(&self, path: &Path<'_>, color: PaintColor) {
        if path.validate().is_err() {
            return;
        }
        self.path(path);
        self.source(color);
        self.record(self.context.fill());
    }
    fn stroke_path(&self, path: &Path<'_>, color: PaintColor, style: &StrokeStyle<'_>) {
        if path.validate().is_err() || !style.is_valid() {
            return;
        }
        self.record(self.context.save());
        self.path(path);
        self.source(color);
        self.context.set_line_width(style.width);
        self.context.set_miter_limit(style.miter_limit);
        self.context.set_dash(style.dash, 0.0);
        self.context.set_line_cap(match style.cap {
            LineCap::Butt => cairo::LineCap::Butt,
            LineCap::Round => cairo::LineCap::Round,
            LineCap::Square => cairo::LineCap::Square,
        });
        self.context.set_line_join(match style.join {
            LineJoin::Miter => cairo::LineJoin::Miter,
            LineJoin::Round => cairo::LineJoin::Round,
            LineJoin::Bevel => cairo::LineJoin::Bevel,
        });
        self.record(self.context.stroke());
        self.record(self.context.restore());
    }
    fn fill_circle(&self, cx: f64, cy: f64, radius: f64, color: PaintColor) {
        if radius <= 0.0 {
            return;
        }
        self.context.new_path();
        self.context.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
        self.source(color);
        self.record(self.context.fill());
    }
    fn clear_rect(&self, rect: PixelRect) {
        self.record(self.context.save());
        self.context.set_operator(cairo::Operator::Clear);
        self.context.new_path();
        self.rectangle(rect);
        self.record(self.context.fill());
        self.record(self.context.restore());
    }
    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.context.new_path();
        self.rectangle(rect);
        self.source(color);
        self.context.set_line_width(width);
        self.context.set_dash(&[], 0.0);
        self.record(self.context.stroke());
    }
    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.record(self.context.save());
        self.context.new_path();
        self.rectangle(rect);
        self.source(color);
        self.context.set_line_width(width);
        self.context.set_dash(&[4.0, 3.0], 0.0);
        self.record(self.context.stroke());
        self.record(self.context.restore());
    }
    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        match line {
            Line::H { span, y } => self.stroke_hline(span, f64::from(y), color, width),
            Line::V { x, span } => self.stroke_vline(f64::from(x), span, color, width),
        }
    }
    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        self.line(f64::from(span.from), y, f64::from(span.to), y, color, width);
    }
    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        self.line(x, f64::from(span.from), x, f64::from(span.to), color, width);
    }
    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.line(x1, y, x2, y, color, width);
    }
    fn push_clip(&self, rect: PixelRect) {
        self.record(self.context.save());
        self.context.new_path();
        self.rectangle(rect);
        self.context.clip();
    }
    fn pop_clip(&self) {
        self.record(self.context.restore());
    }
    fn fill_text(
        &self,
        text: &str,
        x: f64,
        y: f64,
        font_css: PaintColor,
        color: PaintColor,
        align: TextAlign,
        baseline: TextBaseline,
    ) {
        let layout = self.layout(text, font_css.as_str());
        let (w, h) = layout.size();
        let w = f64::from(w) / f64::from(pango::SCALE);
        let h = f64::from(h) / f64::from(pango::SCALE);
        let x = x - match align {
            TextAlign::Start => 0.0,
            TextAlign::Center => w / 2.0,
            TextAlign::End => w,
        };
        let y = y - match baseline {
            TextBaseline::Top => 0.0,
            TextBaseline::Middle => h / 2.0,
            TextBaseline::Bottom => h,
            TextBaseline::Alphabetic => f64::from(layout.baseline()) / f64::from(pango::SCALE),
        };
        self.source(color);
        self.context.move_to(x, y);
        pangocairo::functions::show_layout(&self.context, &layout);
    }
    fn invalidate_cache(&self) {}
    fn apply_dpr_transform(&self, dpr: f64) {
        self.context.identity_matrix();
        self.context.scale(dpr, dpr);
    }
    fn reset_text_defaults(&self) {}
    fn begin_group(&self, _: GroupClass) {}
    fn end_group(&self) {}
}
fn rgba(css: &str) -> (f64, f64, f64, f64) {
    let css = css.trim();
    let parse = || -> Option<(f64, f64, f64, f64)> {
        if let Some(hex) = css.strip_prefix('#') {
            let byte = |i| {
                u8::from_str_radix(hex.get(i..i + 2)?, 16)
                    .ok()
                    .map(|v| f64::from(v) / 255.0)
            };
            return match hex.len() {
                3 | 4 => {
                    let digit = |i| {
                        u8::from_str_radix(hex.get(i..i + 1)?, 16)
                            .ok()
                            .map(|v| f64::from(v) / 15.0)
                    };
                    Some((
                        digit(0)?,
                        digit(1)?,
                        digit(2)?,
                        if hex.len() == 4 { digit(3)? } else { 1.0 },
                    ))
                }
                6 | 8 => Some((
                    byte(0)?,
                    byte(2)?,
                    byte(4)?,
                    if hex.len() == 8 { byte(6)? } else { 1.0 },
                )),
                _ => None,
            };
        }
        if let Some(inner) = css
            .strip_prefix("rgba(")
            .or_else(|| css.strip_prefix("rgb("))
            .and_then(|s| s.strip_suffix(')'))
        {
            let values = inner
                .split(',')
                .map(|s| s.trim().parse::<f64>().ok())
                .collect::<Option<Vec<_>>>()?;
            if !(3..=4).contains(&values.len()) || values.iter().any(|v| !v.is_finite()) {
                return None;
            }
            return Some((
                values[0] / 255.0,
                values[1] / 255.0,
                values[2] / 255.0,
                values.get(3).copied().unwrap_or(1.0),
            ));
        }
        match css.to_ascii_lowercase().as_str() {
            "white" => Some((1.0, 1.0, 1.0, 1.0)),
            "transparent" => Some((0.0, 0.0, 0.0, 0.0)),
            _ => None,
        }
    };
    parse().unwrap_or((0.0, 0.0, 0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metric_cache_keeps_font_identity_and_clears_after_invalidation() {
        let surface = ImageSurface::create(cairo::Format::ARgb32, 1, 1).unwrap();
        let painter = NativePainter::new(&surface).unwrap();
        for font in [
            "12px sans-serif",
            "bold 24px serif",
            "italic 18px monospace",
        ] {
            let expected =
                f64::from(painter.layout("Wide text", font).size().0) / f64::from(pango::SCALE);
            assert_eq!(painter.measure_text_width("Wide text", font), expected);
            assert_eq!(painter.measure_text_width("Wide text", font), expected);
        }
        painter.clear_measure_cache();
        assert!(painter.widths.borrow().is_empty());
        assert_eq!(painter.width_count.get(), 0);
        assert!(painter.measure_text_width("Wide text", "12px sans-serif") > 0.0);
    }
}
