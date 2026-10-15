//! Atomic Fresh and retained rendering to a fixed-size browser canvas.
use crate::CanvasPainter;
use iron_canvas_core::{
    CanvasMetrics, PixelRect, Point,
    painter::{Painter, TextMetrics},
    scene::{GridScene, RenderBackend, retained::SceneUpdate},
};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, js_sys};

pub struct Canvas2dSceneBackend {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    metrics_painter: CanvasPainter,
    metrics: CanvasMetrics,
    spare: Option<HtmlCanvasElement>,
}
impl Canvas2dSceneBackend {
    /// Initialize an empty target. Dimensions stay fixed for this session.
    /// Construct a new backend to change them; construction clears the target.
    pub fn new(canvas: HtmlCanvasElement, metrics: CanvasMetrics) -> Result<Self, JsValue> {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("Canvas2D unavailable"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let detached = detached(&canvas)?;
        let metrics_context = detached
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("Canvas2D metrics unavailable"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let (width, height) = metrics.backing_size();
        canvas.set_width(width);
        canvas.set_height(height);
        context.set_global_composite_operation("copy")?;
        context.set_image_smoothing_enabled(false);
        Ok(Self {
            canvas,
            context,
            metrics_painter: CanvasPainter::new(metrics_context),
            metrics,
            spare: None,
        })
    }
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }
}
impl RenderBackend for Canvas2dSceneBackend {
    type StagedFrame = HtmlCanvasElement;
    type Error = JsValue;
    fn metrics(&self) -> &dyn TextMetrics {
        &self.metrics_painter
    }
    fn stage_frame(&mut self, scene: &GridScene) -> Result<HtmlCanvasElement, JsValue> {
        let (width, height) = self.metrics.size().to_logical_extent();
        if scene.clip
            != (PixelRect {
                top_left: Point { x: 0, y: 0 },
                width,
                height,
            })
        {
            return Err(JsValue::from_str("scene dimensions differ from the target"));
        }
        let canvas = match self.spare.take() {
            Some(canvas) => canvas,
            None => detached(&self.canvas)?,
        };
        let (painter, context) = attach_painter(&canvas, self.metrics)?;
        context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)?;
        context.set_global_composite_operation("source-over")?;
        context.clear_rect(
            0.0,
            0.0,
            f64::from(canvas.width()),
            f64::from(canvas.height()),
        );
        painter.invalidate_cache();
        painter.apply_dpr_transform(self.metrics.dpr());
        painter.reset_text_defaults();
        scene.paint(&painter);
        Ok(canvas)
    }
    fn invalidate_metrics(&mut self) {
        self.metrics_painter.clear_measure_cache();
    }
    fn supports_partial_updates(&self) -> bool {
        true
    }
    fn retained_scale(&self) -> Option<f64> {
        Some(self.metrics.dpr())
    }
    fn stage_update(
        &mut self,
        scene: &GridScene,
        update: &SceneUpdate,
    ) -> Result<HtmlCanvasElement, JsValue> {
        if matches!(update, SceneUpdate::Fresh) {
            return self.stage_frame(scene);
        }
        let (width, height) = self.metrics.size().to_logical_extent();
        if scene.clip
            != (PixelRect {
                top_left: Point::default(),
                width,
                height,
            })
        {
            return Err(JsValue::from_str("scene dimensions differ from the target"));
        }
        if matches!(update, SceneUpdate::Reuse) {
            return Ok(self.canvas.clone());
        }
        let canvas = match self.spare.take() {
            Some(canvas) => canvas,
            None => detached(&self.canvas)?,
        };
        let (painter, context) = attach_painter(&canvas, self.metrics)?;
        context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)?;
        context.set_image_smoothing_enabled(false);
        context.set_global_composite_operation("copy")?;
        if matches!(update, SceneUpdate::Damage(_)) {
            context.draw_image_with_html_canvas_element(&self.canvas, 0.0, 0.0)?;
        }
        let scale = self.metrics.dpr();
        let repaint = match update {
            SceneUpdate::Damage(rects) => rects,
            SceneUpdate::Blit {
                source,
                destination,
                repaint,
            } => {
                context.save();
                context.begin_path();
                context.rect(
                    f64::from(destination.x) * scale,
                    f64::from(destination.y) * scale,
                    f64::from(source.width) * scale,
                    f64::from(source.height) * scale,
                );
                context.clip();
                let result = context
                    .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                        &self.canvas,
                        f64::from(source.left()) * scale,
                        f64::from(source.top()) * scale,
                        f64::from(source.width) * scale,
                        f64::from(source.height) * scale,
                        f64::from(destination.x) * scale,
                        f64::from(destination.y) * scale,
                        f64::from(source.width) * scale,
                        f64::from(source.height) * scale,
                    );
                context.restore();
                result?;
                repaint
            }
            _ => unreachable!(),
        };
        context.set_global_composite_operation("source-over")?;
        context.save();
        context.begin_path();
        let mut regions = Vec::with_capacity(repaint.len());
        for rect in repaint {
            // One device-aligned union clip avoids seams and repeated paint.
            let x = (f64::from(rect.left()) * scale).floor();
            let y = (f64::from(rect.top()) * scale).floor();
            let right = (f64::from(rect.right()) * scale).ceil();
            let bottom = (f64::from(rect.bottom()) * scale).ceil();
            context.rect(x, y, right - x, bottom - y);
            regions.push(PixelRect {
                top_left: Point {
                    x: (x / scale).floor() as i32,
                    y: (y / scale).floor() as i32,
                },
                width: (right / scale).ceil() as i32 - (x / scale).floor() as i32,
                height: (bottom / scale).ceil() as i32 - (y / scale).floor() as i32,
            });
        }
        context.clip();
        context.clear_rect(
            0.0,
            0.0,
            f64::from(canvas.width()),
            f64::from(canvas.height()),
        );
        painter.invalidate_cache();
        painter.apply_dpr_transform(scale);
        painter.reset_text_defaults();
        scene.paint_regions(&painter, &regions);
        context.restore();
        Ok(canvas)
    }
    fn commit_frame(&mut self, staged: HtmlCanvasElement) -> Result<(), JsValue> {
        if staged == self.canvas {
            return Ok(());
        }
        // No resize occurs here. A failed copy leaves the previous pixels intact.
        self.context.set_global_composite_operation("copy")?;
        self.context
            .draw_image_with_html_canvas_element(&staged, 0.0, 0.0)?;
        self.spare = Some(staged);
        Ok(())
    }
}

