//! Generic pane geometry. No worksheet identity or content is stored here.
use super::{AxisCell, AxisError, CellCoord, sample_axis};
use crate::{PixelRect, Point, Side};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRange {
    pub first: CellCoord,
    pub last: CellCoord,
}
impl GridRange {
    /// Return the same inclusive range with its coordinates ordered.
    ///
    /// Pointer drags can end above or left of their anchor, so callers may
    /// provide either corner as `first` without changing the represented area.
    pub fn normalized(self) -> Self {
        Self {
            first: CellCoord {
                row: self.first.row.min(self.last.row),
                col: self.first.col.min(self.last.col),
            },
            last: CellCoord {
                row: self.first.row.max(self.last.row),
                col: self.first.col.max(self.last.col),
            },
        }
    }

    pub fn contains(self, cell: CellCoord) -> bool {
        let range = self.normalized();
        (range.first.row..=range.last.row).contains(&cell.row)
            && (range.first.col..=range.last.col).contains(&cell.col)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisBand {
    pub first_index: i32,
    pub last_index: i32,
    pub start: i32,
    pub extent: i32,
    /// True for the frozen band. The scrolling band is `!frozen`, and its
    /// `start` is the frozen band's end when one exists. Used to locate the
    /// scrollable pane without re-reading freeze counts.
    pub frozen: bool,
    /// Cells this band covers. Every pane on this axis holds this same slice.
    pub cells: Rc<[AxisCell]>,
}

/// Retain hidden indices at the viewport edge without adding another visible cell.
fn sample_band_axis<E>(
    first: i32,
    last: i32,
    pixels: i32,
    extent: &mut impl FnMut(i32) -> Result<f64, E>,
) -> Result<(Vec<AxisCell>, i32), AxisError<E>> {
    let mut last_index = first - 1;
    let cells = sample_axis(first, last, pixels, |index| {
        let value = extent(index)?;
        last_index = index;
        Ok(value)
    })?;
    if cells
        .last()
        .is_some_and(|cell| cell.start + cell.extent == pixels)
    {
        while last_index < last {
            let index = last_index + 1;
            let value = extent(index).map_err(|error| AxisError::Read { index, error })?;
            if !value.is_finite() || value < 0.0 || value.round() > f64::from(i32::MAX) {
                return Err(AxisError::Extent { index });
            }
            if value.round() > 0.0 {
                break;
            }
            last_index = index;
        }
    }
    Ok((cells, last_index))
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
        let (mut cells, last_index) = sample_band_axis(1, frozen, pixels, &mut extent)?;
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
            last_index,
            start: origin,
            extent: used,
            frozen: true,
            cells: cells.into(),
        });
    }
    if used < pixels && frozen < last {
        let start = origin + used;
        let scroll_extent = pixels - used;
        let (mut cells, last_index) =
            sample_band_axis(first.max(frozen + 1), last, scroll_extent, &mut extent)?;
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
            last_index,
            start,
            extent: scroll_extent,
            frozen: false,
            cells: cells.into(),
        });
    }
    Ok(bands)
}

