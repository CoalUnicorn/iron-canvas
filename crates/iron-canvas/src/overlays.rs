//! Spreadsheet overlay composition from owned, already prepared content.

use crate::interaction::{LAST_COLUMN, LAST_ROW, autofill_handle_rect, logical_range};
use iron_canvas_core::{
    CanvasTheme, CellCoord, PixelRect, Point, Side,
    geometry::{
        constants::{DASHED_RECT_PATTERN, DEFAULT_MITER_LIMIT, STANDARD_BORDER_WIDTH},
        path::{Path, PathCmd, PointF},
    },
    painter::{LineCap, LineJoin, PaintColor, StrokeStyle},
    scene::{Primitive, SceneBuilder},
    scene_geometry::GridRange,
    theme::{FORMULA_REF_COLORS, FORMULA_REF_TINTS},
};
use iron_canvas_ironcalc::PreparedWorksheet;
use std::rc::Rc;

const HEADER_SELECTION_STROKE_WIDTH: f64 = 2.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaOverlay {
    pub sheet: u32,
    pub range: GridRange,
    pub color_index: usize,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayState {
    pub selection: Vec<GridRange>,
    pub active_cell: Option<CellCoord>,
    pub clipboard: Option<GridRange>,
    /// The host resolves formula references. This type describes only paint.
    pub formula_references: Vec<FormulaOverlay>,
}
/// Append the overlay passes to a prepared worksheet's scene. Call it on a
/// freshly prepared worksheet: an already composed scene would replay its own
/// overlay passes inside the restore clip.
pub(crate) fn compose(
    worksheet: &mut PreparedWorksheet,
    sheet: u32,
    state: &OverlayState,
    theme: &CanvasTheme,
) {
    let Some(geometry) = worksheet.geometry.as_ref() else {
        return;
    };
    let fills = SceneBuilder::default();
    let strokes = SceneBuilder::default();
    for range in &state.selection {
        for fragment in geometry.fragments(*range) {
            fills.rect_fill(fragment.rect, PaintColor::Borrowed(&theme.selection_fill));
            outline(
                &strokes,
                fragment.rect,
                fragment.edges,
                &theme.selection_color,
                false,
            );
        }
    }
    // Re-paint the active cell's model content above the selection fill. The
    // passes are re-referenced, so the restore copies nothing; merged cells
    // paint in their own passes, so every model pass must replay.
    let mut restore: Vec<Rc<[Primitive]>> = Vec::new();
    if let Some(active) = state.active_cell {
        let model_passes = worksheet.scene.passes.len();
        for fragment in geometry.fragments(logical_range(worksheet, active)) {
            restore.push(Rc::from([Primitive::Clip {
                rect: fragment.rect,
            }]));
            restore.extend(worksheet.scene.passes[..model_passes].iter().map(Rc::clone));
            restore.push(Rc::from([Primitive::EndClip]));
            outline(
                &strokes,
                fragment.rect,
                fragment.edges,
                &theme.selection_color,
                false,
            );
        }
    }
    if let Some(range) = state.clipboard {
        for fragment in geometry.fragments(range) {
            outline(
                &strokes,
                fragment.rect,
                fragment.edges,
                &theme.selection_color,
                true,
            );
        }
    }
    let formula = SceneBuilder::default();
    for reference in &state.formula_references {
        if reference.sheet != sheet {
            continue;
        }
        let index = reference.color_index % FORMULA_REF_COLORS.len();
        for fragment in geometry.fragments(reference.range) {
            formula.rect_fill(fragment.rect, PaintColor::Static(FORMULA_REF_TINTS[index]));
            outline(
                &formula,
                fragment.rect,
                fragment.edges,
                FORMULA_REF_COLORS[index],
                true,
            );
        }
    }
    // Header highlights stay clipped to their strips and preserve the labels.
    let header = SceneBuilder::default();
    let mut highlighted = Vec::new();
    for range in &state.selection {
        for fragment in geometry.fragments(*range) {
            let row = PixelRect {
                top_left: Point {
                    x: 0,
                    y: fragment.rect.top(),
                },
                width: geometry.origin.x,
                height: fragment.rect.height,
            };
            let col = PixelRect {
                top_left: Point {
                    x: fragment.rect.left(),
                    y: 0,
                },
                width: fragment.rect.width,
                height: geometry.origin.y,
            };
            for (rect, side) in [(row, Side::Right), (col, Side::Bottom)] {
                if !highlighted.contains(&rect) {
                    header.rect_fill(rect, PaintColor::Borrowed(&theme.selection_fill));
                    header.stroke_line(
                        side.line(rect),
                        PaintColor::Borrowed(&theme.selection_color),
                        HEADER_SELECTION_STROKE_WIDTH,
                    );
                    highlighted.push(rect);
                }
            }
        }
    }
    let autofill_range = state
        .active_cell
        .and_then(|active| {
            state
                .selection
                .iter()
                .rev()
                .find(|range| range.contains(active))
                .copied()
        })
        .or_else(|| state.selection.last().copied())
        .or_else(|| {
            state
                .active_cell
                .map(|active| logical_range(worksheet, active))
        });
    let autofill = SceneBuilder::default();
    if let Some(range) = autofill_range {
        let last = range.normalized().last;
        if last.row < LAST_ROW
            && last.col < LAST_COLUMN
            && let Some(rect) = autofill_handle_rect(geometry, range)
        {
            autofill.rect_fill(rect, PaintColor::Borrowed(&theme.selection_color));
        }
    }
    worksheet.scene.passes.push(fills.finish());
    worksheet.scene.passes.extend(restore);
    worksheet.scene.passes.push(strokes.finish());
    worksheet.scene.passes.push(formula.finish());
    worksheet.scene.passes.push(header.finish());
    worksheet.scene.passes.push(autofill.finish());
}
fn outline(out: &SceneBuilder, rect: PixelRect, edges: [bool; 4], color: &str, dashed: bool) {
    if rect.width <= 1 || rect.height <= 1 {
        return;
    }
    // Half-pixel inset so a 1px stroke lands on the pixel grid.
    let x = f64::from(rect.left()) + 0.5;
    let y = f64::from(rect.top()) + 0.5;
    let right = f64::from(rect.right()) - 0.5;
    let bottom = f64::from(rect.bottom()) - 0.5;
    let mut cmds = Vec::with_capacity(8);
    for side in Side::ALL {
        if !edges[side.index()] {
            continue;
        }
        let (from, to) = side_segment(side, x, y, right, bottom);
        cmds.extend([PathCmd::Move(from), PathCmd::Line(to)]);
    }
    out.stroke_path(
        &Path::new(&cmds),
        PaintColor::Borrowed(color),
        &StrokeStyle {
            width: f64::from(STANDARD_BORDER_WIDTH),
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: DEFAULT_MITER_LIMIT,
            dash: if dashed {
                &DASHED_RECT_PATTERN[..]
            } else {
                &[]
            },
        },
    );
}

/// Endpoints of one side of the half-pixel-inset outline.
fn side_segment(side: Side, x: f64, y: f64, right: f64, bottom: f64) -> (PointF, PointF) {
    match side {
        Side::Left => (PointF::new(x, bottom), PointF::new(x, y)),
        Side::Top => (PointF::new(x, y), PointF::new(right, y)),
        Side::Right => (PointF::new(right, y), PointF::new(right, bottom)),
        Side::Bottom => (PointF::new(right, bottom), PointF::new(x, bottom)),
    }
}
