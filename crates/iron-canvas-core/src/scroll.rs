//! Engine-independent scroll position arithmetic.
//!
//! A scroll position names the first visible cell and the pixel distance
//! removed from the start of that cell. The geometry stays outside this
//! module. Callers supply rounded extents through [`AxisExtents`].

use crate::scene_geometry::CellCoord;

/// A cell origin plus the visible offset inside that cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollPosition {
    pub first_cell: CellCoord,
    /// Pixels removed from the start of the first visible column.
    pub offset_x: i32,
    /// Pixels removed from the start of the first visible row.
    pub offset_y: i32,
}

impl ScrollPosition {
    /// Align the first cell with the pane corner.
    pub const fn aligned(first_cell: CellCoord) -> Self {
        Self {
            first_cell,
            offset_x: 0,
            offset_y: 0,
        }
    }
}

/// Legal scrolling indices on one axis. `first` follows the frozen band.
/// The window is empty when `first > last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollAxisWindow {
    pub first: i32,
    pub last: i32,
}

impl ScrollAxisWindow {
    pub const fn new(first: i32, last: i32) -> Self {
        Self { first, last }
    }

    /// Build the window after `frozen` frozen indices.
    pub const fn after_frozen(frozen: i32, last: i32) -> Self {
        Self {
            first: frozen.saturating_add(1),
            last,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.first > self.last
    }
}

/// A normalized position on one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisScroll {
    pub first: i32,
    pub offset: i32,
}

/// Rounded extents and prefix sums for every index of one axis.
///
/// Indices are one-based and inclusive. `extent` returns zero for a hidden
/// index. `extent_before` returns the sum before `index`, so it can span many
/// cells without a per-index loop. `locate` returns the cell that contains a
/// distance inside `window`. The distance is relative to `window.first`.
pub trait AxisExtents {
    fn default_extent(&self) -> i32;
    fn extent(&self, index: i32) -> i32;
    fn extent_before(&self, index: i32) -> i64;
    fn total_extent(&self) -> i64;
    fn locate(&self, distance: i64, window: ScrollAxisWindow) -> Option<(i32, i32)>;
}

/// Pixels of scrolling content inside `window`.
pub fn scrolling_extent(extents: &impl AxisExtents, window: ScrollAxisWindow) -> i64 {
    if window.is_empty() {
        return 0;
    }
    (extents.total_extent() - extents.extent_before(window.first)).max(0)
}

fn locate(distance: i64, extents: &impl AxisExtents, window: ScrollAxisWindow) -> AxisScroll {
    match extents.locate(distance, window) {
        Some((first, offset)) => AxisScroll { first, offset },
        None => AxisScroll {
            first: window.first,
            offset: 0,
        },
    }
}

/// Clamp a requested position to a legal cell and offset.
///
/// A negative offset moves backward across cell boundaries. An offset at or
/// past the end clamps to the last visible cell. Zero-sized cells are skipped.
pub fn normalize_axis(
    first: i32,
    offset: i32,
    extents: &impl AxisExtents,
    window: ScrollAxisWindow,
) -> AxisScroll {
    if window.is_empty() {
        return AxisScroll {
            first: window.last,
            offset: 0,
        };
    }
    let total = scrolling_extent(extents, window);
    if total <= 0 {
        return AxisScroll {
            first: window.first,
            offset: 0,
        };
    }
    let first = first.clamp(window.first, window.last);
    let distance =
        extents.extent_before(first) - extents.extent_before(window.first) + i64::from(offset);
    locate(distance.clamp(0, total - 1), extents, window)
}

/// Move an axis position by a signed pixel delta.
///
/// The escape distance is limited to `[0, max(0, E - P)]`, where `E` is the
/// scrolling extent and `P` is the pane extent. A zero pane keeps the position
/// unchanged because a frozen band covers the axis.
pub fn move_axis(
    first: i32,
    offset: i32,
    delta: i32,
    extents: &impl AxisExtents,
    window: ScrollAxisWindow,
    pane: i32,
) -> AxisScroll {
    let current = normalize_axis(first, offset, extents, window);
    if window.is_empty() || pane <= 0 {
        return current;
    }
    let total = scrolling_extent(extents, window);
    if total <= 0 {
        return current;
    }
    let max_distance = (total - i64::from(pane)).max(0);
    let distance = (extents.extent_before(current.first) - extents.extent_before(window.first)
        + i64::from(current.offset))
    .saturating_add(i64::from(delta))
    .clamp(0, max_distance);
    locate(distance, extents, window)
}

