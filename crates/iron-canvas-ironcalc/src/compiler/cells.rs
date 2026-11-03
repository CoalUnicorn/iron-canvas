//! Plan dense engine reads from visible worksheet samples.
use super::{CellView, LAST_COLUMN, LAST_ROW};
use iron_canvas_core::CellCoord;
use iron_canvas_core::scene_geometry::{AxisCell, GridPane};
use ironcalc_base::{CellsBlock, types::CellType};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BlockRect {
    pub row: i32,
    pub column: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BlockLimits {
    pub width: usize,
    pub height: usize,
    pub cells_per_block: usize,
    pub cells_per_attempt: usize,
    pub blocks_per_attempt: usize,
}

impl Default for BlockLimits {
    fn default() -> Self {
        Self {
            width: 64,
            height: 64,
            cells_per_block: 4_096,
            cells_per_attempt: 131_072,
            blocks_per_attempt: 256,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BlockPlanError {
    Bounds,
    Arithmetic,
}

impl BlockRect {
    fn checked(row: i32, column: i32, width: usize, height: usize) -> Result<Self, BlockPlanError> {
        let width = i32::try_from(width).map_err(|_| BlockPlanError::Arithmetic)?;
        let height = i32::try_from(height).map_err(|_| BlockPlanError::Arithmetic)?;
        let last_row = row
            .checked_add(height - 1)
            .ok_or(BlockPlanError::Arithmetic)?;
        let last_column = column
            .checked_add(width - 1)
            .ok_or(BlockPlanError::Arithmetic)?;
        if width < 1
            || height < 1
            || row < 1
            || column < 1
            || last_row > LAST_ROW
            || last_column > LAST_COLUMN
        {
            return Err(BlockPlanError::Bounds);
        }
        // The engine uses i32 for the returned cell count and usize for the
        // temporary border ring. Check both before a request can reach it.
        let count = width
            .checked_mul(height)
            .ok_or(BlockPlanError::Arithmetic)?;
        usize::try_from(count).map_err(|_| BlockPlanError::Arithmetic)?;
        let ring_width = width.checked_add(2).ok_or(BlockPlanError::Arithmetic)?;
        let ring_height = height.checked_add(2).ok_or(BlockPlanError::Arithmetic)?;
        usize::try_from(ring_width)
            .ok()
            .and_then(|w| {
                usize::try_from(ring_height)
                    .ok()
                    .and_then(|h| w.checked_mul(h))
            })
            .ok_or(BlockPlanError::Arithmetic)?;
        Ok(Self {
            row,
            column,
            width,
            height,
        })
    }
}

fn valid_axis(cells: &[AxisCell], last: i32) -> bool {
    cells
        .iter()
        .all(|cell| (1..=last).contains(&cell.index) && cell.extent > 0)
        && cells.windows(2).all(|pair| pair[0].index < pair[1].index)
}

/// Return reusable tiles for the visible cells. Dense panes use absolute
/// 64-cell tiles so the same rectangles return across scroll frames. Sparse or
/// fragmented panes keep the exact visible tiling to avoid wide reads.
pub(super) fn plan_blocks(
    panes: &[GridPane],
    limits: BlockLimits,
) -> Result<Vec<BlockRect>, BlockPlanError> {
    for pane in panes {
        if !valid_axis(&pane.rows, LAST_ROW) || !valid_axis(&pane.columns, LAST_COLUMN) {
            return Err(BlockPlanError::Bounds);
        }
    }
    let tile_width = limits.width.min(limits.cells_per_block);
    if tile_width == 0 || limits.height == 0 || limits.blocks_per_attempt == 0 {
        return Ok(Vec::new());
    }
    let tile_height = limits.height.min(limits.cells_per_block / tile_width);
    let exact = plan_exact(panes, limits, tile_width, tile_height)?;
    let Some(absolute) = plan_absolute(panes, tile_width, tile_height)? else {
        return Ok(exact);
    };
    let visible_cells: usize = panes
        .iter()
        .map(|pane| pane.rows.len() * pane.columns.len())
        .sum();
    let visible_rows: usize = panes.iter().map(|pane| pane.rows.len()).sum();
    let visible_columns: usize = panes.iter().map(|pane| pane.columns.len()).sum();
    let absolute_cells: usize = absolute
        .iter()
        .map(|rect| (rect.width as usize) * (rect.height as usize))
        .sum();
    // Wide absolute tiles are a cache trade. Bound the two alignment strips
    // outside the visible cells. Small and sparse panes keep the exact plan.
    let budget = visible_cells.saturating_add(
        2usize.saturating_mul(
            tile_width
                .saturating_mul(visible_rows)
                .saturating_add(tile_height.saturating_mul(visible_columns)),
        ),
    );
    if absolute_cells > budget {
        return Ok(exact);
    }
    Ok(limit_blocks(absolute, limits))
}

/// Absolute tiles that contain at least one visible row and column.
fn plan_absolute(
    panes: &[GridPane],
    tile_width: usize,
    tile_height: usize,
) -> Result<Option<Vec<BlockRect>>, BlockPlanError> {
    let mut row_tiles = BTreeSet::new();
    let mut column_tiles = BTreeSet::new();
    for pane in panes {
        for row in pane.rows.iter() {
            row_tiles.insert((row.index - 1) / tile_height as i32);
        }
        for column in pane.columns.iter() {
            column_tiles.insert((column.index - 1) / tile_width as i32);
        }
    }
    if row_tiles.is_empty() || column_tiles.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut blocks = Vec::with_capacity(row_tiles.len() * column_tiles.len());
    for row_tile in row_tiles {
        let row = row_tile * tile_height as i32 + 1;
        let height = (tile_height as i32).min(LAST_ROW - row + 1);
        for column_tile in &column_tiles {
            let column = column_tile * tile_width as i32 + 1;
            let width = (tile_width as i32).min(LAST_COLUMN - column + 1);
            blocks.push(BlockRect::checked(
                row,
                column,
                width as usize,
                height as usize,
            )?);
        }
    }
    Ok(Some(blocks))
}

/// Apply the per-attempt cell and block limits to an ordered plan.
fn limit_blocks(blocks: Vec<BlockRect>, limits: BlockLimits) -> Vec<BlockRect> {
    let mut out = Vec::new();
    let mut remaining = limits.cells_per_attempt;
    for rect in blocks {
        let count = (rect.width as usize) * (rect.height as usize);
        if out.len() == limits.blocks_per_attempt || count > remaining {
            break;
        }
        remaining -= count;
        out.push(rect);
    }
    out
}

/// Split each consecutive run into tiles that start at the run origin.
fn plan_exact(
    panes: &[GridPane],
    limits: BlockLimits,
    tile_width: usize,
    tile_height: usize,
) -> Result<Vec<BlockRect>, BlockPlanError> {
    let mut blocks = Vec::new();
    let mut remaining = limits.cells_per_attempt;
    for pane in panes {
        let mut row_start = 0;
        while row_start < pane.rows.len() {
            let row_end = run_end(&pane.rows, row_start);
            let row_run = &pane.rows[row_start..row_end];
            let mut row_chunk = 0;
            while row_chunk < row_run.len() {
                let row_chunk_end = (row_chunk + tile_height).min(row_run.len());
                let rows = &row_run[row_chunk..row_chunk_end];
                let mut column_start = 0;
                while column_start < pane.columns.len() {
                    let column_end = run_end(&pane.columns, column_start);
                    let column_run = &pane.columns[column_start..column_end];
                    let mut column_chunk = 0;
                    while column_chunk < column_run.len() {
                        let column_chunk_end = (column_chunk + tile_width).min(column_run.len());
                        let columns = &column_run[column_chunk..column_chunk_end];
                        let count = rows
                            .len()
                            .checked_mul(columns.len())
                            .ok_or(BlockPlanError::Arithmetic)?;
                        if blocks.len() == limits.blocks_per_attempt || count > remaining {
                            return Ok(blocks);
                        }
                        blocks.push(BlockRect::checked(
                            rows[0].index,
                            columns[0].index,
                            columns.len(),
                            rows.len(),
                        )?);
                        remaining -= count;
                        if remaining == 0 || blocks.len() == limits.blocks_per_attempt {
                            return Ok(blocks);
                        }
                        column_chunk = column_chunk_end;
                    }
                    column_start = column_end;
                }
                row_chunk = row_chunk_end;
            }
            row_start = row_end;
        }
    }
    Ok(blocks)
}

/// Return the end of the consecutive run that starts at `start`.
fn run_end(cells: &[AxisCell], start: usize) -> usize {
    let mut end = start + 1;
    while end < cells.len() && cells[end - 1].index.checked_add(1) == Some(cells[end].index) {
        end += 1;
    }
    end
}

// Validate public engine vectors before any indexed cell access. Keep indices,
// rather than references, so blocks can move while the attempt owns them.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FillPaint {
    Cells,
    Runs,
}

pub(super) struct ValidatedBlock {
    data: CellsBlock,
    text_slots: Vec<Option<usize>>,
    fill_rows: Vec<std::ops::Range<usize>>,
}

fn cell_type(value: i32) -> Option<CellType> {
    match value {
        1 => Some(CellType::Number),
        2 => Some(CellType::Text),
        4 => Some(CellType::LogicalValue),
        16 => Some(CellType::ErrorValue),
        64 => Some(CellType::Array),
        128 => Some(CellType::CompoundData),
        _ => None,
    }
}

impl ValidatedBlock {
    pub(super) fn new(rect: BlockRect, data: CellsBlock) -> Result<Self, &'static str> {
        BlockRect::checked(
            rect.row,
            rect.column,
            usize::try_from(rect.width).map_err(|_| "invalid width")?,
            usize::try_from(rect.height).map_err(|_| "invalid height")?,
        )
        .map_err(|_| "invalid bounds")?;
        if (data.row, data.column, data.width, data.height)
            != (rect.row, rect.column, rect.width, rect.height)
        {
            return Err("returned bounds differ from request");
        }
        let count = (rect.width as usize)
            .checked_mul(rect.height as usize)
            .ok_or("invalid cell count")?;
        if data.style_indices.len() != count
            || data
                .style_indices
                .iter()
                .any(|&index| index as usize >= data.styles.len())
        {
            return Err("invalid style indices");
        }
        let mut text_slots = vec![None; count];
        let mut previous = None;
        for (index, text) in data.texts.iter().enumerate() {
            if text.row < rect.row
                || text.row >= rect.row + rect.height
                || text.column < rect.column
                || text.column >= rect.column + rect.width
                || text.text.is_empty()
                || cell_type(text.cell_type).is_none()
            {
                return Err("invalid text entry");
            }
            let slot = ((text.row - rect.row) * rect.width + (text.column - rect.column)) as usize;
            if previous.is_some_and(|previous| previous >= slot) {
                return Err("text entries are not unique and ordered");
            }
            text_slots[slot] = Some(index);
            previous = Some(slot);
        }
        // Require complete row coverage before ordinary fills can be skipped.
        // Empty colors still cover cells, but add no paint over the sheet background.
        let mut fill_rows = Vec::with_capacity(rect.height as usize);
        let mut index = 0;
        for row in rect.row..rect.row + rect.height {
            let first = index;
            let mut column = rect.column;
            while column < rect.column + rect.width {
                let run = data.fills.get(index).ok_or("missing fill run")?;
                let end = run
                    .column
                    .checked_add(run.width)
                    .ok_or("invalid fill width")?;
                if run.row != row
                    || run.column != column
                    || run.width < 1
                    || end > rect.column + rect.width
                {
                    return Err("invalid fill coverage");
                }
                column = end;
                index += 1;
            }
            fill_rows.push(first..index);
        }
        if index != data.fills.len() {
            return Err("extra fill runs");
        }
        Ok(Self {
            data,
            text_slots,
            fill_rows,
        })
    }
    pub(super) fn cell_count(&self) -> usize {
        self.data.style_indices.len()
    }
    pub(super) fn style_count(&self) -> usize {
        self.data.styles.len()
    }
}

pub(super) struct BlockRecord {
    pub block: usize,
    pub slot: usize,
    pub kind: CellType,
    pub occupied: bool,
}

struct RowInterval {
    first: i32,
    last: i32,
    block: usize,
    row_slot: usize,
}

#[derive(Default)]
pub(super) struct CellBlocks {
    blocks: Vec<std::rc::Rc<ValidatedBlock>>,
    rows: BTreeMap<i32, Vec<RowInterval>>,
}
impl CellBlocks {
    pub(super) fn insert(&mut self, block: std::rc::Rc<ValidatedBlock>) {
        let index = self.blocks.len();
        let data = &block.data;
        for row in data.row..data.row + data.height {
            let intervals = self.rows.entry(row).or_default();
            let position = intervals.partition_point(|interval| interval.first < data.column);
            intervals.insert(
                position,
                RowInterval {
                    first: data.column,
                    last: data.column + data.width - 1,
                    block: index,
                    row_slot: ((row - data.row) * data.width) as usize,
                },
            );
        }
        self.blocks.push(block);
    }
    pub(super) fn locate(&self, cell: CellCoord) -> Option<(usize, usize)> {
        let intervals = self.rows.get(&cell.row)?;
        let position = intervals
            .partition_point(|interval| interval.first <= cell.col)
            .checked_sub(1)?;
        let interval = &intervals[position];
        (cell.col <= interval.last).then_some((
            interval.block,
            interval.row_slot + (cell.col - interval.first) as usize,
        ))
    }
    pub(super) fn paint_row_fills(
        &self,
        row: &AxisCell,
        pane: &GridPane,
        out: &mut iron_canvas_core::scene::SceneBuilder,
    ) {
        use iron_canvas_core::{PixelRect, Point, painter::PaintColor};
        let Some(intervals) = self.rows.get(&row.index) else {
            return;
        };
        for interval in intervals {
            let block = &self.blocks[interval.block];
            let row_slot = (row.index - block.data.row) as usize;
            for run in &block.data.fills[block.fill_rows[row_slot].clone()] {
                if run.color.is_empty() {
                    continue;
                }
                // Use sampled column positions. Fixed-width multiplication loses
                // custom sizes and can cross hidden or frozen coordinate gaps.
                let first = pane.columns.partition_point(|col| col.index < run.column);
                let end = pane
                    .columns
                    .partition_point(|col| col.index < run.column + run.width);
                if first == end {
                    continue;
                }
                let left = &pane.columns[first];
                let right = &pane.columns[end - 1];
                let rect = PixelRect {
                    top_left: Point {
                        x: left.start,
                        y: row.start,
                    },
                    width: right.start + right.extent - left.start,
                    height: row.extent,
                };
                // Retain full cell bounds and the pane clip. Trimming the last
                // rectangle changes edge antialiasing at fractional scales.
                let mut edges = Vec::with_capacity(end - first + 1);
                edges.push(left.start);
                edges.extend(
                    pane.columns[first..end]
                        .iter()
                        .map(|col| col.start + col.extent),
                );
                out.rect_fill_run(rect, &edges, PaintColor::Borrowed(&run.color));
            }
        }
    }

    pub(super) fn has_text(&self, block: usize, slot: usize) -> bool {
        self.blocks[block].text_slots[slot].is_some()
    }
    pub(super) fn text_kind(&self, block: usize, slot: usize) -> Option<CellType> {
        let block = &self.blocks[block];
        block.text_slots[slot].and_then(|index| cell_type(block.data.texts[index].cell_type))
    }
    pub(super) fn view<'a>(&'a self, record: &'a BlockRecord) -> CellView<'a> {
        self.view_slot(record.block, record.slot, &record.kind, record.occupied)
    }
    /// Borrow one block cell without a coordinate lookup or a `BlockRecord`.
    pub(super) fn view_slot<'a>(
        &'a self,
        block: usize,
        slot: usize,
        kind: &'a CellType,
        occupied: bool,
    ) -> CellView<'a> {
        let block = &self.blocks[block];
        CellView {
            extended: &block.data.styles[block.data.style_indices[slot] as usize],
            kind,
            occupied,
            value: block.text_slots[slot].map_or("", |index| block.data.texts[index].text.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::{CellCoord, PixelRect, Point, scene_geometry::GridRange};

    fn pane(rows: &[i32], columns: &[i32]) -> GridPane {
        let samples = |indices: &[i32]| {
            indices
                .iter()
                .enumerate()
                .map(|(slot, &index)| AxisCell {
                    index,
                    start: slot as i32 * 10,
                    extent: 10,
                })
                .collect::<Vec<_>>()
                .into()
        };
        GridPane {
            // Deliberately wider than the samples. Only the samples define reads.
            range: GridRange {
                first: CellCoord { row: 1, col: 1 },
                last: CellCoord {
                    row: LAST_ROW,
                    col: LAST_COLUMN,
                },
            },
            clip: PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 15,
                height: 15,
            },
            rows: samples(rows),
            columns: samples(columns),
        }
    }

    #[test]
    fn malformed_blocks_fail_before_indexed_access() {
        let mut model =
            ironcalc_base::UserModel::new_empty("validation", "en", "UTC", "en").unwrap();
        model.set_user_input(0, 1, 1, "first").unwrap();
        model.set_user_input(0, 2, 2, "last").unwrap();
        let original = model.get_cells_block(0, 1, 1, 2, 2).unwrap();
        let rect = BlockRect {
            row: 1,
            column: 1,
            width: 2,
            height: 2,
        };
        let mutations: [fn(&mut CellsBlock); 14] = [
            |b| b.width = 3,
            |b| {
                b.style_indices.pop();
            },
            |b| b.style_indices[0] = u32::MAX,
            |b| b.texts[0].row = 0,
            |b| b.texts[0].cell_type = 3,
            |b| b.texts.swap(0, 1),
            |b| b.texts.push(b.texts[0].clone()),
            |b| b.texts[0].text.clear(),
            |b| {
                b.fills.pop();
            },
            |b| b.fills[0].width = 0,
            |b| b.fills[0].width = i32::MAX,
            |b| b.fills[0].column += 1,
            |b| b.fills[0].row += 1,
            |b| b.fills.push(b.fills[0].clone()),
        ];
        for mutate in mutations {
            let mut block = original.clone();
            mutate(&mut block);
            assert!(ValidatedBlock::new(rect, block).is_err());
        }
        assert!(ValidatedBlock::new(rect, original).is_ok());
    }

    #[test]
    fn hidden_gaps_and_frozen_panes_stay_separate() {
        let panes = [
            pane(&[1, 2], &[1]),
            pane(&[1, 2], &[10_000, 10_001]),
            pane(&[900_000, 900_002], &[1, 3, 4]),
        ];
        let blocks = plan_blocks(&panes, BlockLimits::default()).unwrap();
        let actual: Vec<_> = blocks
            .iter()
            .map(|b| (b.row, b.column, b.width, b.height))
            .collect();
        assert_eq!(
            actual,
            [
                (1, 1, 1, 2),
                (1, 10_000, 2, 2),
                (900_000, 1, 1, 1),
                (900_000, 3, 2, 1),
                (900_002, 1, 1, 1),
                (900_002, 3, 2, 1)
            ]
        );
    }

    #[test]
    fn absolute_tiles_cover_visible_cells_and_stay_bounded() {
        let rows: Vec<_> = (1..=130).collect();
        let columns: Vec<_> = (1..=129).collect();
        let blocks = plan_blocks(&[pane(&rows, &columns)], BlockLimits::default()).unwrap();
        assert_eq!(blocks.len(), 9);
        let mut covered = std::collections::HashSet::new();
        for block in blocks {
            assert!(block.width <= 64 && block.height <= 64);
            assert!(block.width * block.height <= 4096);
            assert!(block.row + block.height - 1 <= LAST_ROW);
            assert!(block.column + block.width - 1 <= LAST_COLUMN);
            for row in block.row..block.row + block.height {
                for column in block.column..block.column + block.width {
                    if (1..=130).contains(&row) && (1..=129).contains(&column) {
                        assert!(covered.insert((row, column)));
                    }
                }
            }
        }
        assert_eq!(covered.len(), rows.len() * columns.len());
        assert!(covered.contains(&(130, 129)));
    }

    #[test]
    fn attempt_limits_leave_remaining_cells_for_individual_reads() {
        let rows: Vec<_> = (1..=512).collect();
        let columns: Vec<_> = (1..=512).collect();
        let panes = [pane(&rows, &columns)];
        let blocks = plan_blocks(&panes, BlockLimits::default()).unwrap();
        assert_eq!(blocks.len(), 32);
        assert_eq!(
            blocks.iter().map(|b| b.width * b.height).sum::<i32>(),
            131_072
        );
        let limits = BlockLimits {
            blocks_per_attempt: 2,
            ..BlockLimits::default()
        };
        assert_eq!(plan_blocks(&panes, limits).unwrap().len(), 2);
        let limits = BlockLimits {
            cells_per_attempt: 4095,
            ..BlockLimits::default()
        };
        assert!(plan_blocks(&panes, limits).unwrap().is_empty());
    }

    #[test]
    fn fragmented_axes_stop_at_request_limit() {
        let indices: Vec<_> = (1..=1000).map(|i| i * 2).collect();
        let blocks = plan_blocks(&[pane(&indices, &indices)], BlockLimits::default()).unwrap();
        assert_eq!(blocks.len(), 256);
        assert!(blocks.iter().all(|b| b.width == 1 && b.height == 1));
    }

    #[test]
    fn empty_panes_zero_budgets_and_sheet_edges() {
        assert!(
            plan_blocks(&[pane(&[], &[1]), pane(&[1], &[])], BlockLimits::default())
                .unwrap()
                .is_empty()
        );
        let limits = BlockLimits {
            cells_per_block: 0,
            ..BlockLimits::default()
        };
        assert!(plan_blocks(&[pane(&[1], &[1])], limits).unwrap().is_empty());
        let blocks =
            plan_blocks(&[pane(&[LAST_ROW], &[LAST_COLUMN])], BlockLimits::default()).unwrap();
        assert_eq!(
            blocks,
            [BlockRect {
                row: LAST_ROW,
                column: LAST_COLUMN,
                width: 1,
                height: 1
            }]
        );
    }

    #[test]
    fn sampled_frozen_rows_can_consume_the_viewport() {
        use iron_canvas_core::scene_geometry::{GridGeometry, sample_bands};
        let rows = sample_bands(900_000, LAST_ROW, 3, 0, 15, 0, |_| Ok::<_, ()>(10.0)).unwrap();
        let columns =
            sample_bands(10_000, LAST_COLUMN, 1, 0, 15, 0, |_| Ok::<_, ()>(10.0)).unwrap();
        let geometry = GridGeometry::new(
            PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 15,
                height: 15,
            },
            Point { x: 0, y: 0 },
            rows,
            columns,
        );
        let blocks = plan_blocks(&geometry.panes, BlockLimits::default()).unwrap();
        assert_eq!(
            blocks,
            [
                BlockRect {
                    row: 1,
                    column: 1,
                    width: 1,
                    height: 2
                },
                BlockRect {
                    row: 1,
                    column: 10_000,
                    width: 1,
                    height: 2
                },
            ]
        );
    }

    #[test]
    fn invalid_samples_and_unsafe_engine_arithmetic_fail() {
        for rows in [vec![0], vec![LAST_ROW + 1], vec![2, 1], vec![1, 1]] {
            assert_eq!(
                plan_blocks(&[pane(&rows, &[1])], BlockLimits::default()),
                Err(BlockPlanError::Bounds)
            );
        }
        assert_eq!(
            BlockRect::checked(1, 1, 16_384, 1_048_576),
            Err(BlockPlanError::Arithmetic)
        );
        assert_eq!(
            BlockRect::checked(1, 1, usize::MAX, 1),
            Err(BlockPlanError::Arithmetic)
        );
        assert_eq!(
            BlockRect::checked(LAST_ROW, 1, 1, 2),
            Err(BlockPlanError::Bounds)
        );
    }
}
