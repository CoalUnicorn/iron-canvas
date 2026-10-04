//! Cell text — resolution + paint.
//!
//! Layout (font metrics, line wrap, per-line positioning) is resolved by
//! `TextPaint::resolve_into`; the resulting `TextPaint` is the renderer-ready
//! snapshot consumed by `RendererCore::paint_text`. Per the `*Paint`
//! convention, every allocation that depends on cell content lives in the
//! resolve step, not in the paint loop.
//!
//! `paint_text` is a pure pixel pusher: fill each line, then optionally
//! stroke an underline / strike. Clipping is gated on `needs_clip` so the
//! 99 % of cells that fit don't pay the `save/restore` round-trip (which
//! wipes the painter's font/fill state cache and forces the next cell's
//! binds to miss).

use std::rc::Rc;

use crate::style::{CellKind, CellStyle, HAlign, VAlign};

use crate::geometry::constants::STANDARD_BORDER_WIDTH;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::model::sheet::links::CellLink;
use crate::painter::{CHAR_WIDTH_FACTOR, PaintColor, Painter, TextAlign, TextBaseline};
use crate::renderer::RendererCore;
use crate::renderer::cache::ColorIntern;
use crate::text_layout::{
    CELL_PADDING, LINE_HEIGHT_FACTOR, TEXT_V_INSET_PX, TextLayoutOptions, TextVerticalAlign,
    layout_text_into,
};
pub use crate::text_layout::{TextLine, layout_text_lines_into as layout_into};
use crate::theme::CanvasTheme;

/// Below this in either pixel dimension, no text is laid out at all.
const MIN_TEXT_DIM_PX: f64 = 10.0;

//  paint constants

/// With `textBaseline: "middle"`, `center_y` is the em-square midpoint. The
/// typographic baseline sits at ~`center_y + font_size * 0.15`; `0.35` puts
/// the underline just below the baseline, clear of the glyphs.
const UNDERLINE_OFFSET_FACTOR: f64 = 0.35;
const MIN_UNDERLINE_OFFSET: i32 = 2;

//  types

/// Pre-resolved text paint for one cell. Pure pixel inputs — no model access
/// during paint. The `Vec<TextLine>` lives on the caller's reusable buffer
/// (parked on `FrameCache::text_lines`) so resolve never allocates per cell.
///
/// `Clone` exists for the merge pass, which lays text out once and repaints it
/// under each fragment's own transform: the clone carries the translated
/// `clip` and `anchor`.
#[derive(Clone)]
pub struct TextPaint {
    pub clip: PixelRect,
    /// The rectangle Start/End alignment anchors on. Equal to `clip` for a
    /// plain cell; narrower on the left when a CF icon/rating reserves a band
    /// (see `resolve_into`'s `reserved_left`). The merge pass translates it
    /// alongside `clip`.
    pub anchor: PixelRect,
    /// Interned `ctx.font` string. `Rc::clone` on cache hit; one alloc per
    /// unique (size, bold, italic, family) tuple per renderer lifetime.
    pub font_css: Rc<str>,
    pub font_size_px: f64,
    pub color: TextColor,
    pub underline: bool,
    pub strike: bool,
    /// Resolved horizontal alignment, already normalized to the three paint
    /// directions by `CellTextStyle::resolve`. Carried through to
    /// `paint_text` so backends that can't measure glyphs accurately (SVG)
    /// can use `text-anchor="start"` / `"end"` anchored on cell boundaries
    /// instead of the `CHAR_WIDTH_FACTOR`-approximated `center_x`.
    pub h_align: TextAlign,
    /// True when at least one line overflows horizontally or there are
    /// multiple lines (which can overflow vertically). Resolved here so
    /// `paint_text` can skip `push_clip`/`pop_clip` when the cell can't
    /// spill — clip = save/restore on Canvas2D, which wipes the painter's
    /// font/fill state cache and forces the next cell's binds to miss.
    pub needs_clip: bool,
}

/// Per-cell text color. The theme paths (`ThemeDefault`, `ThemeError`) carry no
/// payload — they name a theme slot resolved at paint time against the live
/// `CanvasTheme`, so a text cell never clones the theme's color `String` (B-5:
/// on host-page `Cow::Owned` themes that was one alloc per text cell per frame).
/// `Owned` carries an interned `Rc<str>` from `ColorIntern` for the per-cell
/// font-color override — `Rc::clone` after the first sighting.
#[derive(Clone)]
pub enum TextColor {
    ThemeDefault,
    ThemeError,
    Owned(Rc<str>),
}

