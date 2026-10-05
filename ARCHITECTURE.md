<!-- last-verified-against: a5a7230 (2026-08-14) -->
<!-- working-tree-verified: 2026-09-26; edited: 2026-10-04 -->
<!-- covers: iron-canvas/crates/ iron-canvas/Cargo.toml iron-canvas/web-test/Makefile .github/workflows/test.yml -->

# iron-canvas architecture

## M3 measured reuse (2026-10-05)

`CanvasSession` retains the committed request, prepared base content, and
paint fingerprints. Unchanged requests reuse content without engine reads.
Overlay changes reuse the base content. A workbook revision still recompiles
all visible content. This includes CF changes caused by offscreen edits.
The compiler resamples dimensions, then shares equal slot geometry.

Core hashes resolved primitives and effective clips. It builds full-width
32-pixel damage bands in one traversal. Canvas2D stages damaged bands on a
detached surface. A vertical scroll can copy a verified interior overlap.
The scroll must keep the revision, columns, theme, and empty overlay state.
It must have no frozen panes or merges. Movement and copy edges must land on
whole device pixels. The backend paints exposed regions under one union clip.
It recycles the detached staging surface after successful presentation.
Other scrolls use Fresh. Full-surface damage also uses Fresh.

Pixels, query metadata, base content, and fingerprints commit together.
A stage or commit failure keeps the previous frame and cache truth.
`render_fresh` bypasses reuse for reference checks. `RenderOutcome` reports
the effective strategy, extended-style reads, and geometry reuse.

Merges use Fresh raster rendering. Native output reuses unchanged frames,
but partial Cairo repaint remains disabled after a text-edge mismatch in
pixel tests. Native metrics use a 4,096-entry cache. SVG and PDF still render
complete documents. Call `CanvasSession::invalidate` after fonts change.
It clears backend metric caches and forces the next render to prepare again.
`backend_mut` also invalidates the session. Target size and scale remain fixed
per backend instance.

Scene passes, cells, geometry, and engine metadata use shared immutable
storage. Pass builders intern color and font strings and reserve common
primitive buffers. Backend code still reads no workbook data.

See the [M3 review and measurements](docs/reviews/2026-10-05-m3-measured-optimizations.md)
for supported cases, performance trade-offs, and exact validation.

## M2 worksheet composition (2026-10-05)

The new session composes visible headers, frozen panes, hidden axes, merges,
links, and host overlays. Core owns `GridGeometry` and range fragments.
The compiler owns IronCalc reads and merge/link semantics. A preparation-local
cache serves visible cells and offscreen anchors. `CanvasSession` publishes
query geometry, engine payloads, and the matching scene after backend commit.

`OverlayState` carries selection, clipboard, and formula paint ranges.
Active-cell restoration replays prepared content under fragment clips.
Committed queries produce `HostIntent` values. Only the host applies writes.
An empty overlay state gives exports without interactive overlays.

The shared scene now reaches Canvas2D, SVG, PDF, and native raster output.
`iron-canvas-native` uses Cairo/Pango and owns staged image surfaces. The host
owns the window and presentation. Native development requires Cairo and Pango
headers and `pkg-config` (for example, `libcairo2-dev` and `libpango1.0-dev`
on Debian). The crate is empty on wasm32. PDF keeps its existing font and
opaque-color limits. Neither backend reads the workbook.

The separate playground route exercises composition and host intents.
See the [M2 review](docs/reviews/2026-10-05-m2-worksheet-composition.md) for checks
and limits. M3 adds measured reuse above. M4 replaces the main legacy facade.
See the [M4 migration map](docs/reviews/2026-10-05-m4-migration-map.md) for the
legacy inventory and ordered cut plan.

## M1 scene path (2026-10-04)

The new `iron-canvas` crate owns `CanvasSession`. It borrows `&UserModel`
while `WorksheetCompiler` samples the viewport and lowers engine styles
into an owned `GridScene`. The compiler owns CF glyphs, rating policy,
spreadsheet text alignment, and border policy. Core owns generic axis
sampling, text layout, primitives, and scene execution.

`SvgSceneBackend` and `Canvas2dSceneBackend` consume the same scene contract.
They receive no workbook or spreadsheet DTOs. The session publishes its
prepared worksheet only after the backend commits the staged output.
Preparation, staging, and presentation errors preserve the previous frame
under the backend contract. Canvas2D target dimensions are fixed per backend.

Core and compiler retain a default `legacy` feature for the existing facade.
The new session disables both defaults. A standalone core build without
that feature exposes no `CanvasModel`, `CellDecoration`, or style mirrors.
M4 removes the old renderer after the main worksheet integration migrates.

The separate transition-playground route is `/playground/grid-core`, built
with `dev-tools`. See the [M1 review](docs/reviews/2026-10-04-m1-grid-core.md)
for validation and limits. The remainder of this document describes the
legacy pipeline unless a section says otherwise.

`iron-canvas` is the read-only rendering workspace used by RustyCalc. It
paints worksheet data supplied through `CanvasModel`, but it never mutates
that model. The live browser facade renders into two stacked canvases:

- the grid layer owns persistent cell, border, header, and frozen-pane
  pixels;
