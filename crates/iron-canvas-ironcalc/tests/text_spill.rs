//! Exact spill policy checks with deterministic text measurements.

use iron_canvas_core::{
    CanvasTheme, CellCoord, Line, PixelRect, painter::TextMetrics, scene::Primitive,
};
use iron_canvas_ironcalc::{PreparedWorksheet, WorksheetCompiler, WorksheetViewport};
use ironcalc_base::{
    UserModel,
    expressions::types::Area,
    types::{BorderItem, BorderStyle, Color},
};

struct Metrics;
impl TextMetrics for Metrics {
    fn measure_text_width(&self, text: &str, _: &str) -> f64 {
        text.chars().count() as f64 * 10.0
    }
}

const TEXT: &str = "abcdefghijklmnopqrst";

fn model() -> UserModel<'static> {
    let mut model = UserModel::new_empty("spill", "en", "UTC", "en").unwrap();
    model.set_columns_width(0, 1, 200, 50.0).unwrap();
    model.set_rows_height(0, 1, 20, 32.0).unwrap();
    model
}

fn style(model: &mut UserModel<'_>, row: i32, col: i32, key: &str, value: &str) {
    model
        .update_range_style(
            &Area {
                sheet: 0,
                row,
                column: col,
                width: 1,
                height: 1,
            },
            key,
            value,
        )
        .unwrap();
}

fn prepare(model: &UserModel<'_>, first: i32, width: i32) -> PreparedWorksheet {
    WorksheetCompiler::new()
        .prepare(
            model,
            0,
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: first },
                width,
                height: 120,
            },
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap()
}

fn rect(frame: &PreparedWorksheet, row: i32, col: i32) -> PixelRect {
    frame
        .cells
        .iter()
        .find(|cell| cell.coordinate == CellCoord { row, col })
        .unwrap()
        .bounds
}

fn text_paint(frame: &PreparedWorksheet, value: &str) -> Vec<(PixelRect, f64)> {
    let mut found = Vec::new();
    for pass in &frame.scene.passes {
        let mut clips = vec![frame.scene.clip];
        for op in pass.iter() {
            match op {
                Primitive::Clip { rect } => clips.push(
                    clips
                        .last()
                        .unwrap()
                        .intersection(*rect)
                        .unwrap_or_default(),
                ),
                Primitive::EndClip => {
                    clips.pop().unwrap();
                }
                Primitive::Text { text, x, .. } if text == value => {
                    found.push((*clips.last().unwrap(), *x))
                }
                _ => {}
            }
        }
    }
    found
}

#[test]
fn left_right_and_center_keep_the_source_anchor_and_expand_only_the_clip() {
    for (alignment, left, right, x) in [
        ("general", 148, 348, 152.0),
        ("left", 148, 348, 152.0),
        ("right", 48, 198, 194.0),
        ("center", 48, 298, 173.0),
    ] {
        let mut model = model();
        model.set_user_input(0, 1, 3, TEXT).unwrap();
        style(&mut model, 1, 3, "alignment.horizontal", alignment);
        let before = model.to_bytes();
        let frame = prepare(&model, 1, 348);
        let paint = text_paint(&frame, TEXT);
        assert_eq!(paint.len(), 1);
        assert_eq!(
            (paint[0].0.left(), paint[0].0.right(), paint[0].1),
            (left, right, x),
            "{alignment}"
        );
        assert_eq!(model.to_bytes(), before);
    }
}

#[test]
fn occupied_neighbors_include_formulas_with_empty_results_and_hidden_numbers() {
    for value in ["block", "=\"\"", " ", "42"] {
        let mut model = model();
        model.set_user_input(0, 1, 1, TEXT).unwrap();
        model.set_user_input(0, 1, 3, value).unwrap();
        if value == "=\"\"" {
            assert_eq!(model.get_formatted_cell_value(0, 1, 3).unwrap(), "");
        }
        if value == "42" {
            style(&mut model, 1, 3, "num_fmt", ";;;");
        }
        let frame = prepare(&model, 1, 348);
        let clip = text_paint(&frame, TEXT)[0].0;
        assert_eq!(clip.right(), rect(&frame, 1, 3).left(), "{value}");
    }
}

