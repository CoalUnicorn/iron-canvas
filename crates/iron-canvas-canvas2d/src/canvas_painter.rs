//! Canvas 2D backend for `Painter`.
//!
//! Owns the `CanvasRenderingContext2d` plus the per-frame state cache
//! (`RefCell<CachedString>` for fill/stroke/font, `Cell<f64>` for line_width).
//! The cache is a Canvas2D-only optimization — it lives here, not in the
//! trait, because the SVG/PDF backends do not need it.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, js_sys};

use iron_canvas_core::geometry::{
    constants::{DASHED_RECT_PATTERN, STANDARD_BORDER_WIDTH},
    path::{Path, PathCmd, PointF},
    pixel_rect::PixelRect,
    prim::{Line, Span},
};
use iron_canvas_core::painter::{
    BlitPainter, FontMetrics, GroupClass, LineCap, LineJoin, PaintColor, Painter, StrokeStyle,
    TextAlign, TextBaseline, TextMetrics, approx_text_width, parse_font_size_px,
};

use crate::measure_cache::MeasureCache;

// Private `console.warn` binding — zero IronCalc, kept local so this crate
// does not depend on the crate-private `wasm` diagnostics of the former web
// facade.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn console_warn(s: &str);
}

/// One-shot guard for the `measure_text` fallback warning. Process-wide
/// (not per-painter) so grid + overlay layers share a single signal —
/// the failure mode is a ctx-level capability, not a layer-local one.
static MEASURE_WARN_EMITTED: AtomicBool = AtomicBool::new(false);

/// One-shot guard for the `apply_dpr_transform` rejection warning. Separate
/// from `MEASURE_WARN_EMITTED`: a lost ctx can surface through either call
/// site first, and one warning must not silence the other.
static DPR_WARN_EMITTED: AtomicBool = AtomicBool::new(false);

/// Print `message` through the private `console.warn` binding at most once
/// per process for `flag`. Every one-shot canvas-capability warning routes
/// through here so the compare-exchange idiom lives in one place.
fn warn_once(flag: &AtomicBool, message: &str) {
    if flag
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        console_warn(message);
    }
}

/// Snap an axis-aligned stroke's cross-axis coordinate onto the pixel grid.
///
/// Canvas centers a stroke on its path, so a width-1 line at integer `coord`
/// covers `coord-0.5 .. coord+0.5` and antialiases into two half-opacity
/// columns — fuzzy borders whose corners read as a faint/missing pixel where
/// two perpendicular edges meet. Odd-width strokes are crisp when centered on
/// a half-pixel; even-width strokes when centered on the integer. Mirrors the
/// `+ 0.5` trick `draw_corner_box` already uses for frozen separators.
fn snap_stroke_cross(coord: f64, width: f64) -> f64 {
    if (width.round() as i32) % 2 == 0 {
        coord.round()
    } else {
        coord.floor() + 0.5
    }
}

/// Canvas `lineCap` value for a [`LineCap`].
fn cap_str(cap: LineCap) -> &'static str {
    match cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    }
}

/// Canvas `lineJoin` value for a [`LineJoin`].
fn join_str(join: LineJoin) -> &'static str {
    match join {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    }
}

/// One cached color or font. Static values need no allocation.
/// Borrowed values use one buffer because no caller shares the stored value.
/// Keep its capacity through invalidation and static values to reuse it later.
#[derive(Default)]
struct CachedString {
    static_value: Option<&'static str>,
    buffer: String,
    valid: bool,
}

impl CachedString {
    /// True when the next paint can skip the `ctx.set_*` round-trip.
    ///
    /// `Static-Static` compares the string address and length. Both must match
    /// because static slices can share an address. Content equality across the
    /// other variants keeps us correct when a `Borrowed` color happens to
    /// equal a previously cached `Static`, or vice-versa.
    fn matches(&self, other: PaintColor) -> bool {
        if !self.valid {
            return false;
        }
        match (self.static_value, other) {
            (Some(a), PaintColor::Static(b)) => std::ptr::eq(a, b),
            (Some(a), PaintColor::Borrowed(b)) => a == b,
            (None, other) => self.buffer == other.as_str(),
        }
    }

    fn store(&mut self, value: PaintColor) {
        self.static_value = match value {
            PaintColor::Static(value) => Some(value),
            PaintColor::Borrowed(value) => {
                self.buffer.clear();
                self.buffer.push_str(value);
                None
            }
        };
        self.valid = true;
    }

