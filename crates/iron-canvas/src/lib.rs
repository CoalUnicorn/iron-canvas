//! Shared worksheet rendering session and atomic frame publication boundary.
//!
//! The host owns the workbook. Rendering borrows it only while the compiler
//! prepares an owned candidate frame; backends stage that complete frame
//! before the session publishes it.

mod interaction;
mod overlays;
mod retained;
pub use interaction::{AUTOFILL_HANDLE_PX, DisplayCell, HostIntent, PointerAction};
pub use iron_canvas_core::scene::retained::RenderStrategy;
use iron_canvas_core::scene::retained::{SceneFingerprint, SceneUpdate};
pub use overlays::{FormulaOverlay, OverlayState};

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
/// Headers are always visible. Empty overlays give a content-only export.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderRequest {
    pub sheet: u32,
    pub revision: RevisionToken,
    pub viewport: WorksheetViewport,
    pub theme: CanvasTheme,
    pub overlays: OverlayState,
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
    pub extended_style_reads: usize,
    pub reused_geometry: bool,
    pub strategy: RenderStrategy,
}

/// Owns the current backend and the last successfully committed frame.
pub struct CanvasSession<B> {
    compiler: WorksheetCompiler,
    backend: B,
    committed_frame: Option<PreparedWorksheetFrame>,
    next_sequence: u64,
    committed_request: Option<RenderRequest>,
    base: Option<PreparedWorksheet>,
    fingerprint: Option<SceneFingerprint>,
    reusable: bool,
}

impl<B: RenderBackend> CanvasSession<B> {
    pub fn new(backend: B) -> Self {
        Self {
            compiler: WorksheetCompiler::new(),
            backend,
            committed_frame: None,
            next_sequence: 1,
            committed_request: None,
            base: None,
            fingerprint: None,
            reusable: false,
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
        self.render_attempt(workbook, request, false)
    }

    /// Bypass prepared-content and pixel reuse for an independent reference frame.
    pub fn render_fresh(
        &mut self,
        workbook: &UserModel<'_>,
        request: &RenderRequest,
    ) -> Result<RenderOutcome, RenderError<B::Error>> {
        self.render_attempt(workbook, request, true)
    }

    fn render_attempt(
        &mut self,
        workbook: &UserModel<'_>,
        request: &RenderRequest,
        force: bool,
    ) -> Result<RenderOutcome, RenderError<B::Error>> {
        let sequence = self.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(RenderError::SequenceExhausted)?;
        let previous = self
            .committed_request
            .as_ref()
            .filter(|_| self.reusable && !force);
        let compatible = previous.is_some_and(|old| {
            old.sheet == request.sheet
                && old.revision.workbook_id == request.revision.workbook_id
                && old.theme == request.theme
        });
        let reuse = compatible
            && previous.is_some_and(|old| {
                old.revision == request.revision && old.viewport == request.viewport
            });
        let base = if reuse {
            self.base
                .as_ref()
                .expect("a reusable request has committed content")
                .clone()
        } else {
            self.compiler
                .prepare_reusing_geometry(
                    workbook,
                    request.sheet,
                    request.viewport,
                    self.backend.metrics(),
                    &request.theme,
                    self.base
                        .as_ref()
                        .filter(|_| compatible)
                        .and_then(|b| b.geometry.as_ref()),
                )
                .map_err(RenderError::Preparation)?
        };
        let reads = if reuse { 0 } else { base.extended_style_reads };
        let reused_geometry = self
            .base
            .as_ref()
            .and_then(|b| b.geometry.as_ref())
            .zip(base.geometry.as_ref())
            .is_some_and(|(a, b)| std::rc::Rc::ptr_eq(a, b));
        let unchanged = reuse && previous.is_some_and(|old| old.overlays == request.overlays);
        let mut worksheet = if unchanged {
            self.committed_frame
                .as_ref()
                .expect("unchanged committed frame")
                .worksheet
                .clone()
        } else {
            let mut composed = base.clone();
            overlays::compose(
                &mut composed,
                request.sheet,
                &request.overlays,
                &request.theme,
            );
            composed
        };
        worksheet.extended_style_reads = reads;
        let partial = self.backend.supports_partial_updates();
        let fingerprint = if !partial {
            None
        } else if unchanged {
            self.fingerprint.clone()
        } else {
            Some(SceneFingerprint::new(&worksheet.scene))
        };
        let mut update = SceneUpdate::Fresh;
        if let Some(scale) = self
            .backend
            .retained_scale()
            .filter(|s| s.is_finite() && *s > 0.0)
            && compatible
            && worksheet.merges.is_empty()
            && self
                .committed_frame
                .as_ref()
                .is_some_and(|f| f.worksheet.merges.is_empty())
        {
            let old = self
                .committed_frame
                .as_ref()
                .expect("compatible committed frame");
            if old.worksheet.geometry == worksheet.geometry {
                if unchanged {
                    update = SceneUpdate::Reuse;
                } else if let Some((candidate, painted)) =
                    fingerprint.as_ref().zip(self.fingerprint.as_ref())
                {
                    update = candidate.damage(painted);
                }
            } else if partial
                && let Some(plan) = retained::scroll_plan(
                    old,
                    &worksheet,
                    previous.expect("compatible request"),
                    request,
                    scale,
                )
            {
                update = plan;
            }
        }
        // Damage and blit plans are gated on `partial` above: the fingerprint
        // only exists for a backend that supports partial updates, and
        // `scroll_plan` is only reached under `partial`.
        debug_assert!(
            partial || !matches!(update, SceneUpdate::Damage(_) | SceneUpdate::Blit { .. }),
            "a backend without partial updates cannot stage damage or blit"
        );
        let strategy = update.strategy();
        let staged = self
            .backend
            .stage_update(&worksheet.scene, &update)
            .map_err(RenderError::Backend)?;
        self.backend
            .commit_frame(staged)
            .map_err(RenderError::Backend)?;
        let outcome = RenderOutcome {
            sequence,
            sheet: request.sheet,
            revision: request.revision,
            cell_count: worksheet.cells.len(),
            extended_style_reads: reads,
            reused_geometry,
            strategy,
        };
        self.committed_frame = Some(PreparedWorksheetFrame {
            sequence,
            sheet: request.sheet,
            revision: request.revision,
            worksheet,
        });
        self.base = Some(base);
        self.fingerprint = fingerprint;
        self.committed_request = Some(request.clone());
        self.reusable = true;
        self.next_sequence = next_sequence;
        Ok(outcome)
    }

    /// Invalidate prepared content after backend fonts or metrics change.
    /// The next successful render establishes new cache truth.
    pub fn invalidate(&mut self) {
        self.backend.invalidate_metrics();
        self.reusable = false;
    }

    pub fn committed_frame(&self) -> Option<&PreparedWorksheetFrame> {
        self.committed_frame.as_ref()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        self.invalidate();
        &mut self.backend
    }
}
