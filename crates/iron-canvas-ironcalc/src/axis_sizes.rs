//! Bulk row and column size reads over IronCalc worksheet records.
//!
//! The engine returns one size at a time. This module resolves stored records
//! into ordered runs and keeps the same rules as the individual getters.
//! Rules checked against IronCalc revision `b063c523`.
//!
//! The resumable builder counts record reads, interval lookups, moved cover
//! entries, sort copies, and emitted intervals. Sorting and vector moves stop
//! at the step budget. The constructor reserves buffers from the record count
//! so a step does not copy a growing allocation. Interactive hosts can advance
//! the builder between frames; synchronous callers run it to completion.

pub use iron_canvas_core::Axis;
pub use iron_canvas_core::scroll::AxisRun;
use iron_canvas_core::{
    ScrollAxisWindow,
    scroll::{ExtentRun, RunAxis, RunAxisError, scrolling_extent},
};
use ironcalc_base::{UserModel, types::Worksheet};

// The pinned engine keeps its dimension limits and constants private.
// Record these values here and compare them with the public getters after
// each engine upgrade.
const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;
const DEFAULT_ROW_HEIGHT: f64 = 25.0;
const DEFAULT_COLUMN_WIDTH: f64 = 90.0;
const ROW_HEIGHT_FACTOR: f64 = 1.5625;
const COLUMN_WIDTH_FACTOR: f64 = 9.0;

/// Upper bound for stored input records and resolved output runs.
const CONSTRUCTION_LIMIT: usize = 1_048_576;

/// Default number of operations in one resumable builder step.
pub const DEFAULT_SLICE_OPERATIONS: usize = 4096;

/// Workbook, sheet, and content identity for one build.
///
/// The host keys the builder by workbook. The other fields let the builder
/// cancel a stale read before it touches the model again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisSource {
    pub workbook_id: u64,
    pub sheet: u32,
    pub sheet_id: u32,
    pub revision: u64,
}

impl AxisSource {
    /// Read the stable engine sheet ID for this source.
    pub fn resolve(
        workbook: &UserModel<'_>,
        sheet: u32,
        workbook_id: u64,
        revision: u64,
    ) -> Result<Self, AxisSizeError> {
        let worksheet = worksheet(workbook, sheet)?;
        Ok(Self {
            workbook_id,
            sheet,
            sheet_id: worksheet.sheet_id,
            revision,
        })
    }
}

/// A resolved range with one size in unrounded logical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeRun {
    pub first: i32,
    pub last: i32,
    pub size: f64,
}

/// Resolved sizes for one axis. Indices outside the runs use `default_size`.
/// Runs are sorted, non-overlapping, and do not contain default sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisSizes {
    pub default_size: f64,
    pub runs: Vec<SizeRun>,
}

impl AxisSizes {
    /// Return the resolved size for one index.
    pub fn size_at(&self, index: i32) -> f64 {
        self.runs
            .iter()
            .find(|run| run.first <= index && index <= run.last)
            .map_or(self.default_size, |run| run.size)
    }

    /// Round and join sizes into contiguous runs with checked prefix sums.
    /// Keep the engine error context for invalid records and sizes.
    pub fn to_run_axis(&self, last_index: i32) -> Result<RunAxis, AxisSizeError> {
        let default_extent = rounded_extent(self.default_size, 0)?;
        let mut runs = Vec::with_capacity(self.runs.len());
        for (record, run) in self.runs.iter().enumerate() {
            runs.push(ExtentRun {
                first: run.first,
                last: run.last,
                extent: rounded_extent(run.size, record)?,
            });
        }
        // Core owns run order, bounds, and prefix-sum validation. Map its
        // record index back to the engine record that produced the run.
        RunAxis::from_runs(default_extent, last_index, &runs).map_err(|error| match error {
            RunAxisError::InvalidRun { run } => {
                let record = &self.runs[run];
                AxisSizeError::InvalidRecord {
                    record: run,
                    first: record.first,
                    last: record.last,
                }
            }
            RunAxisError::InvalidDefaultExtent => AxisSizeError::InvalidSize {
                record: 0,
                size: self.default_size,
            },
            RunAxisError::InvalidLimit | RunAxisError::Overflow | RunAxisError::Limit => {
                AxisSizeError::Limit
            }
        })
    }
}

/// Failure to read or construct axis sizes.
#[derive(Debug, Clone, PartialEq)]
pub enum AxisSizeError {
    Sheet {
        sheet: u32,
    },
    Stale,
    InvalidRecord {
        record: usize,
        first: i32,
        last: i32,
    },
    InvalidSize {
        record: usize,
        size: f64,
    },
    Limit,
    Incomplete,
}

impl std::fmt::Display for AxisSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sheet { sheet } => write!(f, "worksheet {sheet} is not available"),
            Self::Stale => f.write_str("axis size source changed during construction"),
            Self::InvalidRecord {
                record,
                first,
                last,
            } => write!(f, "axis record {record} has invalid range {first}..={last}"),
            Self::InvalidSize { record, size } => {
                write!(f, "axis record {record} has invalid size {size}")
            }
            Self::Limit => f.write_str("axis size construction limit reached"),
            Self::Incomplete => f.write_str("axis size construction is incomplete"),
        }
    }
}

impl std::error::Error for AxisSizeError {}

/// Progress of one resumable builder step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSizeProgress {
    Pending,
    Complete,
}

#[derive(Debug, Clone, Copy)]
struct PendingRecord {
    first: i32,
    last: i32,
    size: f64,
}

#[derive(Debug, Clone, Copy)]
struct Cover {
    start: i32,
    end: i32,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedInterval {
    first: i32,
    last: i32,
    size: f64,
    record: usize,
}

/// Keep a partial vector move private until the covered set is ordered again.
#[derive(Debug, Clone, Copy)]
enum CoverMerge {
    Locate(Cover),
    Insert {
        cover: Cover,
        index: usize,
        next: usize,
    },
    Compact {
        next: usize,
        removed: usize,
    },
}

/// Merge adjacent sorted spans. One output copy consumes one operation.
#[derive(Debug, Clone, Copy)]
struct IntervalSort {
    width: usize,
    start: usize,
    left: usize,
    right: usize,
}

impl IntervalSort {
    fn new() -> Self {
        Self {
            width: 1,
            start: 0,
            left: 0,
            right: 1,
        }
    }