- the transparent overlay layer owns selection, autofill, clipboard,
  point-mode, formula-reference, hover, and consumer decoration pixels.

Both painting and queries use the same committed `Chrome`, so hit testing
cannot observe candidate geometry that failed to paint.

This is the supplied architecture snapshot, not a claim that every listed
backend is present in the current checkout. See
[`docs/m0-baseline.md`](docs/m0-baseline.md) for the M0 inventory and current
native-renderer availability.

## Workspace topology

| Crate | Responsibility |
| --- | --- |
| `iron-canvas-core` | Backend-neutral model traits, geometry, frame input capture and classification, planning, transactional paint orchestration, grid segment rendering, queries, decorations, and `Painter`/`Surface` traits. No `web-sys`, `wasm-bindgen`, or IronCalc types. |
| `iron-canvas-canvas2d` | Browser Canvas2D backend: `CanvasPainter`, `WebSurface`, paired `Canvas2dRuntime`, CSS theme bridge, setter caches, DPR handling, and text-measurement memo. No IronCalc dependency. |
| `iron-canvas-native` | Cairo/Pango raster backend: `NativePainter`, `NativeSurface`, and `CompositeTarget`. The host owns the window, model, event loop, and presentation. |
| `iron-canvas-web` | Spreadsheet `#[wasm_bindgen]` facade, JS-backed model bridge (feature `js-model`, default on), recording/playback integration, and SVG/PDF export entry points. |
| `iron-canvas-ironcalc` | `IronCalcModel<'a>` adapter plus the shared IronCalc-to-core style/color conversion and conditional-formatting bridge. |
| `iron-canvas-export` | Pure-Rust SVG and PDF painters/surfaces. Uses `ttf-parser` for font metrics and `base64` for font embedding. Export uses throwaway orchestrators and discards overlay output. |
| `iron-canvas-recorder` | `RecorderPainter`/`MemSurface` test backend and optional `RecordingPainter<P>`/`RecordingSurface<S>` `.icr` capture decorator. |

Deleting every adapter crate must still leave `iron-canvas-core` compiling and
testing on its own. Concrete backends and model adapters belong beside core,
not inside it.

## Ownership map

| State or decision | Owner | Invariant |
| --- | --- | --- |
| queued paint intent | `Orchestrator::pending: PendingWork` | One mergeable value, taken once per paint attempt with `mem::take`. Layers own no dirty gates. |
| scalar attempt inputs | `FrameInputs` | Sheet, view, freeze counts, header visibility, DPR, theme, model generation, and selection visibility are captured once. |
| committed geometry | `Orchestrator::last_frame: Option<Chrome>` | Queries see only committed geometry. |
| geometry verdict | `Chrome::classify` | Exactly one `FrameDelta::{Stable, Scroll, Rebuild}` per attempt. |
| paint plan | `FramePlan` | Grid scope and overlay policy are explicit values; no paint scope is stored on `Chrome`. |
| prepared model data | `FetchedCells` and `PreparedGrid` | Fallible reads may use scratch, but cannot mutate committed `GridCache` buffers or the painted fingerprint tree. |
| grid cache installation | `GridCacheCommit` | Execution returns owned commits; only the completion boundary installs them. |
| frame publication, presentation, retry | `Orchestrator::finish_attempt` | Cache commit is installed before its matching frame is published or surfaces are presented. |
| backend state | `Surface` and `Painter` | Renderer code stays independent of Canvas2D and IronCalc. |

## Paint-attempt lifecycle

Every non-idle `Orchestrator::render_pending` call follows this pipeline:

```text
PendingWork
    │ mem::take
    ▼
FrameInputs::capture
    │
    ├─ failure ───────────────► Held + retry all taken work
    ▼
Chrome::classify
    │ FrameDelta
    ▼
plan_frame
    │ FramePlan { GridWork, OverlayWork, consumes }
    ▼
prepare ──► execute ──► PaintOutcome
                           │
                           ▼
                 Orchestrator::finish_attempt
                 cache → frame → overlay → present → retry → trace
```

The stages are deliberately separate. A bridge read can fail while preparing,
but it cannot leave half-installed cache metadata, a candidate `Chrome`, or
shifted pixels behind.

### Pending work

`PendingWork` is the single queued-work algebra. It contains:

- `GeometryWork::{Clean, Rebuild}`;
- a `view` flag;
- `ContentWork::{Clean, Rows { sheet, spans }, All}`;
- an `overlay` flag.

Row spans are normalized and adjacent/overlapping spans are merged. More than
eight disjoint spans or rows from different sheets degrades to `All`. Once
work becomes whole-grid, row precision is not recovered.

`WorkFlags::{VIEW, CONTENT, GEOMETRY, OVERLAY}` is only a diagnostic projection
for traces and `.icr` frames. It is not queued state and must not be used to
reconstruct dispatch policy.

### Capture and classification

`FrameInputs::capture` reads these model values in fixed order:

1. selected sheet;
2. selected view, checked to reference the same sheet;
3. frozen row and column counts;
4. row and column header visibility;
5. selection visibility;
6. the sheet's link list;
7. the sheet's merged-range list.

