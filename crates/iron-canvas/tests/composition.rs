use iron_canvas::{
    CanvasSession, FormulaOverlay, HostIntent, OverlayState, PointerAction, RenderBackend,
    RenderRequest, RevisionToken,
};
use iron_canvas_core::{
    CanvasTheme, CellCoord, Point,
    painter::TextMetrics,
    scene::{GridScene, Primitive},
    scene_geometry::{GridHit, GridRange},
};
use iron_canvas_export::{PdfSceneBackend, SvgSceneBackend};
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc_base::{UserModel, expressions::types::Area, types::Link};
fn coord(row: i32, col: i32) -> CellCoord {
    CellCoord { row, col }
}
fn range(r1: i32, c1: i32, r2: i32, c2: i32) -> GridRange {
    GridRange {
        first: coord(r1, c1),
        last: coord(r2, c2),
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
            first_cell: coord(5, 5),
            width: 400,
            height: 220,
        },
        theme: CanvasTheme::light(),
        overlays: OverlayState::default(),
    }
}
fn workbook() -> UserModel<'static> {
    let mut w = UserModel::new_empty("M2", "en", "UTC", "en").unwrap();
    w.set_rows_height(0, 1, 20, 24.0).unwrap();
    w.set_columns_width(0, 1, 20, 60.0).unwrap();
    w.set_frozen_rows_count(0, 2).unwrap();
    w.set_frozen_columns_count(0, 2).unwrap();
    w.set_rows_hidden(0, 2, 2, true).unwrap();
    w.set_columns_hidden(0, 2, 2, true).unwrap();
    w
}
#[test]
fn frozen_panes_hidden_axes_headers_and_explicit_sheet_share_hit_geometry() {
    let mut w = workbook();
    w.new_sheet().unwrap();
    let before = w.to_bytes();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    session.render(&w, &request()).unwrap();
    let frame = session.committed_frame().unwrap();
    let geometry = frame.worksheet.geometry.as_ref().unwrap();
    assert_eq!(geometry.panes.len(), 4);
    assert!(
        frame
            .worksheet
            .cells
            .iter()
            .all(|p| p.coordinate.row != 2 && p.coordinate.col != 2)
    );
    assert_eq!(
        session.hit_test(Point { x: 1, y: 1 }),
        Some(GridHit::Corner)
    );
    assert_eq!(
        session.hit_test(Point {
            x: geometry.origin.x + 1,
            y: 1
        }),
        Some(GridHit::ColumnHeader(1))
    );
    assert_eq!(
        session.hit_test(Point {
            x: 1,
            y: geometry.origin.y + 1
        }),
        Some(GridHit::RowHeader(1))
    );
    for p in &geometry.panes {
        assert_eq!(
            session.hit_test(Point {
                x: p.clip.left() + 1,
                y: p.clip.top() + 1
            }),
            Some(GridHit::Cell(coord(p.rows[0].index, p.columns[0].index)))
        );
    }
    assert_eq!(w.to_bytes(), before);
}
#[test]
fn offscreen_merge_anchor_is_read_once_and_link_intents_do_not_write() {
    let mut w = workbook();
    w.set_user_input(0, 3, 3, "anchor").unwrap();
    w.merge_cells(&Area {
        sheet: 0,
        row: 3,
        column: 3,
        width: 5,
        height: 5,
    })
    .unwrap();
    w.set_cell_link(
        0,
        3,
        3,
        Link::External {
            target: "https://example.com".into(),
            tooltip: Some("anchor link".into()),
        },
        None,
    )
    .unwrap();
    let before = w.to_bytes();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    let mut req = request();
    req.overlays.active_cell = Some(coord(5, 5));
    session.render(&w, &req).unwrap();
    let worksheet = &session.committed_frame().unwrap().worksheet;
    assert_eq!(worksheet.extended_style_reads, worksheet.cells.len() + 1);
    let fragments = session.visible_fragments(range(3, 3, 7, 7));
    assert_eq!(fragments.len(), 1);
    let point = fragments[0].rect.center();
    let hit = session.display_cell_at(point).unwrap();
    assert_eq!(hit.anchor, coord(3, 3));
    assert!(
        matches!(&hit.link.unwrap().link,Link::External {tooltip:Some(t),..} if t=="anchor link")
    );
    assert!(
        matches!(session.pointer_intent(point,PointerAction::Select),Some(HostIntent::Select {anchor,range:r,..}) if anchor==coord(3,3) && r==range(3,3,7,7))
    );
    assert!(matches!(
        session.pointer_intent(point, PointerAction::ActivateLink),
        Some(HostIntent::ActivateLink { .. })
    ));
    assert_eq!(w.to_bytes(), before);
}
#[test]
fn merge_crossing_freeze_has_four_fragments_and_one_anchor_read() {
    let mut w = workbook();
    w.set_user_input(0, 1, 1, "merged").unwrap();
    w.merge_cells(&Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 7,
        height: 7,
    })
    .unwrap();
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    session.render(&w, &request()).unwrap();
    assert_eq!(session.visible_fragments(range(1, 1, 7, 7)).len(), 4);
    let worksheet = &session.committed_frame().unwrap().worksheet;
    assert_eq!(worksheet.extended_style_reads, worksheet.cells.len());
    for fragment in session.visible_fragments(range(1, 1, 7, 7)) {
        assert_eq!(
            session
                .display_cell_at(fragment.rect.center())
                .unwrap()
                .anchor,
            coord(1, 1)
        );
    }
}
struct Failing {
    svg: SvgSceneBackend,
    fail: bool,
}
impl RenderBackend for Failing {
    type StagedFrame = String;
    type Error = &'static str;
    fn metrics(&self) -> &dyn TextMetrics {
        self.svg.metrics()
    }
    fn stage_frame(&mut self, scene: &GridScene) -> Result<String, Self::Error> {
        Ok(self.svg.stage_frame(scene).unwrap())
    }
    fn commit_frame(&mut self, staged: String) -> Result<(), Self::Error> {
        if self.fail {
            Err("presentation failed")
        } else {
            self.svg.commit_frame(staged).unwrap();
            Ok(())
        }
    }
}
#[test]
fn failed_commit_keeps_old_merge_link_and_geometry_queries() {
    let mut w = workbook();
    w.set_cell_link(
        0,
        5,
        5,
        Link::Internal {
            location: "Sheet1!A1".into(),
            tooltip: None,
        },
        Some("link"),
    )
    .unwrap();
    let mut session = CanvasSession::new(Failing {
        svg: SvgSceneBackend::new(),
        fail: false,
    });
    let mut req = request();
    session.render(&w, &req).unwrap();
    let point = session.visible_fragments(range(5, 5, 5, 5))[0]
        .rect
        .center();
    let before = session.display_cell_at(point).unwrap().clone();
    let old_link = before.link.cloned();
    let old_geometry = session
        .committed_frame()
        .unwrap()
        .worksheet
        .geometry
        .clone();
    w.delete_cell_link(0, 5, 5).unwrap();
    w.merge_cells(&Area {
        sheet: 0,
        row: 4,
        column: 4,
        width: 4,
        height: 4,
    })
    .unwrap();
    req.viewport.first_cell = coord(7, 7);
    req.revision.revision += 1;
    session.backend_mut().fail = true;
    assert!(session.render(&w, &req).is_err());
    assert_eq!(session.link_at(point), old_link.as_ref());
    assert_eq!(
        session.committed_frame().unwrap().worksheet.geometry,
        old_geometry
    );
    session.backend_mut().fail = false;
    session.render(&w, &req).unwrap();
    assert_ne!(
        session.committed_frame().unwrap().worksheet.geometry,
        old_geometry
    );
}
#[test]
fn pdf_and_svg_stage_the_identical_prepared_scene_with_overlays() {
    let w = workbook();
    let mut request = request();
    request.overlays = OverlayState {
        selection: vec![range(1, 1, 7, 7)],
        active_cell: Some(coord(1, 1)),
        clipboard: Some(range(5, 5, 6, 6)),
        formula_references: vec![
            FormulaOverlay {
                sheet: 0,
                range: range(5, 5, 8, 8),
                color_index: 2,
            },
            FormulaOverlay {
                sheet: 1,
                range: range(5, 5, 8, 8),
                color_index: 3,
            },
        ],
    };
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    session.render(&w, &request).unwrap();
    let worksheet = &session.committed_frame().unwrap().worksheet;
    assert_eq!(worksheet.extended_style_reads, worksheet.cells.len());
    assert!(
        worksheet
            .scene
            .passes
            .iter()
            .flat_map(|p| p.iter())
            .any(|p| matches!(p,Primitive::StrokePath {style,..} if !style.dash.is_empty()))
    );
    let mut pdf = PdfSceneBackend::new();
    let stage = pdf.stage_frame(&worksheet.scene).unwrap();
    pdf.commit_frame(stage).unwrap();
    let document = String::from_utf8_lossy(pdf.document().unwrap());
    assert!(document.starts_with("%PDF-"));
    assert!(document.contains("/MediaBox [0 0 400 220]"));
    assert!(session.backend().document().unwrap().contains("#3358B7"));
    assert!(!session.backend().document().unwrap().contains("#F0C419"));
}
#[test]
fn dragged_range_overlay_covers_both_corners_and_draws_its_handle() {
    let w = workbook();
    let mut request = request();
    let forward = range(5, 5, 6, 6);
    let backward = range(6, 6, 5, 5);
    request.overlays.selection = vec![backward];
    request.overlays.active_cell = Some(coord(5, 5));
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    session.render(&w, &request).unwrap();

    let worksheet = &session.committed_frame().unwrap().worksheet;
    let selection_rect = session.visible_fragments(forward)[0].rect;
    let handle_rect = session
        .autofill_handle_rect(backward)
        .expect("selected range handle");
    let primitives = worksheet.scene.passes.iter().flat_map(|pass| pass.iter());

    assert!(primitives.clone().any(|primitive| matches!(
        primitive,
        Primitive::RectFill { rect, color }
            if *rect == selection_rect
                && color.as_ref() == request.theme.selection_fill.as_ref()
    )));
    assert!(primitives.clone().any(|primitive| matches!(
        primitive,
        Primitive::RectFill { rect, color }
            if *rect == handle_rect
                && color.as_ref() == request.theme.selection_color.as_ref()
    )));
}
#[test]
fn active_cell_overlay_draws_an_autofill_handle() {
    let w = workbook();
    let mut request = request();
    request.overlays.active_cell = Some(coord(5, 5));
    let mut session = CanvasSession::new(SvgSceneBackend::new());
    session.render(&w, &request).unwrap();

    let handle_rect = session
        .autofill_handle_rect(range(5, 5, 5, 5))
        .expect("active cell handle");
    assert!(
        session
            .committed_frame()
            .unwrap()
            .worksheet
            .scene
            .passes
            .iter()
            .flat_map(|pass| pass.iter())
            .any(|primitive| matches!(
                primitive,
                Primitive::RectFill { rect, color }
                    if *rect == handle_rect
                        && color.as_ref() == request.theme.selection_color.as_ref()
            ))
    );
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_stages_the_shared_scene_and_rejects_invalid_sizes_without_publication() {
    use iron_canvas_native::NativeSceneBackend;
    let w = workbook();
    let mut source = CanvasSession::new(SvgSceneBackend::new());
    source.render(&w, &request()).unwrap();
    let scene = &source.committed_frame().unwrap().worksheet.scene;
    let mut native = NativeSceneBackend::new(1.25).unwrap();
    let stage = native.stage_frame(scene).unwrap();
    native.commit_frame(stage).unwrap();
    assert_eq!(
        (
            native.surface().unwrap().width(),
            native.surface().unwrap().height()
        ),
        (500, 275)
    );
    let mut png = Vec::new();
    native.surface().unwrap().write_to_png(&mut png).unwrap();
    assert!(png.len() > 1000);
    let mut bad = scene.clone();
    bad.clip.width = -1;
    assert!(native.stage_frame(&bad).is_err());
    let mut after = Vec::new();
    native.surface().unwrap().write_to_png(&mut after).unwrap();
    assert_eq!(png, after);
}