    fn step(
        &mut self,
        source: &mut Vec<ResolvedInterval>,
        output: &mut Vec<ResolvedInterval>,
        budget: &mut usize,
    ) -> bool {
        let len = source.len();
        while *budget > 0 {
            if self.start >= len {
                // Every entry is now in the other buffer. Swap ownership
                // without copying entries, then reuse the empty buffer.
                std::mem::swap(source, output);
                output.clear();
                self.width *= 2;
                if self.width >= len {
                    return true;
                }
                self.start = 0;
                self.left = 0;
                self.right = self.width;
            }
            let middle = (self.start + self.width).min(len);
            let end = (middle + self.width).min(len);
            if self.left == middle && self.right == end {
                self.start = end;
                self.left = end;
                self.right = (end + self.width).min(len);
                continue;
            }
            let next = if self.left < middle
                && (self.right == end || source[self.left].first <= source[self.right].first)
            {
                let next = source[self.left];
                self.left += 1;
                next
            } else {
                let next = source[self.right];
                self.right += 1;
                next
            };
            output.push(next);
            *budget -= 1;
        }
        false
    }
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Read {
        next: usize,
    },
    Resolve {
        record: usize,
        cursor: i32,
        cover: Option<CoverMerge>,
    },
    Sort(IntervalSort),
    Emit {
        next: usize,
    },
    Done,
}

/// Builds [`AxisSizes`] in bounded steps.
pub struct AxisSizeBuilder {
    axis: Axis,
    source: AxisSource,
    slice_operations: usize,
    default_size: f64,
    total_records: usize,
    records: Vec<PendingRecord>,
    phase: Phase,
    covered: Vec<Cover>,
    resolved: Vec<ResolvedInterval>,
    resolved_ordered: bool,
    sort_buffer: Vec<ResolvedInterval>,
    runs: Vec<SizeRun>,
}

impl AxisSizeBuilder {
    /// Start a build and validate the sheet identity.
    pub fn new(
        workbook: &UserModel<'_>,
        source: AxisSource,
        axis: Axis,
    ) -> Result<Self, AxisSizeError> {
        Self::with_slice_operations(workbook, source, axis, DEFAULT_SLICE_OPERATIONS)
    }

    /// Start a build with an explicit operation bound per step.
    pub fn with_slice_operations(
        workbook: &UserModel<'_>,
        source: AxisSource,
        axis: Axis,
        slice_operations: usize,
    ) -> Result<Self, AxisSizeError> {
        let sheet = worksheet(workbook, source.sheet)?;
        if sheet.sheet_id != source.sheet_id {
            return Err(AxisSizeError::Stale);
        }
        let (default_size, total_records) = match axis {
            Axis::Row => (DEFAULT_ROW_HEIGHT, sheet.rows.len()),
            Axis::Column => (DEFAULT_COLUMN_WIDTH, sheet.cols.len()),
        };
        if total_records > CONSTRUCTION_LIMIT {
            return Err(AxisSizeError::Limit);
        }
        // Each record adds two interval boundaries. Resolution can produce
        // at most twice as many pieces as input records. Reserve this bound
        // before stepping so a push never moves a whole buffer in one step.
        let intervals = total_records.saturating_mul(2).min(CONSTRUCTION_LIMIT);
        Ok(Self {
            axis,
            source,
            slice_operations: slice_operations.max(1),
            default_size,
            total_records,
            records: Vec::with_capacity(total_records),
            phase: Phase::Read { next: 0 },
            covered: Vec::with_capacity(total_records),
            resolved: Vec::with_capacity(intervals),
            resolved_ordered: true,
            sort_buffer: Vec::with_capacity(intervals),
            runs: Vec::with_capacity(intervals),
        })
    }