`CanvasMetrics` (a size/DPR pair parsed at the host boundary), theme, and
`model_generation` come from the orchestrator.
Steps 1-5 are scalar reads; steps 6-7 are the allocating, fallible list
reads. They build a validated `LinkIndex` and `MergeTable` once per attempt, so
the renderer never crosses the model per cell; a `None` read or an invalid list
holds the attempt. Every model scalar except selection visibility is fallible. A
failure produces `FrameOutcome::HeldOnInputFailure`, paints and presents
nothing, and requeues the complete taken `PendingWork`. No synthetic sheet
`0`, A1 view, or visible header default is substituted.

Steps 6-7 are skipped when the last validated pair can be reused. The
orchestrator holds one `MetadataSnapshot` and rebuilds it only when the model
generation, sheet, or host metadata epoch changes; `set_model` clears it. The
host pushes the epoch with `Orchestrator::set_metadata_epoch`, and an epoch of
`None` (the default) clears the snapshot and disables reuse. Re-enabling reuse
starts a new snapshot. The setter does not queue paint work; the host must
also request a repaint. The web facade exposes this setter to Rust hosts only.
A cached pair serves the same digest
comparison and merge-impact classification, so an unchanged pair still stays
overlay-only while a moved epoch re-reads and escalates a changed list. The
epoch is the host's promise that either list may have changed whenever it
changes; a stale value is visible (an unclickable link or an editable merged
region).

`Chrome::classify(prev, model, inputs, active_cell)` compares the capture with
the committed frame in stable order. It returns:

- `FrameDelta::Stable` when size, DPR, theme, model generation, sheet, freeze,
  header visibility, and effective scroll origins still match;
- `FrameDelta::Scroll(BlitPlan)` for one-axis movement with compatible retained
  overlap and a live active-cell value matching its committed snapshot;
- `FrameDelta::Rebuild(RebuildReason)` for a missing frame, any hard break,
  two-axis movement, unknown/changed active-cell content, or incompatible
  overlap.

This single classifier replaces the former split `is_still_valid` and
`screen_for_blit` verdicts.

### Planning and five strategies

`plan_frame` combines the typed `PendingWork` and `FrameDelta` into a closed
`FramePlan`:

| Selected strategy | `GridWork` | Eligibility and work |
| --- | --- | --- |
| `OverlayOnly` | `None` | Stable view or overlay work without content or geometry work. Reuse committed `Chrome`. Repaint only the overlay. |
| `ScrollBlit` | `Blit(plan)` | Safe one-axis scroll without content or geometry work. Preflight all revealed address strips before a pixel moves. |
| `DamagedRows` | `Rows { sheet, spans }` | Stable geometry and row-addressed content on the visible sheet. Repaint full-width row bands. |
| `ChangedCells` | `AllContent` | Stable geometry and whole-grid content. Reuse slot vectors. Then fingerprint the candidate grid. |
| `FullRebuild` | `Fresh` | First frame, geometry work, rebuild delta, or content with a real scroll. Rebuild geometry and repaint atomically. |

The planner probes a safe scroll before the stable overlay fallback, because a
host view mark may represent real pixel movement. Changed content is never
blitted. A changed link set (`links_changed`, compared as one digest of the
committed `LinkIndex`) forces `ChangedCells`/`AllContent` on reusable
geometry — a metadata-only link change must repaint and commit with its new
query data — and bars the `Blit` arm, so a real scroll keeps `FullRebuild`.
See the `LinkIndex` digest note below.

Merge state is the second metadata escalation, classified by
`MergeImpact::{None, Visible, Changed}` from the committed frame and the
captured list: a digest change, or a merge that intersects either the committed
or the candidate visible area, forces `GridWork::Fresh` and bars the `Blit`
arm. `Damage` and `Blit` strips repaint cells without the merge pass, so a
merge must never reach them. An overlay-only attempt on stable geometry stays
`GridWork::None` even with a merge *in view*: nothing repaints the grid, so the
committed merge pixels are still correct. A *changed* merge table is excluded
from that arm — the arm commits the previous frame unchanged, so a table the
capture just discovered would be dropped and its geometry would stay
uncommitted until some other attempt rebuilt.
`OverlayWork::{Preserve, Paint}` is computed once in the plan.
`FullRebuild` and `ScrollBlit` always paint it. `DamagedRows` and `ChangedCells` paint it when
selection is visible or overlay work was explicitly marked.

`RenderStrategy` preserves the five public/recorded strategy names. It is a
data-free attribution tag, not the dispatch value; `GridWork` carries the
payloads and drives the exhaustive match.

## Transactional preparation, execution, and completion
### Prepared grid data

`FetchedCells` bundles the equal-length row-major channels:

- `Vec<Fetched<CellStyle>>`;
- `Vec<Fetched<String>>`;
- `Vec<Fetched<CellKind>>`;
- `Vec<Fetched<CellDecoration>>`.

Canvas2D lifecycle is owned by `Canvas2dRuntime<S>` in the Canvas2D backend.
It retains both HTML canvas handles, the raw painter handles, and the live DPR,
while exposing explicit access to the core `Orchestrator<S>`. Recording wrappers,
models, and application schedulers remain host-owned.

