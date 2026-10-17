//! Bounded selection algebra. The host validates merges before acceptance.

use crate::interaction::{LAST_COLUMN, LAST_ROW};
use iron_canvas_core::{CellCoord, scene_geometry::GridRange};

pub const SELECTION_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AreaId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    Cells,
    Rows,
    Columns,
    Sheet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Normal,
    Extend,
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionError {
    Bounds,
    InvalidArea,
    InvalidActiveArea,
    InvalidActiveCell,
    Empty,
    AreaLimit,
    PieceLimit,
    IdentityExhausted,
    RevisionExhausted,
    UnsupportedMerge,
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid selection: {self:?}")
    }
}
impl std::error::Error for SelectionError {}

/// Engine-resolved coordinates. Rectangle algebra alone cannot prove merge closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionTarget {
    kind: SelectionKind,
    anchor: CellCoord,
    focus: CellCoord,
    range: GridRange,
}

impl SelectionTarget {
    pub fn new(
        kind: SelectionKind,
        anchor: CellCoord,
        focus: CellCoord,
        range: GridRange,
    ) -> Result<Self, SelectionError> {
        let range = range.normalized();
        if !valid_cell(range.first) || !valid_cell(range.last) {
            return Err(SelectionError::Bounds);
        }
        if !range.contains(anchor) || !range.contains(focus) {
            return Err(SelectionError::InvalidArea);
        }
        let full_rows = range.first.col == 1 && range.last.col == LAST_COLUMN;
        let full_cols = range.first.row == 1 && range.last.row == LAST_ROW;
        if match kind {
            SelectionKind::Cells => full_rows || full_cols,
            SelectionKind::Rows => !full_rows,
            SelectionKind::Columns => !full_cols,
            SelectionKind::Sheet => !(full_rows && full_cols),
        } {
            return Err(SelectionError::InvalidArea);
        }
        Ok(Self {
            kind,
            anchor,
            focus,
            range,
        })
    }
    pub fn range(&self) -> GridRange {
        self.range
    }
    pub fn anchor(&self) -> CellCoord {
        self.anchor
    }
    pub fn focus(&self) -> CellCoord {
        self.focus
    }
    pub fn kind(&self) -> SelectionKind {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionArea {
    id: AreaId,
    target: SelectionTarget,
}

impl SelectionArea {
    pub fn new(id: AreaId, target: SelectionTarget) -> Self {
        Self { id, target }
    }
    pub fn id(&self) -> AreaId {
        self.id
    }
    pub fn target(&self) -> SelectionTarget {
        self.target
    }
    pub fn range(&self) -> GridRange {
        self.target.range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionSnapshot {
    workbook_id: u64,
    sheet: u32,
    revision: u64,
    areas: Vec<SelectionArea>,
    active_area: AreaId,
    active_cell: CellCoord,
    mode: SelectionMode,
    last_area_id: u64,
}

impl SelectionSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workbook_id: u64,
        sheet: u32,
        revision: u64,
        areas: Vec<SelectionArea>,
        active_area: AreaId,
        active_cell: CellCoord,
        mode: SelectionMode,
    ) -> Result<Self, SelectionError> {
        if areas.is_empty() {
            return Err(SelectionError::Empty);
        }
        if areas.len() > SELECTION_LIMIT {
            return Err(SelectionError::AreaLimit);
        }
        for (index, area) in areas.iter().enumerate() {
            if areas[..index].iter().any(|other| other.id == area.id) {
                return Err(SelectionError::InvalidArea);
            }
        }
        let active = areas
            .iter()
            .find(|area| area.id == active_area)
            .ok_or(SelectionError::InvalidActiveArea)?;
        if !active.range().contains(active_cell) {
            return Err(SelectionError::InvalidActiveCell);
        }
        bounded_union(areas.iter().map(SelectionArea::range))?;
        let last_area_id = areas.iter().map(|area| area.id.0).max().unwrap_or(0);
        Ok(Self {
            last_area_id,
            workbook_id,
            sheet,
            revision,
            areas,
            active_area,
            active_cell,
            mode,
        })
    }
    pub fn workbook_id(&self) -> u64 {
        self.workbook_id
    }
    pub fn sheet(&self) -> u32 {
        self.sheet
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn areas(&self) -> &[SelectionArea] {
        &self.areas
    }
    pub fn active_area(&self) -> AreaId {
        self.active_area
    }
    pub fn active_cell(&self) -> CellCoord {
        self.active_cell
    }
    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// Assign the revision only after host validation accepts the candidate.
    pub fn with_next_revision(mut self) -> Result<Self, SelectionError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(SelectionError::RevisionExhausted)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOperation {
    Replace,
    Add,
    ExtendActive,
    Subtract,
    Activate(AreaId),
    SetMode(SelectionMode),
}

/// Return a candidate with the existing revision. The input stays unchanged.
/// The host checks merge closure and assigns a revision before publication.
pub fn propose_selection(
    snapshot: &SelectionSnapshot,
    operation: SelectionOperation,
    target: SelectionTarget,
) -> Result<SelectionSnapshot, SelectionError> {
    let mut areas = snapshot.areas.clone();
    let mut active_area = snapshot.active_area;
    let mut active_cell = snapshot.active_cell;
    let mut mode = snapshot.mode;
    let mut next_id = snapshot.last_area_id;
    let mut allocate = || {
        next_id = next_id
            .checked_add(1)
            .ok_or(SelectionError::IdentityExhausted)?;
        Ok(AreaId(next_id))
    };
    match operation {
        SelectionOperation::Replace | SelectionOperation::Add => {
            if operation == SelectionOperation::Replace {
                areas.clear();
            }
            let id = allocate()?;
            areas.push(SelectionArea::new(id, target));
            active_area = id;
            active_cell = target.focus;
        }
        SelectionOperation::ExtendActive => {
            let area = areas
                .iter_mut()
                .find(|area| area.id == active_area)
                .ok_or(SelectionError::InvalidActiveArea)?;
            if area.target.anchor != target.anchor {
                return Err(SelectionError::InvalidArea);
            }
            area.target = target;
            active_cell = target.focus;
        }
        SelectionOperation::Subtract => {
            let mut pieces = Vec::new();
            for area in &areas {
                let mut ranges = subtract_range(area.range(), target.range);
                // Keep the old identity with the piece nearest its moving focus.
                // Additional pieces receive fresh identities in deterministic order.
                ranges.sort_by_key(|range| nearest_key(*range, area.target.focus));
                for (index, range) in ranges.into_iter().enumerate() {
                    let id = if index == 0 { area.id } else { allocate()? };
                    let anchor = clamp_cell(area.target.anchor, range);
                    let focus = clamp_cell(area.target.focus, range);
                    let kind = kind_for_range(range);
                    pieces.push(SelectionArea::new(
                        id,
                        SelectionTarget::new(kind, anchor, focus, range)?,
                    ));
                    if pieces.len() > SELECTION_LIMIT {
                        return Err(SelectionError::AreaLimit);
                    }
                }
            }
            pieces.sort_by_key(|piece| {
                areas
                    .iter()
                    .position(|area| area.id == piece.id)
                    .unwrap_or(areas.len())
            });
            if pieces.is_empty() {
                let id = allocate()?;
                pieces.push(SelectionArea::new(id, target));
                active_area = id;
                active_cell = target.focus;
            } else {
                let old_focus = areas
                    .iter()
                    .find(|area| area.id == active_area)
                    .ok_or(SelectionError::InvalidActiveArea)?
                    .target
                    .focus;
                let chosen = pieces
                    .iter()
                    .filter(|area| area.range().contains(old_focus))
                    .min_by_key(|area| {
                        (
                            area.id != active_area,
                            area.range().first.row,
                            area.range().first.col,
                        )
                    })
                    .or_else(|| {
                        pieces
                            .iter()
                            .min_by_key(|area| nearest_key(area.range(), old_focus))
                    })
                    .ok_or(SelectionError::Empty)?;
                active_area = chosen.id;
                active_cell = clamp_cell(old_focus, chosen.range());
            }
            areas = pieces;
        }
        SelectionOperation::Activate(id) => {
            let area = areas
                .iter()
                .find(|area| area.id == id)
                .ok_or(SelectionError::InvalidActiveArea)?;
            active_area = id;
            active_cell = area.target.focus;
        }
        SelectionOperation::SetMode(value) => mode = value,
    }
    let mut candidate = SelectionSnapshot::new(
        snapshot.workbook_id,
        snapshot.sheet,
        snapshot.revision,
        areas,
        active_area,
        active_cell,
        mode,
    )?;
    candidate.last_area_id = next_id;
    Ok(candidate)
}

fn kind_for_range(range: GridRange) -> SelectionKind {
    if range.first.col == 1 && range.last.col == LAST_COLUMN {
        if range.first.row == 1 && range.last.row == LAST_ROW {
            SelectionKind::Sheet
        } else {
            SelectionKind::Rows
        }
    } else if range.first.row == 1 && range.last.row == LAST_ROW {
        SelectionKind::Columns
    } else {
        SelectionKind::Cells
    }
}

pub(crate) fn valid_cell(cell: CellCoord) -> bool {
    (1..=LAST_ROW).contains(&cell.row) && (1..=LAST_COLUMN).contains(&cell.col)
}

fn clamp_cell(cell: CellCoord, range: GridRange) -> CellCoord {
    CellCoord {
        row: cell.row.clamp(range.first.row, range.last.row),
        col: cell.col.clamp(range.first.col, range.last.col),
    }
}
fn nearest_key(range: GridRange, cell: CellCoord) -> (i64, i32, i32) {
    let nearest = clamp_cell(cell, range);
    (
        (i64::from(nearest.row) - i64::from(cell.row)).abs()
            + (i64::from(nearest.col) - i64::from(cell.col)).abs(),
        nearest.row,
        nearest.col,
    )
}

/// Split around an intersection into at most four disjoint rectangles.
pub(crate) fn subtract_range(source: GridRange, remove: GridRange) -> Vec<GridRange> {
    let source = source.normalized();
    let remove = remove.normalized();
    let top = source.first.row.max(remove.first.row);
    let bottom = source.last.row.min(remove.last.row);
    let left = source.first.col.max(remove.first.col);
    let right = source.last.col.min(remove.last.col);
    if top > bottom || left > right {
        return vec![source];
    }
    let mut parts = Vec::with_capacity(4);
    let mut push = |r1, c1, r2, c2| {
        parts.push(GridRange {
            first: CellCoord { row: r1, col: c1 },
            last: CellCoord { row: r2, col: c2 },
        })
    };
    if source.first.row < top {
        push(source.first.row, source.first.col, top - 1, source.last.col);
    }
    if bottom < source.last.row {
        push(
            bottom + 1,
            source.first.col,
            source.last.row,
            source.last.col,
        );
    }
    if source.first.col < left {
        push(top, source.first.col, bottom, left - 1);
    }
    if right < source.last.col {
        push(top, right + 1, bottom, source.last.col);
    }
    parts
}

pub(crate) fn union_ranges(ranges: impl IntoIterator<Item = GridRange>) -> Vec<GridRange> {
    let mut result = Vec::new();
    for range in ranges {
        let mut pending = vec![range.normalized()];
        for covered in &result {
            pending = pending
                .into_iter()
                .flat_map(|part| subtract_range(part, *covered))
                .collect();
        }
        result.extend(pending);
    }
    result
}

fn bounded_union(
    ranges: impl IntoIterator<Item = GridRange>,
) -> Result<Vec<GridRange>, SelectionError> {
    union_impl(ranges, SELECTION_LIMIT)
}

fn union_impl(
    ranges: impl IntoIterator<Item = GridRange>,
    limit: usize,
) -> Result<Vec<GridRange>, SelectionError> {
    let mut result = Vec::new();
    for range in ranges {
        let mut pending = vec![range.normalized()];
        for covered in &result {
            let mut next = Vec::new();
            for candidate in pending {
                next.extend(subtract_range(candidate, *covered));
                if next.len() > limit {
                    return Err(SelectionError::PieceLimit);
                }
            }
            pending = next;
        }
        if result.len().saturating_add(pending.len()) > limit {
            return Err(SelectionError::PieceLimit);
        }
        result.extend(pending);
    }
    Ok(result)
}
