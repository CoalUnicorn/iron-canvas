//! Atomic Fresh rendering to a fixed-size browser canvas.
use crate::{CanvasPainter, WebSurface};
use iron_canvas_core::{
    CanvasMetrics, PixelRect, Point, Surface,
    painter::{Painter, TextMetrics},
    scene::{GridScene, RenderBackend},
};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

pub struct Canvas2dSceneBackend {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    metrics_painter: CanvasPainter,
    metrics: CanvasMetrics,
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
        let canvas = detached(&self.canvas)?;
        let mut surface = WebSurface::overlay(canvas.clone())?;
        surface.resize(self.metrics);
        surface.painter().invalidate_cache();
        surface.painter().apply_dpr_transform(self.metrics.dpr());
        surface.painter().reset_text_defaults();
        scene.paint(surface.painter());
        Ok(canvas)
    }
    fn commit_frame(&mut self, staged: HtmlCanvasElement) -> Result<(), JsValue> {
        // No resize occurs here. A failed copy leaves the previous pixels intact.
        self.context
            .draw_image_with_html_canvas_element(&staged, 0.0, 0.0)
    }
}
fn detached(canvas: &HtmlCanvasElement) -> Result<HtmlCanvasElement, JsValue> {
    canvas
        .owner_document()
        .ok_or_else(|| JsValue::from_str("canvas has no document"))?
        .create_element("canvas")?
        .dyn_into()
        .map_err(Into::into)
}
