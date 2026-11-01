#![cfg_attr(not(feature = "js"), allow(dead_code, unused_imports))]

use iron_canvas_canvas2d::{Canvas2dSceneBackend, theme_from_element};
use iron_canvas_core::{
    CanvasMetrics, CanvasSize, CanvasTheme, CellCoord, Line, PixelRect, Point, Span,
    geometry::path::{ArcSpec, Path, PathCmd, PointF},
    navigation::{
        BindingProfile, Direction, NavigationIntent, NavigationKey, NavigationModifiers,
        PageDirection, TraversalAxis, TraversalDirection, classify_navigation,
    },
    painter::{LineCap, LineJoin, PaintColor, StrokeStyle, TextAlign, TextBaseline, TextMetrics},
    scene::{
        GridScene, RenderBackend, SceneBuilder,
        retained::{SceneFingerprint, SceneUpdate},
    },
    scroll::{
        self, ExtentRun, RunAxis, ScrollAxisWindow, move_axis, move_position, normalize_axis,
        normalize_position, scrolling_extent,
    },
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlCanvasElement};

/// Rust-side paint target shared by the JavaScript adapter and linked hosts.
///
/// It owns the Canvas2D backend, a fixed metric set, and the committed scene
/// fingerprint. A linked host can use it as a `CanvasSession` backend; the
/// JavaScript adapter uses `render_scene` for retained frames.
pub struct SceneTarget {
    backend: Canvas2dSceneBackend,
    metrics: CanvasMetrics,
    /// Fingerprint of the committed scene. The next scene reuses, damages, or
    /// repaints from it. `None` forces a full frame.
    previous: Option<SceneFingerprint>,
}

impl SceneTarget {
    pub fn new(canvas: HtmlCanvasElement, metrics: CanvasMetrics) -> Result<Self, JsValue> {
        Ok(Self {
            backend: Canvas2dSceneBackend::new(canvas, metrics)?,
            metrics,
            previous: None,
        })
    }

    pub fn metrics(&self) -> CanvasMetrics {
        self.metrics
    }

    /// Render a scene with retained updates. An unchanged scene keeps the
    /// committed pixels; a changed scene repaints only its damaged bands.
    pub fn render_scene(&mut self, scene: &GridScene) -> Result<(), JsValue> {
        let fingerprint = SceneFingerprint::new(scene);
        let update = plan_update(self.previous.as_ref(), &fingerprint);
        let staged = self.backend.stage_update(scene, &update)?;
        self.backend.commit_frame(staged)?;
        // Publish the fingerprint only after the frame commits, so a failed
        // stage or commit leaves the reuse state on the previous frame.
        self.previous = Some(fingerprint);
        Ok(())
    }

    /// Render a scene and repaint every pixel, even when the scene is
    /// unchanged.
    pub fn render_scene_fresh(&mut self, scene: &GridScene) -> Result<(), JsValue> {
        let staged = self.backend.stage_frame(scene)?;
        self.backend.commit_frame(staged)?;
        self.previous = Some(SceneFingerprint::new(scene));
        Ok(())
    }

    /// Drop memoized text measurements and force the next scene to repaint.
    pub fn invalidate_metrics(&mut self) {
        self.backend.invalidate_metrics();
        self.previous = None;
    }
}

impl RenderBackend for SceneTarget {
    type StagedFrame = HtmlCanvasElement;
    type Error = JsValue;

    fn metrics(&self) -> &dyn TextMetrics {
        self.backend.metrics()
    }

    fn stage_frame(&mut self, scene: &GridScene) -> Result<HtmlCanvasElement, JsValue> {
        self.backend.stage_frame(scene)
    }

    fn invalidate_metrics(&mut self) {
        self.backend.invalidate_metrics();
        self.previous = None;
    }

    fn supports_partial_updates(&self) -> bool {
        self.backend.supports_partial_updates()
    }

