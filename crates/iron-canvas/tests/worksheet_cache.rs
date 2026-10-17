use std::cell::Cell;

use iron_canvas::{
    CanvasSession, OverlayState, RenderBackend, RenderRequest, RenderStrategy, RevisionToken,
    TextMetricsKey, WorksheetCache,
};
use iron_canvas_core::{
    CanvasTheme, CellCoord,
    painter::TextMetrics,
    scene::{GridScene, retained::SceneUpdate},
    scene_geometry::GridRange,
};
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc_base::{UserModel, expressions::types::Area};

const METRICS: TextMetricsKey = TextMetricsKey::new(1, 0);

// Record planning decisions and count measurements without a drawing surface.
// The browser tests compare the corresponding committed pixels with Fresh.
struct Backend {
    width: Cell<f64>,
    measurements: Cell<usize>,
    fail_commit: Cell<bool>,
}
impl Backend {
    fn new(width: f64) -> Self {
        Self {
            width: Cell::new(width),
            measurements: Cell::new(0),
            fail_commit: Cell::new(false),
        }
    }
}
impl TextMetrics for Backend {
    fn measure_text_width(&self, text: &str, _: &str) -> f64 {
        self.measurements.set(self.measurements.get() + 1);
        text.chars().count() as f64 * self.width.get()
    }
}
impl RenderBackend for Backend {
    type StagedFrame = ();
    type Error = &'static str;
    fn metrics(&self) -> &dyn TextMetrics {
        self
    }
    fn retained_scale(&self) -> Option<f64> {
        Some(1.0)
    }
    fn supports_partial_updates(&self) -> bool {
        true
    }
    fn stage_frame(&mut self, _: &GridScene) -> Result<(), Self::Error> {
        Ok(())
    }
    fn stage_update(&mut self, _: &GridScene, _: &SceneUpdate) -> Result<(), Self::Error> {
        Ok(())
    }
    fn commit_frame(&mut self, _: ()) -> Result<(), Self::Error> {
        if self.fail_commit.get() {
            return Err("commit failed");
        }
        Ok(())
    }
}

fn fixture() -> (UserModel<'static>, RenderRequest) {
    let mut model = UserModel::new_empty("cache", "en", "UTC", "en").unwrap();
    model.new_sheet().unwrap();
    for sheet in 0..2 {
        model
            .set_user_input(sheet, 1, 1, "a long text for line breaking")
            .unwrap();
        model
            .update_range_style(
                &Area {
                    sheet,
                    row: 1,
                    column: 1,
                    width: 1,
                    height: 1,
                },
                "alignment.wrap_text",
                "true",
            )
            .unwrap();
        model.set_rows_height(sheet, 1, 1, 80.0).unwrap();
        model
            .set_user_input(sheet, 2, 1, &format!("sheet {sheet}"))
            .unwrap();
    }
    let request = RenderRequest {
        sheet: 0,
        revision: RevisionToken {
            workbook_id: 1,
            revision: 0,
        },
        viewport: WorksheetViewport {
            first_cell: CellCoord { row: 1, col: 1 },
            width: 400,
            height: 200,
        },
        theme: CanvasTheme::light(),
        overlays: Default::default(),
    };
    (model, request)
}

fn assert_fresh(session: &CanvasSession<Backend>, model: &UserModel<'_>, request: &RenderRequest) {
    let mut fresh = CanvasSession::new(Backend::new(session.backend().width.get()));
    fresh.render_fresh(model, request).unwrap();
    let actual = &session.committed_frame().unwrap().worksheet;
    let expected = &fresh.committed_frame().unwrap().worksheet;
    assert_eq!(actual.scene, expected.scene);
    assert_eq!(actual.geometry, expected.geometry);
    assert_eq!(actual.cells, expected.cells);
    assert_eq!(
        session.hit_test(iron_canvas_core::Point { x: 100, y: 100 }),
        fresh.hit_test(iron_canvas_core::Point { x: 100, y: 100 })
    );
}

