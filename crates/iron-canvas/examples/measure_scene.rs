//! Run with `cargo run -p iron-canvas --example measure_scene --release --locked`.
#[cfg(not(target_arch = "wasm32"))]
#[path = "support/allocator.rs"]
mod allocator;
#[cfg(not(target_arch = "wasm32"))]
#[path = "support/measurement.rs"]
mod measurement;
fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let start = std::time::Instant::now();
        print!(
            "{}",
            measurement::run(
                iron_canvas_native::NativeSceneBackend::new(1.25).unwrap(),
                || start.elapsed().as_secs_f64() * 1_000_000.0
            )
        );
    }
}
