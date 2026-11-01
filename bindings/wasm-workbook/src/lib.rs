//! Workbook renderer binding for IronCalc's WebAssembly model.
//!
//! IronCalc owns the workbook, the selected view, and navigation. This crate
//! paints the committed model view with Iron Canvas. A JavaScript host calls
//! `Model` methods directly and asks this renderer for a frame. The React
//! example consumes this crate; it holds no wasm adapter of its own.

pub use wasm::Model;

use iron_canvas::{
    CanvasSession, HostIntent, OverlayState, PointerAction, RenderRequest, RevisionToken,
    TextMetricsKey, WorksheetCache,
};
use iron_canvas_core::{
    CanvasMetrics, CanvasSize, CanvasTheme, CellCoord, Point, ScrollPosition,
    scene_geometry::GridRange,
};
use iron_canvas_ironcalc::{
    AxesScope, WorksheetAxes, WorksheetViewport, get_column_sizes, get_row_sizes,
};
use iron_canvas_navigation::{
    NavigationController, NavigationScope, ScrollCommand, ScrollViewport,
};
use iron_canvas_wasm::SceneTarget;
use ironcalc_base::UserModel;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

/// Logical pixels for one wheel line in line mode.
const LINE_STEP: f64 = 24.0;

/// Paints one IronCalc model into a canvas.
#[wasm_bindgen]
pub struct CanvasRenderer {
    canvas: HtmlCanvasElement,
    metrics: CanvasMetrics,
    session: CanvasSession<SceneTarget>,
    cache: WorksheetCache,
    /// Keyed by workbook, stable sheet ID, and text metric generation.
    axes: HashMap<AxesScope, Rc<WorksheetAxes>>,
    scroll: NavigationController,
    metrics_generation: u64,
    workbook_id: u64,
    revision: u64,
}

#[wasm_bindgen]
impl CanvasRenderer {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<CanvasRenderer, JsValue> {
        let metrics = metrics_for(0.0, 0.0, 1.0)?;
        let session = new_session(canvas.clone(), metrics)?;
        Ok(CanvasRenderer {
            canvas,
            metrics,
            session,
            cache: WorksheetCache::new(1),
            axes: HashMap::new(),
            scroll: NavigationController::default(),
            metrics_generation: 0,
            workbook_id: 1,
            revision: 0,
        })
    }