`Fetched<T>` distinguishes `Value`, `Absent`, and `BridgeFailed` across both
single-cell and bulk paths. Decorations use the same type as the other three
channels; `BridgeFailed` must never be collapsed into a blank cell before the
hold decision.
Preparation produces `PreparedGrid::{Empty, Full, Damage, Blit}`. Those variants
carry the deferred reset/replace/splice/shift transition directly. Preparation may reuse renderer
scratch and increment trace counters, but it may not write committed
`SegmentBuffers` content or the painted fingerprint tree.

Execution paints only prepared data and returns one owned `GridCacheCommit`
installed only by `finish_attempt`; a held attempt contributes no entry.

### Completion policy

Every strategy returns one private `AttemptOutcome`:

| Outcome | Meaning |
| --- | --- |
| `OverlayCommitted` | The overlay-only attempt executed. Preserve the committed frame and grid cache. |
| `GridCommitted` | The grid strategy executed. Install its owned cache commit and publish/present the matching frame. |
| `Held` | Nothing executed or presented. Preserve or roll back the committed frame and retry the required work. |

`finish_attempt` is the only completion boundary. Its order is load-bearing:

1. install `GridCacheCommit`;
2. preserve or replace `last_frame`;
3. refresh committed selection/active-cell overlay state;
4. paint the overlay when the plan requires it;
5. present the grid and overlay layers that painted;
6. merge retry work back into `self.pending`;
7. publish the selected strategy, effective strategy, work flags, and `FrameTrace`.

`FullRebuild` and `ScrollBlit` are whole-frame atomic. `FullRebuild` builds from an orchestrator
`spare_slots` pool without consuming `last_frame`; a failed candidate returns
its slot vectors to the pool. `ScrollBlit` uses `PreparedBlitFrame`, whose
move-based rollback reconstructs the previous `Chrome` if strip preflight
fails after candidate geometry was built. `ChangedCells` and `DamagedRows` require stable
geometry, so they may safely publish the reused candidate; a bridge failure
holds the whole attempt, prior pixels stay, and content is retried grid-wide.

`PaintResult::RetryRequired` means the scheduler should call `render_pending` again
without requiring a new host signal. A host may also defer the retry to its own
next wake-up: the failed work stays in `pending`, the committed pixels and query
geometry stay readable, and no host signal is lost.

## Chrome and pane geometry

`Chrome` is committed frame geometry, not a bag of paint work. It owns:

- sheet and `PaneSet`;
- row/column header thickness and shared `cell_origin`;
- `CanvasMetrics` (validated logical size + DPR) and `Rc<CanvasTheme>`;
- `model_generation` and captured header-visibility flags;
- `FrameKindTag::{Fresh, SlotsReused, Blitted}`;
- the committed `Rc<LinkIndex>` for its sheet, empty when the sheet has no
  links. A candidate frame carries the attempt's captured index (set by
  `Chrome::build`/`next`/`prepare_blit` from `FrameInputs`), so prepare and
  paint read one source; `finish_attempt` attaches it on the committed branch
  only, and a held strategy restores the previously committed index. Queries
  (`Orchestrator::link_at`) and hit testing read `Chrome::links()`, never the
  live model, so painted pixels and clickable targets cannot disagree;
- the committed `Rc<MergeTable>` for its sheet, empty when the sheet has no
  merges, attached on the same boundary as the link index. Merge geometry must
  commit with the pixels that used it, or hit queries and painted pixels would
  disagree; `Orchestrator::display_cell_at`/`visible_fragments` read it.

It does not own row spans, segment buffers, or fingerprints; it derives the
piecewise `GridLayout` of 1–4 dense address segments (TL, TR, BL, BR).
`GridWork` passes paint scope explicitly, preventing a SlotsReuse frame from
inheriting a previous blit's scope.

### Construction paths

`Chrome::next(prev, model, inputs, FramePath)` has two variants:

- `Fresh` walks the model and builds new geometry. `Orchestrator` normally
  calls `Chrome::build` directly with its standing `RecycledSlots` pool so a
  candidate can be held without dismantling the committed frame.
- `SlotsReuse` keeps the previous slot vectors and header labels verbatim,
  refreshing theme, DPR, model generation, header flags, and kind. Stable edit
  repaints therefore skip row/column geometry walks.

`Chrome::prepare_blit` builds a reversible scroll candidate for the live
orchestrator. `Chrome::next_blit` is the immediate-commit convenience wrapper
and returns `BlitOutcome::{Blitted, FreshFallback}`. In-place reuse can still
reject after classification, for example when row-header width changes at a
digit boundary; that path rebuilds Fresh.

### Fresh build phases

`FramePath::Fresh` runs five ordered phases:

```text
A  captured freeze counts
B  row walk into PaneSet
C  measure row-header thickness from the last visible row label
D  column walk using the measured X origin
E  resolve header labels and assemble Chrome
```

The scalar inputs come from `FrameInputs`; only row heights, column widths,
model bounds, and header labels are read during the geometry walk. Hidden
headers have thickness and inset `0`, allowing cells to reclaim that edge.

