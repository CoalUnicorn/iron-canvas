//! Owned, resolved drawing input. This module has no worksheet model or policy.
//!
//! Each paint pass shares its command buffer. Each command owns its path and
//! dash buffers. A scene clone shares complete passes and their contents.
pub mod retained;

use crate::geometry::path::{Path, PathCmd};
use crate::painter::{
    LineCap, LineJoin, PaintColor, Painter, StrokeStyle, TextAlign, TextBaseline,
};
use crate::{Line, PixelRect};
use std::{collections::HashSet, rc::Rc};

#[derive(Debug, Clone, PartialEq)]
pub struct SceneStroke {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    /// Owned dash lengths. Cloning this style copies the slice.
    pub dash: Box<[f64]>,
}

/// One drawing command. A clone copies path and dash buffers.
/// Clone a complete scene to share its passes without these copies.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    RectFill {
        rect: PixelRect,
        color: Rc<str>,
    },
    Gradient {
        rect: PixelRect,
        from: Rc<str>,
        to: Rc<str>,
    },
    FillPath {
        path: Box<[PathCmd]>,
        color: Rc<str>,
    },
    StrokePath {
        path: Box<[PathCmd]>,
        color: Rc<str>,
        style: SceneStroke,
    },
    Circle {
        cx: f64,
        cy: f64,
        radius: f64,
        color: Rc<str>,
    },
    Line {
        line: Line,
        color: Rc<str>,
        width: f64,
    },
    TextLine {
        x1: f64,
        x2: f64,
        y: f64,
        color: Rc<str>,
        width: f64,
    },
    Clip {
        rect: PixelRect,
    },
    EndClip,
    Text {
        text: String,
        x: f64,
        y: f64,
        font_css: Rc<str>,
        color: Rc<str>,
        align: TextAlign,
        baseline: TextBaseline,
    },
}
/// Ordered drawing passes. The producer decides the meaning of each pass.
#[derive(Debug, Clone, PartialEq)]
pub struct GridScene {
    pub clip: PixelRect,
    pub passes: Vec<Rc<[Primitive]>>,
}
impl GridScene {
    /// Execute prepared primitives without model reads or text measurement.
    pub fn paint(&self, painter: &impl Painter) {
        painter.push_clip(self.clip);
        for pass in &self.passes {
            for op in pass.iter() {
                op.paint(painter);
            }
        }
        painter.pop_clip();
    }
    /// Cull commands outside a backend's device-aligned repair clip.
    /// The backend owns the actual clip; this does not introduce a second edge.
    pub fn paint_regions(&self, painter: &impl Painter, regions: &[PixelRect]) {
        painter.push_clip(self.clip);
        let mut clips = vec![(self.clip, true)];
        for pass in &self.passes {
            for op in pass.iter() {
                let clip = clips.last().expect("root clip").0;
                match op {
                    Primitive::Clip { rect } => {
                        let inner = clip.intersection(*rect).unwrap_or_default();
                        let active = regions.iter().any(|r| inner.intersection(*r).is_some());
                        if active {
                            painter.push_clip(*rect);
                        }
                        clips.push((inner, active));
                    }
                    Primitive::EndClip => {
                        if clips.len() > 1 && clips.pop().is_some_and(|c| c.1) {
                            painter.pop_clip();
                        }
                    }
                    _ => {
                        if clip.width > 0
                            && clip.height > 0
                            && retained::coverage(op, clip, crate::Point::default()).is_some_and(
                                |coverage| {
                                    regions.iter().any(|r| coverage.intersection(*r).is_some())
                                },
                            )
                        {
                            op.paint(painter);
                        }
                    }
                }
            }
        }
        painter.pop_clip();
    }
}
impl Primitive {
    fn paint(&self, painter: &impl Painter) {
        match self {
            Primitive::RectFill { rect, color } => {
                painter.rect_fill(*rect, PaintColor::Borrowed(color))
            }
            Primitive::Gradient { rect, from, to } => painter.rect_fill_hgradient(
                *rect,
                PaintColor::Borrowed(from),
                PaintColor::Borrowed(to),
            ),
            Primitive::FillPath { path, color } => {
                painter.fill_path(&Path::new(path), PaintColor::Borrowed(color))
            }
            Primitive::StrokePath { path, color, style } => painter.stroke_path(
                &Path::new(path),
                PaintColor::Borrowed(color),
                &StrokeStyle {
                    width: style.width,
                    cap: style.cap,
                    join: style.join,
                    miter_limit: style.miter_limit,
                    dash: &style.dash,
                },
            ),
            Primitive::Circle {
                cx,
                cy,
                radius,
                color,
            } => painter.fill_circle(*cx, *cy, *radius, PaintColor::Borrowed(color)),
            Primitive::Line { line, color, width } => {
                painter.stroke_line(*line, PaintColor::Borrowed(color), *width)
            }
            Primitive::TextLine {
                x1,
                x2,
                y,
                color,
                width,
            } => painter.stroke_text_hline(*x1, *x2, *y, PaintColor::Borrowed(color), *width),
            Primitive::Clip { rect } => painter.push_clip(*rect),
            Primitive::EndClip => painter.pop_clip(),
            Primitive::Text {
                text,
                x,
                y,
                font_css,
                color,
                align,
                baseline,
            } => painter.fill_text(
                text,
                *x,
                *y,
                PaintColor::Borrowed(font_css),
                PaintColor::Borrowed(color),
                *align,
                *baseline,
            ),
        }
    }
}

