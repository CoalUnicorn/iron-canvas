//! Border lowering. Dash styles keep the baseline's solid-stroke policy.
use iron_canvas_core::{CanvasTheme, PixelRect, Side, painter::PaintColor, scene::SceneBuilder};
use ironcalc_base::{
    UserModel,
    types::{Border, BorderStyle as B},
};

pub(super) fn compile_borders(
    out: &SceneBuilder,
    grid: &SceneBuilder,
    workbook: &UserModel<'_>,
    border: &Border,
    rect: PixelRect,
    show_grid: bool,
    theme: &CanvasTheme,
) {
    for (side, item) in [
        (Side::Left, &border.left),
        (Side::Top, &border.top),
        (Side::Right, &border.right),
        (Side::Bottom, &border.bottom),
    ] {
        if let Some(item) = item {
            let color = workbook.resolve_color(&item.color);
            let color = if color.is_empty() {
                &theme.grid_color
            } else {
                color.as_str()
            };
            let (width, double) = match item.style {
                B::Medium | B::MediumDashed | B::MediumDashDot | B::MediumDashDotDot => (2, false),
                B::Thick => (3, false),
                B::Double => (1, true),
                B::Thin | B::Dotted | B::SlantDashDot => (1, false),
            };
            let line = side.line(rect).extend(width / 2);
            for &offset in if double { &[-1, 1][..] } else { &[0][..] } {
                out.stroke_line(
                    line.offset_cross(offset),
                    PaintColor::Borrowed(color),
                    f64::from(width),
                );
            }
        } else if show_grid && matches!(side, Side::Left | Side::Top) {
            grid.stroke_line(
                side.line(rect),
                PaintColor::Borrowed(&theme.grid_color),
                1.0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::{Point, scene::Primitive};
    use ironcalc_base::types::{BorderItem, Color};
    #[test]
    fn explicit_double_border_uses_two_strokes_and_suppresses_the_grid_edge() {
        let workbook = UserModel::new_empty("borders", "en", "UTC", "en").unwrap();
        let border = Border {
            left: Some(BorderItem {
                style: B::Double,
                color: Color::Rgb("#123456".into()),
            }),
            ..Default::default()
        };
        let explicit = SceneBuilder::default();
        let grid = SceneBuilder::default();
        compile_borders(
            &explicit,
            &grid,
            &workbook,
            &border,
            PixelRect {
                top_left: Point { x: 10, y: 10 },
                width: 80,
                height: 30,
            },
            true,
            &CanvasTheme::light(),
        );
        let grid = grid.finish();
        assert_eq!(grid.len(), 1);
        let strokes = explicit.finish();
        assert_eq!(strokes.len(), 2);
        assert!(strokes.iter().all(|op| matches!(op, Primitive::Line { color, width, .. } if &**color == "#123456" && *width == 1.0)));
    }
}
