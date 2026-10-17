#![cfg(target_arch = "wasm32")]

//! Browser proof that the conditional-formatting workbook fixture reaches the
//! real Canvas2D backend through the M1 compiler and shared session.

use iron_canvas::{CanvasSession, RenderRequest, RevisionToken};
use iron_canvas_canvas2d::Canvas2dSceneBackend;
use iron_canvas_core::{CanvasMetrics, CanvasSize, CanvasTheme, CellCoord};
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::{Model, UserModel};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const FIXTURE: &[u8] = include_bytes!("../../iron-canvas-ironcalc/tests/fixtures/cf_tests.xlsx");

fn make_canvas() -> HtmlCanvasElement {
    let document = web_sys::window()
        .expect("browser window")
        .document()
        .expect("browser document");
    document
        .create_element("canvas")
        .expect("canvas element creates")
        .dyn_into::<HtmlCanvasElement>()
        .expect("element is a canvas")
}

fn pixels(canvas: &HtmlCanvasElement) -> Vec<u8> {
    canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .unwrap()
        .get_image_data(
            0.0,
            0.0,
            f64::from(canvas.width()),
            f64::from(canvas.height()),
        )
        .unwrap()
        .data()
        .0
}

#[wasm_bindgen_test]
fn commit_replaces_old_pixels_after_target_composite_state_changes() {
    use iron_canvas_core::{
        PixelRect, Point,
        scene::{GridScene, Primitive, RenderBackend},
    };
    use std::rc::Rc;

    for reset_size in [false, true] {
        let canvas = make_canvas();
        let mut backend = Canvas2dSceneBackend::new(
            canvas.clone(),
            CanvasMetrics::new(CanvasSize { w: 8.0, h: 8.0 }, 1.0).unwrap(),
        )
        .unwrap();
        let clip = PixelRect {
            top_left: Point::default(),
            width: 8,
            height: 8,
        };
        let scene = GridScene {
            clip,
            passes: vec![Rc::from([Primitive::RectFill {
                rect: clip,
                color: Rc::from("red"),
            }])],
        };
        let staged = backend.stage_frame(&scene).unwrap();
        backend.commit_frame(staged).unwrap();
        if reset_size {
            canvas.set_width(canvas.width());
        }
        let context = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into::<web_sys::CanvasRenderingContext2d>()
            .unwrap();
        context
            .set_global_composite_operation("source-over")
            .unwrap();
        context.set_fill_style_str("red");
        context.fill_rect(0.0, 0.0, 8.0, 8.0);
        assert!(pixels(&canvas).chunks_exact(4).all(|pixel| pixel[3] == 255));

        let empty = GridScene {
            clip,
            passes: Vec::new(),
        };
        let staged = backend.stage_frame(&empty).unwrap();
        backend.commit_frame(staged).unwrap();
        assert!(pixels(&canvas).iter().all(|channel| *channel == 0));
    }
}

#[wasm_bindgen_test]
fn cf_fixture_session_paints_colored_primitives_and_preserves_pixels_on_failure() {
    let workbook = load_from_xlsx_bytes(FIXTURE, "cf_tests", "en", "UTC").unwrap();
    let workbook = UserModel::from_model(Model::from_workbook(workbook, "en").unwrap());
    for dpr in [1.0, 1.25] {
        let canvas = make_canvas();
        let backend = Canvas2dSceneBackend::new(
            canvas.clone(),
            CanvasMetrics::new(
                CanvasSize {
                    w: 1100.0,
                    h: 700.0,
                },
                dpr,
            )
            .unwrap(),
        )
        .unwrap();
        let mut session = CanvasSession::new(backend);
        let mut request = RenderRequest {
            sheet: 0,
            revision: RevisionToken {
                workbook_id: 1,
                revision: 1,
            },
            viewport: WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 1100,
                height: 700,
            },
            theme: CanvasTheme::light(),
            overlays: Default::default(),
        };
        for sheet in [0, 1] {
            request.sheet = sheet;
            session.render(&workbook, &request).unwrap();
            let painted = pixels(&canvas);
            let colorful = painted
                .chunks_exact(4)
                .filter(|p| p[3] > 0 && p[0].abs_diff(p[1]) > 30)
                .count();
            assert!(
                colorful > 500,
                "fixture needs colored CF pixels, got {colorful}"
            );
            let committed = session.committed_frame().cloned();
            request.sheet = u32::MAX;
            assert!(session.render(&workbook, &request).is_err());
            assert_eq!(session.committed_frame(), committed.as_ref());
            assert_eq!(pixels(&canvas), painted);
            request.sheet = sheet;
            // A staging-size failure must also preserve the visible canvas.
            request.viewport.width = 1000;
            assert!(session.render(&workbook, &request).is_err());
            assert_eq!(session.committed_frame(), committed.as_ref());
            assert_eq!(pixels(&canvas), painted);
            request.viewport.width = 1100;
            session.render(&workbook, &request).unwrap();
            assert_eq!(
                pixels(&canvas),
                painted,
                "Fresh retries must be deterministic"
            );
        }
    }
}

