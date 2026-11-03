//! Read only IronCalc worksheet compilation. No legacy model adapter is used.
mod borders;
mod cells;
#[cfg(test)]
#[path = "../tests/cells_block_evidence.rs"]
mod cells_block_evidence;
#[cfg(test)]
mod cells_block_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "../tests/cells_fill_tests.rs"]
mod cells_fill_tests;
mod cf;
mod composition;
mod glyphs;
mod spill;
mod text;
pub(crate) use text::font_css;

pub use cf::{CfDataBarPaint, CfDecorationPaint, CfIconPaint, CfRatingPaint};
pub use composition::row_header_width;

pub use iron_canvas_core::scroll::ScrollPosition;
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

/// Borrowed inputs for one worksheet compilation.
/// The host retains ownership of the workbook and theme.
/// `'a` covers these borrows. `'m` covers references inside the workbook.
#[derive(Clone, Copy)]
pub struct WorksheetContext<'a, 'm> {
    pub workbook: &'a UserModel<'m>,
    pub sheet: u32,
    pub theme: &'a CanvasTheme,
}

/// A cell and its full logical drawing rectangle. Multiple fragments can share
/// a coordinate; the compiler reads its engine content once per preparation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellPlacement {
    pub coordinate: CellCoord,
    pub bounds: PixelRect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorksheetViewport {
    pub scroll: ScrollPosition,
    pub width: i32,
    pub height: i32,
}