/// Apply the pane end limit without new input.
pub fn clamp_axis_end(
    first: i32,
    offset: i32,
    extents: &impl AxisExtents,
    window: ScrollAxisWindow,
    pane: i32,
) -> AxisScroll {
    move_axis(first, offset, 0, extents, window, pane)
}

/// Normalize both axes of a scroll position.
pub fn normalize_position(
    position: ScrollPosition,
    rows: &impl AxisExtents,
    row_window: ScrollAxisWindow,
    columns: &impl AxisExtents,
    column_window: ScrollAxisWindow,
) -> ScrollPosition {
    let row = normalize_axis(position.first_cell.row, position.offset_y, rows, row_window);
    let column = normalize_axis(
        position.first_cell.col,
        position.offset_x,
        columns,
        column_window,
    );
    ScrollPosition {
        first_cell: CellCoord {
            row: row.first,
            col: column.first,
        },
        offset_x: column.offset,
        offset_y: row.offset,
    }
}

/// Move both axes. Positive deltas move content left and up.
#[allow(clippy::too_many_arguments)]
pub fn move_position(
    position: ScrollPosition,
    delta_x: i32,
    delta_y: i32,
    rows: &impl AxisExtents,
    row_window: ScrollAxisWindow,
    columns: &impl AxisExtents,
    column_window: ScrollAxisWindow,
    pane_width: i32,
    pane_height: i32,
) -> ScrollPosition {
    let row = move_axis(
        position.first_cell.row,
        position.offset_y,
        delta_y,
        rows,
        row_window,
        pane_height,
    );
    let column = move_axis(
        position.first_cell.col,
        position.offset_x,
        delta_x,
        columns,
        column_window,
        pane_width,
    );
    ScrollPosition {
        first_cell: CellCoord {
            row: row.first,
            col: column.first,
        },
        offset_x: column.offset,
        offset_y: row.offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rounded extents for indices `1..=len`. Zero values represent hidden cells.
    struct TestAxis(Vec<i32>);

    impl AxisExtents for TestAxis {
        fn default_extent(&self) -> i32 {
            0
        }
        fn extent(&self, index: i32) -> i32 {
            usize::try_from(index - 1)
                .ok()
                .and_then(|index| self.0.get(index))
                .copied()
                .unwrap_or(0)
        }
        fn extent_before(&self, index: i32) -> i64 {
            let end = usize::try_from(index - 1).unwrap_or(0).min(self.0.len());
            self.0[..end].iter().map(|&extent| i64::from(extent)).sum()
        }
        fn total_extent(&self) -> i64 {
            self.0.iter().map(|&extent| i64::from(extent)).sum()
        }
        fn locate(&self, distance: i64, window: ScrollAxisWindow) -> Option<(i32, i32)> {
            if window.is_empty() || distance < 0 {
                return None;
            }
            let mut remaining = distance;
            for index in window.first..=window.last {
                let extent = i64::from(self.extent(index));
                if remaining < extent {
                    return Some((index, remaining as i32));
                }
                remaining -= extent;
            }
            None
        }
    }

    fn window(first: i32, last: i32) -> ScrollAxisWindow {
        ScrollAxisWindow::new(first, last)
    }

    #[test]
    fn crossing_a_boundary_forward_and_back_keeps_the_example_position() {
        let axis = TestAxis(vec![20, 30]);
        assert_eq!(
            move_axis(1, 7, 15, &axis, window(1, 2), 10),
            AxisScroll {
                first: 2,
                offset: 2
            }
        );
        assert_eq!(
            move_axis(2, 2, -3, &axis, window(1, 2), 10),
            AxisScroll {
                first: 1,
                offset: 19
            }
        );
    }

    #[test]
    fn zero_sized_cells_are_skipped_in_both_directions() {
        let axis = TestAxis(vec![0, 0, 20, 0, 30]);
        assert_eq!(
            normalize_axis(1, 0, &axis, window(1, 5)),
            AxisScroll {
                first: 3,
                offset: 0
            }
        );
        assert_eq!(
            normalize_axis(5, 0, &axis, window(1, 5)),
            AxisScroll {
                first: 5,
                offset: 0
            }
        );
        assert_eq!(
            move_axis(3, 0, -5, &axis, window(1, 5), 10),
            AxisScroll {
                first: 3,
                offset: 0
            }
        );
        assert_eq!(
            move_axis(3, 0, 25, &axis, window(1, 5), 10),
            AxisScroll {
                first: 5,
                offset: 5
            }
        );
    }

    #[test]
    fn empty_axes_keep_a_legal_first_index_and_zero_offset() {
        let axis = TestAxis(vec![0, 0, 0]);
        assert_eq!(
            normalize_axis(2, 9, &axis, window(1, 3)),
            AxisScroll {
                first: 1,
                offset: 0
            }
        );
        assert_eq!(
            move_axis(2, 9, 100, &axis, window(1, 3), 50),
            AxisScroll {
                first: 1,
                offset: 0
            }
        );
        // Every index is frozen: the window is empty and uses the last legal index.
        assert_eq!(
            normalize_axis(9, 9, &axis, ScrollAxisWindow::after_frozen(3, 3)),
            AxisScroll {
                first: 3,
                offset: 0
            }
        );
        assert_eq!(
            move_axis(1, 0, 100, &axis, ScrollAxisWindow::after_frozen(3, 3), 50),
            AxisScroll {
                first: 3,
                offset: 0
            }
        );
    }

    #[test]
    fn end_limit_keeps_the_last_cell_at_the_pane_edge() {
        let axis = TestAxis(vec![10; 10]);
        assert_eq!(
            move_axis(1, 0, 1_000, &axis, window(1, 10), 25),
            AxisScroll {
                first: 8,
                offset: 5
            }
        );
        assert_eq!(
            clamp_axis_end(10, 0, &axis, window(1, 10), 25),
            AxisScroll {
                first: 8,
                offset: 5
            }
        );
        // A pane larger than the content keeps the first scrolling cell.
        assert_eq!(
            move_axis(1, 0, 1_000, &axis, window(1, 10), 200),
            AxisScroll {
                first: 1,
                offset: 0
            }
        );
        // A covering frozen band preserves the position.
        assert_eq!(
            move_axis(4, 3, 1_000, &axis, window(1, 10), 0),
            AxisScroll {
                first: 4,
                offset: 3
            }
        );
    }

    #[test]
    fn frozen_window_starts_after_the_frozen_band() {
        let axis = TestAxis(vec![10; 10]);
        assert_eq!(
            move_axis(3, 0, -100, &axis, ScrollAxisWindow::after_frozen(2, 10), 25),
            AxisScroll {
                first: 3,
                offset: 0
            }
        );
        assert_eq!(
            clamp_axis_end(10, 0, &axis, ScrollAxisWindow::after_frozen(2, 10), 25),
            AxisScroll {
                first: 8,
                offset: 5
            }
        );
        assert_eq!(
            scrolling_extent(&axis, ScrollAxisWindow::after_frozen(2, 10)),
            80
        );
    }

    #[test]
    fn large_offsets_clamp_without_overflow() {
        let axis = TestAxis(vec![i32::MAX, 10]);
        let result = move_axis(1, i32::MAX - 1, i32::MAX, &axis, window(1, 2), 10);
        assert!(result.first >= 1 && result.first <= 2);
        assert!(result.offset >= 0);
        assert_eq!(
            move_axis(2, 0, i32::MIN, &axis, window(1, 2), 10),
            AxisScroll {
                first: 1,
                offset: 0
            }
        );
    }

    #[test]
    fn position_axes_move_independently() {
        let rows = TestAxis(vec![20, 30, 40]);
        let columns = TestAxis(vec![50, 60, 70]);
        let start = ScrollPosition::aligned(CellCoord { row: 1, col: 1 });
        let moved = move_position(
            start,
            60,
            35,
            &rows,
            window(1, 3),
            &columns,
            window(1, 3),
            120,
            50,
        );
        assert_eq!(moved.first_cell, CellCoord { row: 2, col: 2 });
        assert_eq!(moved.offset_y, 15);
        assert_eq!(moved.offset_x, 10);
        let normalized = normalize_position(moved, &rows, window(1, 3), &columns, window(1, 3));
        assert_eq!(normalized, moved);
    }
}