#[wasm_bindgen_test]
fn composed_merge_overlay_restores_candidate_pixels_at_both_scales() {
    use iron_canvas::{HostIntent, OverlayState, PointerAction};
    use iron_canvas_core::{Point, scene_geometry::GridRange};
    use ironcalc_base::{expressions::types::Area, types::Link};
    let coord = |row, col| CellCoord { row, col };
    let mut workbook = UserModel::new_empty("composition", "en", "UTC", "en").unwrap();
    workbook.set_rows_height(0, 1, 20, 30.0).unwrap();
    workbook.set_columns_width(0, 1, 20, 70.0).unwrap();
    workbook.set_frozen_rows_count(0, 1).unwrap();
    workbook.set_frozen_columns_count(0, 1).unwrap();
    workbook.set_rows_hidden(0, 2, 2, true).unwrap();
    workbook.set_columns_hidden(0, 2, 2, true).unwrap();
    workbook.set_user_input(0, 3, 3, "linked merge").unwrap();
    workbook
        .merge_cells(&Area {
            sheet: 0,
            row: 3,
            column: 3,
            width: 4,
            height: 4,
        })
        .unwrap();
    workbook
        .set_cell_link(
            0,
            3,
            3,
            Link::External {
                target: "https://example.com".into(),
                tooltip: None,
            },
            None,
        )
        .unwrap();
    for dpr in [1.0, 1.25] {
        let canvas = make_canvas();
        let metrics = CanvasMetrics::new(CanvasSize { w: 500.0, h: 300.0 }, dpr).unwrap();
        let mut session =
            CanvasSession::new(Canvas2dSceneBackend::new(canvas.clone(), metrics).unwrap());
        let mut request = RenderRequest {
            sheet: 0,
            revision: RevisionToken {
                workbook_id: 5,
                revision: 1,
            },
            viewport: WorksheetViewport {
                first_cell: coord(4, 4),
                width: 500,
                height: 300,
            },
            theme: CanvasTheme::light(),
            overlays: OverlayState::default(),
        };
        session.render(&workbook, &request).unwrap();
        let base = pixels(&canvas);
        let merge = GridRange {
            first: coord(3, 3),
            last: coord(6, 6),
        };
        let rect = session.visible_fragments(merge)[0].rect;
        let point = rect.center();
        assert!(matches!(
            session.pointer_intent(point, PointerAction::ActivateLink),
            Some(HostIntent::ActivateLink { .. })
        ));
        request.overlays.selection = vec![GridRange {
            first: coord(1, 1),
            last: coord(9, 9),
        }];
        request.overlays.active_cell = Some(coord(4, 4));
        session.render(&workbook, &request).unwrap();
        let selected = pixels(&canvas);
        assert_ne!(selected, base);
        let interior = rect.inset(3, 3);
        for y in ((f64::from(interior.top()) * dpr).ceil() as usize)
            ..((f64::from(interior.bottom()) * dpr).floor() as usize)
        {
            for x in ((f64::from(interior.left()) * dpr).ceil() as usize)
                ..((f64::from(interior.right()) * dpr).floor() as usize)
            {
                let i = (y * canvas.width() as usize + x) * 4;
                assert_eq!(
                    &selected[i..i + 4],
                    &base[i..i + 4],
                    "active merge interior must restore prepared pixels"
                );
            }
        }
        let before = workbook.to_bytes();
        let old_link = session.link_at(point).cloned();
        workbook.delete_cell_link(0, 3, 3).unwrap();
        request.viewport.width = 499;
        assert!(session.render(&workbook, &request).is_err());
        assert_eq!(session.link_at(point), old_link.as_ref());
        assert_eq!(pixels(&canvas), selected);
        assert!(session.hit_test(Point { x: 1, y: 1 }).is_some());
        workbook = UserModel::from_bytes(&before, "en").unwrap();
        request.viewport.width = 500;
        session.render(&workbook, &request).unwrap();
        assert_eq!(pixels(&canvas), selected);
    }
}

