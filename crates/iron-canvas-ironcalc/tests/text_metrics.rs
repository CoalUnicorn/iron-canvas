//! The fitted row and the emitted baselines use the same font extents.
use iron_canvas_core::{
    CanvasTheme, CellCoord,
    painter::{FontMetrics, TextBaseline, TextMetrics},
    scene::{Primitive, RenderBackend},
};
use iron_canvas_ironcalc::{
    WorksheetCompiler, WorksheetContext, WorksheetViewport, autofit::fit_height,
};
use ironcalc_base::{UserModel, expressions::types::Area};

struct Metrics;
impl TextMetrics for Metrics {
    fn measure_text_width(&self, text: &str, _: &str) -> f64 {
        text.chars().count() as f64 * 10.0
    }
    fn font_metrics(&self, _: &str) -> Option<FontMetrics> {
        Some(FontMetrics {
            ascent: 13.0,
            descent: 5.0,
            line_gap: 3.0,
        })
    }
}

fn model() -> UserModel<'static> {
    let mut model = UserModel::new_empty("text metrics", "en", "UTC", "en").unwrap();
    model.set_columns_width(0, 1, 1, 58.0).unwrap();
    model.set_rows_height(0, 1, 1, 80.0).unwrap();
    model.set_user_input(0, 1, 1, "abcdefghij").unwrap();
    model
        .update_range_style(&area(), "alignment.wrap_text", "true")
        .unwrap();
    model
}
fn area() -> Area {
    Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 1,
        height: 1,
    }
}
fn viewport() -> WorksheetViewport {
    WorksheetViewport {
        first_cell: CellCoord { row: 1, col: 1 },
        width: 400,
        height: 300,
    }
}

#[test]
fn autofit_and_compiler_share_measured_baselines_for_all_vertical_alignments() {
    let mut model = model();
    assert_eq!(
        fit_height(&model, &Metrics, 0, 1, 1, 1).unwrap(),
        Some(47.0)
    );
    // In an 80-pixel cell the 39-pixel text block has 41 pixels of free space.
    for (align, first_baseline) in [("top", 17.0), ("center", 33.5), ("bottom", 50.0)] {
        model
            .update_range_style(&area(), "alignment.vertical", align)
            .unwrap();
        let frame = WorksheetCompiler::new()
            .prepare(
                WorksheetContext {
                    workbook: &model,
                    sheet: 0,
                    theme: &CanvasTheme::light(),
                },
                viewport(),
                &Metrics,
            )
            .unwrap();
        let top = f64::from(
            frame
                .cells()
                .find(|cell| cell.coordinate == CellCoord { row: 1, col: 1 })
                .unwrap()
                .bounds
                .top(),
        );
        let lines: Vec<_> = frame
            .scene
            .passes
            .iter()
            .flat_map(|pass| pass.iter())
            .filter_map(|op| match op {
                Primitive::Text {
                    text, y, baseline, ..
                } if text == "abcde" || text == "fghij" => {
                    Some((text.as_ref(), *y - top, *baseline))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            lines,
            [
                ("abcde", first_baseline, TextBaseline::Alphabetic),
                ("fghij", first_baseline + 21.0, TextBaseline::Alphabetic)
            ]
        );
    }
}

fn check_backend(metrics: &dyn TextMetrics) {
    let mut model = model();
    let text = "Ág AV office abcdefghijklmnopqrstuvwxyz";
    model.set_user_input(0, 1, 1, text).unwrap();
    for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
        model
            .update_range_style(&area(), "font.b", &bold.to_string())
            .unwrap();
        model
            .update_range_style(&area(), "font.i", &italic.to_string())
            .unwrap();
        let height = fit_height(&model, metrics, 0, 1, 1, 1).unwrap().unwrap();
        model.set_rows_height(0, 1, 1, height.ceil()).unwrap();
        let frame = WorksheetCompiler::new()
            .prepare(
                WorksheetContext {
                    workbook: &model,
                    sheet: 0,
                    theme: &CanvasTheme::light(),
                },
                viewport(),
                metrics,
            )
            .unwrap();
        let cell = frame
            .cells()
            .find(|cell| cell.coordinate == CellCoord { row: 1, col: 1 })
            .unwrap()
            .bounds;
        let mut collected = String::new();
        let mut baselines = Vec::new();
        for op in frame.scene.passes.iter().flat_map(|pass| pass.iter()) {
            if let Primitive::Text {
                text,
                y,
                font_css,
                baseline,
                ..
            } = op
                && *baseline == TextBaseline::Alphabetic
            {
                collected.push_str(text);
                let font = metrics.font_metrics(font_css).unwrap();
                assert!(font.is_valid());
                assert!(*y - font.ascent >= f64::from(cell.top()) + 4.0 - 1e-8);
                assert!(*y + font.descent <= f64::from(cell.bottom()) - 4.0 + 1e-8);
                assert!(metrics.measure_text_width(text, font_css) <= f64::from(cell.width) - 8.0);
                baselines.push(*y);
            }
        }
        assert_eq!(collected, text);
        assert!(baselines.len() > 1);
        assert!(baselines.windows(2).all(|ys| ys[1] > ys[0]));
    }
}

#[test]
fn svg_and_pdf_fitted_rows_contain_their_own_font_boxes() {
    let svg = iron_canvas_export::SvgSceneBackend::new();
    check_backend(svg.metrics());
    let pdf = iron_canvas_export::PdfSceneBackend::new();
    check_backend(pdf.metrics());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn pango_fitted_rows_contain_their_own_font_boxes() {
    let native = iron_canvas_native::NativeSceneBackend::new(1.25).unwrap();
    check_backend(native.metrics());
}

fn check_underline(metrics: &dyn TextMetrics) {
    let mut model = model();
    model.set_user_input(0, 1, 1, "Underline").unwrap();
    model.set_columns_width(0, 1, 1, 300.0).unwrap();
    model.update_range_style(&area(), "font.u", "true").unwrap();
    model
        .update_range_style(&area(), "font.size", "32")
        .unwrap();
    for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
        model
            .update_range_style(&area(), "font.b", &bold.to_string())
            .unwrap();
        model
            .update_range_style(&area(), "font.i", &italic.to_string())
            .unwrap();
        let frame = WorksheetCompiler::new()
            .prepare(
                WorksheetContext {
                    workbook: &model,
                    sheet: 0,
                    theme: &CanvasTheme::light(),
                },
                viewport(),
                metrics,
            )
            .unwrap();
        let mut baseline = None;
        let mut underline = None;
        for op in frame.scene.passes.iter().flat_map(|pass| pass.iter()) {
            match op {
                Primitive::Text {
                    text,
                    y,
                    baseline: TextBaseline::Alphabetic,
                    ..
                } if text == "Underline" => baseline = Some(*y),
                Primitive::TextLine { y, .. } => underline = Some(*y),
                _ => {}
            }
        }
        let baseline = baseline.unwrap();
        let underline = underline.unwrap();
        assert!(
            underline > baseline,
            "underline {underline} crosses baseline {baseline}; bold {bold}, italic {italic}"
        );
    }
}

#[test]
fn svg_and_pdf_underlines_stay_below_the_alphabetic_baseline() {
    let svg = iron_canvas_export::SvgSceneBackend::new();
    check_underline(svg.metrics());
    let pdf = iron_canvas_export::PdfSceneBackend::new();
    check_underline(pdf.metrics());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn pango_underlines_stay_below_the_alphabetic_baseline() {
    let native = iron_canvas_native::NativeSceneBackend::new(1.25).unwrap();
    check_underline(native.metrics());
}
