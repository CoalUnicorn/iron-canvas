#[cfg(target_os = "linux")]
use std::{cell::RefCell, env, path::Path, process, rc::Rc};

#[cfg(target_os = "linux")]
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, ComboBoxText, DrawingArea,
    EventControllerScroll, Label, Orientation, glib, prelude::*,
};

#[cfg(target_os = "linux")]
use iron_canvas::{CanvasSession, RenderRequest, RevisionToken};

#[cfg(target_os = "linux")]
use iron_canvas_core::{CanvasTheme, CellCoord};

#[cfg(target_os = "linux")]
use iron_canvas_ironcalc::WorksheetViewport;

#[cfg(target_os = "linux")]
use iron_canvas_native::NativeSceneBackend;

#[cfg(target_os = "linux")]
use ironcalc::import::load_from_xlsx;

#[cfg(target_os = "linux")]
use ironcalc_base::UserModel;

#[cfg(target_os = "linux")]
struct ViewerState {
    workbook: UserModel<'static>,
    sheets: Vec<(u32, String)>,
    session: CanvasSession<NativeSceneBackend>,
    request: RenderRequest,
}

#[cfg(target_os = "linux")]
impl ViewerState {
    fn new(workbook: UserModel<'static>) -> Result<Self, gtk4::cairo::Error> {
        let sheets = workbook
            .get_worksheets_properties()
            .into_iter()
            .enumerate()
            .map(|(index, sheet)| (index as u32, sheet.name))
            .collect::<Vec<_>>();
        let first_sheet = sheets.first().map_or(0, |(index, _)| *index);
        Ok(Self {
            workbook,
            sheets,
            session: CanvasSession::new(NativeSceneBackend::new(1.0)?),
            request: RenderRequest {
                sheet: first_sheet,
                revision: RevisionToken {
                    workbook_id: 1,
                    revision: 1,
                },
                viewport: WorksheetViewport {
                    first_cell: CellCoord { row: 1, col: 1 },
                    width: 900,
                    height: 620,
                },
                theme: CanvasTheme::light(),
                overlays: Default::default(),
            },
        })
    }
}

#[cfg(target_os = "linux")]
fn main() {
    let Some(path) = env::args_os().nth(1) else {
        eprintln!("Usage: cargo run -p iron-canvas --example linux_xlsx_viewer -- <file.xlsx>");
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
        .unwrap_or_else(|| String::from("linux_xlsx_viewer"));
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

    let content = GtkBox::new(Orientation::Vertical, 0);
    let toolbar = GtkBox::new(Orientation::Horizontal, 8);
    toolbar.set_margin_start(10);
    toolbar.set_margin_end(10);
    toolbar.set_margin_top(8);
    toolbar.set_margin_bottom(8);
    toolbar.append(&Label::new(Some("Worksheet:")));
    let sheet_selector = ComboBoxText::new();
    let sheets = state.borrow().sheets.clone();
    for (sheet_index, sheet_name) in &sheets {
        sheet_selector.append(Some(&sheet_index.to_string()), sheet_name);
    }
    let selected_sheet = state.borrow().request.sheet.to_string();
    sheet_selector.set_active_id(Some(&selected_sheet));
    toolbar.append(&sheet_selector);
    toolbar.append(&Label::new(Some("Scroll to explore")));

    let drawing_area = DrawingArea::new();
    drawing_area.set_content_width(900);
    drawing_area.set_content_height(620);
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);

    let sheet_state = state.clone();
    let area_to_redraw = drawing_area.clone();
    sheet_selector.connect_changed(move |selector| {
        let Some(sheet) = selector.active_id().and_then(|id| id.parse::<u32>().ok()) else {
            return;
        };
        if let Ok(mut state) = sheet_state.try_borrow_mut() {
            state.request.sheet = sheet;
            state.request.viewport.first_cell = CellCoord { row: 1, col: 1 };
        }
        area_to_redraw.queue_draw();
    });

    let draw_state = state.clone();
    drawing_area.set_draw_func(move |_area, context, width, height| {
        let mut state = draw_state.borrow_mut();
        draw_frame(&mut state, context, width, height);
    });

    let scroll = EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::DISCRETE,
    );
    let scroll_state = state;
    let area_to_redraw = drawing_area.clone();
    scroll.connect_scroll(move |_controller, dx, dy| {
        if let Ok(mut state) = scroll_state.try_borrow_mut() {
            let first_cell = &mut state.request.viewport.first_cell;
            first_cell.row = advance_cell(first_cell.row, dy, 1_048_576);
            first_cell.col = advance_cell(first_cell.col, dx, 16_384);
        }
        area_to_redraw.queue_draw();
        glib::Propagation::Stop
    });
    drawing_area.add_controller(scroll);

    content.append(&toolbar);
    content.append(&drawing_area);
    window.set_child(Some(&content));
    window.present();
}

#[cfg(target_os = "linux")]
fn advance_cell(current: i32, delta: f64, maximum: i32) -> i32 {
    let steps = (delta * 3.0).round() as i32;
    current.saturating_add(steps).clamp(1, maximum)
}

#[cfg(target_os = "linux")]
fn draw_frame(state: &mut ViewerState, context: &gtk4::cairo::Context, width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        return;
    }

    state.request.viewport.width = width;
    state.request.viewport.height = height;
    let ViewerState {
        workbook,
        session,
        request,
        ..
    } = state;
    if let Err(error) = session.render(workbook, request) {
        draw_message(context, &format!("Could not render worksheet: {error}"));
        return;
    }

    if let Some(surface) = session.backend().surface() {
        if let Err(error) = context.set_source_surface(surface, 0.0, 0.0) {
            draw_message(context, &format!("Could not display worksheet: {error}"));
            return;
        }
        if let Err(error) = context.paint() {
            draw_message(context, &format!("Could not display worksheet: {error}"));
        }
    }
}

#[cfg(target_os = "linux")]
fn draw_message(context: &gtk4::cairo::Context, message: &str) {
    context.set_source_rgb(0.75, 0.0, 0.0);
    context.move_to(12.0, 24.0);
    let _ = context.show_text(message);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The linux_xlsx_viewer example is only available on Linux.");
}
