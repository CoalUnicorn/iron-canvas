# Iron Canvas

Spreadsheets renderer using IronCalc as engine.

## Overview

Worksheet viewport is composed trough [IronCalc](https://github.com/ironcalc/IronCalc) engine via  [compiler](crates/iron-canvas-ironcalc/src/compiler.rs).

## Examples

GTK4 workbook viewer.
``` sh
cargo run -r --example linux_xlsx_viewer_gtk4 <file.xlsx>
```

React workbook viewer.
*Note:* The example embeds IronCalc's WebAssembly `Model` and paints its view
with Iron Canvas. It uses the sibling `IronCalc` checkout for the engine
binding.
```sh
cd examples/react
npm install
npm run dev
```
