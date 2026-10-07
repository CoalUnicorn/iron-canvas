# React example

A small Vite + React host for the `iron-canvas` WebAssembly renderer. The
workbook and spreadsheet rendering stay in Rust; React owns the canvas element
and reports its size and device-pixel ratio.

## Run

Install Node.js 18 or newer, Rust with the `wasm32-unknown-unknown` target, and
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/).

```sh
rustup target add wasm32-unknown-unknown
npm install
npm run dev
```

Open the local URL printed by Vite. `npm run dev` builds the Rust WebAssembly
package before starting the development server. `npm run build` builds the
WebAssembly package, type-checks the React app, and produces `dist/`.

The renderer's Canvas2D backend has fixed metrics for a session. The example
creates a new session when the `ResizeObserver` reports a new canvas size or
device-pixel ratio.
