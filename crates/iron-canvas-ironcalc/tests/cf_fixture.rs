//! Engine CF fixture checks for both the legacy adapter and the M1 compiler.
//! The SVG test uses the resolved scene backend. Browser pixels are covered
//! by `iron-canvas-web/tests/cf_fixture_canvas2d.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use iron_canvas_core::scene::{Primitive, RenderBackend};
use iron_canvas_core::{CanvasTheme, CellCoord};
use iron_canvas_core::{CellDecoration, IconGlyph, RatingStyle};
use iron_canvas_export::SvgSceneBackend;
use iron_canvas_ironcalc::convert::cell_decoration_from_extended;
use iron_canvas_ironcalc::{WorksheetCompiler, WorksheetViewport, color_resolver};
use ironcalc::import::{load_from_xlsx, load_from_xlsx_bytes};
use ironcalc_base::{Model, UserModel};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cf_tests.xlsx");

fn load(path: &str) -> UserModel<'static> {
    UserModel::from_model(load_from_xlsx(path, "en", "UTC", "en").expect("workbook loads"))
}

/// The adapter's decoration for one cell, with colors resolved through the
/// workbook theme exactly as the app's adapter does.
fn decoration(
    model: &UserModel<'static>,
    sheet: u32,
    row: i32,
    col: i32,
) -> Option<CellDecoration> {
    let ext = model
        .get_extended_cell_style(sheet, row, col)
        .expect("cell read");
    cell_decoration_from_extended(&ext, &color_resolver(model))
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
        .expect("B17 has a decoration")
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
        .expect("H2 has a decoration")
        .data_bar
        .expect("H2 is a data-bar cell");
    assert!(gradient.is_gradient, "H2 is a gradient bar");

    // I2:I8 — solid bars from the x14 gradient="0" override.
    let solid = decoration(&model, 0, 2, 9)
        .expect("I2 has a decoration")
        .data_bar
        .expect("I2 is a data-bar cell");
    assert!(
        !solid.is_gradient,
        "the x14 override must force a solid bar"
    );

    // J2:J8 — solid bars, bounds -5..10, hidden values. Zero sits at one third
    // of the range, so the axis is not at the left edge.
    let axis = decoration(&model, 0, 2, 10)
        .expect("J2 has a decoration")
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
        .expect("L22 has a decoration")
        .rating
        .expect("L22 is a rating cell");
    assert_eq!(star.style, RatingStyle::RepeatedGlyph(IconGlyph::Star));
    assert_eq!(
        star.color.as_deref().map(str::to_lowercase).as_deref(),
        Some("#ffd700")
    );
    assert_eq!((star.count, star.max), (1, 3));

    let circle = decoration(&model, 1, 23, 3)
        .expect("M23 has a decoration")
        .rating
        .expect("M23 is a rating cell");
    assert_eq!(circle.style, RatingStyle::FiveQuarters);
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
            &model,
            0,
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 1100,
                height: 700,
            },
            backend.metrics(),
            &CanvasTheme::light(),
        )
        .unwrap();
    assert_eq!(prepared.extended_style_reads, prepared.cells.len());
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
            &model,
            0,
            WorksheetViewport {
                first_cell: CellCoord { row: 17, col: 2 },
                width: 58,
                height: 34,
            },
            backend.metrics(),
            &CanvasTheme::light(),
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

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cf_fixture_shared_scene_reaches_native_raster_at_two_scales() {
    use iron_canvas_native::NativeSceneBackend;
    let workbook = load(FIXTURE);
    let svg = SvgSceneBackend::new();
    for sheet in [0, 1] {
        let prepared = WorksheetCompiler::new()
            .prepare(
                &workbook,
                sheet,
                WorksheetViewport {
                    first_cell: CellCoord { row: 1, col: 1 },
                    width: 1100,
                    height: 700,
                },
                svg.metrics(),
                &CanvasTheme::light(),
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
                    for pixel in pixels.chunks_exact(4) {
                        // Cairo ARGB32 uses native-endian premultiplied channels.
                        let argb = u32::from_ne_bytes(pixel.try_into().unwrap());
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
