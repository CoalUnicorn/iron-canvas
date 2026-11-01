# WebAssembly workbook binding

`iron-canvas-wasm-workbook` paints IronCalc's WebAssembly `Model` with Iron
Canvas. It re-exports `Model` and exports `CanvasRenderer`.

This crate is separate from `bindings/wasm`:

- `bindings/wasm` stays engine-free. It provides `SceneTarget`, scene
  rendering, theme, scroll arithmetic, and key classification.
- `bindings/wasm-workbook` adds the workbook session, model reads, navigation,
  and wheel input. It depends on the IronCalc checkout.

## Build

Build the sibling IronCalc checkout first. Then run:

```sh
wasm-pack build bindings/wasm-workbook --target web --out-dir <dir>
```

The crate is a separate Cargo workspace. It patches IronCalc path dependencies
to the checkout next to this repository. Update the paths in `Cargo.toml` when
the checkout moves.

The React example builds this crate into `examples/react/pkg`:

```sh
cd examples/react
npm run build:wasm
```

## JavaScript API

The module exports `CanvasRenderer` and `Model`. The renderer methods are:

```ts
new CanvasRenderer(canvas: HTMLCanvasElement);
resize(model: Model, width: number, height: number, dpr: number): void;
reset(): void;
setRevision(revision: number): void;
syncNavigation(model: Model): void;
render(model: Model, revealActive: boolean): void;
invalidateFonts(model: Model): void;
wheel(model: Model, deltaX: number, deltaY: number, mode: number, shift: boolean): void;
selectAt(model: Model, x: number, y: number): void;
cellRect(row: number, column: number): number[];
cellFragmentRects(row: number, column: number): number[];
```

Call `setRevision` with a higher number after any cell or dimension edit.
The revision keys the worksheet and axis caches. Revisions must not decrease;
call `reset` to start a new workbook.

Call `syncNavigation` after the host changes the selection directly. It
reconciles the controller before a following wheel event, so the event order
stays navigation then scroll. `mode` is 0 for pixels, 1 for lines, and 2 for
pages. `cellRect` returns
`[x, y, width, height]`, or an empty array when the cell is outside the
committed panes. `cellFragmentRects` returns four values per pane-clipped
fragment.
