use super::*;
use iron_canvas_core::{
    Axis, Point, Side,
    painter::{TextAlign, TextBaseline},
    scene::Primitive,
    scene_geometry::{AxisCell, AxisError, GridGeometry, GridPane, GridRange, sample_bands},
};
use ironcalc_base::{expressions::utils::number_to_column, types::Border};

/// Height of the column-header strip, in logical pixels.
const HEADER_STRIP_HEIGHT: i32 = 24;

/// Font for header labels, in CSS `font` shorthand form.
const HEADER_FONT_CSS: &str = "12px sans-serif";

/// Padding added to the measured widest visible row label to get the
/// row-header strip width.
const HEADER_LABEL_PADDING: i32 = 16;

/// Floor for the row-header strip width.
const MIN_HEADER_WIDTH: i32 = 48;

pub(super) fn prepare(
    context: WorksheetContext<'_, '_>,
    viewport: WorksheetViewport,
    shaper: &mut TextShaper<'_>,
    reuse: WorksheetReuse<'_>,
) -> Result<PreparedWorksheet, WorksheetCompileError> {
    let WorksheetContext {
        workbook,
        sheet,
        theme,
    } = context;
    let WorksheetReuse {
        geometry: previous,
        metadata,
    } = reuse;
    let mut attempt = Attempt::new(context, shaper, metadata.map(|data| data.links))?;
    let (frozen, merges) = read_structure(&attempt, viewport, metadata)?;
    let mut dimensions = Dimensions {
        workbook,
        sheet,
        rows: HashMap::new(),
        cols: HashMap::new(),
    };
    let geometry = prepare_geometry(&mut attempt, viewport, frozen, &mut dimensions, previous)?;

    // Draw ordinary cells first. Merges cover their contents and reuse their records.
    let mut scene = paint_cells(&mut attempt, &geometry, frozen, &merges, &mut dimensions)?;
    paint_merges(
        &mut attempt,
        &geometry,
        &merges,
        &mut dimensions,
        &mut scene,
    )?;
    scene
        .passes
        .push(paint_headers_and_separators(&geometry, theme));

    // Keep query data in engine types. All reads stop before staging starts.
    Ok(PreparedWorksheet {
        scene,
        explicit_cells: None,
        geometry: Some(geometry),
        frozen,
        merges,
        links: attempt.links,
        extended_style_reads: attempt.reads,
    })
}

