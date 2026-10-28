//! Shared worksheet rendering session and atomic frame publication boundary.
//!
//! The host owns the workbook. Rendering borrows it only while the compiler
//! prepares an owned candidate frame; backends stage that complete frame
//! before the session publishes it.

mod cache;
mod interaction;
use cache::CacheLookup;
pub use cache::{TextMetricsKey, WorksheetCache};
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
use iron_canvas_core::text_shaper::{TextLayoutCache, TextShaper};
use iron_canvas_ironcalc::{
    PreparedWorksheet, WorksheetCompileError, WorksheetCompiler, WorksheetContext, WorksheetReuse,
    WorksheetViewport,
};
use ironcalc_base::UserModel;

/// Host-provided workbook identity and monotonically increasing revision.
///
/// Replacing a workbook requires a new `workbook_id`, even if its revision
/// starts at zero.
/// Advance the revision after content or geometry changes, including evaluated
/// link targets, link presence, merges, and sheet insertion, deletion, or order.
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
    /// True when the shared cache supplies prepared content without compilation.
    pub content_cached: bool,
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
    metrics_key: Option<TextMetricsKey>,
}

/// Select content independently from the last frame used for pixel planning.
enum ContentSource<'a> {
    Previous,
    Cache(&'a mut WorksheetCache, TextMetricsKey),
    Fresh,
}

/// Owns the current backend and the last successfully committed frame.
pub struct CanvasSession<B> {
    compiler: WorksheetCompiler,
    backend: B,
    committed: Option<Committed>,
    next_sequence: u64,
    invalidated: bool,
    text_layouts: TextLayoutCache,
    text_metrics_key: Option<TextMetricsKey>,
}

