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
*Note:* Example includes wasm adapter for IronCalc to update workbook model view.  
```sh
cd examples/react
npm install
npm run dev
```