    /// Forget prepared content and view state after a workbook replacement.
    #[wasm_bindgen(js_name = reset)]
    pub fn reset(&mut self) -> Result<(), JsValue> {
        let workbook_id = self
            .workbook_id
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("workbook identity exhausted"))?;
        self.workbook_id = workbook_id;
        self.revision = 0;
        self.cache.clear();
        self.axes.clear();
        self.scroll
            .reset()
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Resize the paint target. The new target starts with one fresh frame.
    #[wasm_bindgen(js_name = resize)]
    pub fn resize(
        &mut self,
        model: &mut Model,
        width: f64,
        height: f64,
        dpr: f64,
    ) -> Result<(), JsValue> {
        let metrics = metrics_for(width, height, dpr)?;
        if metrics != self.metrics {
            let generation = self
                .metrics_generation
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("text metric generation exhausted"))?;
            // Target size and scale are fixed per session. Construction also
            // clears the canvas, so the next frame repaints every pixel.
            self.session = new_session(self.canvas.clone(), metrics)?;
            self.metrics = metrics;
            self.metrics_generation = generation;
            self.cache.clear();
        }
        self.paint(model)?;
        // Validate a saved position against the new pane only after the new
        // target has its first frame.
        if self.render_request(model).is_some() {
            let scope = self.scope(model)?;
            if self.scroll.view_position(scope).is_some() {
                let axes = self.axes_for_selected_sheet(model)?;
                let viewport = self.viewport_context()?;
                let update = self
                    .scroll
                    .resize_view(model.user_model_mut(), scope, &axes, viewport)
                    .map_err(|error| JsValue::from_str(&error.to_string()))?;
                if update.changed {
                    self.paint(model)?;
                }
            }
        }
        self.update_window(model);
        Ok(())
    }

    /// Refresh metric-dependent content after browser fonts change.
    #[wasm_bindgen(js_name = invalidateFonts)]
    pub fn invalidate_fonts(&mut self, model: &Model) -> Result<(), JsValue> {
        self.metrics_generation = self
            .metrics_generation
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("text metric generation exhausted"))?;
        self.cache.clear();
        self.session.invalidate();
        self.paint(model)
    }

    /// Reconcile the controller after the host changes the selection directly.
    ///
    /// Call this before a following wheel event. The controller then reveals
    /// the engine selection before the wheel changes the pixel position.
    #[wasm_bindgen(js_name = syncNavigation)]
    pub fn sync_navigation(&mut self, model: &mut Model) -> Result<(), JsValue> {
        let (width, height) = self.metrics.size().to_logical_extent();
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.ensure_view_imported(model)?;
        let scope = self.scope(model)?;
        let axes = self.axes_for_selected_sheet(model)?;
        let viewport = self.viewport_context()?;
        self.scroll
            .navigation_applied(model.user_model_mut(), scope, &axes, viewport)
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Paint the current model view.
    ///
    /// Set `reveal_active` after a navigation command. The renderer then
    /// reveals the new selection and keeps the model's scroll origin. Wheel
    /// input and pointer selection pass `false` and keep the pixel position.
    #[wasm_bindgen(js_name = render)]
    pub fn render(&mut self, model: &mut Model, reveal_active: bool) -> Result<(), JsValue> {
        let (width, height) = self.metrics.size().to_logical_extent();
        if width == 0 || height == 0 {
            return Ok(());
        }
        if reveal_active && let Err(error) = self.sync_navigation(model) {
            // An engine write can fail after it moves the view. Present the
            // actual view and report the failure once.
            let presentation = self.paint(model);
            return Err(presentation.err().unwrap_or(error));
        }
        self.paint(model)?;
        self.update_window(model);
        Ok(())
    }

    /// Wheel input in browser units. `mode` is 0 for pixels, 1 for lines,
    /// and 2 for pages. Shift maps vertical movement to horizontal only when
    /// the device supplied no horizontal movement.
    #[wasm_bindgen(js_name = wheel)]
    pub fn wheel(
        &mut self,
        model: &mut Model,
        delta_x: f64,
        delta_y: f64,
        mode: u32,
        shift: bool,
    ) -> Result<(), JsValue> {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return Err(JsValue::from_str("wheel delta must be finite"));
        }
        let (width, height) = self.metrics.size().to_logical_extent();
        if width == 0 || height == 0 {
            return Ok(());
        }
        let (mut dx, mut dy) = (delta_x, delta_y);
        if shift && dx == 0.0 {
            dx = dy;
            dy = 0.0;
        }
        match mode {
            0 => {}
            1 => {
                dx *= LINE_STEP;
                dy *= LINE_STEP;
            }
            2 => {
                let Some((pane_width, pane_height)) = self.committed_pane_extent() else {
                    return Ok(());
                };
                dx *= f64::from(pane_width);
                dy *= f64::from(pane_height);
            }
            _ => return Err(JsValue::from_str("unknown wheel mode")),
        }
        if dx == 0.0 && dy == 0.0 {
            return Ok(());
        }
        self.ensure_view_imported(model)?;
        if self.session.committed_frame().is_none() {
            return Ok(());
        }
        let scope = self.scope(model)?;
        let axes = self.axes_for_selected_sheet(model)?;
        let viewport = self.viewport_context()?;
        self.scroll
            .apply_scroll(
                model.user_model_mut(),
                scope,
                ScrollCommand::Pixels { dx, dy },
                &axes,
                viewport,
            )
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(())
    }

    /// Select the cell under a pointer position in the committed frame.
    #[wasm_bindgen(js_name = selectAt)]
    pub fn select_at(&mut self, model: &mut Model, x: i32, y: i32) -> Result<(), JsValue> {
        if let Some(HostIntent::Select { anchor, .. }) = self
            .session
            .pointer_intent(Point { x, y }, PointerAction::Select)
        {
            model
                .user_model_mut()
                .set_selected_cell(anchor.row, anchor.col)
                .map_err(|error| JsValue::from_str(&error))?;
        }
        Ok(())
    }

    /// Full cell rectangle for an editor text box, `[x, y, width, height]`.
    /// Empty when the cell is outside the committed panes.
    #[wasm_bindgen(js_name = cellRect)]
    pub fn cell_rect(&self, row: i32, col: i32) -> Vec<i32> {
        self.session
            .cell_rect(row, col)
            .map_or_else(Vec::new, |rect| {
                vec![rect.left(), rect.top(), rect.width, rect.height]
            })
    }

    /// Pane-clipped rectangles for the same cell, four values per fragment.
    #[wasm_bindgen(js_name = cellFragmentRects)]
    pub fn cell_fragment_rects(&self, row: i32, col: i32) -> Vec<i32> {
        let cell = CellCoord { row, col };
        self.session
            .visible_fragments(GridRange {
                first: cell,
                last: cell,
            })
            .into_iter()
            .flat_map(|fragment| {
                [
                    fragment.rect.left(),
                    fragment.rect.top(),
                    fragment.rect.width,
                    fragment.rect.height,
                ]
            })
            .collect()
    }
}

impl CanvasRenderer {
    fn paint(&mut self, model: &Model) -> Result<(), JsValue> {
        let Some(request) = self.render_request(model) else {
            return Ok(());
        };
        self.cache
            .set_capacity(visible_sheet_count(model.user_model()));
        self.session
            .render_cached(
                &mut self.cache,
                model.user_model(),
                &request,
                TextMetricsKey::new(1, self.metrics_generation),
            )
            .map_err(|error| JsValue::from_str(&format!("{error:?}")))?;
        Ok(())
    }

