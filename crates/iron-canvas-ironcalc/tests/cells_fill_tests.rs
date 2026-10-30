//! Compare run painting with the original cell fills, including mixed read paths.
//! compiler.rs loads this file as a native unit-test module for private paint access.
use super::*;
use iron_canvas_core::scene::{Primitive, RenderBackend};
use iron_canvas_native::NativeSceneBackend;
use ironcalc_base::expressions::types::Area;

fn fixture() -> (UserModel<'static>, WorksheetViewport) {
    let mut model = UserModel::new_empty("fills", "en", "UTC", "en").unwrap();
    model
        .update_range_style(
            &Area {
                sheet: 0,
                row: 1,
                column: 1,
                width: 100,
                height: 20,
            },
            "fill.color",
            "#00bfff",
        )
        .unwrap();
    for (col, width) in [(1, 20.0), (2, 35.0), (4, 12.0), (6, 90.0)] {
        model.set_columns_width(0, col, col, width).unwrap();
    }
    // Keep both empty fill coverage and an evaluated CF color in the same row.
    model
        .range_clear_formatting(&Area {
            sheet: 0,
            row: 2,
            column: 4,
            width: 1,
            height: 1,
        })
        .unwrap();
    model
        .add_conditional_formatting(
            0,
            "G2:L2",
            ironcalc_base::cf_types::CfRuleInput::Formula {
                formula: "TRUE".into(),
                stop_if_true: false,
                format: ironcalc_base::types::Dxf {
                    fill: Some(ironcalc_base::types::Fill {
                        color: ironcalc_base::types::Color::Rgb("#ff8800".into()),
                    }),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    model.evaluate();
    model
        .set_user_input(0, 2, 1, "spill across filled empty cells")
        .unwrap();
    model
        .merge_cells(&Area {
            sheet: 0,
            row: 3,
            column: 1,
            width: 5,
            height: 2,
        })
        .unwrap();
    model.set_user_input(0, 3, 1, "offscreen merge").unwrap();
    model.set_columns_hidden(0, 5, 5, true).unwrap();
    model.set_rows_hidden(0, 5, 6, true).unwrap();
    model.set_frozen_rows_count(0, 1).unwrap();
    model.set_frozen_columns_count(0, 1).unwrap();
    (
        model,
        WorksheetViewport {
            first_cell: CellCoord { row: 2, col: 3 },
            width: 850,
            height: 330,
        },
    )
}

fn prepare(
    model: &UserModel<'_>,
    viewport: WorksheetViewport,
    policy: cells::FillPaint,
    limits: cells::BlockLimits,
    metrics: &dyn iron_canvas_core::painter::TextMetrics,
) -> PreparedWorksheet {
    let mut cache = TextLayoutCache::default();
    let mut shaper = TextShaper::new(metrics, TextMetricsKey::new(0, 0), &mut cache);
    composition::prepare_with_fill_paint(
        WorksheetContext {
            workbook: model,
            sheet: 0,
            theme: &CanvasTheme::light(),
        },
        viewport,
        &mut shaper,
        WorksheetReuse::default(),
        Some(limits),
        policy,
    )
    .unwrap()
}

fn fills(frame: &PreparedWorksheet) -> usize {
    frame.scene.passes[0]
        .iter()
        .filter(|op| {
            matches!(
                op,
                Primitive::RectFill { .. } | Primitive::RectFillRun { .. }
            )
        })
        .count()
}

#[test]
fn run_fills_reduce_rectangles_and_preserve_native_pixels_and_queries() {
    let (model, viewport) = fixture();
    for scale in [1.0, 1.25, 2.0] {
        for limits in [
            cells::BlockLimits::default(),
            cells::BlockLimits {
                width: 2,
                height: 2,
                cells_per_attempt: 24,
                ..cells::BlockLimits::default()
            },
        ] {
            let mut backend = NativeSceneBackend::new(scale).unwrap();
            let reference = prepare(
                &model,
                viewport,
                cells::FillPaint::Cells,
                limits,
                backend.metrics(),
            );
            let candidate = prepare(
                &model,
                viewport,
                cells::FillPaint::Runs,
                limits,
                backend.metrics(),
            );
            assert_eq!(reference.geometry, candidate.geometry);
            assert_eq!(reference.merges, candidate.merges);
            assert_eq!(reference.links, candidate.links);
            // Only ordinary backgrounds change. Other passes include merge restoration.
            assert_eq!(reference.scene.passes[1..], candidate.scene.passes[1..]);
            assert!(fills(&candidate) < fills(&reference));
            assert_eq!(
                iron_canvas_core::scene::retained::SceneFingerprint::new(&reference.scene),
                iron_canvas_core::scene::retained::SceneFingerprint::new(&candidate.scene)
            );
            let a = backend.stage_frame(&reference.scene).unwrap();
            let b = backend.stage_frame(&candidate.scene).unwrap();
            let mut pixels = Vec::new();
            a.with_data(|data| pixels.extend_from_slice(data)).unwrap();
            b.with_data(|data| {
                let differences: Vec<_> = pixels
                    .iter()
                    .zip(data)
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .map(|(i, _)| i)
                    .collect();
                assert!(
                    differences.is_empty(),
                    "scale {scale}, {} changed bytes; first indices {:?}; stride {}",
                    differences.len(),
                    &differences[..differences.len().min(16)],
                    b.stride()
                );
            })
            .unwrap();
        }
    }
}

#[test]
fn run_fills_preserve_export_document_bytes() {
    use iron_canvas_export::{PdfSceneBackend, SvgSceneBackend};
    let (model, viewport) = fixture();
    fn compare<B: RenderBackend>(
        backend: &mut B,
        model: &UserModel<'_>,
        viewport: WorksheetViewport,
    ) where
        B::StagedFrame: PartialEq,
        B::Error: std::fmt::Debug,
    {
        for limits in [
            cells::BlockLimits::default(),
            cells::BlockLimits {
                width: 2,
                height: 2,
                cells_per_attempt: 24,
                ..cells::BlockLimits::default()
            },
        ] {
            let reference = prepare(
                model,
                viewport,
                cells::FillPaint::Cells,
                limits,
                backend.metrics(),
            );
            let candidate = prepare(
                model,
                viewport,
                cells::FillPaint::Runs,
                limits,
                backend.metrics(),
            );
            assert!(
                backend.stage_frame(&reference.scene).unwrap()
                    == backend.stage_frame(&candidate.scene).unwrap()
            );
        }
    }
    compare(&mut SvgSceneBackend::new(), &model, viewport);
    compare(&mut PdfSceneBackend::new(), &model, viewport);
}
