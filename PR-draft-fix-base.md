# Fix crate review findings 1–14

This branch repairs the geometry panics, text layout, selection boundaries,
auto-fit, resize intents, Canvas2D publication, and formula export behavior
reported in [`CRATE_REVIEW.md`](CRATE_REVIEW.md), findings 1–14.
Target: local `main` at `6cbaf30`.

## Changes

- Reject empty rectangle intersections before arithmetic. Retain source
  whitespace, non-breaking spaces, and fractional text centers.
  Commits: [fbb75da], [5e101d4], [d5a8fc5], [703893e].
- Preserve selection edges and autofill corners when a range ends in hidden
  cells, including exact viewport and frozen boundaries.
  Commits: [5c918ca], [ddf6a10], [9d4f8a5].
- Include CF icon space and explicit lines in column auto-fit. Use the full
  merged width in row auto-fit. Preserve explicit black font colors.
  Commits: [e051cf6], [3cd0ed2], [7fb737f], [3d47ef8], [9d4f8a5].
- Guard tiny CF placements before inset and prevent icon cursor overflow.
  Commits: [c8cf182], [9d4f8a5].
- Resolve resize intents from the divider target. Restore Canvas2D `copy`
  before publishing staged pixels.
  Commits: [da1a94a] (resize), [9d4f8a5] (Canvas2D and regressions).
- Evaluate formulas after XLSX import. Add original missing/stale cache
  fixtures for SVG and PDF. Keep export tests valid for each feature variant.
  Commits: [a0157b0], [9d4f8a5].
- Record the review and P1 progress in the roadmap and architecture guidance.
  Commit: [9d4f8a5] — `fix(canvas): close review gaps`.

[fbb75da]: https://github.com/CoalUnicorn/iron-canvas/commit/fbb75dac1adde62e830b996df5cfbe9656b8ff07
[5e101d4]: https://github.com/CoalUnicorn/iron-canvas/commit/5e101d4a67755aac06592aa871609c5b5bc911de
[d5a8fc5]: https://github.com/CoalUnicorn/iron-canvas/commit/d5a8fc5bd2c621449d2f84799b6d02b4d8f828c8
[703893e]: https://github.com/CoalUnicorn/iron-canvas/commit/703893ecabb34f28154eb96a266d34d7abc82b5b
[5c918ca]: https://github.com/CoalUnicorn/iron-canvas/commit/5c918ca86e786f5f65b5611bd47ae3eca82a8d28
[ddf6a10]: https://github.com/CoalUnicorn/iron-canvas/commit/ddf6a101611659dedcf7486b61fbeb0d66e0d3fb
[e051cf6]: https://github.com/CoalUnicorn/iron-canvas/commit/e051cf66b6a2fa36a584e94b898d60d2a4b57ff9
[3cd0ed2]: https://github.com/CoalUnicorn/iron-canvas/commit/3cd0ed29f2ac5e52dec244b4fe595d92bb734493
[7fb737f]: https://github.com/CoalUnicorn/iron-canvas/commit/7fb737fc487ce556caef39eca181db7266b713dd
[3d47ef8]: https://github.com/CoalUnicorn/iron-canvas/commit/3d47ef8770047566b2f5efd830612b28897307e0
[c8cf182]: https://github.com/CoalUnicorn/iron-canvas/commit/c8cf182dc184f9551ccf3b36e03b0c8db1738994
[da1a94a]: https://github.com/CoalUnicorn/iron-canvas/commit/da1a94af1396537b29588cad90b1088d90fb985d
[a0157b0]: https://github.com/CoalUnicorn/iron-canvas/commit/a0157b09450a7f69d3cf30c5a6374c4628b70695
[9d4f8a5]: https://github.com/CoalUnicorn/iron-canvas/commit/9d4f8a5907bf33f0bdbcc906dc38052209a2fe47

## Validation

- Workspace tests, clippy with warnings denied, and formatting passed.
- The Wasm workspace check passed.
- Export tests passed with no default features, SVG only, and PDF only.
- Seven headless Chrome tests passed with the matching Wasm runner.
  These include transparent-frame replacement and retained/Fresh pixel checks.
- Three XLSX cache fixtures produced evaluated formula results in SVG and PDF.

See [the branch review](docs/reviews/2026-10-15-crate-review-base.md) for commands,
the additional review repairs, and validation limits.

## Compatibility and remaining work

`AutoFitError` adds `RowExtent` and `MergedCells`. Consumers with exhaustive
matches must handle these variants. Resize intent points identify exact header
dividers; hosts retain the `resize_target` result for a drag with tolerance.
XLSX file export now recalculates formulas, including volatile formulas.

Findings 15–18 remain open: PDF encoding, SVG arcs, bold/italic export, and SVG
whitespace. Roadmap P1 remains partial. Cache release 1 is a later phase.

## Local preparation

The source fixes, fixtures, and review documents are committed through
[9d4f8a5]. Unrelated `.gitignore`, workbook, and session artifacts remain local.
