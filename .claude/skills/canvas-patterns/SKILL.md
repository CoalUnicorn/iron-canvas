---
name: canvas-patterns
description: Use when extending, modifying, reviewing, or explaining the iron-canvas workspace: CanvasSession frame lifecycle, the resolved GridScene, the IronCalc worksheet compiler, retained damage and blit, Canvas2D/native/SVG/PDF backends, overlays, auto-fit, and committed-frame queries.
---

# iron-canvas patterns

Read live source before you rely on a symbol in this guide. Start with
[ARCHITECTURE.md](../../../ARCHITECTURE.md) at the repository root.
All paths below are relative to the iron-canvas repository root.

## Workspace boundaries

| Crate | Responsibility |
| --- | --- |
| `iron-canvas-core` | Generic geometry, text layout, paint contract, resolved `GridScene`, grid and pane geometry, retained fingerprints. No IronCalc, browser, or Cairo types. |
| `iron-canvas-ironcalc` | Read-only `UserModel` worksheet compilation, style/CF/border/text lowering, merges, links, glyph catalogue, auto-fit. |
| `iron-canvas` | `CanvasSession`: request, reuse, prepare/stage/commit, committed-frame queries, overlays. Generic over the backend. |
| `iron-canvas-canvas2d` | Browser `<canvas>` surfaces, `CanvasPainter`, text-metric memo, CSS-var theme bridge. |
| `iron-canvas-native` | Cairo/Pango painter and staged image surfaces. Empty on wasm32. |
| `iron-canvas-export` | SVG and PDF painters and scene backends, document assembly, XLSX loading, export orchestration. |

`iron-canvas` depends on no concrete backend. No crate depends on
`iron-canvas` except a host. Keep core free of `web-sys`, `wasm-bindgen`,
Cairo, and IronCalc types. Put backends and the model adapter in sibling
crates.

## The frame lifecycle

`CanvasSession::render` is the single publication boundary. The host owns the
workbook. The session borrows `&UserModel` only while it prepares an owned
candidate.

```text
RenderRequest { sheet, revision, viewport, theme, overlays }
  -> reuse check (sheet, workbook_id, theme, revision, viewport)
  -> WorksheetCompiler::prepare_reusing_geometry
  -> overlays::compose
  -> plan (Fresh | Reuse | Damage | Blit)
  -> backend.stage_update
  -> backend.commit_frame
  -> publish frame + request + reuse data together
```

- Reuse skips preparation only when sheet, `revision.workbook_id`, theme,
  revision, and viewport all match. An overlay-only change reuses content and
  recomposes overlays.
- A revision change recompiles all visible content. Conditional formatting can
  depend on offscreen cells.
- Any preparation, stage, or commit error leaves the previous committed frame
  and its cache truth unchanged.
- `RenderOutcome` reports `sequence`, `sheet`, `revision`, `cell_count`,
  `extended_style_reads`, `reused_geometry`, and `strategy`.
- `render_fresh` bypasses reuse for reference frames. `invalidate` drops
  prepared reuse and clears backend metric caches after a font change; the
  published frame stays queryable.
- Sequence numbers start at 1. An exhausted sequence is
  `RenderError::SequenceExhausted`.

Do not collapse the stages. Preparation is pure and fallible. Staging may
fail. Commit publishes.

## Retained rendering

Core hashes every resolved primitive and its effective clip into 32-pixel
bands in one traversal (`SceneFingerprint`). A backend that reports
`supports_partial_updates` receives a `SceneUpdate`:

- `Reuse` — no paint change.
- `Damage(bands)` — changed bands; the backend clears their union and repaints
  intersecting commands.
- `Blit { .. }` — a verified translated overlap plus exposed strips.
- `Fresh` — full repaint.

`RenderStrategy` (`Fresh`/`Reuse`/`Damage`/`Blit`) is the recorded verdict from
`SceneUpdate::strategy`.

Blit needs an unfrozen, unmerged, single-axis vertical scroll with integral
device-pixel movement. `retained::scroll_plan` also requires
`translated_overlap_matches` on the interior overlap, minus a 4-pixel inset
that repaints boundary pixels. Merges force `Fresh`.

A backend without partial updates receives `Fresh` only. Do not stage a
`Damage` or `Blit` update on such a backend.

## Scene and painter contract

`iron-canvas-core::scene::{GridScene, SceneBuilder, Primitive, RenderBackend}`.

- `GridScene` is a clip plus ordered passes of `Primitive` values. The
  producer decides each pass's meaning. Paths and colors are `Rc`-shared, so a
  cloned prepared scene is cheap.
- `SceneBuilder` interns color and font strings and owns the pass buffer.
  Build one pass per paint group, not one allocation per cell.
- `painter::{Painter, TextMetrics}` plus `PaintColor`, `StrokeStyle`,
  `TextAlign`, `TextBaseline`, `LineCap`, `LineJoin`. Backends implement both
  traits. Nothing above them draws.

