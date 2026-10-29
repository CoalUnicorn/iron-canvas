//! Engine CF fixture checks for the M1 compiler and its resolved CF paint.
//! The SVG test uses the resolved scene backend. Browser pixels are covered
//! by `iron-canvas/tests/cf_fixture_canvas2d.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use iron_canvas_core::scene::{Primitive, RenderBackend};
use iron_canvas_core::{CanvasTheme, CellCoord, PixelRect, Point};
use iron_canvas_export::SvgSceneBackend;
use iron_canvas_ironcalc::{
    CellPlacement, CfDecorationPaint, WorksheetCompiler, WorksheetContext, WorksheetViewport,
};
use ironcalc::import::{load_from_xlsx, load_from_xlsx_bytes};
use ironcalc_base::cf_types::{CfRuleInput, Cfvo, Icon};
use ironcalc_base::types::Color;
use ironcalc_base::{Model, UserModel};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cf_tests.xlsx");

fn load(path: &str) -> UserModel<'static> {
    UserModel::from_model(load_from_xlsx(path, "en", "UTC", "en").expect("workbook loads"))
}

/// The compiler's resolved conditional formatting for one cell, with colors
/// resolved through the workbook theme exactly as the paint path does.
fn decoration(model: &UserModel<'static>, sheet: u32, row: i32, col: i32) -> CfDecorationPaint {
    let ext = model
        .get_extended_cell_style(sheet, row, col)
        .expect("cell read");
    CfDecorationPaint::resolve(&ext, model)
}

fn rule_count(model: &UserModel<'static>, sheet: u32) -> usize {
    model
        .get_conditional_formatting_list(sheet)
        .expect("rule list")
        .len()
}

#[test]
fn fixture_imports_fifty_four_rules_across_seven_sheets() {
    let model = load(FIXTURE);
    let counts: Vec<usize> = (0..7).map(|sheet| rule_count(&model, sheet)).collect();
    assert_eq!(
        counts,
        vec![18, 20, 3, 4, 3, 2, 4],
        "the 54 imported rules the audit expects, per sheet"
    );
}

#[test]
fn icons_with_show_value_zero_reach_the_canvas_hidden() {
    let model = load(FIXTURE);
    // Sheet1!B17:B22 carry icons with showValue="0".
    let icon = decoration(&model, 0, 17, 2)
        .icon
        .expect("B17 is an icon cell");
    assert!(
        !icon.show_value,
        "showValue=0 must survive as show_value=false"
    );
}

#[test]
fn x14_data_bar_overrides_reach_the_canvas() {
    let model = load(FIXTURE);
    // H2:H8 — normal gradient bars.
    let gradient = decoration(&model, 0, 2, 8)
        .data_bar
        .expect("H2 is a data-bar cell");
    assert!(gradient.is_gradient, "H2 is a gradient bar");

    // I2:I8 — solid bars from the x14 gradient="0" override.
    let solid = decoration(&model, 0, 2, 9)
        .data_bar
        .expect("I2 is a data-bar cell");
    assert!(
        !solid.is_gradient,
        "the x14 override must force a solid bar"
    );

    // J2:J8 — solid bars, bounds -5..10, hidden values. Zero sits at one third
    // of the range, so the axis is not at the left edge.
    let axis = decoration(&model, 0, 2, 10)
        .data_bar
        .expect("J2 is a data-bar cell");
    assert!(!axis.is_gradient);
    assert!(!axis.show_value, "J2 hides its value");
    assert!(
        (axis.axis_position - 1.0 / 3.0).abs() < 1e-6,
        "bounds -5..10 put the zero axis at one third, got {}",
        axis.axis_position
    );
    // 5 / 15 = 1/3 of the range from the axis is the value endpoint.
    assert!((axis.value - 8.0 / 15.0).abs() < 1e-6, "got {}", axis.value);
}

#[test]
fn ratings_carry_the_engine_glyph_color_and_counts() {
    let model = load(FIXTURE);
    // IconSets!L22:L26 — the rating columns.
    let star = decoration(&model, 1, 22, 2)
        .rating
        .expect("L22 is a rating cell");
    assert_eq!(star.style, Icon::Star);
    assert_eq!(star.color.to_lowercase(), "#ffd700");
    assert_eq!((star.count, star.max), (1, 3));

    let circle = decoration(&model, 1, 23, 3)
        .rating
        .expect("M23 is a rating cell");
    assert_eq!(circle.style, Icon::Circle);
    assert_eq!((circle.count, circle.max), (2, 5));
}

#[test]
fn cf_fixture_renders_through_svg_backend() {
    let workbook = load_from_xlsx_bytes(
        include_bytes!("fixtures/cf_tests.xlsx"),
        "cf_tests",
        "en",
        "UTC",
    )
    .expect("fixture workbook loads");
    let model = UserModel::from_model(
        Model::from_workbook(workbook, "en").expect("fixture model constructs"),
    );
    let mut backend = SvgSceneBackend::new();
    let prepared = WorksheetCompiler::new()
        .prepare(
            WorksheetContext {
                workbook: &model,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 1100,
                height: 700,
            },
            backend.metrics(),
        )
        .unwrap();
    // Unchanged styles share entries even when other cells have CF.
    assert!(prepared.extended_style_reads > 0);
    assert!(prepared.extended_style_reads < prepared.cell_count());
    assert!(
        prepared.scene.passes[1]
            .iter()
            .any(|p| matches!(p, Primitive::Gradient { .. }))
    );
    let staged = backend.stage_frame(&prepared.scene).unwrap();
    backend.commit_frame(staged).unwrap();
    let svg = backend.document().unwrap();
    assert!(svg.starts_with("<svg "));
    assert!(svg.contains("<linearGradient"));
}