//  resolve

impl TextPaint {
    /// Build a `TextPaint` laid out in `rect` and clipped to `clip`, filling
    /// `lines` with the resolved per-line text/width/position. Returns `None`
    /// (with `lines` left empty) for empty/too-small cells. Formatted value AND
    /// cell type are supplied by the caller — the grid pass drains both from the
    /// prefetched value and cell-type buffers;
    /// `repaint_active_cell` reads the model directly for the active cell.
    /// Font / alignment / colour are resolved via `CellTextStyle`.
    ///
    /// `rect` and `clip` are the same rectangle for a single cell. They differ
    /// for a merged range: line positioning and alignment read the full logical
    /// rectangle, while the overflow decision and `TextPaint::clip` read the
    /// visible fragment, so a merge scrolled partly out of view clips its text
    /// at the visible edge instead of at the logical one.
    ///
    /// `reserved_left` shrinks `rect` at the left edge for a shown CF icon or
    /// rating, so the value lays out after the indicator. The text clip also
    /// excludes this band, including for centered and right-aligned values.
    ///
    /// The split between `TextPaint` (per-cell scalars) and the externally
    /// owned `lines` buffer is what makes the per-cell text path zero-alloc:
    /// the caller takes the buffer once for the grid segment, hands it
    /// to every cell, and parks it back on `FrameCache::text_lines`.
    // `rect` and `clip` are separate inputs, not one rectangle: a merged cell
    // lays text out against the logical rectangle and shows it through the
    // visible fragment. Bundling them would hide which is which.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_into<P: Painter>(
        renderer: &RendererCore<P>,
        rect: PixelRect,
        clip: PixelRect,
        reserved_left: i32,
        style: &CellStyle,
        text: String,
        cell_type: CellKind,
        link: Option<&CellLink>,
        lines: &mut Vec<TextLine>,
    ) -> Option<TextPaint> {
        if text.is_empty() {
            return None;
        }
        if f64::from(rect.width) < MIN_TEXT_DIM_PX || f64::from(rect.height) < MIN_TEXT_DIM_PX {
            return None;
        }

        let CellTextStyle {
            text_color,
            underline,
            strike,
            h_align,
            v_align,
            wrap_text,
        } = CellTextStyle::resolve(cell_type, style, &renderer.color_intern, link);

        // Font interning: skips `FontStyle::build` on cache hit. Same lookup
        // is shared across cells with identical (size, weight, slant, family).
        let size_px = style.font.size;
        let font_css = renderer.font_intern.get_or_build(
            size_px,
            style.font.bold,
            style.font.italic,
            &style.font.name,
            "Calibri",
        );

        let approx_char_w = size_px * CHAR_WIDTH_FACTOR;

        // Keep long values out of the indicator band for every alignment.
        let reserved_left = reserved_left.clamp(0, rect.width);
        let layout_rect = if reserved_left > 0 {
            PixelRect {
                top_left: Point {
                    x: rect.left() + reserved_left,
                    y: rect.top(),
                },
                width: rect.width - reserved_left,
                height: rect.height,
            }
        } else {
            rect
        };
        let clip = if reserved_left > 0 {
            clip.intersection(layout_rect)?
        } else {
            clip
        };
        let usable_w = f64::from(layout_rect.width) - 2.0 * CELL_PADDING;
        if usable_w <= 0.0 {
            return None;
        }

        let vertical = match v_align {
            VAlign::Bottom => TextVerticalAlign::Bottom,
            VAlign::Center => TextVerticalAlign::Center,
            _ => TextVerticalAlign::Top,
        };

        // The shared layout helper handles wrapping, placement, and clipping
        // from resolved paint inputs; spreadsheet alignment policy stays here.
        let mut wrap_buf = renderer.frame_cache.wrap_buf.borrow_mut();
        let needs_clip = layout_text_into(
            renderer.painter(),
            &font_css,
            &text,
            TextLayoutOptions {
                bounds: layout_rect,
                clip,
                font_size_px: size_px,
                horizontal: h_align,
                vertical,
                wrap: wrap_text,
                horizontal_padding: CELL_PADDING,
                vertical_inset: TEXT_V_INSET_PX,
                line_height_factor: LINE_HEIGHT_FACTOR,
                approx_char_width: approx_char_w,
            },
            lines,
            &mut wrap_buf,
        );
        drop(wrap_buf);

        Some(TextPaint {
            clip,
            anchor: layout_rect,
            font_css,
            font_size_px: size_px,
            color: text_color,
            underline,
            strike,
            h_align,
            needs_clip,
        })
    }
}

