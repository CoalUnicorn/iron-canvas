//! Read-only IronCalc worksheet compilation. No legacy model adapter is used.
mod borders;
mod cf;
mod glyphs;
mod text;

use iron_canvas_core::{CanvasTheme, CellCoord, PixelRect, Point};
use iron_canvas_core::{
    painter::{PaintColor, TextMetrics},
    scene::{GridScene, SceneBuilder},
    scene_geometry::{AxisError, sample_axis},
};
use ironcalc_base::{UserModel, cf_types::ExtendedStyle, types::CellType};
use std::{collections::HashMap, fmt};

// The pinned engine keeps these limits private. Keep the worksheet limits
// here, outside the generic core geometry.
const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;

/// A cell and its full logical drawing rectangle. Multiple fragments can share
/// a coordinate; the compiler reads its engine content once per preparation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellPlacement {
    pub coordinate: CellCoord,
    pub bounds: PixelRect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorksheetViewport {
    pub first_cell: CellCoord,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedWorksheet {
    pub scene: GridScene,
    pub cells: Vec<CellPlacement>,
    /// Actual calls to `get_extended_cell_style` in this preparation.
    pub extended_style_reads: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellReadOperation {
    Request,
    Sheet,
    RowHeight,
    ColumnWidth,
    ExtendedStyle,
    CellType,
    FormattedValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorksheetCompileError {
    pub sheet: u32,
    pub cell: CellCoord,
    pub operation: CellReadOperation,
    pub message: String,
}
impl fmt::Display for WorksheetCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to read {:?} for sheet {}, cell ({}, {}): {}",
            self.operation, self.sheet, self.cell.row, self.cell.col, self.message
        )
    }
}
impl std::error::Error for WorksheetCompileError {}

#[derive(Debug, Default, Clone, Copy)]
pub struct WorksheetCompiler;
impl WorksheetCompiler {
    pub const fn new() -> Self {
        Self
    }

    /// Prepare a Fresh viewport. Header, merge, freeze, and overlay composition
    /// belong to M2. The selected workbook view is never changed.
    pub fn prepare(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        viewport: WorksheetViewport,
        metrics: &dyn TextMetrics,
        theme: &CanvasTheme,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        let first = viewport.first_cell;
        let error = |operation, message: String| WorksheetCompileError {
            sheet,
            cell: first,
            operation,
            message,
        };
        // Validate the sheet even for a zero-sized viewport.
        workbook
            .get_show_grid_lines(sheet)
            .map_err(|e| error(CellReadOperation::Sheet, e))?;
        if first.row < 1
            || first.row > LAST_ROW
            || first.col < 1
            || first.col > LAST_COLUMN
            || viewport.width < 0
            || viewport.height < 0
        {
            return Err(error(
                CellReadOperation::Request,
                "invalid viewport bounds".into(),
            ));
        }
        let axis_error = |e: AxisError<String>, operation| error(operation, format!("{e:?}"));
        let rows = sample_axis(first.row, LAST_ROW, viewport.height, |row| {
            workbook.get_row_height(sheet, row)
        })
        .map_err(|e| axis_error(e, CellReadOperation::RowHeight))?;
        let cols = sample_axis(first.col, LAST_COLUMN, viewport.width, |col| {
            workbook.get_column_width(sheet, col)
        })
        .map_err(|e| axis_error(e, CellReadOperation::ColumnWidth))?;
        let cells = rows.iter().flat_map(|row| {
            cols.iter().map(move |col| CellPlacement {
                coordinate: CellCoord {
                    row: row.index,
                    col: col.index,
                },
                bounds: PixelRect {
                    top_left: Point {
                        x: col.start,
                        y: row.start,
                    },
                    width: col.extent,
                    height: row.extent,
                },
            })
        });
        self.prepare_cells(
            workbook,
            sheet,
            PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: viewport.width,
                height: viewport.height,
            },
            cells,
            metrics,
            theme,
        )
    }

    /// Lower positioned cells into five shared paint buffers. Repeated
    /// coordinates reuse one preparation-local engine record.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_cells(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        clip: PixelRect,
        cells: impl IntoIterator<Item = CellPlacement>,
        metrics: &dyn TextMetrics,
        theme: &CanvasTheme,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        let err = |cell, operation, message| WorksheetCompileError {
            sheet,
            cell,
            operation,
            message,
        };
        let first = CellCoord { row: 1, col: 1 };
        let show_grid = workbook
            .get_show_grid_lines(sheet)
            .map_err(|e| err(first, CellReadOperation::Sheet, e))?;
        let valid_rect = |rect: PixelRect| {
            rect.width >= 0
                && rect.height >= 0
                && rect.left().checked_add(rect.width).is_some()
                && rect.top().checked_add(rect.height).is_some()
        };
        if !valid_rect(clip) {
            return Err(err(
                first,
                CellReadOperation::Request,
                "invalid viewport clip".into(),
            ));
        }
        let passes: [SceneBuilder; 5] = std::array::from_fn(|_| SceneBuilder::default());
        passes[0].rect_fill(clip, PaintColor::Borrowed(&theme.cell_bg));
        let mut reads = 0;
        let mut cached: HashMap<(i32, i32), (ExtendedStyle, CellType, String)> = HashMap::new();
        let mut placements = Vec::new();
        let mut lines = Vec::new();
        let mut scratch = String::new();
        for placement in cells {
            let coordinate = placement.coordinate;
            if coordinate.row < 1
                || coordinate.row > LAST_ROW
                || coordinate.col < 1
                || coordinate.col > LAST_COLUMN
                || !valid_rect(placement.bounds)
            {
                return Err(err(
                    coordinate,
                    CellReadOperation::Request,
                    "invalid cell placement".into(),
                ));
            }
            let record = match cached.entry((coordinate.row, coordinate.col)) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    reads += 1;
                    let style = workbook
                        .get_extended_cell_style(sheet, coordinate.row, coordinate.col)
                        .map_err(|e| {
                            err(
                                coordinate,
                                CellReadOperation::ExtendedStyle,
                                format!("{e:?}"),
                            )
                        })?;
                    let kind = workbook
                        .get_cell_type(sheet, coordinate.row, coordinate.col)
                        .map_err(|e| err(coordinate, CellReadOperation::CellType, e))?;
                    let text = workbook
                        .get_formatted_cell_value(sheet, coordinate.row, coordinate.col)
                        .map_err(|e| err(coordinate, CellReadOperation::FormattedValue, e))?;
                    entry.insert((style, kind, text))
                }
            };
            let (extended, kind, value) = record;
            let rect = placement.bounds;
            let style = &extended.style;
            let fill = workbook.resolve_color(&style.fill.color);
            if !fill.is_empty() {
                passes[0].rect_fill(rect, PaintColor::Borrowed(&fill));
            }
            let cf = cf::CfDecorationPaint::resolve(extended, workbook);
            cf.paint(&passes[1], rect);
            borders::compile_borders(
                &passes[3],
                &passes[2],
                workbook,
                &style.border,
                rect,
                show_grid && fill.is_empty(),
                theme,
            );
            if !cf.hides_value() {
                text::compile_text(
                    &passes[4],
                    workbook,
                    style,
                    kind,
                    value,
                    rect,
                    cf.reserved_left(rect),
                    metrics,
                    theme,
                    &mut lines,
                    &mut scratch,
                );
            }
            placements.push(placement);
        }
        Ok(PreparedWorksheet {
            scene: GridScene {
                clip,
                passes: passes.into_iter().map(SceneBuilder::finish).collect(),
            },
            cells: placements,
            extended_style_reads: reads,
        })
    }
}
