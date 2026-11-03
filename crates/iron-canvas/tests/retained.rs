#![cfg(not(target_arch = "wasm32"))]
use iron_canvas::{CanvasSession, RenderBackend, RenderRequest, RenderStrategy, RevisionToken};
use iron_canvas_core::{
    CanvasTheme, CellCoord, PixelRect, Point,
    painter::TextMetrics,
    scene::{
        GridScene,
        retained::{SceneUpdate, pass_bounds},
    },
    scene_geometry::GridRange,
};
use iron_canvas_ironcalc::WorksheetViewport;
use iron_canvas_native::NativeSceneBackend;
use ironcalc_base::{UserModel, expressions::types::Area};
use std::{cell::RefCell, rc::Rc};
fn fixture() -> (UserModel<'static>, RenderRequest) {
    let mut m = UserModel::new_empty("retained", "en", "UTC", "en").unwrap();
    m.set_rows_height(0, 1, 100, 24.0).unwrap();
    m.set_columns_width(0, 1, 20, 80.0).unwrap();
    for r in 1..40 {
        for c in 1..8 {
            m.set_user_input(0, r, c, &format!("{r}-{c}")).unwrap();
        }
    }
    let r = RenderRequest {
        sheet: 0,
        revision: RevisionToken {
            workbook_id: 1,
            revision: 0,
        },
        viewport: WorksheetViewport::aligned(CellCoord { row: 1, col: 1 }, 640, 400),
        theme: CanvasTheme::light(),
        overlays: Default::default(),
    };
    (m, r)
}
fn pixels(s: &CanvasSession<NativeSceneBackend>) -> Vec<u8> {
    let mut bytes = Vec::new();
    s.backend()
        .surface()
        .unwrap()
        .with_data(|data| bytes.extend_from_slice(data))
        .unwrap();
    bytes
}
fn compare(
    s: &CanvasSession<NativeSceneBackend>,
    m: &UserModel<'_>,
    r: &RenderRequest,
    scale: f64,
) {
    let mut fresh = CanvasSession::new(NativeSceneBackend::new(scale).unwrap());
    fresh.render_fresh(m, r).unwrap();
    let a = pixels(s);
    let b = pixels(&fresh);
    let differences = a.iter().zip(&b).filter(|(a, b)| a != b).count();
    assert_eq!(differences, 0, "retained pixels must match Fresh");
}

#[test]
fn native_spill_matches_fresh_after_offscreen_source_and_blocker_edits() {
    for (scale, source_width) in [1.0, 1.25, 2.0]
        .into_iter()
        .flat_map(|scale| [1.0, 60.0].map(|width| (scale, width)))
    {
        let (mut model, mut request) = fixture();
        model
            .range_clear_all(&Area {
                sheet: 0,
                row: 1,
                column: 1,
                width: 20,
                height: 100,
            })
            .unwrap();
        model.set_columns_width(0, 1, 20, 60.0).unwrap();
        model.set_columns_width(0, 1, 1, source_width).unwrap();
        request.viewport.scroll.first_cell.col = 2;
        model
            .set_user_input(0, 4, 1, "Offscreen text extends across empty cells")
            .unwrap();
        let mut session = CanvasSession::new(NativeSceneBackend::new(scale).unwrap());
        session.render(&model, &request).unwrap();
        compare(&session, &model, &request, scale);
        let before = pixels(&session);
        model.set_user_input(0, 4, 2, "=\"\"").unwrap();
        request.revision.revision += 1;
        session.render(&model, &request).unwrap();
        compare(&session, &model, &request, scale);
        assert_ne!(pixels(&session), before);
        model
            .range_clear_contents(&Area {
                sheet: 0,
                row: 4,
                column: 2,
                width: 1,
                height: 1,
            })
            .unwrap();
        request.revision.revision += 1;
        session.render(&model, &request).unwrap();
        compare(&session, &model, &request, scale);
        assert_eq!(pixels(&session), before);
    }
}

#[path = "support/text_spill.rs"]
mod text_spill;

