# Review of crate review fixes 1–14

Date: 2026-10-15.
Branch: `crate-review`.
Review baseline: `a0157b0` against local `main` at `6cbaf30`.
Engine revision: `7e6ae21c0f163cacf56b9d47acf777454bfaa243`.
No remote refs were refreshed.

## Result

The branch and the additional review fixes address findings 1–14 in
[`CRATE_REVIEW.md`](../../CRATE_REVIEW.md).
The implementation follows roadmap P1. It keeps model writes in the host.
It keeps generic geometry in core and spreadsheet policy in the compiler.
No separate feature design is required for these repairs to existing behavior.

Findings 15–18 remain open. P1 is partial. P2 remains blocked by P1.
The branch does not include the .NET or workbook-review tool integration.
The [PR draft](../../PR-draft-fix-base.md) describes the complete proposed diff.

## Findings repaired during this review

| Priority | Finding | Evidence before repair | Repair |
| --- | --- | --- | --- |
| P2 | Finding 13 was not implemented | `commit_frame` still called `draw_image` without restoring `copy`. Commit `da1a94a` changed resize intent code instead | Restore `copy` at publication. A browser regression clears opaque old pixels with a transparent frame after a context change or backing-width reset |
| P2 | Findings 4 and 5 missed exact viewport boundaries | The new 40×40 regression produced `[true, true, false, false]` edge flags for a range ending in hidden cells | Keep the last sampled logical index. At an exact boundary, inspect hidden cells until the next visible cell or axis limit. Apply this to frozen and scrolling bands |
| P2 | The finding 11 fix still allowed tiny valid placements to panic | A one-pixel placement at `i32::MAX - 1` overflowed in `PixelRect::inset`. A decorated five-pixel placement also overflowed while advancing the icon cursor | Reject decoration interiors that cannot contain positive area before inset. Saturate icon cursor advancement. Cover zero through eight pixels with icons and data bars |
| P2 | The finding 14 regression broke export feature variants | `cargo test -p iron-canvas-export --no-default-features --locked` failed because `fs` was unavailable. PDF-only tests also contained a required SVG export | Gate the SVG-specific unit test and import on `feature = "svg"` |
| P2 | The finding 8 regression did not detect the original defect | `twelvechars` cannot wrap. The layout keeps that word on one line at both 60 px and 180 px | Use `one two three`, which needs three lines at the anchor width and one line at the merged width |

These repairs retain the existing public signatures.
`AutoFitError::RowExtent` and `MergedCells` were already added by the branch.
They add variants to a public enum. Consumers with exhaustive matches must
handle them.

## Coverage added or clarified

- Core tests cover hidden terminal cells at exact viewport and frozen boundaries.
  A separate test proves that the next visible cell is not included.
- Compiler tests cover zero and one-pixel placements at maximum coordinates.
  CF unit tests exercise tiny placements with actual icons and data bars.
- Session tests cover row and column divider intents, zero extent, negative
  extent rejection, and cell-body rejection.
- The existing browser harness now tests transparent-frame publication.
  All retained/Fresh comparisons remain in that harness.
- Export integration tests load three original XLSX fixtures. They cover a
  missing untyped cache, a missing numeric cache, and a stale numeric cache.
  Both SVG and PDF must contain the evaluated result `2` for `1+1`.
  The fixture README records provenance and hashes.
- The merged-width auto-fit test uses breakable text so the old anchor-only
  width calculation fails the regression.
- Architecture notes, local canvas guidance, the original review status, and
  roadmap P1 progress now match the source.

## Validation

The following checks passed on the reviewed source:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo check --workspace --target wasm32-unknown-unknown --locked
cargo test -p iron-canvas-export --no-default-features --locked
cargo test -p iron-canvas-export --no-default-features --features svg --locked
cargo test -p iron-canvas-export --no-default-features --features pdf --locked
```

The browser gate passed all seven tests in headless Chrome:

```sh
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=/tmp/iron-canvas-wbg-0120/bin/wasm-bindgen-test-runner \
  WASM_BINDGEN_USE_BROWSER=1 WASM_BINDGEN_USE_CHROME=1 \
  cargo test -p iron-canvas --target wasm32-unknown-unknown --test cf_fixture_canvas2d --locked
```

The runner version is `0.2.120`, matching the lockfile.
The installed default runner was `0.2.125`, so a matching runner was built in
`/tmp`. Browser execution required an unsandboxed run because the sandbox
prevented the test server from binding. The unsandboxed run passed.
The retained sequence test compares pixels at DPR 1, 1.25, 1.3, and 1.5.
The other fixture tests use DPR 1 and 1.25 as specified in their source.
This review does not certify other scales, manual host input, Windows, or
fidelity for arbitrary workbooks.

## Change boundaries

The review started with no staged changes and no modified tracked Rust files.
The existing `.gitignore` change and untracked workbook/session artifacts were
preserved. The initial review did not commit or publish changes.

The user then requested a commit and commit hashes in the PR draft.
The commit includes the complete source fixes, regression fixtures, architecture
notes, review status, roadmap, this review, and the local canvas guidance.
The PR draft records that commit and the earlier fix commits.
Other existing documents, workbook/session artifacts, and generated
workbook-review output remain outside this change.