    fn invalidate(&mut self) {
        self.valid = false;
    }
}

/// Sticky `ctx.set_*` state cache for `CanvasPainter`. Lifted out as its
/// own type so the scene backend owns the invalidation contract explicitly:
/// a full rebuild invalidates at the start (ctx state about to change),
/// while a reuse/blit-only frame preserves it (grid ctx state untouched).
/// Other painter backends (SvgPainter, PdfPainter) have no analog — the
/// cache is a Canvas2D-specific optimization.
#[derive(Default)]
pub(crate) struct SetterCache {
    last_fill: RefCell<CachedString>,
    last_stroke: RefCell<CachedString>,
    last_font: RefCell<CachedString>,
    pub(crate) last_line_width: Cell<f64>,
}

impl SetterCache {
    /// Clear all four cached bindings after a context state change.
    /// Retain string capacity so restoration does not cause new allocations.
    pub(crate) fn invalidate(&self) {
        self.last_fill.borrow_mut().invalidate();
        self.last_stroke.borrow_mut().invalidate();
        self.last_font.borrow_mut().invalidate();
        self.last_line_width.set(0.0);
    }
}

pub struct CanvasPainter {
    /// Crate-internal: hosts drive the painter through the `Painter` trait,
    /// and an external write would desync `setter_cache` from the ctx.
    pub(crate) ctx: CanvasRenderingContext2d,
    /// Cross-canvas blit source. `Some` on the double-buffered grid layer:
    /// `blit` reads the kept band from the front (visible) canvas instead of
    /// self-copying `ctx`'s own canvas — same-canvas `drawImage` is the
    /// interpolation hazard documented in `apply_dpr_transform`.
    blit_src: Option<HtmlCanvasElement>,
    pub(crate) setter_cache: SetterCache,
    /// Only this crate's own modules touch the ctx-state mirrors below
    /// (`dash_pattern` / `dash_empty` / `clip_depth` / `dpr`); every consumer
    /// outside the crate sees the `Painter`/`BlitPainter` trait surface.
    pub(crate) dash_pattern: js_sys::Array,
    pub(crate) dash_empty: js_sys::Array,
    pub(crate) clip_depth: Cell<u32>,
    /// Mirror of the active ctx.scale factor, written by every
    /// `apply_dpr_transform`. Read by `blit` so the source rect (which
    /// reads from the DPR-scaled backing store) is sized in backing-store
    /// pixels — dest coords go through the active transform unchanged.
    pub(crate) dpr: Cell<f64>,
    /// Memo of `ctx.measure_text` widths keyed `(font_css, text)`.
    /// Interior mutability because `TextMetrics::measure_text_width` takes
    /// `&self`. `get` and `insert` are separate short borrows — never held
    /// across the JS measure call. Content-keyed, so `SetterCache`
    /// invalidation must NOT touch it; only `clear_measure_cache` (font
    /// load) empties it.
    measure_cache: RefCell<MeasureCache>,
    font_metrics: RefCell<std::collections::HashMap<String, Option<FontMetrics>>>,
}