/// Builds one pass from already resolved primitive arguments.
#[derive(Default)]
pub struct SceneBuilder {
    ops: Vec<Primitive>,
    strings: HashSet<Rc<str>>,
}
impl SceneBuilder {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ops: Vec::with_capacity(capacity),
            strings: HashSet::new(),
        }
    }

    fn intern(&mut self, value: &str) -> Rc<str> {
        if let Some(value) = self.strings.get(value) {
            return Rc::clone(value);
        }
        let value: Rc<str> = Rc::from(value);
        self.strings.insert(Rc::clone(&value));
        value
    }

    pub fn finish(self) -> Rc<[Primitive]> {
        self.ops.into()
    }
    pub fn rect_fill(&mut self, rect: PixelRect, color: PaintColor) {
        let color = self.intern(color.as_str());
        self.ops.push(Primitive::RectFill { rect, color });
    }
    pub fn rect_fill_hgradient(&mut self, rect: PixelRect, from: PaintColor, to: PaintColor) {
        let from = self.intern(from.as_str());
        let to = self.intern(to.as_str());
        self.ops.push(Primitive::Gradient { rect, from, to });
    }
    pub fn fill_path(&mut self, path: &Path<'_>, color: PaintColor) {
        let color = self.intern(color.as_str());
        self.ops.push(Primitive::FillPath {
            path: Box::from(path.cmds()),
            color,
        });
    }
    pub fn stroke_path(&mut self, path: &Path<'_>, color: PaintColor, style: &StrokeStyle<'_>) {
        let color = self.intern(color.as_str());
        self.ops.push(Primitive::StrokePath {
            path: Box::from(path.cmds()),
            color,
            style: SceneStroke {
                width: style.width,
                cap: style.cap,
                join: style.join,
                miter_limit: style.miter_limit,
                dash: Box::from(style.dash),
            },
        });
    }
    pub fn fill_circle(&mut self, cx: f64, cy: f64, radius: f64, color: PaintColor) {
        let color = self.intern(color.as_str());
        self.ops.push(Primitive::Circle {
            cx,
            cy,
            radius,
            color,
        });
    }
    pub fn stroke_line(&mut self, line: Line, color: PaintColor, width: f64) {
        let color = self.intern(color.as_str());
        self.ops.push(Primitive::Line { line, color, width });
    }
    pub fn stroke_text_hline(&mut self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        let color = self.intern(color.as_str());
        self.ops.push(Primitive::TextLine {
            x1,
            x2,
            y,
            color,
            width,
        });
    }
    pub fn push_clip(&mut self, rect: PixelRect) {
        self.ops.push(Primitive::Clip { rect });
    }
    pub fn pop_clip(&mut self) {
        self.ops.push(Primitive::EndClip);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn fill_text(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        font_css: PaintColor,
        color: PaintColor,
        align: TextAlign,
        baseline: TextBaseline,
    ) {
        let font_css = self.intern(font_css.as_str());
        let color = self.intern(color.as_str());
        self.ops.push(Primitive::Text {
            text: text.to_owned(),
            x,
            y,
            font_css,
            color,
            align,
            baseline,
        });
    }
}

/// A backend stages a complete scene without changing visible output.
/// A failed stage or commit must preserve the previous visible frame.
/// A successful commit publishes the staged output in one operation.
pub trait RenderBackend {
    type StagedFrame;
    type Error;
    fn metrics(&self) -> &dyn crate::painter::TextMetrics;
    fn stage_frame(&mut self, scene: &GridScene) -> Result<Self::StagedFrame, Self::Error>;
    /// Drop backend measurement memoization after fonts or metrics change.
    fn invalidate_metrics(&mut self) {}
    /// Return a fixed raster scale when this backend implements retained updates.
    fn retained_scale(&self) -> Option<f64> {
        None
    }
    /// Enable partial raster repair only after backend pixel-equivalence checks.
    fn supports_partial_updates(&self) -> bool {
        false
    }
    /// Default export behavior always produces a complete document.
    fn stage_update(
        &mut self,
        scene: &GridScene,
        _update: &retained::SceneUpdate,
    ) -> Result<Self::StagedFrame, Self::Error> {
        self.stage_frame(scene)
    }
    fn commit_frame(&mut self, staged: Self::StagedFrame) -> Result<(), Self::Error>;
}
