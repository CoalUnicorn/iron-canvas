# Bindings

Language-specific adapters for Iron Canvas live here. Each adapter should keep
its host-facing API separate from the renderer's core Rust API.

- `wasm/` contains the `wasm-bindgen` Canvas2D scene renderer. It accepts
  Iron Canvas scene data and has no workbook or XLSX API.
- `nodejs/` and `python/` are reserved for future Node.js and Python adapters;
  those bindings are not implemented yet.

The React example's IronCalc workbook integration remains local to
`examples/react/wasm`; it is example glue, not part of the shared bindings.
