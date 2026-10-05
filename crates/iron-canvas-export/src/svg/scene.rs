//! Fresh SVG output from a resolved core scene.
use super::SvgPainter;
use iron_canvas_core::{
    painter::TextMetrics,
    scene::{GridScene, RenderBackend},
};
use std::convert::Infallible;

pub struct SvgSceneBackend {
    metrics: SvgPainter,
    committed: Option<String>,
}
impl Default for SvgSceneBackend {
    fn default() -> Self {
        Self::new()
    }
}
impl SvgSceneBackend {
    pub fn new() -> Self {
        Self {
            metrics: SvgPainter::new(0, 0),
            committed: None,
        }
    }
    pub fn document(&self) -> Option<&str> {
        self.committed.as_deref()
    }
}
impl RenderBackend for SvgSceneBackend {
    type StagedFrame = String;
    type Error = Infallible;
    fn metrics(&self) -> &dyn TextMetrics {
        &self.metrics
    }
    fn stage_frame(&mut self, scene: &GridScene) -> Result<String, Infallible> {
        let painter = SvgPainter::new(
            scene.clip.width.max(0) as u32,
            scene.clip.height.max(0) as u32,
        );
        scene.paint(&painter);
        Ok(painter.finish())
    }
    fn commit_frame(&mut self, staged: String) -> Result<(), Infallible> {
        self.committed = Some(staged);
        Ok(())
    }
}
