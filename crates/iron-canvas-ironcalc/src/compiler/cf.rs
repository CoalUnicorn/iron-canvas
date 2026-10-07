//! IronCalc CF placement policy, lowered to generic scene primitives.
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
use std::rc::Rc;

use iron_canvas_core::geometry::constants::DEFAULT_MITER_LIMIT;
use iron_canvas_core::geometry::path::{ArcSpec, Path, PathCmd, PointF};
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
use iron_canvas_core::painter::{LineCap, LineJoin, PaintColor, StrokeStyle};
use iron_canvas_core::scene::SceneBuilder;
use ironcalc_base::{
    UserModel,
    cf_types::{ExtendedStyle, Icon},
};

use super::glyphs::{self as cf, GlyphDef, GlyphPart};
use iron_canvas_core::shape::place::{Placement, emit_poly, star_vertices};

/// Resolved icon decoration: the engine-selected glyph and its resolved,
/// interned color.
#[derive(Debug, Clone, PartialEq)]
pub struct CfIconPaint {
    pub glyph: Icon,
    pub color: Rc<str>,
    /// When false, the painted cell value is hidden (the model value, the
    /// formula bar, and editing are unaffected).
    pub show_value: bool,
}

/// Resolved data bar: interned positive/negative colors plus the lighter
/// shade each gradient runs from at the axis. `value` and `axis_position` are
/// normalized positions in `[0, 1]`; the bar spans them, so it is not
/// necessarily anchored at the cell's left edge.
#[derive(Debug, Clone, PartialEq)]
pub struct CfDataBarPaint {
    pub positive: Rc<str>,
    pub positive_light: Rc<str>,
    pub negative: Rc<str>,
    pub negative_light: Rc<str>,
    pub is_gradient: bool,
    pub value: f64,
    pub axis_position: f64,
    /// When false, the painted cell value is hidden.
    pub show_value: bool,
}

/// Resolved rating with its draw style and engine counts.
#[derive(Debug, Clone, PartialEq)]
pub struct CfRatingPaint {
    pub style: Icon,
    pub color: Rc<str>,
    /// Rating points the engine selected (`1..=max`).
    pub count: u32,
    /// Rating scale maximum (3, 4, or 5 for the built-in sets).
    pub max: u32,
    /// When false, the painted cell value is hidden.
    pub show_value: bool,
}

/// Resolved CF decoration for one cell. The engine resolves each category
/// independently, so all three may be present; `paint` draws them in
/// data-bar -> icon -> rating order (background to foreground).
#[derive(Debug, Clone, PartialEq)]
pub struct CfDecorationPaint {
    pub icon: Option<CfIconPaint>,
    pub data_bar: Option<CfDataBarPaint>,
    pub rating: Option<CfRatingPaint>,
}

impl CfDecorationPaint {
    /// Resolve the engine's evaluated conditional formatting for one cell.
    ///
    /// Colors resolve through `workbook`, matching what the compiler paints.
    /// Every category is independent, so all three may be present; absent
    /// categories are `None`. The value-hiding flags and normalized bar
    /// positions are the engine's, so a host can read them without
    /// reimplementing the resolution.
    pub fn resolve(deco: &ExtendedStyle, workbook: &UserModel<'_>) -> Self {
        let color = |c: &ironcalc_base::types::Color, fallback: &str| -> Rc<str> {
            let value = workbook.resolve_color(c);
            Rc::from(if value.is_empty() { fallback } else { &value })
        };
        Self {
            icon: deco.icon.as_ref().map(|icon| CfIconPaint {
                glyph: icon.icon.clone(),
                color: color(&icon.color, "#000000"),
                show_value: icon.show_value,
            }),
            data_bar: deco.data_bar.as_ref().map(|bar| {
                let positive = color(&bar.positive_color, "#638ec6");
                let negative = color(&bar.negative_color, "#ff0000");
                CfDataBarPaint {
                    positive_light: light_color(&positive),
                    negative_light: light_color(&negative),
                    positive,
                    negative,
                    is_gradient: bar.is_gradient,
                    value: bar.value.clamp(0.0, 1.0),
                    axis_position: bar.axis_position.clamp(0.0, 1.0),
                    show_value: bar.show_value,
                }
            }),
            rating: deco.rating.as_ref().map(|rating| CfRatingPaint {
                style: rating.icon.clone(),
                color: color(&rating.color, "#000000"),
                count: rating.count,
                max: rating.max,
                show_value: rating.show_value,
            }),
        }
    }