`PaneSet` owns `AxisSlots<RowSlot>` and `AxisSlots<ColSlot>`. Each axis owns
frozen and scroll vectors, a `frozen_offset`, and `last_id`. Slot coordinates
are absolute integer CSS pixels; DPR remains a separate `f64` backend concern.
The 3-pixel `FROZEN_SEP` gap is woven into the slot vectors by `PaneSet` and
painted after cells so the separator wins shared pixels.

Use `Chrome::hit_test`, `cell_rect`, `range_rect`, `resize_handle_at`,
`scroll_pane_rect`, and `scroll_to_show`; hosts must not duplicate slot or
frozen-pane arithmetic.

## Grid renderer and retained pixels

FullRebuild, ChangedCells, DamagedRows, and ScrollBlit share one grid shell:

```text
optional blit shifts
Grid
  Cells          strategy-specific prepared grid work
  FrozenSep      after cells
  Headers        both axes, or only the scrolled axis for ScrollBlit
  Corner         only when both header strips are visible
```

Each painted segment uses five ordered passes over resolved `CellPaint` slots:

1. backgrounds;
2. conditional-format decorations;
3. grid borders;
4. explicit borders;
5. text.

This order keeps decorations below borders, explicit borders above grid
borders on shared edges, and text above neighbouring backgrounds.

### Cache lifetimes

| Lifetime | Type | Purpose |
| --- | --- | --- |
| per-call content, retained capacity | `FrameCache` | `CellPaint` slots, grid-line flag, text lines/wrap string, and a pool of strip `FetchedCells`. Contents are overwritten; capacity survives. |
| cross-frame model/pixel truth | `GridCache` | `SegmentBuffers` per region plus one `FingerprintState`, identity keyed by `GridLayout`. Only installed commits change it. |
| renderer lifetime | `FontIntern`, `ColorIntern`, Canvas2D setter/measurement caches | Avoid repeated allocation and backend crossings. |

The fingerprint is one grid tree keyed by the exact `GridLayout`: one digest
per absolute row folding every dense column segment containing that row, plus
one flat `CellFingerprint` leaf per addressed cell. Row metadata points into
the flat leaf vector, so exact changed cells can be recovered without a
per-row allocation. Two warm trees cost one additional `u64` per addressed
cell each. The frozen-row band is stored first with `scroll_band_start`
marking the split. A full-grid candidate is compared with the committed tree
to select:

- `Skip` for equal pixels;
- `Cell` for one exact changed leaf;
- `Range` for a bounded merged envelope around several changed leaves;
- `Rows` for safe full-width changed bands;
- `Full` when exact comparison is unavailable or the merged envelope covers
  the visible grid.

`Cell` and `Range` clear one two-pixel-padded clip, aligned outward to backing
pixel boundaries, and replay each contributing
layout segment at most once through the normal five paint passes. Contributor
ranges come from pixel intersection, so shared borders and frozen seams keep
Fresh ordering. They still install the complete candidate buffers and
fingerprint through `GridCacheCommit::Replace` at `finish_attempt`; the clip
narrows painter work, not cache truth or model fetching.

Row-shift blits rotate only the scroll-row band and rebase its leaf slices;
column shifts and damage strips mark truth stale. Retained fingerprint truth
is explicit: a commit either installs a complete derived tree or marks the
prior tree stale; it never implies that a partial observation is exact.
`FingerprintState::painted()` exposes only exact history to repaint planning.
After a stale commit, the next full-grid content check selects `Full` and
installs an exact tree. A later unchanged content check can select `Skip`.

### Merge state

`MergeTable` stores one rectangle per merge and an order-independent digest.
Lookups scan the rectangles. Storage is O(merge count), independent of covered
area. Construction validates bounds, compares rectangle pairs for overlap, and
sorts a copy for the digest. Overlap validation is O(merge count squared).
An interval index remains a measured optimization for sheets with many merges.

A merge is painted **over** the cells rather than by suppressing them: the
per-cell pass fills, strokes, and lays out every covered cell as usual, and the
merge pass then paints the anchor's fill, perimeter borders, conditional
formatting, and text on top. Frozen separators still paint after the whole
`GroupClass::Cells` group, so they stay visible over a merge. This drops the
alternative's per-slot "covered" predicate from the fingerprint, the blit
shift, and the repaint envelope; the wasted work is bounded by (merge
intersection viewport) and suppressing it is a deferred optimization.

The active-cell overlay restores the **logical** cell through the same
geometry and paint path, so the selection tint cannot leave a second, smaller
label or interior grid edges over the merged pixels. It runs on the overlay
surface, which has no fetched buffers, so it reads the anchor and its visible
perimeter from the model. Perimeter reads are bounded by the visible fragments.
Geometry still sums the full logical row and column extents for text layout.

The perimeter is built from the **already-fetched** segment styles, not from
new model reads: IronCalc spreads the anchor style across the range and keeps
each border side only on the perimeter, and an imported file can give
consecutive perimeter cells different sides. Adjacent runs with an equal
resolve coalesce, and a run never crosses a cell whose style was not read.
Anchor content likewise prefers the fetched buffers and falls back to the
single-cell accessors only when the anchor is scrolled out of view — so a
visible merge costs no bridge crossing.

### Link state

