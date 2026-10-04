//! Shared worksheet rendering session and atomic frame publication boundary.
//!
//! The host owns the workbook. Rendering borrows it only while the compiler
//! prepares an owned candidate frame; backends stage that complete frame
//! before the session publishes it.

use std::fmt;

use iron_canvas_core::CellCoord;
use iron_canvas_ironcalc::{PreparedCell, WorksheetCompileError, WorksheetCompiler};
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
/// This initial session slice accepts explicit cell coordinates. Viewport
/// geometry and overlay state can be added without changing the commit rule.
#[derive(Debug, Clone, Copy)]
pub struct RenderRequest<'a> {
    pub sheet: u32,
    pub revision: RevisionToken,
    pub cells: &'a [CellCoord],
}

/// Fully prepared, owned worksheet content for a single render attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedWorksheetFrame {
    pub sequence: u64,
    pub sheet: u32,
    pub revision: RevisionToken,
    pub cells: Vec<PreparedCell>,
}

/// A backend that stages a complete candidate frame and then atomically
/// publishes it.
///
/// `stage_frame` must not alter visible backend output. `commit_frame` is
/// intentionally infallible: once staging succeeds, backend pixels and the
/// session's committed frame advance together.
pub trait RenderBackend {
    type StagedFrame;
    type Error;

    fn stage_frame(
        &mut self,
        frame: &PreparedWorksheetFrame,
    ) -> Result<Self::StagedFrame, Self::Error>;

    fn commit_frame(&mut self, staged: Self::StagedFrame);
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
            Self::Backend(error) => write!(f, "backend staging failed: {error}"),
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
    /// and does not call the backend's commit operation.
    pub fn render(
        &mut self,
        workbook: &UserModel<'_>,
        request: &RenderRequest<'_>,
    ) -> Result<RenderOutcome, RenderError<B::Error>> {
        let sequence = self.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(RenderError::SequenceExhausted)?;
        let cells = self
            .compiler
            .prepare_cells(workbook, request.sheet, request.cells.iter().copied())
            .map_err(RenderError::Preparation)?;
        let candidate = PreparedWorksheetFrame {
            sequence,
            sheet: request.sheet,
            revision: request.revision,
            cells,
        };
        let staged = self
            .backend
            .stage_frame(&candidate)
            .map_err(RenderError::Backend)?;

        self.backend.commit_frame(staged);
        let outcome = RenderOutcome {
            sequence,
            sheet: candidate.sheet,
            revision: candidate.revision,
            cell_count: candidate.cells.len(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestBackend {
        visible: Option<PreparedWorksheetFrame>,
        fail_staging: bool,
        commits: usize,
    }

    impl RenderBackend for TestBackend {
        type StagedFrame = PreparedWorksheetFrame;
        type Error = &'static str;

        fn stage_frame(
            &mut self,
            frame: &PreparedWorksheetFrame,
        ) -> Result<Self::StagedFrame, Self::Error> {
            if self.fail_staging {
                return Err("injected stage failure");
            }
            Ok(frame.clone())
        }

        fn commit_frame(&mut self, staged: Self::StagedFrame) {
            self.visible = Some(staged);
            self.commits += 1;
        }
    }

    fn workbook() -> UserModel<'static> {
        UserModel::new_empty("session-test", "en", "UTC", "en").expect("empty workbook constructs")
    }

    fn request<'a>(sheet: u32, revision: u64, cells: &'a [CellCoord]) -> RenderRequest<'a> {
        RenderRequest {
            sheet,
            revision: RevisionToken {
                workbook_id: 1,
                revision,
            },
            cells,
        }
    }

    #[test]
    fn successful_render_commits_backend_and_frame_from_same_candidate() {
        let workbook = workbook();
        let cells = [CellCoord { row: 1, col: 1 }, CellCoord { row: 1, col: 1 }];
        let mut session = CanvasSession::new(TestBackend::default());

        let outcome = session
            .render(&workbook, &request(0, 3, &cells))
            .expect("frame commits");

        assert_eq!(outcome.sequence, 1);
        assert_eq!(outcome.cell_count, 1);
        assert_eq!(
            session.committed_frame(),
            session.backend().visible.as_ref()
        );
        assert_eq!(session.backend().commits, 1);
    }

    #[test]
    fn preparation_failure_preserves_previous_frame_and_backend_output() {
        let workbook = workbook();
        let cell = [CellCoord { row: 1, col: 1 }];
        let mut session = CanvasSession::new(TestBackend::default());
        session
            .render(&workbook, &request(0, 1, &cell))
            .expect("initial frame commits");
        let committed = session.committed_frame().cloned();
        let visible = session.backend().visible.clone();
        let result = session.render(&workbook, &request(u32::MAX, 2, &cell));

        assert!(matches!(result, Err(RenderError::Preparation(_))));
        assert_eq!(session.committed_frame(), committed.as_ref());
        assert_eq!(session.backend().visible, visible);
        assert_eq!(session.backend().commits, 1);
    }

    #[test]
    fn backend_stage_failure_preserves_previous_frame_and_pixels() {
        let workbook = workbook();
        let cell = [CellCoord { row: 1, col: 1 }];
        let mut session = CanvasSession::new(TestBackend::default());
        session
            .render(&workbook, &request(0, 1, &cell))
            .expect("initial frame commits");
        let committed = session.committed_frame().cloned();
        let visible = session.backend().visible.clone();
        session.backend_mut().fail_staging = true;

        let result = session.render(&workbook, &request(0, 2, &cell));

        assert!(matches!(result, Err(RenderError::Backend(_))));
        assert_eq!(session.committed_frame(), committed.as_ref());
        assert_eq!(session.backend().visible, visible);
        assert_eq!(session.backend().commits, 1);
    }
}
