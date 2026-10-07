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