/// One crossing of a row band and a column band. The cell slices are shared
/// with the bands, so a pane costs no copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridPane {
    pub range: GridRange,
    pub clip: PixelRect,
    pub rows: Rc<[AxisCell]>,
    pub columns: Rc<[AxisCell]>,
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
/// A header edge within the resize hot zone, named by the slot it trails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridResize {
    Row(i32),
    Column(i32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeFragment {
    pub pane: usize,
    pub range: GridRange,
    pub rect: PixelRect,
    /// Sides of `rect` that are true range edges, keyed by [`Side::index`].
    /// A clipped edge is false.
    pub edges: [bool; 4],
}

/// Build a [`Side`]-keyed edge array from a per-side predicate.
fn side_flags(mut edge: impl FnMut(Side) -> bool) -> [bool; 4] {
    let mut edges = [false; 4];
    for side in Side::ALL {
        edges[side.index()] = edge(side);
    }
    edges
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
                    rows: Rc::clone(&r.cells),
                    columns: Rc::clone(&c.cells),
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
        let range = range.normalized();
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
                    edges: side_flags(|side| match side {
                        Side::Left => {
                            range.first.col >= p.range.first.col && rect.left() == c1.start
                        }
                        Side::Top => range.first.row >= p.range.first.row && rect.top() == r1.start,
                        Side::Right => {
                            range.last.col <= p.range.last.col
                                && rect.right() == c2.start + c2.extent
                        }
                        Side::Bottom => {
                            range.last.row <= p.range.last.row
                                && rect.bottom() == r2.start + r2.extent
                        }
                    }),
                })
            })
            .collect()
    }

    /// Slot rectangle for one cell in canvas pixels, before pane or viewport
    /// clipping. `None` when the cell is outside every sampled pane — the
    /// address gap between a frozen band and the scrolled-to band, or beyond
    /// the walked extent. A frozen row and a scrolling column resolve through
    /// the pane that owns both.
    pub fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        self.panes.iter().find_map(|pane| {
            let r = pane.rows.iter().find(|r| r.index == row)?;
            let c = pane.columns.iter().find(|c| c.index == col)?;
            Some(PixelRect {
                top_left: Point {
                    x: c.start,
                    y: r.start,
                },
                width: c.extent,
                height: r.extent,
            })
        })
    }

    /// Canvas-space rect of the scrollable pane: everything past the frozen
    /// bands, running to [`bounds`](Self::bounds). An axis whose frozen band
    /// consumes the whole drawable extent collapses to zero rather than
    /// going negative.
    pub fn scroll_pane_rect(&self) -> PixelRect {
        let x = scroll_start(&self.column_bands, self.origin.x);
        let y = scroll_start(&self.row_bands, self.origin.y);
        PixelRect {
            top_left: Point { x, y },
            width: (self.bounds.right() - x).max(0),
            height: (self.bounds.bottom() - y).max(0),
        }
    }

    /// Bottom-right corner of the range's lower-right slot, not clipped to
    /// the viewport. The host must suppress the handle at the model's last
    /// row/column; core does not know engine bounds.
    pub fn autofill_handle(&self, range: GridRange) -> Option<Point> {
        let range = range.normalized();
        let Some(rect) = self.cell_rect(range.last.row, range.last.col) else {
            let fragment = self.fragments(range).into_iter().find(|fragment| {
                fragment.edges[Side::Right.index()] && fragment.edges[Side::Bottom.index()]
            })?;
            return Some(Point {
                x: fragment.rect.right(),
                y: fragment.rect.bottom(),
            });
        };
        Some(Point {
            x: rect.right(),
            y: rect.bottom(),
        })
    }

    /// Header edge within `tolerance` of `point`, named by the slot it trails.
    /// Only the column header strip (`point.y < origin.y`) and row header strip
    /// (`point.x < origin.x`) are considered, mirroring hit-test priority.
    pub fn resize_target(&self, point: Point, tolerance: i32) -> Option<GridResize> {
        if tolerance < 0 || !contains(self.bounds, point) {
            return None;
        }
        if point.y < self.origin.y && point.x >= self.origin.x {
            return boundary(&self.column_bands, point.x, tolerance).map(GridResize::Column);
        }
        if point.x < self.origin.x && point.y >= self.origin.y {
            return boundary(&self.row_bands, point.y, tolerance).map(GridResize::Row);
        }
        None
    }
}
/// First slot whose trailing edge lies within `tolerance` of `pixel`.
/// Frozen then scrolling, stopping once the edges pass the hot zone.
fn boundary(bands: &[AxisBand], pixel: i32, tolerance: i32) -> Option<i32> {
    for cell in bands.iter().flat_map(|band| band.cells.iter()) {
        let end = cell.start + cell.extent;
        if (end - pixel).abs() <= tolerance {
            return Some(cell.index);
        }
        if end > pixel + tolerance {
            break;
        }
    }
    None
}
/// Start of the scrolling band, or the end of a fused frozen band.
fn scroll_start(bands: &[AxisBand], origin: i32) -> i32 {
    match bands.iter().find(|b| !b.frozen) {
        Some(band) => band.start,
        None => bands.last().map_or(origin, |band| band.start + band.extent),
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
        assert_eq!(
            fragments[0].edges,
            side_flags(|side| matches!(side, Side::Left | Side::Top))
        );
        assert_eq!(fragments[3].edges, [false; 4]);
        assert_eq!(
            geometry.hit_test(Point { x: 137, y: 113 }),
            Some(GridHit::Cell(CellCoord { row: 8, col: 8 }))
        );
        assert_eq!(geometry.hit_test(Point { x: 138, y: 113 }), None);
    }

    #[test]
    fn fragments_keep_edges_when_range_ends_in_hidden_row_and_column() {
        let sample_axis = || {
            sample_bands(1, 3, 0, 0, 100, |index| {
                Ok::<_, ()>(if index == 3 { 0.0 } else { 20.0 })
            })
            .unwrap()
        };
        let geometry = GridGeometry::new(
            PixelRect {
                top_left: Point::default(),
                width: 100,
                height: 100,
            },
            Point::default(),
            sample_axis(),
            sample_axis(),
        );

        let fragments = geometry.fragments(GridRange {
            first: CellCoord { row: 1, col: 1 },
            last: CellCoord { row: 3, col: 3 },
        });

        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].rect.width, 40);
        assert_eq!(fragments[0].rect.height, 40);
        assert_eq!(fragments[0].edges, [true; 4]);
    }

    #[test]
    fn hidden_boundary_cells_keep_edges_and_handle_at_exact_viewport_extent() {
        for frozen in [0, 4] {
            let sample = || {
                sample_bands(1, 4, frozen, 0, 40, |index| {
                    Ok::<_, ()>(if index >= 3 { 0.0 } else { 20.0 })
                })
                .unwrap()
            };
            let geometry = GridGeometry::new(
                PixelRect {
                    top_left: Point::default(),
                    width: 40,
                    height: 40,
                },
                Point::default(),
                sample(),
                sample(),
            );
            let range = GridRange {
                first: CellCoord { row: 1, col: 1 },
                last: CellCoord { row: 4, col: 4 },
            };
            assert_eq!(geometry.fragments(range)[0].edges, [true; 4]);
            assert_eq!(
                geometry.autofill_handle(range),
                Some(Point { x: 40, y: 40 })
            );
        }
    }

    #[test]
    fn exact_viewport_boundary_stops_before_the_next_visible_cell() {
        let mut reads = Vec::new();
        let bands = sample_bands(1, 10, 0, 0, 40, |index| {
            reads.push(index);
            Ok::<_, ()>(if (3..=4).contains(&index) { 0.0 } else { 20.0 })
        })
        .unwrap();
        assert_eq!(reads, [1, 2, 3, 4, 5]);
        assert_eq!(bands[0].last_index, 4);
        assert_eq!(bands[0].cells.len(), 2);
    }

    #[test]
    fn autofill_handle_uses_visible_corner_when_range_ends_in_hidden_row_and_column() {
        let sample_axis = || {
            sample_bands(1, 3, 0, 0, 100, |index| {
                Ok::<_, ()>(if index == 3 { 0.0 } else { 20.0 })
            })
            .unwrap()
        };
        let geometry = GridGeometry::new(
            PixelRect {
                top_left: Point::default(),
                width: 100,
                height: 100,
            },
            Point::default(),
            sample_axis(),
            sample_axis(),
        );

        assert_eq!(
            geometry.autofill_handle(GridRange {
                first: CellCoord { row: 1, col: 1 },
                last: CellCoord { row: 3, col: 3 },
            }),
            Some(Point { x: 40, y: 40 })
        );
    }

    #[test]
    fn autofill_handle_is_absent_for_a_fully_hidden_range() {
        let sample_axis = || {
            sample_bands(1, 3, 0, 0, 100, |index| {
                Ok::<_, ()>(if index == 3 { 0.0 } else { 20.0 })
            })
            .unwrap()
        };
        let geometry = GridGeometry::new(
            PixelRect {
                top_left: Point::default(),
                width: 100,
                height: 100,
            },
            Point::default(),
            sample_axis(),
            sample_axis(),
        );

        assert_eq!(
            geometry.autofill_handle(GridRange {
                first: CellCoord { row: 3, col: 3 },
                last: CellCoord { row: 3, col: 3 },
            }),
            None
        );
    }

    #[test]
    fn queries_resolve_frozen_scroll_cells_and_the_scroll_pane() {
        let rows = sample_bands(5, 100, 1, 24, 80, |_| Ok::<_, ()>(20.0)).unwrap();
        let columns = sample_bands(5, 100, 1, 48, 80, |_| Ok::<_, ()>(20.0)).unwrap();
        // Frozen row 1 / col 1 at the origin, then scrolling cells from id 5.
        assert!(rows[0].frozen);
        assert!(!rows[1].frozen);
        let geometry = GridGeometry::new(
            PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 128,
                height: 104,
            },
            Point { x: 48, y: 24 },
            rows,
            columns,
        );
        let rect = |x, y| PixelRect {
            top_left: Point { x, y },
            width: 20,
            height: 20,
        };
        assert_eq!(geometry.cell_rect(1, 1), Some(rect(48, 24)));
        assert_eq!(geometry.cell_rect(1, 5), Some(rect(68, 24)));
        assert_eq!(geometry.cell_rect(5, 1), Some(rect(48, 44)));
        // Beyond the sampled extent and the address gap return `None`.
        assert_eq!(geometry.cell_rect(1, 50), None);
        assert_eq!(geometry.cell_rect(100, 100), None);
        assert_eq!(
            geometry.scroll_pane_rect(),
            PixelRect {
                top_left: Point { x: 68, y: 44 },
                width: 60,
                height: 60,
            }
        );
        assert_eq!(
            geometry.autofill_handle(GridRange {
                first: CellCoord { row: 5, col: 5 },
                last: CellCoord { row: 6, col: 6 },
            }),
            Some(Point { x: 108, y: 84 })
        );
        let forward = GridRange {
            first: CellCoord { row: 5, col: 5 },
            last: CellCoord { row: 6, col: 6 },
        };
        let backward = GridRange {
            first: forward.last,
            last: forward.first,
        };
        assert_eq!(geometry.fragments(backward), geometry.fragments(forward));
        assert_eq!(
            geometry.autofill_handle(backward),
            geometry.autofill_handle(forward)
        );
        assert_eq!(
            geometry.autofill_handle(GridRange {
                first: CellCoord { row: 1, col: 1 },
                last: CellCoord { row: 1, col: 50 },
            }),
            None
        );
        // Header edges resolve only inside their strip and hot zone.
        assert_eq!(
            geometry.resize_target(Point { x: 68, y: 10 }, 2),
            Some(GridResize::Column(1))
        );
        assert_eq!(geometry.resize_target(Point { x: 70, y: 10 }, 1), None);
        assert_eq!(
            geometry.resize_target(Point { x: 10, y: 44 }, 2),
            Some(GridResize::Row(1))
        );
        // The cell area is not a resize zone, and points outside bounds miss.
        assert_eq!(geometry.resize_target(Point { x: 68, y: 44 }, 2), None);
        assert_eq!(geometry.resize_target(Point { x: 200, y: 10 }, 2), None);
    }

    #[test]
    fn frozen_band_that_fills_the_viewport_leaves_no_scroll_pane() {
        let rows = sample_bands(90, 100, 5, 24, 25, |_| Ok::<_, ()>(20.0)).unwrap();
        let columns = sample_bands(90, 100, 5, 48, 25, |_| Ok::<_, ()>(20.0)).unwrap();
        assert!(rows.iter().all(|b| b.frozen));
        let geometry = GridGeometry::new(
            PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 73,
                height: 49,
            },
            Point { x: 48, y: 24 },
            rows,
            columns,
        );
        assert_eq!(
            geometry.scroll_pane_rect(),
            PixelRect {
                top_left: Point { x: 73, y: 49 },
                width: 0,
                height: 0,
            }
        );
    }
}