#[test]
fn compiler_suppresses_hidden_values_but_keeps_icon_primitives() {
    let model = load(FIXTURE);
    let backend = SvgSceneBackend::new();
    let prepared = WorksheetCompiler::new()
        .prepare(
            WorksheetContext {
                workbook: &model,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            WorksheetViewport {
                first_cell: CellCoord { row: 17, col: 2 },
                width: 58,
                height: 34,
            },
            backend.metrics(),
        )
        .unwrap();
    assert_eq!(prepared.extended_style_reads, 1);
    assert!(
        !prepared.scene.passes[4]
            .iter()
            .any(|p| matches!(p, Primitive::Text { .. }))
    );
    assert!(prepared.scene.passes[1].iter().any(|p| matches!(
        p,
        Primitive::FillPath { .. } | Primitive::Circle { .. } | Primitive::StrokePath { .. }
    )));
}

/// The compiled gradient data bar paints the resolved colors: the light shade
/// at the axis side and the full color at the tip.
///
/// Two rules with different colors share one preparation, so the shade memo
/// must keep them apart.
#[test]
fn a_compiled_gradient_bar_uses_the_resolved_shade() {
    let mut model = load(FIXTURE);
    for (row, value) in [(2, "5"), (3, "10"), (4, "0")] {
        model
            .set_user_input(0, row, 8, value)
            .expect("write H cell");
    }
    for (row, value) in [(1, "5"), (2, "10"), (3, "0")] {
        model
            .set_user_input(0, row, 26, value)
            .expect("write Z cell");
    }
    model
        .add_conditional_formatting(
            0,
            "Z1:Z3",
            CfRuleInput::DataBar {
                min: Some(Cfvo::Number(0.0)),
                max: Some(Cfvo::Number(10.0)),
                positive_color: Color::Rgb("#ff00ff".to_string()),
                negative_color: Color::Rgb("#ff0000".to_string()),
                is_gradient: true,
                show_value: true,
            },
        )
        .expect("add rule");
    model.evaluate();

    let cell = |row: i32, col: i32, y: i32| {
        (
            CellCoord { row, col },
            PixelRect {
                top_left: Point { x: 0, y },
                width: 100,
                height: 30,
            },
        )
    };
    let cells = [cell(2, 8, 0), cell(1, 26, 30)];
    let backend = SvgSceneBackend::new();
    let prepared = WorksheetCompiler::new()
        .prepare_cells(
            WorksheetContext {
                workbook: &model,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 100,
                height: 60,
            },
            cells.iter().map(|(coordinate, bounds)| CellPlacement {
                coordinate: *coordinate,
                bounds: *bounds,
            }),
            backend.metrics(),
        )
        .expect("cell preparation");

    for (coordinate, bounds) in cells {
        let bar = decoration(&model, 0, coordinate.row, coordinate.col)
            .data_bar
            .expect("a data-bar cell");
        let gradient = prepared.scene.passes[1]
            .iter()
            .find_map(|op| match op {
                Primitive::Gradient { rect, from, to }
                    if rect.top() >= bounds.top() && rect.bottom() <= bounds.bottom() =>
                {
                    Some((from.to_string(), to.to_string()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("a gradient data bar at {coordinate:?}"));
        let expected = if bar.value >= bar.axis_position {
            (bar.positive_light.to_string(), bar.positive.to_string())
        } else {
            (bar.negative.to_string(), bar.negative_light.to_string())
        };
        assert_eq!(gradient, expected, "{coordinate:?}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cf_fixture_shared_scene_reaches_native_raster_at_two_scales() {
    use iron_canvas_native::NativeSceneBackend;
    let workbook = load(FIXTURE);
    let svg = SvgSceneBackend::new();
    for sheet in [0, 1] {
        let prepared = WorksheetCompiler::new()
            .prepare(
                WorksheetContext {
                    workbook: &workbook,
                    sheet,
                    theme: &CanvasTheme::light(),
                },
                WorksheetViewport {
                    first_cell: CellCoord { row: 1, col: 1 },
                    width: 1100,
                    height: 700,
                },
                svg.metrics(),
            )
            .unwrap();
        for scale in [1.0, 1.25] {
            let mut backend = NativeSceneBackend::new(scale).unwrap();
            let staged = backend.stage_frame(&prepared.scene).unwrap();
            backend.commit_frame(staged).unwrap();
            let surface = backend.surface().unwrap();
            let mut colorful = 0;
            surface
                .with_data(|pixels| {
                    for &pixel in pixels.as_chunks::<4>().0 {
                        // Cairo ARGB32 uses native-endian premultiplied channels.
                        let argb = u32::from_ne_bytes(pixel);
                        let red = ((argb >> 16) & 255) as u8;
                        let green = ((argb >> 8) & 255) as u8;
                        if argb >> 24 > 0 && red.abs_diff(green) > 30 {
                            colorful += 1;
                        }
                    }
                })
                .unwrap();
            assert!(
                colorful > 500,
                "sheet {sheet}, scale {scale}: {colorful} colored pixels"
            );
        }
    }
}