fn read_structure(
    attempt: &Attempt<'_, '_, '_>,
    viewport: WorksheetViewport,
    metadata: Option<WorksheetMetadata<'_>>,
) -> Result<(FrozenAxes, Rc<[MergedCell]>), WorksheetCompileError> {
    let workbook = attempt.context.workbook;
    let sheet = attempt.context.sheet;
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
    let merges: Rc<[MergedCell]> = match metadata {
        Some(data) => Rc::clone(data.merges),
        None => workbook
            .get_merged_cells(sheet)
            .map_err(|e| err(CellReadOperation::Merges, e))?
            .into(),
    };
    for merge in merges.iter() {
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
    Ok((
        FrozenAxes {
            rows: frozen_rows,
            cols: frozen_cols,
        },
        merges,
    ))
}

fn prepare_geometry(
    attempt: &mut Attempt<'_, '_, '_>,
    viewport: WorksheetViewport,
    frozen: FrozenAxes,
    dimensions: &mut Dimensions<'_, '_>,
    previous: Option<&Rc<GridGeometry>>,
) -> Result<Rc<GridGeometry>, WorksheetCompileError> {
    let first = viewport.first_cell;
    // Sample rows first. Their labels set the width of the row headers.
    let header_height = HEADER_STRIP_HEIGHT.min(viewport.height);
    let rows = sample_bands(
        first.row,
        LAST_ROW,
        frozen.rows,
        header_height,
        viewport.height - header_height,
        |i| dimensions.extent(Axis::Row, i),
    )
    .map_err(|e| axis_error(attempt.context.sheet, first, Axis::Row, e))?;
    let label = rows
        .iter()
        .flat_map(|b| b.cells.iter())
        .map(|c| c.index)
        .max()
        .unwrap_or(first.row)
        .to_string();
    let header_width = (attempt
        .shaper
        .metrics()
        .measure_text_width(&label, HEADER_FONT_CSS)
        .ceil() as i32)
        .saturating_add(HEADER_LABEL_PADDING)
        .max(MIN_HEADER_WIDTH)
        .min(viewport.width);
    let columns = sample_bands(
        first.col,
        LAST_COLUMN,
        frozen.cols,
        header_width,
        viewport.width - header_width,
        |i| dimensions.extent(Axis::Column, i),
    )
    .map_err(|e| axis_error(attempt.context.sheet, first, Axis::Column, e))?;
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
    Ok(geometry)
}

fn paint_cells(
    attempt: &mut Attempt<'_, '_, '_>,
    geometry: &GridGeometry,
    frozen: FrozenAxes,
    merges: &[MergedCell],
    dimensions: &mut Dimensions<'_, '_>,
) -> Result<GridScene, WorksheetCompileError> {
    let cell_count: usize = geometry
        .panes
        .iter()
        .map(|p| p.rows.len() * p.columns.len())
        .sum();
    let clips = geometry.panes.len() * 2;
    let mut passes = std::array::from_fn(|index| {
        SceneBuilder::with_capacity(match index {
            2 => cell_count * 2 + clips,
            4 => cell_count * 3 + clips,
            _ => 0,
        })
    });
    passes[0].rect_fill(
        geometry.bounds,
        PaintColor::Borrowed(&attempt.context.theme.cell_bg),
    );
    for pane in &geometry.panes {
        for pass in &mut passes {
            pass.push_clip(pane.clip);
        }
        for row in pane.rows.iter() {
            let limits = if pane
                .columns
                .first()
                .is_some_and(|column| column.index <= frozen.cols)
            {
                (1, frozen.cols)
            } else {
                (frozen.cols + 1, LAST_COLUMN)
            };
            let spill =
                attempt.plan_spill(row, &pane.columns, pane.clip, limits, merges, |index| {
                    dimensions
                        .extent(Axis::Column, index)
                        .map(|value| value as i32)
                })?;
            for col in pane.columns.iter() {
                let cell = CellPlacement {
                    coordinate: CellCoord {
                        row: row.index,
                        col: col.index,
                    },
                    bounds: cell_rect(row, col),
                };
                attempt.paint(
                    cell,
                    &mut passes,
                    false,
                    spill.clip(cell.coordinate),
                    spill.hides_grid(col.start),
                )?;
            }
            for source in &spill.sources {
                if pane
                    .columns
                    .binary_search_by_key(&source.placement.coordinate.col, |column| column.index)
                    .is_err()
                {
                    let cf = cf::CfDecorationPaint::resolve(
                        &attempt.cached[&source.placement.coordinate].extended,
                        attempt.context.workbook,
                    );
                    attempt.paint_text(source.placement, &mut passes[4], Some(source.clip), &cf);
                }
            }
        }
        for pass in &mut passes {
            pass.pop_clip();
        }
    }
    Ok(GridScene {
        clip: geometry.bounds,
        passes: passes.into_iter().map(SceneBuilder::finish).collect(),
    })
}

fn paint_merges(
    attempt: &mut Attempt<'_, '_, '_>,
    geometry: &GridGeometry,
    merges: &[MergedCell],
    dimensions: &mut Dimensions<'_, '_>,
    scene: &mut GridScene,
) -> Result<(), WorksheetCompileError> {
    // Merges cover the completed ordinary-cell passes. Each pane uses the
    // same logical merge rectangle, translated by that pane's axis samples.
    for merge in merges.iter() {
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
            let mut merged_passes = new_passes();
            for pass in &mut merged_passes {
                pass.push_clip(pane.clip);
            }
            attempt.paint(
                CellPlacement {
                    coordinate: range.first,
                    bounds: logical,
                },
                &mut merged_passes,
                true,
                None,
                false,
            )?;
            let show_merge_grid = attempt.show_grid
                && attempt
                    .context
                    .workbook
                    .resolve_color(&attempt.cached[&range.first].extended.style.fill.color)
                    .is_empty();
            paint_merge_borders(attempt, pane, range, show_merge_grid, &mut merged_passes);
            for pass in &mut merged_passes {
                pass.pop_clip();
            }
            scene
                .passes
                .extend(merged_passes.into_iter().map(SceneBuilder::finish));
        }
    }
    Ok(())
}

fn paint_merge_borders(
    attempt: &Attempt<'_, '_, '_>,
    pane: &GridPane,
    range: GridRange,
    show_merge_grid: bool,
    passes: &mut [SceneBuilder; 5],
) {
    // IronCalc stores the border on each perimeter cell. Reuse the
    // records fetched by the ordinary pass, including partial edges.
    for row in pane.rows.iter() {
        if !(range.first.row..=range.last.row).contains(&row.index) {
            continue;
        }
        for col in pane.columns.iter() {
            if !(range.first.col..=range.last.col).contains(&col.index) {
                continue;
            }
            let coord = CellCoord {
                row: row.index,
                col: col.index,
            };
            let record = &attempt.cached[&coord];
            let b = &record.extended.style.border;
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
            let (grid_side, border_side) = passes.split_at_mut(3);
            borders::compile_borders(
                &mut border_side[0],
                &mut grid_side[2],
                attempt.context,
                &border,
                rect,
                false,
                false,
            );
            if show_merge_grid {
                for (edge, side, item) in [
                    (coord.col == range.first.col, Side::Left, &border.left),
                    (coord.col == range.last.col, Side::Right, &border.right),
                    (coord.row == range.first.row, Side::Top, &border.top),
                    (coord.row == range.last.row, Side::Bottom, &border.bottom),
                ] {
                    if edge && item.is_none() {
                        passes[2].stroke_line(
                            side.line(rect),
                            PaintColor::Borrowed(&attempt.context.theme.grid_color),
                            1.0,
                        );
                    }
                }
            }
        }
    }
}

