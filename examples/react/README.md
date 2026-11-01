# React grid example

This example hosts IronCalc's WebAssembly `Model` in React and paints its
committed view with `iron-canvas`.

## Run

The example uses the sibling IronCalc checkout for the engine binding:

```sh
cd examples/react
npm install
npm run dev
```

The local `wasm/` crate re-exports IronCalc's `Model` and adds
`CanvasRenderer`. The renderer paints through `SceneTarget` from the shared
`bindings/wasm` crate with its `js` feature disabled. IronCalc owns the
workbook, the selected view, and navigation state.

> The example depends on the `IronCalc` checkout next to this repository and
> patches IronCalc to that local path. Update the paths in
> `wasm/Cargo.toml` when the checkout moves.

The demo opens with a small sample workbook. Use **Load workbook** to choose an
`.xlsx` file; visible worksheets are shown as tabs along the bottom. Scroll over
the grid to browse a sheet, and use `+` to add a worksheet.

`npm run build` builds the example-local WebAssembly module before producing
the Vite bundle. The module links `SceneTarget` from `bindings/wasm` with the
`js` feature disabled, so the example's API is the only JavaScript surface.

## Navigation

Navigation mirrors
[`Workbook/useKeyboardNavigation.ts`](https://github.com/ironcalc/IronCalc/blob/main/webapp/IronCalc/src/components/Workbook/useKeyboardNavigation.ts)
in the IronCalc webapp. The React host calls `Model` methods directly. The
renderer paints one animation frame for the latest view.

| Key | Action |
| --- | --- |
| Arrow | Move one logical cell |
| Tab | Move right |
| Enter | Move down |
| Shift+Arrow | Extend or contract the selection |
| Shift+Tab | Move left |
| Control+Arrow, or Command+Arrow on macOS | Move to the data edge |
| Control+Home | Select A1 |
| Home / End | Move to column 1 / column 16,384 |
| PageUp / PageDown | Move by the engine's window height |
| Control+Space | Select the active column |
| Shift+Space | Select the active row |
| Alt+ArrowDown / Alt+ArrowUp | Select the next / previous sheet |
| Escape | No grid binding |

Shift+Enter is unbound, matching the webapp. Control+End is unbound because
the webapp asks the engine for an unsupported edge direction.

The engine moves the active cell, extends merged ranges, skips hidden cells,
and adjusts the scroll origin. A navigation command changes no cell content.
Wheel input keeps the continuous pixel scroll position and shares the same
frame scheduler as navigation.

## Browser checks

Build the application, then run the DOM event tests with installed Chrome and
ChromeDriver:

```sh
npm run build
node tests/navigation.mjs
```

The test starts local Vite and ChromeDriver servers. It checks engine
navigation keys, modifiers, sheet switching, IME, focus, pointer selection, and
wheel repaint.