Every backend implements `RenderBackend`:

```rust,ignore
type StagedFrame;
type Error;
fn metrics(&self) -> &dyn TextMetrics;
fn stage_frame(&mut self, scene: &GridScene) -> Result<Self::StagedFrame, Self::Error>;
fn commit_frame(&mut self, staged: Self::StagedFrame) -> Result<(), Self::Error>;
// Optional retained hooks:
fn invalidate_metrics(&mut self) {}
fn retained_scale(&self) -> Option<f64> { None }
fn supports_partial_updates(&self) -> bool { false }
fn stage_update(&mut self, scene: &GridScene, update: &SceneUpdate)
    -> Result<Self::StagedFrame, Self::Error> { self.stage_frame(scene) }
```

`commit_frame` publishes in one operation. A failed stage or commit must leave
the previous visible frame intact.

## The IronCalc compiler

`WorksheetCompiler::prepare(workbook, sheet, viewport, metrics, theme)` reads a
borrowed `UserModel` and returns an owned `PreparedWorksheet`:

```rust,ignore
pub struct PreparedWorksheet {
    pub scene: GridScene,
    pub cells: Rc<[CellPlacement]>,
    pub geometry: Option<Rc<GridGeometry>>,
    pub merges: Rc<[MergedCell]>,
    pub links: Rc<[CellLinkView]>,
    pub extended_style_reads: usize,
}
```

- `WorksheetViewport { first_cell, width, height }` selects the window.
- `prepare_reusing_geometry` resamples dimensions and reuses a prior
  `GridGeometry` when `bounds`, `origin`, and bands are equal.
- Each distinct needed cell is read once per preparation, including offscreen
  merge anchors and active-cell restoration (`get_extended_cell_style`).
- Fixed pass order: background, CF graphics, grid borders, explicit borders,
  text, then merge composition. `CellReadOperation` and
  `WorksheetCompileError` report structured failures.
- `autofit::{fit_width, fit_height, FIT_PADDING, FIT_SCAN_CAP, AutoFitError}`
  measure the widest formatted value and tallest wrapped block with the
  backend's `TextMetrics`, using the same font rule as the text pass. The span
  is capped; an empty span is `Ok(None)`.
  Column fitting measures explicit lines separately and includes CF icon space.
  Row fitting uses all columns of a merged anchor. `AutoFitError::RowExtent`
  and `MergedCells` report the added read failures.

Do not add workbook writes, a second formula parser, or a second renderer here.

## Geometry and committed-frame queries

`scene_geometry` is model-independent. `sample_axis` and `sample_bands` sample
the frozen band first, then the scrolling band, both clamped to the viewport.
A frozen band can consume the whole extent. `AxisBand::frozen` marks each band.
Band indices include hidden cells at an exact viewport boundary. Hidden cells
have no hit rectangle, but selection edges and autofill use the visible corner.

`GridGeometry` answers:
`hit_test(point) -> GridHit`, `fragments(range) -> Vec<RangeFragment>`
(per-pane clipped rects with true-edge flags), `cell_rect(row, col)`,
`scroll_pane_rect()`, `autofill_handle(range)`,
`resize_target(point, tolerance) -> GridResize`.

Queries on `CanvasSession` read only the committed frame:
`hit_test`, `formula_ref_hit_test`, `display_cell_at`, `visible_fragments`,
`link_at`, `cell_rect`, `scroll_pane_rect`, `autofill_handle(_rect)`,
`resize_target`, `scroll_origin`, `committed_viewport`, `committed_frame`,
`scroll_to_show(&UserModel, row, col)`, `fit_column_width`, `fit_row_height`,
`pointer_intent(Point, PointerAction)`, `resize_intent(Point, pixels)`.

`HostIntent` is a request, never a write: `Select`, `ActivateLink`,
`ResizeRow`, `ResizeColumn`. Only the host applies it. Do not re-derive
visibility, frozen-pane, or slot math in a host.
`resize_intent` takes an exact header-divider point. A drag with a tolerance
retains the target from `resize_target`.

## Overlays

`OverlayState { selection, active_cell, clipboard, formula_references }` is
composed onto the candidate scene, so it shares the frame commit.
`overlays::compose` runs on a freshly prepared worksheet only; a composed
scene would replay its own overlay passes inside the restore clip.

- Selection fill and outline are edge-aware per fragment.
- Active-cell restoration replays prepared content under the fragment clip, so
  CF and merged content reappear under the selection. No engine reads.
- Header highlights stay in their strips.
- `FormulaOverlay { sheet, range, color_index }` describes paint only. The host
  resolves formula references.
- An empty `OverlayState` gives a content-only export.

Put persistent model pixels on the grid scene. Put transient interaction
visuals on the overlay.

## Backends

