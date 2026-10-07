# WebAssembly binding

This `wasm-bindgen` crate renders Iron Canvas scenes to an HTML canvas. It has
no workbook, formula, sheet-management, or XLSX API and does not depend on
IronCalc.

## JavaScript API

Build it from the repository root with:

```sh
wasm-pack build bindings/wasm --target web
```

The generated module exports `CanvasRenderer`:

```ts
const renderer = new CanvasRenderer(canvas);
renderer.resize(width, height, devicePixelRatio);
renderer.renderScene(scene);
```

`resize` clears the canvas when its dimensions change; render a scene after
resizing. The scene is an object with a `clip` rectangle and ordered `passes`.
Rectangles use `{ left, top, width, height }`. Each primitive is tagged by its
`type`; supported types are `rectFill`, `gradient`, `fillPath`, `strokePath`,
`circle`, `line`, `textLine`, `clip`, `endClip`, and `text`. Path commands use
`move`, `line`, `quad`, `cubic`, `arc`, and `close`. Invalid geometry and
unbalanced clips are rejected before painting.

The tagged object shapes are:

```ts
type Point = { x: number; y: number };
type Rect = { left: number; top: number; width: number; height: number };
type PathCommand =
  | { type: "move" | "line"; to: Point }
  | { type: "quad"; control: Point; to: Point }
  | { type: "cubic"; control1: Point; control2: Point; to: Point }
  | { type: "arc"; center: Point; radius: number; startAngle: number; sweepAngle: number }
  | { type: "close" };
type Primitive =
  | { type: "rectFill"; rect: Rect; color: string }
  | { type: "gradient"; rect: Rect; from: string; to: string }
  | { type: "fillPath"; path: PathCommand[]; color: string }
  | {
      type: "strokePath";
      path: PathCommand[];
      color: string;
      style: {
        width: number;
        cap: "butt" | "round" | "square";
        join: "miter" | "round" | "bevel";
        miterLimit: number;
        dash: number[];
      };
    }
  | { type: "circle"; cx: number; cy: number; radius: number; color: string }
  | {
      type: "line";
      line:
        | { type: "horizontal"; from: number; to: number; y: number }
        | { type: "vertical"; from: number; to: number; x: number };
      color: string;
      width: number;
    }
  | { type: "textLine"; x1: number; x2: number; y: number; color: string; width: number }
  | { type: "clip"; rect: Rect }
  | { type: "endClip" }
  | {
      type: "text";
      text: string;
      x: number;
      y: number;
      fontCss: string;
      color: string;
      align: "start" | "center" | "end";
      baseline: "top" | "middle" | "bottom" | "alphabetic";
    };
type Scene = { clip: Rect; passes: Primitive[][] };
```

The React example has separate workbook glue in `examples/react/wasm`; that
crate is not the shared Canvas binding.
