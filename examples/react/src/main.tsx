import { useEffect, useRef, useState } from "react";
import type { ChangeEvent, WheelEvent } from "react";
import { createRoot } from "react-dom/client";
import init, { Workbook } from "../wasm/pkg/iron_canvas_react_example.js";
import "./style.css";

interface SheetTab {
  index: number;
  name: string;
}

function messageFromError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const workbookRef = useRef<Workbook | null>(null);
  const [ready, setReady] = useState(false);
  const [loading, setLoading] = useState(false);
  const [sheets, setSheets] = useState<SheetTab[]>([]);
  const [selectedSheet, setSelectedSheet] = useState(0);
  const [workbookName, setWorkbookName] = useState("Example workbook");
  const [error, setError] = useState<string | null>(null);

  const refreshWorkbook = (workbook: Workbook) => {
    setSheets(workbook.getSheets());
    setSelectedSheet(workbook.selectedSheet());
    setWorkbookName(workbook.workbookName());
  };

  useEffect(() => {
    let cancelled = false;
    let observer: ResizeObserver | undefined;
    let onWindowResize: (() => void) | undefined;

    const start = async () => {
      try {
        await init();
        if (cancelled || !canvasRef.current) return;

        const canvas = canvasRef.current;
        const workbook = new Workbook(canvas);
        workbookRef.current = workbook;
        refreshWorkbook(workbook);

        const resizeCanvas = () => {
          try {
            const { width, height } = canvas.getBoundingClientRect();
            if (width > 0 && height > 0) {
              workbook.resize(width, height, window.devicePixelRatio || 1);
            }
          } catch (resizeError) {
            setError(messageFromError(resizeError));
          }
        };

        observer = new ResizeObserver(resizeCanvas);
        observer.observe(canvas);
        onWindowResize = resizeCanvas;
        window.addEventListener("resize", resizeCanvas);
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
      workbookRef.current = null;
    };
  }, []);

  const openWorkbook = async (event: ChangeEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const file = input.files?.[0];
    if (!file || !workbookRef.current) return;

    setError(null);
    setLoading(true);
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const workbook = workbookRef.current;
      workbook.loadXlsx(bytes, file.name);
      refreshWorkbook(workbook);
    } catch (loadError) {
      setError(messageFromError(loadError));
    } finally {
      setLoading(false);
      input.value = "";
    }
  };

  const selectSheet = (index: number) => {
    const workbook = workbookRef.current;
    if (!workbook) return;

    try {
      workbook.selectSheet(index);
      setSelectedSheet(workbook.selectedSheet());
      setError(null);
    } catch (selectionError) {
      setError(messageFromError(selectionError));
    }
  };

  const addSheet = () => {
    const workbook = workbookRef.current;
    if (!workbook) return;

    try {
      workbook.addSheet();
      refreshWorkbook(workbook);
      setError(null);
    } catch (sheetError) {
      setError(messageFromError(sheetError));
    }
  };

  const scrollGrid = (event: WheelEvent<HTMLCanvasElement>) => {
    event.preventDefault();
    const workbook = workbookRef.current;
    if (!workbook) return;

    const scale = event.deltaMode === 1 ? 1 : event.deltaMode === 2 ? 20 : 1 / 24;
    const step = (delta: number) =>
      delta === 0
        ? 0
        : Math.sign(delta) * Math.max(1, Math.round(Math.abs(delta) * scale));
    const verticalDelta = step(event.deltaY);
    const horizontalDelta = step(event.deltaX);
    const rowDelta = event.shiftKey ? 0 : verticalDelta;
    const columnDelta = event.shiftKey ? verticalDelta || horizontalDelta : horizontalDelta;

    if (rowDelta === 0 && columnDelta === 0) return;
    try {
      workbook.scrollBy(rowDelta, columnDelta);
      setError(null);
    } catch (scrollError) {
      setError(messageFromError(scrollError));
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
          aria-label={`${workbookName} worksheet`}
          onWheel={scrollGrid}
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
