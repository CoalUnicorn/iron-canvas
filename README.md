# Iron Canvas

Spreadsheets renderer using IronCalc as engine.

## Overview

Worksheet viewport is composed trough [IronCalc](https://github.com/ironcalc/IronCalc) engine via  [compiler](crates/iron-canvas-ironcalc/src/compiler.rs).

## Examples

GTK4 workbook viewer.
``` sh
cargo run -r --example linux_xlsx_viewer_gtk4 <file.xlsx>
```

The viewer uses the shared navigation and scroll controller:

- Wheel and touchpad input scroll in pixels and keep sub-cell offsets.
- Arrow keys, Tab, Enter, Home, End, Page Up, Page Down, and Control+Arrow
  move the selection through the engine.
- Click selects a cell or header. Shift+click extends from the fixed anchor.
- A status line shows the active cell, the selection, and the pixel origin.

React workbook viewer.
*Note:* The example embeds IronCalc's WebAssembly `Model` and paints its view
with Iron Canvas. It uses the sibling `IronCalc` checkout for the engine
binding.
```sh
cd examples/react
npm install
npm run dev
```
