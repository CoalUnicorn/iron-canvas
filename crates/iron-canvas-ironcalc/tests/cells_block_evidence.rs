//! Rendering evidence and an opt-in release measurement for native block reads.
//! compiler.rs loads this file as a unit-test module for private read-path access.
use super::*;
use iron_canvas_core::painter::TextMetrics;
#[cfg(not(target_arch = "wasm32"))]
use iron_canvas_core::scene::RenderBackend;

struct Metrics;
impl TextMetrics for Metrics {
    fn measure_text_width(&self, text: &str, _: &str) -> f64 {
        text.chars().count() as f64 * 8.0
    }
}

fn fixture(case: &str) -> (UserModel<'static>, WorksheetViewport) {
    let mut model = if case == "cf" {
        UserModel::from_model(
            ironcalc::import::load_from_xlsx(
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cf_tests.xlsx"),
                "en",
                "UTC",
                "en",
            )
            .unwrap(),
        )
    } else {
        UserModel::new_empty("measurement", "en", "UTC", "en").unwrap()
    };
    if case == "dense" || case == "borders" || case == "all_cf" {
        for row in 1..=60 {
            for col in 1..=24 {
                model
                    .set_user_input(
                        0,
                        row,
                        col,
                        &if case == "all_cf" {
                            (row * col).to_string()
                        } else {
                            format!("{row}:{col}")
                        },
                    )
                    .unwrap();
            }
        }
    }
    if case == "all_cf" {
        use ironcalc_base::{
            cf_types::{CfRuleInput, Cfvo},
            types::Color,
        };
        model
            .add_conditional_formatting(
                0,
                "A1:X60",
                CfRuleInput::DataBar {
                    min: Some(Cfvo::Number(0.0)),
                    max: Some(Cfvo::Number(1440.0)),
                    positive_color: Color::Rgb("#00aaff".into()),
                    negative_color: Color::Rgb("#ff0000".into()),
                    is_gradient: true,
                    show_value: true,
                },
            )
            .unwrap();
        model.evaluate();
    }
    if case == "fills" {
        model
            .update_range_style(
                &ironcalc_base::expressions::types::Area {
                    sheet: 0,
                    row: 1,
                    column: 1,
                    width: 24,
                    height: 60,
                },
                "fill.color",
                "#00bfff",
            )
            .unwrap();
    }
    if case == "borders" {
        use ironcalc_base::types::{BorderItem, BorderStyle, Color};
        let mut style = model.get_cell_style(0, 1, 1).unwrap();
        let edge = Some(BorderItem {
            style: BorderStyle::Thin,
            color: Color::Rgb("#123456".into()),
        });
        style.border.top = edge.clone();
        style.border.bottom = edge.clone();
        style.border.left = edge.clone();
        style.border.right = edge;
        model.on_paste_styles(&vec![vec![style; 24]; 60]).unwrap();
    }
    if case == "hidden" {
        model.set_rows_hidden(0, 3, 1002, true).unwrap();
        model.set_columns_hidden(0, 3, 1002, true).unwrap();
    }
    let mut viewport = WorksheetViewport {
        first_cell: CellCoord { row: 1, col: 1 },
        width: 1280,
        height: 800,
    };
    if case == "frozen" {
        model.set_frozen_rows_count(0, 2).unwrap();
        model.set_frozen_columns_count(0, 2).unwrap();
        viewport.first_cell = CellCoord {
            row: 10_000,
            col: 1000,
        };
    }
    (model, viewport)
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare(
    model: &UserModel<'_>,
    viewport: WorksheetViewport,
    blocks: bool,
    shaper: &mut TextShaper<'_>,
) -> PreparedWorksheet {
    composition::prepare_with_blocks(
        WorksheetContext {
            workbook: model,
            sheet: 0,
            theme: &CanvasTheme::light(),
        },
        viewport,
        shaper,
        WorksheetReuse::default(),
        blocks.then(cells::BlockLimits::default),
    )
    .unwrap()
}

#[test]
#[ignore = "run in release with CELL_BLOCK_CASE, CELL_BLOCK_MODE, and CELL_BLOCK_CACHE"]
fn measure_preparation() {
    if std::hint::black_box(cfg!(debug_assertions)) {
        panic!("measure with --release");
    }
    let case = std::env::var("CELL_BLOCK_CASE").unwrap();
    assert!(
        [
            "plain", "dense", "cf", "all_cf", "borders", "hidden", "frozen", "fills"
        ]
        .contains(&case.as_str())
    );
    let mode = std::env::var("CELL_BLOCK_MODE").unwrap();
    assert!(["individual", "block", "runs"].contains(&mode.as_str()));
    let warm = std::env::var("CELL_BLOCK_CACHE").unwrap() == "warm";
    let repeats: usize = std::env::var("CELL_BLOCK_REPEATS")
        .unwrap_or("31".into())
        .parse()
        .unwrap();
    assert!(repeats > 0);
    let (model, viewport) = fixture(&case);
    let compile = |shaper: &mut TextShaper<'_>| {
        composition::prepare_with_fill_paint(
            WorksheetContext {
                workbook: &model,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            viewport,
            shaper,
            WorksheetReuse::default(),
            (mode != "individual").then(cells::BlockLimits::default),
            if mode == "runs" {
                cells::FillPaint::Runs
            } else {
                cells::FillPaint::Cells
            },
        )
        .unwrap()
    };
    let mut cache = TextLayoutCache::default();
    let mut shaper = TextShaper::new(&Metrics, TextMetricsKey::new(0, 0), &mut cache);
    if warm {
        drop(compile(&mut shaper));
    }
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..repeats {
        // Cold runs use a new layout cache. The immutable workbook and metrics
        // stay equal between modes. Drop the prior scene outside the timer.
        drop(last.take());
        let mut cold_cache = TextLayoutCache::default();
        let mut cold_shaper = TextShaper::new(&Metrics, TextMetricsKey::new(0, 0), &mut cold_cache);
        let start = std::time::Instant::now();
        let frame = if warm {
            compile(&mut shaper)
        } else {
            compile(&mut cold_shaper)
        };
        times.push(start.elapsed().as_nanos());
        last = Some(frame);
    }
    times.sort_unstable();
    let frame = last.unwrap();
    let stats = &frame.read_stats;
    let primitives: usize = frame.scene.passes.iter().map(|pass| pass.len()).sum();
    println!(
        "MEASURE,{case},{mode},{},{},{},{},{},{},{},{},{},{}",
        if warm { "warm" } else { "cold" },
        times[times.len() / 2],
        times[times.len() / 10],
        times[(times.len() - 1) * 9 / 10],
        stats.block_calls,
        stats.block_cells,
        stats.block_styles,
        stats.fallback_cells,
        frame.extended_style_reads,
        primitives
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn native_pixels_and_export_documents_match_individual_reads() {
    use iron_canvas_export::{PdfSceneBackend, SvgSceneBackend};
    use iron_canvas_native::NativeSceneBackend;
    for case in [
        "plain", "dense", "cf", "all_cf", "borders", "hidden", "frozen", "fills",
    ] {
        let (model, viewport) = fixture(case);
        for scale in [1.0, 1.25, 2.0] {
            let mut backend = NativeSceneBackend::new(scale).unwrap();
            let mut cache = TextLayoutCache::default();
            let mut shaper =
                TextShaper::new(backend.metrics(), TextMetricsKey::new(0, 0), &mut cache);
            let reference = prepare(&model, viewport, false, &mut shaper);
            let block = prepare(&model, viewport, true, &mut shaper);
            assert_eq!(reference.scene, block.scene, "{case}, {scale}");
            let a = backend.stage_frame(&reference.scene).unwrap();
            let b = backend.stage_frame(&block.scene).unwrap();
            let mut pixels = Vec::new();
            a.with_data(|data| pixels.extend_from_slice(data)).unwrap();
            b.with_data(|data| assert_eq!(pixels, data, "{case}, {scale}"))
                .unwrap();
        }
        fn documents<B: RenderBackend>(
            backend: &mut B,
            model: &UserModel<'_>,
            viewport: WorksheetViewport,
        ) where
            B::StagedFrame: PartialEq + std::fmt::Debug,
            B::Error: std::fmt::Debug,
        {
            let mut cache = TextLayoutCache::default();
            let mut shaper =
                TextShaper::new(backend.metrics(), TextMetricsKey::new(0, 0), &mut cache);
            let reference = prepare(model, viewport, false, &mut shaper);
            let block = prepare(model, viewport, true, &mut shaper);
            assert_eq!(
                backend.stage_frame(&reference.scene).unwrap(),
                backend.stage_frame(&block.scene).unwrap()
            );
        }
        documents(&mut SvgSceneBackend::new(), &model, viewport);
        documents(&mut PdfSceneBackend::new(), &model, viewport);
    }
}
