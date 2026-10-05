//! Cairo/Pango rendering of resolved grid scenes. The host owns presentation.
#![cfg(not(target_arch = "wasm32"))]
mod painter;
use cairo::{Format, ImageSurface};
use iron_canvas_core::{
    painter::TextMetrics,
    scene::{GridScene, RenderBackend},
};
pub use painter::NativePainter;

pub struct NativeSceneBackend {
    metrics: NativePainter,
    scale: f64,
    committed: Option<ImageSurface>,
}
impl NativeSceneBackend {
    pub fn new(scale: f64) -> Result<Self, cairo::Error> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(cairo::Error::InvalidMatrix);
        }
        Ok(Self {
            metrics: NativePainter::new(&ImageSurface::create(Format::ARgb32, 1, 1)?)?,
            scale,
            committed: None,
        })
    }
    /// The surface is read-only to consumers by convention. A host can paint
    /// it into its window or encode it with `write_to_png`.
    pub fn surface(&self) -> Option<&ImageSurface> {
        self.committed.as_ref()
    }
}
impl RenderBackend for NativeSceneBackend {
    type StagedFrame = ImageSurface;
    type Error = cairo::Error;
    fn metrics(&self) -> &dyn TextMetrics {
        &self.metrics
    }
    fn stage_frame(&mut self, scene: &GridScene) -> Result<ImageSurface, cairo::Error> {
        if scene.clip.left() != 0
            || scene.clip.top() != 0
            || scene.clip.width < 0
            || scene.clip.height < 0
        {
            return Err(cairo::Error::InvalidSize);
        }
        let dimension = |size: i32| {
            let scaled = (f64::from(size) * self.scale).ceil();
            if scaled > f64::from(i32::MAX) {
                Err(cairo::Error::InvalidSize)
            } else {
                Ok(scaled as i32)
            }
        };
        let surface = ImageSurface::create(
            Format::ARgb32,
            dimension(scene.clip.width)?,
            dimension(scene.clip.height)?,
        )?;
        let painter = NativePainter::new(&surface)?;
        painter.context().scale(self.scale, self.scale);
        scene.paint(&painter);
        painter.status()?;
        surface.flush();
        surface.status()?;
        Ok(surface)
    }
    fn commit_frame(&mut self, staged: ImageSurface) -> Result<(), cairo::Error> {
        staged.status()?;
        self.committed = Some(staged);
        Ok(())
    }
}