Committed links are sparse: `LinkIndex` is a keyed map over absolute
`(row, column)` plus one digest, so it costs O(links) and needs no blit
shift. Each cell digest folds its link's precomputed `u64` (presence,
resolved color, target, tooltip, dynamic), so a same-label target change
repaints the cell and cannot leave a stale clickable target.

The index digest is the change detector: `plan_frame` compares it against the
committed frame and escalates a change to whole-grid content work. Whole
content, not row damage, because a link can change in a row the host never
edited. The row-shift blit path therefore never compares a shifted prior
digest against a new link truth; a column shift already reports stale.

Deferred optimization, recorded for later profiling: separate visual equality
(hash, underline, color) from interaction equality (target, tooltip, dynamic)
in the metadata comparison. Visual equality alone is not sufficient for the
first implementation — the clickable target must always match the committed
state.

### Blit safety

`RendererCore::prepare_blit` preflights the 1–2 candidate-derived address
strips before the single `Painter::blit`:

- a compatible cache shift stages the revealed strip;
- a cold cache or incompatible layout falls back to a full-grid replacement
  (traced as `BlitFallback`);
- any `BridgeFailed` recycles all prepared work and holds the whole attempt.

Only after every strip is clean does execution apply the single merged shift,
run the shared grid shell, and splice the strips into the shifted buffers.
Moving pixels before preflight would strand stale shifted pixels on a failed
attempt.

## Layers, painters, and surfaces

`Orchestrator<S>` owns `LayerBase<S, GridRenderer<S::P>>` and
`LayerBase<S, OverlayRenderer<S::P>>`. `LayerBase` is gate-free. `Surface`
owns a painter and provides `resize` and `present`; all drawing goes through
the unsealed `Painter` trait, with scroll copying isolated in `BlitPainter`.

Browser surfaces have different presentation semantics:

- grid paints into a detached back canvas with alpha enabled and reads blit
  source pixels from the visible front canvas. Blits use `copy` inside the
  destination clip. `present` copies the full back canvas to the front with
  `copy`. Both operations preserve source alpha and replace old pixels;
- overlay draws directly into a transparent/desynchronized canvas, so its
  `present` is a no-op.

Empty and resized grid buffers are transparent. Sheet backgrounds must still
be opaque. The renderer uses background fills to erase retained content;
translucent fills would accumulate alpha and leave old content visible.

Replay must present each replayed grid frame before a later recorded blit,
because the later blit reads the visible front canvas.

The shipped painters are `CanvasPainter`, `NativePainter`, `SvgPainter`,
`PdfPainter`, `RecorderPainter`, and the `RecordingPainter<P>` decorator. SVG measures and
draws embedded Inter; PDF measures and draws base-14 Helvetica; Recorder uses
the core approximation. `CanvasPainter` owns DPR, sticky setter caches,
interned custom strings (colors *and* font CSS), and a bounded
`(font_css, text)` measurement memo.

The native backend keeps grid and overlay image surfaces. Its per-layer
`present` is a no-op. After `PaintResult::Rendered`, the host composites grid
then overlay and copies the result to its window buffer. Native blits use a
separate scratch source to preserve overlapping pixels. Cairo and Pango both
receive each DPR change. `MeasureCache` and `snap_stroke_cross` live in core
and are shared with Canvas2D.

## Overlay contract

Persistent model pixels belong on the grid. Transient interaction pixels
belong on the overlay. `Decorations` owns built-in and consumer layer order.
Committed selection and active-cell state is refreshed once in
`finish_attempt`, using the same captured sheet/view/selection visibility as
the frame being completed.

Content work may also require overlay paint because `ActiveCellRepaint`
contains model-derived pixels. The planner owns this CONTENT → OVERLAY rule;
strategy helpers must not re-derive it independently.

## Query pipeline

The public query methods delegate through the facade to
`Orchestrator::last_frame`:

- `hit_test` classifies corner, headers, cells, autofill, and formula-ref
  handles;
- `cell_rect` and `range_rect` resolve visible geometry;
- `resize_handle_at` snaps to row/column trailing edges;
- `autofill_handle` derives the handle from committed selection state;
- `scroll_pane_rect`, `legal_scroll_origin`, and `scroll_to_show` expose
  renderer-owned viewport math for navigation consumers.

Before the first committed paint these return their absent variants. A held
attempt does not advance `last_frame`, so queries continue to match the
visible committed pixels.

## Adapter and format boundaries

`CanvasModel: CellContentQuery` is read-only. `IronCalcModel<'a>` exists
because orphan rules prevent implementing the core trait directly for
IronCalc's `UserModel`. The JS-backed adapter batches styles, values, and cell
types across the wasm boundary and preserves `Fetched::BridgeFailed` on its
degrade path; conditional-format decorations currently use the trait's
per-cell default loop.

Sheet-level link state is the one non-scalar capture read,
`CanvasModel::get_sheet_links`: `Some(empty)` is a known empty collection or a
statically absent optional capability; `None` holds the attempt. Never convert
a throw, a malformed payload, or an invalid list into empty data — a dropped
link is a visible link the host can no longer click. The native adapters call
`UserModel::get_links_list`, which already merges worksheet and formula links
with worksheet precedence and preserves `dynamic`; the JS adapter calls
`getLinks` and resolves the hyperlink color from the cached workbook theme.

