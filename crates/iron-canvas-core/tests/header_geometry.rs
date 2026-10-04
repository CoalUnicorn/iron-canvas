//! Always-visible header geometry, retained blit origins, and paint.

mod common;

use iron_canvas_core::CanvasSize;
use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::geometry::constants::{CELL_AREA_INSET, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT};
use iron_canvas_core::theme::CanvasTheme;

use common::{TestModel, test_inputs};

const CANVAS: CanvasSize = CanvasSize { w: 600.0, h: 400.0 };

fn frame(model: &TestModel) -> Chrome {
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, CANVAS, &theme);
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

#[test]
fn both_header_strips_and_corner_have_expected_geometry() {
    let f = frame(&TestModel::synthetic_grid());
    assert_eq!(f.row_header_thickness, HEADER_COL_WIDTH);
    assert_eq!(f.col_header_thickness, HEADER_ROW_HEIGHT);
    assert_eq!(f.cell_origin.x, f.row_header_thickness + CELL_AREA_INSET);
    assert_eq!(f.cell_origin.y, f.col_header_thickness + CELL_AREA_INSET);
}

// ── Blit frozen-band geometry ────────────────────────────────────────────────

use common::canvas_default;
use iron_canvas_core::BlitPlan;
use iron_canvas_core::CanvasModel;
use iron_canvas_core::chrome::ActiveCellSnapshot;
use iron_canvas_core::{FrameDelta, FrameInputs};

fn snap_at_top(m: &TestModel) -> ActiveCellSnapshot {
    let Some(view) = m.get_selected_view() else {
        panic!("get_selected_view() returned None")
    };
    ActiveCellSnapshot::capture(m, view.sheet, view.row, view.column)
}

/// Classify `prev` against `m`'s live state and unwrap the qualifying
/// `BlitPlan`, panicking with `msg` otherwise. Mirrors `scroll_blit.rs`'s
/// helper of the same shape.
fn qualify_scroll(
    prev: &Chrome,
    m: &TestModel,
    inputs: &FrameInputs,
    msg: &'static str,
) -> BlitPlan {
    match Chrome::classify(Some(prev), m, inputs, Some(&snap_at_top(m))) {
        FrameDelta::Scroll(plan) => plan,
        _ => panic!("{msg}"),
    }
}

// The merged shift covers both frozen and scrolling column bands.
fn merged_band_x(plan: &BlitPlan) -> i32 {
    plan.shift.src.top_left.x
}

/// The BottomLeft frozen-column band starts at the committed cell origin.
#[test]
fn frozen_cols_blit_band_starts_at_committed_cell_origin() {
    let canvas = canvas_default();
    let theme = std::rc::Rc::new(CanvasTheme::light());

    let m = TestModel::synthetic_grid().with_frozen_cols(2);
    m.set_data_until(30);

    let inputs0 = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs0, FramePath::Fresh);
    let expected_x = frame0.cell_origin.x;
    assert!(
        expected_x > 0,
        "row-header strip must be included in the committed cell origin"
    );

    m.set_top_row(2);
    let inputs1 = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs1,
        "single-row scroll with frozen cols must qualify for blit",
    );

    let band_x = merged_band_x(&plan);

    assert_eq!(
        band_x, expected_x,
        "frozen-cols band must start at cell_origin.x ({expected_x}); got {band_x}"
    );
}

// The frozen-rows sibling on a column scroll is the `TopRight` pane.

// The merged shift covers both frozen and scrolling row bands.
fn merged_band_y(plan: &BlitPlan) -> i32 {
    plan.shift.src.top_left.y
}

/// The TopRight frozen-row band starts at the committed cell origin.
#[test]
fn frozen_rows_blit_band_starts_at_committed_cell_origin() {
    let canvas = canvas_default();
    let theme = std::rc::Rc::new(CanvasTheme::light());

    let m = TestModel::synthetic_grid().with_frozen_rows(2);
    m.set_data_until(30);

    let inputs0 = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs0, FramePath::Fresh);
    let expected_y = frame0.cell_origin.y;
    assert!(
        expected_y > 0,
        "column-header strip must be included in the committed cell origin"
    );

    m.set_left_column(2);
    let inputs1 = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs1,
        "single-column scroll with frozen rows must qualify for blit",
    );

    let band_y = merged_band_y(&plan);

    assert_eq!(
        band_y, expected_y,
        "frozen-rows band must start at cell_origin.y ({expected_y}); got {band_y}"
    );
}

// ── Paint-gating tests ────────────────────────────────────────────────────────

use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize as OrchCanvasSize;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_recorder::{DrawOp, MemSurface};
use std::rc::Rc;

fn paint(model: Rc<TestModel>) -> Vec<DrawOp> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(
        iron_canvas_core::CanvasMetrics::new(OrchCanvasSize { w: 600.0, h: 400.0 }, 1.0)
            .expect("test canvas metrics are valid"),
    );
    orch.set_model(model);
    orch.render_pending();
    orch.grid_surface().recorder().ops().clone()
}

fn has_group(ops: &[DrawOp], class: GroupClass) -> bool {
    ops.iter()
        .any(|op| matches!(op, DrawOp::BeginGroup { class: c } if *c == class))
}

fn overlay_paint(model: Rc<TestModel>) -> Vec<DrawOp> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(
        iron_canvas_core::CanvasMetrics::new(OrchCanvasSize { w: 600.0, h: 400.0 }, 1.0)
            .expect("test canvas metrics are valid"),
    );
    orch.set_model(model);
    orch.render_pending();
    orch.overlay_surface().recorder().ops().clone()
}

#[test]
fn headers_paint_corner_and_active_selection_highlights() {
    let theme = iron_canvas_core::CanvasTheme::light();
    let selected: &str = &theme.header_selected_bg;

    let has_sel_fill = |ops: &[DrawOp]| {
        ops.iter()
            .any(|op| matches!(op, DrawOp::RectFill { color, .. } if color.as_str() == selected))
    };

    // Default active cell (1,1) highlights its row + col header.
    let grid = paint(Rc::new(TestModel::synthetic_grid()));
    assert!(
        has_group(&grid, GroupClass::Headers),
        "header painter group remains present"
    );
    assert!(
        has_group(&grid, GroupClass::Corner),
        "corner box remains present"
    );

    let shown = overlay_paint(Rc::new(TestModel::synthetic_grid()));
    assert!(
        has_sel_fill(&shown),
        "active cell continues to highlight its row and column headers"
    );
}