/// Per-cell text styling resolved from the model's raw `Style`. Private step
/// inside `TextPaint::resolve_into`. Font css is interned separately via
/// `FontIntern` so this struct carries no per-cell `String`.
struct CellTextStyle {
    text_color: TextColor,
    underline: bool,
    strike: bool,
    /// The model's `HAlign` is collapsed here, once, into the three
    /// paint directions — so `position_lines` and `paint_text` match an
    /// exhaustive enum instead of each re-deriving the fallback rules.
    h_align: TextAlign,
    v_align: VAlign,
    wrap_text: bool,
}

impl CellTextStyle {
    fn resolve(
        cell_type: CellKind,
        style: &CellStyle,
        intern: &ColorIntern,
        link: Option<&CellLink>,
    ) -> Self {
        // Error cells render in the theme's error color regardless of per-cell
        // font color, and regardless of a link — an error is not a link. Both
        // theme slots resolve at paint time (B-5), so this is payload-free.
        let text_color = if matches!(cell_type, CellKind::Error) {
            TextColor::ThemeError
        } else if let Some(color) = link.and_then(|link| link.color()) {
            // A link's resolved color wins over the cell's own font color:
            // the adapter resolved it from the workbook theme, the same
            // authority every other engine color comes from.
            TextColor::Owned(intern.get(color))
        } else {
            match style.font.color.as_deref() {
                None | Some("#000000") => TextColor::ThemeDefault,
                Some(c) => TextColor::Owned(intern.get(c)),
            }
        };

        let alignment = style.alignment.as_ref();
        let h_align = match alignment.map(|a| a.horizontal) {
            Some(HAlign::Right) => TextAlign::End,
            Some(HAlign::Center) | Some(HAlign::CenterContinuous) => TextAlign::Center,
            Some(HAlign::Left) | Some(HAlign::Fill) => TextAlign::Start,
            // Canvas 2D has no justify/distributed — fall back to left.
            Some(HAlign::Justify) | Some(HAlign::Distributed) => TextAlign::Start,
            // General or unset: numbers right, everything else left.
            None | Some(HAlign::General) => match cell_type {
                CellKind::Number => TextAlign::End,
                _ => TextAlign::Start,
            },
        };
        let v_align = alignment.map(|a| a.vertical).unwrap_or(VAlign::Bottom);
        let wrap_text = alignment.is_some_and(|a| a.wrap_text);

        Self {
            text_color,
            // A link is underlined whether or not a font style says so: a
            // static link already carries `font.u`, a formula link does not.
            underline: style.font.underline || link.is_some(),
            strike: style.font.strike,
            h_align,
            v_align,
            wrap_text,
        }
    }
}

//  paint

