//! Measure axis construction with the workspace release profile.
//! Run: `cargo run --release -p iron-canvas-ironcalc --example axis_size_perf -- file.xlsx`.

use std::{env, hint::black_box, time::Instant};

use iron_canvas_ironcalc::{
    Axis, AxisSizeBuilder, AxisSizeProgress, AxisSource, get_column_sizes, get_row_sizes,
};
use ironcalc::import::load_from_xlsx;
use ironcalc_base::UserModel;

fn main() {
    let path = env::args().nth(1).expect("supply an XLSX path");
    let model =
        UserModel::from_model(load_from_xlsx(&path, "en", "UTC", "en").expect("load workbook"));
    let sheet = model.get_model().workbook.worksheet(0).unwrap();
    println!(
        "records: rows={}, columns={}",
        sheet.rows.len(),
        sheet.cols.len()
    );
    for axis in [Axis::Row, Axis::Column] {
        let mut samples = Vec::with_capacity(100);
        for iteration in 0..110 {
            let start = Instant::now();
            black_box(match axis {
                Axis::Row => get_row_sizes(black_box(&model), 0),
                Axis::Column => get_column_sizes(black_box(&model), 0),
            })
            .unwrap();
            if iteration >= 10 {
                samples.push(start.elapsed());
            }
        }
        samples.sort_unstable();
        println!("{axis:?}: median={:?}, p90={:?}", samples[50], samples[90]);

        // Constructor time is separate. Report each step so a full-build
        // average cannot hide one large sort or vector move.
        for budget in [64, 4096] {
            let source = AxisSource::resolve(&model, 0, 1, 1).unwrap();
            let mut builder =
                AxisSizeBuilder::with_slice_operations(&model, source, axis, budget).unwrap();
            let mut steps = Vec::new();
            loop {
                let start = Instant::now();
                let progress = builder.step(black_box(&model), source).unwrap();
                steps.push(start.elapsed());
                if progress == AxisSizeProgress::Complete {
                    break;
                }
            }
            black_box(builder.into_sizes().unwrap());
            steps.sort_unstable();
            println!(
                "  budget={budget}: calls={}, median={:?}, max={:?}",
                steps.len(),
                steps[steps.len() / 2],
                steps.last().unwrap()
            );
        }
    }
}
