//! Exact spill policy checks with deterministic text measurements.

use iron_canvas_core::{
    CanvasTheme, CellCoord, Line, PixelRect, Side, painter::TextMetrics, scene::Primitive,
};
use iron_canvas_ironcalc::{
    PreparedWorksheet, WorksheetCompiler, WorksheetContext, WorksheetViewport,
};
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

fn border(model: &mut UserModel<'_>, row: i32, col: i32, side: Side) {
    let mut style = model.get_cell_style(0, row, col).unwrap();
    let edge = match side {
        Side::Left => &mut style.border.left,
        Side::Top => &mut style.border.top,
        Side::Right => &mut style.border.right,
        Side::Bottom => &mut style.border.bottom,
    };
    *edge = Some(BorderItem {
        style: BorderStyle::Thin,
        color: Color::Rgb("#123456".into()),
    });
    model.set_selected_cell(row, col).unwrap();
    model.on_paste_styles(&[vec![style]]).unwrap();
}

fn prepare(model: &UserModel<'_>, first: i32, width: i32) -> PreparedWorksheet {
    WorksheetCompiler::new()
        .prepare(
            WorksheetContext {
                workbook: model,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: first },
                width,
                height: 120,
            },
            &Metrics,
        )
        .unwrap()
}

fn rect(frame: &PreparedWorksheet, row: i32, col: i32) -> PixelRect {
    frame
        .cells()
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
fn narrow_visible_columns_can_supply_spill_from_both_viewport_edges() {
    for width in [1, 8, 9, 10] {
        for (alignment, first) in [
            ("left", 1),
            ("center", 1),
            ("right", 1),
            ("left", 4),
            ("right", 2),
        ] {
            let mut model = model();
            model.set_columns_width(0, 3, 3, f64::from(width)).unwrap();
            model.set_user_input(0, 1, 3, TEXT).unwrap();
            style(&mut model, 1, 3, "alignment.horizontal", alignment);
            // The final two cases put the source outside the viewport.
            let viewport_width = if first == 2 { 98 } else { 348 };
            let frame = prepare(&model, first, viewport_width);
            let paint = text_paint(&frame, TEXT);
            assert_eq!(paint.len(), 1, "width {width}, {alignment}, first {first}");
            assert!(paint[0].0.width > width);
            if first == 1 {
                let source = rect(&frame, 1, 3);
                let anchor = match alignment {
                    "left" => f64::from(source.left()) + 4.0,
                    "right" => f64::from(source.right()) - 4.0,
                    _ => f64::from(source.left()) + f64::from(width) / 2.0,
                };
                assert_eq!(paint[0].1, anchor);
            }
        }
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
fn conditional_fill_on_an_empty_cell_preserves_spill_and_background() {
    use ironcalc_base::{
        cf_types::CfRuleInput,
        types::{Dxf, Fill},
    };
    let mut model = model();
    model.set_user_input(0, 1, 1, TEXT).unwrap();
    let expected = text_paint(&prepare(&model, 1, 348), TEXT);
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
    assert_eq!(text_paint(&frame, TEXT), expected);
    assert!(frame.scene.passes[0].iter().any(|op| matches!(
        op, Primitive::RectFill { rect: bounds, color }
            if *bounds == rect(&frame, 1, 3) && color.as_ref() == "#ff0000"
    )));
}

#[test]
fn offscreen_sources_cross_formatted_empty_cells_but_stop_at_hidden_columns() {
    for (source, alignment, first) in [(1, "left", 3), (7, "right", 3)] {
        let mut model = model();
        model.set_user_input(0, 1, source, TEXT).unwrap();
        style(&mut model, 1, source, "alignment.horizontal", alignment);
        let expected = text_paint(&prepare(&model, first, 198), TEXT);
        assert_eq!(expected.len(), 1);
        // Include the source, the path outside the view, and both pane edges.
        for col in 1..=7 {
            style(&mut model, 1, col, "fill.color", "#ffffff");
            border(&mut model, 1, col, Side::Top);
        }
        let frame = prepare(&model, first, 198);
        assert_eq!(text_paint(&frame, TEXT), expected);
        assert!(!frame.cells().any(|cell| cell.coordinate.col == source));
        let hidden = if source == 1 { 2 } else { 6 };
        model.set_columns_hidden(0, hidden, hidden, true).unwrap();
        assert!(text_paint(&prepare(&model, first, 198), TEXT).is_empty());
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
fn fills_and_font_only_styles_preserve_spill_and_backgrounds() {
    for alignment in ["left", "center", "right"] {
        for color in ["#ffffff", "#00bfff"] {
            let mut model = model();
            model.set_user_input(0, 2, 3, TEXT).unwrap();
            style(&mut model, 2, 3, "alignment.horizontal", alignment);
            let expected = text_paint(&prepare(&model, 1, 348), TEXT);
            for col in 1..=6 {
                style(&mut model, 2, col, "fill.color", color);
                if col != 3 {
                    style(&mut model, 2, col, "font.b", "true");
                }
            }
            let frame = prepare(&model, 1, 348);
            assert_eq!(text_paint(&frame, TEXT), expected, "{alignment}, {color}");
            for col in 1..=6 {
                assert!(frame.scene.passes[0].iter().any(|op| matches!(
                    op, Primitive::RectFill { rect: bounds, color: fill }
                        if *bounds == rect(&frame, 2, col) && fill.as_ref() == color
                )));
            }
        }
    }
}

#[test]
fn source_and_destination_borders_preserve_spill_and_explicit_edges() {
    for alignment in ["left", "center", "right"] {
        for col in [2, 3, 4] {
            for side in [Side::Left, Side::Top, Side::Right, Side::Bottom] {
                let mut model = model();
                model.set_user_input(0, 2, 3, TEXT).unwrap();
                style(&mut model, 2, 3, "alignment.horizontal", alignment);
                let expected = text_paint(&prepare(&model, 1, 348), TEXT);
                border(&mut model, 2, col, side);
                let frame = prepare(&model, 1, 348);
                assert_eq!(
                    text_paint(&frame, TEXT),
                    expected,
                    "{alignment}, {col}, {side:?}"
                );
                assert!(frame.scene.passes[3].iter().any(|op| matches!(
                    op, Primitive::Line { line, color, .. }
                        if *line == side.line(rect(&frame, 2, col)) && color.as_ref() == "#123456"
                )));
            }
        }
    }
}

#[test]
fn borders_from_adjacent_rows_preserve_source_and_destination_spill() {
    for col in [1, 3] {
        for (row, side) in [(1, Side::Bottom), (3, Side::Top)] {
            let mut model = model();
            model.set_user_input(0, 2, 1, TEXT).unwrap();
            let expected = text_paint(&prepare(&model, 1, 348), TEXT);
            border(&mut model, row, col, side);
            // IronCalc resolves the shared edge onto the text row too.
            let resolved = model
                .get_extended_cell_style(0, 2, col)
                .unwrap()
                .style
                .border;
            assert!(if row == 1 {
                resolved.top.is_some()
            } else {
                resolved.bottom.is_some()
            });
            assert_eq!(text_paint(&prepare(&model, 1, 348), TEXT), expected);
        }
    }
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
        assert!(!frame.cells().any(|cell| cell.coordinate.col == col));
        assert!(frame.extended_style_reads <= frame.cell_count() + 2);
    }
}

#[test]
fn offscreen_search_and_spill_distance_stop_at_64_columns() {
    let mut model = model();
    let long = "text".repeat(1000);
    model.set_user_input(0, 1, 1, &long).unwrap();
    let within = prepare(&model, 65, 198);
    assert_eq!(text_paint(&within, &long)[0].0.width, 50);
    let beyond = prepare(&model, 66, 198);
    assert!(text_paint(&beyond, &long).is_empty());
    // One visible block style plus 64 individual offscreen cells within reach.
    assert_eq!(beyond.extended_style_reads, 1);
    assert_eq!(
        within.extended_style_reads,
        beyond.extended_style_reads + 64
    );

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
        .prepare(
            WorksheetContext {
                workbook: &model,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            viewport,
            svg.metrics(),
        )
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
        .prepare(
            WorksheetContext {
                workbook: &model,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            viewport,
            pdf.metrics(),
        )
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