Merges are captured the same way through `CanvasModel::get_merged_ranges`:
inclusive `RCRange`s, `Some(empty)` for a merge-less sheet or a statically
absent optional capability, `None` for a throw, a malformed payload, or a list
`MergeTable::from_ranges` rejects. The native adapters convert IronCalc's typed
`MergedCell` with checked arithmetic; the JS adapter decodes `getMergedCells`
through `serde_wasm_bindgen`. Neither path parses A1 strings, and neither
converts an invalid list into empty data.

The JS-backed adapter and the facade's `setModel` compile only with
`iron-canvas-web`'s `js-model` feature. The feature is on by default, so the JS
package is unchanged. A native Rust host turns it off and binds its model
through `IronCanvas::set_model`; `ironcalc_base` and `iron-canvas-ironcalc`
leave the facade's dependency list with it.

`Surface` implementations for SVG and PDF are throwaway Fresh renderers, so
their `BlitPainter` implementations are no-ops and live viewport reuse must
never reach them.

The recorder's `.icr` schema is version 11. This version adds `StrokePath`
with an owned stroke style; version 10 changed `FillPath` from integer points
to a float path command list, so shapes carry curves (quadratics, cubics, arcs)
instead of rounded polygons; version 9 added the native circle op and version 8
the horizontal gradient fill. Each attempt stores a recorder-owned `FrameTrace`
projection: selected versus effective strategy,
work bits, one grid verdict (`GridVerdict`), whole-frame outcome, blit
fallback, attempt/commit identities, and fetch attribution. Non-idle zero-op
holds remain in the timeline.

Runtime frame diagnostics use schema version 4. Repaint diagnostics separate
exact `changedCells` evidence from the applied `clip` and region-tagged
`sourceRanges`; compact traces use `grid:cell` and `grid:range`.

With `dev-tools`, the web facade exposes a Rust-only `CanvasFrameSnapshot`.
It owns one completed attempt's diagnostics and the backing size read at
capture time. `frame_diagnostics_value` projects this snapshot to a JSON
object without a live canvas; the host serializes that value. It shares
`project_frame_diagnostics` with the JS `frameDiagnostics()` method.
`frame_attempt_seq()` reads the lightweight trace, so attempt identity is
available with detailed capture off.

## Source map

Paths are relative to `iron-canvas/crates/`.