#[test]
fn center_spill_stops_independently_on_each_side() {
    let mut model = model();
    model.set_user_input(0, 1, 3, TEXT).unwrap();
    model.set_user_input(0, 1, 2, "block").unwrap();
    style(&mut model, 1, 3, "alignment.horizontal", "center");
    let frame = prepare(&model, 1, 348);
    let clip = text_paint(&frame, TEXT)[0].0;
    assert_eq!(clip.left(), rect(&frame, 1, 3).left());
    assert!(clip.right() > rect(&frame, 1, 3).right());
}

#[test]
fn conditional_fill_on_an_empty_cell_blocks_spill() {
    use ironcalc_base::{
        cf_types::CfRuleInput,
        types::{Dxf, Fill},
    };
    let mut model = model();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    // Store a style-only empty cell so the engine evaluates this CF target.
    style(&mut model, 1, 3, "font.b", "true");
    model
        .add_conditional_formatting(
            0,
            "C1:C1",
            CfRuleInput::Formula {
                formula: "TRUE".into(),
                format: Dxf {
                    fill: Some(Fill {
                        color: Color::Rgb("#ff0000".into()),
                    }),
                    ..Default::default()
                },
                stop_if_true: false,
            },
        )
        .unwrap();
    assert!(
        !model
            .get_extended_cell_style(0, 1, 3)
            .unwrap()
            .style
            .fill
            .color
            .is_none()
    );
    let frame = prepare(&model, 1, 348);
    assert_eq!(
        text_paint(&frame, TEXT)[0].0.right(),
        rect(&frame, 1, 3).left()
    );
}

#[test]
fn offscreen_sources_stop_at_formatting_blockers_and_hidden_columns() {
    for hidden in [false, true] {
        let mut model = model();
        model.set_user_input(0, 1, 1, TEXT).unwrap();
        if hidden {
            model.set_columns_hidden(0, 2, 2, true).unwrap();
        } else {
            style(&mut model, 1, 2, "fill.color", "#ffffff");
        }
        assert!(text_paint(&prepare(&model, 3, 198), TEXT).is_empty());
    }
}

#[test]
fn sheet_limits_end_spill_without_out_of_bounds_reads() {
    let mut model = model();
    model.set_user_input(0, 1, 16384, TEXT).unwrap();
    let frame = prepare(&model, 16384, 348);
    assert_eq!(text_paint(&frame, TEXT)[0].0, rect(&frame, 1, 16384));
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    style(&mut model, 1, 1, "alignment.horizontal", "right");
    let frame = prepare(&model, 1, 348);
    assert_eq!(text_paint(&frame, TEXT)[0].0, rect(&frame, 1, 1));
}

#[test]
fn evaluated_text_formula_can_spill() {
    let mut model = model();
    model
        .set_user_input(0, 1, 1, &format!("=\"{TEXT}\""))
        .unwrap();
    let frame = prepare(&model, 1, 348);
    assert!(text_paint(&frame, TEXT)[0].0.width > rect(&frame, 1, 1).width);
}

#[test]
fn numbers_logical_errors_wrap_and_other_alignments_stay_clipped() {
    for value in ["123456789012345", "TRUE", "=1/0"] {
        let mut model = model();
        model.set_user_input(0, 1, 2, value).unwrap();
        style(&mut model, 1, 2, "alignment.horizontal", "left");
        let formatted = model.get_formatted_cell_value(0, 1, 2).unwrap();
        let frame = prepare(&model, 1, 348);
        assert_eq!(text_paint(&frame, &formatted)[0].0, rect(&frame, 1, 2));
    }
    for (key, value) in [
        ("alignment.wrap_text", "true"),
        ("alignment.horizontal", "centerContinuous"),
        ("alignment.horizontal", "fill"),
        ("alignment.horizontal", "justify"),
        ("alignment.horizontal", "distributed"),
    ] {
        let mut model = model();
        model.set_user_input(0, 1, 2, TEXT).unwrap();
        style(&mut model, 1, 2, key, value);
        let frame = prepare(&model, 1, 348);
        if key == "alignment.wrap_text" {
            for line in ["abcd", "efgh", "ijkl", "mnop", "qrst"] {
                assert_eq!(text_paint(&frame, line)[0].0, rect(&frame, 1, 2));
            }
        } else {
            assert_eq!(text_paint(&frame, TEXT)[0].0, rect(&frame, 1, 2), "{value}");
        }
    }
}