    /// Completion has one source: the phase that reached emission end.
    fn is_complete(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    /// Advance the build by at most the configured number of operations.
    /// One operation reads a record, looks up an interval, moves one cover
    /// entry, copies one sort entry, or emits one interval. A cover lookup
    /// uses at most two binary searches over at most 1,048,576 entries.
    /// Emission includes the duplicate check and adjacent-run combination.
    /// Fixed phase changes and source validation are outside the count.
    /// This bounds algorithm work, not elapsed time or constructor allocation.
    /// The model borrow ends before this method returns.
    pub fn step(
        &mut self,
        workbook: &UserModel<'_>,
        source: AxisSource,
    ) -> Result<AxisSizeProgress, AxisSizeError> {
        if source != self.source {
            return Err(AxisSizeError::Stale);
        }
        let sheet = worksheet(workbook, source.sheet)?;
        if sheet.sheet_id != source.sheet_id {
            return Err(AxisSizeError::Stale);
        }
        let current_records = match self.axis {
            Axis::Row => sheet.rows.len(),
            Axis::Column => sheet.cols.len(),
        };
        if current_records != self.total_records {
            return Err(AxisSizeError::Stale);
        }
        let mut budget = self.slice_operations;
        self.advance(Some(sheet), &mut budget)?;
        Ok(if self.is_complete() {
            AxisSizeProgress::Complete
        } else {
            AxisSizeProgress::Pending
        })
    }

    /// Return the finished sizes. Partial results are never returned.
    pub fn into_sizes(self) -> Result<AxisSizes, AxisSizeError> {
        if !self.is_complete() {
            return Err(AxisSizeError::Incomplete);
        }
        Ok(AxisSizes {
            default_size: self.default_size,
            runs: self.runs,
        })
    }

    fn advance(
        &mut self,
        worksheet: Option<&Worksheet>,
        budget: &mut usize,
    ) -> Result<(), AxisSizeError> {
        loop {
            match self.phase {
                Phase::Read { next } => {
                    if next >= self.total_records {
                        self.phase = Phase::Resolve {
                            record: 0,
                            cursor: 1,
                            cover: None,
                        };
                        continue;
                    }
                    if *budget == 0 {
                        return Ok(());
                    }
                    let sheet = worksheet.ok_or(AxisSizeError::Incomplete)?;
                    self.read_record(sheet, next)?;
                    self.phase = Phase::Read { next: next + 1 };
                    *budget -= 1;
                }
                Phase::Resolve {
                    record,
                    cursor,
                    cover,
                } => {
                    if *budget == 0 {
                        return Ok(());
                    }
                    let (record, cursor, cover, finished) =
                        self.resolve_step(record, cursor, cover, budget)?;
                    self.phase = if finished && self.resolved_ordered {
                        Phase::Emit { next: 0 }
                    } else if finished {
                        Phase::Sort(IntervalSort::new())
                    } else {
                        Phase::Resolve {
                            record,
                            cursor,
                            cover,
                        }
                    };
                }
                Phase::Sort(mut sort) => {
                    self.phase = if sort.step(&mut self.resolved, &mut self.sort_buffer, budget) {
                        Phase::Emit { next: 0 }
                    } else {
                        Phase::Sort(sort)
                    };
                    if *budget == 0 {
                        return Ok(());
                    }
                }
                Phase::Emit { mut next } => {
                    let complete = self.emit_step(&mut next, budget)?;
                    self.phase = if complete {
                        Phase::Done
                    } else {
                        Phase::Emit { next }
                    };
                    return Ok(());
                }
                Phase::Done => return Ok(()),
            }
        }
    }

    fn read_record(&mut self, sheet: &Worksheet, record: usize) -> Result<(), AxisSizeError> {
        let (first, last, size) = match self.axis {
            Axis::Row => {
                let row = &sheet.rows[record];
                let first = row.r;
                let last = row.r;
                if !(1..=LAST_ROW).contains(&first) {
                    return Err(AxisSizeError::InvalidRecord {
                        record,
                        first,
                        last,
                    });
                }
                let size = if row.hidden {
                    0.0
                } else {
                    row.height * ROW_HEIGHT_FACTOR
                };
                (first, last, size)
            }
            Axis::Column => {
                let column = &sheet.cols[record];
                let (first, last) = (column.min, column.max);
                if !(1..=LAST_COLUMN).contains(&first) || !(first..=LAST_COLUMN).contains(&last) {
                    return Err(AxisSizeError::InvalidRecord {
                        record,
                        first,
                        last,
                    });
                }
                let size = if column.hidden {
                    0.0
                } else if column.custom_width {
                    column.width * COLUMN_WIDTH_FACTOR
                } else {
                    DEFAULT_COLUMN_WIDTH
                };
                (first, last, size)
            }
        };
        validate_size(record, size)?;
        if self.records.len() >= CONSTRUCTION_LIMIT {
            return Err(AxisSizeError::Limit);
        }
        self.records.push(PendingRecord { first, last, size });
        Ok(())
    }

    /// Resolve one bounded piece. Returns the next state and completion.
    fn resolve_step(
        &mut self,
        mut record: usize,
        mut cursor: i32,
        mut cover: Option<CoverMerge>,
        budget: &mut usize,
    ) -> Result<(usize, i32, Option<CoverMerge>, bool), AxisSizeError> {
        loop {
            if *budget == 0 {
                return Ok((record, cursor, cover, false));
            }
            if let Some(mut current) = cover {
                if merge_cover(&mut self.covered, &mut current, budget)? {
                    cover = None;
                } else {
                    cover = Some(current);
                }
                continue;
            }
            let Some(pending) = self.records.get(record).copied() else {
                return Ok((record, cursor, None, true));
            };
            if cursor > pending.last {
                cover = Some(CoverMerge::Locate(Cover {
                    start: pending.first,
                    end: pending.last,
                }));
                record += 1;
                cursor = 1;
                continue;
            }
            cursor = cursor.max(pending.first);
            *budget -= 1;
            match next_overlap(&self.covered, cursor, pending.last) {
                Some((start, end)) => {
                    if start > cursor {
                        self.emit(cursor, start - 1, pending.size, record)?;
                        cursor = start;
                    } else {
                        cursor = end.checked_add(1).ok_or(AxisSizeError::InvalidRecord {
                            record,
                            first: pending.first,
                            last: pending.last,
                        })?;
                    }
                }
                None => {
                    self.emit(cursor, pending.last, pending.size, record)?;
                    cursor = pending
                        .last
                        .checked_add(1)
                        .ok_or(AxisSizeError::InvalidRecord {
                            record,
                            first: pending.first,
                            last: pending.last,
                        })?;
                }
            }
        }
    }

    fn emit(
        &mut self,
        first: i32,
        last: i32,
        size: f64,
        record: usize,
    ) -> Result<(), AxisSizeError> {
        if first > last {
            return Ok(());
        }
        if self.resolved.len() >= CONSTRUCTION_LIMIT {
            return Err(AxisSizeError::Limit);
        }
        // Stored rows and columns often arrive in ascending order. Record
        // that fact during resolution so they need no sort pass.
        if let Some(previous) = self.resolved.last() {
            self.resolved_ordered &= previous.first <= first;
        }
        self.resolved.push(ResolvedInterval {
            first,
            last,
            size,
            record,
        });
        Ok(())
    }

    /// Emit resolved intervals in sorted order. Returns true when complete.
    fn emit_step(&mut self, next: &mut usize, budget: &mut usize) -> Result<bool, AxisSizeError> {
        while *budget > 0 {
            let Some(entry) = self.resolved.get(*next).copied() else {
                return Ok(true);
            };
            *budget -= 1;
            // Check before omitting defaults. A default-sized interval can
            // also have an invalid duplicate start.
            if *next > 0 && self.resolved[*next - 1].first == entry.first {
                return Err(AxisSizeError::InvalidRecord {
                    record: entry.record,
                    first: entry.first,
                    last: entry.last,
                });
            }
            *next += 1;
            // Default-size runs carry no information after precedence resolution.
            if entry.size == self.default_size {
                continue;
            }
            if let Some(previous) = self.runs.last_mut()
                && previous.last.checked_add(1) == Some(entry.first)
                && previous.size == entry.size
            {
                previous.last = entry.last;
            } else {
                if self.runs.len() >= CONSTRUCTION_LIMIT {
                    return Err(AxisSizeError::Limit);
                }
                self.runs.push(SizeRun {
                    first: entry.first,
                    last: entry.last,
                    size: entry.size,
                });
            }
        }
        Ok(false)
    }

    #[cfg(test)]
    fn synthetic(axis: Axis, default_size: f64, records: &[(i32, i32, f64)]) -> Self {
        let intervals = records.len().saturating_mul(2).min(CONSTRUCTION_LIMIT);
        Self {
            axis,
            source: AxisSource {
                workbook_id: 0,
                sheet: 0,
                sheet_id: 0,
                revision: 0,
            },
            slice_operations: 1,
            default_size,
            total_records: 0,
            records: records
                .iter()
                .map(|&(first, last, size)| PendingRecord { first, last, size })
                .collect(),
            phase: Phase::Resolve {
                record: 0,
                cursor: 1,
                cover: None,
            },
            covered: Vec::with_capacity(records.len()),
            resolved: Vec::with_capacity(intervals),
            resolved_ordered: true,
            sort_buffer: Vec::with_capacity(intervals),
            runs: Vec::with_capacity(intervals),
        }
    }

    #[cfg(test)]
    fn finish_synthetic(mut self) -> Vec<SizeRun> {
        let mut steps = 0;
        while !self.is_complete() {
            let mut budget = 1;
            self.advance(None, &mut budget).unwrap();
            steps += 1;
            assert!(steps < 1_000_000, "synthetic builder did not finish");
        }
        self.runs
    }
}

fn worksheet<'a>(workbook: &'a UserModel<'_>, sheet: u32) -> Result<&'a Worksheet, AxisSizeError> {
    workbook
        .get_model()
        .workbook
        .worksheet(sheet)
        .map_err(|_| AxisSizeError::Sheet { sheet })
}

fn validate_size(record: usize, size: f64) -> Result<(), AxisSizeError> {
    if !size.is_finite() || size < 0.0 || size.round() > f64::from(i32::MAX) {
        return Err(AxisSizeError::InvalidSize { record, size });
    }
    Ok(())
}

fn rounded_extent(size: f64, record: usize) -> Result<i32, AxisSizeError> {
    validate_size(record, size)?;
    Ok(size.round() as i32)
}

/// Return the covered interval that contains `at` or starts after it.
fn next_overlap(covered: &[Cover], at: i32, last: i32) -> Option<(i32, i32)> {
    let index = covered.partition_point(|cover| cover.start <= at);
    if let Some(cover) = index.checked_sub(1).and_then(|index| covered.get(index))
        && cover.end >= at
    {
        return Some((cover.start, cover.end));
    }
    covered
        .get(index)
        .filter(|cover| cover.start <= last)
        .map(|cover| (cover.start, cover.end))
}

/// Merge a cover interval into the covered set. Returns true when inserted.
fn merge_cover(
    covered: &mut Vec<Cover>,
    merge: &mut CoverMerge,
    budget: &mut usize,
) -> Result<bool, AxisSizeError> {
    while *budget > 0 {
        match *merge {
            CoverMerge::Locate(mut cover) => {
                *budget -= 1;
                // Covers are disjoint and nonadjacent. Two searches find the
                // whole replacement span. Its first and last entries supply
                // the merged bounds; no scan or repeated removal is needed.
                let first =
                    covered.partition_point(|item| item.end < cover.start.saturating_sub(1));
                let end = covered.partition_point(|item| item.start <= cover.end.saturating_add(1));
                if first == end {
                    if covered.len() >= CONSTRUCTION_LIMIT {
                        return Err(AxisSizeError::Limit);
                    }
                    let next = covered.len();
                    covered.push(cover);
                    if first == next {
                        return Ok(true);
                    }
                    *merge = CoverMerge::Insert {
                        cover,
                        index: first,
                        next,
                    };
                } else {
                    cover.start = cover.start.min(covered[first].start);
                    cover.end = cover.end.max(covered[end - 1].end);
                    covered[first] = cover;
                    let removed = end - first - 1;
                    if removed == 0 {
                        return Ok(true);
                    }
                    *merge = CoverMerge::Compact { next: end, removed };
                }
            }
            CoverMerge::Insert { cover, index, next } => {
                // Copy backward so a later slice still reads the old entries.
                // The copy length counts against the budget, not as one move.
                let count = (next - index).min(*budget);
                covered.copy_within(next - count..next, next - count + 1);
                *budget -= count;
                let next = next - count;
                if next == index {
                    covered[index] = cover;
                    return Ok(true);
                }
                *merge = CoverMerge::Insert { cover, index, next };
            }
            CoverMerge::Compact { next, removed } => {
                // Copy forward after a replacement. Keep the old length until
                // all source entries move. Cover has no destructor to run.
                let count = (covered.len() - next).min(*budget);
                covered.copy_within(next..next + count, next - removed);
                *budget -= count;
                let next = next + count;
                if next == covered.len() {
                    covered.truncate(covered.len() - removed);
                    return Ok(true);
                }
                *merge = CoverMerge::Compact { next, removed };
            }
        }
    }
    Ok(false)
}

/// Resolve row sizes with the individual getter rules.
pub fn get_row_sizes(workbook: &UserModel<'_>, sheet: u32) -> Result<AxisSizes, AxisSizeError> {
    finish_sync(workbook, sheet, Axis::Row)
}

/// Resolve column sizes with the individual getter rules.
pub fn get_column_sizes(workbook: &UserModel<'_>, sheet: u32) -> Result<AxisSizes, AxisSizeError> {
    finish_sync(workbook, sheet, Axis::Column)
}

fn finish_sync(
    workbook: &UserModel<'_>,
    sheet: u32,
    axis: Axis,
) -> Result<AxisSizes, AxisSizeError> {
    let source = AxisSource::resolve(workbook, sheet, 0, 0)?;
    let mut builder = AxisSizeBuilder::new(workbook, source, axis)?;
    while builder.step(workbook, source)? == AxisSizeProgress::Pending {}
    builder.into_sizes()
}

/// Identity of the worksheet and content that produced axis sizes.
///
/// A host stores one value with each cached [`WorksheetAxes`]. Navigation
/// rejects axes for another workbook or sheet. Advance `revision` only when
/// cell content or dimensions change. Scroll and selection do not change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AxesScope {
    pub workbook_id: u64,
    pub sheet_id: u32,
    pub revision: u64,
}

/// Rounded runs for both worksheet axes.
#[derive(Debug, Clone, PartialEq)]
pub struct WorksheetAxes {
    pub rows: RunAxis,
    pub cols: RunAxis,
    /// Source identity. `None` accepts unscoped axes for tests and exports.
    pub scope: Option<AxesScope>,
}

impl WorksheetAxes {
    pub fn from_sizes(rows: &AxisSizes, cols: &AxisSizes) -> Result<Self, AxisSizeError> {
        Ok(Self {
            rows: rows.to_run_axis(LAST_ROW)?,
            cols: cols.to_run_axis(LAST_COLUMN)?,
            scope: None,
        })
    }

