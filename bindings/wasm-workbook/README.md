# WebAssembly workbook binding

`iron-canvas-wasm-workbook` paints IronCalc's WebAssembly model with Iron
Canvas. It exports a `Model` wrapper and `CanvasRenderer`.

This crate is separate from `bindings/wasm`:

- `bindings/wasm` stays engine-free. It provides `SceneTarget`, scene
  rendering, theme, scroll arithmetic, and key classification.
- `bindings/wasm-workbook` adds the workbook session, model reads, navigation,
  and wheel input. It uses the official IronCalc repository.

## Build

The crate is a member of the root Cargo workspace. Cargo fetches the pinned
official IronCalc revision. Run:

```sh
wasm-pack build bindings/wasm-workbook --target web --out-dir <dir>
```

No sibling IronCalc checkout is required.

The React example builds this crate into `examples/react/pkg`:

```sh
cd examples/react
npm run build:wasm
```

## JavaScript API

The module exports `CanvasRenderer`, `Model`, and
`columnNameFromNumber`. The model methods are:

```ts
new Model(name: string, locale: string, timezone: string, languageId: string);
static fromXlsx(bytes: Uint8Array, name: string, locale: string, timezone: string, languageId: string): Model;
getName(): string;
getSelectedSheet(): number;
getSelectedView(): SelectedView;
getWorksheetsProperties(): WorksheetProperties[];
setSelectedSheet(sheet: number): void;
setSelectedCell(row: number, column: number): void;
setSelectedRange(startRow: number, startColumn: number, endRow: number, endColumn: number): void;
setUserInput(sheet: number, row: number, column: number, value: string): void;
newSheet(): void;
```

The renderer methods are:

```ts
new CanvasRenderer(canvas: HTMLCanvasElement);
resize(model: Model, width: number, height: number, dpr: number): void;
reset(): void;
setRevision(revision: number): void;
navigate(model: Model, key: string, primary: boolean, shift: boolean, alt: boolean, altGraph: boolean): boolean;
syncNavigation(model: Model): void;
render(model: Model, revealActive: boolean): void;
invalidateFonts(model: Model): void;
wheel(model: Model, deltaX: number, deltaY: number, mode: number, shift: boolean): void;
selectAt(model: Model, x: number, y: number): void;
cellRect(row: number, column: number): number[];
cellFragmentRects(row: number, column: number): number[];
```

Call `navigate` for a grid key. It applies the shared classifier and engine
adapter, and returns false for an unbound key or modifier combination.

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
