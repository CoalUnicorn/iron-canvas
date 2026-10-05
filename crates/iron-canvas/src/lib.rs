//! Shared worksheet rendering session and atomic frame publication boundary.
//!
//! The host owns the workbook. Rendering borrows it only while the compiler
//! prepares an owned candidate frame; backends stage that complete frame
//! before the session publishes it.

use std::fmt;

use iron_canvas_core::CanvasTheme;
pub use iron_canvas_core::scene::RenderBackend;
use iron_canvas_ironcalc::{
    PreparedWorksheet, WorksheetCompileError, WorksheetCompiler, WorksheetViewport,
};
use ironcalc_base::UserModel;

/// Host-provided workbook identity and monotonically increasing revision.
///
/// Replacing a workbook requires a new `workbook_id`, even if its revision
/// starts at zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RevisionToken {
    pub workbook_id: u64,
    pub revision: u64,
}

/// The worksheet content requested for one complete frame.
///
/// This slice renders a Fresh cell viewport. Worksheet composition is added
/// without changing the commit rule.
#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub sheet: u32,
    pub revision: RevisionToken,
    pub viewport: WorksheetViewport,
    pub theme: CanvasTheme,
}

/// Fully prepared, owned worksheet content for a single render attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedWorksheetFrame {
    pub sequence: u64,
    pub sheet: u32,
    pub revision: RevisionToken,
    pub worksheet: PreparedWorksheet,
}

/// An error that prevented the candidate frame from being committed.
#[derive(Debug)]
pub enum RenderError<E> {
    Preparation(WorksheetCompileError),
    Backend(E),
    SequenceExhausted,
}

impl<E: fmt::Display> fmt::Display for RenderError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preparation(error) => write!(f, "worksheet preparation failed: {error}"),
            Self::Backend(error) => write!(f, "backend frame failed: {error}"),
            Self::SequenceExhausted => f.write_str("render frame sequence exhausted"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for RenderError<E> {}

/// Summary of a successful frame commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOutcome {
    pub sequence: u64,
    pub sheet: u32,
    pub revision: RevisionToken,
    pub cell_count: usize,
}

/// Owns the current backend and the last successfully committed frame.
pub struct CanvasSession<B> {
    compiler: WorksheetCompiler,
    backend: B,
    committed_frame: Option<PreparedWorksheetFrame>,
    next_sequence: u64,
}

impl<B: RenderBackend> CanvasSession<B> {
    pub fn new(backend: B) -> Self {
        Self {
            compiler: WorksheetCompiler::new(),
            backend,
            committed_frame: None,
            next_sequence: 1,
        }
    }

    /// Prepare and stage a full candidate before publishing either backend
    /// output or queryable frame data.
    ///
    /// Any preparation or staging error leaves `committed_frame` unchanged
    /// and does not call the backend's commit operation. A failed backend
    /// commit also preserves the previous frame under the backend contract.
    pub fn render(
        &mut self,
        workbook: &UserModel<'_>,
        request: &RenderRequest,
    ) -> Result<RenderOutcome, RenderError<B::Error>> {
        let sequence = self.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(RenderError::SequenceExhausted)?;
        let worksheet = self
            .compiler
            .prepare(
                workbook,
                request.sheet,
                request.viewport,
                self.backend.metrics(),
                &request.theme,
            )
            .map_err(RenderError::Preparation)?;
        let candidate = PreparedWorksheetFrame {
            sequence,
            sheet: request.sheet,
            revision: request.revision,
            worksheet,
        };
        let staged = self
            .backend
            .stage_frame(&candidate.worksheet.scene)
            .map_err(RenderError::Backend)?;

        self.backend
            .commit_frame(staged)
            .map_err(RenderError::Backend)?;
        let outcome = RenderOutcome {
            sequence,
            sheet: candidate.sheet,
            revision: candidate.revision,
            cell_count: candidate.worksheet.cells.len(),
        };
        self.committed_frame = Some(candidate);
        self.next_sequence = next_sequence;
        Ok(outcome)
    }

    pub fn committed_frame(&self) -> Option<&PreparedWorksheetFrame> {
        self.committed_frame.as_ref()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}
