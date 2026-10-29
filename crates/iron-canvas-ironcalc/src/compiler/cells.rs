//! Plan dense engine reads from visible worksheet samples.
use super::{LAST_COLUMN, LAST_ROW};
use iron_canvas_core::scene_geometry::{AxisCell, GridPane};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            cells_per_attempt: 65_536,
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

/// Return a bounded prefix of visible tiles. Uncovered cells retain individual reads.
pub(super) fn plan_blocks(
    panes: &[GridPane],
    limits: BlockLimits,
) -> Result<Vec<BlockRect>, BlockPlanError> {
    let mut blocks = Vec::new();
    let mut remaining = limits.cells_per_attempt;
    let tile_width = limits.width.min(limits.cells_per_block);
    if tile_width == 0 || limits.height == 0 || remaining == 0 || limits.blocks_per_attempt == 0 {
        return Ok(blocks);
    }
    let tile_height = limits.height.min(limits.cells_per_block / tile_width);
    for pane in panes {
        if !valid_axis(&pane.rows, LAST_ROW) || !valid_axis(&pane.columns, LAST_COLUMN) {
            return Err(BlockPlanError::Bounds);
        }
        // Keep coordinate gaps out of dense requests. Iterate borrowed runs so
        // fragmented axes do not allocate a list of every run combination.
        for rows in pane
            .rows
            .chunk_by(|a, b| a.index.checked_add(1) == Some(b.index))
        {
            for rows in rows.chunks(tile_height) {
                for columns in pane
                    .columns
                    .chunk_by(|a, b| a.index.checked_add(1) == Some(b.index))
                {
                    for columns in columns.chunks(tile_width) {
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
                    }
                }
            }
        }
    }
    Ok(blocks)
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
    fn tiles_cover_visible_cells_once_and_include_clipped_last_cells() {
        let rows: Vec<_> = (1..=130).collect();
        let columns: Vec<_> = (1..=129).collect();
        let blocks = plan_blocks(&[pane(&rows, &columns)], BlockLimits::default()).unwrap();
        assert_eq!(blocks.len(), 9);
        let mut covered = std::collections::HashSet::new();
        for b in blocks {
            assert!(b.width <= 64 && b.height <= 64 && b.width * b.height <= 4096);
            for row in b.row..b.row + b.height {
                for col in b.column..b.column + b.width {
                    assert!(covered.insert((row, col)));
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
        assert_eq!(blocks.len(), 16);
        assert_eq!(
            blocks.iter().map(|b| b.width * b.height).sum::<i32>(),
            65_536
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
        let rows = sample_bands(900_000, LAST_ROW, 3, 0, 15, |_| Ok::<_, ()>(10.0)).unwrap();
        let columns = sample_bands(10_000, LAST_COLUMN, 1, 0, 15, |_| Ok::<_, ()>(10.0)).unwrap();
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