- `iron-canvas-canvas2d`: `Canvas2dSceneBackend::new(canvas, metrics)` owns a
  detached staging canvas and implements `RenderBackend` with retained scale
  and partial updates. `CanvasPainter` draws primitives; text metrics are
  memoized. `theme_from_element` reads CSS variables into a `CanvasTheme`.
  Size and scale are fixed per instance.
  Commit restores the target's `copy` composite mode before publication.
  Transparent staged pixels replace old pixels.
- `iron-canvas-native`: `NativeSceneBackend::new(scale)` rasterizes with
  Cairo/Pango into staged image surfaces. The host owns presentation.
  `surface()` exposes the committed `ImageSurface`; `surface_digest()` gives a
  content digest for retained-vs-fresh comparison. Partial Cairo repaint
  stays disabled. Native builds need Cairo and Pango headers plus
  `pkg-config`.
- `iron-canvas-export`: `SvgSceneBackend` and `PdfSceneBackend` render complete
  documents from the same scene. `workbook::{export_xlsx, export_prepared,
  inspect_prepared}` provide file export and prepared-scene inspection.
  `export_xlsx` loads and evaluates the workbook, resolves the sheet and range, prepares with
  `WorksheetCompiler`, and writes through the selected backend. The scene
  backends do not read the workbook.

Forward device pixel ratio as `f64`; only backing-size conversion rounds.

## WebAssembly boundary

- `bindings/wasm` is scene-only. `CanvasRenderer` accepts an Iron Canvas scene
  and draws it with `Canvas2dSceneBackend`. It has no engine dependency.
- `examples/react/wasm` is an app-specific `cdylib`. It holds a
  `CanvasSession` with a `SceneCompilerBackend` and passes committed scenes to
  the painter. Read it for the single-module pattern.
- Do not send scenes or cell data across the JavaScript boundary when the host
  can link the crates into one module.

## Source map

| Source | Read for |
| --- | --- |
| `crates/iron-canvas/src/lib.rs` | `CanvasSession`, `RenderRequest`, `RenderOutcome`, `RenderError`, reuse and publication |
| `crates/iron-canvas/src/interaction.rs` | committed queries, `HostIntent`, `DisplayCell`, `FormulaRefHit` |
| `crates/iron-canvas/src/overlays.rs` | overlay pass composition |
| `crates/iron-canvas/src/retained.rs` | scroll-blit planning |
| `crates/iron-canvas-core/src/scene.rs` | `GridScene`, `SceneBuilder`, `Primitive`, `RenderBackend` |
| `crates/iron-canvas-core/src/scene/retained.rs` | fingerprints, damage bands, blit verification |
| `crates/iron-canvas-core/src/scene_geometry.rs`, `scene_geometry/` | axis sampling, `GridGeometry`, hits, fragments, resize |
| `crates/iron-canvas-core/src/painter/`, `text_layout.rs`, `theme.rs`, `shape/` | paint contract, wrapping, palette, shape placement |
| `crates/iron-canvas-ironcalc/src/compiler.rs`, `compiler/` | worksheet preparation and lowering passes |
| `crates/iron-canvas-ironcalc/src/autofit.rs` | auto-fit measurement |
| `crates/iron-canvas-canvas2d/src/` | `CanvasPainter`, `Canvas2dSceneBackend`, metric memo, theme bridge |
| `crates/iron-canvas-native/src/` | Cairo/Pango painter and native backend |
| `crates/iron-canvas-export/src/{svg,pdf,common}/`, `workbook.rs` | export painters, backends, workbook export |
| `bindings/wasm/src/lib.rs` | scene-only JS adapter |
| `examples/react/wasm/src/lib.rs` | single-module host pattern |

## Verification

```bash
cargo test --workspace --locked
cargo check -p iron-canvas-core --locked
cargo check --workspace --target wasm32-unknown-unknown --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo test -p iron-canvas --target wasm32-unknown-unknown --test cf_fixture_canvas2d
```

The last gate is the browser pixel proof. It needs a
`wasm-bindgen-test-runner` whose schema matches the workspace's pinned
`wasm-bindgen` version, plus ChromeDriver. Compare retained Canvas2D output
against a forced `Fresh` render. A compile check does not verify browser
behavior.

## Change checklist

- [ ] Read `ARCHITECTURE.md` and the live caller and callee path.
- [ ] Keep core free of IronCalc, browser, and Cairo types.
- [ ] Keep the order: reuse -> prepare -> compose -> plan -> stage -> commit.
- [ ] Preserve the previous frame on any preparation, stage, or commit error.
- [ ] Install the aggregate commit before publishing pixels or query state.
- [ ] Include every paint input in the fingerprint domain.
- [ ] Gate `Damage` and `Blit` on `supports_partial_updates`.
- [ ] Update every backend for a new `Painter` or `RenderBackend` method.
- [ ] Update the compiler for a new `UserModel` read.
- [ ] Keep the device pixel ratio `f64` until backing-size conversion.
- [ ] Run the native tests and the browser gate that match the change.