#[test]
fn native_spill_paints_over_fills_and_keeps_borders_after_style_changes() {
    for scale in [1.0, 1.25, 2.0] {
        let (mut model, mut request) = text_spill::fixture();
        let mut session = CanvasSession::new(NativeSceneBackend::new(scale).unwrap());
        session.render(&model, &request).unwrap();
        for color in ["#ffffff", "#00bfff", "#ffffff"] {
            text_spill::format(&mut model, color);
            request.revision.revision += 1;
            session.render(&model, &request).unwrap();
            compare(&session, &model, &request, scale);
            let before = pixels(&session);
            let cell = session.cell_rect(2, 2).unwrap();
            let stride = session.backend().surface().unwrap().stride() as usize;
            model.set_user_input(0, 2, 1, "").unwrap();
            request.revision.revision += 1;
            session.render(&model, &request).unwrap();
            compare(&session, &model, &request, scale);
            assert_ne!(
                text_spill::interior_pixels(&before, stride, cell, scale),
                text_spill::interior_pixels(&pixels(&session), stride, cell, scale),
                "text must reach the formatted destination at scale {scale}"
            );
            model.set_user_input(0, 2, 1, text_spill::TEXT).unwrap();
            request.revision.revision += 1;
            session.render(&model, &request).unwrap();
            compare(&session, &model, &request, scale);
            assert_eq!(pixels(&session), before);
        }
    }
}
#[test]
fn native_reuses_stable_frames_and_falls_back_for_partial_work() {
    for scale in [1.0, 1.25, 1.5] {
        let (mut model, mut request) = fixture();
        let mut s = CanvasSession::new(NativeSceneBackend::new(scale).unwrap());
        assert_eq!(
            s.render(&model, &request).unwrap().strategy,
            RenderStrategy::Fresh
        );
        let same = s.render(&model, &request).unwrap();
        assert_eq!(same.strategy, RenderStrategy::Reuse);
        assert_eq!(same.extended_style_reads, 0);
        assert!(same.reused_geometry);
        for value in ["changed", "6-3", ""] {
            model.set_user_input(0, 6, 3, value).unwrap();
            request.revision.revision += 1;
            let result = s.render(&model, &request).unwrap();
            assert_eq!(result.strategy, RenderStrategy::Fresh);
            assert!(result.reused_geometry);
            assert!(result.extended_style_reads > 0);
            compare(&s, &model, &request, scale);
        }
        for row in [2, 3, 2, 1] {
            request.viewport.scroll.first_cell.row = row;
            let result = s.render(&model, &request).unwrap();
            assert_eq!(result.strategy, RenderStrategy::Fresh);
            compare(&s, &model, &request, scale);
        }
    }
}
#[test]
fn overlays_metrics_and_merges_keep_cache_boundaries() {
    let (mut model, mut request) = fixture();
    let mut s = CanvasSession::new(NativeSceneBackend::new(1.25).unwrap());
    s.render(&model, &request).unwrap();
    request.overlays.selection = vec![GridRange {
        first: CellCoord { row: 4, col: 2 },
        last: CellCoord { row: 8, col: 5 },
    }];
    request.overlays.active_cell = Some(CellCoord { row: 5, col: 3 });
    assert_eq!(s.render(&model, &request).unwrap().extended_style_reads, 0);
    compare(&s, &model, &request, 1.25);
    request.overlays = Default::default();
    s.render(&model, &request).unwrap();
    compare(&s, &model, &request, 1.25);
    s.invalidate();
    assert_eq!(
        s.render(&model, &request).unwrap().strategy,
        RenderStrategy::Fresh
    );
    for row in 2..5 {
        for col in 2..4 {
            model.set_user_input(0, row, col, "").unwrap();
        }
    }
    model
        .merge_cells(&Area {
            sheet: 0,
            row: 2,
            column: 2,
            width: 2,
            height: 3,
        })
        .unwrap();
    request.revision.revision += 1;
    assert_eq!(
        s.render(&model, &request).unwrap().strategy,
        RenderStrategy::Fresh
    );
    compare(&s, &model, &request, 1.25);
}

#[path = "support/text_wrap.rs"]
mod text_wrap;

#[test]
fn native_unicode_wrap_and_autofit_match_fresh_after_font_changes() {
    use iron_canvas_core::scene::RenderBackend;
    for scale in [1.0, 1.25, 2.0] {
        let (mut model, mut request) = text_wrap::fixture();
        let mut session = CanvasSession::new(NativeSceneBackend::new(scale).unwrap());
        text_wrap::fit(&mut model, session.backend().metrics());
        session.render(&model, &request).unwrap();
        compare(&session, &model, &request, scale);
        for step in 0..6 {
            text_wrap::update(&mut model, step);
            text_wrap::fit(&mut model, session.backend().metrics());
            request.revision.revision += 1;
            session.render(&model, &request).unwrap();
            compare(&session, &model, &request, scale);
        }
        session.invalidate();
        session.render(&model, &request).unwrap();
        compare(&session, &model, &request, scale);
    }
}

