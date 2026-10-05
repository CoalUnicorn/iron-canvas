//! Owned, resolved drawing input. This module has no worksheet model or policy.
//!
//! Commands share one buffer per paint pass, rather than one allocation per
//! cell. Paths and colors remain shared when a prepared scene is cloned.
use crate::geometry::path::{Path, PathCmd};
use crate::painter::{
    LineCap, LineJoin, PaintColor, Painter, StrokeStyle, TextAlign, TextBaseline,
};
use crate::{Line, PixelRect};
use std::{cell::RefCell, rc::Rc};

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
    pub passes: Vec<Vec<Primitive>>,
}
impl GridScene {
    /// Execute prepared primitives without model reads or text measurement.
    pub fn paint(&self, painter: &impl Painter) {
        painter.push_clip(self.clip);
        for pass in &self.passes {
            for op in pass {
                match op {
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
                    } => {
                        painter.stroke_text_hline(*x1, *x2, *y, PaintColor::Borrowed(color), *width)
                    }
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
        painter.pop_clip();
    }
}
/// Builds one pass from already resolved primitive arguments.
#[derive(Default)]
pub struct SceneBuilder {
    ops: RefCell<Vec<Primitive>>,
}
impl SceneBuilder {
    pub fn finish(self) -> Vec<Primitive> {
        self.ops.into_inner()
    }
    pub fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.ops.borrow_mut().push(Primitive::RectFill {
            rect,
            color: Rc::from(color.as_str()),
        });
    }
    pub fn rect_fill_hgradient(&self, rect: PixelRect, from: PaintColor, to: PaintColor) {
        self.ops.borrow_mut().push(Primitive::Gradient {
            rect,
            from: Rc::from(from.as_str()),
            to: Rc::from(to.as_str()),
        });
    }
    pub fn fill_path(&self, path: &Path<'_>, color: PaintColor) {
        self.ops.borrow_mut().push(Primitive::FillPath {
            path: Rc::from(path.cmds()),
            color: Rc::from(color.as_str()),
        });
    }
    pub fn stroke_path(&self, path: &Path<'_>, color: PaintColor, style: &StrokeStyle<'_>) {
        self.ops.borrow_mut().push(Primitive::StrokePath {
            path: Rc::from(path.cmds()),
            color: Rc::from(color.as_str()),
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
            color: Rc::from(color.as_str()),
        });
    }
    pub fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        self.ops.borrow_mut().push(Primitive::Line {
            line,
            color: Rc::from(color.as_str()),
            width,
        });
    }
    pub fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.ops.borrow_mut().push(Primitive::TextLine {
            x1,
            x2,
            y,
            color: Rc::from(color.as_str()),
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
            font_css: Rc::from(font_css.as_str()),
            color: Rc::from(color.as_str()),
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
    fn commit_frame(&mut self, staged: Self::StagedFrame) -> Result<(), Self::Error>;
}
