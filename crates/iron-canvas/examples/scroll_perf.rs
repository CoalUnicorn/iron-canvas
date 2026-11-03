//! Measure committed frame time during worksheet scrolling.
//!
//! Run on an idle machine and pin one core:
//! `taskset -c 7 cargo run --release -p iron-canvas --example scroll_perf -- <file.xlsx> [width] [height] [mode] [frames] [plain]`
//!
//! `mode` is `row`, `pixel`, or `both`. `row` moves one row per frame.
//! `pixel` moves five logical pixels per frame. `plain` omits the selection
//! overlay. `partial` reports retained-copy planning with a backend that
//! accepts partial updates and does not paint. The default backend paints
//! with Cairo and reports `Fresh` because it has no partial update support.

#[cfg(target_os = "linux")]
use std::{collections::BTreeMap, env, hint::black_box, time::Instant};

#[cfg(target_os = "linux")]
use iron_canvas::{
    CanvasSession, OverlayState, RenderBackend, RenderRequest, RevisionToken, TextMetricsKey,
    WorksheetCache,
};
#[cfg(target_os = "linux")]
use iron_canvas_core::{
    CanvasTheme, CellCoord,
    painter::TextMetrics,
    scene::{GridScene, retained::SceneUpdate},
    scene_geometry::GridRange,
};
#[cfg(target_os = "linux")]
use iron_canvas_ironcalc::WorksheetViewport;
#[cfg(target_os = "linux")]
use iron_canvas_native::NativeSceneBackend;
#[cfg(target_os = "linux")]
use ironcalc::import::load_from_xlsx;
#[cfg(target_os = "linux")]
use ironcalc_base::UserModel;

#[cfg(target_os = "linux")]
const PIXELS_PER_STEP: i32 = 5;

#[cfg(target_os = "linux")]
struct Options {
    path: String,
    width: i32,
    height: i32,
    mode: String,
    frames: i32,
    plain: bool,
    partial: bool,
    verbose: bool,
}

#[cfg(target_os = "linux")]
impl Options {
    fn parse() -> Self {
        let mut args = env::args().skip(1);
        let path = args.next().unwrap_or_else(|| {
            eprintln!(
                "usage: scroll_perf <file.xlsx> [width] [height] [row|pixel|both] [frames] [plain] [partial] [verbose]"
            );
            std::process::exit(2);
        });
        let number = |value: Option<String>, fallback: i32| {
            value
                .and_then(|value| value.parse().ok())
                .unwrap_or(fallback)
        };
        let width = number(args.next(), 1600);
        let height = number(args.next(), 1000);
        let mode = args.next().unwrap_or_else(|| "both".to_owned());
        let frames = number(args.next(), 40);
        let flags: Vec<String> = args.collect();
        Self {
            path,
            width,
            height,
            mode,
            frames,
            plain: flags.iter().any(|flag| flag == "plain"),
            partial: flags.iter().any(|flag| flag == "partial"),
            verbose: flags.iter().any(|flag| flag == "verbose"),
        }
    }
}

#[cfg(target_os = "linux")]
fn scroll_request(options: &Options) -> RenderRequest {
    RenderRequest {
        sheet: 0,
        revision: RevisionToken {
            workbook_id: 1,
            revision: 1,
        },
        viewport: WorksheetViewport::aligned(
            CellCoord { row: 1, col: 1 },
            options.width,
            options.height,
        ),
        theme: CanvasTheme::light(),
        overlays: if options.plain {
            OverlayState::default()
        } else {
            OverlayState {
                selection: vec![GridRange {
                    first: CellCoord { row: 1, col: 1 },
                    last: CellCoord { row: 1, col: 1 },
                }],
                active_cell: Some(CellCoord { row: 1, col: 1 }),
                ..OverlayState::default()
            }
        },
    }
}

#[cfg(target_os = "linux")]
fn pixel_position(step: i32, row_height: i32) -> (i32, i32) {
    let pixels = step * PIXELS_PER_STEP;
    (1 + pixels / row_height, pixels % row_height)
}

/// Report retained-copy plans without painting. Cairo has no partial update
/// support, so the default backend always returns `Fresh`.
#[cfg(target_os = "linux")]
struct PartialProbe {
    metrics: NativeSceneBackend,
}

#[cfg(target_os = "linux")]
impl RenderBackend for PartialProbe {
    type StagedFrame = ();
    type Error = std::convert::Infallible;

    fn metrics(&self) -> &dyn TextMetrics {
        self.metrics.metrics()
    }

