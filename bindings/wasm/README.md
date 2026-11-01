# WebAssembly binding

This `wasm-bindgen` crate renders Iron Canvas scenes to an HTML canvas. It has
no workbook, formula, sheet-management, or XLSX API and does not depend on
IronCalc.

## JavaScript API

Build it from the repository root with:

```sh
wasm-pack build bindings/wasm --target web
```

The generated module exports `CanvasRenderer` and `themeFromElement`:

```ts
const renderer = new CanvasRenderer(canvas);
renderer.resize(width, height, devicePixelRatio);
renderer.renderScene(scene);      // retained: repaints only changed bands
renderer.renderSceneFresh(scene); // repaints every pixel
renderer.invalidateMetrics();     // after browser fonts load or change
```

`resize` clears the canvas when its dimensions change; render a scene after
resizing. `renderScene` compares the scene with the committed fingerprint. An
unchanged scene keeps the committed pixels. A changed scene repaints only the
damaged bands. `invalidateMetrics` drops memoized text measurements and forces
the next scene to repaint. Call it after a font change.

`themeFromElement(element)` reads CSS custom properties from the computed
style of `element`. It returns these camelCase keys:

```ts
type Theme = {
  gridColor: string;
  gridSeparatorColor: string;
  headerBg: string;
  headerBorderColor: string;
  headerTextColor: string;
  headerSelectedBg: string;
  headerSelectedColor: string;
  defaultTextColor: string;
  errorTextColor: string;
  selectionColor: string;
  cellBg: string;
  pointing: string;
  selectionFill: string;
  pointingTint: string;
};
```

The palette derives from the `--palette-*` custom properties. Rebuild the
scene with the new colors after a theme change.

The scene is an object with a `clip` rectangle and ordered `passes`.
Rectangles use `{ left, top, width, height }`. Each primitive is tagged by its
`type`; supported types are `rectFill`, `rectFillRun`, `gradient`, `fillPath`,
`strokePath`, `circle`, `line`, `textLine`, `clip`, `endClip`, and `text`. Path
commands use `move`, `line`, `quad`, `cubic`, `arc`, and `close`. A
`rectFillRun` takes ordered `edges` from `rect.left` through `rect.right`. Each
adjacent pair paints one rectangle. Invalid geometry and unbalanced clips are
rejected before painting.

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
  | { type: "rectFillRun"; rect: Rect; edges: number[]; color: string }
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

## Navigation and scroll

The module exports a model-free keyboard classifier and pixel scroll
arithmetic. The host applies each returned intent to its own model. This crate
performs no engine writes.

```ts
type Modifiers = {
  primary?: boolean;
  shift?: boolean;
  alt?: boolean;
  altGraph?: boolean;
};

type NavigationIntent =
  | { type: "step"; direction: Direction }
  | { type: "extendStep"; direction: Direction }
  | { type: "dataEdge"; direction: Direction }
  | { type: "traverse"; axis: "rows" | "columns"; direction: "forward" | "backward" }
  | { type: "page"; direction: "up" | "down" }
  | { type: "rowStart" }
  | { type: "rowEnd" }
  | { type: "sheetStart" }
  | { type: "usedEnd" }
  | { type: "selectUsedRange" }
  | { type: "cancel" };

type Direction = "up" | "down" | "left" | "right";

classifyNavigation(
  key: string,
  modifiers?: Modifiers,
  profile?: "basic" | "extended",
): NavigationIntent | null;
```

`key` uses `KeyboardEvent.key` values. The classifier returns `null` for an
unknown key or an unsupported modifier combination. The basic profile is the
default.

Scroll arithmetic uses rounded extents. Build one axis for each direction.
Indices are one-based and inclusive. Runs hold extents that differ from
`defaultExtent`; gaps use the default.

```ts
class ScrollWindow {
  constructor(first: number, last: number);
}

class ScrollAxis {
  constructor(
    defaultExtent: number,
    lastIndex: number,
    runs?: { first: number; last: number; extent: number }[],
  );
  readonly defaultExtent: number;
  readonly lastIndex: number;
  readonly totalExtent: number;
}

class AxisScroll {
  readonly first: number;
  readonly offset: number;
}

class ScrollPosition {
  constructor(firstRow: number, firstCol: number, offsetX: number, offsetY: number);
  readonly firstRow: number;
  readonly firstCol: number;
  readonly offsetX: number;
  readonly offsetY: number;
}

scrollingExtent(axis: ScrollAxis, window: ScrollWindow): number;
normalizeAxis(first: number, offset: number, axis: ScrollAxis, window: ScrollWindow): AxisScroll;
moveAxis(first: number, offset: number, delta: number, axis: ScrollAxis, window: ScrollWindow, pane: number): AxisScroll;
clampAxisEnd(first: number, offset: number, axis: ScrollAxis, window: ScrollWindow, pane: number): AxisScroll;
normalizeScroll(position: ScrollPosition, rows: ScrollAxis, rowWindow: ScrollWindow, columns: ScrollAxis, columnWindow: ScrollWindow): ScrollPosition;
moveScroll(position: ScrollPosition, dx: number, dy: number, rows: ScrollAxis, rowWindow: ScrollWindow, columns: ScrollAxis, columnWindow: ScrollWindow, paneWidth: number, paneHeight: number): ScrollPosition;
```

`moveScroll` uses positive deltas to move content left and up. `ScrollWindow`
uses an inclusive `last` index.

The React example consumes `bindings/wasm-workbook`. That crate links
`SceneTarget` from this crate with the `js` feature disabled, so it owns the
only JavaScript surface.

## Rust hosts

Linked Rust hosts can use `SceneTarget` from the rlib. It owns the Canvas2D
backend, the fixed metrics, and the retained fingerprint. It implements
`RenderBackend`, so it can serve as a `CanvasSession` backend.

```rust
let target = iron_canvas_wasm::SceneTarget::new(canvas, metrics)?;
let session = CanvasSession::new(target);
```

`SceneTarget::render_scene` stages and commits a typed `GridScene` with
retained updates. `render_scene_fresh` repaints every pixel.

Disable the `js` feature when another `wasm-bindgen` crate in the same build
defines its own JavaScript API. The feature gates every exported class and
function so `wasm-bindgen` sees one host surface:

```toml
iron-canvas-wasm = { path = "bindings/wasm", default-features = false }
```