impl CanvasPainter {
    pub fn new(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            ctx,
            blit_src: None,
            setter_cache: SetterCache::default(),
            dash_pattern: js_sys::Array::of2(
                &DASHED_RECT_PATTERN[0].into(),
                &DASHED_RECT_PATTERN[1].into(),
            ),
            dash_empty: js_sys::Array::new(),
            clip_depth: Cell::new(0),
            dpr: Cell::new(1.0),
            measure_cache: RefCell::new(MeasureCache::default()),
            font_metrics: RefCell::default(),
        }
    }

    /// Construct a painter whose `blit` reads the kept band from `src` (the
    /// front/visible canvas) instead of `ctx`'s own backing store. Used for
    /// the double-buffered grid layer; see `blit_src`.
    pub fn with_blit_source(ctx: CanvasRenderingContext2d, src: HtmlCanvasElement) -> Self {
        let mut painter = Self::new(ctx);
        painter.blit_src = Some(src);
        painter
    }

    pub(crate) fn set_fill_cached(&self, color: PaintColor) {
        if self.setter_cache.last_fill.borrow().matches(color) {
            return;
        }
        self.ctx.set_fill_style_str(color.as_str());
        self.setter_cache.last_fill.borrow_mut().store(color);
    }

    fn set_stroke_cached(&self, color: PaintColor) {
        if self.setter_cache.last_stroke.borrow().matches(color) {
            return;
        }
        self.ctx.set_stroke_style_str(color.as_str());
        self.setter_cache.last_stroke.borrow_mut().store(color);
    }

    pub(crate) fn set_font_cached(&self, font: PaintColor) {
        if self.setter_cache.last_font.borrow().matches(font) {
            return;
        }
        self.ctx.set_font(font.as_str());
        self.setter_cache.last_font.borrow_mut().store(font);
    }

    pub(crate) fn set_line_width_cached(&self, width: f64) {
        if (self.setter_cache.last_line_width.get() - width).abs() > f64::EPSILON {
            self.ctx.set_line_width(width);
            self.setter_cache.last_line_width.set(width);
        }
    }

    pub(crate) fn with_stroke_width<F: FnOnce(&Self)>(&self, width: f64, f: F) {
        self.set_line_width_cached(width);
        f(self);
        self.set_line_width_cached(f64::from(STANDARD_BORDER_WIDTH));
    }

    /// Install a fresh ctx path from `path`. The caller must have validated
    /// `path`. Shared by `fill_path` and `stroke_path` so both backends emit
    /// the same command sequence.
    fn build_path(&self, path: &Path<'_>) {
        self.ctx.begin_path();
        for cmd in path.cmds() {
            match cmd {
                PathCmd::Move(p) => self.ctx.move_to(p.x, p.y),
                PathCmd::Line(p) => self.ctx.line_to(p.x, p.y),
                PathCmd::Quad(c, e) => self.ctx.quadratic_curve_to(c.x, c.y, e.x, e.y),
                PathCmd::Cubic(a, b, e) => {
                    self.ctx.bezier_curve_to(a.x, a.y, b.x, b.y, e.x, e.y);
                }
                PathCmd::Arc(spec) => {
                    if spec.sweep_angle == 0.0 {
                        continue;
                    }
                    let _ = self.ctx.arc_with_anticlockwise(
                        spec.center.x,
                        spec.center.y,
                        spec.radius,
                        spec.start_angle,
                        spec.start_angle + spec.sweep_angle,
                        spec.sweep_angle < 0.0,
                    );
                }
                PathCmd::Close => self.ctx.close_path(),
            }
        }
    }

    /// A `js_sys::Array` for `ctx.setLineDash`. An empty pattern reuses the
    /// painter-lifetime empty array, so a solid stroke allocates nothing.
    fn dash_array(&self, dash: &[f64]) -> js_sys::Array {
        if dash.is_empty() {
            return self.dash_empty.clone();
        }
        let arr = js_sys::Array::new_with_length(dash.len() as u32);
        for (i, v) in dash.iter().enumerate() {
            arr.set(i as u32, JsValue::from_f64(*v));
        }
        arr
    }

    /// Drop all memoized text widths. Called via the facades'
    /// `fontsChanged()` when `document.fonts` finishes loading — widths
    /// measured against a fallback font are stale the moment the real
    /// font arrives.
    pub fn clear_measure_cache(&self) {
        self.measure_cache.borrow_mut().clear();
        self.font_metrics.borrow_mut().clear();
    }
}

impl Drop for CanvasPainter {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.clip_depth.get(),
            0,
            "CanvasPainter dropped with unbalanced push_clip/pop_clip"
        );
    }
}

