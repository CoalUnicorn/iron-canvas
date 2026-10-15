# Crate review

Review date: 2026-10-09  
Workspace: current checkout at `b898b430d5251982079374bbd2f1a854caada60b` when the review began.  
Scope: Rust crates under `crates/`, with direct inspection of the workbook export boundary.  
Review posture: findings only; no production code was changed for this review.

## Fix status: 2026-10-15

The findings below describe the original checkout. Branch `crate-review` fixes
findings 1–14 against local `main` (`6cbaf30`). The branch review started at
`a0157b0`. Additional review fixes are recorded in the branch history.

| Finding | Current result | Evidence |
| --- | --- | --- |
| 1 | Fixed | Disjoint extreme rectangle regression in `geometry/pixel_rect.rs` |
| 2 | Fixed | Offscreen circle coverage regression in `scene/retained.rs` |
| 3 | Fixed | Source spaces and non-breaking spaces are retained by core layout tests |
| 4 | Fixed | Hidden terminal cells retain selection edges, including exact viewport and frozen boundaries |
| 5 | Fixed | Hidden terminal cells use the visible selection corner; fully hidden ranges have no handle |
| 6 | Fixed | Odd rectangle dimensions retain fractional text centers |
| 7 | Fixed | Column auto-fit includes the compiler's CF icon reservation |
| 8 | Fixed | Row auto-fit sums the merged anchor's column widths |
| 9 | Fixed | Compiler test preserves explicit black text on the dark theme |
| 10 | Fixed | Column auto-fit measures newline-separated lines individually |
| 11 | Fixed | Zero and tiny placements at maximum coordinates do not overflow, including CF decorations |
| 12 | Fixed | Row and column divider intents match their resize targets |
| 13 | Fixed | Commit restores `copy`; browser pixels prove replacement after context changes and size reset |
| 14 | Fixed | SVG and PDF fixture exports evaluate absent numeric, absent untyped, and stale formula caches |
| 15–18 | Open | Export encoding, arc, font-style, and whitespace work remains |

Workspace tests, clippy, formatting, the Wasm workspace check, export feature
tests, and all seven browser tests passed during this review. These results
apply to the reviewed source and fixtures. P1 remains open for findings 15–18.
See [the branch review](docs/reviews/2026-10-15-crate-review-base.md) and
[the PR draft](PR-draft-fix-base.md).

## Findings

### `iron-canvas-core`

1. **P1 — Disjoint rectangles can panic during intersection.** [pixel_rect.rs:30](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-core/src/geometry/pixel_rect.rs:30)  
   `then_some` eagerly evaluates its value, including `right - left` and `bottom - top`, before it checks the overlap condition. With overflow checks enabled, intersecting a far-offscreen rectangle such as `(i32::MIN, 0, 1, 1)` with `(2, 0, 100, 96)` panics instead of returning `None`.

2. **P1 — Offscreen paint bounds can panic in retained coverage.** [retained.rs:358](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-core/src/scene/retained.rs:358)  
   This path has the same eager `then_some` issue. A circle whose bounds are far outside the scene can saturate during float-to-integer conversion, then overflow when the clipped dimensions are subtracted. Skip empty intersections before calculating dimensions.

3. **P2 — Wrapped text loses source whitespace and splits non-breaking spaces.** [text_layout.rs:216](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-core/src/text_layout.rs:216)  
   `split_whitespace` discards leading, trailing, and repeated spaces, then the wrapper inserts one ordinary space between words. It also treats a non-breaking space as a break. For example, wrapped `"  a  b  "` becomes `"a b"`; `"a b"` can split into two lines.

4. **P2 — A hidden final row or column removes the selection edge.** [scene_geometry/composition.rs:109](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-core/src/scene_geometry/composition.rs:109)  
   The band’s `last_index` is taken from the last visible `AxisCell`. If the requested range ends in a hidden axis cell, `GridGeometry::fragments` concludes that the visible fragment is clipped and marks its true right or bottom edge false. Selecting rows and columns 1 through 3 when cell 3 is hidden paints a visible 40×40 fragment without its outer bottom and right outlines.

5. **P2 — Autofill has no handle when a range ends at a hidden cell.** [scene_geometry/composition.rs:324](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-core/src/scene_geometry/composition.rs:324)  
   `autofill_handle` looks up the last cell with `cell_rect`. Hidden cells have no sampled rectangle, so a range ending at one returns `None`, even when earlier visible cells provide a clear lower-right boundary.

6. **P3 — Centered text loses half-pixel placement.** [text_layout.rs:140](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-core/src/text_layout.rs:140)  
   `PixelRect::center` uses integer division. Centering text in a 101×61 rectangle places it at `(50, 30)` instead of `(50.5, 30.5)`.

### `iron-canvas-ironcalc`

7. **P2 — Column auto-fit ignores conditional-formatting icon width.** [autofit.rs:107](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-ironcalc/src/autofit.rs:107)  
   Auto-fit reads the style but measures only the formatted text. For a value with a visible conditional-formatting icon, painting reserves 25 px for the icon while auto-fit returns a width that leaves only 23 px for 40 px of text. The result clips a value that auto-fit should show.

