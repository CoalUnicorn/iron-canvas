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

/// One requested run of equal rounded extents. Gaps between runs use the
/// axis default extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentRun {
    pub first: i32,
    pub last: i32,
    pub extent: i32,
}

/// One resolved run. `prefix` is the checked total before `first`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisRun {
    pub first: i32,
    pub last: i32,
    pub extent: i32,
    pub prefix: i64,
}

/// Failure to build a [`RunAxis`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAxisError {
    /// `last_index` must be at least 1.
    InvalidLimit,
    /// The default extent must not be negative.
    InvalidDefaultExtent,
    /// A run is out of order, out of range, or has a negative extent.
    InvalidRun { run: usize },
    /// The extent total or an index arithmetic step overflowed.
    Overflow,
    /// The resolved run count reached the construction limit.
    Limit,
}

impl std::fmt::Display for RunAxisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLimit => f.write_str("axis last index must be positive"),
            Self::InvalidDefaultExtent => f.write_str("axis default extent must not be negative"),
            Self::InvalidRun { run } => write!(
                f,
                "axis run {run} is out of order, out of range, or negative"
            ),
            Self::Overflow => f.write_str("axis extent sum overflowed"),
            Self::Limit => f.write_str("axis run construction limit reached"),
        }
    }
}

impl std::error::Error for RunAxisError {}

/// Upper bound for resolved runs. This keeps a hostile or corrupt input from
/// allocating without limit.
const MAX_AXIS_RUNS: usize = 1_048_576;

/// Rounded, contiguous extents for one axis. Every index in
/// `1..=last_index` belongs to exactly one run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunAxis {
    default_extent: i32,
    runs: Vec<AxisRun>,
    total: i64,
    last_index: i32,
}

impl RunAxis {
    /// Build contiguous runs from ordered, non-overlapping runs. Missing
    /// intervals use `default_extent`. Adjacent equal extents combine.
    pub fn from_runs(
        default_extent: i32,
        last_index: i32,
        runs: &[ExtentRun],
    ) -> Result<Self, RunAxisError> {
        if default_extent < 0 {
            return Err(RunAxisError::InvalidDefaultExtent);
        }
        if last_index < 1 {
            return Err(RunAxisError::InvalidLimit);
        }
        let mut resolved = Vec::with_capacity(runs.len().saturating_mul(2).max(1));
        let mut prefix = 0_i64;
        let mut cursor = 1_i32;
        for (run, extent) in runs.iter().enumerate() {
            let valid = extent.first >= cursor
                && extent.last >= extent.first
                && extent.last <= last_index
                && extent.extent >= 0;
            if !valid {
                return Err(RunAxisError::InvalidRun { run });
            }
            if cursor < extent.first {
                push_run(
                    &mut resolved,
                    &mut prefix,
                    cursor,
                    extent.first - 1,
                    default_extent,
                )?;
            }
            push_run(
                &mut resolved,
                &mut prefix,
                extent.first,
                extent.last,
                extent.extent,
            )?;
            cursor = extent.last.checked_add(1).ok_or(RunAxisError::Overflow)?;
        }
        if cursor <= last_index {
            push_run(
                &mut resolved,
                &mut prefix,
                cursor,
                last_index,
                default_extent,
            )?;
        }
        Ok(Self {
            default_extent,
            runs: resolved,
            total: prefix,
            last_index,
        })
    }

    pub fn default_extent(&self) -> i32 {
        self.default_extent
    }

    pub fn total_extent(&self) -> i64 {
        self.total
    }

    pub fn last_index(&self) -> i32 {
        self.last_index
    }

    pub fn runs(&self) -> &[AxisRun] {
        &self.runs
    }

    fn run_at(&self, index: i32) -> Option<&AxisRun> {
        if index < 1 || index > self.last_index {
            return None;
        }
        let position = self
            .runs
            .partition_point(|run| run.first <= index)
            .checked_sub(1)?;
        self.runs.get(position).filter(|run| run.last >= index)
    }
}

fn push_run(
    runs: &mut Vec<AxisRun>,
    prefix: &mut i64,
    first: i32,
    last: i32,
    extent: i32,
) -> Result<(), RunAxisError> {
    let length = i64::from(last)
        .checked_sub(i64::from(first))
        .and_then(|value| value.checked_add(1))
        .ok_or(RunAxisError::Overflow)?;
    let span = length
        .checked_mul(i64::from(extent))
        .ok_or(RunAxisError::Overflow)?;
    let next = prefix.checked_add(span).ok_or(RunAxisError::Overflow)?;
    if let Some(previous) = runs.last_mut()
        && previous.last.checked_add(1) == Some(first)
        && previous.extent == extent
    {
        previous.last = last;
    } else {
        if runs.len() >= MAX_AXIS_RUNS {
            return Err(RunAxisError::Limit);
        }
        runs.push(AxisRun {
            first,
            last,
            extent,
            prefix: *prefix,
        });
    }
    *prefix = next;
    Ok(())
}

impl AxisExtents for RunAxis {
    fn default_extent(&self) -> i32 {
        self.default_extent
    }

    fn extent(&self, index: i32) -> i32 {
        self.run_at(index)
            .map_or(self.default_extent, |run| run.extent)
    }

