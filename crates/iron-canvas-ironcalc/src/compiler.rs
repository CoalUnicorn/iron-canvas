//! Read only IronCalc worksheet compilation. No legacy model adapter is used.
mod borders;
mod cf;
mod composition;
mod glyphs;
mod spill;
mod text;
pub(crate) use text::font_css;

pub use cf::{CfDataBarPaint, CfDecorationPaint, CfIconPaint, CfRatingPaint};

use iron_canvas_core::{CanvasTheme, CellCoord, PixelRect};
use iron_canvas_core::{
    painter::{PaintColor, TextMetrics},
    scene::{GridScene, SceneBuilder},
    scene_geometry::GridGeometry,
    text_shaper::{TextLayoutCache, TextMetricsKey, TextShaper},
};
use ironcalc_base::{
    UserModel,
    cf_types::ExtendedStyle,
    links::CellLinkView,
    types::{CellType, MergedCell},
};
use std::rc::Rc;
use std::{collections::HashMap, fmt};

// The pinned engine keeps these limits private. Keep the worksheet limits
// here, outside the generic core geometry.
const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;

/// A cell and its full logical drawing rectangle. Multiple fragments can share
/// a coordinate; the compiler reads its engine content once per preparation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellPlacement {
    pub coordinate: CellCoord,
    pub bounds: PixelRect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorksheetViewport {
    pub first_cell: CellCoord,
    pub width: i32,
    pub height: i32,
}

/// Frozen row and column counts of the compiled viewport. Query paths use
/// them instead of re-reading the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrozenAxes {
    pub rows: i32,
    pub cols: i32,
}

/// Worksheet content and query data from [`WorksheetCompiler`].
/// Use [`Self::cell_count`] and [`Self::cells`] to inspect placements.
/// Regular viewports derive placements from [`Self::geometry`].
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedWorksheet {
    pub scene: GridScene,
    // Arbitrary rectangles cannot be recovered from grid axes. Regular
    // viewports use geometry alone and do not allocate this slice.
    explicit_cells: Option<Rc<[CellPlacement]>>,
    pub geometry: Option<Rc<GridGeometry>>,
    pub frozen: FrozenAxes,
    pub merges: Rc<[MergedCell]>,
    pub links: Rc<[CellLinkView]>,
    /// Actual calls to `get_extended_cell_style` in this preparation.
    pub extended_style_reads: usize,
}

/// Borrowed complete link and merge lists from a prepared viewport.
/// Reuse only for the same workbook, sheet, and content revision.
/// Keep the source lists unchanged after compilation.
#[derive(Debug, Clone, Copy)]
pub struct WorksheetMetadata<'a> {
    links: &'a Rc<[CellLinkView]>,
    merges: &'a Rc<[MergedCell]>,
}

/// Optional inputs from an earlier compilation. The default reuses neither input.
/// Geometry and metadata have separate validity rules.
#[derive(Debug, Default, Clone, Copy)]
pub struct WorksheetReuse<'a> {
    /// Resample dimensions and reuse this geometry only when it is equal.
    pub geometry: Option<&'a Rc<GridGeometry>>,
    /// Supply complete lists for the same workbook, sheet, and content revision.
    /// Advance the revision after evaluation changes links or after merges change.
    /// Use `None` when the revision is unknown.
    pub metadata: Option<WorksheetMetadata<'a>>,
}

impl PreparedWorksheet {
    /// Borrow metadata for another viewport at the same content revision.
    /// Explicit cell preparation has no complete merge list and returns `None`.
    pub fn metadata(&self) -> Option<WorksheetMetadata<'_>> {
        self.explicit_cells.is_none().then_some(WorksheetMetadata {
            links: &self.links,
            merges: &self.merges,
        })
    }

    /// Return the number of placements, including repeated explicit coordinates.
    /// Exclude additional cells read for text spill and merge anchors.
    pub fn cell_count(&self) -> usize {
        if let Some(cells) = &self.explicit_cells {
            cells.len()
        } else {
            self.geometry
                .iter()
                .flat_map(|geometry| &geometry.panes)
                .map(|pane| pane.rows.len() * pane.columns.len())
                .sum()
        }
    }

    /// Return placements by value without allocating a list.
    ///
    /// Regular viewports use pane order, then row order, then column order.
    /// Bounds include the full cell, even where the pane clips it.
    /// Explicit placements keep their input order and rectangles.
    /// Exclude additional cells read for text spill and merge anchors.
    pub fn cells(&self) -> impl Iterator<Item = CellPlacement> + '_ {
        let explicit = self
            .explicit_cells
            .iter()
            .flat_map(|cells| cells.iter())
            .copied();
        let regular = self
            .geometry
            .iter()
            .filter(|_| self.explicit_cells.is_none())
            .flat_map(|geometry| &geometry.panes)
            .flat_map(|pane| {
                pane.rows.iter().flat_map(move |row| {
                    pane.columns.iter().map(move |col| CellPlacement {
                        coordinate: CellCoord {
                            row: row.index,
                            col: col.index,
                        },
                        bounds: composition::cell_rect(row, col),
                    })
                })
            });
        explicit.chain(regular)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellReadOperation {
    Request,
    Sheet,
    Freeze,
    Merges,
    Links,
    RowHeight,
    ColumnWidth,
    ExtendedStyle,
    CellType,
    FormattedValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorksheetCompileError {
    pub sheet: u32,
    pub cell: CellCoord,
    pub operation: CellReadOperation,
    pub message: String,
}
impl fmt::Display for WorksheetCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to read {:?} for sheet {}, cell ({}, {}): {}",
            self.operation, self.sheet, self.cell.row, self.cell.col, self.message
        )
    }
}
impl std::error::Error for WorksheetCompileError {}

