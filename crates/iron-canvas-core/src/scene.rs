//! Owned, resolved drawing input. This module has no worksheet model or policy.
//!
//! Commands share one buffer per paint pass, rather than one allocation per
//! cell. Paths and colors remain shared when a prepared scene is cloned.
pub mod retained;

use crate::geometry::path::{Path, PathCmd};
use crate::painter::{
    LineCap, LineJoin, PaintColor, Painter, StrokeStyle, TextAlign, TextBaseline,
};
use crate::{Line, PixelRect};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

#[derive(Debug, Clone, PartialEq)]
pub struct SceneStroke {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub dash: Rc<[f64]>,
}

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
        path: Rc<[PathCmd]>,
        color: Rc<str>,
    },
    StrokePath {
        path: Rc<[PathCmd]>,
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
    ops: RefCell<Vec<Primitive>>,
    strings: RefCell<HashSet<Rc<str>>>,
}
impl SceneBuilder {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ops: RefCell::new(Vec::with_capacity(capacity)),
            strings: RefCell::default(),
        }
    }

    fn intern(&self, value: &str) -> Rc<str> {
        let mut strings = self.strings.borrow_mut();
        if let Some(value) = strings.get(value) {
            return Rc::clone(value);
        }
        let value: Rc<str> = Rc::from(value);
        strings.insert(Rc::clone(&value));
        value
    }

    pub fn finish(self) -> Rc<[Primitive]> {
        self.ops.into_inner().into()
    }
    pub fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.ops.borrow_mut().push(Primitive::RectFill {
            rect,
            color: self.intern(color.as_str()),
        });
    }
    pub fn rect_fill_hgradient(&self, rect: PixelRect, from: PaintColor, to: PaintColor) {
        self.ops.borrow_mut().push(Primitive::Gradient {
            rect,
            from: self.intern(from.as_str()),
            to: self.intern(to.as_str()),
        });
    }
    pub fn fill_path(&self, path: &Path<'_>, color: PaintColor) {
        self.ops.borrow_mut().push(Primitive::FillPath {
            path: Rc::from(path.cmds()),
            color: self.intern(color.as_str()),
        });
    }
    pub fn stroke_path(&self, path: &Path<'_>, color: PaintColor, style: &StrokeStyle<'_>) {
        self.ops.borrow_mut().push(Primitive::StrokePath {
            path: Rc::from(path.cmds()),
            color: self.intern(color.as_str()),
            style: SceneStroke {
                width: style.width,
                cap: style.cap,
                join: style.join,
                miter_limit: style.miter_limit,
                dash: Rc::from(style.dash),
            },
        });
    }
    pub fn fill_circle(&self, cx: f64, cy: f64, radius: f64, color: PaintColor) {
        self.ops.borrow_mut().push(Primitive::Circle {
            cx,
            cy,
            radius,
            color: self.intern(color.as_str()),
        });
    }
    pub fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        self.ops.borrow_mut().push(Primitive::Line {
            line,
            color: self.intern(color.as_str()),
            width,
        });
    }
    pub fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.ops.borrow_mut().push(Primitive::TextLine {
            x1,
            x2,
            y,
            color: self.intern(color.as_str()),
            width,
        });
    }
    pub fn push_clip(&self, rect: PixelRect) {
        self.ops.borrow_mut().push(Primitive::Clip { rect });
    }
    pub fn pop_clip(&self) {
        self.ops.borrow_mut().push(Primitive::EndClip);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn fill_text(
        &self,
        text: &str,
        x: f64,
        y: f64,
        font_css: PaintColor,
        color: PaintColor,
        align: TextAlign,
        baseline: TextBaseline,
    ) {
        self.ops.borrow_mut().push(Primitive::Text {
            text: text.to_owned(),
            x,
            y,
            font_css: self.intern(font_css.as_str()),
            color: self.intern(color.as_str()),
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