fn assert_fresh(
    session: &CanvasSession<Canvas2dSceneBackend>,
    model: &UserModel<'_>,
    request: &RenderRequest,
    dpr: f64,
) {
    let canvas = make_canvas();
    let metrics = CanvasMetrics::new(
        CanvasSize {
            w: f64::from(request.viewport.width),
            h: f64::from(request.viewport.height),
        },
        dpr,
    )
    .unwrap();
    let mut fresh = CanvasSession::new(Canvas2dSceneBackend::new(canvas.clone(), metrics).unwrap());
    fresh.render_fresh(model, request).unwrap();
    let actual = pixels(session.backend().canvas());
    let expected = pixels(&canvas);
    let differences = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
    assert_eq!(
        differences, 0,
        "retained Canvas2D must match forced Fresh byte for byte"
    );
}

#[wasm_bindgen_test]
fn warm_text_layout_matches_fresh_after_wrap_style_and_metric_changes() {
    use iron_canvas::{TextMetricsKey, WorksheetCache};
    use ironcalc_base::expressions::types::Area;

    let mut model = UserModel::new_empty("text cache", "en", "UTC", "en").unwrap();
    let area = Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 3,
        height: 8,
    };
    model
        .update_range_style(&area, "alignment.wrap_text", "true")
        .unwrap();
    model.set_columns_width(0, 1, 3, 85.0).unwrap();
    model.set_rows_height(0, 1, 8, 54.0).unwrap();
    for row in 1..=8 {
        for col in 1..=3 {
            model
                .set_user_input(0, row, col, "  one  two\nthree ")
                .unwrap();
        }
    }
    for dpr in [1.0, 1.25, 2.0] {
        model.set_columns_width(0, 1, 3, 85.0).unwrap();
        model.update_range_style(&area, "font.b", "false").unwrap();
        let metrics = CanvasMetrics::new(CanvasSize { w: 400.0, h: 300.0 }, dpr).unwrap();
        let mut session =
            CanvasSession::new(Canvas2dSceneBackend::new(make_canvas(), metrics).unwrap());
        let mut cache = WorksheetCache::new(1);
        let mut request = RenderRequest {
            sheet: 0,
            revision: RevisionToken {
                workbook_id: 1,
                revision: 0,
            },
            viewport: WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 400,
                height: 300,
            },
            theme: CanvasTheme::light(),
            overlays: Default::default(),
        };
        let mut key = TextMetricsKey::new(1, 0);
        session
            .render_cached(&mut cache, &model, &request, key)
            .unwrap();
        assert_fresh(&session, &model, &request, dpr);
        for step in 0..6 {
            match step {
                0 => {} // Equal text with a new revision exercises warm layout.
                1 => request.viewport.first_cell.row = 2,
                2 => model.set_columns_width(0, 1, 3, 65.0).unwrap(),
                3 => model.update_range_style(&area, "font.b", "true").unwrap(),
                4 => key = TextMetricsKey::new(1, 1),
                _ => session.invalidate(),
            }
            request.revision.revision += 1;
            let outcome = session
                .render_cached(&mut cache, &model, &request, key)
                .unwrap();
            assert!(!outcome.content_cached);
            assert!(outcome.extended_style_reads > 0);
            assert_fresh(&session, &model, &request, dpr);
        }
    }
}

