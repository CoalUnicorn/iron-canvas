#[cfg(target_os = "linux")]
use std::{cell::RefCell, collections::HashMap, env, path::Path, process, rc::Rc};

#[cfg(target_os = "linux")]
use gtk4::{
    Application, ApplicationWindow, DrawingArea, EventControllerKey, EventControllerScroll,
    GestureClick, Label, Notebook, Orientation, PositionType, gdk, glib, prelude::*,
};

#[cfg(target_os = "linux")]
use iron_canvas::{
    BindingProfile, CanvasSession, HostIntent, NavigationIntent, NavigationKey,
    NavigationModifiers, OverlayState, PointerAction, RenderRequest, RevisionToken, TextMetricsKey,
    WorksheetCache, classify_navigation,
};

#[cfg(target_os = "linux")]
use iron_canvas_core::{
    CanvasTheme, CellCoord, Point, ScrollPosition,
    scene_geometry::{GridHit, GridRange},
};

#[cfg(target_os = "linux")]
use iron_canvas_ironcalc::{WorksheetAxes, WorksheetViewport, get_column_sizes, get_row_sizes};

#[cfg(target_os = "linux")]
use iron_canvas_native::NativeSceneBackend;

#[cfg(target_os = "linux")]
use iron_canvas_navigation::{
    NavigationController, NavigationScope, ScrollCommand, ScrollUpdate, ScrollViewport,
    apply_navigation, select_anchor_and_focus,
};

#[cfg(target_os = "linux")]
use ironcalc::import::load_from_xlsx;

#[cfg(target_os = "linux")]
use ironcalc_base::UserModel;

/// Logical pixels for one wheel unit. Surface-unit deltas are already pixels.
#[cfg(target_os = "linux")]
const LINE_STEP: f64 = 24.0;

/// Worksheet limits; the crate keeps these private.
#[cfg(target_os = "linux")]
const LAST_ROW: i32 = 1_048_576;

#[cfg(target_os = "linux")]
const LAST_COLUMN: i32 = 16_384;

#[cfg(target_os = "linux")]
struct ViewerState {
    workbook: UserModel<'static>,
    sheets: Vec<(u32, String)>,
    session: CanvasSession<NativeSceneBackend>,
    request: RenderRequest,
    cache: WorksheetCache,
    /// Owns the accepted view positions and the ordered command queue.
    navigation: NavigationController,
    /// Axis extents per sheet. The example uses one fixed metric environment.
    axes: HashMap<u32, Rc<WorksheetAxes>>,
    /// Fixed cell of the last pointer selection, used by Shift+click.
    anchor: CellCoord,
    /// The last host error, or `None` after a successful input.
    status: Option<String>,
}

#[cfg(target_os = "linux")]
impl ViewerState {
    fn new(workbook: UserModel<'static>) -> Result<Self, gtk4::cairo::Error> {
        let properties = workbook.get_worksheets_properties();
        let sheets = properties
            .into_iter()
            .enumerate()
            .filter(|(_, sheet)| sheet.state == "visible")
            .map(|(index, sheet)| (index as u32, sheet.name))
            .collect::<Vec<_>>();
        let first_sheet = sheets.first().map_or(0, |(index, _)| *index);
        let mut workbook = workbook;
        // The engine must select the sheet that the first tab shows, because
        // the navigation controller mirrors the selected engine view only.
        let _ = workbook.set_selected_sheet(first_sheet);
        let view = workbook.get_selected_view();
        Ok(Self {
            workbook,
            cache: WorksheetCache::new(sheets.len()),
            sheets,
            session: CanvasSession::new(NativeSceneBackend::new(1.0)?),
            request: RenderRequest {
                sheet: first_sheet,
                revision: RevisionToken {
                    workbook_id: 1,
                    revision: 1,
                },
                viewport: WorksheetViewport::aligned(CellCoord { row: 1, col: 1 }, 900, 620),
                theme: CanvasTheme::light(),
                overlays: OverlayState::default(),
            },
            navigation: NavigationController::default(),
            axes: HashMap::new(),
            anchor: CellCoord {
                row: view.row.max(1),
                col: view.column.max(1),
            },
            status: None,
        })
    }

