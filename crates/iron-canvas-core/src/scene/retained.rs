//! Paint fingerprints and conservative full-width damage bands.
use super::{GridScene, Primitive};
use crate::{
    Line, PixelRect, Point,
    geometry::path::{PathCmd, PointF},
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

/// Height of one damage-fingerprint band, in logical pixels.
///
/// A change marks whole bands, so a smaller band buys a tighter damage
/// rectangle at the cost of more per-band hashes. Every band index derives
/// from this value.
const BAND_HEIGHT: i32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStrategy {
    Fresh,
    Reuse,
    Damage,
    Blit,
}

/// The backend must stage all copies and repairs before publishing any pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneUpdate {
    Fresh,
    Reuse,
    Damage(Vec<PixelRect>),
    Blit {
        source: PixelRect,
        destination: Point,
        repaint: Vec<PixelRect>,
    },
}
impl SceneUpdate {
    pub fn strategy(&self) -> RenderStrategy {
        match self {
            Self::Fresh => RenderStrategy::Fresh,
            Self::Reuse => RenderStrategy::Reuse,
            Self::Damage(_) => RenderStrategy::Damage,
            Self::Blit { .. } => RenderStrategy::Blit,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneFingerprint {
    pub bands: Vec<(PixelRect, u64)>,
}
impl SceneFingerprint {
    pub fn new(scene: &GridScene) -> Self {
        let mut bands = Vec::new();
        let mut top = scene.clip.top();
        while top < scene.clip.bottom() {
            let rect = PixelRect {
                top_left: Point {
                    x: scene.clip.left(),
                    y: top,
                },
                width: scene.clip.width,
                height: BAND_HEIGHT.min(scene.clip.bottom() - top),
            };
            bands.push((rect, DefaultHasher::new()));
            top += rect.height;
        }
        visit(scene, scene.clip, Point::default(), |op, clip, coverage| {
            let mut h = DefaultHasher::new();
            hash_paint(op, clip, coverage, Point::default(), &mut h);
            let value = h.finish();
            let first = ((coverage.top() - scene.clip.top()) / BAND_HEIGHT) as usize;
            let last = ((coverage.bottom() - 1 - scene.clip.top()) / BAND_HEIGHT) as usize;
            for (_, h) in &mut bands[first..=last] {
                value.hash(h);
            }
        });
        let bands = bands.into_iter().map(|(r, h)| (r, h.finish())).collect();
        Self { bands }
    }
    pub fn damage(&self, previous: &Self) -> SceneUpdate {
        if self.bands.len() != previous.bands.len()
            || self
                .bands
                .iter()
                .zip(&previous.bands)
                .any(|(a, b)| a.0 != b.0)
        {
            return SceneUpdate::Fresh;
        }
        let mut changed: Vec<PixelRect> = Vec::new();
        for ((rect, hash), (_, old)) in self.bands.iter().zip(&previous.bands) {
            if hash == old {
                continue;
            }
            if let Some(last) = changed
                .last_mut()
                .filter(|last| last.bottom() == rect.top())
            {
                last.height += rect.height;
            } else {
                changed.push(*rect);
            }
        }
        if changed.len() == 1
            && self
                .bands
                .first()
                .is_some_and(|b| changed[0].top() == b.0.top())
            && self
                .bands
                .last()
                .is_some_and(|b| changed[0].bottom() == b.0.bottom())
        {
            return SceneUpdate::Fresh;
        }
        if changed.is_empty() {
            SceneUpdate::Reuse
        } else {
            SceneUpdate::Damage(changed)
        }
    }
}

/// Verify a translated overlap from resolved paint, including effective clips.
/// The caller must also require integral device-pixel movement.
pub fn translated_overlap_matches(
    old: &GridScene,
    new: &GridScene,
    region: PixelRect,
    delta: Point,
) -> bool {
    region_hash(old, region, delta) == region_hash(new, region, Point::default())
}

fn shifted(mut r: PixelRect, delta: Point) -> PixelRect {
    r.top_left.x += delta.x;
    r.top_left.y += delta.y;
    r
}
fn rect_hash(r: PixelRect, h: &mut impl Hasher) {
    [r.left(), r.top(), r.width, r.height].hash(h);
}
fn floats(values: &[f64], h: &mut impl Hasher) {
    for v in values {
        v.to_bits().hash(h);
    }
}
fn point(p: PointF, delta: Point, h: &mut impl Hasher) {
    floats(&[p.x + f64::from(delta.x), p.y + f64::from(delta.y)], h);
}
fn path_hash(path: &[PathCmd], delta: Point, h: &mut impl Hasher) {
    path.len().hash(h);
    for cmd in path {
        std::mem::discriminant(cmd).hash(h);
        match cmd {
            PathCmd::Move(p) | PathCmd::Line(p) => point(*p, delta, h),
            PathCmd::Quad(a, b) => {
                point(*a, delta, h);
                point(*b, delta, h);
            }
            PathCmd::Cubic(a, b, c) => {
                point(*a, delta, h);
                point(*b, delta, h);
                point(*c, delta, h);
            }
            PathCmd::Arc(a) => {
                point(a.center, delta, h);
                floats(&[a.radius, a.start_angle, a.sweep_angle], h);
            }
            PathCmd::Close => {}
        }
    }
}
fn region_hash(scene: &GridScene, region: PixelRect, delta: Point) -> u64 {
    let mut h = DefaultHasher::new();
    visit(scene, region, delta, |op, clip, coverage| {
        hash_paint(op, clip, coverage, delta, &mut h)
    });
    h.finish()
}
fn visit(
    scene: &GridScene,
    region: PixelRect,
    delta: Point,
    mut emit: impl FnMut(&Primitive, PixelRect, PixelRect),
) {
    let root = shifted(scene.clip, delta)
        .intersection(region)
        .unwrap_or_default();
    let mut stack = vec![root];
    for pass in &scene.passes {
        for op in pass.iter() {
            let clip = *stack.last().expect("root clip remains on stack");
            match op {
                Primitive::Clip { rect } => {
                    stack.push(clip.intersection(shifted(*rect, delta)).unwrap_or_default());
                    continue;
                }
                Primitive::EndClip => {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                    continue;
                }
                _ => {}
            }
            if clip.width == 0 || clip.height == 0 {
                continue;
            }
            let Some(coverage) = coverage(op, clip, delta) else {
                continue;
            };
            emit(op, clip, coverage);
        }
    }
}
fn hash_paint(
    op: &Primitive,
    clip: PixelRect,
    coverage: PixelRect,
    delta: Point,
    h: &mut impl Hasher,
) {
    std::mem::discriminant(op).hash(h);
    // Solid fills can be canonicalized to their visible rectangle.
    if let Primitive::RectFill { color, .. } = op {
        rect_hash(coverage, h);
        color.hash(h);
        return;
    }
    rect_hash(clip, h);
    match op {
        Primitive::Gradient { rect, from, to } => {
            rect_hash(shifted(*rect, delta), h);
            from.hash(h);
            to.hash(h);
        }
        Primitive::FillPath { path, color } => {
            path_hash(path, delta, h);
            color.hash(h);
        }
        Primitive::StrokePath { path, color, style } => {
            path_hash(path, delta, h);
            color.hash(h);
            floats(&[style.width, style.miter_limit], h);
            std::mem::discriminant(&style.cap).hash(h);
            std::mem::discriminant(&style.join).hash(h);
            style.dash.len().hash(h);
            floats(&style.dash, h);
        }
        Primitive::Circle {
            cx,
            cy,
            radius,
            color,
        } => {
            floats(
                &[cx + f64::from(delta.x), cy + f64::from(delta.y), *radius],
                h,
            );
            color.hash(h);
        }
        Primitive::Line { line, color, width } => {
            std::mem::discriminant(line).hash(h);
            match line {
                Line::H { span, y } => {
                    [span.from + delta.x, span.to + delta.x, y + delta.y].hash(h)
                }
                Line::V { x, span } => {
                    [x + delta.x, span.from + delta.y, span.to + delta.y].hash(h)
                }
            }
            color.hash(h);
            floats(&[*width], h);
        }
        Primitive::TextLine {
            x1,
            x2,
            y,
            color,
            width,
        } => {
            floats(
                &[
                    x1 + f64::from(delta.x),
                    x2 + f64::from(delta.x),
                    y + f64::from(delta.y),
                    *width,
                ],
                h,
            );
            color.hash(h);
        }
        Primitive::Text {
            text,
            x,
            y,
            font_css,
            color,
            align,
            baseline,
        } => {
            text.hash(h);
            floats(&[x + f64::from(delta.x), y + f64::from(delta.y)], h);
            font_css.hash(h);
            color.hash(h);
            std::mem::discriminant(align).hash(h);
            std::mem::discriminant(baseline).hash(h);
        }
        Primitive::RectFill { .. } | Primitive::Clip { .. } | Primitive::EndClip => {
            unreachable!()
        }
    }
}

pub(super) fn coverage(op: &Primitive, clip: PixelRect, delta: Point) -> Option<PixelRect> {
    let bounds = match op {
        Primitive::RectFill { rect, .. } | Primitive::Gradient { rect, .. } => {
            return shifted(*rect, delta).intersection(clip);
        }
        Primitive::Line { line, width, .. } => {
            let w = width / 2.0 + 1.0;
            match line {
                Line::H { span, y } => (
                    f64::from(span.from.min(span.to)) - w,
                    f64::from(*y) - w,
                    f64::from(span.from.max(span.to)) + w,
                    f64::from(*y) + w,
                ),
                Line::V { x, span } => (
                    f64::from(*x) - w,
                    f64::from(span.from.min(span.to)) - w,
                    f64::from(*x) + w,
                    f64::from(span.from.max(span.to)) + w,
                ),
            }
        }
        Primitive::TextLine {
            x1, x2, y, width, ..
        } => {
            let w = width / 2.0 + 1.0;
            (x1.min(*x2) - w, y - w, x1.max(*x2) + w, y + w)
        }
        Primitive::Circle { cx, cy, radius, .. } => (
            cx - radius - 1.0,
            cy - radius - 1.0,
            cx + radius + 1.0,
            cy + radius + 1.0,
        ),
        // Text and paths use their clip as a conservative bound. Glyph ink and
        // miter joins can extend beyond their nominal layout/control points.
        _ => return Some(clip),
    };
    let left = (bounds.0 + f64::from(delta.x))
        .floor()
        .max(f64::from(clip.left())) as i32;
    let top = (bounds.1 + f64::from(delta.y))
        .floor()
        .max(f64::from(clip.top())) as i32;
    let right = (bounds.2 + f64::from(delta.x))
        .ceil()
        .min(f64::from(clip.right())) as i32;
    let bottom = (bounds.3 + f64::from(delta.y))
        .ceil()
        .min(f64::from(clip.bottom())) as i32;
    (left < right && top < bottom).then_some(PixelRect {
        top_left: Point { x: left, y: top },
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Span,
        painter::{LineCap, LineJoin, TextAlign, TextBaseline},
        scene::SceneStroke,
    };
    use std::rc::Rc;
    fn scene(op: Primitive) -> GridScene {
        GridScene {
            clip: PixelRect {
                top_left: Point::default(),
                width: 100,
                height: 96,
            },
            passes: vec![Rc::from([op])],
        }
    }
    #[test]
    fn paint_inputs_change_fingerprints() {
        let rect = PixelRect {
            top_left: Point { x: 5, y: 5 },
            width: 20,
            height: 20,
        };
        let color = Rc::from("#123456");
        let path = Rc::from([
            PathCmd::Move(PointF::new(2.0, 3.0)),
            PathCmd::Quad(PointF::new(4.0, 5.0), PointF::new(6.0, 7.0)),
            PathCmd::Cubic(
                PointF::new(8.0, 9.0),
                PointF::new(10.0, 11.0),
                PointF::new(12.0, 13.0),
            ),
            PathCmd::Close,
        ]);
        let ops = vec![
            Primitive::RectFill {
                rect,
                color: Rc::clone(&color),
            },
            Primitive::Gradient {
                rect,
                from: Rc::clone(&color),
                to: Rc::from("#abcdef"),
            },
            Primitive::FillPath {
                path: Rc::clone(&path),
                color: Rc::clone(&color),
            },
            Primitive::StrokePath {
                path,
                color: Rc::clone(&color),
                style: SceneStroke {
                    width: 2.0,
                    cap: LineCap::Round,
                    join: LineJoin::Miter,
                    miter_limit: 10.0,
                    dash: Rc::from([3.0, 4.0]),
                },
            },
            Primitive::Circle {
                cx: 12.0,
                cy: 10.0,
                radius: 3.0,
                color: Rc::clone(&color),
            },
            Primitive::Line {
                line: Line::H {
                    span: Span { from: 0, to: 50 },
                    y: 20,
                },
                color: Rc::clone(&color),
                width: 3.0,
            },
            Primitive::TextLine {
                x1: 0.0,
                x2: 50.0,
                y: 20.0,
                color: Rc::clone(&color),
                width: 1.0,
            },
            Primitive::Text {
                text: "hello".into(),
                x: 12.0,
                y: 20.0,
                font_css: Rc::from("12px sans-serif"),
                color,
                align: TextAlign::Start,
                baseline: TextBaseline::Middle,
            },
        ];
        for op in ops {
            let original = scene(op.clone());
            let mut changed = op;
            match &mut changed {
                Primitive::RectFill { color, .. }
                | Primitive::FillPath { color, .. }
                | Primitive::Circle { color, .. }
                | Primitive::Line { color, .. }
                | Primitive::TextLine { color, .. } => *color = Rc::from("#fedcba"),
                Primitive::Gradient { to, .. } => *to = Rc::from("#fedcba"),
                Primitive::StrokePath { style, .. } => style.dash = Rc::from([4.0, 3.0]),
                Primitive::Text { font_css, .. } => *font_css = Rc::from("bold 12px sans-serif"),
                _ => unreachable!(),
            }
            assert_ne!(
                SceneFingerprint::new(&original),
                SceneFingerprint::new(&scene(changed))
            );
        }
    }
    #[test]
    fn damage_includes_removed_content_and_stroke_coverage() {
        let old = scene(Primitive::Line {
            line: Line::H {
                span: Span { from: 0, to: 50 },
                y: 32,
            },
            color: Rc::from("red"),
            width: 4.0,
        });
        let blank = GridScene {
            clip: old.clip,
            passes: vec![],
        };
        assert_eq!(
            SceneFingerprint::new(&blank).damage(&SceneFingerprint::new(&old)),
            SceneUpdate::Damage(vec![PixelRect {
                top_left: Point::default(),
                width: 100,
                height: 64
            }])
        );
    }
    #[test]
    fn clip_changes_and_path_changes_invalidate_painted_truth() {
        let r = PixelRect {
            top_left: Point::default(),
            width: 100,
            height: 96,
        };
        let make = |radius| Primitive::FillPath {
            path: Rc::from([PathCmd::Arc(crate::geometry::path::ArcSpec::new(
                PointF::new(20.0, 20.0),
                radius,
                0.2,
                2.0,
            ))]),
            color: Rc::from("red"),
        };
        let a = scene(make(5.0));
        let b = scene(make(6.0));
        assert_ne!(SceneFingerprint::new(&a), SceneFingerprint::new(&b));
        let clipped = GridScene {
            clip: r,
            passes: vec![Rc::from([
                Primitive::Clip {
                    rect: r.inset(4, 4),
                },
                make(5.0),
                Primitive::EndClip,
            ])],
        };
        assert_ne!(SceneFingerprint::new(&a), SceneFingerprint::new(&clipped));
    }
}