#[test]
fn text_layout_survives_revision_changes_and_fresh_bypasses_it() {
    let (model, mut request) = fixture();
    let mut session = CanvasSession::new(Backend::new(6.0));
    session.render(&model, &request).unwrap();
    let cold_calls = session.backend().measurements.get();
    let scene = session.committed_frame().unwrap().worksheet.scene.clone();

    // A changed revision forces worksheet preparation with identical text.
    request.revision.revision += 1;
    let outcome = session.render(&model, &request).unwrap();
    assert!(outcome.extended_style_reads > 0);
    let warm_calls = session.backend().measurements.get() - cold_calls;
    assert_eq!(warm_calls, 1, "only the header width needs measurement");
    assert!(cold_calls > warm_calls);
    assert_eq!(session.committed_frame().unwrap().worksheet.scene, scene);

    let before = session.backend().measurements.get();
    session.render_fresh(&model, &request).unwrap();
    let fresh_calls = session.backend().measurements.get() - before;
    assert_eq!(fresh_calls, cold_calls);
    assert_eq!(session.committed_frame().unwrap().worksheet.scene, scene);
    eprintln!(
        "text layout calls per frame: cold={cold_calls}, warm={warm_calls}, Fresh={fresh_calls}"
    );
}

#[test]
fn local_metrics_and_explicit_zero_key_have_separate_cache_scopes() {
    let (model, mut request) = fixture();
    let mut session = CanvasSession::new(Backend::new(3.0));
    let mut cache = WorksheetCache::new(0);
    session.render(&model, &request).unwrap();
    session.backend().width.set(14.0);
    session
        .render_cached(&mut cache, &model, &request, TextMetricsKey::new(0, 0))
        .unwrap();
    assert_fresh(&session, &model, &request);
    session.backend().width.set(7.0);
    request.revision.revision += 1;
    session.render(&model, &request).unwrap();
    assert_fresh(&session, &model, &request);
}

#[test]
fn backend_mutation_clears_local_text_layout() {
    let (model, request) = fixture();
    let mut session = CanvasSession::new(Backend::new(3.0));
    session.render(&model, &request).unwrap();
    session.backend_mut().width.set(14.0);
    session.render(&model, &request).unwrap();
    assert_fresh(&session, &model, &request);
}

#[test]
fn failed_frame_cannot_leave_layout_from_a_different_metric_scope() {
    let (model, mut request) = fixture();
    let mut session = CanvasSession::new(Backend::new(3.0));
    let mut cache = WorksheetCache::new(1);
    let zero_key = TextMetricsKey::new(0, 0);
    session
        .render_cached(&mut cache, &model, &request, zero_key)
        .unwrap();
    let committed = session.committed_frame().unwrap().clone();

    // A failed local render can fill the text cache without publishing a frame.
    // Local metrics and an explicit zero key still identify different scopes.
    session.backend().width.set(14.0);
    session.backend().fail_commit.set(true);
    assert!(session.render(&model, &request).is_err());
    assert_eq!(session.committed_frame(), Some(&committed));
    session.backend().width.set(3.0);
    session.backend().fail_commit.set(false);
    request.revision.revision += 1;
    session
        .render_cached(&mut cache, &model, &request, zero_key)
        .unwrap();
    assert_fresh(&session, &model, &request);
}

#[test]
fn tab_return_skips_all_measurements_and_reads_but_keeps_pixel_plan_fresh() {
    let (model, mut request) = fixture();
    let mut session = CanvasSession::new(Backend::new(6.0));
    let mut cache = WorksheetCache::new(2);
    for sheet in [0, 1, 0] {
        request.sheet = sheet;
        let measurements = session.backend().measurements.get();
        let outcome = session
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap();
        assert_eq!(outcome.strategy, RenderStrategy::Fresh);
        if outcome.sequence == 3 {
            assert!(outcome.content_cached);
            assert_eq!(outcome.extended_style_reads, 0);
            assert_eq!(session.backend().measurements.get(), measurements);
        } else {
            assert!(!outcome.content_cached);
            assert!(outcome.extended_style_reads > 0);
        }
        assert_fresh(&session, &model, &request);
    }
    assert_eq!(cache.len(), 2);
    assert_eq!(
        session
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap()
            .strategy,
        RenderStrategy::Reuse
    );
}

#[test]
fn shared_content_excludes_overlays_and_fingerprints_belong_to_each_session() {
    let (model, mut request) = fixture();
    let mut cache = WorksheetCache::new(2);
    let mut first = CanvasSession::new(Backend::new(6.0));
    let mut second = CanvasSession::new(Backend::new(6.0));
    request.overlays.selection = vec![GridRange {
        first: CellCoord { row: 1, col: 1 },
        last: CellCoord { row: 2, col: 2 },
    }];
    first
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    request.overlays = OverlayState::default();
    assert!(
        second
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap()
            .content_cached
    );
    assert_fresh(&second, &model, &request);
    // An equal request in the first session removes its prior selection.
    assert_ne!(
        first
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap()
            .strategy,
        RenderStrategy::Reuse
    );
    assert_fresh(&first, &model, &request);
    assert_eq!(
        second
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap()
            .strategy,
        RenderStrategy::Reuse
    );
}

