//! Read-only IronCalc worksheet compiler for resolved core scenes.
//!
//! The default `legacy` feature retains `IronCalcModel` and style converters
//! for the old renderer until M4. The shared session disables this feature.

pub mod compiler;
#[cfg(feature = "legacy")]
pub mod convert;

pub use compiler::{
    CellPlacement, CellReadOperation, PreparedWorksheet, WorksheetCompileError, WorksheetCompiler,
    WorksheetViewport,
};

#[cfg(feature = "legacy")]
mod adapter;
#[cfg(feature = "legacy")]
pub use adapter::{IronCalcModel, color_resolver};
