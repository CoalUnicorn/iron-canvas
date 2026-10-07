# Bindings

Language-specific adapters for Iron Canvas live here. Each adapter should keep
its host-facing API separate from the renderer's core Rust API.

- `wasm/` contains the current `wasm-bindgen` adapter, used by the React
  example.
- `nodejs/` and `python/` are reserved for future Node.js and Python adapters;
  those bindings are not implemented yet.
