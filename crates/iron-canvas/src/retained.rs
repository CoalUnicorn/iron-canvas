use crate::{PreparedWorksheetFrame, RenderRequest};
use iron_canvas_core::{
    PixelRect, Point,
    scene::{
        GridScene,
        retained::{SceneUpdate, translated_overlap_matches},
    },
};
use iron_canvas_ironcalc::PreparedWorksheet;

/// Only one-axis vertical scroll in an unfrozen, unmerged sheet is enabled.
/// Other changes use fingerprint damage or Fresh.
pub(crate) fn scroll_plan(
    old: &PreparedWorksheetFrame,
    new: &PreparedWorksheet,
    previous: &RenderRequest,
    request: &RenderRequest,
    scale: f64,
) -> Option<SceneUpdate> {
    if previous.revision != request.revision
        || previous.sheet != request.sheet
        || previous.theme != request.theme
        || previous.overlays != request.overlays
        || request.overlays != Default::default()
        || previous.viewport.scroll.first_cell.col != request.viewport.scroll.first_cell.col
        // Only vertical movement is eligible. Horizontal pixel copies remain a
        // separate optimization, so the column offset must not change.
        || previous.viewport.scroll.offset_x != request.viewport.scroll.offset_x
    {
        return None;
    }
    let a = old.worksheet.geometry.as_ref()?;
    let b = new.geometry.as_ref()?;
    if a.bounds != b.bounds
        || a.origin != b.origin
        || a.column_bands != b.column_bands
        || a.row_bands.len() != 1
        || b.row_bands.len() != 1
        || a.column_bands.len() != 1
        || b.column_bands.len() != 1
    {
        return None;
    }
    // A single band can also mean a frozen band consumes the entire viewport.
    if a.row_bands[0].first_index != previous.viewport.scroll.first_cell.row
        || b.row_bands[0].first_index != request.viewport.scroll.first_cell.row
    {
        return None;
    }
    let row = b.row_bands[0].cells.iter().find_map(|r| {
        a.row_bands[0]
            .cells
            .iter()
            .find(|o| o.index == r.index)
            .map(|o| (r, o))
    })?;
    let dy = row.0.start - row.1.start;
    if dy == 0 || (f64::from(dy) * scale).fract() != 0.0 {
        return None;
    }
    let body = PixelRect {
        top_left: b.origin,
        width: b.bounds.right() - b.origin.x,
        height: b.bounds.bottom() - b.origin.y,
    };
    let shifted = PixelRect {
        top_left: Point {
            x: body.left(),
            y: body.top() + dy,
        },
        ..body
    };
    // Retain an interior rectangle. Repaint boundary pixels, headers and exposed strips.
    let region = body.intersection(shifted)?.inset(4, 4);
    // The row header width comes from font metrics, so an inset edge is not
    // always an integral device pixel. Move each edge inward to the nearest
    // copyable coordinate before asking for a blit.
    let region = integral_region(region, scale)?;
    if !translated_overlap_matches(
        &old.worksheet.scene,
        &new.scene,
        region,
        Point { x: 0, y: dy },
    ) {
        return None;
    }
    let source = PixelRect {
        top_left: Point {
            x: region.left(),
            y: region.top() - dy,
        },
        ..region
    };
    Some(SceneUpdate::Blit {
        source,
        destination: region.top_left,
        repaint: outside(&new.scene, region),
    })
}
/// Nearest coordinate inward from `edge` that lands on an integral device pixel.
fn integral_edge(edge: i32, step: i32, scale: f64) -> Option<i32> {
    (0..=64).find_map(|offset| {
        let value = edge.checked_add(step.checked_mul(offset)?)?;
        ((f64::from(value) * scale).fract() == 0.0).then_some(value)
    })
}

/// Shrink a copy region so every edge is an integral device pixel.
fn integral_region(region: PixelRect, scale: f64) -> Option<PixelRect> {
    let left = integral_edge(region.left(), 1, scale)?;
    let top = integral_edge(region.top(), 1, scale)?;
    let right = integral_edge(region.right(), -1, scale)?;
    let bottom = integral_edge(region.bottom(), -1, scale)?;
    (right > left && bottom > top).then(|| PixelRect {
        top_left: Point { x: left, y: top },
        width: right - left,
        height: bottom - top,
    })
}

fn outside(scene: &GridScene, r: PixelRect) -> Vec<PixelRect> {
    let b = scene.clip;
    [
        PixelRect {
            top_left: b.top_left,
            width: b.width,
            height: r.top() - b.top(),
        },
        PixelRect {
            top_left: Point {
                x: b.left(),
                y: r.bottom(),
            },
            width: b.width,
            height: b.bottom() - r.bottom(),
        },
        PixelRect {
            top_left: Point {
                x: b.left(),
                y: r.top(),
            },
            width: r.left() - b.left(),
            height: r.height,
        },
        PixelRect {
            top_left: Point {
                x: r.right(),
                y: r.top(),
            },
            width: b.right() - r.right(),
            height: r.height,
        },
    ]
    .into_iter()
    .filter(|r| r.width > 0 && r.height > 0)
    .collect()
}