impl TextMetrics for CanvasPainter {
    fn font_metrics(&self, font_css: &str) -> Option<FontMetrics> {
        if let Some(metrics) = self.font_metrics.borrow().get(font_css) {
            return *metrics;
        }
        self.set_font_cached(PaintColor::Borrowed(font_css));
        // Canvas reports extents relative to the current baseline. Measure at
        // the alphabetic baseline, then restore the caller's context state.
        let baseline = self.ctx.text_baseline();
        self.ctx.set_text_baseline("alphabetic");
        let metrics = self
            .ctx
            .measure_text("Mg")
            .ok()
            .map(|metrics| FontMetrics {
                ascent: metrics.font_bounding_box_ascent(),
                descent: metrics.font_bounding_box_descent(),
                line_gap: 0.0,
            })
            .filter(|metrics| metrics.is_valid());
        self.ctx.set_text_baseline(&baseline);
        let mut cache = self.font_metrics.borrow_mut();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(font_css.to_owned(), metrics);
        metrics
    }

    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        if let Some(w) = self.measure_cache.borrow().get(font_css, text) {
            return w;
        }
        // Metrics callers (FontIntern + HEADER_FONT only feed `fill_text`)
        // pass an `&str` here. Treat as Borrowed — the cache will still hit
        // against a previously cached Static via content-eq if it was the
        // same literal.
        self.set_font_cached(PaintColor::Borrowed(font_css));
        match self.ctx.measure_text(text) {
            Ok(m) => {
                let w = m.width();
                self.measure_cache.borrow_mut().insert(font_css, text, w);
                w
            }
            Err(_) => {
                // Debug builds: crash on first occurrence so a regressing
                // ctx state (lost context, bad font_css) is caught loud
                // in dev. Release: fall back to the shared `approx_text_width`
                // estimate so a measure error agrees with the SVG/PDF/recorder
                // backends instead of diverging; a single console.warn whispers
                // once per session. Deliberately NOT cached: a broken ctx must
                // keep remeasuring so recovery heals.
                debug_assert!(
                    false,
                    "CanvasPainter::measure_text_width: ctx.measure_text errored; \
                     falling back to approx_text_width for {text:?} font={font_css:?}"
                );
                warn_once(
                    &MEASURE_WARN_EMITTED,
                    "iron-canvas: ctx.measure_text errored; using approx_text_width fallback \
                     (subsequent measure errors silenced)",
                );
                approx_text_width(parse_font_size_px(font_css), text)
            }
        }
    }
}