    fn retained_scale(&self) -> Option<f64> {
        self.backend.retained_scale()
    }

    fn stage_update(
        &mut self,
        scene: &GridScene,
        update: &SceneUpdate,
    ) -> Result<HtmlCanvasElement, JsValue> {
        self.backend.stage_update(scene, update)
    }

    fn commit_frame(&mut self, staged: HtmlCanvasElement) -> Result<(), JsValue> {
        self.backend.commit_frame(staged)
    }
}

#[cfg(feature = "js")]
#[wasm_bindgen]
pub struct CanvasRenderer {
    canvas: HtmlCanvasElement,
    target: SceneTarget,
}

#[cfg(feature = "js")]
#[wasm_bindgen]
impl CanvasRenderer {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<CanvasRenderer, JsValue> {
        let metrics = metrics_for(0.0, 0.0, 1.0)?;
        Ok(CanvasRenderer {
            canvas: canvas.clone(),
            target: SceneTarget::new(canvas, metrics)?,
        })
    }

    #[cfg(feature = "js")]
    #[wasm_bindgen(js_name = resize)]
    pub fn resize(&mut self, width: f64, height: f64, dpr: f64) -> Result<(), JsValue> {
        let metrics = metrics_for(width, height, dpr)?;
        if metrics != self.target.metrics() {
            // The new target has no painted pixels. The next scene is fresh.
            self.target = SceneTarget::new(self.canvas.clone(), metrics)?;
        }
        Ok(())
    }

    /// Render a scene with retained updates. An unchanged scene keeps the
    /// committed pixels; a changed scene repaints only its damaged bands.
    #[cfg(feature = "js")]
    #[wasm_bindgen(js_name = renderScene)]
    pub fn render_scene(&mut self, value: JsValue) -> Result<(), JsValue> {
        let scene = scene_from_value(value)?;
        self.target.render_scene(&scene)
    }

    /// Render a scene and repaint every pixel, even when the scene is
    /// unchanged. Use after a display or compositing change outside the scene.
    #[cfg(feature = "js")]
    #[wasm_bindgen(js_name = renderSceneFresh)]
    pub fn render_scene_fresh(&mut self, value: JsValue) -> Result<(), JsValue> {
        let scene = scene_from_value(value)?;
        self.target.render_scene_fresh(&scene)
    }

    /// Drop memoized text measurements and force the next scene to repaint.
    /// Call after browser fonts load or change.
    #[cfg(feature = "js")]
    #[wasm_bindgen(js_name = invalidateMetrics)]
    pub fn invalidate_metrics(&mut self) {
        self.target.invalidate_metrics();
    }
}

#[cfg(feature = "js")]
impl CanvasRenderer {
    /// Render a typed Iron Canvas scene. This Rust entry point also lets
    /// application-specific hosts reuse the Canvas2D renderer without
    /// routing internal scene data through the JavaScript DTO.
    pub fn render_grid_scene(&mut self, scene: &GridScene) -> Result<(), JsValue> {
        self.target.render_scene_fresh(scene)
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

fn scene_from_value(value: JsValue) -> Result<GridScene, JsValue> {
    let dto: SceneDto = serde_wasm_bindgen::from_value(value)
        .map_err(|error| JsValue::from_str(&format!("invalid Iron Canvas scene: {error}")))?;
    dto.into_scene().map_err(|error| JsValue::from_str(&error))
}

/// Choose the retained update for the next scene. The first scene and a
/// dimension change are fresh; otherwise core compares paint bands.
fn plan_update(previous: Option<&SceneFingerprint>, fingerprint: &SceneFingerprint) -> SceneUpdate {
    previous.map_or(SceneUpdate::Fresh, |previous| fingerprint.damage(previous))
}

/// Palette returned by `themeFromElement`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThemeDto<'a> {
    grid_color: &'a str,
    grid_separator_color: &'a str,
    header_bg: &'a str,
    header_border_color: &'a str,
    header_text_color: &'a str,
    header_selected_bg: &'a str,
    header_selected_color: &'a str,
    default_text_color: &'a str,
    error_text_color: &'a str,
    selection_color: &'a str,
    cell_bg: &'a str,
    pointing: &'a str,
    selection_fill: &'a str,
    pointing_tint: &'a str,
}

impl<'a> From<&'a CanvasTheme> for ThemeDto<'a> {
    fn from(theme: &'a CanvasTheme) -> Self {
        Self {
            grid_color: &theme.grid_color,
            grid_separator_color: &theme.grid_separator_color,
            header_bg: &theme.header_bg,
            header_border_color: &theme.header_border_color,
            header_text_color: &theme.header_text_color,
            header_selected_bg: &theme.header_selected_bg,
            header_selected_color: &theme.header_selected_color,
            default_text_color: &theme.default_text_color,
            error_text_color: &theme.error_text_color,
            selection_color: &theme.selection_color,
            cell_bg: &theme.cell_bg,
            pointing: &theme.pointing,
            selection_fill: &theme.selection_fill,
            pointing_tint: &theme.pointing_tint,
        }
    }
}

