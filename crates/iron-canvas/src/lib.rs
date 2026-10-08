//! Shared worksheet rendering session and atomic frame publication boundary.
//!
//! The host owns the workbook. Rendering borrows it only while the compiler
//! prepares an owned candidate frame; backends stage that complete frame
//! before the session publishes it.

mod interaction;
mod overlays;
mod retained;
pub use interaction::{
    AUTOFILL_HANDLE_PX, DisplayCell, FormulaRefHit, HostIntent, PointerAction, RefCorner, RefZone,
};
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

/// The published frame with the data a later render can reuse.
///
/// One commit establishes these fields together. `invalidate` clears only
/// `reuse`, because frame queries still describe the published frame.
struct Committed {
    frame: PreparedWorksheetFrame,
    /// The request that produced `frame`.
    request: RenderRequest,
    /// Present while a later render can skip preparation.
    reuse: Option<Reuse>,
}

/// Prepared content kept for a repeat of the same request.
struct Reuse {
    base: PreparedWorksheet,
    fingerprint: Option<SceneFingerprint>,
}

/// Owns the current backend and the last successfully committed frame.
pub struct CanvasSession<B> {
    compiler: WorksheetCompiler,
    backend: B,
    committed: Option<Committed>,
    next_sequence: u64,
}

impl<B: RenderBackend> CanvasSession<B> {
    pub fn new(backend: B) -> Self {
        Self {
            compiler: WorksheetCompiler::new(),
            backend,
            committed: None,
            next_sequence: 1,
        }
    }

    /// Prepare and stage a full candidate before publishing either backend
    /// output or queryable frame data.
    ///
    /// Any preparation or staging error leaves the committed frame unchanged
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
        let cached = self
            .committed
            .as_ref()
            .filter(|_| !force)
            .and_then(|committed| Some((committed, committed.reuse.as_ref()?)));
        let compatible = cached.filter(|(committed, _)| {
            committed.request.sheet == request.sheet
                && committed.request.revision.workbook_id == request.revision.workbook_id
                && committed.request.theme == request.theme
        });
        let reuse = compatible.filter(|(committed, _)| {
            committed.request.revision == request.revision
                && committed.request.viewport == request.viewport
        });
        let base = match reuse {
            Some((_, reuse)) => reuse.base.clone(),
            None => self
                .compiler
                .prepare_reusing_geometry(
                    workbook,
                    request.sheet,
                    request.viewport,
                    self.backend.metrics(),
                    &request.theme,
                    compatible.and_then(|(_, reuse)| reuse.base.geometry.as_ref()),
                )
                .map_err(RenderError::Preparation)?,
        };
        let reads = if reuse.is_some() {
            0
        } else {
            base.extended_style_reads
        };
        let reused_geometry = compatible
            .and_then(|(_, reuse)| reuse.base.geometry.as_ref())
            .zip(base.geometry.as_ref())
            .is_some_and(|(a, b)| std::rc::Rc::ptr_eq(a, b));
        let unchanged =
            reuse.filter(|(committed, _)| committed.request.overlays == request.overlays);
        let mut worksheet = match unchanged {
            Some((committed, _)) => committed.frame.worksheet.clone(),
            None => {
                let mut composed = base.clone();
                overlays::compose(
                    &mut composed,
                    request.sheet,
                    &request.overlays,
                    &request.theme,
                );
                composed
            }
        };
        worksheet.extended_style_reads = reads;
        let partial = self.backend.supports_partial_updates();
        let fingerprint = if !partial {
            None
        } else if let Some((_, reuse)) = unchanged {
            reuse.fingerprint.clone()
        } else {
            Some(SceneFingerprint::new(&worksheet.scene))
        };
        let mut update = SceneUpdate::Fresh;
        if let Some(scale) = self
            .backend
            .retained_scale()
            .filter(|s| s.is_finite() && *s > 0.0)
            && let Some((committed, reuse)) = compatible
            && worksheet.merges.is_empty()
            && committed.frame.worksheet.merges.is_empty()
        {
            if committed.frame.worksheet.geometry == worksheet.geometry {
                if unchanged.is_some() {
                    update = SceneUpdate::Reuse;
                } else if let Some((candidate, painted)) =
                    fingerprint.as_ref().zip(reuse.fingerprint.as_ref())
                {
                    update = candidate.damage(painted);
                }
            } else if partial
                && let Some(plan) = retained::scroll_plan(
                    &committed.frame,
                    &worksheet,
                    &committed.request,
                    request,
                    scale,
                )
            {
                update = plan;
            }
        }
        // Both plans need `partial`; see the fingerprint above.
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
        self.committed = Some(Committed {
            frame: PreparedWorksheetFrame {
                sequence,
                sheet: request.sheet,
                revision: request.revision,
                worksheet,
            },
            request: request.clone(),
            reuse: Some(Reuse { base, fingerprint }),
        });
        self.next_sequence = next_sequence;
        Ok(outcome)
    }

    /// Invalidate prepared content after backend fonts or metrics change.
    /// The frame stays published; the next render rebuilds it.
    pub fn invalidate(&mut self) {
        self.backend.invalidate_metrics();
        if let Some(committed) = &mut self.committed {
            committed.reuse = None;
        }
    }

    pub fn committed_frame(&self) -> Option<&PreparedWorksheetFrame> {
        self.committed.as_ref().map(|committed| &committed.frame)
    }

    pub(crate) fn committed_request(&self) -> Option<&RenderRequest> {
        self.committed.as_ref().map(|committed| &committed.request)
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        self.invalidate();
        &mut self.backend
    }
}
