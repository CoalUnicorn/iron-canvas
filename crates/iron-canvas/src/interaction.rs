//! Host intents derived only from the committed worksheet snapshot.
use crate::{CanvasSession, RenderBackend};
use iron_canvas_core::{
    CellCoord, PixelRect, Point,
    scene_geometry::{GridGeometry, GridHit, GridRange, RangeFragment, contains},
};
use iron_canvas_ironcalc::{PreparedWorksheet, WorksheetViewport};
use ironcalc_base::{links::CellLinkView, types::MergedCell};

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayCell<'a> {
    pub physical: CellCoord,
    pub anchor: CellCoord,
    pub merged: Option<MergedCell>,
    pub fragment: RangeFragment,
    pub link: Option<&'a CellLinkView>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerAction {
    Select,
    ExtendSelection,
    ActivateLink,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostIntent {
    Select {
        sheet: u32,
        anchor: CellCoord,
        range: GridRange,
        extend: bool,
    },
    ActivateLink {
        sheet: u32,
        link: CellLinkView,
    },
    ResizeRow {
        sheet: u32,
        row: i32,
        pixels: i32,
    },
    ResizeColumn {
        sheet: u32,
        column: i32,
        pixels: i32,
    },
}
impl<B: RenderBackend> CanvasSession<B> {
    pub fn hit_test(&self, point: Point) -> Option<GridHit> {
        self.committed_frame()?
            .worksheet
            .geometry
            .as_ref()?
            .hit_test(point)
    }
    pub fn display_cell_at(&self, point: Point) -> Option<DisplayCell<'_>> {
        let worksheet = &self.committed_frame()?.worksheet;
        let geometry = worksheet.geometry.as_ref()?;
        let GridHit::Cell(physical) = geometry.hit_test(point)? else {
            return None;
        };
        let merged = worksheet
            .merges
            .iter()
            .find(|m| m.contains(physical.row, physical.col))
            .copied();
        let range = merged.map_or(
            GridRange {
                first: physical,
                last: physical,
            },
            merge_range,
        );
        let fragment = geometry
            .fragments(range)
            .into_iter()
            .find(|f| contains(f.rect, point))?;
        Some(DisplayCell {
            physical,
            anchor: range.first,
            merged,
            fragment,
            link: worksheet
                .links
                .iter()
                .find(|l| l.row == range.first.row && l.column == range.first.col),
        })
    }
    pub fn visible_fragments(&self, range: GridRange) -> Vec<RangeFragment> {
        self.committed_frame()
            .and_then(|f| f.worksheet.geometry.as_ref())
            .map_or_else(Vec::new, |g| g.fragments(range))
    }
    pub fn link_at(&self, point: Point) -> Option<&CellLinkView> {
        self.display_cell_at(point)?.link
    }
    /// Slot rectangle of one cell from the committed frame, before viewport
    /// clipping. `None` before the first commit or outside the sampled panes.
    pub fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        self.committed_geometry()?.cell_rect(row, col)
    }
    /// Canvas-space rect of the scrollable pane, or `None` before the first
    /// commit. A frozen band that consumes an axis collapses that extent to 0.
    pub fn scroll_pane_rect(&self) -> Option<PixelRect> {
        Some(self.committed_geometry()?.scroll_pane_rect())
    }
    /// Bottom-right autofill-handle corner for `range`'s last cell, before
    /// viewport clipping. The host suppresses it at the model's last row/column.
    pub fn autofill_handle(&self, range: GridRange) -> Option<Point> {
        self.committed_geometry()?.autofill_handle(range)
    }
    /// The viewport of the last committed request, after a successful commit.
    pub fn committed_viewport(&self) -> Option<WorksheetViewport> {
        self.committed_request
            .as_ref()
            .map(|request| request.viewport)
    }
    fn committed_geometry(&self) -> Option<&GridGeometry> {
        self.committed_frame.as_ref()?.worksheet.geometry.as_deref()
    }
    /// Return an intent. The host decides whether and how to apply it.
    pub fn pointer_intent(&self, point: Point, action: PointerAction) -> Option<HostIntent> {
        let frame = self.committed_frame()?;
        if action == PointerAction::ActivateLink {
            return Some(HostIntent::ActivateLink {
                sheet: frame.sheet,
                link: self.link_at(point)?.clone(),
            });
        }
        let (anchor, range) = match self.hit_test(point)? {
            GridHit::Cell(_) => {
                let cell = self.display_cell_at(point)?;
                (
                    cell.anchor,
                    cell.merged.map_or(
                        GridRange {
                            first: cell.anchor,
                            last: cell.anchor,
                        },
                        merge_range,
                    ),
                )
            }
            GridHit::RowHeader(row) => {
                let first = CellCoord { row, col: 1 };
                (
                    first,
                    GridRange {
                        first,
                        last: CellCoord { row, col: 16_384 },
                    },
                )
            }
            GridHit::ColumnHeader(col) => {
                let first = CellCoord { row: 1, col };
                (
                    first,
                    GridRange {
                        first,
                        last: CellCoord {
                            row: 1_048_576,
                            col,
                        },
                    },
                )
            }
            GridHit::Corner => {
                let first = CellCoord { row: 1, col: 1 };
                (
                    first,
                    GridRange {
                        first,
                        last: CellCoord {
                            row: 1_048_576,
                            col: 16_384,
                        },
                    },
                )
            }
        };
        Some(HostIntent::Select {
            sheet: frame.sheet,
            anchor,
            range,
            extend: action == PointerAction::ExtendSelection,
        })
    }
    pub fn resize_intent(&self, point: Point, pixels: i32) -> Option<HostIntent> {
        if pixels < 0 {
            return None;
        }
        let sheet = self.committed_frame()?.sheet;
        match self.hit_test(point)? {
            GridHit::RowHeader(row) => Some(HostIntent::ResizeRow { sheet, row, pixels }),
            GridHit::ColumnHeader(column) => Some(HostIntent::ResizeColumn {
                sheet,
                column,
                pixels,
            }),
            _ => None,
        }
    }
}
pub(crate) fn merge_range(m: MergedCell) -> GridRange {
    GridRange {
        first: CellCoord {
            row: m.row,
            col: m.column,
        },
        last: CellCoord {
            row: m.last_row(),
            col: m.last_column(),
        },
    }
}
pub(crate) fn logical_range(worksheet: &PreparedWorksheet, cell: CellCoord) -> GridRange {
    worksheet
        .merges
        .iter()
        .find(|m| m.contains(cell.row, cell.col))
        .copied()
        .map_or(
            GridRange {
                first: cell,
                last: cell,
            },
            merge_range,
        )
}
