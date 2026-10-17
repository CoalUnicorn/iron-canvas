use iron_canvas::{RenderRequest, RevisionToken};
use iron_canvas_core::{CanvasTheme, CellCoord, painter::TextMetrics};
use iron_canvas_ironcalc::{WorksheetViewport, autofit::fit_height};
use ironcalc_base::{UserModel, expressions::types::Area};

pub fn fixture() -> (UserModel<'static>, RenderRequest) {
    let mut model = UserModel::new_empty("Unicode wrap", "en", "UTC", "en").unwrap();
    model.set_columns_width(0, 1, 4, 115.0).unwrap();
    for (index, text) in [
        "https://example.com/longpath?query=abcdefghijklmnop",
        "漢字、漢字。（漢字）長い文字列",
        "a\u{301} e\u{301} 👩‍💻 🇬🇧 👍🏽 👨‍👩‍👧‍👦",
        "  Ág AV office  \n\ntrailing spaces  ",
    ]
    .into_iter()
    .enumerate()
    {
        let row = index as i32 + 1;
        model.set_user_input(0, row, 1, text).unwrap();
        let cell = Area {
            sheet: 0,
            row,
            column: 1,
            width: 1,
            height: 1,
        };
        for (key, value) in [
            ("alignment.wrap_text", "true"),
            (
                "alignment.vertical",
                ["top", "center", "bottom", "top"][index],
            ),
            ("font.size", ["14", "18", "20", "16"][index]),
        ] {
            model.update_range_style(&cell, key, value).unwrap();
        }
        let mut style = model.get_cell_style(0, row, 1).unwrap();
        style.font.name = ["sans-serif", "serif", "sans-serif", "serif"][index].into();
        model.set_selected_cell(row, 1).unwrap();
        model.on_paste_styles(&[vec![style]]).unwrap();
    }
    let request = RenderRequest {
        sheet: 0,
        revision: RevisionToken {
            workbook_id: 71,
            revision: 0,
        },
        viewport: WorksheetViewport {
            first_cell: CellCoord { row: 1, col: 1 },
            width: 500,
            height: 600,
        },
        theme: CanvasTheme::light(),
        overlays: Default::default(),
    };
    (model, request)
}

pub fn fit(model: &mut UserModel<'_>, metrics: &dyn TextMetrics) {
    for row in 1..=4 {
        let height = fit_height(model, metrics, 0, row, 1, 1).unwrap().unwrap();
        model.set_rows_height(0, row, row, height.ceil()).unwrap();
    }
}

pub fn update(model: &mut UserModel<'_>, step: usize) {
    let area = Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 1,
        height: 4,
    };
    match step {
        0 => {} // Rebuild with equal text to exercise warm line layout.
        1 => model.set_columns_width(0, 1, 1, 83.0).unwrap(),
        2 => model.update_range_style(&area, "font.b", "true").unwrap(),
        3 => model.update_range_style(&area, "font.i", "true").unwrap(),
        4 => model.update_range_style(&area, "font.size", "22").unwrap(),
        _ => model
            .set_user_input(0, 3, 1, "e\u{301}👩‍💻 — changed wrap")
            .unwrap(),
    }
}