#[test]
fn fills_and_borders_block_but_font_only_styles_do_not() {
    let mut model = model();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    style(&mut model, 1, 3, "font.b", "true");
    let frame = prepare(&model, 1, 348);
    assert!(text_paint(&frame, TEXT)[0].0.right() > rect(&frame, 1, 3).right());
    style(&mut model, 1, 3, "fill.color", "#ffffff");
    let frame = prepare(&model, 1, 348);
    assert_eq!(
        text_paint(&frame, TEXT)[0].0.right(),
        rect(&frame, 1, 3).left()
    );

    let mut border = model.get_cell_style(0, 1, 3).unwrap();
    border.fill.color = Color::None;
    border.border.top = Some(BorderItem {
        style: BorderStyle::Thin,
        color: Color::Rgb("#123456".into()),
    });
    model.set_selected_cell(1, 3).unwrap();
    model.on_paste_styles(&[vec![border]]).unwrap();
    let frame = prepare(&model, 1, 348);
    assert_eq!(
        text_paint(&frame, TEXT)[0].0.right(),
        rect(&frame, 1, 3).left()
    );
    assert!(
        frame.scene.passes[3]
            .iter()
            .any(|op| matches!(op, Primitive::Line { color, .. } if color.as_ref() == "#123456"))
    );
}

#[test]
fn spill_removes_only_crossed_vertical_grid_lines() {
    let mut model = model();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    let frame = prepare(&model, 1, 348);
    let source = rect(&frame, 1, 1);
    let crossed = rect(&frame, 1, 2).left();
    let grid = &frame.scene.passes[2];
    assert!(!grid.iter().any(|op| matches!(op, Primitive::Line { line: Line::V { x, span }, .. } if *x == crossed && span.from == source.top())));
    assert!(grid.iter().any(
        |op| matches!(op, Primitive::Line { line: Line::H { y, .. }, .. } if *y == source.top())
    ));
    assert!(grid.iter().any(|op| matches!(op, Primitive::Line { line: Line::V { x, span }, .. } if *x == crossed && span.from == source.bottom())));
}

#[test]
fn competing_spills_stay_in_their_source_cells() {
    let mut model = model();
    let other = "ABCDEFGHIJKLMNOPQRST";
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    model.set_user_input(0, 1, 5, other).unwrap();
    style(&mut model, 1, 5, "alignment.horizontal", "right");
    let frame = prepare(&model, 1, 348);
    assert_eq!(text_paint(&frame, TEXT)[0].0, rect(&frame, 1, 1));
    assert_eq!(text_paint(&frame, other)[0].0, rect(&frame, 1, 5));
}

#[test]
fn sources_outside_both_viewport_edges_reach_visible_empty_cells() {
    for (col, alignment, anchor) in [(1, "left", 2), (6, "right", 2)] {
        let mut model = model();
        model.set_user_input(0, 1, col, TEXT).unwrap();
        style(&mut model, 1, col, "alignment.horizontal", alignment);
        // Three visible columns: B, C, D. The right source is in F.
        let frame = prepare(&model, anchor, 198);
        let paint = text_paint(&frame, TEXT);
        assert_eq!(paint.len(), 1);
        assert!(paint[0].0.width > 0);
        assert!(!frame.cells.iter().any(|cell| cell.coordinate.col == col));
        assert!(frame.extended_style_reads <= frame.cells.len() + 2);
    }
}

#[test]
fn offscreen_search_and_spill_distance_stop_at_64_columns() {
    let mut model = model();
    let long = "text".repeat(1000);
    model.set_user_input(0, 1, 1, &long).unwrap();
    let within = prepare(&model, 65, 198);
    assert_eq!(text_paint(&within, &long)[0].0.width, 50);
    assert_eq!(within.extended_style_reads, within.cells.len() + 64);
    let beyond = prepare(&model, 66, 198);
    assert!(text_paint(&beyond, &long).is_empty());
    assert_eq!(beyond.extended_style_reads, beyond.cells.len());

    let wide = prepare(&model, 1, 5048);
    assert_eq!(
        text_paint(&wide, &long)[0].0.right(),
        rect(&wide, 1, 65).right()
    );
}