#[wasm_bindgen_test]
fn shared_cache_tab_returns_and_pane_overlays_match_fresh() {
    use iron_canvas::{OverlayState, RenderStrategy, TextMetricsKey, WorksheetCache};
    use iron_canvas_core::scene_geometry::GridRange;

    let mut model = UserModel::new_empty("cached panes", "en", "UTC", "en").unwrap();
    model.new_sheet().unwrap();
    for sheet in 0..2 {
        for row in 1..20 {
            model
                .set_user_input(sheet, row, 1, &format!("sheet {sheet} row {row}"))
                .unwrap();
        }
    }
    for dpr in [1.0, 1.25, 2.0] {
        let metrics = CanvasMetrics::new(CanvasSize { w: 640.0, h: 400.0 }, dpr).unwrap();
        let mut first =
            CanvasSession::new(Canvas2dSceneBackend::new(make_canvas(), metrics).unwrap());
        let mut second =
            CanvasSession::new(Canvas2dSceneBackend::new(make_canvas(), metrics).unwrap());
        let mut cache = WorksheetCache::new(2);
        let mut request = RenderRequest {
            sheet: 0,
            revision: RevisionToken {
                workbook_id: 1,
                revision: 0,
            },
            viewport: WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 640,
                height: 400,
            },
            theme: CanvasTheme::light(),
            overlays: Default::default(),
        };
        let key = TextMetricsKey::new(1, 0);
        for sheet in [0, 1, 0] {
            request.sheet = sheet;
            let outcome = first
                .render_cached(&mut cache, &model, &request, key)
                .unwrap();
            assert_eq!(outcome.strategy, RenderStrategy::Fresh);
            if outcome.sequence == 3 {
                assert!(outcome.content_cached);
                assert_eq!(outcome.extended_style_reads, 0);
            }
            assert_fresh(&first, &model, &request, dpr);
        }
        request.overlays.selection = vec![GridRange {
            first: CellCoord { row: 2, col: 1 },
            last: CellCoord { row: 3, col: 2 },
        }];
        request.overlays.active_cell = Some(CellCoord { row: 2, col: 1 });
        assert!(
            second
                .render_cached(&mut cache, &model, &request, key)
                .unwrap()
                .content_cached
        );
        assert_fresh(&second, &model, &request, dpr);
        assert!(
            first
                .render_cached(&mut cache, &model, &request, key)
                .unwrap()
                .content_cached
        );
        assert_fresh(&first, &model, &request, dpr);
        request.overlays = OverlayState::default();
        first
            .render_cached(&mut cache, &model, &request, key)
            .unwrap();
        assert_fresh(&first, &model, &request, dpr);
        request.viewport.first_cell.row = 2;
        assert!(
            !first
                .render_cached(&mut cache, &model, &request, key)
                .unwrap()
                .content_cached
        );
        assert_fresh(&first, &model, &request, dpr);
        first.invalidate();
        assert!(
            !first
                .render_cached(&mut cache, &model, &request, key)
                .unwrap()
                .content_cached
        );
        assert_fresh(&first, &model, &request, dpr);
        let next = TextMetricsKey::new(1, 1);
        let outcome = first
            .render_cached(&mut cache, &model, &request, next)
            .unwrap();
        assert!(!outcome.content_cached);
        assert_eq!(outcome.strategy, RenderStrategy::Fresh);
        assert_fresh(&first, &model, &request, dpr);
    }
}
#[wasm_bindgen_test]
fn retained_damage_scroll_overlay_and_revert_match_fresh() {
    use iron_canvas::RenderStrategy;
    use iron_canvas_core::scene_geometry::GridRange;
    for dpr in [1.0, 1.25, 1.3, 1.5] {
        let mut model = UserModel::new_empty("retained", "en", "UTC", "en").unwrap();
        model.set_rows_height(0, 1, 80, 24.0).unwrap();
        model.set_columns_width(0, 1, 20, 80.0).unwrap();
        for row in 1..40 {
            for col in 1..8 {
                model
                    .set_user_input(0, row, col, &format!("{row}-{col}"))
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
                width: 640,
                height: 400,
            },
            theme: CanvasTheme::light(),
            overlays: Default::default(),
        };
        let metrics = CanvasMetrics::new(CanvasSize { w: 640.0, h: 400.0 }, dpr).unwrap();
        let mut s = CanvasSession::new(Canvas2dSceneBackend::new(make_canvas(), metrics).unwrap());
        s.render(&model, &request).unwrap();
        let same = s.render(&model, &request).unwrap();
        assert_eq!(same.strategy, RenderStrategy::Reuse);
        assert_eq!(same.extended_style_reads, 0);
        for text in ["changed", "6-3", ""] {
            model.set_user_input(0, 6, 3, text).unwrap();
            request.revision.revision += 1;
            assert_eq!(
                s.render(&model, &request).unwrap().strategy,
                RenderStrategy::Damage
            );
            assert_fresh(&s, &model, &request, dpr);
        }
        for row in [2, 3, 2, 1] {
            request.viewport.first_cell.row = row;
            assert_eq!(
                s.render(&model, &request).unwrap().strategy,
                if dpr == 1.3 {
                    RenderStrategy::Fresh
                } else {
                    RenderStrategy::Blit
                }
            );
            assert_fresh(&s, &model, &request, dpr);
        }
        request.overlays.selection = vec![GridRange {
            first: CellCoord { row: 3, col: 2 },
            last: CellCoord { row: 7, col: 4 },
        }];
        request.overlays.active_cell = Some(CellCoord { row: 4, col: 3 });
        assert_eq!(s.render(&model, &request).unwrap().extended_style_reads, 0);
        assert_fresh(&s, &model, &request, dpr);
        request.overlays = Default::default();
        s.render(&model, &request).unwrap();
        assert_fresh(&s, &model, &request, dpr);
        model.set_frozen_rows_count(0, 2).unwrap();
        model.set_frozen_columns_count(0, 2).unwrap();
        model.set_rows_hidden(0, 4, 4, true).unwrap();
        model.set_columns_hidden(0, 3, 3, true).unwrap();
        request.revision.revision += 1;
        assert_eq!(
            s.render(&model, &request).unwrap().strategy,
            RenderStrategy::Fresh
        );
        assert_fresh(&s, &model, &request, dpr);
        model.set_user_input(0, 6, 5, "frozen edit").unwrap();
        request.revision.revision += 1;
        assert_eq!(
            s.render(&model, &request).unwrap().strategy,
            RenderStrategy::Damage
        );
        assert_fresh(&s, &model, &request, dpr);
        request.theme = CanvasTheme::dark();
        assert_eq!(
            s.render(&model, &request).unwrap().strategy,
            RenderStrategy::Fresh
        );
        assert_fresh(&s, &model, &request, dpr);
    }
}
#[wasm_bindgen_test]
fn cf_revisions_repaint_visible_results_of_offscreen_edits() {
    use ironcalc_base::{
        cf_types::{CfRuleInput, Cfvo},
        types::Color,
    };
    for dpr in [1.0, 1.25] {
        let mut model = UserModel::new_empty("cf dependency", "en", "UTC", "en").unwrap();
        model.set_user_input(0, 1, 1, "5").unwrap();
        model.set_user_input(0, 100, 1, "10").unwrap();
        model
            .add_conditional_formatting(
                0,
                "A1:A100",
                CfRuleInput::DataBar {
                    min: Some(Cfvo::Number(0.0)),
                    max: None,
                    positive_color: Color::Rgb("#638EC6".into()),
                    negative_color: Color::Rgb("#FF0000".into()),
                    is_gradient: true,
                    show_value: true,
                },
            )
            .unwrap();
        let mut request = RenderRequest {
            sheet: 0,
            revision: RevisionToken {
                workbook_id: 1,
                revision: 0,
            },
            viewport: WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 400,
                height: 160,
            },
            theme: CanvasTheme::light(),
            overlays: Default::default(),
        };
        let mut s = CanvasSession::new(
            Canvas2dSceneBackend::new(
                make_canvas(),
                CanvasMetrics::new(CanvasSize { w: 400.0, h: 160.0 }, dpr).unwrap(),
            )
            .unwrap(),
        );
        s.render(&model, &request).unwrap();
        let before = pixels(s.backend().canvas());
        model.set_user_input(0, 100, 1, "100").unwrap();
        request.revision.revision += 1;
        let outcome = s.render(&model, &request).unwrap();
        assert!(outcome.extended_style_reads > 0);
        assert_ne!(pixels(s.backend().canvas()), before);
        assert_fresh(&s, &model, &request, dpr);
    }
}

