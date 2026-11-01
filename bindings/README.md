# Bindings

Language-specific adapters for Iron Canvas live here. Each adapter should keep
its host-facing API separate from the renderer's core Rust API.

- `wasm/` contains the `wasm-bindgen` Canvas2D scene renderer. It accepts
  Iron Canvas scene data and provides model-free keyboard classification and
  scroll arithmetic. Its rlib exports `SceneTarget` for linked hosts; disable
  the `js` feature when another cdylib owns the JavaScript API. It has no
  workbook or XLSX API.
- `wasm-workbook/` contains the IronCalc workbook binding used by the React
  example. It depends on the official IronCalc git revision and on `wasm/`'s
  `SceneTarget`. It is a root workspace member.
- `nodejs/` and `python/` are reserved for future Node.js and Python adapters;
  those bindings are not implemented yet.

The React example uses `bindings/wasm-workbook`; it has no local Rust crate.
