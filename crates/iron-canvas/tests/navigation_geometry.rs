use iron_canvas::{
    CanvasSession, ExtentAxis, NavigationPending, NavigationQueryError, NavigationReadiness,
    RenderRequest, RevisionToken, VisibilityAdjustment,
};
use iron_canvas_core::{CanvasTheme, CellCoord};
use iron_canvas_export::SvgSceneBackend;
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc_base::UserModel;

fn fixture() -> (UserModel<'static>, RenderRequest) {
    (
        UserModel::new_empty("queries", "en", "UTC", "en").unwrap(),
        RenderRequest {
            sheet: 0,
            revision: RevisionToken {
                workbook_id: 1,
                revision: 0,
            },
            viewport: WorksheetViewport::aligned(CellCoord { row: 1, col: 1 }, 400, 200),
            theme: CanvasTheme::light(),
            overlays: Default::default(),
        },
    )
}
fn pending(
    session: &CanvasSession<SvgSceneBackend>,
    request: &RenderRequest,
    reason: NavigationPending,
) {
    assert!(
        matches!(session.navigation_geometry(request), NavigationReadiness::Pending(actual) if actual == reason)
    );
}

#[test]
fn readiness_rejects_stale_content_size_theme_and_invalidation_but_accepts_live_origins() {
    let (model, request) = fixture();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    pending(&session, &request, NavigationPending::NoFrame);
    session.render(&model, &request).unwrap();
    let stamp = session.interaction_frame().unwrap();
    for (change, reason) in [
        (0, NavigationPending::Workbook),
        (1, NavigationPending::Sheet),
        (2, NavigationPending::Revision),
        (3, NavigationPending::Size),
        (4, NavigationPending::Theme),
    ] {
        let mut expected = request.clone();
        match change {
            0 => expected.revision.workbook_id += 1,
            1 => expected.sheet += 1,
            2 => expected.revision.revision += 1,
            3 => expected.viewport.height += 1,
            4 => expected.theme = CanvasTheme::dark(),
            _ => unreachable!(),
        }
        pending(&session, &expected, reason);
    }
    let mut live = request.clone();
    live.viewport.scroll.first_cell = CellCoord { row: 30, col: 20 };
    live.overlays.active_cell = Some(CellCoord { row: 30, col: 20 });
    assert!(matches!(
        session.navigation_geometry(&live),
        NavigationReadiness::Ready(_)
    ));
    session.invalidate();
    pending(&session, &request, NavigationPending::Invalidated);
    assert_eq!(session.interaction_frame(), Some(stamp));
    let mut invalid = request.clone();
    invalid.viewport.width = -1;
    assert!(session.render(&model, &invalid).is_err());
    pending(&session, &request, NavigationPending::Invalidated);
}

#[test]
fn visibility_uses_each_live_origin_without_mutating_model_or_committed_frame() {
    let (model, mut request) = fixture();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    let target = CellCoord { row: 40, col: 20 };
    assert!(matches!(
        session.scroll_to_show_checked(&model, &request, target),
        Err(NavigationQueryError::Pending(NavigationPending::NoFrame))
    ));
    session.render(&model, &request).unwrap();
    let bytes = model.to_bytes();
    let stamp = session.interaction_frame();
    for origin in [
        CellCoord { row: 40, col: 20 },
        CellCoord { row: 60, col: 30 },
    ] {
        request.viewport.scroll.first_cell = origin;
        assert_eq!(
            session
                .scroll_to_show_checked(&model, &request, origin)
                .unwrap(),
            VisibilityAdjustment::Unchanged
        );
    }
    assert_eq!(
        session
            .scroll_to_show_checked(&model, &request, target)
            .unwrap(),
        VisibilityAdjustment::ScrollTo(target)
    );
    assert_eq!(
        session.scroll_to_show_checked(&model, &request, CellCoord { row: 0, col: 1 }),
        Err(NavigationQueryError::Bounds)
    );
    assert_eq!(model.to_bytes(), bytes);
    assert_eq!(session.interaction_frame(), stamp);
}

#[test]
fn freeze_edits_wait_for_commit_and_zero_panes_do_not_scroll() {
    let (mut model, mut request) = fixture();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    session.render(&model, &request).unwrap();
    model.set_frozen_rows_count(0, 20).unwrap();
    model.set_frozen_columns_count(0, 20).unwrap();
    request.revision.revision += 1;
    pending(&session, &request, NavigationPending::Revision);
    session.render(&model, &request).unwrap();
    request.viewport.scroll.first_cell = CellCoord { row: 21, col: 21 };
    assert_eq!(
        session
            .scroll_to_show_checked(&model, &request, CellCoord { row: 80, col: 80 })
            .unwrap(),
        VisibilityAdjustment::Unchanged
    );
}

#[test]
fn invalid_live_extents_are_structured_errors() {
    let (mut model, request) = fixture();
    // The offscreen extent does not enter the prepared viewport.
    model.set_rows_height(0, 100, 100, f64::MAX).unwrap();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    session.render(&model, &request).unwrap();
    assert!(matches!(
        session.scroll_to_show_checked(&model, &request, CellCoord { row: 100, col: 1 }),
        Err(NavigationQueryError::Extent {
            axis: ExtentAxis::Row,
            index: 100,
            ..
        })
    ));
}
