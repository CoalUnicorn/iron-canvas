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
