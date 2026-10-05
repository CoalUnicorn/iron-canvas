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
        || previous.viewport.first_cell.col != request.viewport.first_cell.col
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
    if a.row_bands[0].first_index != previous.viewport.first_cell.row
        || b.row_bands[0].first_index != request.viewport.first_cell.row
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
    if region.width <= 0
        || region.height <= 0
        || [region.left(), region.right(), region.top(), region.bottom()]
            .iter()
            .any(|v| (f64::from(*v) * scale).fract() != 0.0)
    {
        return None;
    }
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