/// Build the palette from CSS custom properties on `element`'s computed
/// style. Falls back to the light theme when the document has no style.
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = themeFromElement)]
pub fn theme_from_element_value(element: &Element) -> Result<JsValue, JsValue> {
    let theme = theme_from_element::from_element(element);
    serde_wasm_bindgen::to_value(&ThemeDto::from(&theme))
        .map_err(|error| JsValue::from_str(&format!("failed to serialize theme: {error}")))
}

/// One axis window for scroll arithmetic. `last` is inclusive.
#[cfg(feature = "js")]
#[wasm_bindgen]
pub struct ScrollWindow {
    first: i32,
    last: i32,
}

#[cfg(feature = "js")]
#[wasm_bindgen]
impl ScrollWindow {
    #[wasm_bindgen(constructor)]
    pub fn new(first: i32, last: i32) -> Self {
        Self { first, last }
    }

    #[wasm_bindgen(getter)]
    pub fn first(&self) -> i32 {
        self.first
    }

    #[wasm_bindgen(getter)]
    pub fn last(&self) -> i32 {
        self.last
    }

    fn to_core(&self) -> ScrollAxisWindow {
        ScrollAxisWindow::new(self.first, self.last)
    }
}

/// Rounded extents for one axis. Pass runs that differ from the default; the
/// gaps use `defaultExtent`. Indices are one-based and inclusive.
#[cfg(feature = "js")]
#[wasm_bindgen]
pub struct ScrollAxis {
    axis: RunAxis,
}

#[cfg(feature = "js")]
#[wasm_bindgen]
impl ScrollAxis {
    #[wasm_bindgen(constructor)]
    pub fn new(default_extent: i32, last_index: i32, runs: JsValue) -> Result<ScrollAxis, JsValue> {
        let runs: Vec<ExtentRunDto> = if runs.is_undefined() || runs.is_null() {
            Vec::new()
        } else {
            serde_wasm_bindgen::from_value(runs)
                .map_err(|error| JsValue::from_str(&format!("invalid scroll axis runs: {error}")))?
        };
        let runs: Vec<ExtentRun> = runs.into_iter().map(ExtentRun::from).collect();
        RunAxis::from_runs(default_extent, last_index, &runs)
            .map(|axis| Self { axis })
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(getter, js_name = defaultExtent)]
    pub fn default_extent(&self) -> i32 {
        self.axis.default_extent()
    }

    #[wasm_bindgen(getter, js_name = lastIndex)]
    pub fn last_index(&self) -> i32 {
        self.axis.last_index()
    }

    #[wasm_bindgen(getter, js_name = totalExtent)]
    pub fn total_extent(&self) -> f64 {
        self.axis.total_extent() as f64
    }
}