    fn is_selected(&self, sheet: u32) -> bool {
        self.workbook.get_selected_sheet() == sheet
    }

    fn scope(&self, sheet: u32) -> Option<NavigationScope> {
        let sheet_id = self
            .workbook
            .get_model()
            .workbook
            .worksheet(sheet)
            .ok()?
            .sheet_id;
        Some(NavigationScope {
            workbook_id: 1,
            view_id: 0,
            sheet_id,
        })
    }

    fn axes_for(&mut self, sheet: u32) -> Result<Rc<WorksheetAxes>, String> {
        if let Some(axes) = self.axes.get(&sheet) {
            return Ok(Rc::clone(axes));
        }
        let rows = get_row_sizes(&self.workbook, sheet).map_err(|error| error.to_string())?;
        let columns = get_column_sizes(&self.workbook, sheet).map_err(|error| error.to_string())?;
        let axes =
            Rc::new(WorksheetAxes::from_sizes(&rows, &columns).map_err(|error| error.to_string())?);
        self.axes.insert(sheet, Rc::clone(&axes));
        Ok(axes)
    }

    /// Committed pane size, or `None` before the first frame.
    fn pane_extent(&self) -> Option<(i32, i32)> {
        let frame = self.session.committed_frame()?;
        let pane = frame.worksheet.geometry.as_ref()?.scroll_pane_rect();
        Some((pane.width, pane.height))
    }

    /// Pane and freeze inputs for the next scroll or reveal call.
    fn viewport_context(&self) -> Option<ScrollViewport> {
        let frame = self.session.committed_frame()?;
        let pane = frame.worksheet.geometry.as_ref()?.scroll_pane_rect();
        Some(ScrollViewport {
            pane_width: pane.width,
            pane_height: pane.height,
            frozen_rows: frame.worksheet.frozen.rows,
            frozen_columns: frame.worksheet.frozen.cols,
        })
    }

    /// IronCalc uses its window size for page navigation.
    fn update_window(&mut self) {
        if let Some((width, height)) = self.pane_extent() {
            self.workbook.set_window_width(f64::from(width));
            self.workbook.set_window_height(f64::from(height));
        }
    }

    fn import_view(&mut self, sheet: u32) -> Result<ScrollUpdate, String> {
        let viewport = self
            .viewport_context()
            .ok_or_else(|| "worksheet geometry is not ready".to_owned())?;
        let scope = self
            .scope(sheet)
            .ok_or_else(|| "sheet identity is not available".to_owned())?;
        let axes = self.axes_for(sheet)?;
        self.navigation.set_primary_view(Some(scope));
        self.navigation
            .import_view(&mut self.workbook, scope, &axes, viewport)
            .map_err(|error| error.to_string())
    }

    fn ensure_imported(&mut self, sheet: u32) -> Result<(), String> {
        let scope = self
            .scope(sheet)
            .ok_or_else(|| "sheet identity is not available".to_owned())?;
        if self.navigation.view_position(scope).is_some() {
            self.navigation.set_primary_view(Some(scope));
            return Ok(());
        }
        self.import_view(sheet).map(|_| ())
    }