#[derive(Debug, Default, Clone, Copy)]
pub struct WorksheetCompiler;
impl WorksheetCompiler {
    pub const fn new() -> Self {
        Self
    }

    /// Compose a complete worksheet viewport with visible headers and frozen panes.
    pub fn prepare(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        viewport: WorksheetViewport,
        metrics: &dyn TextMetrics,
        theme: &CanvasTheme,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        self.prepare_reusing_geometry(workbook, sheet, viewport, metrics, theme, None)
    }

    /// Resample dimensions after a revision, but retain equal slot buffers.
    pub fn prepare_reusing_geometry(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        viewport: WorksheetViewport,
        metrics: &dyn TextMetrics,
        theme: &CanvasTheme,
        geometry: Option<&Rc<GridGeometry>>,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        let mut cache = TextLayoutCache::default();
        let mut shaper = TextShaper::new(metrics, TextMetricsKey::new(0, 0), &mut cache);
        self.prepare_with_shaper(
            workbook,
            sheet,
            viewport,
            &mut shaper,
            theme,
            WorksheetReuse {
                geometry,
                ..WorksheetReuse::default()
            },
        )
    }

    /// Prepare with caller-owned text storage and optional geometry and metadata reuse.
    /// Bind the shaper to the same metrics that the backend uses to paint.
    /// Use [`WorksheetReuse::default`] for an independent compilation.
    /// Check the validity rules on [`WorksheetReuse`] before supplying prior data.
    pub fn prepare_with_shaper(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        viewport: WorksheetViewport,
        shaper: &mut TextShaper<'_>,
        theme: &CanvasTheme,
        reuse: WorksheetReuse<'_>,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        composition::prepare(workbook, sheet, viewport, shaper, theme, reuse)
    }

    /// Lower positioned cells into five shared paint buffers. Repeated
    /// coordinates reuse one preparation-local engine record. Text stays
    /// clipped because arbitrary placements do not establish adjacent geometry.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_cells(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        clip: PixelRect,
        cells: impl IntoIterator<Item = CellPlacement>,
        metrics: &dyn TextMetrics,
        theme: &CanvasTheme,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        let mut cache = TextLayoutCache::default();
        let mut shaper = TextShaper::new(metrics, TextMetricsKey::new(0, 0), &mut cache);
        let mut attempt = Attempt::new(workbook, sheet, &mut shaper, theme, None)?;
        if !valid_rect(clip) {
            return Err(attempt.error(
                CellCoord { row: 1, col: 1 },
                CellReadOperation::Request,
                "invalid viewport clip".into(),
            ));
        }
        let mut passes = new_passes();
        passes[0].rect_fill(clip, PaintColor::Borrowed(&theme.cell_bg));
        let mut placements = Vec::new();
        for placement in cells {
            attempt.paint(placement, &mut passes, false, None, false)?;
            placements.push(placement);
        }
        Ok(PreparedWorksheet {
            scene: GridScene {
                clip,
                passes: passes.into_iter().map(SceneBuilder::finish).collect(),
            },
            explicit_cells: Some(placements.into()),
            geometry: None,
            frozen: FrozenAxes::default(),
            merges: Rc::from([]),
            links: attempt.links,
            extended_style_reads: attempt.reads,
        })
    }
}

fn valid_rect(rect: PixelRect) -> bool {
    rect.width >= 0
        && rect.height >= 0
        && rect.left().checked_add(rect.width).is_some()
        && rect.top().checked_add(rect.height).is_some()
}
fn new_passes() -> [SceneBuilder; 5] {
    std::array::from_fn(|_| SceneBuilder::default())
}
/// The engine reads one painted cell needs.
struct CellRecord {
    extended: ExtendedStyle,
    kind: CellType,
    value: String,
    occupied: bool,
}

