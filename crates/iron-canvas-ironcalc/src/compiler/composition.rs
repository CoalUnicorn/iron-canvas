use super::*;
use iron_canvas_core::{
    Point, Side,
    painter::{TextAlign, TextBaseline},
    scene_geometry::{AxisCell, AxisError, GridGeometry, GridRange, sample_bands},
};
use ironcalc_base::{expressions::utils::number_to_column, types::Border};

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    workbook: &UserModel<'_>,
    sheet: u32,
    viewport: WorksheetViewport,
    metrics: &dyn TextMetrics,
    theme: &CanvasTheme,
    previous: Option<&Rc<GridGeometry>>,
) -> Result<PreparedWorksheet, WorksheetCompileError> {
    let mut attempt = Attempt::new(workbook, sheet, metrics, theme)?;
    let first = viewport.first_cell;
    if !(1..=LAST_ROW).contains(&first.row)
        || !(1..=LAST_COLUMN).contains(&first.col)
        || viewport.width < 0
        || viewport.height < 0
    {
        return Err(attempt.error(
            first,
            CellReadOperation::Request,
            "invalid viewport bounds".into(),
        ));
    }
    let err = |op, e| attempt.error(first, op, e);
    let frozen_rows = workbook
        .get_frozen_rows_count(sheet)
        .map_err(|e| err(CellReadOperation::Freeze, e))?;
    let frozen_cols = workbook
        .get_frozen_columns_count(sheet)
        .map_err(|e| err(CellReadOperation::Freeze, e))?;
    let merges = workbook
        .get_merged_cells(sheet)
        .map_err(|e| err(CellReadOperation::Merges, e))?;
    for merge in &merges {
        if merge.row < 1
            || merge.column < 1
            || merge.width < 1
            || merge.height < 1
            || merge
                .row
                .checked_add(merge.height - 1)
                .is_none_or(|v| v > LAST_ROW)
            || merge
                .column
                .checked_add(merge.width - 1)
                .is_none_or(|v| v > LAST_COLUMN)
        {
            return Err(err(
                CellReadOperation::Merges,
                "invalid merge bounds".into(),
            ));
        }
    }
    let mut dimensions = Dimensions {
        workbook,
        sheet,
        rows: HashMap::new(),
        cols: HashMap::new(),
    };
    let header_height = 24.min(viewport.height);
    let rows = sample_bands(
        first.row,
        LAST_ROW,
        frozen_rows,
        header_height,
        viewport.height - header_height,
        |i| dimensions.extent(true, i),
    )
    .map_err(|e| axis_error(sheet, first, true, e))?;
    let label = rows
        .iter()
        .flat_map(|b| &b.cells)
        .map(|c| c.index)
        .max()
        .unwrap_or(first.row)
        .to_string();
    let header_width = (metrics.measure_text_width(&label, "12px sans-serif").ceil() as i32)
        .saturating_add(16)
        .max(48)
        .min(viewport.width);
    let columns = sample_bands(
        first.col,
        LAST_COLUMN,
        frozen_cols,
        header_width,
        viewport.width - header_width,
        |i| dimensions.extent(false, i),
    )
    .map_err(|e| axis_error(sheet, first, false, e))?;
    let clip = PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: viewport.width,
        height: viewport.height,
    };
    let origin = Point {
        x: header_width,
        y: header_height,
    };
    let geometry = if let Some(old) = previous.filter(|old| {
        old.bounds == clip
            && old.origin == origin
            && old.row_bands == rows
            && old.column_bands == columns
    }) {
        Rc::clone(old)
    } else {
        Rc::new(GridGeometry::new(clip, origin, rows, columns))
    };
    let cell_count: usize = geometry
        .panes
        .iter()
        .map(|p| p.rows.len() * p.columns.len())
        .sum();
    let clips = geometry.panes.len() * 2;
    let passes = std::array::from_fn(|index| {
        SceneBuilder::with_capacity(match index {
            2 => cell_count * 2 + clips,
            4 => cell_count * 3 + clips,
            _ => 0,
        })
    });
    passes[0].rect_fill(clip, PaintColor::Borrowed(&theme.cell_bg));
    let mut cells = Vec::with_capacity(cell_count);
    for pane in &geometry.panes {
        for pass in &passes {
            pass.push_clip(pane.clip);
        }
        for row in &pane.rows {
            for col in &pane.columns {
                let cell = CellPlacement {
                    coordinate: CellCoord {
                        row: row.index,
                        col: col.index,
                    },
                    bounds: cell_rect(row, col),
                };
                attempt.paint(cell, &passes, false)?;
                cells.push(cell);
            }
        }
        for pass in &passes {
            pass.pop_clip();
        }
    }
    let mut scene = GridScene {
        clip,
        passes: passes.into_iter().map(SceneBuilder::finish).collect(),
    };
    // Merges cover the completed ordinary-cell passes. Each pane uses the
    // same logical merge rectangle, translated by that pane's axis samples.
    for merge in &merges {
        let range = merge_range(*merge);
        for fragment in geometry.fragments(range) {
            let pane = &geometry.panes[fragment.pane];
            let row = pane
                .rows
                .iter()
                .find(|r| r.index == fragment.range.first.row)
                .expect("fragment row belongs to pane");
            let col = pane
                .columns
                .iter()
                .find(|c| c.index == fragment.range.first.col)
                .expect("fragment column belongs to pane");
            let logical = dimensions
                .logical_rect(*merge, row, col)
                .map_err(|e| attempt.error(range.first, CellReadOperation::Merges, e))?;
            let merged_passes = new_passes();
            for pass in &merged_passes {
                pass.push_clip(pane.clip);
            }
            attempt.paint(
                CellPlacement {
                    coordinate: range.first,
                    bounds: logical,
                },
                &merged_passes,
                true,
            )?;
            let show_merge_grid = attempt.show_grid
                && workbook
                    .resolve_color(
                        &attempt.cached[&(range.first.row, range.first.col)]
                            .0
                            .style
                            .fill
                            .color,
                    )
                    .is_empty();
            // IronCalc stores the border on each perimeter cell. Reuse the
            // records fetched by the ordinary pass, including partial edges.
            for row in &pane.rows {
                if !(range.first.row..=range.last.row).contains(&row.index) {
                    continue;
                }
                for col in &pane.columns {
                    if !(range.first.col..=range.last.col).contains(&col.index) {
                        continue;
                    }
                    let coord = CellCoord {
                        row: row.index,
                        col: col.index,
                    };
                    let record = &attempt.cached[&(coord.row, coord.col)];
                    let b = &record.0.style.border;
                    let border = Border {
                        left: (coord.col == range.first.col)
                            .then(|| b.left.clone())
                            .flatten(),
                        right: (coord.col == range.last.col)
                            .then(|| b.right.clone())
                            .flatten(),
                        top: (coord.row == range.first.row)
                            .then(|| b.top.clone())
                            .flatten(),
                        bottom: (coord.row == range.last.row)
                            .then(|| b.bottom.clone())
                            .flatten(),
                        ..Default::default()
                    };
                    let rect = cell_rect(row, col);
                    borders::compile_borders(
                        &merged_passes[3],
                        &merged_passes[2],
                        workbook,
                        &border,
                        rect,
                        false,
                        theme,
                    );
                    if show_merge_grid {
                        for (edge, side, item) in [
                            (coord.col == range.first.col, Side::Left, &border.left),
                            (coord.col == range.last.col, Side::Right, &border.right),
                            (coord.row == range.first.row, Side::Top, &border.top),
                            (coord.row == range.last.row, Side::Bottom, &border.bottom),
                        ] {
                            if edge && item.is_none() {
                                merged_passes[2].stroke_line(
                                    side.line(rect),
                                    PaintColor::Borrowed(&theme.grid_color),
                                    1.0,
                                );
                            }
                        }
                    }
                }
            }
            for pass in &merged_passes {
                pass.pop_clip();
            }
            scene
                .passes
                .extend(merged_passes.into_iter().map(SceneBuilder::finish));
        }
    }
    let chrome = SceneBuilder::default();
    for (vertical, bands) in [(false, &geometry.row_bands), (true, &geometry.column_bands)] {
        if bands.len() > 1 {
            let split = bands[1].start;
            let rect = if vertical {
                PixelRect {
                    top_left: Point {
                        x: split,
                        y: header_height,
                    },
                    width: 0,
                    height: viewport.height - header_height,
                }
            } else {
                PixelRect {
                    top_left: Point {
                        x: header_width,
                        y: split,
                    },
                    width: viewport.width - header_width,
                    height: 0,
                }
            };
            chrome.stroke_line(
                if vertical { Side::Left } else { Side::Top }.line(rect),
                PaintColor::Borrowed(&theme.grid_separator_color),
                2.0,
            );
        }
    }
    headers(&chrome, &geometry, theme);
    scene.passes.push(chrome.finish());
    // Keep query payloads engine-native. All reads stop before staging begins.
    Ok(PreparedWorksheet {
        scene,
        cells: cells.into(),
        geometry: Some(geometry),
        merges: merges.into(),
        links: attempt.links.into(),
        extended_style_reads: attempt.reads,
    })
}
fn cell_rect(row: &AxisCell, col: &AxisCell) -> PixelRect {
    PixelRect {
        top_left: Point {
            x: col.start,
            y: row.start,
        },
        width: col.extent,
        height: row.extent,
    }
}
pub(super) fn merge_range(merge: MergedCell) -> GridRange {
    GridRange {
        first: CellCoord {
            row: merge.row,
            col: merge.column,
        },
        last: CellCoord {
            row: merge.last_row(),
            col: merge.last_column(),
        },
    }
}
fn headers(out: &SceneBuilder, geometry: &GridGeometry, theme: &CanvasTheme) {
    let origin = geometry.origin;
    let font = PaintColor::Static("12px sans-serif");
    out.rect_fill(
        PixelRect {
            top_left: Point { x: 0, y: 0 },
            width: geometry.bounds.width,
            height: origin.y,
        },
        PaintColor::Borrowed(&theme.header_bg),
    );
    out.rect_fill(
        PixelRect {
            top_left: Point { x: 0, y: 0 },
            width: origin.x,
            height: geometry.bounds.height,
        },
        PaintColor::Borrowed(&theme.header_bg),
    );
    for (vertical, bands) in [(true, &geometry.row_bands), (false, &geometry.column_bands)] {
        for band in bands {
            let clip = if vertical {
                PixelRect {
                    top_left: Point {
                        x: 0,
                        y: band.start,
                    },
                    width: origin.x,
                    height: band.extent,
                }
            } else {
                PixelRect {
                    top_left: Point {
                        x: band.start,
                        y: 0,
                    },
                    width: band.extent,
                    height: origin.y,
                }
            };
            out.push_clip(clip);
            for cell in &band.cells {
                let rect = if vertical {
                    PixelRect {
                        top_left: Point {
                            x: 0,
                            y: cell.start,
                        },
                        width: origin.x,
                        height: cell.extent,
                    }
                } else {
                    PixelRect {
                        top_left: Point {
                            x: cell.start,
                            y: 0,
                        },
                        width: cell.extent,
                        height: origin.y,
                    }
                };
                let label = if vertical {
                    cell.index.to_string()
                } else {
                    number_to_column(cell.index).unwrap_or_default()
                };
                out.push_clip(rect);
                out.fill_text(
                    &label,
                    f64::from(rect.center().x),
                    f64::from(rect.center().y),
                    font,
                    PaintColor::Borrowed(&theme.header_text_color),
                    TextAlign::Center,
                    TextBaseline::Middle,
                );
                out.pop_clip();
                out.stroke_line(
                    if vertical { Side::Bottom } else { Side::Right }.line(rect),
                    PaintColor::Borrowed(&theme.header_border_color),
                    1.0,
                );
            }
            out.pop_clip();
        }
    }
    let top = PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: geometry.bounds.width,
        height: origin.y,
    };
    let left = PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: origin.x,
        height: geometry.bounds.height,
    };
    out.stroke_line(
        Side::Bottom.line(top),
        PaintColor::Borrowed(&theme.header_border_color),
        1.0,
    );
    out.stroke_line(
        Side::Right.line(left),
        PaintColor::Borrowed(&theme.header_border_color),
        1.0,
    );
}
struct Dimensions<'w, 'm> {
    workbook: &'w UserModel<'m>,
    sheet: u32,
    rows: HashMap<i32, i32>,
    cols: HashMap<i32, i32>,
}
impl Dimensions<'_, '_> {
    fn extent(&mut self, row: bool, index: i32) -> Result<f64, String> {
        let cache = if row { &mut self.rows } else { &mut self.cols };
        if let Some(v) = cache.get(&index) {
            return Ok(f64::from(*v));
        }
        let value = if row {
            self.workbook.get_row_height(self.sheet, index)?
        } else {
            self.workbook.get_column_width(self.sheet, index)?
        };
        if !value.is_finite() || value < 0.0 || value.round() > f64::from(i32::MAX) {
            return Err("invalid axis extent".into());
        }
        let value = value.round() as i32;
        cache.insert(index, value);
        Ok(f64::from(value))
    }
    fn sum(&mut self, row: bool, first: i32, end: i32) -> Result<i32, String> {
        let mut total = 0_i32;
        for i in first..end {
            total = total
                .checked_add(self.extent(row, i)? as i32)
                .ok_or("merge extent overflow")?;
        }
        Ok(total)
    }
    fn logical_rect(
        &mut self,
        merge: MergedCell,
        row: &AxisCell,
        col: &AxisCell,
    ) -> Result<PixelRect, String> {
        let x = col
            .start
            .checked_sub(self.sum(false, merge.column, col.index)?)
            .ok_or("merge position overflow")?;
        let y = row
            .start
            .checked_sub(self.sum(true, merge.row, row.index)?)
            .ok_or("merge position overflow")?;
        let rect = PixelRect {
            top_left: Point { x, y },
            width: self.sum(false, merge.column, merge.column + merge.width)?,
            height: self.sum(true, merge.row, merge.row + merge.height)?,
        };
        if !valid_rect(rect) {
            return Err("merge rectangle overflow".into());
        }
        Ok(rect)
    }
}

fn axis_error(
    sheet: u32,
    mut cell: CellCoord,
    row: bool,
    error: AxisError<String>,
) -> WorksheetCompileError {
    let index = match &error {
        AxisError::Bounds => None,
        AxisError::Extent { index } | AxisError::Read { index, .. } => Some(*index),
    };
    if let Some(index) = index {
        if row {
            cell.row = index;
        } else {
            cell.col = index;
        }
    }
    WorksheetCompileError {
        sheet,
        cell,
        operation: if row {
            CellReadOperation::RowHeight
        } else {
            CellReadOperation::ColumnWidth
        },
        message: format!("{error:?}"),
    }
}