    fn stage_frame(&mut self, _scene: &GridScene) -> Result<(), Self::Error> {
        Ok(())
    }

    fn commit_frame(&mut self, _staged: ()) -> Result<(), Self::Error> {
        Ok(())
    }

    fn supports_partial_updates(&self) -> bool {
        true
    }

    fn retained_scale(&self) -> Option<f64> {
        Some(1.0)
    }

    fn stage_update(
        &mut self,
        _scene: &GridScene,
        _update: &SceneUpdate,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn main() {
    let options = Options::parse();
    let load_started = Instant::now();
    let mut workbook = load_from_xlsx(&options.path, "en", "UTC", "en")
        .map(UserModel::from_model)
        .unwrap_or_else(|error| {
            eprintln!("could not open {}: {error}", options.path);
            std::process::exit(1);
        });
    workbook.evaluate();
    let load_ms = load_started.elapsed().as_secs_f64() * 1e3;
    println!(
        "workbook={} viewport={}x{} overlay={} backend={} load_ms={load_ms:.1}",
        options.path,
        options.width,
        options.height,
        if options.plain { "none" } else { "selection" },
        if options.partial { "partial" } else { "cairo" }
    );
    let modes: Vec<&str> = match options.mode.as_str() {
        "row" => vec!["row"],
        "pixel" => vec!["pixel"],
        "both" => vec!["row", "pixel"],
        other => {
            eprintln!("unknown mode {other}; use row, pixel, or both");
            std::process::exit(2);
        }
    };
    for mode in modes {
        if options.partial {
            run_mode(
                &options,
                &workbook,
                mode,
                PartialProbe {
                    metrics: NativeSceneBackend::new(1.0).expect("metrics backend"),
                },
            );
        } else {
            run_mode(
                &options,
                &workbook,
                mode,
                NativeSceneBackend::new(1.0).expect("cairo backend"),
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn run_mode<B: RenderBackend>(options: &Options, workbook: &UserModel<'_>, mode: &str, backend: B)
where
    B::Error: std::fmt::Debug,
{
    let mut session = CanvasSession::new(backend);
    let mut cache = WorksheetCache::new(1);
    let mut request = scroll_request(options);
    let mut timings = Vec::with_capacity(options.frames.max(0) as usize);
    let mut strategies = BTreeMap::new();
    let mut cells = 0;
    let mut styles = 0;

    // The warm-up frame establishes text metrics, the first pane, and the row
    // height used by the pixel sequence. Measurement starts at the next step.
    let warm = session
        .render_cached(&mut cache, workbook, &request, TextMetricsKey::new(1, 0))
        .expect("warm-up frame");
    let row_height = if mode == "pixel" {
        session.cell_rect(1, 1).map_or(1, |rect| rect.height.max(1))
    } else {
        1
    };
    black_box(warm);

    for step in 1..=options.frames {
        match mode {
            "row" => {
                request.viewport.scroll.first_cell = CellCoord {
                    row: 1 + step,
                    col: 1,
                };
                request.viewport.scroll.offset_y = 0;
            }
            _ => {
                let (row, offset) = pixel_position(step, row_height);
                request.viewport.scroll.first_cell = CellCoord { row, col: 1 };
                request.viewport.scroll.offset_y = offset;
            }
        }
        let started = Instant::now();
        let outcome = session
            .render_cached(&mut cache, workbook, &request, TextMetricsKey::new(1, 0))
            .expect("frame");
        let elapsed = started.elapsed().as_secs_f64() * 1e3;
        if options.verbose {
            println!(
                "frame step={step} {elapsed:.1}ms cells={} styles={} strategy={:?}",
                outcome.cell_count, outcome.extended_style_reads, outcome.strategy
            );
        }
        timings.push(elapsed);
        *strategies
            .entry(format!("{:?}", outcome.strategy))
            .or_insert(0) += 1;
        cells = cells.max(outcome.cell_count);
        styles = styles.max(outcome.extended_style_reads);
    }

    timings.sort_by(f64::total_cmp);
    let percentile = |numerator: usize, denominator: usize| {
        timings[(timings.len() * numerator / denominator).min(timings.len() - 1)]
    };
    let count = strategies
        .iter()
        .map(|(name, count)| format!("{name}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "mode={mode} frames={} cells={cells} styles={styles} median={:.1}ms p90={:.1}ms max={:.1}ms strategies={count}",
        timings.len(),
        percentile(1, 2),
        percentile(9, 10),
        timings[timings.len() - 1],
    );
}

#[cfg(not(target_os = "linux"))]
fn main() {}