// Lifetimes
// 'w borrows the workbook, text shaper, and theme.
// 'm references inside UserModel.
// 's borrows the text shaper.
struct Attempt<'w, 'm, 's> {
    workbook: &'w UserModel<'m>,
    sheet: u32,
    shaper: &'w mut TextShaper<'s>,
    theme: &'w CanvasTheme,
    show_grid: bool,
    cached: HashMap<CellCoord, CellRecord>,
    reads: usize,
    links: Rc<[CellLinkView]>,
    lines: Vec<iron_canvas_core::text_layout::TextLine>,
    shades: cf::LightShades,
}
impl<'w, 'm, 's> Attempt<'w, 'm, 's> {
    fn new(
        workbook: &'w UserModel<'m>,
        sheet: u32,
        shaper: &'w mut TextShaper<'s>,
        theme: &'w CanvasTheme,
        links: Option<&Rc<[CellLinkView]>>,
    ) -> Result<Self, WorksheetCompileError> {
        let err = |operation, message| WorksheetCompileError {
            sheet,
            cell: CellCoord { row: 1, col: 1 },
            operation,
            message,
        };
        let show_grid = workbook
            .get_show_grid_lines(sheet)
            .map_err(|e| err(CellReadOperation::Sheet, e))?;
        let links = match links {
            Some(links) => Rc::clone(links),
            None => workbook
                .get_links_list(sheet)
                .map_err(|e| err(CellReadOperation::Links, e))?
                .into(),
        };
        Ok(Self {
            workbook,
            sheet,
            shaper,
            theme,
            show_grid,
            cached: HashMap::new(),
            reads: 0,
            links,
            lines: Vec::new(),
            shades: cf::LightShades::default(),
        })
    }
    fn error(
        &self,
        cell: CellCoord,
        operation: CellReadOperation,
        message: String,
    ) -> WorksheetCompileError {
        WorksheetCompileError {
            sheet: self.sheet,
            cell,
            operation,
            message,
        }
    }
    fn read(&mut self, cell: CellCoord) -> Result<(), WorksheetCompileError> {
        if !(1..=LAST_ROW).contains(&cell.row) || !(1..=LAST_COLUMN).contains(&cell.col) {
            return Err(self.error(
                cell,
                CellReadOperation::Request,
                "invalid cell coordinate".into(),
            ));
        }
        if self.cached.contains_key(&cell) {
            return Ok(());
        }
        self.reads += 1;
        let style = self
            .workbook
            .get_extended_cell_style(self.sheet, cell.row, cell.col)
            .map_err(|e| self.error(cell, CellReadOperation::ExtendedStyle, format!("{e:?}")))?;
        let kind = self
            .workbook
            .get_cell_type(self.sheet, cell.row, cell.col)
            .map_err(|e| self.error(cell, CellReadOperation::CellType, e))?;
        let text = self
            .workbook
            .get_formatted_cell_value(self.sheet, cell.row, cell.col)
            .map_err(|e| self.error(cell, CellReadOperation::FormattedValue, e))?;
        // Display text cannot identify empty cells: formulas and number formats
        // can hide occupied values. Inspect the stored cell without changing it.
        let occupied = self.occupied(cell)?;
        self.cached.insert(
            cell,
            CellRecord {
                extended: style,
                kind,
                value: text,
                occupied,
            },
        );
        Ok(())
    }
    fn paint(
        &mut self,
        placement: CellPlacement,
        passes: &mut [SceneBuilder; 5],
        merged: bool,
        spill_clip: Option<PixelRect>,
        hide_left_grid: bool,
    ) -> Result<(), WorksheetCompileError> {
        if !valid_rect(placement.bounds) {
            return Err(self.error(
                placement.coordinate,
                CellReadOperation::Request,
                "invalid cell placement".into(),
            ));
        }
        self.read(placement.coordinate)?;
        let record = &self.cached[&placement.coordinate];
        let rect = placement.bounds;
        let style = &record.extended.style;
        let fill = self.workbook.resolve_color(&style.fill.color);
        if merged || !fill.is_empty() {
            passes[0].rect_fill(
                rect,
                PaintColor::Borrowed(if fill.is_empty() {
                    &self.theme.cell_bg
                } else {
                    &fill
                }),
            );
        }
        let cf = cf::CfDecorationPaint::resolve_memoized(
            &record.extended,
            self.workbook,
            &mut self.shades,
        );
        cf.paint(&mut passes[1], rect);
        if !merged {
            // The border pass and the grid pass are two slots of one array.
            let (grid_side, border_side) = passes.split_at_mut(3);
            borders::compile_borders(
                &mut border_side[0],
                &mut grid_side[2],
                self.workbook,
                &style.border,
                rect,
                self.show_grid && fill.is_empty(),
                hide_left_grid,
                self.theme,
            );
        }
        self.paint_text(placement, &mut passes[4], spill_clip, &cf);
        Ok(())
    }

    /// Emit text without cell backgrounds. Offscreen sources use this path
    /// because their backgrounds and query rectangles are outside the pane.
    fn paint_text(
        &mut self,
        placement: CellPlacement,
        out: &mut SceneBuilder,
        spill_clip: Option<PixelRect>,
        cf: &cf::CfDecorationPaint,
    ) {
        let record = &self.cached[&placement.coordinate];
        let rect = placement.bounds;
        if !cf.hides_value() {
            let link = self
                .links
                .binary_search_by_key(&(placement.coordinate.row, placement.coordinate.col), |l| {
                    (l.row, l.column)
                })
                .ok()
                .map(|index| &self.links[index]);
            text::compile_text(
                out,
                self.workbook,
                &record.extended.style,
                &record.kind,
                &record.value,
                rect,
                cf.reserved_left(rect),
                spill_clip,
                self.shaper,
                self.theme,
                link,
                &mut self.lines,
            );
        }
    }
}
