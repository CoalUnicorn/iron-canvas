use super::allocator::{BYTES, CALLS};
use iron_canvas::{CanvasSession, RenderBackend, RenderRequest, RevisionToken};
use iron_canvas_core::{
    CanvasTheme, CellCoord,
    painter::TextMetrics,
    scene::{GridScene, Primitive},
};
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc_base::UserModel;
use std::{cell::Cell, collections::HashSet, fmt::Write, rc::Rc, sync::atomic::Ordering::Relaxed};

struct Metered<B, F> {
    inner: B,
    clock: Rc<F>,
    paint: Rc<Cell<f64>>,
}
impl<B: RenderBackend, F: Fn() -> f64> RenderBackend for Metered<B, F> {
    type StagedFrame = B::StagedFrame;
    type Error = B::Error;
    fn metrics(&self) -> &dyn TextMetrics {
        self.inner.metrics()
    }
    fn stage_frame(&mut self, scene: &GridScene) -> Result<Self::StagedFrame, Self::Error> {
        let t = (self.clock)();
        let r = self.inner.stage_frame(scene);
        self.paint.set(self.paint.get() + (self.clock)() - t);
        r
    }
    // M3_START
    fn retained_scale(&self) -> Option<f64> {
        self.inner.retained_scale()
    }
    fn supports_partial_updates(&self) -> bool {
        self.inner.supports_partial_updates()
    }
    fn stage_update(
        &mut self,
        scene: &GridScene,
        u: &iron_canvas_core::scene::retained::SceneUpdate,
    ) -> Result<Self::StagedFrame, Self::Error> {
        let t = (self.clock)();
        let r = self.inner.stage_update(scene, u);
        self.paint.set(self.paint.get() + (self.clock)() - t);
        r
    }
    // M3_END
    fn commit_frame(&mut self, s: Self::StagedFrame) -> Result<(), Self::Error> {
        let t = (self.clock)();
        let r = self.inner.commit_frame(s);
        self.paint.set(self.paint.get() + (self.clock)() - t);
        r
    }
}
/// Descriptive samples, not a statistical regression threshold. Times are microseconds.
pub fn run<B: RenderBackend, F: Fn() -> f64>(backend: B, clock: F) -> String
where
    B::Error: std::fmt::Debug,
{
    let mut model = UserModel::new_empty("measure", "en", "UTC", "en").unwrap();
    model.set_rows_height(0, 1, 200, 24.0).unwrap();
    model.set_columns_width(0, 1, 30, 80.0).unwrap();
    for row in 1..100 {
        for col in 1..20 {
            model
                .set_user_input(0, row, col, &format!("R{row} C{col}"))
                .unwrap();
        }
    }
    let mut request = RenderRequest {
        sheet: 0,
        revision: RevisionToken {
            workbook_id: 1,
            revision: 0,
        },
        viewport: WorksheetViewport {
            first_cell: CellCoord { row: 1, col: 1 },
            width: 1100,
            height: 700,
        },
        theme: CanvasTheme::light(),
        overlays: Default::default(),
    };
    let clock = Rc::new(clock);
    let paint = Rc::new(Cell::new(0.0));
    let mut session = CanvasSession::new(Metered {
        inner: backend,
        clock: Rc::clone(&clock),
        paint: Rc::clone(&paint),
    });
    let mut output = String::new();
    for scenario in ["stable", "edit", "scroll"] {
        let mut samples = Vec::with_capacity(50);
        for i in 0..55 {
            if scenario == "edit" {
                model.set_user_input(0, 6, 3, &i.to_string()).unwrap();
                request.revision.revision += 1;
            }
            if scenario == "scroll" {
                request.viewport.first_cell.row = 1 + i % 10;
            }
            paint.set(0.0);
            let calls = CALLS.load(Relaxed);
            let bytes = BYTES.load(Relaxed);
            let start = clock();
            std::hint::black_box(session.render(&model, &request).unwrap());
            let elapsed = clock() - start;
            let count = CALLS.load(Relaxed) - calls;
            let allocated = BYTES.load(Relaxed) - bytes;
            if i >= 5 {
                samples.push((elapsed, paint.get(), count, allocated));
            }
        }
        samples.sort_by(|a, b| a.0.total_cmp(&b.0));
        let s = samples[samples.len() / 2];
        let worksheet = &session.committed_frame().unwrap().worksheet;
        writeln!(output,"{scenario}: frame_us={:.1} prepare_plan_us={:.1} stage_commit_us={:.1} rust_alloc_calls={} rust_alloc_bytes={} scene_heap_bytes={} cells={} style_reads={}",s.0,s.0-s.1,s.1,s.2,s.3,scene_bytes(&worksheet.scene),worksheet.cells.len(),worksheet.extended_style_reads).unwrap();
    }
    output
}
/// Owned scene heap estimate; shared resources are counted once. Excludes
/// allocator headers, session metadata, backend surfaces and foreign heaps.
fn scene_bytes(scene: &GridScene) -> usize {
    let mut bytes = scene.passes.capacity() * std::mem::size_of_val(&scene.passes[0]);
    let mut seen = HashSet::new();
    let mut resource = |ptr: usize, size: usize| {
        if seen.insert(ptr) {
            size + 2 * std::mem::size_of::<usize>()
        } else {
            0
        }
    };
    for pass in &scene.passes {
        bytes += resource(pass.as_ptr() as usize, std::mem::size_of_val(pass.as_ref()));
        for op in pass.iter() {
            match op {
                Primitive::RectFill { color, .. }
                | Primitive::Circle { color, .. }
                | Primitive::Line { color, .. }
                | Primitive::TextLine { color, .. } => {
                    bytes += resource(color.as_ptr() as usize, color.len())
                }
                Primitive::Gradient { from, to, .. } => {
                    bytes += resource(from.as_ptr() as usize, from.len());
                    bytes += resource(to.as_ptr() as usize, to.len());
                }
                Primitive::FillPath { path, color } => {
                    bytes += resource(path.as_ptr() as usize, std::mem::size_of_val(path.as_ref()));
                    bytes += resource(color.as_ptr() as usize, color.len());
                }
                Primitive::StrokePath { path, color, style } => {
                    bytes += resource(path.as_ptr() as usize, std::mem::size_of_val(path.as_ref()));
                    bytes += resource(color.as_ptr() as usize, color.len());
                    bytes += resource(
                        style.dash.as_ptr() as usize,
                        std::mem::size_of_val(style.dash.as_ref()),
                    );
                }
                Primitive::Text {
                    text,
                    font_css,
                    color,
                    ..
                } => {
                    bytes += text.capacity();
                    bytes += resource(font_css.as_ptr() as usize, font_css.len());
                    bytes += resource(color.as_ptr() as usize, color.len());
                }
                Primitive::Clip { .. } | Primitive::EndClip => {}
            }
        }
    }
    bytes
}