fn paint_headers_and_separators(geometry: &GridGeometry, theme: &CanvasTheme) -> Rc<[Primitive]> {
    let mut chrome = SceneBuilder::default();
    for (axis, bands) in [
        (Axis::Row, &geometry.row_bands),
        (Axis::Column, &geometry.column_bands),
    ] {
        if bands.len() > 1 {
            let split = bands[1].start;
            let rect = match axis {
                Axis::Row => PixelRect {
                    top_left: Point {
                        x: geometry.origin.x,
                        y: split,
                    },
                    width: geometry.bounds.width - geometry.origin.x,
                    height: 0,
                },
                Axis::Column => PixelRect {
                    top_left: Point {
                        x: split,
                        y: geometry.origin.y,
                    },
                    width: 0,
                    height: geometry.bounds.height - geometry.origin.y,
                },
            };
            chrome.stroke_line(
                match axis {
                    Axis::Row => Side::Top,
                    Axis::Column => Side::Left,
                }
                .line(rect),
                PaintColor::Borrowed(&theme.grid_separator_color),
                2.0,
            );
        }
    }
    headers(&mut chrome, geometry, theme);
    chrome.finish()
}

pub(super) fn cell_rect(row: &AxisCell, col: &AxisCell) -> PixelRect {
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
fn headers(out: &mut SceneBuilder, geometry: &GridGeometry, theme: &CanvasTheme) {
    let origin = geometry.origin;
    let font = PaintColor::Static(HEADER_FONT_CSS);
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
    for (axis, bands) in [
        (Axis::Row, &geometry.row_bands),
        (Axis::Column, &geometry.column_bands),
    ] {
        for band in bands {
            let clip = match axis {
                Axis::Row => PixelRect {
                    top_left: Point {
                        x: 0,
                        y: band.start,
                    },
                    width: origin.x,
                    height: band.extent,
                },
                Axis::Column => PixelRect {
                    top_left: Point {
                        x: band.start,
                        y: 0,
                    },
                    width: band.extent,
                    height: origin.y,
                },
            };
            out.push_clip(clip);
            for cell in band.cells.iter() {
                let rect = match axis {
                    Axis::Row => PixelRect {
                        top_left: Point {
                            x: 0,
                            y: cell.start,
                        },
                        width: origin.x,
                        height: cell.extent,
                    },
                    Axis::Column => PixelRect {
                        top_left: Point {
                            x: cell.start,
                            y: 0,
                        },
                        width: cell.extent,
                        height: origin.y,
                    },
                };
                let label = match axis {
                    Axis::Row => cell.index.to_string(),
                    Axis::Column => number_to_column(cell.index).unwrap_or_default(),
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
                    match axis {
                        Axis::Row => Side::Bottom,
                        Axis::Column => Side::Right,
                    }
                    .line(rect),
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
    fn extent(&mut self, axis: Axis, index: i32) -> Result<f64, String> {
        let cache = match axis {
            Axis::Row => &mut self.rows,
            Axis::Column => &mut self.cols,
        };
        if let Some(v) = cache.get(&index) {
            return Ok(f64::from(*v));
        }
        let value = match axis {
            Axis::Row => self.workbook.get_row_height(self.sheet, index)?,
            Axis::Column => self.workbook.get_column_width(self.sheet, index)?,
        };
        if !value.is_finite() || value < 0.0 || value.round() > f64::from(i32::MAX) {
            return Err("invalid axis extent".into());
        }
        let value = value.round() as i32;
        cache.insert(index, value);
        Ok(f64::from(value))
    }
    fn sum(&mut self, axis: Axis, first: i32, end: i32) -> Result<i32, String> {
        let mut total = 0_i32;
        for i in first..end {
            total = total
                .checked_add(self.extent(axis, i)? as i32)
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
            .checked_sub(self.sum(Axis::Column, merge.column, col.index)?)
            .ok_or("merge position overflow")?;
        let y = row
            .start
            .checked_sub(self.sum(Axis::Row, merge.row, row.index)?)
            .ok_or("merge position overflow")?;
        let rect = PixelRect {
            top_left: Point { x, y },
            width: self.sum(Axis::Column, merge.column, merge.column + merge.width)?,
            height: self.sum(Axis::Row, merge.row, merge.row + merge.height)?,
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
    axis: Axis,
    error: AxisError<String>,
) -> WorksheetCompileError {
    let index = match &error {
        AxisError::Bounds => None,
        AxisError::Extent { index } | AxisError::Read { index, .. } => Some(*index),
    };
    if let Some(index) = index {
        match axis {
            Axis::Row => cell.row = index,
            Axis::Column => cell.col = index,
        }
    }
    WorksheetCompileError {
        sheet,
        cell,
        operation: match axis {
            Axis::Row => CellReadOperation::RowHeight,
            Axis::Column => CellReadOperation::ColumnWidth,
        },
        message: format!("{error:?}"),
    }
}