#[test]
fn hidden_columns_stop_spill_and_hidden_sources_do_not_paint() {
    let mut model = model();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    model.set_columns_hidden(0, 3, 3, true).unwrap();
    let frame = prepare(&model, 1, 348);
    assert_eq!(
        text_paint(&frame, TEXT)[0].0.right(),
        rect(&frame, 1, 2).right()
    );
    model.set_columns_hidden(0, 1, 1, true).unwrap();
    assert!(text_paint(&prepare(&model, 1, 348), TEXT).is_empty());
    model.set_columns_hidden(0, 1, 1, false).unwrap();
    model.set_rows_hidden(0, 1, 1, true).unwrap();
    assert!(text_paint(&prepare(&model, 1, 348), TEXT).is_empty());
}

#[test]
fn merges_block_spill_and_merged_sources_keep_the_merged_clip() {
    let mut model = model();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    model
        .merge_cells(&Area {
            sheet: 0,
            row: 1,
            column: 3,
            width: 2,
            height: 1,
        })
        .unwrap();
    let frame = prepare(&model, 1, 348);
    assert_eq!(
        text_paint(&frame, TEXT)[0].0.right(),
        rect(&frame, 1, 3).left()
    );
    model
        .merge_cells(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 2,
            height: 1,
        })
        .unwrap();
    let frame = prepare(&model, 1, 348);
    assert!(
        text_paint(&frame, TEXT)
            .iter()
            .all(|(clip, _)| clip.width <= 100)
    );
}

#[test]
fn frozen_columns_block_spill_and_offscreen_search_across_the_split() {
    let mut model = model();
    model.set_frozen_columns_count(0, 2).unwrap();
    model.set_frozen_rows_count(0, 1).unwrap();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    let frame = prepare(&model, 3, 348);
    assert_eq!(
        text_paint(&frame, TEXT)[0].0.right(),
        rect(&frame, 1, 2).right()
    );
    assert_eq!(text_paint(&frame, TEXT).len(), 1);
    let other = "ABCDEFGHIJKLMNOPQRST";
    model.set_user_input(0, 2, 3, other).unwrap();
    style(&mut model, 2, 3, "alignment.horizontal", "right");
    let frame = prepare(&model, 3, 348);
    assert_eq!(
        text_paint(&frame, other)[0].0.left(),
        rect(&frame, 2, 3).left()
    );
}

#[test]
fn svg_and_pdf_emit_the_spill_clip_selected_with_their_own_metrics() {
    use iron_canvas_core::scene::RenderBackend;
    use iron_canvas_export::{PdfSceneBackend, SvgSceneBackend};
    let mut model = model();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    let viewport = WorksheetViewport {
        first_cell: CellCoord { row: 1, col: 1 },
        width: 348,
        height: 120,
    };

    let mut svg = SvgSceneBackend::new();
    let frame = WorksheetCompiler::new()
        .prepare(&model, 0, viewport, svg.metrics(), &CanvasTheme::light())
        .unwrap();
    let clip = text_paint(&frame, TEXT)[0].0;
    assert!(clip.width > rect(&frame, 1, 1).width);
    let output = svg.stage_frame(&frame.scene).unwrap();
    assert!(output.contains(&format!(
        "<rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\"/></clipPath>",
        f64::from(clip.left()),
        f64::from(clip.top()),
        f64::from(clip.width),
        f64::from(clip.height)
    )));

    let mut pdf = PdfSceneBackend::new();
    let frame = WorksheetCompiler::new()
        .prepare(&model, 0, viewport, pdf.metrics(), &CanvasTheme::light())
        .unwrap();
    let clip = text_paint(&frame, TEXT)[0].0;
    assert!(clip.width > rect(&frame, 1, 1).width);
    let output = pdf.stage_frame(&frame.scene).unwrap();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains(&format!(
        "{:.3} {:.3} {:.3} {:.3} re\nW n",
        f64::from(clip.left()),
        f64::from(clip.top()),
        f64::from(clip.width),
        f64::from(clip.height)
    )));
}