impl<B: RenderBackend> CanvasSession<B> {
    pub fn new(backend: B) -> Self {
        Self {
            compiler: WorksheetCompiler::new(),
            backend,
            committed: None,
            next_sequence: 1,
            invalidated: false,
            text_layouts: TextLayoutCache::default(),
            text_metrics_key: None,
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
        self.render_attempt(workbook, request, ContentSource::Previous)
    }

    /// Bypass prepared-content and pixel reuse for an independent reference frame.
    pub fn render_fresh(
        &mut self,
        workbook: &UserModel<'_>,
        request: &RenderRequest,
    ) -> Result<RenderOutcome, RenderError<B::Error>> {
        self.render_attempt(workbook, request, ContentSource::Fresh)
    }

    /// Use shared pre-overlay content with the supplied metric identity.
    ///
    /// A sheet return can skip compilation, but pixel planning still compares
    /// with this session's last committed frame. Update cache entries and use
    /// order only after the backend commits. Invalidation bypasses cache hits
    /// until a frame commits successfully.
    pub fn render_cached(
        &mut self,
        cache: &mut WorksheetCache,
        workbook: &UserModel<'_>,
        request: &RenderRequest,
        metrics_key: TextMetricsKey,
    ) -> Result<RenderOutcome, RenderError<B::Error>> {
        self.render_attempt(workbook, request, ContentSource::Cache(cache, metrics_key))
    }

    fn render_attempt(
        &mut self,
        workbook: &UserModel<'_>,
        request: &RenderRequest,
        source: ContentSource<'_>,
    ) -> Result<RenderOutcome, RenderError<B::Error>> {
        let sequence = self.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(RenderError::SequenceExhausted)?;
        let metrics_key = match &source {
            ContentSource::Cache(_, key) => Some(*key),
            ContentSource::Previous | ContentSource::Fresh => None,
        };
        let force = matches!(source, ContentSource::Fresh) || self.invalidated;
        // Local metrics and explicit keys have separate identities. Track this
        // per attempt: a failed frame can still populate measured text storage.
        if force || self.text_metrics_key != metrics_key {
            self.text_layouts.clear();
        }
        self.text_metrics_key = metrics_key;
        // Fresh is the independent reference path. Disable layout retention,
        // including repeated values within this preparation.
        let mut fresh_layouts = TextLayoutCache::new(0, 0);
        let layouts = if matches!(source, ContentSource::Fresh) {
            &mut fresh_layouts
        } else {
            &mut self.text_layouts
        };
        let mut shaper = TextShaper::new(
            self.backend.metrics(),
            metrics_key.unwrap_or(TextMetricsKey::new(0, 0)),
            layouts,
        );
        let cached = self
            .committed
            .as_ref()
            .filter(|_| !force)
            .and_then(|committed| Some((committed, committed.reuse.as_ref()?)));
        let compatible = cached.filter(|(committed, reuse)| {
            committed.request.sheet == request.sheet
                && committed.request.revision.workbook_id == request.revision.workbook_id
                && committed.request.theme == request.theme
                && reuse.metrics_key == metrics_key
        });
        let reuse = compatible.filter(|(committed, _)| {
            committed.request.revision == request.revision
                && committed.request.viewport == request.viewport
        });
        // Shared content never carries another session's overlays or retained
        // fingerprint. Keep the previous frame only for composition and paint.
        let lookup = match &source {
            ContentSource::Cache(cache, key) if !force => cache.lookup(request, *key),
            ContentSource::Previous => match (reuse, compatible) {
                (Some((_, reuse)), _) => CacheLookup::Content(&reuse.base),
                (None, Some((_, reuse))) => CacheLookup::Geometry(&reuse.base),
                (None, None) => CacheLookup::Miss,
            },
            ContentSource::Cache(_, _) | ContentSource::Fresh => CacheLookup::Miss,
        };
        let donor = match lookup {
            CacheLookup::Content(base) | CacheLookup::Geometry(base) => base.geometry.as_ref(),
            CacheLookup::Miss => None,
        };
        // A viewport or metric change needs new content but can keep the full
        // sheet metadata. Use only committed data with an exact revision match.
        // Fresh, invalidation, and a disabled shared cache retain their bypass.
        let metadata = if force {
            None
        } else {
            match &source {
                ContentSource::Previous => cached
                    .filter(|(committed, _)| {
                        committed.request.sheet == request.sheet
                            && committed.request.revision == request.revision
                    })
                    .map(|(_, reuse)| &reuse.base),
                ContentSource::Cache(cache, _) => cache.metadata(request),
                ContentSource::Fresh => None,
            }
        }
        .and_then(PreparedWorksheet::metadata);
        let content_reused = matches!(lookup, CacheLookup::Content(_));
        let content_cached = content_reused && matches!(source, ContentSource::Cache(_, _));
        let base = match lookup {
            CacheLookup::Content(base) => base.clone(),
            CacheLookup::Geometry(_) | CacheLookup::Miss => self
                .compiler
                .prepare_with_shaper(
                    WorksheetContext {
                        workbook,
                        sheet: request.sheet,
                        theme: &request.theme,
                    },
                    request.viewport,
                    &mut shaper,
                    WorksheetReuse {
                        geometry: donor,
                        metadata,
                    },
                )
                .map_err(RenderError::Preparation)?,
        };
        let reads = if content_reused {
            0
        } else {
            base.extended_style_reads
        };
        let reused_geometry = donor
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
            cell_count: worksheet.cell_count(),
            extended_style_reads: reads,
            content_cached,
            reused_geometry,
            strategy,
        };
        // Publish cache truth only after the backend accepts the whole frame.
        // A failed attempt also keeps the explicit invalidation pending.
        // Clone only when an enabled shared cache needs another owner.
        // Move the original into session reuse in every case.
        if let ContentSource::Cache(cache, key) = source
            && cache.capacity() > 0
        {
            cache.publish(request, key, base.clone());
        }
        self.committed = Some(Committed {
            frame: PreparedWorksheetFrame {
                sequence,
                sheet: request.sheet,
                revision: request.revision,
                worksheet,
            },
            request: request.clone(),
            reuse: Some(Reuse {
                base,
                fingerprint,
                metrics_key,
            }),
        });
        self.next_sequence = next_sequence;
        self.invalidated = false;
        Ok(outcome)
    }

    /// Invalidate prepared content after backend fonts or metrics change.
    /// The frame stays published; the next render rebuilds it.
    pub fn invalidate(&mut self) {
        self.backend.invalidate_metrics();
        self.text_layouts.clear();
        self.invalidated = true;
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
