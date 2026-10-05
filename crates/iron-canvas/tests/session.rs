use iron_canvas::{CanvasSession, RenderBackend, RenderError, RenderRequest, RevisionToken};
use iron_canvas_core::{
    CanvasTheme, CellCoord, Point, painter::TextMetrics, scene::GridScene,
    scene_geometry::GridRange,
};
use iron_canvas_export::SvgSceneBackend;
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc_base::UserModel;

struct FailingBackend {
    svg: SvgSceneBackend,
    fail_stage: bool,
    fail_commit: bool,
    stages: usize,
}
impl RenderBackend for FailingBackend {
    type StagedFrame = String;
    type Error = &'static str;
    fn metrics(&self) -> &dyn TextMetrics {
        self.svg.metrics()
    }
    fn stage_frame(&mut self, scene: &GridScene) -> Result<String, Self::Error> {
        self.stages += 1;
        if self.fail_stage {
            return Err("stage");
        }
        Ok(self.svg.stage_frame(scene).unwrap())
    }
    fn commit_frame(&mut self, staged: String) -> Result<(), Self::Error> {
        if self.fail_commit {
            return Err("commit");
        }
        self.svg.commit_frame(staged).unwrap();
        Ok(())
    }
}
fn request() -> RenderRequest {
    RenderRequest {
        sheet: 0,
        revision: RevisionToken {
            workbook_id: 1,
            revision: 1,
        },
        viewport: WorksheetViewport {
            first_cell: CellCoord { row: 1, col: 1 },
            width: 200,
            height: 80,
        },
        theme: CanvasTheme::light(),
        overlays: Default::default(),
    }
}
#[test]
fn errors_preserve_visible_svg_and_committed_scene_then_retry_advances_once() {
    let mut workbook = UserModel::new_empty("test", "en", "UTC", "en").unwrap();
    workbook.set_user_input(0, 1, 1, "before").unwrap();
    let mut session = CanvasSession::new(FailingBackend {
        svg: SvgSceneBackend::new(),
        fail_stage: false,
        fail_commit: false,
        stages: 0,
    });
    let mut request = request();
    session.render(&workbook, &request).unwrap();
    let committed = session.committed_frame().cloned();
    let visible = session.backend().svg.document().unwrap().to_owned();
    workbook.set_user_input(0, 1, 1, "after").unwrap();
    request.revision.revision += 1;
    for (stage, commit) in [(true, false), (false, true)] {
        session.backend_mut().fail_stage = stage;
        session.backend_mut().fail_commit = commit;
        assert!(matches!(
            session.render(&workbook, &request),
            Err(RenderError::Backend(_))
        ));
        assert_eq!(session.committed_frame(), committed.as_ref());
        assert_eq!(session.backend().svg.document(), Some(visible.as_str()));
    }
    session.backend_mut().fail_commit = false;
    request.sheet = u32::MAX;
    request.viewport.width = 0;
    let stages = session.backend().stages;
    assert!(matches!(
        session.render(&workbook, &request),
        Err(RenderError::Preparation(_))
    ));
    assert_eq!(session.backend().stages, stages);
    assert_eq!(session.committed_frame(), committed.as_ref());
    assert_eq!(session.backend().svg.document(), Some(visible.as_str()));
    request.sheet = 0;
    request.viewport.width = 200;
    assert_eq!(session.render(&workbook, &request).unwrap().sequence, 2);
    assert!(session.backend().svg.document().unwrap().contains("after"));
    assert_ne!(session.committed_frame(), committed.as_ref());
}

#[test]
fn revision_theme_and_workbook_replacement_recompile_visible_content() {
    let mut workbook = UserModel::new_empty("first", "en", "UTC", "en").unwrap();
    workbook.set_user_input(0, 1, 1, "=Z100").unwrap();
    workbook.set_user_input(0, 100, 26, "17").unwrap();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    let mut request = request();
    session.render(&workbook, &request).unwrap();
    assert!(session.backend().document().unwrap().contains(">17</text>"));
    workbook.set_user_input(0, 100, 26, "23").unwrap();
    request.revision.revision += 1;
    request.theme.default_text_color = "#654321".into();
    session.render(&workbook, &request).unwrap();
    assert!(session.backend().document().unwrap().contains(">23</text>"));
    assert!(session.backend().document().unwrap().contains("#654321"));
    let mut replacement = UserModel::new_empty("replacement", "en", "UTC", "en").unwrap();
    replacement.set_user_input(0, 1, 1, "new workbook").unwrap();
    request.revision = RevisionToken {
        workbook_id: 2,
        revision: 0,
    };
    session.render(&replacement, &request).unwrap();
    assert!(
        session
            .backend()
            .document()
            .unwrap()
            .contains("new workbook")
    );
    assert_eq!(
        session.committed_frame().unwrap().revision,
        request.revision
    );
}

#[test]
fn committed_frame_exposes_cell_rects_scroll_pane_and_autofill_handle() {
    let mut workbook = UserModel::new_empty("test", "en", "UTC", "en").unwrap();
    workbook.set_user_input(0, 1, 1, "x").unwrap();
    workbook.set_frozen_rows_count(0, 1).unwrap();
    workbook.set_frozen_columns_count(0, 1).unwrap();
    let mut request = request();
    request.viewport = WorksheetViewport {
        first_cell: CellCoord { row: 4, col: 4 },
        width: 400,
        height: 300,
    };
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    // No committed frame: every geometry query is empty.
    assert!(session.cell_rect(1, 1).is_none());
    assert!(session.scroll_pane_rect().is_none());
    assert!(session.committed_viewport().is_none());

    session.render(&workbook, &request).unwrap();
    assert_eq!(session.committed_viewport(), Some(request.viewport));

    let frozen = session.cell_rect(1, 1).expect("frozen cell");
    let scroll = session.cell_rect(4, 4).expect("scrolling cell");
    let pane = session.scroll_pane_rect().expect("committed scroll pane");
    // The frozen cell sits up-left of the scroll pane's near edge.
    assert!(frozen.right() <= pane.left());
    assert!(frozen.bottom() <= pane.top());
    // The first scrolling cell aligns with the pane's near edge.
    assert_eq!((scroll.left(), scroll.top()), (pane.left(), pane.top()));

    let range = GridRange {
        first: CellCoord { row: 4, col: 4 },
        last: CellCoord { row: 5, col: 5 },
    };
    let last = session.cell_rect(5, 5).expect("range last cell");
    assert_eq!(
        session.autofill_handle(range),
        Some(Point {
            x: last.right(),
            y: last.bottom(),
        })
    );
    // A cell beyond the sampled panes yields no handle.
    assert_eq!(
        session.autofill_handle(GridRange {
            first: CellCoord { row: 1, col: 1 },
            last: CellCoord { row: 1, col: 9999 },
        }),
        None
    );
}
