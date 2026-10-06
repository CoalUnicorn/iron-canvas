//! Read-only IronCalc worksheet compiler for resolved core scenes.
//!
//! The compiler reads a borrowed [`UserModel`](ironcalc_base::UserModel) and
//! lowers each cell into generic scene primitives: styles, conditional
//! formatting, borders, text, links, and merges. No legacy model adapter is
//! involved.

pub mod autofit;
pub mod compiler;

pub use compiler::{
    CellPlacement, CellReadOperation, CfDataBarPaint, CfDecorationPaint, CfIconPaint,
    CfRatingPaint, PreparedWorksheet, WorksheetCompileError, WorksheetCompiler, WorksheetViewport,
};