    fn render_request(&self, model: &Model) -> Option<RenderRequest> {
        let (width, height) = self.metrics.size().to_logical_extent();
        if width == 0 || height == 0 {
            return None;
        }

        let view = model.user_model().get_selected_view();
        let scroll = self
            .scope(model)
            .ok()
            .and_then(|scope| self.scroll.view_position(scope))
            .unwrap_or_else(|| {
                ScrollPosition::aligned(CellCoord {
                    row: view.top_row.max(1),
                    col: view.left_column.max(1),
                })
            });
        Some(RenderRequest {
            sheet: view.sheet,
            revision: RevisionToken {
                workbook_id: self.workbook_id,
                revision: self.revision,
            },
            viewport: WorksheetViewport {
                scroll,
                width,
                height,
            },
            theme: CanvasTheme::light(),
            overlays: OverlayState {
                selection: vec![GridRange {
                    first: CellCoord {
                        row: view.range[0],
                        col: view.range[1],
                    },
                    last: CellCoord {
                        row: view.range[2],
                        col: view.range[3],
                    },
                }],
                active_cell: Some(CellCoord {
                    row: view.row.max(1),
                    col: view.column.max(1),
                }),
                ..OverlayState::default()
            },
        })
    }

    /// Engine navigation uses the pane size to scroll and page.
    fn update_window(&self, model: &mut Model) {
        if let Some((width, height)) = self.committed_pane_extent() {
            let workbook = model.user_model_mut();
            workbook.set_window_width(f64::from(width));
            workbook.set_window_height(f64::from(height));
        }
    }

    fn scope(&self, model: &Model) -> Result<NavigationScope, JsValue> {
        let workbook = model.user_model();
        let sheet = workbook.get_selected_sheet();
        let sheet_id = workbook
            .get_model()
            .workbook
            .worksheet(sheet)
            .map_err(|error| JsValue::from_str(&error))?
            .sheet_id;
        Ok(NavigationScope {
            workbook_id: self.workbook_id,
            view_id: 0,
            sheet_id,
        })
    }

    fn axes_for_selected_sheet(&mut self, model: &Model) -> Result<Rc<WorksheetAxes>, JsValue> {
        let workbook = model.user_model();
        let sheet = workbook.get_selected_sheet();
        let sheet_id = workbook
            .get_model()
            .workbook
            .worksheet(sheet)
            .map_err(|error| JsValue::from_str(&error))?
            .sheet_id;
        let key = AxesScope {
            workbook_id: self.workbook_id,
            sheet_id,
            revision: self.revision,
        };
        if let Some(axes) = self.axes.get(&key) {
            return Ok(Rc::clone(axes));
        }
        let rows = get_row_sizes(workbook, sheet)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let columns = get_column_sizes(workbook, sheet)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let axes = Rc::new(
            WorksheetAxes::from_sizes(&rows, &columns)
                .map_err(|error| JsValue::from_str(&error.to_string()))?
                .with_scope(key),
        );
        if self.axes.len() >= 8 {
            self.axes.clear();
        }
        self.axes.insert(key, Rc::clone(&axes));
        Ok(axes)
    }

    fn ensure_view_imported(&mut self, model: &mut Model) -> Result<(), JsValue> {
        let (width, height) = self.metrics.size().to_logical_extent();
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.session.committed_frame().is_none() {
            self.paint(model)?;
        }
        let scope = self.scope(model)?;
        self.scroll.set_primary_view(Some(scope));
        let axes = self.axes_for_selected_sheet(model)?;
        let viewport = self.viewport_context()?;
        self.scroll
            .import_view(model.user_model_mut(), scope, &axes, viewport)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(())
    }

    fn committed_pane_extent(&self) -> Option<(i32, i32)> {
        let frame = self.session.committed_frame()?;
        let geometry = frame.worksheet.geometry.as_ref()?;
        let pane = geometry.scroll_pane_rect();
        Some((pane.width, pane.height))
    }

    fn viewport_context(&self) -> Result<ScrollViewport, JsValue> {
        let frame = self
            .session
            .committed_frame()
            .ok_or_else(|| JsValue::from_str("wait for worksheet geometry"))?;
        let geometry = frame
            .worksheet
            .geometry
            .as_ref()
            .ok_or_else(|| JsValue::from_str("wait for worksheet geometry"))?;
        let pane = geometry.scroll_pane_rect();
        Ok(ScrollViewport {
            pane_width: pane.width,
            pane_height: pane.height,
            frozen_rows: frame.worksheet.frozen.rows,
            frozen_columns: frame.worksheet.frozen.cols,
        })
    }
}

fn visible_sheet_count(workbook: &UserModel<'_>) -> usize {
    workbook
        .get_worksheets_properties()
        .iter()
        .filter(|sheet| sheet.state == "visible")
        .count()
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

fn new_session(
    canvas: HtmlCanvasElement,
    metrics: CanvasMetrics,
) -> Result<CanvasSession<SceneTarget>, JsValue> {
    Ok(CanvasSession::new(SceneTarget::new(canvas, metrics)?))
}
