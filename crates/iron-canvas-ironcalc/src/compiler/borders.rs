//! Border lowering. Dash styles keep the baseline's solid-stroke policy.
use super::WorksheetContext;
use iron_canvas_core::{PixelRect, Side, painter::PaintColor, scene::SceneBuilder};
use ironcalc_base::types::{Border, BorderStyle as B};

/// How one border edge is stroked. Dash styles keep the solid baseline, so
/// only the width and the double-line treatment reach `Painter` primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// One line of this width in logical pixels.
    Single(i32),
    /// Two parallel lines at +-1 on the cross axis.
    Double,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct GridEdges {
    pub(super) top: bool,
    pub(super) left: bool,
}

impl Rule {
    /// Extension at each end along the segment.
    /// Where two perpendicular edges meet at a cell corner the thicker
    /// leaves an uncovered notch, extending each edge by half its
    /// width fills it.
    fn miter_extend(self) -> i32 {
        match self {
            Rule::Single(width) => width / 2,
            Rule::Double => 0,
        }
    }

    /// Cross axis offsets at which this rule strokes.
    fn cross_offsets(self) -> &'static [i32] {
        match self {
            Rule::Single(_) => &[0],
            Rule::Double => &[-1, 1],
        }
    }

    /// Stroke width in logical pixels.
    fn width(self) -> f64 {
        match self {
            Rule::Single(width) => f64::from(width),
            Rule::Double => 1.0,
        }
    }
}

pub(super) fn compile_borders(
    out: &mut SceneBuilder,
    grid: &mut SceneBuilder,
    context: WorksheetContext<'_, '_>,
    border: &Border,
    rect: PixelRect,
    grid_edges: GridEdges,
) {
    let WorksheetContext {
        workbook, theme, ..
    } = context;
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
            let rule = match item.style {
                B::Medium | B::MediumDashed | B::MediumDashDot | B::MediumDashDotDot => {
                    Rule::Single(2)
                }
                B::Thick => Rule::Single(3),
                B::Double => Rule::Double,
                B::Thin | B::Dotted | B::SlantDashDot => Rule::Single(1),
            };
            let line = side.line(rect).extend(rule.miter_extend());
            for &offset in rule.cross_offsets() {
                out.stroke_line(
                    line.offset_cross(offset),
                    PaintColor::Borrowed(color),
                    rule.width(),
                );
            }
        } else if (side == Side::Top && grid_edges.top) || (side == Side::Left && grid_edges.left) {
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
    use iron_canvas_core::{CanvasTheme, Point, scene::Primitive};
    use ironcalc_base::UserModel;
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
        let mut explicit = SceneBuilder::default();
        let mut grid = SceneBuilder::default();
        compile_borders(
            &mut explicit,
            &mut grid,
            WorksheetContext {
                workbook: &workbook,
                sheet: 0,
                theme: &CanvasTheme::light(),
            },
            &border,
            PixelRect {
                top_left: Point { x: 10, y: 10 },
                width: 80,
                height: 30,
            },
            GridEdges {
                top: true,
                left: true,
            },
        );
        let grid = grid.finish();
        assert_eq!(grid.len(), 1);
        let strokes = explicit.finish();
        assert_eq!(strokes.len(), 2);
        assert!(strokes.iter().all(|op| matches!(op, Primitive::Line { color, width, .. } if &**color == "#123456" && *width == 1.0)));
    }
}
