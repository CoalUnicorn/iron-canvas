//! PDF finalization from the same resolved scene used by interactive backends.
use super::PdfPainter;
use iron_canvas_core::{
    painter::TextMetrics,
    scene::{GridScene, RenderBackend},
};
use std::convert::Infallible;

pub struct PdfSceneBackend {
    metrics: PdfPainter,
    committed: Option<Vec<u8>>,
}
impl Default for PdfSceneBackend {
    fn default() -> Self {
        Self::new()
    }
}
impl PdfSceneBackend {
    pub fn new() -> Self {
        Self {
            metrics: PdfPainter::new(0, 0),
            committed: None,
        }
    }
    pub fn document(&self) -> Option<&[u8]> {
        self.committed.as_deref()
    }
}
impl RenderBackend for PdfSceneBackend {
    type StagedFrame = Vec<u8>;
    type Error = Infallible;
    fn metrics(&self) -> &dyn TextMetrics {
        &self.metrics
    }
    fn stage_frame(&mut self, scene: &GridScene) -> Result<Vec<u8>, Infallible> {
        let width = scene.clip.width.max(0) as u32;
        let height = scene.clip.height.max(0) as u32;
        let painter = PdfPainter::new(width, height);
        scene.paint(&painter);
        Ok(painter.finish())
    }
    fn commit_frame(&mut self, staged: Vec<u8>) -> Result<(), Infallible> {
        self.committed = Some(staged);
        Ok(())
    }
}