impl WorksheetViewport {
    /// Align the first visible cell with the pane corner.
    /// The host normalizes the cell against frozen and hidden dimensions
    /// before it submits the viewport.
    pub const fn aligned(first_cell: CellCoord, width: i32, height: i32) -> Self {
        Self {
            scroll: ScrollPosition::aligned(first_cell),
            width,
            height,
        }
    }
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
    /// Calls to `get_extended_cell_style`, including calls inside block reads.
    /// At the pinned engine revision, each block style entry counts as one call.
    pub extended_style_reads: usize,
    #[cfg(test)]
    read_stats: ReadStats,
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
    CellsBlock,
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
        context: WorksheetContext<'_, '_>,
        viewport: WorksheetViewport,
        metrics: &dyn TextMetrics,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        self.prepare_reusing_geometry(context, viewport, metrics, None)
    }

    /// Resample dimensions after a revision, but retain equal slot buffers.
    pub fn prepare_reusing_geometry(
        &self,
        context: WorksheetContext<'_, '_>,
        viewport: WorksheetViewport,
        metrics: &dyn TextMetrics,
        geometry: Option<&Rc<GridGeometry>>,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        let mut cache = TextLayoutCache::default();
        let mut shaper = TextShaper::new(metrics, TextMetricsKey::new(0, 0), &mut cache);
        self.prepare_with_shaper(
            context,
            viewport,
            &mut shaper,
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
        context: WorksheetContext<'_, '_>,
        viewport: WorksheetViewport,
        shaper: &mut TextShaper<'_>,
        reuse: WorksheetReuse<'_>,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        composition::prepare(context, viewport, shaper, reuse)
    }

    /// Lower positioned cells into five shared paint buffers. Repeated
    /// coordinates reuse one preparation-local engine record. Text stays
    /// clipped because arbitrary placements do not establish adjacent geometry.
    pub fn prepare_cells(
        &self,
        context: WorksheetContext<'_, '_>,
        clip: PixelRect,
        cells: impl IntoIterator<Item = CellPlacement>,
        metrics: &dyn TextMetrics,
    ) -> Result<PreparedWorksheet, WorksheetCompileError> {
        let mut cache = TextLayoutCache::default();
        let mut shaper = TextShaper::new(metrics, TextMetricsKey::new(0, 0), &mut cache);
        let mut attempt = Attempt::new(context, &mut shaper, None)?;
        if !valid_rect(clip) {
            return Err(attempt.error(
                CellCoord { row: 1, col: 1 },
                CellReadOperation::Request,
                "invalid viewport clip".into(),
            ));
        }
        let mut passes = new_passes();
        passes[0].rect_fill(clip, PaintColor::Borrowed(&context.theme.cell_bg));
        let mut placements = Vec::new();
        for placement in cells {
            attempt.paint(placement, None, &mut passes, false, None, false)?;
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
            #[cfg(test)]
            read_stats: attempt.read_stats,
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

/// Borrow cell data only while a consumer needs it. The view does not clone
/// styles or text, and it can later refer to shared block storage.
#[derive(Clone, Copy)]
struct CellView<'a> {
    extended: &'a ExtendedStyle,
    kind: &'a CellType,
    value: &'a str,
    occupied: bool,
}

/// One visible cell in pane order. Keep this value small: the pane stores one
/// entry for every visible cell.
enum VisibleCell {
    /// Shared block storage. `slot` is the cell index inside the block.
    Block {
        block: usize,
        slot: usize,
        kind: CellType,
        occupied: bool,
    },
    /// One individual engine read, stored in `CellRecords::visible_records`.
    Individual(usize),
}

/// Row-major cells for one pane. `base` is the first slot.
struct VisiblePane {
    base: usize,
    rows: Vec<i32>,
    columns: Vec<i32>,
}

/// Flat index of visible cells. A coordinate resolves through one pane and two
/// binary searches; paint uses slot arithmetic instead.
#[derive(Default)]
struct VisibleCells {
    cells: Vec<VisibleCell>,
    panes: Vec<VisiblePane>,
}

impl VisibleCells {
    fn locate(&self, coordinate: CellCoord) -> Option<usize> {
        for pane in &self.panes {
            let Ok(row) = pane.rows.binary_search(&coordinate.row) else {
                continue;
            };
            let Ok(column) = pane.columns.binary_search(&coordinate.col) else {
                continue;
            };
            return Some(pane.base + row * pane.columns.len() + column);
        }
        None
    }
    fn row_first(&self, pane: usize, row: usize) -> usize {
        self.panes[pane].base + row * self.panes[pane].columns.len()
    }
    fn is_block(&self, index: usize) -> bool {
        matches!(self.cells.get(index), Some(VisibleCell::Block { .. }))
    }
}

#[derive(Default)]
struct CellRecords {
    individual: HashMap<CellCoord, CellRecord>,
    /// Individual reads for visible cells outside the accepted blocks.
    visible_records: Vec<CellRecord>,
    blocks: cells::CellBlocks,
    covered: HashMap<CellCoord, cells::BlockRecord>,
    visible: VisibleCells,
}
impl CellRecords {
    // Keep lookup on cell storage so mutable shaper and paint caches remain
    // available while a consumer borrows the cell view.
    // Call Attempt::read before requesting an offscreen view.
    fn view(&self, coordinate: CellCoord) -> CellView<'_> {
        if let Some(index) = self.visible.locate(coordinate) {
            return self.view_index(index);
        }
        if let Some(record) = self.covered.get(&coordinate) {
            return self.blocks.view(record);
        }
        let record = &self.individual[&coordinate];
        CellView {
            extended: &record.extended,
            kind: &record.kind,
            value: &record.value,
            occupied: record.occupied,
        }
    }
    fn view_index(&self, index: usize) -> CellView<'_> {
        match &self.visible.cells[index] {
            VisibleCell::Block {
                block,
                slot,
                kind,
                occupied,
            } => self.blocks.view_slot(*block, *slot, kind, *occupied),
            VisibleCell::Individual(record) => {
                let record = &self.visible_records[*record];
                CellView {
                    extended: &record.extended,
                    kind: &record.kind,
                    value: &record.value,
                    occupied: record.occupied,
                }
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct ReadStats {
    block_calls: usize,
    block_cells: usize,
    block_styles: usize,
    fallback_cells: usize,
}

// Lifetimes
// 'w covers the context references and the mutable shaper borrow.
// 'm covers references inside UserModel.
// 's covers the metrics and cache references inside TextShaper.
struct Attempt<'w, 'm, 's> {
    context: WorksheetContext<'w, 'm>,
    shaper: &'w mut TextShaper<'s>,
    show_grid: bool,
    fill_paint: cells::FillPaint,
    cached: CellRecords,
    reads: usize,
    read_stats: ReadStats,
    links: Rc<[CellLinkView]>,
    lines: Vec<iron_canvas_core::text_layout::TextLine>,
    shades: cf::LightShades,
}
impl<'w, 'm, 's> Attempt<'w, 'm, 's> {
    fn new(
        context: WorksheetContext<'w, 'm>,
        shaper: &'w mut TextShaper<'s>,
        links: Option<&Rc<[CellLinkView]>>,
    ) -> Result<Self, WorksheetCompileError> {
        let WorksheetContext {
            workbook, sheet, ..
        } = context;
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
            context,
            shaper,
            show_grid,
            fill_paint: cells::FillPaint::Cells,
            cached: CellRecords::default(),
            reads: 0,
            read_stats: ReadStats::default(),
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
            sheet: self.context.sheet,
            cell,
            operation,
            message,
        }
    }
    fn load_blocks(
        &mut self,
        geometry: &GridGeometry,
        limits: cells::BlockLimits,
    ) -> Result<(), WorksheetCompileError> {
        let rectangles = cells::plan_blocks(&geometry.panes, limits).map_err(|e| {
            self.error(
                CellCoord { row: 1, col: 1 },
                CellReadOperation::CellsBlock,
                format!("invalid block plan: {e:?}"),
            )
        })?;
        for rect in rectangles {
            let error = |message| {
                self.error(
                    CellCoord {
                        row: rect.row,
                        col: rect.column,
                    },
                    CellReadOperation::CellsBlock,
                    format!("block {}x{}: {message}", rect.width, rect.height),
                )
            };
            let block = self
                .context
                .workbook
                .get_cells_block(
                    self.context.sheet,
                    rect.row,
                    rect.column,
                    rect.width,
                    rect.height,
                )
                .map_err(error)?;
            let block =
                cells::ValidatedBlock::new(rect, block).map_err(|message| error(message.into()))?;
            self.read_stats.block_calls += 1;
            self.read_stats.block_cells += block.cell_count();
            self.read_stats.block_styles += block.style_count();
            self.reads += block.style_count();
            self.cached.blocks.insert(block);
        }
        Ok(())
    }

    /// Read every visible cell once, in pane order. Paint then indexes this
    /// flat storage by slot. Offscreen spill and merge cells keep the keyed
    /// path.
    fn load_visible(&mut self, geometry: &GridGeometry) -> Result<(), WorksheetCompileError> {
        let cell_count: usize = geometry
            .panes
            .iter()
            .map(|pane| pane.rows.len() * pane.columns.len())
            .sum();
        let mut cells = Vec::with_capacity(cell_count);
        let mut panes = Vec::with_capacity(geometry.panes.len());
        for pane in &geometry.panes {
            let base = cells.len();
            for row in pane.rows.iter() {
                for column in pane.columns.iter() {
                    let coordinate = CellCoord {
                        row: row.index,
                        col: column.index,
                    };
                    if let Some((block, slot)) = self.cached.blocks.locate(coordinate) {
                        let stored = self.occupied_kind(coordinate)?;
                        let occupied = stored.is_some();
                        let kind = self
                            .cached
                            .blocks
                            .text_kind(block, slot)
                            .unwrap_or_else(|| stored.unwrap_or(CellType::Number));
                        cells.push(VisibleCell::Block {
                            block,
                            slot,
                            kind,
                            occupied,
                        });
                    } else {
                        let record = self.read_individual(coordinate)?;
                        let index = self.cached.visible_records.len();
                        self.cached.visible_records.push(record);
                        cells.push(VisibleCell::Individual(index));
                    }
                }
            }
            panes.push(VisiblePane {
                base,
                rows: pane.rows.iter().map(|row| row.index).collect(),
                columns: pane.columns.iter().map(|column| column.index).collect(),
            });
        }
        self.cached.visible = VisibleCells { cells, panes };
        Ok(())
    }

    fn read(&mut self, cell: CellCoord) -> Result<(), WorksheetCompileError> {
        if !(1..=LAST_ROW).contains(&cell.row) || !(1..=LAST_COLUMN).contains(&cell.col) {
            return Err(self.error(
                cell,
                CellReadOperation::Request,
                "invalid cell coordinate".into(),
            ));
        }
        if self.cached.visible.locate(cell).is_some()
            || self.cached.individual.contains_key(&cell)
            || self.cached.covered.contains_key(&cell)
        {
            return Ok(());
        }
        if let Some((block, slot)) = self.cached.blocks.locate(cell) {
            let stored_kind = self.occupied_kind(cell)?;
            let occupied = stored_kind.is_some();
            let kind = self
                .cached
                .blocks
                .text_kind(block, slot)
                .unwrap_or_else(|| stored_kind.unwrap_or(CellType::Number));
            self.cached.covered.insert(
                cell,
                cells::BlockRecord {
                    block,
                    slot,
                    kind,
                    occupied,
                },
            );
            return Ok(());
        }
        let record = self.read_individual(cell)?;
        self.cached.individual.insert(cell, record);
        Ok(())
    }

    // Keep this path independent of block storage. Tests use it as the reference
    // when later changes select block reads for visible cells.
    fn read_individual(&mut self, cell: CellCoord) -> Result<CellRecord, WorksheetCompileError> {
        self.reads += 1;
        self.read_stats.fallback_cells += 1;
        let style = self
            .context
            .workbook
            .get_extended_cell_style(self.context.sheet, cell.row, cell.col)
            .map_err(|e| self.error(cell, CellReadOperation::ExtendedStyle, format!("{e:?}")))?;
        let kind = self
            .context
            .workbook
            .get_cell_type(self.context.sheet, cell.row, cell.col)
            .map_err(|e| self.error(cell, CellReadOperation::CellType, e))?;
        let text = self
            .context
            .workbook
            .get_formatted_cell_value(self.context.sheet, cell.row, cell.col)
            .map_err(|e| self.error(cell, CellReadOperation::FormattedValue, e))?;
        // Display text cannot identify empty cells: formulas and number formats
        // can hide occupied values. Inspect the stored cell without changing it.
        let occupied = self.occupied(cell)?;
        Ok(CellRecord {
            extended: style,
            kind,
            value: text,
            occupied,
        })
    }
    fn paint(
        &mut self,
        placement: CellPlacement,
        slot: Option<usize>,
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
        let record = match slot {
            Some(index) => self.cached.view_index(index),
            None => {
                self.read(placement.coordinate)?;
                self.cached.view(placement.coordinate)
            }
        };
        let rect = placement.bounds;
        let style = &record.extended.style;
        let fill = self.context.workbook.resolve_color(&style.fill.color);
        // Covered ordinary cells already have one background from their run.
        // Merges still cover ordinary content in their later composition pass.
        let block_cell = match slot {
            Some(index) => self.cached.visible.is_block(index),
            None => self.cached.covered.contains_key(&placement.coordinate),
        };
        let run_fill = self.fill_paint == cells::FillPaint::Runs && block_cell;
        if merged || (!run_fill && !fill.is_empty()) {
            passes[0].rect_fill(
                rect,
                PaintColor::Borrowed(if fill.is_empty() {
                    &self.context.theme.cell_bg
                } else {
                    &fill
                }),
            );
        }
        let cf = cf::CfDecorationPaint::resolve_memoized(
            record.extended,
            self.context.workbook,
            &mut self.shades,
        );
        cf.paint(&mut passes[1], rect);
        if !merged {
            // The border pass and the grid pass are two slots of one array.
            let (grid_side, border_side) = passes.split_at_mut(3);
            borders::compile_borders(
                &mut border_side[0],
                &mut grid_side[2],
                self.context,
                &style.border,
                rect,
                borders::GridEdges {
                    top: self.show_grid && fill.is_empty(),
                    left: self.show_grid && fill.is_empty() && !hide_left_grid,
                },
            );
        }
        self.paint_text(placement, slot, &mut passes[4], spill_clip, &cf);
        Ok(())
    }

    /// Emit text without cell backgrounds. Offscreen sources use this path
    /// because their backgrounds and query rectangles are outside the pane.
    fn paint_text(
        &mut self,
        placement: CellPlacement,
        slot: Option<usize>,
        out: &mut SceneBuilder,
        spill_clip: Option<PixelRect>,
        cf: &cf::CfDecorationPaint,
    ) {
        let record = match slot {
            Some(index) => self.cached.view_index(index),
            None => self.cached.view(placement.coordinate),
        };
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
                self.context,
                text::CellTextPaint {
                    record,
                    bounds: rect,
                    reserved_left: cf.reserved_left(rect),
                    spill_clip,
                    has_link: link.is_some(),
                },
                self.shaper,
                &mut self.lines,
            );
        }
    }
}
