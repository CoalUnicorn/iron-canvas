# M0 baseline

**Recorded:** 2026-10-04
**Status:** M0 baseline captured; M1 implementation is in progress.

## Workspace and decision record

This repository is the standalone `iron-canvas` workspace. The workspace
manifest pins both `ironcalc_base` and `ironcalc` to version `0.8.3` at
IronCalc revision `93f3407f822df5925d8b65a23248fd40155579e9`.
[`ARCHITECTURE.md`](../ARCHITECTURE.md) is the architecture record provided
for this workspace. Its obsolete DataGrid crate entries and test command were
removed; the current workspace has no DataGrid crates.

## Fixture inventory

| Fixture | Location | Conditional-format content | Existing evidence |
| --- | --- | --- | --- |
| IronCalc CF fixture | `crates/iron-canvas-ironcalc/tests/fixtures/cf_tests.xlsx` | 54 rules across seven worksheets: `Sheet1`, `IconSets`, `Text`, `TimePeriod`, `ColorScales3`, `ColorScales2`, and `stop-if-true`. | `tests/cf_fixture.rs` asserts imported rule counts, hidden icon values, x14 data-bar overrides and negative axes, and star/circle rating results. `tests/cf_round_trip.rs` checks engine round trips. |
| Dynamic arrays demo | `web-test/demo/dynamic_arrays.xlsx` | No worksheet `conditionalFormatting` elements. | Browser harness demo. |
| Forensics demo | `web-test/demo/forensics.xlsx` | No worksheet `conditionalFormatting` elements. | Browser harness demo. |
| Sales dashboard demo | `web-test/demo/sales_dashboard.xlsx` | No worksheet `conditionalFormatting` elements. | Browser harness demo. |

The fixture provenance file records upstream revision
`93f3407f822df5925d8b65a23248fd40155579e9`, source path
`xlsx/tests/conditional_formatting/cf_tests.xlsx`, and SHA-256
`6d47b1be1937274929ba7ed1239deae3c2ac0bf9781037c4190e605110508888`.

## What is covered before migration

- Engine-backed tests confirm the fixture imports 54 rules with per-sheet
  counts `[18, 20, 3, 4, 3, 2, 4]`.
- The CF fixture tests exercise `showValue="0"` icon handling; gradient bars,
  x14 solid-bar overrides, hidden values and non-zero negative-bar axes; and
  star and quarter-circle rating values. Since M4 step F they read the
  compiler's resolved CF paint rather than the deleted model adapter.
- Core tests exercise glyph mapping and placement, data-bar geometry, rating
  layout, value suppression and coexistence of decoration categories.
- The CF fixture tests explicitly note that raw browser pixel comparison had
  not previously been captured. Their assertions validate engine output and
  compiler conversion, not Excel rendering parity.

The fixture does not establish full rendering parity. The engine evaluates
conditional-formatting rules and supplies semantic results; Canvas2D paints
the resolved style and decoration data. Existing source-audit caveats remain:
IronCalc's extended-style border lookup uses base styles for adjacent cells,
and its formatted-value path takes the number format from the base style
rather than the extended style. These are engine/API limitations, not evidence
that the canvas resolves or evaluates CF rules.

## RustyCalc browser baseline

RustyCalc captured actual Canvas2D `ImageData` for two fixture views:

- `Sheet1`: icon sets, gradient and solid bars, negative-axis bars and color
  scales.
- `IconSets`: directional and shape icon sets, indicators, custom icons and
  rating sets.

The `grid` and transparent `overlay` layers are stored separately as
gzip-compressed raw RGBA8 buffers in
`RustyCalc/docs/baselines/cf-2026-10-04/`. `metadata.json` records per-layer
dimensions and uncompressed-pixel hashes. The capture script is
`RustyCalc/scripts/capture_cf_baseline.mjs`; it loads the fixture through
RustyCalc's browser XLSX import flow before reading each canvas layer.

Capture environment: RustyCalc `HEAD` `3be0aaf` on
`integrate/canvas-next`, iron-canvas `HEAD` `67502f9` on `refactor/grid-core`,
IronCalc `93f3407f822df5925d8b65a23248fd40155579e9`, HeadlessChrome 153,
device-pixel ratio 1, default light palette (no explicit `data-theme`
attribute), browser window 1440 × 1000 CSS pixels. The actual browser viewport
was 1440 × 857 CSS pixels; both canvas layers had a 1440 × 693 pixel backing
size.

This records current browser output for future comparisons. It is not a
forced-Fresh-versus-retained parity test and does not establish output across
Canvas2D implementations or browsers.

## Verification

- `cargo test -p iron-canvas-ironcalc --test cf_fixture --locked`: 4 passed.
- `cargo test -p iron-canvas-core renderer::cell::cf --locked`: 14 passed.
- RustyCalc `cargo check --locked --target wasm32-unknown-unknown`: passed.
- `trunk build --release --minify=false --locked --public-url /`: passed.
- `node --check scripts/capture_cf_baseline.mjs`: passed.
- The capture script imported `cf_tests.xlsx` through the browser, captured
  both layers from each of the two sheets, and wrote the four raw RGBA files.
  Their byte counts and SHA-256 values are checked against the metadata.

For the RustyCalc Delta build only, Cargo's sibling `../iron-canvas` paths were
temporarily resolved to the attached iron-canvas worktree because Delta keeps
the two projects in separate managed checkouts; the manifest paths were
restored afterward. The RustyCalc IronCalc submodule was populated at its
recorded gitlink revision in that Delta worktree. No primary checkout files
were changed by these build steps.

## Native availability

The attached `iron-canvas` workspace at this revision had six crates:
`iron-canvas-core`, `iron-canvas-canvas2d`, `iron-canvas-export`,
`iron-canvas-ironcalc`, `iron-canvas-recorder`, and `iron-canvas-web` (deleted
later in M4 step E). It has
no `iron-canvas-native` crate or native worksheet consumer at this revision.
Native rendering availability is therefore **not available**; no native
result is inferred from the browser or export captures.
