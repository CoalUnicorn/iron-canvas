# Plan: minimal Linux Excel grid viewer

**Status:** implemented as `linux_xlsx_viewer.rs`; smoke-tested with the
conditional-formatting fixture. Window resizing and HiDPI behavior still need
follow-up verification.

## Goal

Add a small Linux GUI example that opens an `.xlsx` workbook and displays a
read-only spreadsheet grid. Reuse the existing IronCalc import and
iron-canvas rendering pipeline rather than building a second spreadsheet
renderer.

## MVP

- Add `linux_xlsx_viewer.rs` as a Linux-only example in this directory.
- Take an `.xlsx` path as a command-line argument, so the first version does
  not need a file-picker UI.
- Open the workbook and show its first worksheet in a GTK4 window, with a
  worksheet selector for the other sheets. Start at cell A1 and render the
  visible area; resize the render viewport with the window.
- Support basic scrolling by updating the viewport's first visible cell and
  rendering again.
- Keep the workbook read-only. Editing, formula-bar UI, and workbook saving
  are out of scope.

## Design

Use GTK4's `DrawingArea` for the window and presentation, and paint the Cairo
image produced by `iron_canvas_native::NativeSceneBackend`. GTK4 is the
recommended choice because the native backend already renders to Cairo
surfaces; a different GUI toolkit would require extra pixel/texture
presentation work. There is no GUI framework currently used by the examples.

The data/render flow should be:

1. Load the workbook with IronCalc's XLSX importer and wrap it as a
   `UserModel`.
2. Create a `CanvasSession<NativeSceneBackend>` and a render request for the
   first worksheet, starting at `CellCoord { row: 1, col: 1 }`, with the
   current drawing area's dimensions and the default light theme.
3. On initial display, resize, or scroll, update the request and call
   `CanvasSession::render`.
4. In the GTK draw callback, paint the committed Cairo surface exposed through
   `CanvasSession::backend().surface()`. Keep rendering separate from drawing
   so ordinary GTK redraws can reuse the committed frame.

The relevant existing pieces are `CanvasSession` and `RenderRequest` in
`crates/iron-canvas/src/lib.rs`, and `NativeSceneBackend::surface` in
`crates/iron-canvas-native/src/lib.rs`.

## Implementation

1. GTK4 and the XLSX importer are Linux-targeted example dependencies, not
   normal library dependencies. The native renderer remains a target-specific
   dev-dependency of `iron-canvas`.
2. The example validates the input path, loads the workbook, and reports
   import/setup errors clearly.
3. The GTK window stores the workbook, render session, and current viewport in
   application state.
4. Worksheet selection, resize, and scroll events update the render request
   and queue a redraw.
5. The conditional-formatting fixture smoke test is described below.

## Running

GTK4 development libraries and `pkg-config` are required on Linux. Run the
viewer with:

```sh
cargo run -p iron-canvas --example linux_xlsx_viewer -- \
  crates/iron-canvas-ironcalc/tests/fixtures/cf_tests.xlsx
```

Use the worksheet selector to view `IconSets`.

## Validation

- Build the Linux example with `cargo check -p iron-canvas --example
  linux_xlsx_viewer`.
- Run it against `crates/iron-canvas-ironcalc/tests/fixtures/cf_tests.xlsx`,
  select `IconSets`, and confirm the icons, cell values, headers, and grid
  lines appear.
- Resize and scroll the window; verify the displayed viewport updates without
  changing the workbook.
- Try a missing path and an invalid workbook; verify the user gets a useful
  error rather than a panic.
- Check that Cairo surface scale and GTK drawing-area coordinates align,
  including on a HiDPI display.

## Dependencies and tradeoffs

GTK4 development libraries are needed on Linux to build the example. The
resolved `gtk4-rs` version is `0.9.7`; Cargo unifies its Cairo dependency with
the renderer's existing `cairo-rs 0.20.12`, so committed surfaces can be
painted directly. Keep the example Linux-gated so GTK is not required to build
the renderer or its other targets.

This first version deliberately has no file chooser. If the basic viewer is
useful, that can follow as a separate addition.
