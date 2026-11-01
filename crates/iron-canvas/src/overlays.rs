//! Spreadsheet overlay composition from owned, already prepared content.

use crate::interaction::{LAST_COLUMN, LAST_ROW, autofill_handle_rect, logical_range};
use crate::selection::{SelectionKind, SelectionSnapshot, union_ranges};
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
    /// A snapshot replaces legacy selection and active-cell paint input.
    pub selection_snapshot: Option<SelectionSnapshot>,
    pub selection: Vec<GridRange>,
    pub active_cell: Option<CellCoord>,
    pub clipboard: Option<GridRange>,
    /// The host resolves formula references. This type describes only paint.
    pub formula_references: Vec<FormulaOverlay>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayError {
    Workbook,
    Sheet,
}
impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "selection snapshot mismatch: {self:?}")
    }
}
impl std::error::Error for OverlayError {}
impl OverlayState {
    pub(crate) fn validate(&self, request: &crate::RenderRequest) -> Result<(), OverlayError> {
        if let Some(selection) = &self.selection_snapshot {
            if selection.workbook_id() != request.revision.workbook_id {
                return Err(OverlayError::Workbook);
            }
            if selection.sheet() != request.sheet {
                return Err(OverlayError::Sheet);
            }
        }
        Ok(())
    }
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
    // Logical coverage stays disjoint before pane clipping. This prevents
    // overlapping areas from applying the transparent fill more than once.
    let ranges: Vec<_> = state.selection_snapshot.as_ref().map_or_else(
        || state.selection.clone(),
        |snapshot| snapshot.areas().iter().map(|area| area.range()).collect(),
    );
    let active = state
        .selection_snapshot
        .as_ref()
        .map_or(state.active_cell, |snapshot| Some(snapshot.active_cell()));
    let active_index = state.selection_snapshot.as_ref().map_or_else(
        || {
            active
                .and_then(|cell| ranges.iter().rposition(|range| range.contains(cell)))
                .or_else(|| ranges.len().checked_sub(1))
        },
        |snapshot| {
            snapshot
                .areas()
                .iter()
                .position(|area| area.id() == snapshot.active_area())
        },
    );
    let mut fills = SceneBuilder::default();
    let mut strokes = SceneBuilder::default();
    for range in union_ranges(ranges.iter().copied()) {
        for fragment in geometry.fragments(range) {
            fills.rect_fill(fragment.rect, PaintColor::Borrowed(&theme.selection_fill));
        }
    }
    // Keep area boundaries separate from coverage pieces. Decomposition seams
    // have no visible outline and cannot become future gesture borders.
    for (index, range) in ranges.iter().enumerate() {
        for fragment in geometry.fragments(*range) {
            outline(
                &mut strokes,
                fragment.rect,
                fragment.edges,
                &theme.selection_color,
                active_index != Some(index),
            );
        }
    }
    // Re-paint the active cell's model content above the selection fill. The
    // passes are re-referenced, so the restore copies nothing; merged cells
    // paint in their own passes, so every model pass must replay.
    let mut restore: Vec<Rc<[Primitive]>> = Vec::new();
    if let Some(active) = active {
        let model_passes = worksheet.scene.passes.len();
        for fragment in geometry.fragments(logical_range(worksheet, active)) {
            restore.push(Rc::from([Primitive::Clip {
                rect: fragment.rect,
            }]));
            restore.extend(worksheet.scene.passes[..model_passes].iter().map(Rc::clone));
            restore.push(Rc::from([Primitive::EndClip]));
            outline(
                &mut strokes,
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
                &mut strokes,
                fragment.rect,
                fragment.edges,
                &theme.selection_color,
                true,
            );
        }
    }
    let mut formula = SceneBuilder::default();
    for reference in &state.formula_references {
        if reference.sheet != sheet {
            continue;
        }
        let index = reference.color_index % FORMULA_REF_COLORS.len();
        for fragment in geometry.fragments(reference.range) {
            formula.rect_fill(fragment.rect, PaintColor::Static(FORMULA_REF_TINTS[index]));
            outline(
                &mut formula,
                fragment.rect,
                fragment.edges,
                FORMULA_REF_COLORS[index],
                true,
            );
        }
    }
    // Header highlights stay clipped to their strips and preserve the labels.
    // A full-row selection highlights the row strip only. A full-column
    // selection highlights the column strip only. Cells and whole sheets keep
    // the covered row and column spans.
    let mut header = SceneBuilder::default();
    let mut rows = Vec::new();
    let mut columns = Vec::new();
    let kinds: Vec<Option<SelectionKind>> = state.selection_snapshot.as_ref().map_or_else(
        || ranges.iter().map(|_| None).collect(),
        |snapshot| {
            snapshot
                .areas()
                .iter()
                .map(|area| Some(area.target().kind()))
                .collect()
        },
    );
    for (range, kind) in ranges.iter().zip(kinds) {
        let (row_strip, column_strip) = header_strips(*range, kind);
        for fragment in geometry.fragments(*range) {
            if row_strip {
                rows.push((fragment.rect.top(), fragment.rect.bottom()));
            }
            if column_strip {
                columns.push((fragment.rect.left(), fragment.rect.right()));
            }
        }
    }
    for (spans, side) in [(&mut rows, Side::Right), (&mut columns, Side::Bottom)] {
        spans.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::new();
        for &(start, end) in spans.iter() {
            if let Some(last) = merged.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        for (start, end) in merged {
            let rect = if side == Side::Right {
                PixelRect {
                    top_left: Point { x: 0, y: start },
                    width: geometry.origin.x,
                    height: end - start,
                }
            } else {
                PixelRect {
                    top_left: Point { x: start, y: 0 },
                    width: end - start,
                    height: geometry.origin.y,
                }
            };
            header.rect_fill(rect, PaintColor::Borrowed(&theme.selection_fill));
            header.stroke_line(
                side.line(rect),
                PaintColor::Borrowed(&theme.selection_color),
                HEADER_SELECTION_STROKE_WIDTH,
            );
        }
    }
    let single_finite = state
        .selection_snapshot
        .as_ref()
        .map_or(ranges.len() <= 1, |snapshot| {
            snapshot.areas().len() == 1
                && snapshot.areas()[0].target().kind() == SelectionKind::Cells
        });
    let autofill_range = if single_finite {
        ranges
            .first()
            .copied()
            .or_else(|| active.map(|cell| logical_range(worksheet, cell)))
    } else {
        None
    };
    let mut autofill = SceneBuilder::default();
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

/// Which header strips a selection covers.
fn header_strips(range: GridRange, kind: Option<SelectionKind>) -> (bool, bool) {
    if let Some(kind) = kind {
        return match kind {
            SelectionKind::Rows => (true, false),
            SelectionKind::Columns => (false, true),
            SelectionKind::Cells | SelectionKind::Sheet => (true, true),
        };
    }
    let range = range.normalized();
    let full_rows = range.first.col == 1 && range.last.col == LAST_COLUMN;
    let full_columns = range.first.row == 1 && range.last.row == LAST_ROW;
    (full_rows || !full_columns, full_columns || !full_rows)
}

fn outline(out: &mut SceneBuilder, rect: PixelRect, edges: [bool; 4], color: &str, dashed: bool) {
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