impl Painter for CanvasPainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.set_fill_cached(color);
        let (x, y, w, h) = rect.as_f64_tuple();
        self.ctx.fill_rect(x, y, w, h);
    }

    fn rect_fill_hgradient(&self, rect: PixelRect, from: PaintColor, to: PaintColor) {
        let (x, y, w, h) = rect.as_f64_tuple();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let gradient = self.ctx.create_linear_gradient(x, y, x + w, y);
        // A malformed color is the only failure mode; a lost ctx surfaces on
        // the fill below. Either way the gradient object keeps its last stop.
        let _ = gradient.add_color_stop(0.0, from.as_str());
        let _ = gradient.add_color_stop(1.0, to.as_str());
        self.ctx.set_fill_style_canvas_gradient(&gradient);
        // The ctx fill style is now a gradient object, not the cached color
        // string: drop the sticky fill so the next `set_fill_cached` re-binds.
        self.setter_cache.last_fill.borrow_mut().invalidate();
        self.ctx.fill_rect(x, y, w, h);
    }

    fn fill_path(&self, path: &Path<'_>, color: PaintColor) {
        if path.validate().is_err() {
            return; // invalid path paints nothing
        }
        self.set_fill_cached(color);
        self.build_path(path);
        self.ctx.fill();
    }

    /// Stroke state (color, width, cap, join, miter, dash) is installed inside
    /// a `save`/`restore` pair, and the sticky setter cache is invalidated after
    /// `restore` because it resets every one of those ctx properties. So a
    /// styled path cannot leak a dash or a round cap into a later grid line,
    /// border, or text underline.
    fn stroke_path(&self, path: &Path<'_>, color: PaintColor, style: &StrokeStyle<'_>) {
        if path.validate().is_err() || !style.is_valid() {
            return;
        }
        self.ctx.save();
        self.set_stroke_cached(color);
        self.set_line_width_cached(style.width);
        self.ctx.set_line_cap(cap_str(style.cap));
        self.ctx.set_line_join(join_str(style.join));
        self.ctx.set_miter_limit(style.miter_limit);
        let dash = self.dash_array(style.dash);
        let _ = self.ctx.set_line_dash(&dash);
        self.build_path(path);
        self.ctx.stroke();
        self.ctx.restore();
        self.setter_cache.invalidate();
    }

    fn fill_circle(&self, cx: f64, cy: f64, radius: f64, color: PaintColor) {
        if !radius.is_finite() || radius <= 0.0 {
            return; // non-finite or non-positive radius is a no-op
        }
        self.set_fill_cached(color);
        self.ctx.begin_path();
        // A full turn; `fill` closes the subpath, so no `close_path` needed.
        // `arc` rejects only a negative radius, which the guard above rules
        // out.
        let _ = self.ctx.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
        self.ctx.fill();
    }

    fn clear_rect(&self, rect: PixelRect) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.ctx.clear_rect(x, y, w, h);
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.set_stroke_cached(color);
        let (x, y, w, h) = rect.as_f64_tuple();
        self.with_stroke_width(width, |this| {
            this.ctx.stroke_rect(x, y, w, h);
        });
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let _ = self.ctx.set_line_dash(&self.dash_pattern);
        self.rect_stroke(rect, color, width);
        let _ = self.ctx.set_line_dash(&self.dash_empty);
    }

    /// Stroke an axis-aligned `Line`. Dispatches on the enum variant so the
    /// caller doesn't pick `stroke_hline` vs `stroke_vline` manually.
    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        match line {
            Line::H { span, y } => self.stroke_hline(span, f64::from(y), color, width),
            Line::V { x, span } => self.stroke_vline(f64::from(x), span, color, width),
        }
    }

    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        let y = snap_stroke_cross(y, width);
        self.set_stroke_cached(color);
        self.set_line_width_cached(width);
        self.ctx.begin_path();
        self.ctx.move_to(f64::from(span.from), y);
        self.ctx.line_to(f64::from(span.to), y);
        self.ctx.stroke();
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        let x = snap_stroke_cross(x, width);
        self.set_stroke_cached(color);
        self.set_line_width_cached(width);
        self.ctx.begin_path();
        self.ctx.move_to(x, f64::from(span.from));
        self.ctx.line_to(x, f64::from(span.to));
        self.ctx.stroke();
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.set_stroke_cached(color);
        self.set_line_width_cached(width);
        self.ctx.begin_path();
        self.ctx.move_to(x1, y);
        self.ctx.line_to(x2, y);
        self.ctx.stroke();
    }

    fn push_clip(&self, rect: PixelRect) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx.rect(x, y, w, h);
        self.ctx.clip();
        self.clip_depth.set(self.clip_depth.get() + 1);
    }

    fn pop_clip(&self) {
        debug_assert!(
            self.clip_depth.get() > 0,
            "pop_clip without matching push_clip"
        );
        self.ctx.restore();
        self.clip_depth.set(self.clip_depth.get() - 1);
        // restore() resets fill/stroke/font/lineWidth/dash — invalidate cache
        self.setter_cache.invalidate();
    }

    fn fill_text(
        &self,
        text: &str,
        position: PointF,
        font_css: PaintColor,
        color: PaintColor,
        align: TextAlign,
        baseline: TextBaseline,
    ) {
        self.set_font_cached(font_css);
        self.set_fill_cached(color);
        self.ctx.set_text_align(match align {
            TextAlign::Start => "start",
            TextAlign::Center => "center",
            TextAlign::End => "end",
        });
        self.ctx.set_text_baseline(match baseline {
            TextBaseline::Top => "top",
            TextBaseline::Middle => "middle",
            TextBaseline::Bottom => "bottom",
            TextBaseline::Alphabetic => "alphabetic",
        });
        let _ = self.ctx.fill_text(text, position.x, position.y);
    }

    fn invalidate_cache(&self) {
        // Public escape hatch for the renderer between frames.
        // (Layers call this in their `paint()` method today.)
        self.setter_cache.invalidate();
    }

    fn apply_dpr_transform(&self, dpr: f64) {
        self.dpr.set(dpr);
        // Both calls are spec-infallible for finite arguments, and `dpr` is
        // already validated finite and positive by `CanvasMetrics::new`, so
        // an `Err` here means the ctx is gone (lost context) rather than a
        // caller error. Panicking would abort the whole wasm app mid-paint,
        // so keep the frame alive and report the unscaled output once —
        // the same discipline `measure_text_width` applies below.
        let reset = self.ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        if reset.and_then(|()| self.ctx.scale(dpr, dpr)).is_err() {
            debug_assert!(
                false,
                "CanvasPainter::apply_dpr_transform: ctx rejected the DPR transform"
            );
            warn_once(
                &DPR_WARN_EMITTED,
                "iron-canvas: ctx rejected the DPR transform; the frame paints unscaled \
                 (subsequent rejections silenced)",
            );
        }
        // Canvas2D defaults `imageSmoothingEnabled = true`, which bilinear-
        // interpolates `drawImage` source pixels. `Painter::blit` without a
        // `blit_src` calls drawImage with src/dst on the same canvas — every
        // interpolation pass smudges 1-pixel edges in the kept band. After a
        // scroll round-trip those smudges visibly accumulate as horizontal /
        // vertical lines across cells that should be empty. The grid layer's
        // cross-canvas `blit_src` (see the `blit_src` field doc) avoids the
        // self-copy entirely, but this stays disabled for both paths because
        // the ctx state is wiped on every canvas resize and this method is
        // the canonical re-init point.
        self.ctx.set_image_smoothing_enabled(false);
    }

    fn reset_text_defaults(&self) {
        self.ctx.set_text_align("center");
        self.ctx.set_text_baseline("middle");
    }

    fn begin_group(&self, _class: GroupClass) {}
    fn end_group(&self) {}
}