/// Size `canvas` to the backing store `metrics` describes, then return a
/// painter over its 2D context together with that context. The overlay keeps
/// its alpha channel and asks for the `desynchronized` hint, matching the
/// direct-draw overlay surface this backend replaced.
fn attach_painter(
    canvas: &HtmlCanvasElement,
    metrics: CanvasMetrics,
) -> Result<(CanvasPainter, CanvasRenderingContext2d), JsValue> {
    let (target_w, target_h) = metrics.backing_size();
    if canvas.width() != target_w || canvas.height() != target_h {
        canvas.set_width(target_w);
        canvas.set_height(target_h);
    }
    let context = overlay_context(canvas)?;
    let painter = CanvasPainter::new(context.clone());
    Ok((painter, context))
}

/// Build the 2D context for an overlay canvas: alpha preserved (an empty
/// canvas must stay transparent) and `desynchronized: true` for direct
/// updates. A backing-store resize resets ctx state, so callers re-apply the
/// transform, composite op, and smoothing pins they need after this.
fn overlay_context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, JsValue> {
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"alpha".into(), &JsValue::from(true))
        .map_err(|_| JsValue::from_str("failed to set canvas context alpha option"))?;
    js_sys::Reflect::set(&opts, &"desynchronized".into(), &JsValue::from(true))
        .map_err(|_| JsValue::from_str("failed to set canvas context desynchronized option"))?;
    canvas
        .get_context_with_context_options("2d", &opts)?
        .ok_or_else(|| JsValue::from_str("Canvas2D unavailable"))
        .map(|c| c.unchecked_into::<CanvasRenderingContext2d>())
}

fn detached(canvas: &HtmlCanvasElement) -> Result<HtmlCanvasElement, JsValue> {
    canvas
        .owner_document()
        .ok_or_else(|| JsValue::from_str("canvas has no document"))?
        .create_element("canvas")?
        .dyn_into()
        .map_err(Into::into)
}
