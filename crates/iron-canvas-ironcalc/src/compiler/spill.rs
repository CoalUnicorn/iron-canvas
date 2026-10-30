//! Bounded worksheet text spill. Geometry and paint remain separate.

use super::*;
use iron_canvas_core::scene_geometry::AxisCell;
use ironcalc_base::types::{Cell, HorizontalAlignment};

/// Maximum logical column distance for a source or a neighbor search.
const COLUMN_LIMIT: usize = 64;

pub(super) struct SpillSource {
    pub placement: CellPlacement,
    pub clip: PixelRect,
    ink_left: f64,
    ink_right: f64,
}

#[derive(Default)]
pub(super) struct SpillPlan {
    pub sources: Vec<SpillSource>,
}

impl SpillPlan {
    pub fn clip(&self, cell: CellCoord) -> Option<PixelRect> {
        self.sources
            .iter()
            .find(|source| source.placement.coordinate == cell)
            .map(|source| source.clip)
    }

    pub fn hides_grid(&self, x: i32) -> bool {
        self.sources
            .iter()
            .any(|source| source.ink_left < f64::from(x) && f64::from(x) < source.ink_right)
    }
}

fn merged(cell: CellCoord, merges: &[MergedCell]) -> bool {
    merges.iter().any(|merge| {
        (merge.row..=merge.last_row()).contains(&cell.row)
            && (merge.column..=merge.last_column()).contains(&cell.col)
    })
}

fn graphics(record: CellView<'_>) -> bool {
    record.extended.icon.is_some()
        || record.extended.data_bar.is_some()
        || record.extended.rating.is_some()
}

fn eligible(record: CellView<'_>) -> bool {
    matches!(record.kind, CellType::Text)
        && !record.value.is_empty()
        && !graphics(record)
        && record
            .extended
            .style
            .alignment
            .as_ref()
            .is_none_or(|alignment| {
                !alignment.wrap_text
                    && matches!(
                        alignment.horizontal,
                        HorizontalAlignment::General
                            | HorizontalAlignment::Left
                            | HorizontalAlignment::Right
                            | HorizontalAlignment::Center
                    )
            })
}