impl BlitPainter for CanvasPainter {
    fn blit(&self, src: PixelRect, dst: PixelRect) {
        // Prefer the cross-canvas source (front buffer). Fall back to the
        // painter's own canvas for direct-drawn surfaces; `ctx.canvas()` is
        // None only for a detached OffscreenCanvas ctx, which iron-canvas
        // never constructs — dropping on None keeps blit a silent no-op.
        let fallback;
        let canvas: &HtmlCanvasElement = if let Some(front) = &self.blit_src {
            front
        } else {
            let Some(own) = self.ctx.canvas() else {
                return;
            };
            fallback = own;
            &fallback
        };
        // Source coords address the DPR-scaled backing store; dest coords
        // flow through the active ctx.scale(dpr,dpr) transform, so only
        // src multiplies explicitly. This matches the asymmetry of
        // CanvasRenderingContext2D.drawImage's source/destination spaces.
        let dpr = self.dpr.get();
        let (sx0, sy0, sw0, sh0) = src.as_f64_tuple();
        let (dx, dy, dw, dh) = dst.as_f64_tuple();
        let (sx, sy, sw, sh) = (sx0 * dpr, sy0 * dpr, sw0 * dpr, sh0 * dpr);

        // Replace transparent source pixels too. Limit `copy` to the
        // destination so it cannot clear headers or frozen cells outside it.
        // drawImage snapshots a self-copy before writing overlapping pixels.
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx.rect(dx, dy, dw, dh);
        self.ctx.clip();
        let _ = self.ctx.set_global_composite_operation("copy");
        let _ = self
            .ctx
            .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                canvas, sx, sy, sw, sh, dx, dy, dw, dh,
            );
        self.ctx.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_cache_distinguishes_slices_with_the_same_start() {
        let full: &'static str = "#123456";
        let short: &'static str = &full[..4];
        assert_eq!(full.as_ptr(), short.as_ptr());

        let mut cached = CachedString::default();
        cached.store(PaintColor::Static(full));
        assert!(!cached.matches(PaintColor::Static(short)));
        assert!(cached.matches(PaintColor::Static(full)));
        cached.store(PaintColor::Static(short));
        assert!(!cached.matches(PaintColor::Static(full)));
        assert_eq!(cached.buffer.capacity(), 0);
    }

    #[test]
    fn cached_string_owns_borrowed_values_and_matches_static_values() {
        let mut cached = CachedString::default();
        assert!(!cached.matches(PaintColor::Borrowed("")));
        cached.store(PaintColor::Static("#123456"));
        assert!(cached.matches(PaintColor::Borrowed("#123456")));

        let mut input = String::from("#abcdef");
        cached.store(PaintColor::Borrowed(&input));
        input.clear();
        assert!(cached.matches(PaintColor::Static("#abcdef")));
        assert!(!cached.matches(PaintColor::Borrowed(&input)));

        cached.store(PaintColor::Borrowed(""));
        assert!(cached.matches(PaintColor::Borrowed("")));
        assert!(!cached.matches(PaintColor::Static("#abcdef")));
    }

    #[test]
    fn invalidation_keeps_buffers_for_all_three_setters() {
        let cache = SetterCache::default();
        let value = "20px sans-serif";
        for slot in [&cache.last_fill, &cache.last_stroke, &cache.last_font] {
            slot.borrow_mut().store(PaintColor::Borrowed(value));
        }
        cache.last_line_width.set(3.0);
        cache.invalidate();
        assert_eq!(cache.last_line_width.get(), 0.0);

        for slot in [&cache.last_fill, &cache.last_stroke, &cache.last_font] {
            let mut cached = slot.borrow_mut();
            assert!(!cached.matches(PaintColor::Borrowed(value)));
            let capacity = cached.buffer.capacity();
            let allocation = cached.buffer.as_ptr();
            cached.store(PaintColor::Static("red"));
            cached.invalidate();
            for value in ["#123456", "#123", "20px serif", "#abcdef"] {
                cached.store(PaintColor::Borrowed(value));
                assert!(cached.matches(PaintColor::Borrowed(value)));
                assert_eq!(cached.buffer.capacity(), capacity);
                assert_eq!(cached.buffer.as_ptr(), allocation);
            }
        }
    }
}