    /// True when any present category hides the painted cell value. The
    /// model value, the formula bar, and editing are deliberately unaffected:
    /// only the canvas text pass consults this.
    pub(crate) fn hides_value(&self) -> bool {
        self.icon.as_ref().is_some_and(|icon| !icon.show_value)
            || self.data_bar.as_ref().is_some_and(|bar| !bar.show_value)
            || self
                .rating
                .as_ref()
                .is_some_and(|rating| !rating.show_value)
    }

    /// Pixels reserved at `rect`'s left edge for the shown indicator, so the
    /// text pass starts after the icon/rating instead of under it. Zero when
    /// nothing is drawn on the left.
    pub(crate) fn reserved_left(&self, rect: PixelRect) -> i32 {
        if self.hides_value() {
            return 0;
        }
        let Some((slot_left, _, size)) = icon_slot(rect) else {
            return 0;
        };
        let offset = slot_left - rect.left();
        let count = self.glyph_count();
        if count == 0 {
            0
        } else {
            offset
                .saturating_add(count.saturating_mul(size))
                .min(rect.width)
        }
    }

    /// Glyph slots the decoration occupies, saturating on overflow.
    fn glyph_count(&self) -> i32 {
        let rating = self
            .rating
            .as_ref()
            .map_or(0, |rating| match &rating.style {
                Icon::Circle => i32::from(rating.max > 0),
                _ => count_as_i32(rating.count),
            });
        i32::from(self.icon.is_some()).saturating_add(rating)
    }

    /// Paint every present decoration over the already-filled cell `rect`,
    /// purely in `Painter` primitives. The renderer owns CF geometry so the
    /// backend stays primitive-only: data bars become a solid or gradient
    /// rects; icons and ratings become path and circle primitives.
    pub(crate) fn paint(&self, painter: &SceneBuilder, rect: PixelRect) {
        // Background to foreground: the bar must not cover the glyphs.
        if let Some(bar) = &self.data_bar {
            paint_data_bar(painter, rect, bar);
        }
        let Some((mut left, top, size)) = icon_slot(rect) else {
            return;
        };
        let inner = rect.inset(CF_INSET, CF_INSET);
        let needs_clip =
            left.saturating_add(self.glyph_count().saturating_mul(size)) > inner.right();
        if needs_clip {
            painter.push_clip(inner);
        }
        if let Some(icon) = &self.icon {
            paint_glyph(painter, icon.glyph.clone(), left, top, size, &icon.color);
            left += size;
        }
        if let Some(rating) = &self.rating {
            match &rating.style {
                Icon::Circle => {
                    if left < inner.right()
                        && let Some(fraction) = circle_rating_fraction(rating.count, rating.max)
                    {
                        paint_circle_rating(painter, left, top, size, fraction, &rating.color);
                    }
                }
                glyph => {
                    let def = rating_def(glyph.clone());
                    for i in 0..count_as_i32(rating.count) {
                        let glyph_left = left.saturating_add(i.saturating_mul(size));
                        if glyph_left >= inner.right() {
                            break;
                        }
                        paint_def(painter, &def, glyph_left, top, size, &rating.color);
                    }
                }
            }
        }
        if needs_clip {
            painter.pop_clip();
        }
    }
}

/// Pixel inset applied to a cell rect before painting a CF decoration, so the
/// decoration never overlaps grid lines or explicit borders.
const CF_INSET: i32 = 2;

/// Width reserved on the left of a decorated cell for an icon-set glyph or a
/// rating. The text layout reserves the same width, so the value never
/// overlaps the indicator.
pub(crate) const CF_ICON_AREA_WIDTH: i32 = 20;

/// Gap between the cell's inner edge and the first icon.
const ICON_LEFT_MARGIN: i32 = 3;

/// How far the gradient's axis-side stop is lightened toward white. Excel's
/// gradient data bars run the same hue from the tip to the axis.
const GRADIENT_LIGHTEN: f64 = 0.6;