#[test]
fn metric_profiles_generations_and_invalidation_rebuild_layout() {
    let (model, request) = fixture();
    let mut cache = WorksheetCache::new(2);
    let mut narrow = CanvasSession::new(Backend::new(3.0));
    let mut wide = CanvasSession::new(Backend::new(14.0));
    narrow
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    let narrow_scene = narrow.committed_frame().unwrap().worksheet.scene.clone();
    let wide_key = TextMetricsKey::new(2, 0);
    let result = wide
        .render_cached(&mut cache, &model, &request, wide_key)
        .unwrap();
    assert!(!result.content_cached);
    assert!(!result.reused_geometry);
    assert_ne!(
        wide.committed_frame().unwrap().worksheet.scene,
        narrow_scene
    );
    assert_fresh(&wide, &model, &request);

    wide.backend().width.set(10.0);
    let result = wide
        .render_cached(&mut cache, &model, &request, TextMetricsKey::new(2, 1))
        .unwrap();
    assert!(!result.content_cached);
    assert_eq!(result.strategy, RenderStrategy::Fresh);
    assert_fresh(&wide, &model, &request);
    // Explicit invalidation takes effect even if the caller repeats the key.
    wide.backend().width.set(8.0);
    wide.invalidate();
    let result = wide
        .render_cached(&mut cache, &model, &request, TextMetricsKey::new(2, 1))
        .unwrap();
    assert!(!result.content_cached);
    assert!(result.extended_style_reads > 0);
    assert_eq!(result.strategy, RenderStrategy::Fresh);
    assert_fresh(&wide, &model, &request);
}

#[test]
fn revisions_theme_viewport_and_workbook_identity_follow_validity_rules() {
    let (mut model, mut request) = fixture();
    let mut cache = WorksheetCache::new(2);
    let mut session = CanvasSession::new(Backend::new(6.0));
    session
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    model.set_user_input(0, 2, 1, "changed").unwrap();
    request.revision.revision += 1;
    let result = session
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    assert!(!result.content_cached);
    assert!(result.reused_geometry);
    assert!(result.extended_style_reads > 0);
    assert_fresh(&session, &model, &request);
    request.viewport.first_cell.row += 1;
    assert!(
        !session
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap()
            .content_cached
    );
    assert_fresh(&session, &model, &request);
    assert_eq!(cache.len(), 1);
    request.theme = CanvasTheme::dark();
    let result = session
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    assert!(!result.content_cached);
    assert!(!result.reused_geometry);
    assert_eq!(result.strategy, RenderStrategy::Fresh);
    assert_fresh(&session, &model, &request);
    request.revision.workbook_id += 1;
    assert!(
        !session
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap()
            .content_cached
    );
    assert_fresh(&session, &model, &request);
    assert_eq!(cache.len(), 2);
}

#[test]
fn capacity_zero_and_fresh_bypass_shared_content() {
    let (model, request) = fixture();
    let mut session = CanvasSession::new(Backend::new(6.0));
    let mut cache = WorksheetCache::new(0);
    for _ in 0..2 {
        let result = session
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap();
        assert!(!result.content_cached);
        assert!(result.extended_style_reads > 0);
        assert!(cache.is_empty());
    }
    cache.set_capacity(1);
    session
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    let result = session.render_fresh(&model, &request).unwrap();
    assert!(!result.content_cached);
    assert!(result.extended_style_reads > 0);
    assert_eq!(result.strategy, RenderStrategy::Fresh);
    assert!(
        session
            .render_cached(&mut cache, &model, &request, METRICS)
            .unwrap()
            .content_cached
    );
    // A plain render cannot reuse content under an untracked metric key.
    assert!(
        session
            .render(&model, &request)
            .unwrap()
            .extended_style_reads
            > 0
    );
    assert_fresh(&session, &model, &request);
}

#[test]
fn empty_viewport_read_count_does_not_imply_a_cache_hit() {
    let (model, mut request) = fixture();
    request.viewport.width = 1;
    request.viewport.height = 1;
    let mut cache = WorksheetCache::new(1);
    let mut session = CanvasSession::new(Backend::new(6.0));
    let first = session
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    assert_eq!(first.cell_count, 0);
    assert_eq!(first.extended_style_reads, 0);
    assert!(!first.content_cached);
    let second = session
        .render_cached(&mut cache, &model, &request, METRICS)
        .unwrap();
    assert!(second.content_cached);
    assert_eq!(second.extended_style_reads, 0);
}