impl Attempt<'_, '_, '_> {
    pub(super) fn occupied_kind(
        &self,
        cell: CellCoord,
    ) -> Result<Option<CellType>, WorksheetCompileError> {
        let sheet = self
            .context
            .workbook
            .get_model()
            .workbook
            .worksheet(self.context.sheet)
            .map_err(|e| self.error(cell, CellReadOperation::Sheet, e))?;
        Ok(sheet.cell(cell.row, cell.col).and_then(|value| {
            if matches!(value.as_ref(), Cell::EmptyCell { .. }) {
                None
            } else {
                Some(value.get_type())
            }
        }))
    }

    pub(super) fn occupied(&self, cell: CellCoord) -> Result<bool, WorksheetCompileError> {
        Ok(self.occupied_kind(cell)?.is_some())
    }

    fn blocker(&self, cell: CellCoord, merges: &[MergedCell]) -> bool {
        let record = self.cached.view(cell);
        // Fills and borders paint before text. They do not occupy an empty
        // cell. Resolved borders can also come from the row above or below;
        // those shared edges cannot determine horizontal text spill.
        merged(cell, merges) || record.occupied || graphics(record)
    }

    /// Find the nearest occupied cell outside one pane edge. Only an eligible
    /// text source can make the intervening styles relevant to this frame.
    #[allow(clippy::too_many_arguments)]
    fn outside_source(
        &mut self,
        row: &AxisCell,
        edge: &AxisCell,
        direction: i32,
        limits: (i32, i32),
        merges: &[MergedCell],
        extent: &mut impl FnMut(i32) -> Result<i32, String>,
    ) -> Result<Vec<AxisCell>, WorksheetCompileError> {
        if self.blocker(
            CellCoord {
                row: row.index,
                col: edge.index,
            },
            merges,
        ) {
            return Ok(Vec::new());
        }
        let mut cells = Vec::new();
        let mut previous = *edge;
        for _ in 0..COLUMN_LIMIT {
            let index = previous.index + direction;
            if !(limits.0..=limits.1).contains(&index) {
                break;
            }
            let coordinate = CellCoord {
                row: row.index,
                col: index,
            };
            if merged(coordinate, merges) {
                break;
            }
            let width = extent(index)
                .map_err(|e| self.error(coordinate, CellReadOperation::ColumnWidth, e))?;
            if width <= 0 {
                break;
            }
            let start = if direction < 0 {
                previous.start.checked_sub(width)
            } else {
                previous.start.checked_add(previous.extent)
            };
            let Some(start) = start.filter(|start| start.checked_add(width).is_some()) else {
                break;
            };
            let column = AxisCell {
                index,
                start,
                extent: width,
            };
            cells.push(column);
            match self.occupied_kind(coordinate)? {
                None => previous = column,
                Some(CellType::Text) => {
                    self.read(coordinate)?;
                    if eligible(self.cached.view(coordinate)) {
                        if direction < 0 {
                            cells.reverse();
                        }
                        return Ok(cells);
                    }
                    break;
                }
                Some(_) => break,
            }
        }
        Ok(Vec::new())
    }

    /// Resolve each row before paint. Hidden-index gaps are hard boundaries.
    /// Full cell records are shared with ordinary paint and merge composition.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_spill(
        &mut self,
        row: &AxisCell,
        visible: &[AxisCell],
        pane_clip: PixelRect,
        limits: (i32, i32),
        merges: &[MergedCell],
        mut extent: impl FnMut(i32) -> Result<i32, String>,
    ) -> Result<SpillPlan, WorksheetCompileError> {
        let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
            return Ok(SpillPlan::default());
        };
        for column in visible {
            self.read(CellCoord {
                row: row.index,
                col: column.index,
            })?;
        }
        // Search only the two pane edges. Empty regions outside the pane need
        // no style reads when there is no text source within the fixed limit.
        let mut columns = self.outside_source(row, first, -1, limits, merges, &mut extent)?;
        columns.extend_from_slice(visible);
        columns.extend(self.outside_source(row, last, 1, limits, merges, &mut extent)?);
        for column in &columns {
            self.read(CellCoord {
                row: row.index,
                col: column.index,
            })?;
        }

        let mut plan = SpillPlan::default();
        for (index, column) in columns.iter().enumerate() {
            let coordinate = CellCoord {
                row: row.index,
                col: column.index,
            };
            let record = self.cached.view(coordinate);
            if !eligible(record) || merged(coordinate, merges) || row.extent < 10 {
                continue;
            }
            let rect = PixelRect {
                top_left: iron_canvas_core::Point {
                    x: column.start,
                    y: row.start,
                },
                width: column.extent,
                height: row.extent,
            };
            let layout = text::layout(
                self.shaper,
                &record.extended.style,
                record.kind,
                record.value,
                rect,
                &mut self.lines,
            );
            if !layout.overflows {
                continue;
            }
            let ink_left = self
                .lines
                .iter()
                .map(|line| line.center_x - line.width / 2.0)
                .fold(f64::INFINITY, f64::min);
            let ink_right = self
                .lines
                .iter()
                .map(|line| line.center_x + line.width / 2.0)
                .fold(f64::NEG_INFINITY, f64::max);
            if !ink_left.is_finite() || !ink_right.is_finite() {
                continue;
            }
            let mut left = index;
            let mut right = index;
            while left > 0
                && index - left < COLUMN_LIMIT
                && ink_left < f64::from(columns[left].start)
            {
                let next = &columns[left - 1];
                if next.index + 1 != columns[left].index
                    || self.blocker(
                        CellCoord {
                            row: row.index,
                            col: next.index,
                        },
                        merges,
                    )
                {
                    break;
                }
                left -= 1;
            }
            while right + 1 < columns.len()
                && right - index < COLUMN_LIMIT
                && ink_right > f64::from(columns[right].start + columns[right].extent)
            {
                let next = &columns[right + 1];
                if next.index != columns[right].index + 1
                    || self.blocker(
                        CellCoord {
                            row: row.index,
                            col: next.index,
                        },
                        merges,
                    )
                {
                    break;
                }
                right += 1;
            }
            if left == index && right == index {
                continue;
            }
            let x1 = columns[left].start.max(pane_clip.left());
            let x2 = (columns[right].start + columns[right].extent).min(pane_clip.right());
            if x1 >= x2 || ink_right <= f64::from(x1) || ink_left >= f64::from(x2) {
                continue;
            }
            plan.sources.push(SpillSource {
                placement: CellPlacement {
                    coordinate,
                    bounds: rect,
                },
                clip: PixelRect {
                    top_left: iron_canvas_core::Point {
                        x: x1,
                        y: rect.top(),
                    },
                    width: x2 - x1,
                    height: rect.height,
                },
                ink_left: ink_left.max(f64::from(x1)),
                ink_right: ink_right.min(f64::from(x2)),
            });
        }
        // Occupied cells separate sources, so only adjacent spill intervals can
        // meet. Disable both competing spills before emitting any paint.
        let mut conflicts = vec![false; plan.sources.len()];
        for (index, pair) in plan.sources.windows(2).enumerate() {
            if pair[0].ink_right > pair[1].ink_left {
                conflicts[index] = true;
                conflicts[index + 1] = true;
            }
        }
        let mut index = 0;
        plan.sources.retain(|_| {
            let keep = !conflicts[index];
            index += 1;
            keep
        });
        Ok(plan)
    }
}
