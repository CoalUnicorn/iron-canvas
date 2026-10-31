//! Navigation reads compare committed geometry with the host's current request.

use crate::interaction::{LAST_COLUMN, LAST_ROW};
use crate::selection::valid_cell;
use crate::{CanvasSession, RenderBackend, RenderRequest, RevisionToken};
use iron_canvas_core::{CellCoord, scene_geometry::GridGeometry};
use iron_canvas_ironcalc::{FrozenAxes, WorksheetViewport};
use ironcalc_base::UserModel;

/// Identity of committed interaction data. A host separately proves presentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteractionFrame {
    pub sequence: u64,
    pub sheet: u32,
    pub revision: RevisionToken,
    pub viewport: WorksheetViewport,
    pub selection_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationPending {
    NoFrame,
    Invalidated,
    Workbook,
    Sheet,
    Revision,
    Size,
    Theme,
    Geometry,
}

#[derive(Debug)]
pub struct NavigationGeometry<'a> {
    pub frame: InteractionFrame,
    pub geometry: &'a GridGeometry,
    pub frozen: FrozenAxes,
}

#[derive(Debug)]
pub enum NavigationReadiness<'a> {
    Ready(NavigationGeometry<'a>),
    Pending(NavigationPending),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityAdjustment {
    Unchanged,
    ScrollTo(CellCoord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtentAxis {
    Row,
    Column,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationQueryError {
    Bounds,
    Pending(NavigationPending),
    Extent {
        sheet: u32,
        axis: ExtentAxis,
        index: i32,
        message: String,
    },
}
impl std::fmt::Display for NavigationQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "navigation query failed: {self:?}")
    }
}
impl std::error::Error for NavigationQueryError {}

impl<B: RenderBackend> CanvasSession<B> {
    pub fn interaction_frame(&self) -> Option<InteractionFrame> {
        let committed = self.committed.as_ref()?;
        Some(InteractionFrame {
            sequence: committed.frame.sequence,
            sheet: committed.frame.sheet,
            revision: committed.frame.revision,
            viewport: committed.request.viewport,
            selection_revision: committed
                .request
                .overlays
                .selection_snapshot
                .as_ref()
                .map(|selection| selection.revision()),
        })
    }

    pub fn navigation_geometry(&self, expected: &RenderRequest) -> NavigationReadiness<'_> {
        let Some(committed) = self.committed.as_ref() else {
            return NavigationReadiness::Pending(NavigationPending::NoFrame);
        };
        let old = &committed.request;
        let pending = if self.invalidated {
            Some(NavigationPending::Invalidated)
        } else if old.revision.workbook_id != expected.revision.workbook_id {
            Some(NavigationPending::Workbook)
        } else if old.sheet != expected.sheet {
            Some(NavigationPending::Sheet)
        } else if old.revision != expected.revision {
            Some(NavigationPending::Revision)
        } else if old.viewport.width != expected.viewport.width
            || old.viewport.height != expected.viewport.height
        {
            Some(NavigationPending::Size)
        } else if old.theme != expected.theme {
            Some(NavigationPending::Theme)
        } else {
            None
        };
        if let Some(reason) = pending {
            return NavigationReadiness::Pending(reason);
        }
        match (
            committed.frame.worksheet.geometry.as_deref(),
            self.interaction_frame(),
        ) {
            (Some(geometry), Some(frame)) => NavigationReadiness::Ready(NavigationGeometry {
                frame,
                geometry,
                frozen: committed.frame.worksheet.frozen,
            }),
            _ => NavigationReadiness::Pending(NavigationPending::Geometry),
        }
    }

    /// Start visibility calculations from the expected live origin, not the old
    /// painted origin. The host supplies a model that matches the expected token.
    pub fn scroll_to_show_checked(
        &self,
        workbook: &UserModel<'_>,
        expected: &RenderRequest,
        target: CellCoord,
    ) -> Result<VisibilityAdjustment, NavigationQueryError> {
        if !valid_cell(target) || !valid_cell(expected.viewport.scroll.first_cell) {
            return Err(NavigationQueryError::Bounds);
        }
        let ready = match self.navigation_geometry(expected) {
            NavigationReadiness::Ready(ready) => ready,
            NavigationReadiness::Pending(reason) => {
                return Err(NavigationQueryError::Pending(reason));
            }
        };
        let pane = ready.geometry.scroll_pane_rect();
        let requested = expected.viewport.scroll.first_cell;
        let origin = CellCoord {
            row: requested.row.max((ready.frozen.rows + 1).min(LAST_ROW)),
            col: requested.col.max((ready.frozen.cols + 1).min(LAST_COLUMN)),
        };
        let read = |axis, index| {
            let result = match axis {
                ExtentAxis::Row => workbook.get_row_height(expected.sheet, index),
                ExtentAxis::Column => workbook.get_column_width(expected.sheet, index),
            };
            let error = |message| NavigationQueryError::Extent {
                sheet: expected.sheet,
                axis,
                index,
                message,
            };
            let value = result.map_err(error)?;
            if !value.is_finite() || value < 0.0 || value.round() > f64::from(i32::MAX) {
                return Err(error("invalid axis extent".into()));
            }
            Ok(value.round() as i32)
        };
        let row = checked_origin(
            target.row,
            origin.row,
            ready.frozen.rows,
            pane.height,
            |index| read(ExtentAxis::Row, index),
        )?;
        let col = checked_origin(
            target.col,
            origin.col,
            ready.frozen.cols,
            pane.width,
            |index| read(ExtentAxis::Column, index),
        )?;
        let adjusted = CellCoord { row, col };
        Ok(if adjusted == requested {
            VisibilityAdjustment::Unchanged
        } else {
            VisibilityAdjustment::ScrollTo(adjusted)
        })
    }
}

fn checked_origin(
    target: i32,
    current: i32,
    frozen: i32,
    extent: i32,
    mut read: impl FnMut(i32) -> Result<i32, NavigationQueryError>,
) -> Result<i32, NavigationQueryError> {
    if extent <= 0 || target <= frozen {
        return Ok(current);
    }
    if target < current {
        return Ok(target);
    }
    let mut first = target;
    let mut total = i64::from(read(target)?);
    while first > frozen + 1 {
        let previous = i64::from(read(first - 1)?);
        if total + previous > i64::from(extent) {
            break;
        }
        first -= 1;
        total += previous;
    }
    Ok(current.max(first))
}
