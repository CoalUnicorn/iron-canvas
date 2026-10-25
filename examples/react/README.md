# React grid example

This example hosts the `iron-canvas` renderer in React, with a worksheet
tab bar and browser-side `.xlsx` loading.

## Run

Install the WebAssembly target and `wasm-pack`, then run:

```sh
cd examples/react
npm install
npm run dev
```

The demo opens with a small sample workbook. Use **Load workbook** to choose an
`.xlsx` file; visible worksheets are shown as tabs along the bottom. Scroll over
the grid to browse a sheet, and use `+` to add a worksheet.

`npm run build` builds the example-local workbook integration before producing
the Vite bundle. It uses the shared Canvas-only renderer from `bindings/wasm`;
the local adapter supplies IronCalc workbook data and rendering requests.

## Basic navigation

Click the grid to give it keyboard focus. A plain click selects a cell.
The host consumes supported keys before it applies the command.

| Key | Action |
| --- | --- |
| Arrow | Collapse the selection and move one logical cell |
| Shift+Arrow | Extend or contract from the fixed anchor |
| Control+Arrow, or Command+Arrow on macOS | Move to the data edge |
| Tab / Shift+Tab | Collapse a larger range and move right / left |
| Enter / Shift+Enter | Collapse a larger range and move down / up |
| PageUp / PageDown | Move by the scrolling pane height |
| Home / End | Move to column 1 / the right data edge |
| Control+Home / Control+End | Move to A1 / the used extent's bottom-right cell |
| Control+A | Select the used extent |
| Escape | Cancel pending navigation |

Use Command for Control shortcuts on macOS.
The engine skips hidden cells and resolves merged cells during movement.
A page command in a zero-height scrolling pane changes no view state.
Navigation changes no cell content and adds no content undo entry.
IME composition and unrelated input retain their keys.

This is the P3a Basic stage. Tab and Enter do not retain a multi-cell range.
Extended shortcuts, multi-area pointer selection, retained-set traversal,
range-border dragging, and move/copy transfers remain disabled for P3b.

The host queues up to 64 keys for the next animation frame.
`iron-canvas-navigation` stores this queue and applies Basic commands through
a short mutable borrow of the host's `UserModel`.
The React adapter owns browser focus and frame presentation.
Focus loss, composition entry, sheet change, and workbook replacement cancel
pending keys. A failed frame leaves unstarted commands pending without a busy
retry loop. A later key, resize, or font event can request another attempt.
An executed command never runs twice after a presentation failure.
Pointer selection waits for a successfully presented frame.

## Browser checks

Build the application, then run the DOM event tests with installed Chrome and
ChromeDriver:

```sh
npm run build
node tests/navigation.mjs
```

The test starts local Vite and ChromeDriver servers.
It checks key order, reverse Enter, unsupported modifiers, IME, input focus,
and cancellation on blur. The Wasm host tests also check engine movement,
queue limits, and failed preparation and presentation.
The workspace rendering test compares retained selection pixels with Fresh
pixels at device pixel ratios 1, 1.25, 1.5, and 2.
