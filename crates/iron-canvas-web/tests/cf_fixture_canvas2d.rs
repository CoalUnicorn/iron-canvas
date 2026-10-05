#![cfg(all(target_arch = "wasm32", feature = "js-model"))]

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