| Source | Read for |
| --- | --- |
| `iron-canvas-core/src/frame/` | one paint attempt: queued intent (`work.rs`), captured scalars and capture failures (`inputs.rs`), the fallible link/merge capture plus its revision-keyed reuse snapshot (`metadata.rs`), the classification delta plus `BlitPlan`/`Shift` (`delta.rs`), the pure planner (`plan.rs`, tests in `plan_tests.rs`), and the report types (`report.rs`). |
| `iron-canvas-core/src/orchestrator/` | `mod.rs` (`Orchestrator`, constructor, accessors), `setters.rs`, `query.rs`, `finish.rs` (`AttemptOutcome`/`finish_attempt`), `dispatch.rs` (`render_pending`), and `strategies.rs` (the five render strategies). |
| `iron-canvas-core/src/chrome/` | committed geometry (`mod.rs`, `build.rs`, `classify.rs`, `query.rs`, `hit.rs`, `blit/{mod,rebuild,reuse}.rs`), stable classification, Fresh/SlotsReuse construction, reversible blit candidates, `GridLayout` piecewise address layout, slot allocation recycling, and the merge viewport impact classification (`merge.rs`: `MergeImpact`, plus the `MergeTable::intersects_visible` geometry probe). |
| `iron-canvas-core/src/geometry/` | `prim.rs`, `pixel_rect.rs`, `path.rs` (`PointF`/`PathCmd`/`ArcSpec`/`Path`, validation, and the quadratic/arc-to-cubic conversion helpers), `constants.rs`, `labels.rs` (`col_name`), `extent.rs` (`CanvasSize`/`CanvasMetrics`), and `slot/{mod,axis,fill,extent}.rs` (axis-generic slot filling and pixel/id queries). |
| `iron-canvas-core/src/address.rs` | cell-space addresses and areas: `RCRange`, `DenseRange`, `CellCoord`, `SheetArea`, `FormulaRef`, `AutofillTarget`. |
| `iron-canvas-core/src/model/sheet/` | engine-neutral worksheet render data, crate-internal: `links.rs` (`LinkTarget`, `CellLink`, `LinkIndex`, and the per-link digest the cell fingerprint folds), `merges.rs` (`MergedRange`, `MergeTable` with validated bounds/overlap, `anchor_at`/`merge_at`, `intersects_rect`, digest), and `snapshot.rs` (the `Rc`-shared captured link+merge metadata one attempt paints and queries from). No `Chrome` import. |
| `iron-canvas-core/src/lib.rs` | Public link and merge types are exported at the crate root. Callers use `iron_canvas_core::{CellLink, LinkIndex, LinkTarget, MergeTable, MergedRange}`. The old `link::` and `merge::` module paths are removed. |
| `iron-canvas-core/src/surface/` | `Surface`, `LayerBase`, and the grid/overlay paint specializations (`mod.rs`, `grid.rs`, `overlay.rs`). |
| `iron-canvas-core/src/renderer/diagnostics/snapshot.rs` | stable diagnostic snapshot vocabulary: `FrameDiagnostics`, its `Diag*` mirrors, and `DIAG_SCHEMA_VERSION`. Ungated. |
| `iron-canvas-core/src/renderer/diagnostics/` | diagnostic collection: `DiagState`, the publication boundary, and cache sampling (`state.rs`), and the gated `RendererCore` capture writers plus the private prepared-data reads (`capture.rs`). Both gate on `dev-diagnostics`. |
| `iron-canvas-core/src/renderer/prepared/` | `FetchedCells`, `PreparedGrid`, `GridCacheCommit` (`mod.rs`), the prepare walks (`prepare.rs`), execute/commit (`execute.rs`), the paint helpers (`paint.rs`), and the repaint decision types (`repaint.rs`). |
| `iron-canvas-core/src/renderer/cache/` | per-call scratch, `GridCache` segment buffers, fingerprint truth and construction, layout-transition classification, and the shared decoration-color decoder. |
| `iron-canvas-core/src/renderer/cell/` | five-pass painting, borders, text, and conditional formatting. The retained-pixel decisions live in `renderer/repaint/`. |
| `iron-canvas-core/src/shape/` | shared unit-box shape definitions (`cf.rs`: one `GlyphDef` per `IconGlyph`) and allocation-free placement (`place.rs`: uniform fit, rotation, stroke scaling, arc transform, bounded command buffers). The CF renderer keeps the exhaustive `IconGlyph` dispatch. |
| `iron-canvas-core/src/renderer/merge/` | merge preparation and paint, split by responsibility: shared prepared types (`mod.rs`: `PreparedMerge`/`MergeFragment`/`MergePerimeter`/`BorderRun`), anchor resolution and the prepare walks (`prepare.rs`), fragment geometry and the extent cache (`geometry.rs`), perimeter run-building (`borders.rs`), and the merge paint pass that runs last inside `GroupClass::Cells` (`paint.rs`). |
| `iron-canvas-core/src/renderer/` | `mod.rs` (`RendererCore`), `grid.rs` (grid shell), `layers.rs` (`GridRenderer`/`OverlayRenderer`), `trace.rs` (trace collection), `blit.rs`, `repaint/{envelope,plan}.rs` (repaint decisions), and `chrome/` (header/corner/separator painters). |
| `iron-canvas-core/src/model/` | `CanvasModel`, `CanvasView`, `CellContentQuery` (`mod.rs`), `Fetched<T>` (`fetched.rs`), autofit measurement (`autofit.rs`), and the crate-internal worksheet render data (`sheet/`). |
| `iron-canvas-canvas2d/src/` | Canvas2D painter, paired runtime, measurement cache, theme bridge, and front/back canvas presentation. |
| `iron-canvas-native/src/` | Cairo painter and blit source (`painter.rs`), layer/composite surfaces (`surface.rs`), and color/font parsing. |
| `iron-canvas-web/src/orchestrator/` | Web facade, JS API, export entry points, diagnostics projection, recording, and playback controls. |
| `iron-canvas-web/src/wasm/mod.rs` | JS model bridge (feature `js-model`), batch capability probes, failures, and theme caching. |
| `iron-canvas-web/src/playback.rs` | timed playback and per-frame grid presentation. |
| `iron-canvas-recorder/src/` | draw-op schema and owned stroke style (`ops.rs`), op-emitting painter (`painter.rs`), capture wrappers (`surfaces.rs`), and the `.icr` v11 attempt schema (`recording.rs`). |
| `iron-canvas-export/src/` | SVG/PDF painters, metrics, document writers, and one-shot surfaces. |
| `iron-canvas-ironcalc/src/lib.rs` | IronCalc adapter and style/decoration conversion. |

## Verification

Use the narrowest gate that proves the changed boundary:

```bash
(cd iron-canvas && cargo test -p iron-canvas-core --locked)
(cd iron-canvas && cargo test --workspace --locked)
(cd iron-canvas && cargo test -p iron-canvas-web --features dev-tools --locked)
(cd iron-canvas && cargo check --target wasm32-unknown-unknown --workspace --exclude iron-canvas-native --locked)
(cd iron-canvas && cargo test --target wasm32-unknown-unknown -p iron-canvas-web --locked)
```

The `dev-tools` native run is the wire-shape gate: it asserts the JS-facing
field names without a browser.

The wasm test command requires the lockfile-matched
`wasm-bindgen-test-runner` and ChromeDriver configuration used by
`.github/workflows/test.yml`. Browser retained-pixel tests compare raw
Canvas2D `ImageData` byte-for-byte with forced-Fresh output. Those tests prove
only their named held FullRebuild/ScrollBlit recovery, row/border repaint,
stable-geometry edit, and post-blit cases; new retained-pixel behavior needs
its own forced-Fresh comparison.

For documentation maintenance, run `scripts/arch-staleness.sh`, then also
inspect `git status`, unstaged/staged diffs, untracked files, and dirty
submodules: the scanner only compares the recorded anchor with `HEAD`.