/// One normalized position on a single axis.
#[cfg(feature = "js")]
#[wasm_bindgen]
pub struct AxisScroll {
    first: i32,
    offset: i32,
}

#[cfg(feature = "js")]
#[wasm_bindgen]
impl AxisScroll {
    #[wasm_bindgen(getter)]
    pub fn first(&self) -> i32 {
        self.first
    }

    #[wasm_bindgen(getter)]
    pub fn offset(&self) -> i32 {
        self.offset
    }

    fn from_core(value: scroll::AxisScroll) -> Self {
        Self {
            first: value.first,
            offset: value.offset,
        }
    }
}

/// First visible cell plus the pixel offset inside it.
#[cfg(feature = "js")]
#[wasm_bindgen]
pub struct ScrollPosition {
    first_row: i32,
    first_col: i32,
    offset_x: i32,
    offset_y: i32,
}

#[cfg(feature = "js")]
#[wasm_bindgen]
impl ScrollPosition {
    #[wasm_bindgen(constructor)]
    pub fn new(first_row: i32, first_col: i32, offset_x: i32, offset_y: i32) -> Self {
        Self {
            first_row,
            first_col,
            offset_x,
            offset_y,
        }
    }

    #[wasm_bindgen(getter, js_name = firstRow)]
    pub fn first_row(&self) -> i32 {
        self.first_row
    }

    #[wasm_bindgen(getter, js_name = firstCol)]
    pub fn first_col(&self) -> i32 {
        self.first_col
    }

    #[wasm_bindgen(getter, js_name = offsetX)]
    pub fn offset_x(&self) -> i32 {
        self.offset_x
    }

    #[wasm_bindgen(getter, js_name = offsetY)]
    pub fn offset_y(&self) -> i32 {
        self.offset_y
    }

    fn to_core(&self) -> scroll::ScrollPosition {
        scroll::ScrollPosition {
            first_cell: CellCoord {
                row: self.first_row,
                col: self.first_col,
            },
            offset_x: self.offset_x,
            offset_y: self.offset_y,
        }
    }

    fn from_core(value: scroll::ScrollPosition) -> Self {
        Self {
            first_row: value.first_cell.row,
            first_col: value.first_cell.col,
            offset_x: value.offset_x,
            offset_y: value.offset_y,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtentRunDto {
    first: i32,
    last: i32,
    extent: i32,
}

impl From<ExtentRunDto> for ExtentRun {
    fn from(value: ExtentRunDto) -> Self {
        Self {
            first: value.first,
            last: value.last,
            extent: value.extent,
        }
    }
}

/// Pixels of scrolling content inside `window`.
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = scrollingExtent)]
pub fn scrolling_extent_js(axis: &ScrollAxis, window: &ScrollWindow) -> f64 {
    scrolling_extent(&axis.axis, window.to_core()) as f64
}

/// Clamp a requested position to a legal cell and offset.
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = normalizeAxis)]
pub fn normalize_axis_js(
    first: i32,
    offset: i32,
    axis: &ScrollAxis,
    window: &ScrollWindow,
) -> AxisScroll {
    AxisScroll::from_core(normalize_axis(first, offset, &axis.axis, window.to_core()))
}

/// Move an axis position by a signed pixel delta.
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = moveAxis)]
pub fn move_axis_js(
    first: i32,
    offset: i32,
    delta: i32,
    axis: &ScrollAxis,
    window: &ScrollWindow,
    pane: i32,
) -> AxisScroll {
    AxisScroll::from_core(move_axis(
        first,
        offset,
        delta,
        &axis.axis,
        window.to_core(),
        pane,
    ))
}

/// Apply the pane end limit without new input.
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = clampAxisEnd)]
pub fn clamp_axis_end_js(
    first: i32,
    offset: i32,
    axis: &ScrollAxis,
    window: &ScrollWindow,
    pane: i32,
) -> AxisScroll {
    AxisScroll::from_core(scroll::clamp_axis_end(
        first,
        offset,
        &axis.axis,
        window.to_core(),
        pane,
    ))
}

