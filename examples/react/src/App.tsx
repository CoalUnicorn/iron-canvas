import { useEffect, useRef, useState } from "react";
import init, { ReactExample } from "./wasm/pkg/iron_canvas_react.js";

export function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState<string | null>("Loading WebAssembly renderer…");

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    let disposed = false;
    let renderer: ReactExample | undefined;
    let observer: ResizeObserver | undefined;
    let pendingFrame = 0;

    const draw = () => {
      cancelAnimationFrame(pendingFrame);
      pendingFrame = requestAnimationFrame(() => {
        if (!renderer || disposed) return;
        const { width, height } = canvas.getBoundingClientRect();
        if (width === 0 || height === 0) return;

        try {
          renderer.render(width, height, window.devicePixelRatio || 1);
          setError(null);
        } catch (cause) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    };

    void init()
      .then(() => {
        if (disposed) return;
        renderer = new ReactExample(canvas);
        observer = new ResizeObserver(draw);
        observer.observe(canvas);
        draw();
      })
      .catch((cause: unknown) => {
        if (!disposed) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });

    return () => {
      disposed = true;
      observer?.disconnect();
      cancelAnimationFrame(pendingFrame);
      renderer?.free();
    };
  }, []);

  return (
    <main className="page">
      <header className="page-header">
        <div>
          <p className="eyebrow">Rust renderer · React host</p>
          <h1>IronCanvas</h1>
          <p className="subtitle">
            A spreadsheet canvas rendered in Rust and mounted by React.
          </p>
        </div>
        <span className="badge">WebAssembly</span>
      </header>

      <section className="workbook" aria-label="Spreadsheet example">
        <div className="sheet-toolbar">
          <span className="sheet-tab">Quarterly results</span>
          <span className="sheet-note">Resize the page to resize the canvas</span>
        </div>
        <div className="canvas-wrap">
          <canvas
            ref={canvasRef}
            className="sheet-canvas"
            aria-label="Rendered spreadsheet grid"
          />
          {error && <p className="canvas-message" role="status">{error}</p>}
        </div>
      </section>

      <p className="footnote">
        React manages the canvas lifecycle; the Rust session prepares and paints
        each worksheet frame.
      </p>
    </main>
  );
}
