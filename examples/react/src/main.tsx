import { useEffect, useRef, useState } from "react";
import type { ChangeEvent, KeyboardEvent, PointerEvent } from "react";
import { createRoot } from "react-dom/client";
import init, {
  CanvasRenderer,
  Model,
  columnNameFromNumber,
} from "../pkg/iron_canvas_wasm_workbook.js";
import "./style.css";

interface SheetTab {
  index: number;
  name: string;
}

interface SelectedView {
  sheet: number;
  row: number;
  column: number;
  range: [number, number, number, number];
  top_row: number;
  left_column: number;
}

interface WorksheetProperties {
  name: string;
  sheet_id: number;
  state: string;
}

const LAST_ROW = 1_048_576;
const LAST_COLUMN = 16_384;

const ARROW_KEYS = [
  "ArrowRight",
  "ArrowLeft",
  "ArrowDown",
  "ArrowUp",
] as const;

const NAVIGATION_KEYS = [...ARROW_KEYS, "Home", "End"] as const;

type ArrowKey = (typeof ARROW_KEYS)[number];

function isArrowKey(key: string): key is ArrowKey {
  return (ARROW_KEYS as readonly string[]).includes(key);
}

function isNavigationKey(key: string): boolean {
  return (NAVIGATION_KEYS as readonly string[]).includes(key);
}

function messageFromError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function selectedView(model: Model): SelectedView {
  return model.getSelectedView() as SelectedView;
}

function activeCellOf(model: Model): string {
  const view = selectedView(model);
  return `${columnNameFromNumber(view.column)}${view.row}`;
}

function visibleSheets(model: Model): SheetTab[] {
  const properties = model.getWorksheetsProperties() as WorksheetProperties[];
  return properties
    .map((sheet, index) => ({ sheet, index }))
    .filter(({ sheet }) => sheet.state === "visible")
    .map(({ sheet, index }) => ({ index, name: sheet.name }));
}

// Mirror the sample that the previous host created in Rust.
function createSampleWorkbook(): Model {
  const model = new Model("Example workbook", "en", "UTC", "en");
  const firstSheet: [number, number, string][] = [
    [1, 1, "Quarter"],
    [1, 2, "Revenue"],
    [1, 3, "Status"],
    [2, 1, "Q1"],
    [2, 2, "128"],
    [2, 3, "On track"],
    [3, 1, "Q2"],
    [3, 2, "146"],
    [3, 3, "On track"],
    [4, 1, "Q3"],
    [4, 2, "139"],
    [4, 3, "Review"],
    [5, 1, "Q4"],
    [5, 2, "171"],
    [5, 3, "On track"],
  ];
  for (const [row, column, value] of firstSheet) {
    model.setUserInput(0, row, column, value);
  }
  model.newSheet();
  model.setUserInput(1, 1, 1, "Load an .xlsx workbook to view its sheets.");
  model.setSelectedSheet(0);
  return model;
}

