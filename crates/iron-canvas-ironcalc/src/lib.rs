//! Read-only IronCalc worksheet compiler for resolved core scenes.
//!
//! The compiler reads a borrowed [`UserModel`](ironcalc_base::UserModel) and
//! lowers each cell into generic scene primitives: styles, conditional
//! formatting, borders, text, links, and merges. No legacy model adapter is
//! involved.

pub mod autofit;
pub mod axis_sizes;
pub mod compiler;

pub use axis_sizes::{
    AxesScope, Axis, AxisRun, AxisSizeBuilder, AxisSizeError, AxisSizeProgress, AxisSizes,
    AxisSource, SizeRun, WorksheetAxes, WorksheetAxis, get_column_sizes, get_row_sizes,
};

pub use compiler::{
    BlockCacheIdentity, CellPlacement, CellReadOperation, CfDataBarPaint, CfDecorationPaint,
    CfIconPaint, CfRatingPaint, FrozenAxes, PreparedWorksheet, ScrollPosition, WorksheetBlockCache,
    WorksheetCompileError, WorksheetCompiler, WorksheetContext, WorksheetMetadata, WorksheetReuse,
    WorksheetViewport, row_header_width,
};