8. **P2 — Row auto-fit uses the anchor column width for merged cells.** [autofit.rs:144](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-ironcalc/src/autofit.rs:144)  
   For merged A1:C1 with three 60 px columns, the renderer wraps against the full 180 px merged width, but auto-fit measures against A’s 60 px width. A value that paints on one line can therefore get a height sized for four lines.

9. **P2 — Explicit black font colors are replaced by the theme.** [compiler/text.rs:77](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-ironcalc/src/compiler/text.rs:77)  
   The theme fallback treats both an unspecified color and resolved `#000000` as missing. On a dark theme, explicitly black workbook text becomes the theme’s light text color, changing workbook formatting.

10. **P2 — Column auto-fit measures newline-separated values as one line.** [autofit.rs:111](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-ironcalc/src/autofit.rs:111)  
    The complete formatted string is sent to `measure_text_width`, so a value such as `abcd\nabcd` is sized as one long line. The compiler paints two lines, making auto-fit choose about twice the required width.

11. **P3 — A valid zero-width placement can panic in icon layout.** [compiler/cf.rs:282](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-ironcalc/src/compiler/cf.rs:282)  
    `prepare_cells` accepts zero-width rectangles, but `icon_slot` calls `PixelRect::inset` before checking the dimensions. A zero-width rectangle at `x = i32::MAX` overflows during inset arithmetic, even when the cell has no conditional formatting.

### `iron-canvas`

12. **P2 — Resize target and resize intent select different columns at a divider.** [interaction.rs:356](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas/src/interaction.rs:356)  
    At the right edge of column A, `resize_target` identifies column A, while `resize_intent` calls `hit_test`; the boundary belongs to column B, so the returned intent resizes B. A focused integration probe reproduced this mismatch.

### `iron-canvas-canvas2d`

13. **P2 — Commit can blend transparent pixels with the old frame.** [scene.rs:194](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-canvas2d/src/scene.rs:194)  
    `commit_frame` copies the staged canvas using the target context's current composite mode. The public `canvas()` accessor lets a host obtain that context, and resizing the canvas resets its state to `source-over`. A transparent area in the staged frame then blends with the previous frame instead of replacing it, leaving stale pixels. Set the composite mode to `copy` before publishing the staged canvas.

### `iron-canvas-export`

14. **P1 — XLSX export can display missing or stale formula results.** [workbook.rs:147](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-export/src/workbook.rs:147)  
    The export path loads the workbook and prepares it without evaluating formulas. A workbook with `A1 = 1+1` and a missing numeric cached result exported as `0`; a missing cache with no numeric cell type exported as `#ERROR!`. Stale cached results are also at risk. Evaluate after loading, or define and apply an explicit cached-value policy.

15. **P1 — PDF encoding corrupts accented text and currency symbols.** [common/escape.rs:23](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-export/src/common/escape.rs:23)  
    The code emits UTF-8 bytes as separate PDF character codes, while the font resource does not declare the WinAnsi encoding assumed by the comments. A PDF text probe containing `café £ € '` was extracted as `cafˆ' ´£ ‹ ’`. This also makes measurement disagree with rendered glyphs.

16. **P2 — SVG arc paths omit the arc start point.** [svg/painter.rs:157](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-export/src/svg/painter.rs:157)  
    The emitted `A` command assumes the SVG path cursor is already at the arc start. An arc-only path begins with `A`, which has no current point; an arc after an unrelated move connects from the wrong point. Emit the start move or connecting segment as required.

17. **P2 — SVG and PDF export discard bold and italic cell styles.** [svg/painter.rs:417](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-export/src/svg/painter.rs:417), [pdf/painter.rs:488](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-export/src/pdf/painter.rs:488)  
    The compiler supplies font weight and style, but SVG emits neither and PDF always uses regular Helvetica. Bold, italic, and bold-italic workbook text therefore renders as regular text.

18. **P2 — SVG collapses whitespace that layout metrics count.** [svg/painter.rs:428](/home/mmm/01_Dev/iron-canvas/crates/iron-canvas-export/src/svg/painter.rs:428)  
    Text is emitted without whitespace preservation. SVG collapses repeated and leading spaces, but the metrics count them. A value such as `A  B` changes on export and can shift when centered or right-aligned.

## Validation

- `cargo test --workspace --locked` passed on the reviewed checkout.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` passed.
- `cargo fmt --all -- --check` passed.
- `cargo check --workspace --target wasm32-unknown-unknown --locked` passed.
- Export feature checks passed for no features, SVG only, and PDF only.
- Focused probes reproduced the resize mismatch and the findings reported by the core, IronCalc, and export reviews.
- Browser pixel comparisons were not run. The native/Canvas2D source review found one commit-mode defect, but its retained raster behavior remains unverified in a browser.

These checks establish that the current code builds and its existing test suite passes. They do not negate the behavior gaps reproduced above.