#[wasm_bindgen_test]
fn failed_partial_updates_keep_pixels_queries_and_fingerprints_until_retry() {
    use iron_canvas::{RenderBackend, RenderStrategy};
    use iron_canvas_core::{
        Point,
        painter::TextMetrics,
        scene::{GridScene, retained::SceneUpdate},
    };
    use ironcalc_base::types::Link;
    use std::{cell::Cell, rc::Rc};
    struct Failing {
        inner: Canvas2dSceneBackend,
        fail: Rc<Cell<u8>>,
    }
    impl RenderBackend for Failing {
        type StagedFrame = HtmlCanvasElement;
        type Error = wasm_bindgen::JsValue;
        fn metrics(&self) -> &dyn TextMetrics {
            self.inner.metrics()
        }
        fn retained_scale(&self) -> Option<f64> {
            self.inner.retained_scale()
        }
        fn supports_partial_updates(&self) -> bool {
            true
        }
        fn stage_frame(&mut self, s: &GridScene) -> Result<Self::StagedFrame, Self::Error> {
            self.inner.stage_frame(s)
        }
        fn stage_update(
            &mut self,
            s: &GridScene,
            u: &SceneUpdate,
        ) -> Result<Self::StagedFrame, Self::Error> {
            let staged = self.inner.stage_update(s, u)?;
            if self.fail.get() == 1 {
                Err("late stage".into())
            } else {
                Ok(staged)
            }
        }
        fn commit_frame(&mut self, s: Self::StagedFrame) -> Result<(), Self::Error> {
            if self.fail.get() == 2 {
                Err("commit".into())
            } else {
                self.inner.commit_frame(s)
            }
        }
    }
    let mut model = UserModel::new_empty("failure", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "same label").unwrap();
    model
        .set_cell_link(
            0,
            1,
            1,
            Link::External {
                target: "https://old.example".into(),
                tooltip: None,
            },
            None,
        )
        .unwrap();
    let canvas = make_canvas();
    let fail = Rc::new(Cell::new(0));
    let mut session = CanvasSession::new(Failing {
        inner: Canvas2dSceneBackend::new(
            canvas.clone(),
            CanvasMetrics::new(CanvasSize { w: 400.0, h: 200.0 }, 1.25).unwrap(),
        )
        .unwrap(),
        fail: Rc::clone(&fail),
    });
    let mut request = RenderRequest {
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
    session.render(&model, &request).unwrap();
    let old = pixels(&canvas);
    let frame = session.committed_frame().cloned();
    let point = Point { x: 55, y: 30 };
    let link = session.link_at(point).cloned();
    model
        .set_cell_link(
            0,
            1,
            1,
            Link::External {
                target: "https://new.example".into(),
                tooltip: None,
            },
            None,
        )
        .unwrap();
    request.revision.revision += 1;
    for mode in [1, 2] {
        fail.set(mode);
        assert!(session.render(&model, &request).is_err());
        assert_eq!(pixels(&canvas), old);
        assert_eq!(session.committed_frame(), frame.as_ref());
        assert_eq!(session.link_at(point), link.as_ref());
    }
    fail.set(0);
    let done = session.render(&model, &request).unwrap();
    assert_eq!(done.sequence, 2);
    assert_eq!(done.strategy, RenderStrategy::Reuse);
    assert_ne!(session.link_at(point), link.as_ref());
    assert_eq!(pixels(&canvas), old);
    model.set_user_input(0, 3, 2, "new pixels").unwrap();
    request.revision.revision += 1;
    for mode in [1, 2] {
        fail.set(mode);
        assert!(session.render(&model, &request).is_err());
        assert_eq!(pixels(&canvas), old);
    }
    fail.set(0);
    assert_eq!(
        session.render(&model, &request).unwrap().strategy,
        RenderStrategy::Damage
    );
    model.set_user_input(0, 3, 2, "").unwrap();
    request.revision.revision += 1;
    assert_eq!(
        session.render(&model, &request).unwrap().strategy,
        RenderStrategy::Damage
    );
    assert_eq!(pixels(&canvas), old);
}

#[wasm_bindgen_test]
fn fixture_cf_edits_match_fresh_with_translucent_background() {
    let book = load_from_xlsx_bytes(FIXTURE, "cf_tests", "en", "UTC").unwrap();
    let mut model = UserModel::from_model(Model::from_workbook(book, "en").unwrap());
    for sheet in [0, 1] {
        let mut request = RenderRequest {
            sheet,
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
        request.theme.cell_bg = "rgba(250,245,240,0.7)".into();
        let mut session = CanvasSession::new(
            Canvas2dSceneBackend::new(
                make_canvas(),
                CanvasMetrics::new(
                    CanvasSize {
                        w: 1100.0,
                        h: 700.0,
                    },
                    1.25,
                )
                .unwrap(),
            )
            .unwrap(),
        );
        session.render(&model, &request).unwrap();
        for value in ["3", "9", "-5"] {
            model
                .set_user_input(sheet, 2, if sheet == 0 { 8 } else { 2 }, value)
                .unwrap();
            request.revision.revision += 1;
            session.render(&model, &request).unwrap();
            assert_fresh(&session, &model, &request, 1.25);
        }
    }
}