struct ProbeBackend {
    metrics: NativeSceneBackend,
    updates: Rc<RefCell<Vec<SceneUpdate>>>,
    scale: f64,
}
impl RenderBackend for ProbeBackend {
    type StagedFrame = ();
    type Error = &'static str;
    fn metrics(&self) -> &dyn TextMetrics {
        self.metrics.metrics()
    }
    fn stage_frame(&mut self, _scene: &GridScene) -> Result<(), Self::Error> {
        Ok(())
    }
    fn commit_frame(&mut self, _staged: ()) -> Result<(), Self::Error> {
        Ok(())
    }
    fn supports_partial_updates(&self) -> bool {
        true
    }
    fn retained_scale(&self) -> Option<f64> {
        Some(self.scale)
    }
    fn stage_update(
        &mut self,
        _scene: &GridScene,
        update: &SceneUpdate,
    ) -> Result<(), Self::Error> {
        self.updates.borrow_mut().push(update.clone());
        Ok(())
    }
}

#[test]
fn selection_overlay_scroll_blits_the_model_and_repaints_overlay_ink() {
    let (model, mut request) = fixture();
    request.overlays.selection = vec![GridRange {
        first: CellCoord { row: 3, col: 2 },
        last: CellCoord { row: 5, col: 4 },
    }];
    request.overlays.active_cell = Some(CellCoord { row: 4, col: 3 });
    let updates = Rc::new(RefCell::new(Vec::new()));
    let mut session = CanvasSession::new(ProbeBackend {
        metrics: NativeSceneBackend::new(1.0).unwrap(),
        updates: Rc::clone(&updates),
        scale: 1.0,
    });
    session.render(&model, &request).unwrap();
    let frame = session.committed_frame().unwrap();
    assert!(
        frame.worksheet.model_passes < frame.worksheet.scene.passes.len(),
        "model passes={} total={}",
        frame.worksheet.model_passes,
        frame.worksheet.scene.passes.len()
    );
    assert!(!pass_bounds(&frame.worksheet.scene, frame.worksheet.model_passes).is_empty());
    request.viewport.scroll.offset_y = 6;
    session.render(&model, &request).unwrap();
    let update = updates.borrow().last().cloned().unwrap();
    let SceneUpdate::Blit {
        source,
        destination,
        repaint,
    } = update
    else {
        panic!("expected Blit, got {update:?}");
    };
    let copy = PixelRect {
        top_left: destination,
        ..source
    };
    for cell in [
        CellCoord { row: 3, col: 2 },
        CellCoord { row: 5, col: 4 },
        CellCoord { row: 4, col: 3 },
    ] {
        let rect = session.cell_rect(cell.row, cell.col).unwrap();
        assert!(
            copy.intersection(rect).is_none(),
            "copy covers overlay ink at {cell:?}: {copy:?} {rect:?}"
        );
        assert!(
            repaint
                .iter()
                .any(|region| region.intersection(rect).is_some()),
            "repaint misses overlay ink at {cell:?}: {repaint:?} {rect:?}"
        );
    }
}

#[test]
fn moving_active_cell_keeps_its_old_rect_out_of_the_blit_copy() {
    let (model, mut request) = fixture();
    request.overlays.selection = vec![GridRange {
        first: CellCoord { row: 1, col: 2 },
        last: CellCoord { row: 1, col: 3 },
    }];
    request.overlays.active_cell = Some(CellCoord { row: 1, col: 2 });
    let updates = Rc::new(RefCell::new(Vec::new()));
    let mut session = CanvasSession::new(ProbeBackend {
        metrics: NativeSceneBackend::new(1.0).unwrap(),
        updates: Rc::clone(&updates),
        scale: 1.0,
    });
    session.render(&model, &request).unwrap();
    for (row, offset) in [(2, 0), (2, 6), (3, 0), (3, 8), (2, 4), (1, 0)] {
        let old_rect = session.cell_rect(4, 3).unwrap();
        let _ = old_rect;
        let old_active = request.overlays.active_cell.unwrap();
        let old_rect = session.cell_rect(old_active.row, old_active.col).unwrap();
        request.viewport.scroll.first_cell.row = row;
        request.viewport.scroll.offset_y = offset;
        request.overlays.selection = vec![GridRange {
            first: CellCoord { row, col: 2 },
            last: CellCoord { row, col: 3 },
        }];
        request.overlays.active_cell = Some(CellCoord { row, col: 2 });
        session.render(&model, &request).unwrap();
        let update = updates.borrow().last().cloned().unwrap();
        let SceneUpdate::Blit {
            source,
            destination,
            ..
        } = update
        else {
            panic!("expected Blit, got {update:?}");
        };
        let copy = PixelRect {
            top_left: destination,
            ..source
        };
        // Old overlay pixels enter the copy from the source frame, so their
        // destination-space position is shifted by the copy translation.
        let dy = destination.y - source.top();
        let old_in_destination = PixelRect {
            top_left: Point {
                x: old_rect.left(),
                y: old_rect.top() + dy,
            },
            ..old_rect
        };
        assert!(
            copy.intersection(old_in_destination).is_none(),
            "copy takes old active pixels {old_in_destination:?}: {copy:?} row={row} offset={offset}"
        );
    }
}