fn light_color(color: &str) -> Rc<str> {
    let hex = color.trim_start_matches('#');
    let rgb = if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        std::array::from_fn(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0))
    } else {
        [0, 0, 0]
    };
    let [r, g, b] = lighten(rgb, GRADIENT_LIGHTEN);
    Rc::from(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// Lighten `rgb` toward white by `amount` (0.0 = unchanged, 1.0 = white).
fn lighten(rgb: [u8; 3], amount: f64) -> [u8; 3] {
    std::array::from_fn(|i| {
        let channel = f64::from(rgb[i]);
        (channel + (255.0 - channel) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// Engine count as `i32`, saturating on overflow.
fn count_as_i32(count: u32) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

/// The icon/rating slot inside `rect`: `(left, top, size)` in pixels, or
/// `None` when the cell is too small to hold one.
fn icon_slot(rect: PixelRect) -> Option<(i32, i32, i32)> {
    let inner = rect.inset(CF_INSET, CF_INSET);
    if inner.width <= 0 || inner.height <= 0 {
        return None;
    }
    let size = CF_ICON_AREA_WIDTH.min(inner.height);
    if size <= 0 {
        return None;
    }
    Some((
        inner.left() + ICON_LEFT_MARGIN,
        inner.top() + (inner.height - size) / 2,
        size,
    ))
}

/// Paint a data bar: a solid or horizontal-gradient rect spanning the zero
/// axis and the value endpoint, colored by the endpoint's side of the axis.
fn paint_data_bar(painter: &SceneBuilder, rect: PixelRect, bar: &CfDataBarPaint) {
    let inner = rect.inset(CF_INSET, CF_INSET);
    if inner.width <= 0 || inner.height <= 0 {
        return;
    }
    let pad_y = ((f64::from(inner.height) * 0.15).round() as i32).max(2);
    let bar_height = inner.height - pad_y * 2;
    if bar_height <= 0 {
        return;
    }
    let width = f64::from(inner.width);
    let axis_x = inner.left() + (bar.axis_position * width).round() as i32;
    let value_x = inner.left() + (bar.value * width).round() as i32;
    let positive_side = bar.value >= bar.axis_position;
    let (left, right) = if positive_side {
        (axis_x, value_x)
    } else {
        (value_x, axis_x)
    };
    let bar_width = right - left;
    // Zero-length bar (including a zero value on a split axis): draw nothing.
    if bar_width <= 0 {
        return;
    }
    let bar_rect = PixelRect {
        top_left: Point {
            x: left,
            y: inner.top() + pad_y,
        },
        width: bar_width,
        height: bar_height,
    };
    if bar.is_gradient && bar_width > 1 {
        // Axial side is lighter, the tip keeps the full color.
        let (from, to) = if positive_side {
            (&bar.positive_light, &bar.positive)
        } else {
            (&bar.negative, &bar.negative_light)
        };
        painter.rect_fill_hgradient(
            bar_rect,
            PaintColor::Borrowed(from),
            PaintColor::Borrowed(to),
        );
    } else {
        let solid = if positive_side {
            &bar.positive
        } else {
            &bar.negative
        };
        painter.rect_fill(bar_rect, PaintColor::Borrowed(solid));
    }
}

/// The shared unit-box definition for an icon glyph.
///
/// The exhaustive `Icon` dispatch lives here: it selects a definition
/// from [`shape::cf`]. Icon geometry is backend-neutral vector work: no font,
/// no glyph asset, no browser-only API.
///
/// Crate-visible for the glyph capacity tests.
pub(crate) fn glyph_def(glyph: Icon) -> GlyphDef {
    match glyph {
        Icon::ArrowUp => cf::arrow(-FRAC_PI_2),
        Icon::ArrowRight => cf::arrow(0.0),
        Icon::ArrowDown => cf::arrow(FRAC_PI_2),
        Icon::ArrowAngleUp => cf::arrow(-FRAC_PI_4),
        Icon::ArrowAngleDown => cf::arrow(FRAC_PI_4),
        Icon::Circle => cf::CIRCLE,
        Icon::TriangleUp => cf::triangle(cf::TriangleStyle::Chevron, 0.0),
        Icon::TriangleDown => cf::triangle(cf::TriangleStyle::Chevron, PI),
        Icon::TriangleUpFilled => cf::triangle(cf::TriangleStyle::Filled, 0.0),
        Icon::TriangleDownFilled => cf::triangle(cf::TriangleStyle::Filled, PI),
        Icon::FlatRectangle => cf::FLAT_RECTANGLE,
        Icon::Rhombus => cf::RHOMBUS,
        Icon::Flag => cf::FLAG,
        Icon::Check => cf::CHECK,
        Icon::Cross => cf::CROSS,
        Icon::Exclamation => cf::EXCLAMATION,
        Icon::Star => cf::STAR,
        Icon::Heart => cf::HEART,
        Icon::ThumbsUp => cf::THUMBS_UP,
        Icon::ThumbsDown => cf::THUMBS_DOWN,
    }
}

/// Rating glyph definition. Ratings reuse the icon definitions, except the
/// flat bar: the renderer tiles one copy per rating point, so that glyph uses
/// the padded `RATING_BAR` and neighbouring points stay separate.
fn rating_def(glyph: Icon) -> GlyphDef {
    match glyph {
        Icon::FlatRectangle => cf::RATING_BAR,
        other => glyph_def(other),
    }
}

/// Every `Icon` variant the renderer lowers. The glyph capacity tests use this
/// list, so the two cannot drift. `glyph_def` makes a new variant a build
/// error; this list makes the geometry checks see it.
#[cfg(test)]
pub(crate) const ALL_ICONS: [Icon; 20] = [
    Icon::ArrowUp,
    Icon::ArrowRight,
    Icon::ArrowDown,
    Icon::ArrowAngleUp,
    Icon::ArrowAngleDown,
    Icon::Circle,
    Icon::TriangleUp,
    Icon::TriangleDown,
    Icon::TriangleUpFilled,
    Icon::TriangleDownFilled,
    Icon::FlatRectangle,
    Icon::Rhombus,
    Icon::Flag,
    Icon::Check,
    Icon::Cross,
    Icon::Exclamation,
    Icon::Star,
    Icon::Heart,
    Icon::ThumbsUp,
    Icon::ThumbsDown,
];

/// Map the engine's quarter-circle threshold count to a filled fraction.
///
/// IronCalc counts passed thresholds from zero. Counts zero and one select the
/// lowest, empty-circle tier. Later tiers add one quarter, up to a full circle.
fn circle_rating_fraction(count: u32, max: u32) -> Option<f64> {
    if max == 0 {
        return None;
    }
    if max == 1 {
        return Some(f64::from(count > 0));
    }
    let tier = count.saturating_sub(1).min(max - 1);
    Some(f64::from(tier) / f64::from(max - 1))
}

/// Paint one circle rating as a filled sector inside a stroked circle.
fn paint_circle_rating(
    painter: &SceneBuilder,
    left: i32,
    top: i32,
    size: i32,
    fraction: f64,
    color: &str,
) {
    if size <= 0 {
        return;
    }
    let center = PointF::new(
        f64::from(left) + f64::from(size) / 2.0,
        f64::from(top) + f64::from(size) / 2.0,
    );
    let radius = f64::from(size) * 0.42;
    let start_angle = -FRAC_PI_2;
    let fill = PaintColor::Borrowed(color);
    if fraction >= 1.0 {
        painter.fill_circle(center.x, center.y, radius, fill);
    } else if fraction > 0.0 {
        let arc = ArcSpec::new(
            center,
            radius,
            start_angle,
            std::f64::consts::TAU * fraction,
        );
        let start = arc.start_point();
        let sector = [
            PathCmd::Move(center),
            PathCmd::Line(start),
            PathCmd::Arc(arc),
            PathCmd::Close,
        ];
        painter.fill_path(&Path::new(&sector), fill);
    }

    let outline = ArcSpec::new(center, radius, 0.0, std::f64::consts::TAU);
    let circle = [
        PathCmd::Move(outline.start_point()),
        PathCmd::Arc(outline),
        PathCmd::Close,
    ];
    let style = StrokeStyle {
        width: (f64::from(size) * 0.06).max(1.0),
        cap: LineCap::Butt,
        join: LineJoin::Round,
        miter_limit: DEFAULT_MITER_LIMIT,
        dash: &[],
    };
    painter.stroke_path(&Path::new(&circle), fill, &style);
}

/// Paint one glyph into the `size`×`size` box at `(left, top)`.
///
/// The glyph box is square, so the uniform fit fills it. The definition's
/// rotation turns the placed geometry around the box centre.
fn paint_glyph(painter: &SceneBuilder, glyph: Icon, left: i32, top: i32, size: i32, color: &str) {
    let def = glyph_def(glyph);
    paint_def(painter, &def, left, top, size, color);
}

/// Place `def` into the `size`×`size` box at `(left, top)` and paint each part.
fn paint_def(painter: &SceneBuilder, def: &GlyphDef, left: i32, top: i32, size: i32, color: &str) {
    if size <= 0 {
        return;
    }
    let size = f64::from(size);
    let Some(place) = Placement::fit(f64::from(left), f64::from(top), size, size, def.rotation)
    else {
        return;
    };
    let color = PaintColor::Borrowed(color);
    // One bounded stack buffer, reused by every part: no per-glyph `Vec`.
    let mut buf = [PathCmd::Close; cf::MAX_PART_CMDS];
    for part in def.parts {
        match part {
            GlyphPart::Poly(vertices) => {
                let Some(n) = emit_poly(vertices, &place, &mut buf) else {
                    debug_assert!(false, "a glyph part exceeds MAX_PART_CMDS");
                    continue;
                };
                painter.fill_path(&Path::new(&buf[..n]), color);
            }
            GlyphPart::Segment { from, to, half } => {
                let a = place.point(PointF::new(from.0, from.1));
                let b = place.point(PointF::new(to.0, to.1));
                if (b.x - a.x).hypot(b.y - a.y) <= f64::EPSILON {
                    continue;
                }
                buf[0] = PathCmd::Move(a);
                buf[1] = PathCmd::Line(b);
                let style = StrokeStyle {
                    width: 2.0 * place.length(*half),
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    miter_limit: DEFAULT_MITER_LIMIT,
                    dash: &[],
                };
                painter.stroke_path(&Path::new(&buf[..2]), color, &style);
            }
            GlyphPart::Circle { radius } => {
                let c = place.point(PointF::new(0.5, 0.5));
                painter.fill_circle(c.x, c.y, place.length(*radius), color);
            }
            GlyphPart::Star { inner } => {
                let vertices = star_vertices(*inner);
                let Some(n) = emit_poly(&vertices, &place, &mut buf) else {
                    debug_assert!(false, "the star exceeds MAX_PART_CMDS");
                    continue;
                };
                painter.fill_path(&Path::new(&buf[..n]), color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::scene::Primitive;
    use ironcalc_base::{
        cf_types::{CfDataBar, CfIcon, CfRating},
        types::{Color, Style},
    };
    fn rect(width: i32) -> PixelRect {
        PixelRect {
            top_left: Point { x: 0, y: 0 },
            width,
            height: 24,
        }
    }
    fn model() -> UserModel<'static> {
        UserModel::new_empty("cf", "en", "UTC", "en").unwrap()
    }
    fn extended() -> ExtendedStyle {
        ExtendedStyle {
            style: Style::default(),
            icon: None,
            data_bar: None,
            rating: None,
        }
    }
    #[test]
    fn combined_categories_keep_order_hide_value_and_clip_indicators() {
        let mut ext = extended();
        ext.icon = Some(CfIcon {
            icon: Icon::ArrowUp,
            color: Color::Rgb("#112233".into()),
            show_value: true,
        });
        ext.data_bar = Some(CfDataBar {
            positive_color: Color::Rgb("#3366cc".into()),
            negative_color: Color::Rgb("#ff0000".into()),
            is_gradient: true,
            value: 0.0,
            axis_position: 0.5,
            show_value: false,
        });
        ext.rating = Some(CfRating {
            icon: Icon::Star,
            count: 3,
            max: 5,
            color: Color::Rgb("#ffd700".into()),
            show_value: true,
        });
        let cf = CfDecorationPaint::resolve(&ext, &model());
        assert!(cf.hides_value());
        let builder = SceneBuilder::default();
        cf.paint(&builder, rect(30));
        let ops = builder.finish();
        assert!(
            matches!(&ops[0], Primitive::Gradient { from, to, .. } if &**from == "#ff0000" && &**to == "#ff9999")
        );
        assert!(matches!(&ops[1], Primitive::Clip { .. }));
        assert!(matches!(ops.last(), Some(Primitive::EndClip)));
        ext.data_bar.as_mut().unwrap().show_value = true;
        let cf = CfDecorationPaint::resolve(&ext, &model());
        assert!(!cf.hides_value());
        assert_eq!(cf.reserved_left(rect(30)), 30);
    }
    #[test]
    fn every_engine_icon_lowers_to_geometry() {
        for icon in ALL_ICONS {
            let mut ext = extended();
            ext.icon = Some(CfIcon {
                icon,
                color: Color::None,
                show_value: true,
            });
            let builder = SceneBuilder::default();
            CfDecorationPaint::resolve(&ext, &model()).paint(&builder, rect(100));
            assert!(!builder.finish().is_empty());
        }
    }
    #[test]
    fn circle_rating_uses_threshold_quarters() {
        assert_eq!(circle_rating_fraction(1, 5), Some(0.0));
        assert_eq!(circle_rating_fraction(2, 5), Some(0.25));
        assert_eq!(circle_rating_fraction(5, 5), Some(1.0));
        assert_eq!(circle_rating_fraction(2, 0), None);
    }
}
