use iron_canvas::{RenderRequest, RevisionToken};
use iron_canvas_core::{CanvasTheme, CellCoord, PixelRect};
use iron_canvas_ironcalc::WorksheetViewport;
use ironcalc_base::{
    UserModel,
    expressions::types::Area,
    types::{BorderItem, BorderStyle, Color},
};

pub const TEXT: &str = "Offscreen text crosses filled and bordered empty cells";

pub fn fixture() -> (UserModel<'static>, RenderRequest) {
    let mut model = UserModel::new_empty("formatted spill", "en", "UTC", "en").unwrap();
    model.set_columns_width(0, 1, 8, 60.0).unwrap();
    model.set_rows_height(0, 1, 8, 32.0).unwrap();
    model.set_user_input(0, 2, 1, TEXT).unwrap();
    let request = RenderRequest {
        sheet: 0,
        revision: RevisionToken {
            workbook_id: 72,
            revision: 0,
        },
        viewport: WorksheetViewport {
            first_cell: CellCoord { row: 1, col: 2 },
            width: 348,
            height: 180,
        },
        theme: CanvasTheme::light(),
        overlays: Default::default(),
    };
    (model, request)
}

pub fn format(model: &mut UserModel<'_>, color: &str) {
    model
        .update_range_style(
            &Area {
                sheet: 0,
                row: 2,
                column: 1,
                width: 5,
                height: 1,
            },
            "fill.color",
            color,
        )
        .unwrap();
    let edge = BorderItem {
        style: BorderStyle::Thin,
        color: Color::Rgb("#123456".into()),
    };
    // Shared horizontal edges reach the text row through adjacent styles.
    // Vertical edges remain visible where text crosses them.
    for col in 1..=5 {
        for row in 1..=3 {
            let mut style = model.get_cell_style(0, row, col).unwrap();
            match row {
                1 => style.border.bottom = Some(edge.clone()),
                2 => {
                    style.border.left = Some(edge.clone());
                    style.border.right = Some(edge.clone());
                }
                _ => style.border.top = Some(edge.clone()),
            }
            model.set_selected_cell(row, col).unwrap();
            model.on_paste_styles(&[vec![style]]).unwrap();
        }
    }
}

pub fn interior_pixels(bytes: &[u8], stride: usize, rect: PixelRect, scale: f64) -> Vec<u8> {
    // Exclude the grid and explicit edges. Changes here prove that text reaches
    // the filled destination, even if both rendering paths share a clip error.
    let left = (f64::from(rect.left() + 4) * scale).ceil() as usize;
    let right = (f64::from(rect.right() - 4) * scale).floor() as usize;
    let top = (f64::from(rect.top() + 4) * scale).ceil() as usize;
    let bottom = (f64::from(rect.bottom() - 4) * scale).floor() as usize;
    (top..bottom)
        .flat_map(|row| {
            bytes[row * stride + left * 4..row * stride + right * 4]
                .iter()
                .copied()
        })
        .collect()
}