function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const rendererRef = useRef<CanvasRenderer | null>(null);
  const modelRef = useRef<Model | null>(null);
  const frameRef = useRef<number | null>(null);
  const composing = useRef(false);
  const [activeCell, setActiveCell] = useState("A1");
  const [ready, setReady] = useState(false);
  const [loading, setLoading] = useState(false);
  const [sheets, setSheets] = useState<SheetTab[]>([]);
  const [selectedSheet, setSelectedSheet] = useState(0);
  const [workbookName, setWorkbookName] = useState("Example workbook");
  const [error, setError] = useState<string | null>(null);

  const refreshWorkbook = (model: Model) => {
    setActiveCell(activeCellOf(model));
    setSheets(visibleSheets(model));
    setSelectedSheet(model.getSelectedSheet());
    setWorkbookName(model.getName());
  };

  // Navigation and wheel input share one paint per animation frame. The key
  // handler reconciles navigation before a following wheel event, so the
  // frame only paints the accepted position.
  const scheduleRender = () => {
    if (frameRef.current !== null) return;
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null;
      const renderer = rendererRef.current;
      const model = modelRef.current;
      if (!renderer || !model) return;
      try {
        renderer.render(model, false);
        setError(null);
      } catch (renderError) {
        setError(messageFromError(renderError));
      } finally {
        setActiveCell(activeCellOf(model));
      }
    });
  };

  useEffect(() => {
    let cancelled = false;
    let observer: ResizeObserver | undefined;
    let onWindowResize: (() => void) | undefined;
    let onFontsLoaded: (() => void) | undefined;

    const start = async () => {
      try {
        await init();
        if (cancelled || !canvasRef.current) return;

        const canvas = canvasRef.current;
        const renderer = new CanvasRenderer(canvas);
        const model = createSampleWorkbook();
        rendererRef.current = renderer;
        modelRef.current = model;
        refreshWorkbook(model);

        const resizeCanvas = () => {
          const active = modelRef.current;
          if (!active) return;
          try {
            const { width, height } = canvas.getBoundingClientRect();
            if (width > 0 && height > 0) {
              renderer.resize(active, width, height, window.devicePixelRatio || 1);
              setActiveCell(activeCellOf(active));
            }
          } catch (resizeError) {
            setError(messageFromError(resizeError));
          }
        };

        observer = new ResizeObserver(resizeCanvas);
        observer.observe(canvas);
        onWindowResize = resizeCanvas;
        window.addEventListener("resize", resizeCanvas);
        onFontsLoaded = () => {
          const active = modelRef.current;
          if (!active) return;
          try {
            renderer.invalidateFonts(active);
            setActiveCell(activeCellOf(active));
          } catch (fontError) {
            setError(messageFromError(fontError));
          }
        };
        document.fonts.addEventListener("loadingdone", onFontsLoaded);
        document.fonts.addEventListener("loadingerror", onFontsLoaded);
        resizeCanvas();
        setReady(true);
      } catch (initializationError) {
        setError(messageFromError(initializationError));
      }
    };

    void start();
    return () => {
      cancelled = true;
      observer?.disconnect();
      if (onWindowResize) window.removeEventListener("resize", onWindowResize);
      if (onFontsLoaded) {
        document.fonts.removeEventListener("loadingdone", onFontsLoaded);
        document.fonts.removeEventListener("loadingerror", onFontsLoaded);
      }
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
      rendererRef.current?.free();
      modelRef.current?.free();
      rendererRef.current = null;
      modelRef.current = null;
    };
  }, []);

  // React attaches wheel listeners to its root container as passive listeners.
  // A preventDefault call inside onWheel cannot stop the page scroll there.
  // Register a native listener with passive: false so the grid consumes the
  // gesture, and leave browser zoom gestures on the existing browser path.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const scrollGrid = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey) return;
      event.preventDefault();
      const renderer = rendererRef.current;
      const model = modelRef.current;
      if (!renderer || !model) return;

      try {
        renderer.wheel(
          model,
          event.deltaX,
          event.deltaY,
          event.deltaMode,
          event.shiftKey,
        );
        scheduleRender();
      } catch (scrollError) {
        setError(messageFromError(scrollError));
      }
    };

    canvas.addEventListener("wheel", scrollGrid, { passive: false });
    return () => canvas.removeEventListener("wheel", scrollGrid);
  }, []);

  const openWorkbook = async (event: ChangeEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const file = input.files?.[0];
    const renderer = rendererRef.current;
    if (!file || !renderer) return;

    setError(null);
    setLoading(true);
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const model = Model.fromXlsx(bytes, file.name, "en", "UTC", "en");
      renderer.reset();
      modelRef.current?.free();
      modelRef.current = model;
      renderer.render(model, false);
      refreshWorkbook(model);
    } catch (loadError) {
      setError(messageFromError(loadError));
    } finally {
      setLoading(false);
      input.value = "";
    }
  };

  const selectSheet = (index: number) => {
    const model = modelRef.current;
    if (!model) return;

    try {
      model.setSelectedSheet(index);
      setSelectedSheet(model.getSelectedSheet());
      scheduleRender();
      setError(null);
    } catch (selectionError) {
      setError(messageFromError(selectionError));
    }
  };

  const addSheet = () => {
    const model = modelRef.current;
    if (!model) return;

    try {
      model.newSheet();
      refreshWorkbook(model);
      scheduleRender();
      setError(null);
    } catch (sheetError) {
      setError(messageFromError(sheetError));
    }
  };

  const selectAdjacentSheet = (delta: number) => {
    const model = modelRef.current;
    if (!model) return;
    const visible = visibleSheets(model);
    if (visible.length === 0) return;
    const step = delta >= 0 ? 1 : -1;
    const current = visible.findIndex(
      ({ index }) => index === model.getSelectedSheet(),
    );
    const base = current === -1 ? (step > 0 ? visible.length - 1 : 0) : current;
    const next = visible[(base + step + visible.length) % visible.length].index;
    model.setSelectedSheet(next);
    setSelectedSheet(next);
  };

  // Mirror the IronCalc webapp keyboard precedence. Editing keys are ignored
  // because this example is read-only.
  const navigateGrid = (event: KeyboardEvent<HTMLCanvasElement>) => {
    const canvas = event.currentTarget;
    const renderer = rendererRef.current;
    const model = modelRef.current;
    if (
      !renderer ||
      !model ||
      event.target !== canvas ||
      document.activeElement !== canvas ||
      event.defaultPrevented ||
      composing.current ||
      event.nativeEvent.isComposing ||
      event.keyCode === 229
    ) {
      return;
    }

    const { key } = event;
    const primary = event.ctrlKey || event.metaKey;
    const { shiftKey: shift, altKey: alt } = event;

    const consume = () => {
      event.preventDefault();
      try {
        renderer.syncNavigation(model);
        setError(null);
      } catch (navigationError) {
        setError(messageFromError(navigationError));
      }
      scheduleRender();
    };

    if (primary && !shift && !alt) {
      if (isNavigationKey(key)) {
        if (key === "Home") {
          model.setSelectedCell(1, 1);
        } else if (key === "End") {
          // The webapp requests an unsupported edge direction for End.
          return;
        } else {
          model.onNavigateToEdgeInDirection(key);
        }
        consume();
      } else if (key === " ") {
        const { column } = selectedView(model);
        model.setSelectedRange(1, column, LAST_ROW, column);
        consume();
      }
      return;
    }
    if (primary && shift && !alt) return;
    if (shift && !alt && !primary) {
      if (key === " ") {
        const { row } = selectedView(model);
        model.setSelectedRange(row, 1, row, LAST_COLUMN);
        consume();
      } else if (isArrowKey(key)) {
        model.onExpandSelectedRange(key);
        consume();
      } else if (key === "Tab") {
        model.onArrowLeft();
        consume();
      }
      return;
    }
    if (alt && !primary && !shift) {
      if (key === "ArrowDown") {
        selectAdjacentSheet(1);
        consume();
      } else if (key === "ArrowUp") {
        selectAdjacentSheet(-1);
        consume();
      }
      return;
    }
    if (primary || alt || shift) return;

    switch (key) {
      case "ArrowRight":
      case "Tab":
        model.onArrowRight();
        break;
      case "ArrowLeft":
        model.onArrowLeft();
        break;
      case "ArrowDown":
      case "Enter":
        model.onArrowDown();
        break;
      case "ArrowUp":
        model.onArrowUp();
        break;
      case "Home": {
        const cell = model.getSelectedCell();
        model.setSelectedCell(cell[1], 1);
        break;
      }
      case "End": {
        const cell = model.getSelectedCell();
        model.setSelectedCell(cell[1], LAST_COLUMN);
        break;
      }
      case "PageDown":
        model.onPageDown();
        break;
      case "PageUp":
        model.onPageUp();
        break;
      default:
        return;
    }
    consume();
  };

  const selectCell = (event: PointerEvent<HTMLCanvasElement>) => {
    const canvas = event.currentTarget;
    canvas.focus();
    if (
      event.button !== 0 ||
      event.shiftKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.altKey
    ) {
      return;
    }
    const renderer = rendererRef.current;
    const model = modelRef.current;
    if (!renderer || !model) return;
    const rect = canvas.getBoundingClientRect();
    try {
      renderer.selectAt(
        model,
        Math.floor(event.clientX - rect.left),
        Math.floor(event.clientY - rect.top),
      );
      scheduleRender();
      setError(null);
    } catch (selectionError) {
      setError(messageFromError(selectionError));
    }
  };

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            IC
          </span>
          <div>
            <h1>Iron Canvas</h1>
            <p>React grid example</p>
          </div>
        </div>
        <div className="header-actions">
          <span className="workbook-name" title={workbookName}>
            {workbookName}
          </span>
          <input
            ref={fileInputRef}
            className="file-input"
            type="file"
            accept=".xlsx,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            aria-label="Choose an Excel workbook"
            onChange={(event) => void openWorkbook(event)}
          />
          <button
            className="load-button"
            type="button"
            disabled={!ready || loading}
            onClick={() => fileInputRef.current?.click()}
          >
            {loading ? "Loading…" : "Load workbook"}
          </button>
        </div>
      </header>

      {error && (
        <div className="error-banner" role="alert">
          {error}
        </div>
      )}

      <section className="grid-region" aria-label="Worksheet grid">
        <canvas
          ref={canvasRef}
          aria-label={`${workbookName} worksheet, ${activeCell}`}
          role="grid"
          tabIndex={0}
          onKeyDown={navigateGrid}
          onPointerDown={selectCell}
          onCompositionStart={() => {
            composing.current = true;
          }}
          onCompositionEnd={() => {
            composing.current = false;
          }}
        />
      </section>

      <nav className="sheet-bar" aria-label="Workbook sheets">
        <button
          className="add-sheet"
          type="button"
          aria-label="Add sheet"
          title="Add sheet"
          disabled={!ready}
          onClick={addSheet}
        >
          +
        </button>
        <div className="sheet-tabs" role="tablist" aria-label="Worksheets">
          {sheets.map((sheet) => (
            <button
              key={sheet.index}
              className={`sheet-tab${sheet.index === selectedSheet ? " selected" : ""}`}
              type="button"
              role="tab"
              aria-selected={sheet.index === selectedSheet}
              onClick={() => selectSheet(sheet.index)}
            >
              {sheet.name}
            </button>
          ))}
        </div>
        <span className="sheet-count">
          {sheets.length} {sheets.length === 1 ? "sheet" : "sheets"}
        </span>
      </nav>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