    /// Reconcile the stored position with the engine after a selection change.
    fn sync_view(&mut self, sheet: u32) -> Result<(), String> {
        let Some(scope) = self.scope(sheet) else {
            return Ok(());
        };
        if self.navigation.view_position(scope).is_none() {
            return Ok(());
        }
        let Some(viewport) = self.viewport_context() else {
            return Ok(());
        };
        let axes = self.axes_for(sheet)?;
        self.navigation
            .navigation_applied(&mut self.workbook, scope, &axes, viewport)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn select_sheet(&mut self, sheet: u32) {
        if !self.is_selected(sheet) {
            if let Err(error) = self.workbook.set_selected_sheet(sheet) {
                self.status = Some(error);
                return;
            }
            let view = self.workbook.get_selected_view();
            self.anchor = CellCoord {
                row: view.row.max(1),
                col: view.column.max(1),
            };
        }
        if let Some(scope) = self.scope(sheet) {
            self.navigation.set_primary_view(Some(scope));
            // Restore a saved pixel position before the next frame.
            if self.navigation.view_position(scope).is_some()
                && self.session.committed_frame().is_some()
                && let Err(error) = self.import_view(sheet)
            {
                self.status = Some(error);
            }
        }
    }

    /// Wheel input in logical pixels, with Shift mapped to the horizontal axis.
    fn scroll(&mut self, sheet: u32, dx: f64, dy: f64) -> Result<(), String> {
        if !self.is_selected(sheet) {
            return Ok(());
        }
        if !dx.is_finite() || !dy.is_finite() {
            return Err("scroll delta must be finite".to_owned());
        }
        if dx == 0.0 && dy == 0.0 {
            return Ok(());
        }
        self.ensure_imported(sheet)?;
        let viewport = self
            .viewport_context()
            .ok_or_else(|| "worksheet geometry is not ready".to_owned())?;
        let scope = self
            .scope(sheet)
            .ok_or_else(|| "sheet identity is not available".to_owned())?;
        let axes = self.axes_for(sheet)?;
        self.navigation
            .apply_scroll(
                &mut self.workbook,
                scope,
                ScrollCommand::Pixels { dx, dy },
                &axes,
                viewport,
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Apply one classified key intent, then reveal the new selection.
    fn navigate(&mut self, sheet: u32, intent: NavigationIntent) -> Result<(), String> {
        if !self.is_selected(sheet) {
            return Ok(());
        }
        self.update_window();
        let pane_height = self.pane_extent().map_or(0, |(_, height)| height);
        let engine_error = apply_navigation(&mut self.workbook, intent, pane_height).err();
        // An engine write can fail after it moves the view. Always present the
        // actual view, so reconcile even when the command reported an error.
        let sync_error = self.sync_view(sheet).err();
        match (engine_error, sync_error) {
            (Some(error), _) | (None, Some(error)) => Err(error),
            (None, None) => Ok(()),
        }
    }

    /// Select the cell or header under a committed-frame point.
    ///
    /// Cell hits use the engine area selection, which moves the view to reveal
    /// the pointer target. Header hits set the full row or column with
    /// `set_selected_range`, which does not scroll. The follow-up reveal then
    /// shows the selected row or column without crossing the sheet, because a
    /// full-axis range already covers the pane on its long axis.
    fn select(&mut self, point: Point, extend: bool) -> Result<(), String> {
        let action = if extend {
            PointerAction::ExtendSelection
        } else {
            PointerAction::Select
        };
        let Some(HostIntent::Select { anchor, .. }) = self.session.pointer_intent(point, action)
        else {
            return Ok(());
        };
        match self.session.hit_test(point) {
            Some(GridHit::Cell(_)) => {
                let anchor_cell = if extend { self.anchor } else { anchor };
                select_anchor_and_focus(
                    &mut self.workbook,
                    anchor_cell.row,
                    anchor_cell.col,
                    anchor.row,
                    anchor.col,
                )?;
                self.anchor = anchor_cell;
            }
            Some(GridHit::RowHeader(row)) => {
                let (first, last) = if extend {
                    ordered(self.anchor.row, row)
                } else {
                    (row, row)
                };
                let (active_row, active_column) = if extend {
                    (self.anchor.row, self.anchor.col)
                } else {
                    (row, 1)
                };
                self.workbook.set_selected_cell(active_row, active_column)?;
                self.workbook
                    .set_selected_range(first, 1, last, LAST_COLUMN)?;
                if !extend {
                    self.anchor = CellCoord { row, col: 1 };
                }
            }
            Some(GridHit::ColumnHeader(column)) => {
                let (first, last) = if extend {
                    ordered(self.anchor.col, column)
                } else {
                    (column, column)
                };
                let (active_row, active_column) = if extend {
                    (self.anchor.row, self.anchor.col)
                } else {
                    (1, column)
                };
                self.workbook.set_selected_cell(active_row, active_column)?;
                self.workbook.set_selected_range(1, first, LAST_ROW, last)?;
                if !extend {
                    self.anchor = CellCoord {
                        row: 1,
                        col: column,
                    };
                }
            }
            Some(GridHit::Corner) => {
                let (active_row, active_column) = if extend {
                    (self.anchor.row, self.anchor.col)
                } else {
                    (1, 1)
                };
                self.workbook.set_selected_cell(active_row, active_column)?;
                self.workbook
                    .set_selected_range(1, 1, LAST_ROW, LAST_COLUMN)?;
                if !extend {
                    self.anchor = CellCoord { row: 1, col: 1 };
                }
            }
            None => return Ok(()),
        }
        let sheet = self.workbook.get_selected_sheet();
        self.sync_view(sheet)?;
        Ok(())
    }

    /// Prepare the request for one sheet and commit a frame.
    fn render(&mut self, sheet: u32, width: i32, height: i32) -> Result<(), String> {
        self.request.sheet = sheet;
        self.request.viewport.width = width;
        self.request.viewport.height = height;

        let view = self.workbook.get_selected_view();
        let selected = view.sheet;
        let position = if sheet == selected {
            self.scope(sheet)
                .and_then(|scope| self.navigation.view_position(scope))
                .unwrap_or_else(|| {
                    ScrollPosition::aligned(CellCoord {
                        row: view.top_row.max(1),
                        col: view.left_column.max(1),
                    })
                })
        } else {
            // Only the selected engine sheet has a view. Other tabs draw from
            // their own saved position, or from A1 before the first visit.
            ScrollPosition::aligned(CellCoord { row: 1, col: 1 })
        };
        self.request.viewport.scroll = position;

        self.request.overlays = if sheet == selected {
            OverlayState {
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
            }
        } else {
            OverlayState::default()
        };

        let ViewerState {
            workbook,
            session,
            request,
            cache,
            ..
        } = self;
        session
            .render_cached(cache, workbook, request, TextMetricsKey::new(1, 0))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Import or re-clamp the view after a frame publishes new pane geometry.
    /// Returns true when the position changed and needs one more frame.
    fn after_frame(&mut self, sheet: u32) -> bool {
        if !self.is_selected(sheet) || self.session.committed_frame().is_none() {
            return false;
        }
        let Some(scope) = self.scope(sheet) else {
            return false;
        };
        if self.navigation.view_position(scope).is_none() {
            return match self.import_view(sheet) {
                Ok(update) => update.changed,
                Err(error) => {
                    self.status = Some(error);
                    false
                }
            };
        }
        let Some(viewport) = self.viewport_context() else {
            return false;
        };
        let axes = match self.axes_for(sheet) {
            Ok(axes) => axes,
            Err(error) => {
                self.status = Some(error);
                return false;
            }
        };
        match self
            .navigation
            .resize_view(&mut self.workbook, scope, &axes, viewport)
        {
            Ok(update) => update.changed,
            Err(error) => {
                self.status = Some(error.to_string());
                false
            }
        }
    }

    fn status_text(&self, sheet: u32) -> String {
        let Some((_, name)) = self.sheets.iter().find(|(index, _)| *index == sheet) else {
            return String::new();
        };
        if !self.is_selected(sheet) {
            return name.clone();
        }
        let view = self.workbook.get_selected_view();
        let scroll = self
            .scope(sheet)
            .and_then(|scope| self.navigation.view_position(scope))
            .unwrap_or_else(|| {
                ScrollPosition::aligned(CellCoord {
                    row: view.top_row.max(1),
                    col: view.left_column.max(1),
                })
            });
        format!(
            "{name} · active {}{} · range {}{}:{}{} · origin {}{} +{}px,+{}px",
            column_name(view.column),
            view.row,
            column_name(view.range[1]),
            view.range[0],
            column_name(view.range[3]),
            view.range[2],
            column_name(scroll.first_cell.col),
            scroll.first_cell.row,
            scroll.offset_x,
            scroll.offset_y,
        )
    }
}

#[cfg(target_os = "linux")]
fn main() {
    let Some(path) = env::args_os().nth(1) else {
        eprintln!(
            "Usage: cargo run -p iron-canvas --example linux_xlsx_viewer_gtk4 -- <file.xlsx>"
        );
        process::exit(2);
    };
    let path = Path::new(&path);
    let workbook_path = path.to_string_lossy();
    let workbook = match load_from_xlsx(&workbook_path, "en", "UTC", "en") {
        Ok(workbook) => UserModel::from_model(workbook),
        Err(error) => {
            eprintln!("Could not open {}: {error}", path.display());
            process::exit(1);
        }
    };
    let state = match ViewerState::new(workbook) {
        Ok(state) => Rc::new(RefCell::new(state)),
        Err(error) => {
            eprintln!("Could not initialize the Cairo renderer: {error}");
            process::exit(1);
        }
    };

    let application = Application::builder()
        .application_id("org.ironcanvas.XlsxViewer")
        .build();
    let workbook_name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    application.connect_activate(move |application| {
        build_window(application, state.clone(), &workbook_name);
    });

    // The workbook path is handled above. Do not let GTK parse it as an
    // application option or attempt to open it as a second application file.
    let program = env::args()
        .next()
        .unwrap_or_else(|| String::from("linux_xlsx_viewer_gtk4"));
    let _ = application.run_with_args(&[program.as_str()]);
}

#[cfg(target_os = "linux")]
fn build_window(application: &Application, state: Rc<RefCell<ViewerState>>, workbook_name: &str) {
    let window = ApplicationWindow::builder()
        .application(application)
        .title(format!("Iron Canvas — {workbook_name}"))
        .default_width(1050)
        .default_height(720)
        .build();

    let notebook = Notebook::new();
    notebook.set_tab_pos(PositionType::Bottom);
    notebook.set_scrollable(true);
    notebook.set_show_border(false);
    notebook.set_vexpand(true);
    notebook.set_hexpand(true);

    // One line below the grid reports the selection and the pixel origin.
    let status = Label::new(None);
    status.set_xalign(0.0);
    status.set_margin_top(4);
    status.set_margin_bottom(4);
    status.set_margin_start(8);
    status.set_margin_end(8);
    status.set_selectable(true);

    let sheets = state.borrow().sheets.clone();
    for (sheet_index, sheet_name) in &sheets {
        let sheet_index = *sheet_index;
        let drawing_area = DrawingArea::new();
        drawing_area.set_content_width(900);
        drawing_area.set_content_height(620);
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        // Key events need a focusable grid and a click must return focus.
        drawing_area.set_focusable(true);

        let draw_state = state.clone();
        let draw_status = status.clone();
        drawing_area.set_draw_func(move |_area, context, width, height| {
            if let Ok(mut state) = draw_state.try_borrow_mut() {
                draw_frame(&mut state, sheet_index, context, width, height);
                let text = state
                    .status
                    .clone()
                    .unwrap_or_else(|| state.status_text(sheet_index));
                // Changing a sibling widget during snapshot triggers a layout
                // warning. Update the label after the frame is complete.
                if draw_status.text().as_str() != text {
                    let draw_status = draw_status.clone();
                    glib::idle_add_local_once(move || draw_status.set_text(&text));
                }
            }
        });

        // Wheel and touchpad input use the shared pixel scroll state. GTK
        // reports wheel notches in wheel units and touchpads in pixels.
        let scroll = EventControllerScroll::new(gtk4::EventControllerScrollFlags::BOTH_AXES);
        let scroll_state = state.clone();
        let scroll_area = drawing_area.clone();
        scroll.connect_scroll(move |controller, dx, dy| {
            let shift = controller
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK);
            let (dx, dy) = wheel_pixels(controller.unit(), dx, dy, shift);
            if let Ok(mut state) = scroll_state.try_borrow_mut() {
                state.status = state.scroll(sheet_index, dx, dy).err();
            }
            scroll_area.queue_draw();
            glib::Propagation::Stop
        });
        drawing_area.add_controller(scroll);

        // Same classification and engine writes as the other hosts.
        let keys = EventControllerKey::new();
        let key_state = state.clone();
        let key_area = drawing_area.clone();
        keys.connect_key_pressed(move |_controller, keyval, _keycode, modifiers| {
            let Some(key) = navigation_key(keyval) else {
                return glib::Propagation::Proceed;
            };
            let navigation_modifiers = NavigationModifiers {
                primary: modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                    || modifiers.contains(gdk::ModifierType::META_MASK),
                shift: modifiers.contains(gdk::ModifierType::SHIFT_MASK),
                alt: modifiers.contains(gdk::ModifierType::ALT_MASK)
                    || modifiers.contains(gdk::ModifierType::SUPER_MASK),
                alt_graph: false,
            };
            let Some(intent) =
                classify_navigation(key, navigation_modifiers, BindingProfile::Basic)
            else {
                return glib::Propagation::Proceed;
            };
            if let Ok(mut state) = key_state.try_borrow_mut() {
                state.status = state.navigate(sheet_index, intent).err();
            }
            key_area.queue_draw();
            glib::Propagation::Stop
        });
        drawing_area.add_controller(keys);

        // Pointer input resolves against the committed frame. Shift extends
        // from the stored anchor instead of starting a new selection.
        let click = GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let click_state = state.clone();
        let click_area = drawing_area.clone();
        click.connect_pressed(move |gesture, _presses, x, y| {
            click_area.grab_focus();
            let extend = gesture
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK);
            let point = Point {
                x: x.round() as i32,
                y: y.round() as i32,
            };
            if let Ok(mut state) = click_state.try_borrow_mut() {
                state.status = state.select(point, extend).err();
            }
            click_area.queue_draw();
        });
        drawing_area.add_controller(click);

        notebook.append_page(&drawing_area, Some(&Label::new(Some(sheet_name))));
    }

    let switch_state = state.clone();
    notebook.connect_switch_page(move |notebook, _page, page| {
        let sheet = switch_state
            .borrow()
            .sheets
            .get(page as usize)
            .map(|(index, _)| *index);
        if let Some(sheet) = sheet {
            if let Ok(mut state) = switch_state.try_borrow_mut() {
                state.select_sheet(sheet);
            }
            if let Some(widget) = notebook.nth_page(Some(page)) {
                widget.queue_draw();
            }
        }
    });

    let initial = sheets.first().map_or(0, |(index, _)| *index);
    if let Ok(mut state) = state.try_borrow_mut() {
        state.select_sheet(initial);
    }
    notebook.set_current_page(
        sheets
            .iter()
            .position(|(sheet, _)| *sheet == initial)
            .map(|page| page as u32),
    );

    let root = gtk4::Box::new(Orientation::Vertical, 0);
    root.append(&notebook);
    root.append(&status);
    window.set_child(Some(&root));
    window.present();
}

/// Convert GTK scroll units to logical pixels and apply the Shift mapping.
#[cfg(target_os = "linux")]
fn wheel_pixels(unit: gdk::ScrollUnit, dx: f64, dy: f64, shift: bool) -> (f64, f64) {
    let (dx, dy) = match unit {
        gdk::ScrollUnit::Wheel => (dx * LINE_STEP, dy * LINE_STEP),
        _ => (dx, dy),
    };
    if shift && dx == 0.0 {
        (dy, 0.0)
    } else {
        (dx, dy)
    }
}

/// Map a GDK key value to the shared keyboard classification input.
#[cfg(target_os = "linux")]
fn navigation_key(key: gdk::Key) -> Option<NavigationKey> {
    use gdk::Key;
    Some(match key {
        Key::Up => NavigationKey::Up,
        Key::Down => NavigationKey::Down,
        Key::Left => NavigationKey::Left,
        Key::Right => NavigationKey::Right,
        Key::Tab | Key::ISO_Left_Tab => NavigationKey::Tab,
        Key::Return | Key::KP_Enter => NavigationKey::Enter,
        Key::Page_Up | Key::KP_Page_Up => NavigationKey::PageUp,
        Key::Page_Down | Key::KP_Page_Down => NavigationKey::PageDown,
        Key::Home => NavigationKey::Home,
        Key::End => NavigationKey::End,
        Key::a | Key::A => NavigationKey::A,
        Key::space => NavigationKey::Space,
        Key::F8 => NavigationKey::F8,
        Key::Escape => NavigationKey::Escape,
        _ => return None,
    })
}

#[cfg(target_os = "linux")]
fn draw_frame(
    state: &mut ViewerState,
    sheet: u32,
    context: &gtk4::cairo::Context,
    width: i32,
    height: i32,
) {
    if width <= 0 || height <= 0 {
        return;
    }

    // A size change can move the accepted view after the first frame. Render
    // again only when the controller reports a changed position.
    for _ in 0..2 {
        if let Err(error) = state.render(sheet, width, height) {
            state.status = Some(format!("Could not render worksheet: {error}"));
            draw_message(context, error_text(state));
            return;
        }
        if !state.after_frame(sheet) {
            break;
        }
    }

    if let Some(surface) = state.session.backend().surface() {
        if let Err(error) = context.set_source_surface(surface, 0.0, 0.0) {
            state.status = Some(format!("Could not display worksheet: {error}"));
            draw_message(context, error_text(state));
            return;
        }
        if let Err(error) = context.paint() {
            state.status = Some(format!("Could not display worksheet: {error}"));
            draw_message(context, error_text(state));
        }
    }
}

#[cfg(target_os = "linux")]
fn error_text(state: &ViewerState) -> &str {
    state.status.as_deref().unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn draw_message(context: &gtk4::cairo::Context, message: &str) {
    context.set_source_rgb(0.75, 0.0, 0.0);
    context.move_to(12.0, 24.0);
    let _ = context.show_text(message);
}

#[cfg(target_os = "linux")]
fn ordered(first: i32, last: i32) -> (i32, i32) {
    (first.min(last), first.max(last))
}

#[cfg(target_os = "linux")]
fn column_name(mut column: i32) -> String {
    let mut name = String::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        name.insert(0, (b'A' + remainder as u8) as char);
        column = (column - 1) / 26;
    }
    name
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn surface(width: i32, height: i32) -> (gtk4::cairo::ImageSurface, gtk4::cairo::Context) {
        let surface =
            gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, width, height).unwrap();
        let context = gtk4::cairo::Context::new(&surface).unwrap();
        (surface, context)
    }

    #[test]
    fn tab_return_draws_cached_content_without_style_reads() {
        let mut workbook = UserModel::new_empty("tabs", "en", "UTC", "en").unwrap();
        workbook.new_sheet().unwrap();
        workbook.new_sheet().unwrap();
        workbook.hide_sheet(1).unwrap();
        for sheet in [0, 2] {
            workbook
                .set_user_input(sheet, 1, 1, &format!("sheet {sheet}"))
                .unwrap();
        }
        let mut state = ViewerState::new(workbook).unwrap();
        assert_eq!(
            state
                .sheets
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        let (_surface, context) = surface(900, 620);
        for sheet in [0, 2, 0] {
            draw_frame(&mut state, sheet, &context, 900, 620);
            let frame = state.session.committed_frame().unwrap();
            assert_eq!(frame.sheet, sheet);
            if frame.sequence == 3 {
                assert_eq!(frame.worksheet.extended_style_reads, 0);
            } else {
                assert!(frame.worksheet.extended_style_reads > 0);
            }
        }
        assert_eq!(state.cache.len(), 2);
    }

    #[test]
    fn wheel_scroll_and_keyboard_navigation_share_the_view_state() {
        let mut workbook = UserModel::new_empty("scroll", "en", "UTC", "en").unwrap();
        workbook.set_user_input(0, 1, 1, "origin").unwrap();
        let mut state = ViewerState::new(workbook).unwrap();
        let (_surface, context) = surface(900, 620);
        draw_frame(&mut state, 0, &context, 900, 620);

        let scope = state.scope(0).unwrap();
        let aligned = ScrollPosition::aligned(CellCoord { row: 1, col: 1 });
        // The first frame imports the engine origin into the controller.
        assert_eq!(state.navigation.view_position(scope), Some(aligned));

        state.scroll(0, 0.0, 80.0).unwrap();
        let scrolled = state.navigation.view_position(scope).unwrap();
        assert_ne!(scrolled, aligned, "wheel input must move the pixel origin");
        draw_frame(&mut state, 0, &context, 900, 620);
        assert_eq!(state.request.viewport.scroll, scrolled);

        state
            .navigate(0, NavigationIntent::Step(iron_canvas::Direction::Down))
            .unwrap();
        assert_eq!(state.workbook.get_selected_cell().1, 2);
        let revealed = state.navigation.view_position(scope).unwrap();
        assert!(
            revealed.first_cell.row <= 2,
            "keyboard navigation must reveal the new active cell"
        );
        draw_frame(&mut state, 0, &context, 900, 620);
        assert_eq!(
            state.request.overlays.active_cell,
            Some(CellCoord { row: 2, col: 1 })
        );
    }

    #[test]
    fn pointer_selection_uses_committed_geometry_and_paints_overlays() {
        let mut workbook = UserModel::new_empty("pointer", "en", "UTC", "en").unwrap();
        workbook.set_user_input(0, 1, 1, "origin").unwrap();
        let mut state = ViewerState::new(workbook).unwrap();
        let (_surface, context) = surface(900, 620);
        draw_frame(&mut state, 0, &context, 900, 620);

        state.select(Point { x: 140, y: 140 }, false).unwrap();
        let (_, row, column) = state.workbook.get_selected_cell();
        assert_ne!((row, column), (1, 1));
        assert_eq!(state.anchor, CellCoord { row, col: column });

        draw_frame(&mut state, 0, &context, 900, 620);
        assert_eq!(
            state.request.overlays.active_cell,
            Some(CellCoord { row, col: column })
        );
        assert_eq!(
            state.request.overlays.selection,
            vec![GridRange {
                first: CellCoord { row, col: column },
                last: CellCoord { row, col: column },
            }]
        );
    }

    /// A header click selects the full row or column without scrolling the
    /// other axis to the worksheet limit.
    #[test]
    fn header_selection_keeps_the_orthogonal_origin() {
        let mut workbook = UserModel::new_empty("headers", "en", "UTC", "en").unwrap();
        workbook.set_user_input(0, 1, 1, "origin").unwrap();
        let mut state = ViewerState::new(workbook).unwrap();
        let (_surface, context) = surface(900, 620);
        draw_frame(&mut state, 0, &context, 900, 620);

        let scope = state.scope(0).unwrap();
        state.scroll(0, 600.0, 400.0).unwrap();
        draw_frame(&mut state, 0, &context, 900, 620);
        let pane = state
            .session
            .committed_frame()
            .unwrap()
            .worksheet
            .geometry
            .as_ref()
            .unwrap()
            .scroll_pane_rect();
        let before = state.navigation.view_position(scope).unwrap();

        let row_header = Point {
            x: pane.left() / 2,
            y: pane.top() + 30,
        };
        assert!(
            matches!(
                state.session.hit_test(row_header),
                Some(GridHit::RowHeader(_))
            ),
            "test point must hit the row header"
        );
        state.select(row_header, false).unwrap();
        let after_row = state.navigation.view_position(scope).unwrap();
        assert_eq!(
            after_row.first_cell.col, before.first_cell.col,
            "row header must not scroll to the last column"
        );
        let view = state.workbook.get_selected_view();
        assert_eq!((view.range[1], view.range[3]), (1, LAST_COLUMN));

        let column_header = Point {
            x: pane.left() + 40,
            y: pane.top() / 2,
        };
        assert!(
            matches!(
                state.session.hit_test(column_header),
                Some(GridHit::ColumnHeader(_))
            ),
            "test point must hit the column header"
        );
        state.select(column_header, false).unwrap();
        let after_column = state.navigation.view_position(scope).unwrap();
        assert_eq!(
            after_column.first_cell.row, after_row.first_cell.row,
            "column header must not scroll to the last row"
        );
        let view = state.workbook.get_selected_view();
        assert_eq!((view.range[0], view.range[2]), (1, LAST_ROW));
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The linux_xlsx_viewer_gtk4 example is only available on Linux.");
}
