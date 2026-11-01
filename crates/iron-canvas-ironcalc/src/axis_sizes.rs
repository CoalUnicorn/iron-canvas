//! Bulk row and column size reads over IronCalc worksheet records.
//!
//! The engine returns one size at a time. This module resolves stored records
//! into ordered runs and keeps the same rules as the individual getters.
//! Rules checked against IronCalc revision `0be0ff0c`.
//!
//! The resumable builder reads a bounded number of source records per step.
//! Interactive hosts can advance it between frames; synchronous callers run it
//! to completion.

pub use iron_canvas_core::scroll::AxisRun;
use iron_canvas_core::{
    AxisExtents, ScrollAxisWindow,
    scroll::{ExtentRun, RunAxis, RunAxisError, scrolling_extent},
};
use ironcalc_base::{UserModel, types::Worksheet};
use std::collections::BTreeMap;

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

/// Axis of a worksheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

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
enum Phase {
    Read {
        next: usize,
    },
    Resolve {
        record: usize,
        cursor: i32,
        cover: Option<Cover>,
    },
    Emit,
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
    covered: BTreeMap<i32, i32>,
    resolved: BTreeMap<i32, (i32, f64)>,
    runs: Vec<SizeRun>,
    done: bool,
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
        Ok(Self {
            axis,
            source,
            slice_operations: slice_operations.max(1),
            default_size,
            total_records,
            records: Vec::new(),
            phase: Phase::Read { next: 0 },
            covered: BTreeMap::new(),
            resolved: BTreeMap::new(),
            runs: Vec::new(),
            done: false,
        })
    }

    /// Advance the build by at most the configured number of operations.
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
        Ok(if self.done {
            AxisSizeProgress::Complete
        } else {
            AxisSizeProgress::Pending
        })
    }

    /// Return the finished sizes. Partial results are never returned.
    pub fn into_sizes(self) -> Result<AxisSizes, AxisSizeError> {
        if !self.done {
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
                    self.phase = if finished {
                        Phase::Emit
                    } else {
                        Phase::Resolve {
                            record,
                            cursor,
                            cover,
                        }
                    };
                }
                Phase::Emit => {
                    if !self.emit_step(budget)? {
                        return Ok(());
                    }
                    self.phase = Phase::Done;
                    self.done = true;
                    return Ok(());
                }
                Phase::Done => {
                    self.done = true;
                    return Ok(());
                }
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
        mut cover: Option<Cover>,
        budget: &mut usize,
    ) -> Result<(usize, i32, Option<Cover>, bool), AxisSizeError> {
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
                cover = Some(Cover {
                    start: pending.first,
                    end: pending.last,
                });
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
        if self.resolved.insert(first, (last, size)).is_some() {
            return Err(AxisSizeError::InvalidRecord {
                record,
                first,
                last,
            });
        }
        Ok(())
    }

    /// Drain resolved intervals in sorted order. Returns true when complete.
    fn emit_step(&mut self, budget: &mut usize) -> Result<bool, AxisSizeError> {
        while *budget > 0 {
            let Some((first, (last, size))) = self.resolved.pop_first() else {
                return Ok(true);
            };
            *budget -= 1;
            // Default-size runs carry no information after precedence resolution.
            if size == self.default_size {
                continue;
            }
            if let Some(previous) = self.runs.last_mut()
                && previous.last.checked_add(1) == Some(first)
                && previous.size == size
            {
                previous.last = last;
            } else {
                if self.runs.len() >= CONSTRUCTION_LIMIT {
                    return Err(AxisSizeError::Limit);
                }
                self.runs.push(SizeRun { first, last, size });
            }
        }
        Ok(false)
    }

    #[cfg(test)]
    fn synthetic(axis: Axis, default_size: f64, records: &[(i32, i32, f64)]) -> Self {
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
            covered: BTreeMap::new(),
            resolved: BTreeMap::new(),
            runs: Vec::new(),
            done: false,
        }
    }

    #[cfg(test)]
    fn finish_synthetic(mut self) -> Vec<SizeRun> {
        let mut steps = 0;
        while !self.done {
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
fn next_overlap(covered: &BTreeMap<i32, i32>, at: i32, last: i32) -> Option<(i32, i32)> {
    if let Some((&start, &end)) = covered.range(..=at).next_back()
        && end >= at
    {
        return Some((start, end));
    }
    covered
        .range(at..=last)
        .next()
        .map(|(&start, &end)| (start, end))
}

/// Merge a cover interval into the covered set. Returns true when inserted.
fn merge_cover(
    covered: &mut BTreeMap<i32, i32>,
    cover: &mut Cover,
    budget: &mut usize,
) -> Result<bool, AxisSizeError> {
    loop {
        if *budget == 0 {
            return Ok(false);
        }
        *budget -= 1;
        if let Some((&start, &end)) = covered.range(..=cover.start).next_back()
            && end >= cover.start.saturating_sub(1)
        {
            covered.remove(&start);
            cover.start = start;
            cover.end = cover.end.max(end);
            continue;
        }
        let limit = cover.end.saturating_add(1);
        if let Some((&start, &end)) = covered.range(cover.start..=limit).next() {
            covered.remove(&start);
            cover.end = cover.end.max(end);
            continue;
        }
        if covered.len() >= CONSTRUCTION_LIMIT {
            return Err(AxisSizeError::Limit);
        }
        covered.insert(cover.start, cover.end);
        return Ok(true);
    }
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

/// Rounded, contiguous extents for one axis. Every index in
/// `1..=last_index` belongs to exactly one run.
#[derive(Debug, Clone, PartialEq)]
pub struct WorksheetAxis {
    axis: RunAxis,
}

impl WorksheetAxis {
    /// Round and join sizes into contiguous runs with checked prefix sums.
    pub fn from_sizes(sizes: &AxisSizes, last_index: i32) -> Result<Self, AxisSizeError> {
        if last_index < 1 {
            return Err(AxisSizeError::Limit);
        }
        let default_extent = rounded_extent(sizes.default_size, 0)?;
        let mut runs = Vec::with_capacity(sizes.runs.len());
        let mut cursor = 1_i32;
        for (record, run) in sizes.runs.iter().enumerate() {
            if run.first < 1 || run.last < run.first || run.last > last_index {
                return Err(AxisSizeError::InvalidRecord {
                    record,
                    first: run.first,
                    last: run.last,
                });
            }
            if run.first < cursor {
                return Err(AxisSizeError::InvalidRecord {
                    record,
                    first: run.first,
                    last: run.last,
                });
            }
            runs.push(ExtentRun {
                first: run.first,
                last: run.last,
                extent: rounded_extent(run.size, record)?,
            });
            cursor = run.last.checked_add(1).ok_or(AxisSizeError::Limit)?;
        }
        let axis =
            RunAxis::from_runs(default_extent, last_index, &runs).map_err(|error| match error {
                RunAxisError::InvalidRun { run } => {
                    let record = &sizes.runs[run];
                    AxisSizeError::InvalidRecord {
                        record: run,
                        first: record.first,
                        last: record.last,
                    }
                }
                RunAxisError::InvalidDefaultExtent => AxisSizeError::InvalidSize {
                    record: 0,
                    size: sizes.default_size,
                },
                RunAxisError::InvalidLimit | RunAxisError::Overflow | RunAxisError::Limit => {
                    AxisSizeError::Limit
                }
            })?;
        Ok(Self { axis })
    }

    pub fn default_extent(&self) -> i32 {
        self.axis.default_extent()
    }

    pub fn total_extent(&self) -> i64 {
        self.axis.total_extent()
    }

    pub fn last_index(&self) -> i32 {
        self.axis.last_index()
    }

    pub fn runs(&self) -> &[AxisRun] {
        self.axis.runs()
    }
}

impl AxisExtents for WorksheetAxis {
    fn default_extent(&self) -> i32 {
        self.axis.default_extent()
    }

    fn extent(&self, index: i32) -> i32 {
        AxisExtents::extent(&self.axis, index)
    }

    fn extent_before(&self, index: i32) -> i64 {
        AxisExtents::extent_before(&self.axis, index)
    }

    fn total_extent(&self) -> i64 {
        self.axis.total_extent()
    }

    fn locate(&self, distance: i64, window: ScrollAxisWindow) -> Option<(i32, i32)> {
        AxisExtents::locate(&self.axis, distance, window)
    }
}

/// Rounded runs for both worksheet axes.
#[derive(Debug, Clone, PartialEq)]
pub struct WorksheetAxes {
    pub rows: WorksheetAxis,
    pub cols: WorksheetAxis,
}

impl WorksheetAxes {
    pub fn from_sizes(rows: &AxisSizes, cols: &AxisSizes) -> Result<Self, AxisSizeError> {
        Ok(Self {
            rows: WorksheetAxis::from_sizes(rows, LAST_ROW)?,
            cols: WorksheetAxis::from_sizes(cols, LAST_COLUMN)?,
        })
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
        let axis = WorksheetAxis::from_sizes(&sizes(DEFAULT_ROW_HEIGHT, vec![]), 3).unwrap();
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
            WorksheetAxis::from_sizes(&sizes(DEFAULT_ROW_HEIGHT, vec![row_run(3, 2, 10.0)]), 10),
            Err(AxisSizeError::InvalidRecord { .. })
        ));
        assert!(matches!(
            WorksheetAxis::from_sizes(
                &sizes(
                    DEFAULT_ROW_HEIGHT,
                    vec![row_run(1, 3, 10.0), row_run(3, 4, 20.0)]
                ),
                10
            ),
            Err(AxisSizeError::InvalidRecord { .. })
        ));
        assert!(matches!(
            WorksheetAxis::from_sizes(
                &sizes(DEFAULT_ROW_HEIGHT, vec![row_run(1, 1, f64::NAN)]),
                10
            ),
            Err(AxisSizeError::InvalidSize { .. })
        ));
    }

    #[test]
    fn axes_round_defaults_between_records_and_keep_prefix_sums() {
        let rows = sizes(
            DEFAULT_ROW_HEIGHT,
            vec![row_run(2, 3, 32.4), row_run(5, 5, 0.0)],
        );
        let axis = WorksheetAxis::from_sizes(&rows, 6).unwrap();
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
        let combined =
            WorksheetAxis::from_sizes(&sizes(DEFAULT_ROW_HEIGHT, vec![row_run(1, 2, 30.2)]), 3)
                .unwrap();
        assert_eq!(combined.runs()[0].last, 2);
        assert_eq!(combined.runs()[1].first, 3);
    }

    #[test]
    fn axes_skip_zero_runs_and_apply_end_limits() {
        let axis = WorksheetAxis::from_sizes(
            &sizes(
                DEFAULT_ROW_HEIGHT,
                vec![row_run(2, 4, 0.0), row_run(5, 6, 10.0), row_run(7, 7, 0.0)],
            ),
            8,
        )
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