    fn extent_before(&self, index: i32) -> i64 {
        if index <= 1 {
            return 0;
        }
        let last = (index - 1).min(self.last_index);
        let Some(run) = self.run_at(last) else {
            return self.total;
        };
        let length = i64::from(last - run.first + 1);
        run.prefix + length * i64::from(run.extent)
    }

    fn total_extent(&self) -> i64 {
        self.total
    }

    fn locate(&self, distance: i64, window: ScrollAxisWindow) -> Option<(i32, i32)> {
        if window.is_empty() {
            return None;
        }
        let start = self.extent_before(window.first);
        let end = self.extent_before(window.last.saturating_add(1));
        if end <= start {
            return None;
        }
        let target = start + distance.clamp(0, end - start - 1);
        let position = self
            .runs
            .partition_point(|run| run.prefix <= target)
            .checked_sub(1)?;
        let run = self.runs.get(position)?;
        let extent = i64::from(run.extent);
        if extent <= 0 {
            return None;
        }
        let into = target - run.prefix;
        let offset = i32::try_from(into % extent).ok()?;
        let advance = i32::try_from(into / extent).ok()?;
        Some((run.first.checked_add(advance)?, offset))
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

    #[test]
    fn run_axis_fills_gaps_and_keeps_prefix_sums() {
        let axis = RunAxis::from_runs(
            25,
            6,
            &[
                ExtentRun {
                    first: 2,
                    last: 3,
                    extent: 32,
                },
                ExtentRun {
                    first: 5,
                    last: 5,
                    extent: 0,
                },
            ],
        )
        .unwrap();

        assert_eq!(
            axis.runs(),
            [
                AxisRun {
                    first: 1,
                    last: 1,
                    extent: 25,
                    prefix: 0,
                },
                AxisRun {
                    first: 2,
                    last: 3,
                    extent: 32,
                    prefix: 25,
                },
                AxisRun {
                    first: 4,
                    last: 4,
                    extent: 25,
                    prefix: 89,
                },
                AxisRun {
                    first: 5,
                    last: 5,
                    extent: 0,
                    prefix: 114,
                },
                AxisRun {
                    first: 6,
                    last: 6,
                    extent: 25,
                    prefix: 114,
                },
            ]
        );
        assert_eq!(axis.total_extent(), 139);
        assert_eq!(axis.extent_before(3), 57);
        assert_eq!(axis.extent_before(7), 139);
        assert_eq!(axis.extent(5), 0);
        assert_eq!(axis.extent(6), 25);
    }

    #[test]
    fn run_axis_matches_equivalent_explicit_extents() {
        let explicit = TestAxis(vec![25, 25, 50, 50, 25, 25]);
        let axis = RunAxis::from_runs(
            25,
            6,
            &[ExtentRun {
                first: 3,
                last: 4,
                extent: 50,
            }],
        )
        .unwrap();
        let windows = [
            window(1, 6),
            ScrollAxisWindow::after_frozen(2, 6),
            ScrollAxisWindow::after_frozen(6, 6),
        ];
        for window in windows {
            for first in 1..=6 {
                for offset in [0, 1, 24, 100] {
                    assert_eq!(
                        normalize_axis(first, offset, &explicit, window),
                        normalize_axis(first, offset, &axis, window),
                        "{first} {offset} {window:?}"
                    );
                    assert_eq!(
                        move_axis(first, offset, 37, &explicit, window, 64),
                        move_axis(first, offset, 37, &axis, window, 64),
                        "{first} {offset} {window:?}"
                    );
                }
            }
            assert_eq!(
                scrolling_extent(&explicit, window),
                scrolling_extent(&axis, window)
            );
        }
    }

    #[test]
    fn run_axis_rejects_invalid_runs() {
        let run = |first, last, extent| ExtentRun {
            first,
            last,
            extent,
        };
        assert_eq!(
            RunAxis::from_runs(25, 10, &[run(3, 2, 10)]),
            Err(RunAxisError::InvalidRun { run: 0 })
        );
        assert_eq!(
            RunAxis::from_runs(25, 10, &[run(1, 2, 10), run(2, 3, 10)]),
            Err(RunAxisError::InvalidRun { run: 1 })
        );
        assert_eq!(
            RunAxis::from_runs(25, 10, &[run(1, 11, 10)]),
            Err(RunAxisError::InvalidRun { run: 0 })
        );
        assert_eq!(
            RunAxis::from_runs(25, 10, &[run(1, 2, -1)]),
            Err(RunAxisError::InvalidRun { run: 0 })
        );
        assert_eq!(
            RunAxis::from_runs(-1, 10, &[]),
            Err(RunAxisError::InvalidDefaultExtent)
        );
        assert_eq!(
            RunAxis::from_runs(25, 0, &[]),
            Err(RunAxisError::InvalidLimit)
        );
    }

    #[test]
    fn run_prefix_sums_reject_overflow() {
        let mut runs = Vec::new();
        let mut prefix = i64::MAX - 5;
        assert_eq!(
            push_run(&mut runs, &mut prefix, 1, 2, 10),
            Err(RunAxisError::Overflow)
        );
    }
}