    /// Attach the source identity used by host caches and navigation checks.
    pub fn with_scope(mut self, scope: AxesScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Scrolling extent for the row axis.
    pub fn scrolling_rows(&self, frozen_rows: i32) -> i64 {
        scrolling_extent(
            &self.rows,
            ScrollAxisWindow::after_frozen(frozen_rows, LAST_ROW),
        )
    }

    /// Scrolling extent for the column axis.
    pub fn scrolling_columns(&self, frozen_columns: i32) -> i64 {
        scrolling_extent(
            &self.cols,
            ScrollAxisWindow::after_frozen(frozen_columns, LAST_COLUMN),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::AxisExtents;
    use iron_canvas_core::scroll::{ScrollPosition, move_axis, normalize_axis, normalize_position};

    fn row_run(first: i32, last: i32, size: f64) -> SizeRun {
        SizeRun { first, last, size }
    }

    fn sizes(default_size: f64, runs: Vec<SizeRun>) -> AxisSizes {
        AxisSizes { default_size, runs }
    }

    #[test]
    fn bulk_rows_and_columns_match_individual_getters() {
        let mut model = UserModel::new_empty("sizes", "en", "UTC", "en").unwrap();
        assert_eq!(
            get_row_sizes(&model, 0).unwrap(),
            sizes(DEFAULT_ROW_HEIGHT, Vec::new())
        );
        assert_eq!(
            get_column_sizes(&model, 0).unwrap(),
            sizes(DEFAULT_COLUMN_WIDTH, Vec::new())
        );
        model.set_rows_height(0, 2, 4, 32.0).unwrap();
        model.set_rows_height(0, 7, 7, 12.5).unwrap();
        model.set_rows_hidden(0, 3, 3, true).unwrap();
        model.set_columns_width(0, 2, 5, 12.0).unwrap();
        model.set_columns_width(0, 9, 9, 7.5).unwrap();
        model.set_columns_hidden(0, 4, 4, true).unwrap();

        let rows = get_row_sizes(&model, 0).unwrap();
        for index in 1..=20 {
            assert_eq!(
                rows.size_at(index),
                model.get_row_height(0, index).unwrap(),
                "row {index}"
            );
        }
        let columns = get_column_sizes(&model, 0).unwrap();
        for index in 1..=20 {
            assert_eq!(
                columns.size_at(index),
                model.get_column_width(0, index).unwrap(),
                "column {index}"
            );
        }
        assert_eq!(rows.default_size, DEFAULT_ROW_HEIGHT);
        assert_eq!(columns.default_size, DEFAULT_COLUMN_WIDTH);
        assert_eq!(rows.size_at(3), 0.0);
        assert_eq!(columns.size_at(4), 0.0);
    }

    #[test]
    fn model_changes_and_history_refresh_the_bulk_result() {
        let mut model = UserModel::new_empty("changes", "en", "UTC", "en").unwrap();
        model.set_columns_width(0, 2, 2, 20.0).unwrap();
        assert_eq!(
            get_column_sizes(&model, 0).unwrap().runs,
            [row_run(2, 2, 20.0)]
        );
        model.set_columns_width(0, 2, 2, 30.0).unwrap();
        assert_eq!(
            get_column_sizes(&model, 0).unwrap().runs,
            [row_run(2, 2, 30.0)]
        );
        model.undo().unwrap();
        assert_eq!(
            get_column_sizes(&model, 0).unwrap().runs,
            [row_run(2, 2, 20.0)]
        );
        model.redo().unwrap();
        assert_eq!(
            get_column_sizes(&model, 0).unwrap().runs,
            [row_run(2, 2, 30.0)]
        );
    }

    #[test]
    fn engine_axis_sums_are_checked() {
        // Core covers prefix-sum overflow on `RunAxis`. Engine sizes cannot
        // exceed the i64 sum over at most one million rows.
        let axis = sizes(DEFAULT_ROW_HEIGHT, vec![]).to_run_axis(3).unwrap();
        assert_eq!(axis.total_extent(), 75);
    }

    #[test]
    fn precedence_keeps_the_first_record_and_masks_later_ones() {
        let runs = AxisSizeBuilder::synthetic(
            Axis::Row,
            90.0,
            &[(3, 7, 20.0), (5, 9, 50.0), (1, 4, 10.0)],
        )
        .finish_synthetic();
        assert_eq!(
            runs,
            [
                row_run(1, 2, 10.0),
                row_run(3, 7, 20.0),
                row_run(8, 9, 50.0)
            ]
        );
    }

    #[test]
    fn duplicate_indices_use_the_first_record() {
        let runs = AxisSizeBuilder::synthetic(
            Axis::Row,
            DEFAULT_ROW_HEIGHT,
            &[(2, 2, 30.0), (2, 2, 60.0), (4, 4, 40.0)],
        )
        .finish_synthetic();
        assert_eq!(runs, [row_run(2, 2, 30.0), row_run(4, 4, 40.0)]);
    }

    #[test]
    fn an_earlier_default_column_masks_a_later_custom_record() {
        let runs = AxisSizeBuilder::synthetic(
            Axis::Column,
            DEFAULT_COLUMN_WIDTH,
            &[(2, 4, DEFAULT_COLUMN_WIDTH), (3, 3, 180.0)],
        )
        .finish_synthetic();
        assert!(runs.is_empty());
    }

    #[test]
    fn adversarial_record_orders_finish_with_single_operation_steps() {
        // Later records overlap earlier ones, so covers merge and move entries.
        let overlapping = AxisSizeBuilder::synthetic(
            Axis::Row,
            DEFAULT_ROW_HEIGHT,
            &[(8, 9, 20.0), (6, 10, 30.0), (1, 7, 10.0)],
        )
        .finish_synthetic();
        assert_eq!(
            overlapping,
            [
                row_run(1, 5, 10.0),
                row_run(6, 7, 30.0),
                row_run(8, 9, 20.0),
                row_run(10, 10, 30.0)
            ]
        );
        // Reverse order emits in ascending index order.
        let reverse = AxisSizeBuilder::synthetic(
            Axis::Row,
            DEFAULT_ROW_HEIGHT,
            &[(5, 5, 50.0), (3, 3, 30.0), (1, 1, 10.0)],
        )
        .finish_synthetic();
        assert_eq!(
            reverse,
            [
                row_run(1, 1, 10.0),
                row_run(3, 3, 30.0),
                row_run(5, 5, 50.0)
            ]
        );
        // Sparse records keep large default gaps.
        let sparse = AxisSizeBuilder::synthetic(
            Axis::Column,
            DEFAULT_COLUMN_WIDTH,
            &[(1, 1, 40.0), (100, 100, 50.0), (1_000, 1_000, 60.0)],
        )
        .finish_synthetic();
        assert_eq!(
            sparse,
            [
                row_run(1, 1, 40.0),
                row_run(100, 100, 50.0),
                row_run(1_000, 1_000, 60.0)
            ]
        );
    }

    #[test]
    fn cover_insertion_and_compaction_yield_during_entry_moves() {
        let original: Vec<_> = (1..=512)
            .map(|index| Cover {
                start: index * 4,
                end: index * 4,
            })
            .collect();
        for inserted in [Cover { start: 1, end: 1 }, Cover { start: 4, end: 20 }] {
            let mut covered = Vec::with_capacity(original.len() + 1);
            covered.extend_from_slice(&original);
            let mut merge = CoverMerge::Locate(inserted);
            let mut budget = 1;
            assert!(!merge_cover(&mut covered, &mut merge, &mut budget).unwrap());
            assert_eq!(budget, 0);

            // The first call finds the replacement. The second moves just
            // one entry, so neither large suffix can be complete yet.
            let mut budget = 1;
            assert!(!merge_cover(&mut covered, &mut merge, &mut budget).unwrap());
            assert_eq!(budget, 0);
            let capacity = covered.capacity();
            loop {
                let mut budget = 3;
                if merge_cover(&mut covered, &mut merge, &mut budget).unwrap() {
                    break;
                }
                assert_eq!(covered.capacity(), capacity);
            }
            let expected: Vec<_> = std::iter::once((inserted.start, inserted.end))
                .chain(
                    original
                        .iter()
                        .filter(|cover| cover.start > inserted.end)
                        .map(|cover| (cover.start, cover.end)),
                )
                .collect();
            assert_eq!(
                covered
                    .iter()
                    .map(|cover| (cover.start, cover.end))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(covered.capacity(), capacity);
        }
    }

    #[test]
    fn sorting_copies_only_the_budget_and_keeps_both_capacities() {
        let mut source: Vec<_> = (1..=513)
            .rev()
            .map(|first| ResolvedInterval {
                first,
                last: first,
                size: 40.0,
                record: first as usize,
            })
            .collect();
        let mut output = Vec::with_capacity(source.capacity());
        let capacity = source.capacity();
        let mut sort = IntervalSort::new();
        let mut budget = 1;
        assert!(!sort.step(&mut source, &mut output, &mut budget));
        assert_eq!(output.len(), 1);
        assert_eq!(source[0].first, 513);
        let mut budget = 2;
        assert!(!sort.step(&mut source, &mut output, &mut budget));
        assert_eq!(output.len(), 3);
        let mut calls = 0;
        loop {
            let mut budget = 7;
            let complete = sort.step(&mut source, &mut output, &mut budget);
            assert_eq!(source.capacity(), capacity);
            assert_eq!(output.capacity(), capacity);
            calls += 1;
            assert!(calls < 10_000, "sort did not finish");
            if complete {
                break;
            }
        }
        assert_eq!(
            source.iter().map(|entry| entry.first).collect::<Vec<_>>(),
            (1..=513).collect::<Vec<_>>()
        );
        assert!(output.is_empty());
    }

    #[test]
    fn duplicate_check_yields_with_emission_and_includes_default_sizes() {
        let mut builder = AxisSizeBuilder::synthetic(Axis::Row, DEFAULT_ROW_HEIGHT, &[]);
        builder.phase = Phase::Emit { next: 0 };
        builder.resolved = (0..2)
            .map(|record| ResolvedInterval {
                first: 1,
                last: 1,
                size: DEFAULT_ROW_HEIGHT,
                record,
            })
            .collect();
        let mut budget = 1;
        builder.advance(None, &mut budget).unwrap();
        assert!(!builder.is_complete());
        assert!(builder.runs.is_empty());
        let mut budget = 1;
        assert_eq!(
            builder.advance(None, &mut budget),
            Err(AxisSizeError::InvalidRecord {
                record: 1,
                first: 1,
                last: 1,
            })
        );
    }

    #[test]
    fn ordered_records_skip_sorting_and_all_steps_keep_reserved_capacity() {
        let mut model = UserModel::new_empty("capacity", "en", "UTC", "en").unwrap();
        for row in 1..=100 {
            model
                .set_rows_height(0, row * 2, row * 2, 30.0 + f64::from(row % 2))
                .unwrap();
        }
        let source = AxisSource::resolve(&model, 0, 1, 1).unwrap();
        let mut builder =
            AxisSizeBuilder::with_slice_operations(&model, source, Axis::Row, 1).unwrap();
        let capacities = |builder: &AxisSizeBuilder| {
            [
                builder.records.capacity(),
                builder.covered.capacity(),
                builder.resolved.capacity(),
                builder.sort_buffer.capacity(),
                builder.runs.capacity(),
            ]
        };
        let reserved = capacities(&builder);
        let mut calls = 0;
        while builder.step(&model, source).unwrap() == AxisSizeProgress::Pending {
            assert!(!matches!(builder.phase, Phase::Sort(_)));
            assert_eq!(capacities(&builder), reserved);
            calls += 1;
            assert!(calls < 10_000, "builder did not finish");
        }
        assert_eq!(capacities(&builder), reserved);
        let sizes = builder.into_sizes().unwrap();
        for row in 1..=201 {
            assert_eq!(sizes.size_at(row), model.get_row_height(0, row).unwrap());
        }
    }

    #[test]
    fn fragmented_resolution_and_sorting_keep_reserved_capacity() {
        // Each early single-cell record splits the final broad record. This
        // reaches 2*n-1 pieces and exercises the reservation bound.
        let mut records: Vec<_> = (1..=128).map(|i| (i * 2, i * 2, 40.0)).collect();
        records.push((1, 257, 20.0));
        let mut builder = AxisSizeBuilder::synthetic(Axis::Row, DEFAULT_ROW_HEIGHT, &records);
        let capacities = |builder: &AxisSizeBuilder| {
            [
                builder.covered.capacity(),
                builder.resolved.capacity(),
                builder.sort_buffer.capacity(),
                builder.runs.capacity(),
            ]
        };
        let reserved = capacities(&builder);
        let mut calls = 0;
        while !builder.is_complete() {
            let mut budget = 1;
            builder.advance(None, &mut budget).unwrap();
            assert_eq!(capacities(&builder), reserved);
            calls += 1;
            assert!(calls < 10_000, "builder did not finish");
        }
        let expected: Vec<_> = (1..=257)
            .map(|i| row_run(i, i, if i % 2 == 0 { 40.0 } else { 20.0 }))
            .collect();
        assert_eq!(builder.runs, expected);
    }

    #[test]
    fn merged_covers_match_brute_force_precedence() {
        const LIMIT: i32 = 80;
        let sizes = [0.0, 10.0, DEFAULT_ROW_HEIGHT, 40.0, 60.0];
        for seed in 1..=20_u64 {
            let mut state = seed;
            let mut records = Vec::with_capacity(200);
            for _ in 0..200 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let first = 1 + (state % LIMIT as u64) as i32;
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let last = (first + (state % 12) as i32).min(LIMIT);
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                records.push((first, last, sizes[(state % 5) as usize]));
            }
            // First matching record wins for each index; default runs vanish.
            let mut expected: Vec<SizeRun> = Vec::new();
            for index in 1..=LIMIT {
                let size = records
                    .iter()
                    .find(|(first, last, _)| *first <= index && index <= *last)
                    .map_or(DEFAULT_ROW_HEIGHT, |record| record.2);
                if size == DEFAULT_ROW_HEIGHT {
                    continue;
                }
                if let Some(run) = expected.last_mut()
                    && run.last + 1 == index
                    && run.size == size
                {
                    run.last = index;
                } else {
                    expected.push(SizeRun {
                        first: index,
                        last: index,
                        size,
                    });
                }
            }
            for step_budget in [1, 2, 7, 64, DEFAULT_SLICE_OPERATIONS] {
                let mut builder =
                    AxisSizeBuilder::synthetic(Axis::Row, DEFAULT_ROW_HEIGHT, &records);
                let mut calls = 0;
                while !builder.is_complete() {
                    let mut budget = step_budget;
                    builder.advance(None, &mut budget).unwrap();
                    calls += 1;
                    assert!(calls < 100_000, "builder did not finish");
                }
                assert_eq!(builder.runs, expected, "seed {seed}, budget {step_budget}");
            }
        }
    }

    #[test]
    fn default_runs_are_omitted_only_after_resolution() {
        let runs = AxisSizeBuilder::synthetic(
            Axis::Row,
            DEFAULT_ROW_HEIGHT,
            &[(1, 1, 20.0), (2, 2, DEFAULT_ROW_HEIGHT), (3, 3, 20.0)],
        )
        .finish_synthetic();
        assert_eq!(runs, [row_run(1, 1, 20.0), row_run(3, 3, 20.0)]);
    }

    #[test]
    fn sliced_construction_matches_synchronous_construction() {
        let mut model = UserModel::new_empty("slices", "en", "UTC", "en").unwrap();
        for row in 1..=40 {
            model
                .set_rows_height(0, row, row, f64::from(10 + row % 7))
                .unwrap();
        }
        for column in 1..=25 {
            model
                .set_columns_width(0, column, column, f64::from(5 + column % 5))
                .unwrap();
        }
        model.set_rows_hidden(0, 5, 9, true).unwrap();
        model.set_columns_hidden(0, 3, 3, true).unwrap();
        for (axis, sync) in [
            (Axis::Row, get_row_sizes(&model, 0).unwrap()),
            (Axis::Column, get_column_sizes(&model, 0).unwrap()),
        ] {
            let source = AxisSource::resolve(&model, 0, 1, 7).unwrap();
            let mut sliced =
                AxisSizeBuilder::with_slice_operations(&model, source, axis, 1).unwrap();
            let mut steps = 0;
            while sliced.step(&model, source).unwrap() == AxisSizeProgress::Pending {
                steps += 1;
                assert!(steps < 10_000, "builder did not finish");
            }
            assert_eq!(sliced.into_sizes().unwrap(), sync);
        }
    }

    #[test]
    fn a_changed_source_cancels_the_builder() {
        let mut model = UserModel::new_empty("stale", "en", "UTC", "en").unwrap();
        model.set_rows_height(0, 1, 3, 24.0).unwrap();
        let source = AxisSource::resolve(&model, 0, 1, 1).unwrap();
        let mut builder =
            AxisSizeBuilder::with_slice_operations(&model, source, Axis::Row, 1).unwrap();
        assert_eq!(builder.step(&model, source), Ok(AxisSizeProgress::Pending));
        let mut stale = source;
        stale.revision = 2;
        assert_eq!(builder.step(&model, stale), Err(AxisSizeError::Stale));
        stale = source;
        stale.workbook_id = 2;
        assert_eq!(builder.step(&model, stale), Err(AxisSizeError::Stale));
        assert_eq!(
            AxisSizeBuilder::new(&model, source, Axis::Row)
                .unwrap()
                .into_sizes(),
            Err(AxisSizeError::Incomplete)
        );
    }

    #[test]
    fn invalid_sheets_sizes_and_runs_are_errors() {
        let model = UserModel::new_empty("invalid", "en", "UTC", "en").unwrap();
        assert_eq!(
            get_row_sizes(&model, 9),
            Err(AxisSizeError::Sheet { sheet: 9 })
        );
        assert!(matches!(
            sizes(DEFAULT_ROW_HEIGHT, vec![row_run(3, 2, 10.0)]).to_run_axis(10),
            Err(AxisSizeError::InvalidRecord { .. })
        ));
        assert!(matches!(
            sizes(
                DEFAULT_ROW_HEIGHT,
                vec![row_run(1, 3, 10.0), row_run(3, 4, 20.0)]
            )
            .to_run_axis(10),
            Err(AxisSizeError::InvalidRecord { .. })
        ));
        assert!(matches!(
            sizes(DEFAULT_ROW_HEIGHT, vec![row_run(1, 1, f64::NAN)]).to_run_axis(10),
            Err(AxisSizeError::InvalidSize { .. })
        ));
        // Core rejects a run beyond the limit; the adapter restores the record.
        assert_eq!(
            sizes(DEFAULT_ROW_HEIGHT, vec![row_run(1, 11, 10.0)]).to_run_axis(10),
            Err(AxisSizeError::InvalidRecord {
                record: 0,
                first: 1,
                last: 11
            })
        );
    }

    #[test]
    fn axes_round_defaults_between_records_and_keep_prefix_sums() {
        let rows = sizes(
            DEFAULT_ROW_HEIGHT,
            vec![row_run(2, 3, 32.4), row_run(5, 5, 0.0)],
        );
        let axis = rows.to_run_axis(6).unwrap();
        assert_eq!(
            axis.runs(),
            [
                AxisRun {
                    first: 1,
                    last: 1,
                    extent: 25,
                    prefix: 0
                },
                AxisRun {
                    first: 2,
                    last: 3,
                    extent: 32,
                    prefix: 25
                },
                AxisRun {
                    first: 4,
                    last: 4,
                    extent: 25,
                    prefix: 89
                },
                AxisRun {
                    first: 5,
                    last: 5,
                    extent: 0,
                    prefix: 114
                },
                AxisRun {
                    first: 6,
                    last: 6,
                    extent: 25,
                    prefix: 114
                },
            ]
        );
        assert_eq!(axis.total_extent(), 139);
        assert_eq!(axis.extent_before(3), 57);
        assert_eq!(axis.extent_before(7), 139);
        assert_eq!(axis.extent(5), 0);
        assert_eq!(axis.extent(6), 25);
        // Equal adjacent extents combine into one run.
        let combined = sizes(DEFAULT_ROW_HEIGHT, vec![row_run(1, 2, 30.2)])
            .to_run_axis(3)
            .unwrap();
        assert_eq!(combined.runs()[0].last, 2);
        assert_eq!(combined.runs()[1].first, 3);
    }

    #[test]
    fn axes_skip_zero_runs_and_apply_end_limits() {
        let axis = sizes(
            DEFAULT_ROW_HEIGHT,
            vec![row_run(2, 4, 0.0), row_run(5, 6, 10.0), row_run(7, 7, 0.0)],
        )
        .to_run_axis(8)
        .unwrap();
        let window = ScrollAxisWindow::new(1, 8);
        assert_eq!(
            normalize_axis(1, 0, &axis, window),
            iron_canvas_core::AxisScroll {
                first: 1,
                offset: 0
            }
        );
        assert_eq!(
            move_axis(1, 0, 25 + 10 + 1, &axis, window, 32),
            iron_canvas_core::AxisScroll {
                first: 6,
                offset: 1
            }
        );
        // E = 25 + 10*2 + 25, pane 32 -> max distance 38.
        assert_eq!(
            move_axis(1, 0, 1_000, &axis, window, 32),
            iron_canvas_core::AxisScroll {
                first: 6,
                offset: 3
            }
        );
        let position = normalize_position(
            ScrollPosition {
                first_cell: iron_canvas_core::CellCoord { row: 3, col: 1 },
                offset_x: 0,
                offset_y: 7,
            },
            &axis,
            window,
            &axis,
            window,
        );
        assert_eq!(position.first_cell.row, 5);
        assert_eq!(position.offset_y, 7);
    }

    #[test]
    fn worksheet_axes_observe_frozen_bands() {
        let rows = sizes(DEFAULT_ROW_HEIGHT, vec![row_run(1, 2, 40.4)]);
        let cols = sizes(DEFAULT_COLUMN_WIDTH, vec![row_run(1, 1, 100.0)]);
        let axes = WorksheetAxes::from_sizes(&rows, &cols).unwrap();
        assert_eq!(axes.scrolling_rows(2), axes.rows.total_extent() - 80);
        assert_eq!(axes.scrolling_columns(1), axes.cols.total_extent() - 100);
    }
}
