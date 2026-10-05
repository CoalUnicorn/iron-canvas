//! `iron-canvas-core` — the pure-Rust application layer of the iron-canvas
//! grid renderer. Browser (wasm) backends and probes live in sibling crates.
//!
//! The default `legacy` feature retains the old renderer during migration.
//! Disable it to build only generic geometry, layout, and resolved scenes.

#[cfg(feature = "legacy")]
pub mod address;
#[cfg(feature = "legacy")]
pub mod chrome;
#[cfg(feature = "legacy")]
pub mod decoration;
#[cfg(feature = "legacy")]
mod frame;
pub mod geometry;
#[cfg(feature = "legacy")]
pub mod model;
#[cfg(feature = "legacy")]
mod orchestrator;
pub mod painter;
#[cfg(feature = "legacy")]
pub mod renderer;
pub mod scene;
pub mod scene_geometry;
pub mod shape;
#[cfg(feature = "legacy")]
mod style;
pub mod surface;
pub mod text_layout;
pub mod theme;

#[cfg(feature = "legacy")]
pub use frame::{
    BlitFallback, BlitPlan, FrameDelta, FrameInputFailure, FrameInputs, FrameOutcome, FrameTrace,
    GridVerdict, PaintResult, RebuildReason, RenderStrategy, Shift,
};
#[cfg(feature = "legacy")]
pub use orchestrator::{DisplayCell, Orchestrator};

#[cfg(feature = "dev-diagnostics")]
pub use renderer::diagnostics::{
    DIAG_SCHEMA_VERSION, DiagBlit, DiagBlitResultTag, DiagBufferTruth, DiagCache,
    DiagCacheActionTag, DiagCacheResolution, DiagCacheTruth, DiagChangedCell, DiagDeltaKind,
    DiagFetch, DiagFetchPurpose, DiagFetchRequest, DiagFingerprintActionTag, DiagFingerprintTruth,
    DiagGeometry, DiagPaintCounts, DiagPaintedLayers, DiagRepaint, DiagRepaintReason,
    DiagRevealedStrip, DiagSegment, DiagSourceRange, FrameDiagnostics,
};

#[cfg(feature = "legacy")]
pub use address::{AutofillTarget, FormulaRef, FormulaRefKind, RCRange, SheetArea};
#[cfg(feature = "legacy")]
pub use chrome::hit::{HitTest, RefZone, ResizeTarget};
#[cfg(feature = "legacy")]
pub use chrome::{
    ActiveCellSnapshot, BlitOutcome, Chrome, FrameKindTag, FramePath, GridLayout, GridShape,
    PaneRegion, measure_row_header_width,
};
#[cfg(feature = "legacy")]
pub use decoration::{DecorationId, Layer, RenderOverlays};
#[cfg(feature = "legacy")]
pub use frame::{RowSpan, WorkFlags};
pub use geometry::{
    CanvasMetricError, CanvasMetrics, CanvasSize,
    pixel_rect::PixelRect,
    prim::{Axis, Line, Point, RectCorner, Side, Span},
};
#[cfg(feature = "legacy")]
pub use model::autofit::AutoFitError;
#[cfg(feature = "legacy")]
pub use model::fetched::Fetched;
#[cfg(feature = "legacy")]
pub use model::sheet::links::{CellLink, LinkIndex, LinkIndexError, LinkTarget};
#[cfg(feature = "legacy")]
pub use model::sheet::merges::{MergeTable, MergeTableError, MergedRange};
#[cfg(feature = "legacy")]
pub use model::{CanvasModel, CanvasView, CellContentQuery};
#[cfg(feature = "legacy")]
pub use style::{
    Alignment, Border, BorderItem, BorderStyle, CellDecoration, CellKind, CellStyle, DataBarSpec,
    FontStyle, HAlign, IconGlyph, IconSpec, RatingSpec, RatingStyle, VAlign,
};
#[cfg(feature = "legacy")]
pub use surface::LayerBase;
pub use surface::Surface;
pub use text_layout::{
    TextLayoutOptions, TextLine, TextVerticalAlign, layout_text_into, layout_text_lines_into,
};
pub use theme::{CanvasTheme, ThemeVariables};

#[cfg(feature = "legacy")]
pub use geometry::{
    constants::{
        AUTOFILL_HANDLE_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, HEADER_COL_WIDTH,
        HEADER_ROW_HEIGHT, HEADER_SEPARATOR_WIDTH, LAST_COLUMN, LAST_ROW,
    },
    labels::col_name,
};
pub use scene_geometry::CellCoord;