impl<P: Painter> RendererCore<P> {
    /// Paint a pre-computed `TextPaint` onto the canvas. Pure pixel pusher:
    /// no model access, no layout work — everything is already resolved.
    /// `lines` is the externally owned line buffer that `TextPaint::resolve_into`
    /// just filled; passing it alongside `t` keeps the per-cell allocation off
    /// the path while preserving the old "set state then clip then stroke"
    /// ordering.
    pub fn paint_text(&self, t: &TextPaint, theme: &CanvasTheme, lines: &[TextLine]) {
        // The theme text colors resolve here, at paint time, rather than being
        // cloned into every cell at resolve time (B-5). `from_theme_str` still
        // routes the built-in `Cow::Borrowed` slot through the painter's ptr-eq
        // fast path. `TextColor::Owned` is the per-cell font-color override.
        let color = match &t.color {
            TextColor::ThemeDefault => PaintColor::from_theme_str(&theme.default_text_color),
            TextColor::ThemeError => PaintColor::from_theme_str(&theme.error_text_color),
            TextColor::Owned(s) => PaintColor::Borrowed(s),
        };
        // `font_css` is interned `Rc<str>` per (size, weight, slant, family);
        // not `&'static`, so it goes through Borrowed (content-eq cache hit
        // across cells with the same interned font).
        let font_css = PaintColor::Borrowed(&t.font_css);
        let underline_offset =
            (t.font_size_px * UNDERLINE_OFFSET_FACTOR).max(f64::from(MIN_UNDERLINE_OFFSET));
        let stroke_w = f64::from(STANDARD_BORDER_WIDTH);

        if t.needs_clip {
            self.painter.push_clip(t.clip);
        }
        for line in lines {
            let (x, align) = match t.h_align {
                TextAlign::End => (f64::from(t.anchor.right()) - CELL_PADDING, TextAlign::End),
                TextAlign::Center => (line.center_x, TextAlign::Center),
                TextAlign::Start => {
                    // Start-anchored on the cell's (or reserved layout
                    // rectangle's) left edge. No width approximation needed;
                    // the SVG `text-anchor="start"` renders glyphs at their
                    // natural width and `needs_clip` contains any escaping
                    // glyphs to the cell rectangle on both backends.
                    (f64::from(t.anchor.left()) + CELL_PADDING, TextAlign::Start)
                }
            };
            self.painter.fill_text(
                &line.text,
                x,
                line.center_y,
                font_css,
                color,
                align,
                TextBaseline::Middle,
            );
            let x1 = line.center_x - line.width / 2.0;
            let x2 = line.center_x + line.width / 2.0;
            if t.underline {
                self.painter.stroke_text_hline(
                    x1,
                    x2,
                    line.center_y + underline_offset,
                    color,
                    stroke_w,
                );
            }
            if t.strike {
                self.painter
                    .stroke_text_hline(x1, x2, line.center_y, color, stroke_w);
            }
        }
        if t.needs_clip {
            self.painter.pop_clip();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::RCRange;
    use crate::model::sheet::links::{CellLink, LinkTarget};

    fn link(color: Option<&str>, dynamic: bool) -> CellLink {
        CellLink::new(
            RCRange::from_cell(1, 1),
            LinkTarget::External("https://example.com".to_string()),
            None,
            dynamic,
            color.map(str::to_string),
        )
    }

    // A formula link carries no `font.u`, so the link rule must underline it;
    // the resolved color comes from the link, not the cell's font color.
    #[test]
    fn a_dynamic_link_resolves_to_underlined_link_color() {
        let intern = ColorIntern::default();
        let style = CellStyle::default();
        let link = link(Some("#0563C1"), true);
        let resolved = CellTextStyle::resolve(CellKind::Text, &style, &intern, Some(&link));
        assert!(resolved.underline, "a link must be underlined");
        match resolved.text_color {
            TextColor::Owned(color) => assert_eq!(&*color, "#0563c1"),
            _ => panic!("the link's resolved color must win"),
        }
    }

    // No link means no behavioural change from the pre-link renderer.
    #[test]
    fn a_cell_without_a_link_resolves_as_before() {
        let intern = ColorIntern::default();
        let resolved = CellTextStyle::resolve(CellKind::Text, &CellStyle::default(), &intern, None);
        assert!(!resolved.underline);
        assert!(matches!(resolved.text_color, TextColor::ThemeDefault));
    }

    // The error rule stays first: an error cell is not a link, even when the
    // address also holds one.
    #[test]
    fn an_error_cell_keeps_the_error_color_with_a_link() {
        let intern = ColorIntern::default();
        let style = CellStyle::default();
        let link = link(Some("#0563C1"), false);
        let resolved = CellTextStyle::resolve(CellKind::Error, &style, &intern, Some(&link));
        assert!(matches!(resolved.text_color, TextColor::ThemeError));
        assert!(resolved.underline);
    }

    // A link with no resolved color falls back to the existing font rules.
    #[test]
    fn a_colorless_link_falls_back_to_the_font_color() {
        let intern = ColorIntern::default();
        let style = CellStyle::default();
        let link = link(None, false);
        let resolved = CellTextStyle::resolve(CellKind::Text, &style, &intern, Some(&link));
        assert!(resolved.underline);
        assert!(matches!(resolved.text_color, TextColor::ThemeDefault));
    }
}