/// Normalize both axes of a scroll position.
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = normalizeScroll)]
pub fn normalize_scroll_js(
    position: &ScrollPosition,
    rows: &ScrollAxis,
    row_window: &ScrollWindow,
    columns: &ScrollAxis,
    column_window: &ScrollWindow,
) -> ScrollPosition {
    ScrollPosition::from_core(normalize_position(
        position.to_core(),
        &rows.axis,
        row_window.to_core(),
        &columns.axis,
        column_window.to_core(),
    ))
}

/// Move both axes. Positive deltas move content left and up.
#[allow(clippy::too_many_arguments)]
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = moveScroll)]
pub fn move_scroll_js(
    position: &ScrollPosition,
    delta_x: i32,
    delta_y: i32,
    rows: &ScrollAxis,
    row_window: &ScrollWindow,
    columns: &ScrollAxis,
    column_window: &ScrollWindow,
    pane_width: i32,
    pane_height: i32,
) -> ScrollPosition {
    ScrollPosition::from_core(move_position(
        position.to_core(),
        delta_x,
        delta_y,
        &rows.axis,
        row_window.to_core(),
        &columns.axis,
        column_window.to_core(),
        pane_width,
        pane_height,
    ))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct ModifiersDto {
    primary: bool,
    shift: bool,
    alt: bool,
    alt_graph: bool,
}

impl From<ModifiersDto> for NavigationModifiers {
    fn from(value: ModifiersDto) -> Self {
        Self {
            primary: value.primary,
            shift: value.shift,
            alt: value.alt,
            alt_graph: value.alt_graph,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum DirectionDto {
    Up,
    Down,
    Left,
    Right,
}

impl From<Direction> for DirectionDto {
    fn from(value: Direction) -> Self {
        match value {
            Direction::Up => Self::Up,
            Direction::Down => Self::Down,
            Direction::Left => Self::Left,
            Direction::Right => Self::Right,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum TraversalAxisDto {
    Rows,
    Columns,
}

impl From<TraversalAxis> for TraversalAxisDto {
    fn from(value: TraversalAxis) -> Self {
        match value {
            TraversalAxis::Rows => Self::Rows,
            TraversalAxis::Columns => Self::Columns,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum TraversalDirectionDto {
    Forward,
    Backward,
}

impl From<TraversalDirection> for TraversalDirectionDto {
    fn from(value: TraversalDirection) -> Self {
        match value {
            TraversalDirection::Forward => Self::Forward,
            TraversalDirection::Backward => Self::Backward,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum PageDirectionDto {
    Up,
    Down,
}

impl From<PageDirection> for PageDirectionDto {
    fn from(value: PageDirection) -> Self {
        match value {
            PageDirection::Up => Self::Up,
            PageDirection::Down => Self::Down,
        }
    }
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum IntentDto {
    Step {
        direction: DirectionDto,
    },
    ExtendStep {
        direction: DirectionDto,
    },
    DataEdge {
        direction: DirectionDto,
    },
    Traverse {
        axis: TraversalAxisDto,
        direction: TraversalDirectionDto,
    },
    Page {
        direction: PageDirectionDto,
    },
    RowStart,
    RowEnd,
    SheetStart,
    UsedEnd,
    SelectUsedRange,
    Cancel,
}

impl From<NavigationIntent> for IntentDto {
    fn from(value: NavigationIntent) -> Self {
        match value {
            NavigationIntent::Step(direction) => Self::Step {
                direction: direction.into(),
            },
            NavigationIntent::ExtendStep(direction) => Self::ExtendStep {
                direction: direction.into(),
            },
            NavigationIntent::DataEdge(direction) => Self::DataEdge {
                direction: direction.into(),
            },
            NavigationIntent::Traverse { axis, direction } => Self::Traverse {
                axis: axis.into(),
                direction: direction.into(),
            },
            NavigationIntent::Page(direction) => Self::Page {
                direction: direction.into(),
            },
            NavigationIntent::RowStart => Self::RowStart,
            NavigationIntent::RowEnd => Self::RowEnd,
            NavigationIntent::SheetStart => Self::SheetStart,
            NavigationIntent::UsedEnd => Self::UsedEnd,
            NavigationIntent::SelectUsedRange => Self::SelectUsedRange,
            NavigationIntent::Cancel => Self::Cancel,
        }
    }
}

/// Classify a normalized key for a host-owned grid. Returns an intent object,
/// or `null` when the key does not map. The host applies the intent to its
/// model; this binding performs no engine writes.
#[cfg(feature = "js")]
#[wasm_bindgen(js_name = classifyNavigation)]
pub fn classify_navigation_js(
    key: &str,
    modifiers: JsValue,
    profile: Option<String>,
) -> Result<JsValue, JsValue> {
    let profile = match profile.as_deref() {
        None | Some("basic") => BindingProfile::Basic,
        Some("extended") => BindingProfile::Extended,
        Some(other) => {
            return Err(JsValue::from_str(&format!(
                "unknown navigation profile: {other}"
            )));
        }
    };
    let Some(key) = NavigationKey::from_key(key) else {
        return Ok(JsValue::NULL);
    };
    let modifiers: ModifiersDto = if modifiers.is_undefined() || modifiers.is_null() {
        ModifiersDto::default()
    } else {
        serde_wasm_bindgen::from_value(modifiers)
            .map_err(|error| JsValue::from_str(&format!("invalid navigation modifiers: {error}")))?
    };
    match classify_navigation(key, modifiers.into(), profile) {
        None => Ok(JsValue::NULL),
        Some(intent) => serde_wasm_bindgen::to_value(&IntentDto::from(intent))
            .map_err(|error| JsValue::from_str(&format!("failed to serialize intent: {error}"))),
    }
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
            let mut builder = SceneBuilder::with_capacity(commands.len());
            for command in commands {
                match command {
                    PrimitiveDto::RectFill { rect, color } => {
                        builder.rect_fill(rect.into_rect()?, PaintColor::Borrowed(&color));
                    }
                    PrimitiveDto::RectFillRun { rect, edges, color } => {
                        let rect = rect.into_rect()?;
                        validate_run_edges(&rect, &edges)?;
                        builder.rect_fill_run(rect, &edges, PaintColor::Borrowed(&color));
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
                            PointF::new(x, y),
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
    RectFillRun {
        rect: RectDto,
        edges: Vec<i32>,
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

/// `rect_fill_run` expects ordered boundaries from `rect.left` through
/// `rect.right`, with no repeated edge. Reject bad input before the builder's
/// debug assertion can fire.
fn validate_run_edges(rect: &PixelRect, edges: &[i32]) -> Result<(), String> {
    let valid = edges.len() >= 2
        && edges.first() == Some(&rect.left())
        && edges.last() == Some(&rect.right())
        && edges.windows(2).all(|pair| pair[0] < pair[1]);
    if valid {
        Ok(())
    } else {
        Err("rectFillRun edges must increase strictly from left to right".into())
    }
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

#[cfg(all(test, feature = "js"))]
mod tests {
    use super::{IntentDto, SceneDto, ThemeDto, plan_update};
    use iron_canvas_core::{
        CanvasTheme,
        navigation::{
            Direction, NavigationIntent, PageDirection, TraversalAxis, TraversalDirection,
        },
        scene::retained::{SceneFingerprint, SceneUpdate},
    };
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

    fn scene_from_json(value: serde_json::Value) -> iron_canvas_core::scene::GridScene {
        let dto: SceneDto = serde_json::from_value(value).unwrap();
        dto.into_scene().unwrap()
    }

    #[test]
    fn builds_rect_fill_runs_with_ordered_edges() {
        let scene = scene_from_json(json!({
            "clip": { "left": 0, "top": 0, "width": 60, "height": 20 },
            "passes": [[{
                "type": "rectFillRun",
                "rect": { "left": 0, "top": 0, "width": 60, "height": 20 },
                "edges": [0, 15, 40, 60],
                "color": "#123456"
            }]]
        }));

        match &scene.passes[0][0] {
            iron_canvas_core::scene::Primitive::RectFillRun { rect, edges, color } => {
                assert_eq!(rect.width, 60);
                assert_eq!(&**edges, &[0, 15, 40, 60]);
                assert_eq!(&**color, "#123456");
            }
            other => panic!("expected a rect fill run, got {other:?}"),
        }
    }

    #[test]
    fn rejects_rect_fill_runs_with_unordered_edges() {
        let dto: SceneDto = serde_json::from_value(json!({
            "clip": { "left": 0, "top": 0, "width": 60, "height": 20 },
            "passes": [[{
                "type": "rectFillRun",
                "rect": { "left": 0, "top": 0, "width": 60, "height": 20 },
                "edges": [0, 40, 15, 60],
                "color": "#123456"
            }]]
        }))
        .unwrap();

        assert!(
            dto.into_scene()
                .unwrap_err()
                .contains("edges must increase strictly")
        );
    }

    #[test]
    fn plans_a_fresh_reuse_and_damage_update() {
        let first = scene_from_json(json!({
            "clip": { "left": 0, "top": 0, "width": 10, "height": 96 },
            "passes": [[{
                "type": "rectFill",
                "rect": { "left": 0, "top": 0, "width": 10, "height": 32 },
                "color": "#ffffff"
            }]]
        }));
        let same = first.clone();
        let moved = scene_from_json(json!({
            "clip": { "left": 0, "top": 0, "width": 10, "height": 96 },
            "passes": [[{
                "type": "rectFill",
                "rect": { "left": 0, "top": 32, "width": 10, "height": 32 },
                "color": "#ffffff"
            }]]
        }));

        let first_fingerprint = SceneFingerprint::new(&first);
        assert_eq!(plan_update(None, &first_fingerprint), SceneUpdate::Fresh);
        assert_eq!(
            plan_update(Some(&first_fingerprint), &SceneFingerprint::new(&same)),
            SceneUpdate::Reuse
        );
        assert!(matches!(
            plan_update(Some(&first_fingerprint), &SceneFingerprint::new(&moved)),
            SceneUpdate::Damage(_)
        ));
    }

    #[test]
    fn serializes_the_theme_with_camel_case_keys() {
        let theme = CanvasTheme::light();
        let value = serde_json::to_value(ThemeDto::from(&theme)).unwrap();

        assert_eq!(value["gridColor"], theme.grid_color.as_ref());
        assert_eq!(value["selectionFill"], theme.selection_fill.as_ref());
        assert_eq!(value["pointingTint"], theme.pointing_tint.as_ref());
    }

    #[test]
    fn serializes_navigation_intents_as_tagged_objects() {
        let cases = [
            (
                NavigationIntent::ExtendStep(Direction::Left),
                json!({ "type": "extendStep", "direction": "left" }),
            ),
            (
                NavigationIntent::Traverse {
                    axis: TraversalAxis::Rows,
                    direction: TraversalDirection::Backward,
                },
                json!({ "type": "traverse", "axis": "rows", "direction": "backward" }),
            ),
            (
                NavigationIntent::Page(PageDirection::Down),
                json!({ "type": "page", "direction": "down" }),
            ),
            (NavigationIntent::UsedEnd, json!({ "type": "usedEnd" })),
            (NavigationIntent::Cancel, json!({ "type": "cancel" })),
        ];
        for (intent, expected) in cases {
            assert_eq!(
                serde_json::to_value(IntentDto::from(intent)).unwrap(),
                expected
            );
        }
    }
}
