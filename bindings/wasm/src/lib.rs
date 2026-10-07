use iron_canvas_canvas2d::Canvas2dSceneBackend;
use iron_canvas_core::{
    CanvasMetrics, CanvasSize, Line, PixelRect, Point, Span,
    geometry::path::{ArcSpec, Path, PathCmd, PointF},
    painter::{LineCap, LineJoin, PaintColor, StrokeStyle, TextAlign, TextBaseline},
    scene::{GridScene, RenderBackend, SceneBuilder},
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

#[wasm_bindgen]
pub struct CanvasRenderer {
    canvas: HtmlCanvasElement,
    metrics: CanvasMetrics,
    backend: Canvas2dSceneBackend,
}

#[wasm_bindgen]
impl CanvasRenderer {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<CanvasRenderer, JsValue> {
        let metrics = metrics_for(0.0, 0.0, 1.0)?;
        let backend = Canvas2dSceneBackend::new(canvas.clone(), metrics)?;
        Ok(CanvasRenderer {
            canvas,
            metrics,
            backend,
        })
    }

    #[wasm_bindgen(js_name = resize)]
    pub fn resize(&mut self, width: f64, height: f64, dpr: f64) -> Result<(), JsValue> {
        let metrics = metrics_for(width, height, dpr)?;
        if metrics != self.metrics {
            self.backend = Canvas2dSceneBackend::new(self.canvas.clone(), metrics)?;
            self.metrics = metrics;
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = renderScene)]
    pub fn render_scene(&mut self, value: JsValue) -> Result<(), JsValue> {
        let dto: SceneDto = serde_wasm_bindgen::from_value(value)
            .map_err(|error| JsValue::from_str(&format!("invalid Iron Canvas scene: {error}")))?;
        let scene = dto
            .into_scene()
            .map_err(|error| JsValue::from_str(&error))?;
        self.render_grid_scene(&scene)
    }
}

impl CanvasRenderer {
    /// Render a typed Iron Canvas scene. This Rust entry point also lets
    /// application-specific hosts reuse the Canvas2D renderer without
    /// routing internal scene data through the JavaScript DTO.
    pub fn render_grid_scene(&mut self, scene: &GridScene) -> Result<(), JsValue> {
        let staged = self.backend.stage_frame(scene)?;
        self.backend.commit_frame(staged)
    }
}

fn metrics_for(width: f64, height: f64, dpr: f64) -> Result<CanvasMetrics, JsValue> {
    CanvasMetrics::new(
        CanvasSize {
            w: width,
            h: height,
        },
        dpr,
    )
    .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SceneDto {
    clip: RectDto,
    passes: Vec<Vec<PrimitiveDto>>,
}

impl SceneDto {
    fn into_scene(self) -> Result<GridScene, String> {
        let clip = self.clip.into_rect()?;
        let mut clip_depth = 0usize;
        let mut passes = Vec::with_capacity(self.passes.len());

        for commands in self.passes {
            let builder = SceneBuilder::with_capacity(commands.len());
            for command in commands {
                match command {
                    PrimitiveDto::RectFill { rect, color } => {
                        builder.rect_fill(rect.into_rect()?, PaintColor::Borrowed(&color));
                    }
                    PrimitiveDto::Gradient { rect, from, to } => {
                        builder.rect_fill_hgradient(
                            rect.into_rect()?,
                            PaintColor::Borrowed(&from),
                            PaintColor::Borrowed(&to),
                        );
                    }
                    PrimitiveDto::FillPath { path, color } => {
                        let path = into_path(path)?;
                        builder.fill_path(&Path::new(&path), PaintColor::Borrowed(&color));
                    }
                    PrimitiveDto::StrokePath { path, color, style } => {
                        let path = into_path(path)?;
                        let dash = style.dash;
                        let stroke = StrokeStyle {
                            width: style.width,
                            cap: style.cap.into(),
                            join: style.join.into(),
                            miter_limit: style.miter_limit,
                            dash: &dash,
                        };
                        if !stroke.is_valid() {
                            return Err("invalid stroke style".into());
                        }
                        builder.stroke_path(
                            &Path::new(&path),
                            PaintColor::Borrowed(&color),
                            &stroke,
                        );
                    }
                    PrimitiveDto::Circle {
                        cx,
                        cy,
                        radius,
                        color,
                    } => {
                        if !cx.is_finite()
                            || !cy.is_finite()
                            || !radius.is_finite()
                            || radius <= 0.0
                        {
                            return Err(
                                "circle coordinates must be finite and radius positive".into()
                            );
                        }
                        builder.fill_circle(cx, cy, radius, PaintColor::Borrowed(&color));
                    }
                    PrimitiveDto::Line { line, color, width } => {
                        if !width.is_finite() || width <= 0.0 {
                            return Err("line width must be finite and positive".into());
                        }
                        builder.stroke_line(line.into_line()?, PaintColor::Borrowed(&color), width);
                    }
                    PrimitiveDto::TextLine {
                        x1,
                        x2,
                        y,
                        color,
                        width,
                    } => {
                        if !x1.is_finite()
                            || !x2.is_finite()
                            || !y.is_finite()
                            || !width.is_finite()
                            || width <= 0.0
                            || x1 > x2
                        {
                            return Err(
                                "text line coordinates must be finite and width positive".into()
                            );
                        }
                        builder.stroke_text_hline(x1, x2, y, PaintColor::Borrowed(&color), width);
                    }
                    PrimitiveDto::Clip { rect } => {
                        builder.push_clip(rect.into_rect()?);
                        clip_depth += 1;
                    }
                    PrimitiveDto::EndClip => {
                        if clip_depth == 0 {
                            return Err("scene contains an unmatched endClip command".into());
                        }
                        builder.pop_clip();
                        clip_depth -= 1;
                    }
                    PrimitiveDto::Text {
                        text,
                        x,
                        y,
                        font_css,
                        color,
                        align,
                        baseline,
                    } => {
                        if !x.is_finite() || !y.is_finite() {
                            return Err("text coordinates must be finite".into());
                        }
                        builder.fill_text(
                            &text,
                            x,
                            y,
                            PaintColor::Borrowed(&font_css),
                            PaintColor::Borrowed(&color),
                            align.into(),
                            baseline.into(),
                        );
                    }
                }
            }
            passes.push(builder.finish());
        }

        if clip_depth != 0 {
            return Err("scene contains an unmatched clip command".into());
        }

        Ok(GridScene { clip, passes })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RectDto {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

impl RectDto {
    fn into_rect(self) -> Result<PixelRect, String> {
        if self.width < 0
            || self.height < 0
            || self.left.checked_add(self.width).is_none()
            || self.top.checked_add(self.height).is_none()
        {
            return Err("rect dimensions must be non-negative and fit i32".into());
        }
        Ok(PixelRect {
            top_left: Point {
                x: self.left,
                y: self.top,
            },
            width: self.width,
            height: self.height,
        })
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum PrimitiveDto {
    RectFill {
        rect: RectDto,
        color: String,
    },
    Gradient {
        rect: RectDto,
        from: String,
        to: String,
    },
    FillPath {
        path: Vec<PathCommandDto>,
        color: String,
    },
    StrokePath {
        path: Vec<PathCommandDto>,
        color: String,
        style: StrokeDto,
    },
    Circle {
        cx: f64,
        cy: f64,
        radius: f64,
        color: String,
    },
    Line {
        line: LineDto,
        color: String,
        width: f64,
    },
    TextLine {
        x1: f64,
        x2: f64,
        y: f64,
        color: String,
        width: f64,
    },
    Clip {
        rect: RectDto,
    },
    EndClip,
    Text {
        text: String,
        x: f64,
        y: f64,
        font_css: String,
        color: String,
        align: TextAlignDto,
        baseline: TextBaselineDto,
    },
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum PathCommandDto {
    Move {
        to: PointFDto,
    },
    Line {
        to: PointFDto,
    },
    Quad {
        control: PointFDto,
        to: PointFDto,
    },
    Cubic {
        control1: PointFDto,
        control2: PointFDto,
        to: PointFDto,
    },
    Arc {
        center: PointFDto,
        radius: f64,
        start_angle: f64,
        sweep_angle: f64,
    },
    Close,
}

impl PathCommandDto {
    fn into_path_command(self) -> PathCmd {
        match self {
            Self::Move { to } => PathCmd::Move(to.into_point()),
            Self::Line { to } => PathCmd::Line(to.into_point()),
            Self::Quad { control, to } => PathCmd::Quad(control.into_point(), to.into_point()),
            Self::Cubic {
                control1,
                control2,
                to,
            } => PathCmd::Cubic(
                control1.into_point(),
                control2.into_point(),
                to.into_point(),
            ),
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep_angle,
            } => PathCmd::Arc(ArcSpec {
                center: center.into_point(),
                radius,
                start_angle,
                sweep_angle,
            }),
            Self::Close => PathCmd::Close,
        }
    }
}

fn into_path(path: Vec<PathCommandDto>) -> Result<Vec<PathCmd>, String> {
    let path: Vec<_> = path
        .into_iter()
        .map(PathCommandDto::into_path_command)
        .collect();
    Path::new(&path)
        .validate()
        .map_err(|error| format!("invalid path: {error:?}"))?;
    Ok(path)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PointFDto {
    x: f64,
    y: f64,
}

impl PointFDto {
    fn into_point(self) -> PointF {
        PointF::new(self.x, self.y)
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum LineDto {
    Horizontal { from: i32, to: i32, y: i32 },
    Vertical { from: i32, to: i32, x: i32 },
}

impl LineDto {
    fn into_line(self) -> Result<Line, String> {
        match self {
            Self::Horizontal { from, to, y } if from <= to => Ok(Line::H {
                span: Span { from, to },
                y,
            }),
            Self::Vertical { from, to, x } if from <= to => Ok(Line::V {
                span: Span { from, to },
                x,
            }),
            _ => Err("line span endpoints must be ordered".into()),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StrokeDto {
    width: f64,
    cap: LineCapDto,
    join: LineJoinDto,
    miter_limit: f64,
    dash: Vec<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LineCapDto {
    Butt,
    Round,
    Square,
}

impl From<LineCapDto> for LineCap {
    fn from(value: LineCapDto) -> Self {
        match value {
            LineCapDto::Butt => Self::Butt,
            LineCapDto::Round => Self::Round,
            LineCapDto::Square => Self::Square,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LineJoinDto {
    Miter,
    Round,
    Bevel,
}

impl From<LineJoinDto> for LineJoin {
    fn from(value: LineJoinDto) -> Self {
        match value {
            LineJoinDto::Miter => Self::Miter,
            LineJoinDto::Round => Self::Round,
            LineJoinDto::Bevel => Self::Bevel,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum TextAlignDto {
    Start,
    Center,
    End,
}

impl From<TextAlignDto> for TextAlign {
    fn from(value: TextAlignDto) -> Self {
        match value {
            TextAlignDto::Start => Self::Start,
            TextAlignDto::Center => Self::Center,
            TextAlignDto::End => Self::End,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum TextBaselineDto {
    Top,
    Middle,
    Bottom,
    Alphabetic,
}

impl From<TextBaselineDto> for TextBaseline {
    fn from(value: TextBaselineDto) -> Self {
        match value {
            TextBaselineDto::Top => Self::Top,
            TextBaselineDto::Middle => Self::Middle,
            TextBaselineDto::Bottom => Self::Bottom,
            TextBaselineDto::Alphabetic => Self::Alphabetic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SceneDto;
    use serde_json::json;

    #[test]
    fn builds_a_scene_from_canvas_tagged_primitives() {
        let dto: SceneDto = serde_json::from_value(json!({
            "clip": { "left": 0, "top": 0, "width": 120, "height": 40 },
            "passes": [[
                {
                    "type": "rectFill",
                    "rect": { "left": 0, "top": 0, "width": 120, "height": 40 },
                    "color": "#ffffff"
                },
                {
                    "type": "text",
                    "text": "Canvas",
                    "x": 4.0,
                    "y": 18.0,
                    "fontCss": "12px sans-serif",
                    "color": "#000000",
                    "align": "start",
                    "baseline": "alphabetic"
                }
            ]]
        }))
        .unwrap();

        let scene = dto.into_scene().unwrap();
        assert_eq!(scene.clip.width, 120);
        assert_eq!(scene.passes.len(), 1);
        assert_eq!(scene.passes[0].len(), 2);
    }

    #[test]
    fn rejects_unbalanced_clip_commands() {
        let dto: SceneDto = serde_json::from_value(json!({
            "clip": { "left": 0, "top": 0, "width": 10, "height": 10 },
            "passes": [[{ "type": "endClip" }]]
        }))
        .unwrap();

        assert!(dto.into_scene().unwrap_err().contains("unmatched endClip"));
    }

    #[test]
    fn rejects_paths_that_do_not_start_a_subpath() {
        let dto: SceneDto = serde_json::from_value(json!({
            "clip": { "left": 0, "top": 0, "width": 10, "height": 10 },
            "passes": [[{
                "type": "fillPath",
                "color": "#000000",
                "path": [{ "type": "line", "to": { "x": 1.0, "y": 1.0 } }]
            }]]
        }))
        .unwrap();

        assert!(dto.into_scene().unwrap_err().contains("NoSubpath"));
    }

    #[test]
    fn rejects_arc_paths_with_overflowing_endpoints() {
        let dto: SceneDto = serde_json::from_value(json!({
            "clip": { "left": 0, "top": 0, "width": 10, "height": 10 },
            "passes": [[{
                "type": "fillPath",
                "color": "#000000",
                "path": [
                    { "type": "move", "to": { "x": 0.0, "y": 0.0 } },
                    {
                        "type": "arc",
                        "center": { "x": 1.7e308, "y": 0.0 },
                        "radius": 1.7e308,
                        "startAngle": 0.0,
                        "sweepAngle": 1.0
                    }
                ]
            }]]
        }))
        .unwrap();

        assert!(dto.into_scene().unwrap_err().contains("InvalidArc"));
    }

    #[test]
    fn rejects_overflowing_rectangles() {
        let dto: SceneDto = serde_json::from_value(json!({
            "clip": {
                "left": 2147483647,
                "top": 0,
                "width": 1,
                "height": 1
            },
            "passes": []
        }))
        .unwrap();

        assert!(dto.into_scene().unwrap_err().contains("fit i32"));
    }
}
