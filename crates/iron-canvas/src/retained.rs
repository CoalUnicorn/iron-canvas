use crate::{PreparedWorksheetFrame, RenderRequest};
use iron_canvas_core::{
    PixelRect, Point,
    scene::{
        GridScene,
        retained::{SceneUpdate, pass_bounds, translated_overlap_matches},
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
        || old.worksheet.model_passes != new.model_passes
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
    // Overlay paint is not part of the model copy. The old bitmap also holds
    // overlay pixels, so reserve both scenes' overlay bounds for repaint.
    let mut holes = pass_bounds(&old.worksheet.scene, old.worksheet.model_passes);
    holes.extend(pass_bounds(&new.scene, new.model_passes));
    let mut pieces = vec![region];
    for hole in holes {
        let mut next = Vec::with_capacity(pieces.len() + 3);
        for piece in pieces {
            next.extend(subtract(piece, hole));
            if next.len() > MAX_OVERLAY_PIECES {
                return None;
            }
        }
        pieces = next;
        if pieces.is_empty() {
            return None;
        }
    }
    // Blit one largest overlay-free piece. Everything outside it repaints.
    let copy = *pieces
        .iter()
        .max_by_key(|piece| i64::from(piece.width) * i64::from(piece.height))?;
    let copy = integral_region(copy, scale)?;
    let old_model = model_passes(&old.worksheet.scene, old.worksheet.model_passes);
    let new_model = model_passes(&new.scene, new.model_passes);
    if !translated_overlap_matches(&old_model, &new_model, copy, Point { x: 0, y: dy }) {
        return None;
    }
    let source = PixelRect {
        top_left: Point {
            x: copy.left(),
            y: copy.top() - dy,
        },
        ..copy
    };
    let repaint = outside(&new.scene, copy);
    Some(SceneUpdate::Blit {
        source,
        destination: copy.top_left,
        repaint,
    })
}

/// Upper bound on disjoint pieces after overlay subtraction.
const MAX_OVERLAY_PIECES: usize = 16;

/// Return the scene passes that contain worksheet content.
fn model_passes(scene: &GridScene, count: usize) -> GridScene {
    GridScene {
        clip: scene.clip,
        passes: scene.passes[..count].to_vec(),
    }
}

/// Remove one rectangle from another and return disjoint pieces.
fn subtract(rect: PixelRect, hole: PixelRect) -> Vec<PixelRect> {
    let Some(hit) = rect.intersection(hole) else {
        return vec![rect];
    };
    let mut out = Vec::with_capacity(4);
    let mut push = |x: i32, y: i32, width: i32, height: i32| {
        if width > 0 && height > 0 {
            out.push(PixelRect {
                top_left: Point { x, y },
                width,
                height,
            });
        }
    };
    push(rect.left(), rect.top(), rect.width, hit.top() - rect.top());
    push(
        rect.left(),
        hit.bottom(),
        rect.width,
        rect.bottom() - hit.bottom(),
    );
    push(rect.left(), hit.top(), hit.left() - rect.left(), hit.height);
    push(
        hit.right(),
        hit.top(),
        rect.right() - hit.right(),
        hit.height,
    );
    out
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
