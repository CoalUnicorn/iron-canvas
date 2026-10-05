//! Generic pane geometry. No worksheet identity or content is stored here.
use super::{AxisCell, AxisError, CellCoord, sample_axis};
use crate::{PixelRect, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRange {
    pub first: CellCoord,
    pub last: CellCoord,
}
impl GridRange {
    pub fn contains(self, cell: CellCoord) -> bool {
        (self.first.row..=self.last.row).contains(&cell.row)
            && (self.first.col..=self.last.col).contains(&cell.col)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisBand {
    pub first_index: i32,
    pub last_index: i32,
    pub start: i32,
    pub extent: i32,
    pub cells: Vec<AxisCell>,
}

/// Sample the frozen band first, then the scrolling band. Both are bounded by
/// the viewport. A frozen band can consume the entire drawable extent.
pub fn sample_bands<E>(
    first: i32,
    last: i32,
    frozen: i32,
    origin: i32,
    pixels: i32,
    mut extent: impl FnMut(i32) -> Result<f64, E>,
) -> Result<Vec<AxisBand>, AxisError<E>> {
    if first < 1
        || first > last
        || frozen < 0
        || frozen > last
        || origin < 0
        || pixels < 0
        || origin.checked_add(pixels).is_none()
    {
        return Err(AxisError::Bounds);
    }
    let mut bands = Vec::new();
    let mut used = 0;
    if frozen > 0 && pixels > 0 {
        let mut cells = sample_axis(1, frozen, pixels, &mut extent)?;
        used = cells.last().map_or(0, |c| c.start + c.extent).min(pixels);
        for cell in &mut cells {
            cell.start = cell
                .start
                .checked_add(origin)
                .ok_or(AxisError::Extent { index: cell.index })?;
            cell.start
                .checked_add(cell.extent)
                .ok_or(AxisError::Extent { index: cell.index })?;
        }
        bands.push(AxisBand {
            first_index: 1,
            last_index: if used < pixels {
                frozen
            } else {
                cells.last().map_or(frozen, |c| c.index)
            },
            start: origin,
            extent: used,
            cells,
        });
    }
    if used < pixels && frozen < last {
        let start = origin + used;
        let mut cells = sample_axis(first.max(frozen + 1), last, pixels - used, extent)?;
        for cell in &mut cells {
            cell.start = cell
                .start
                .checked_add(start)
                .ok_or(AxisError::Extent { index: cell.index })?;
            cell.start
                .checked_add(cell.extent)
                .ok_or(AxisError::Extent { index: cell.index })?;
        }
        bands.push(AxisBand {
            first_index: first.max(frozen + 1),
            last_index: cells.last().map_or(last, |c| c.index),
            start,
            extent: pixels - used,
            cells,
        });
    }
    Ok(bands)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridPane {
    pub range: GridRange,
    pub clip: PixelRect,
    pub rows: Vec<AxisCell>,
    pub columns: Vec<AxisCell>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridGeometry {
    pub bounds: PixelRect,
    pub origin: Point,
    pub row_bands: Vec<AxisBand>,
    pub column_bands: Vec<AxisBand>,
    pub panes: Vec<GridPane>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridHit {
    Cell(CellCoord),
    RowHeader(i32),
    ColumnHeader(i32),
    Corner,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeFragment {
    pub pane: usize,
    pub range: GridRange,
    pub rect: PixelRect,
    /// True range edges: left, top, right, bottom. Clipped edges are false.
    pub edges: [bool; 4],
}
impl GridGeometry {
    pub fn new(
        bounds: PixelRect,
        origin: Point,
        rows: Vec<AxisBand>,
        columns: Vec<AxisBand>,
    ) -> Self {
        let panes = rows
            .iter()
            .flat_map(|r| {
                columns.iter().map(move |c| GridPane {
                    range: GridRange {
                        first: CellCoord {
                            row: r.first_index,
                            col: c.first_index,
                        },
                        last: CellCoord {
                            row: r.last_index,
                            col: c.last_index,
                        },
                    },
                    clip: PixelRect {
                        top_left: Point {
                            x: c.start,
                            y: r.start,
                        },
                        width: c.extent,
                        height: r.extent,
                    },
                    rows: r.cells.clone(),
                    columns: c.cells.clone(),
                })
            })
            .filter(|p| p.clip.width > 0 && p.clip.height > 0)
            .collect();
        Self {
            bounds,
            origin,
            row_bands: rows,
            column_bands: columns,
            panes,
        }
    }
    pub fn hit_test(&self, point: Point) -> Option<GridHit> {
        if !contains(self.bounds, point) {
            return None;
        }
        let row = slot_at(&self.row_bands, point.y);
        let col = slot_at(&self.column_bands, point.x);
        match (point.x < self.origin.x, point.y < self.origin.y) {
            (true, true) => Some(GridHit::Corner),
            (true, false) => row.map(GridHit::RowHeader),
            (false, true) => col.map(GridHit::ColumnHeader),
            (false, false) => Some(GridHit::Cell(CellCoord {
                row: row?,
                col: col?,
            })),
        }
    }
    pub fn fragments(&self, range: GridRange) -> Vec<RangeFragment> {
        self.panes
            .iter()
            .enumerate()
            .filter_map(|(pane, p)| {
                let mut rows = p
                    .rows
                    .iter()
                    .filter(|r| (range.first.row..=range.last.row).contains(&r.index));
                let mut cols = p
                    .columns
                    .iter()
                    .filter(|c| (range.first.col..=range.last.col).contains(&c.index));
                let r1 = rows.next()?;
                let r2 = rows.next_back().unwrap_or(r1);
                let c1 = cols.next()?;
                let c2 = cols.next_back().unwrap_or(c1);
                let rect = PixelRect {
                    top_left: Point {
                        x: c1.start,
                        y: r1.start,
                    },
                    width: c2.start + c2.extent - c1.start,
                    height: r2.start + r2.extent - r1.start,
                }
                .intersection(p.clip)?;
                Some(RangeFragment {
                    pane,
                    range: GridRange {
                        first: CellCoord {
                            row: r1.index,
                            col: c1.index,
                        },
                        last: CellCoord {
                            row: r2.index,
                            col: c2.index,
                        },
                    },
                    rect,
                    edges: [
                        range.first.col >= p.range.first.col && rect.left() == c1.start,
                        range.first.row >= p.range.first.row && rect.top() == r1.start,
                        range.last.col <= p.range.last.col && rect.right() == c2.start + c2.extent,
                        range.last.row <= p.range.last.row && rect.bottom() == r2.start + r2.extent,
                    ],
                })
            })
            .collect()
    }
}
fn slot_at(bands: &[AxisBand], pixel: i32) -> Option<i32> {
    let band = bands
        .iter()
        .find(|b| pixel >= b.start && pixel < b.start + b.extent)?;
    band.cells
        .iter()
        .find(|c| pixel >= c.start && pixel < c.start + c.extent)
        .map(|c| c.index)
}
pub fn contains(rect: PixelRect, point: Point) -> bool {
    point.x >= rect.left()
        && point.x < rect.right()
        && point.y >= rect.top()
        && point.y < rect.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_band_consumes_viewport_without_reading_scroll_window() {
        let mut reads = Vec::new();
        let bands = sample_bands(90, 100, 5, 24, 25, |i| {
            reads.push(i);
            Ok::<_, ()>(20.0)
        })
        .unwrap();
        assert_eq!(reads, [1, 2]);
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].extent, 25);
        assert_eq!(bands[0].cells[1].extent, 20);
    }
    #[test]
    fn hidden_frozen_axes_leave_room_for_scroll_and_bounds_are_checked() {
        let bands = sample_bands(1, 10, 2, 24, 50, |i| {
            Ok::<_, ()>(if i <= 2 { 0.0 } else { 20.0 })
        })
        .unwrap();
        assert_eq!(bands[0].extent, 0);
        assert_eq!(bands[1].cells[0].index, 3);
        assert!(sample_bands(1, 10, 11, 0, 10, |_| Ok::<_, ()>(20.0)).is_err());
        assert!(sample_bands(1, 10, 0, i32::MAX, 10, |_| Ok::<_, ()>(20.0)).is_err());
    }
    #[test]
    fn fragments_do_not_invent_edges_at_freeze_or_viewport_clips() {
        let rows = sample_bands(5, 100, 1, 24, 90, |_| Ok::<_, ()>(20.0)).unwrap();
        let columns = sample_bands(5, 100, 1, 48, 90, |_| Ok::<_, ()>(20.0)).unwrap();
        let geometry = GridGeometry::new(
            PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 138,
                height: 114,
            },
            Point { x: 48, y: 24 },
            rows,
            columns,
        );
        let fragments = geometry.fragments(GridRange {
            first: CellCoord { row: 1, col: 1 },
            last: CellCoord { row: 9, col: 9 },
        });
        assert_eq!(fragments.len(), 4);
        assert_eq!(fragments[0].edges, [true, true, false, false]);
        assert_eq!(fragments[3].edges, [false, false, false, false]);
        assert_eq!(
            geometry.hit_test(Point { x: 137, y: 113 }),
            Some(GridHit::Cell(CellCoord { row: 8, col: 8 }))
        );
        assert_eq!(geometry.hit_test(Point { x: 138, y: 113 }), None);
    }
}
