//! Compare native block data with the compiler's individual read path.
use super::*;
use ironcalc_base::{expressions::types::Area, types::Cell};

struct Metrics;
impl TextMetrics for Metrics {
    fn measure_text_width(&self, text: &str, _: &str) -> f64 {
        text.len() as f64
    }
}

fn compare(model: &UserModel<'_>, sheet: u32, area: (i32, i32, i32, i32)) {
    let (row, column, width, height) = area;
    let block = model
        .get_cells_block(sheet, row, column, width, height)
        .unwrap();
    let theme = CanvasTheme::light();
    let mut cache = TextLayoutCache::default();
    let mut shaper = TextShaper::new(&Metrics, TextMetricsKey::new(0, 0), &mut cache);
    let mut attempt = Attempt::new(
        WorksheetContext {
            workbook: model,
            sheet,
            theme: &theme,
        },
        &mut shaper,
        None,
    )
    .unwrap();
    assert_eq!(block.style_indices.len(), (width * height) as usize);
    for r in row..row + height {
        for c in column..column + width {
            let coordinate = CellCoord { row: r, col: c };
            let reference = attempt.read_individual(coordinate).unwrap();
            let slot = ((r - row) * width + c - column) as usize;
            let extended = &block.styles[block.style_indices[slot] as usize];
            assert_eq!(
                extended.style, reference.extended.style,
                "style at sheet {sheet}, {coordinate:?}"
            );
            // ExtendedStyle has no PartialEq. Compare its base style and the
            // complete CF paint input that the compiler consumes.
            assert_eq!(
                CfDecorationPaint::resolve(extended, model),
                CfDecorationPaint::resolve(&reference.extended, model),
                "CF at sheet {sheet}, {coordinate:?}"
            );
            let text = block
                .texts
                .iter()
                .find(|text| text.row == r && text.column == c);
            assert_eq!(
                text.map_or("", |text| text.text.as_str()),
                reference.value,
                "text at sheet {sheet}, {coordinate:?}"
            );
            // Sparse display text has no occupancy flag. Read stored cells even
            // when the formatted value is empty, as the future adapter does.
            let stored = model
                .get_model()
                .workbook
                .worksheet(sheet)
                .unwrap()
                .cell(r, c);
            let occupied = stored
                .as_ref()
                .is_some_and(|cell| !matches!(cell.as_ref(), Cell::EmptyCell { .. }));
            assert_eq!(occupied, reference.occupied);
            let kind = match text {
                Some(text) => match text.cell_type {
                    1 => CellType::Number,
                    2 => CellType::Text,
                    4 => CellType::LogicalValue,
                    16 => CellType::ErrorValue,
                    64 => CellType::Array,
                    128 => CellType::CompoundData,
                    other => panic!("unknown block cell type {other}"),
                },
                None => stored.map_or(CellType::Number, |cell| cell.get_type()),
            };
            assert_eq!(kind, reference.kind, "type at {coordinate:?}");
        }
    }
    assert_eq!(attempt.reads, (width * height) as usize);
}

#[test]
fn block_matches_individual_values_and_empty_display_occupancy() {
    let mut model = UserModel::new_empty("blocks", "en", "UTC", "en").unwrap();
    for (column, input) in ["text", "42", "=TRUE()", "=1/0", "=1+2", "=\"\"", "42"]
        .iter()
        .enumerate()
    {
        model
            .set_user_input(0, 2, column as i32 + 1, input)
            .unwrap();
    }
    model
        .update_range_style(
            &Area {
                sheet: 0,
                row: 2,
                column: 7,
                width: 1,
                height: 1,
            },
            "num_fmt",
            ";;;",
        )
        .unwrap();
    // A style-only cell differs from a stored value with empty display text.
    model
        .update_range_style(
            &Area {
                sheet: 0,
                row: 2,
                column: 8,
                width: 1,
                height: 1,
            },
            "font.b",
            "true",
        )
        .unwrap();
    let block = model.get_cells_block(0, 1, 1, 9, 3).unwrap();
    for column in [6, 7, 8, 9] {
        assert!(
            !block
                .texts
                .iter()
                .any(|text| text.row == 2 && text.column == column)
        );
    }
    for (column, expected) in [(6, true), (7, true), (8, false), (9, false)] {
        let stored = model
            .get_model()
            .workbook
            .worksheet(0)
            .unwrap()
            .cell(2, column);
        assert_eq!(
            stored.is_some_and(|cell| !matches!(cell.as_ref(), Cell::EmptyCell { .. })),
            expected
        );
    }
    // Default styles share entries. This is the saving that later steps use.
    assert!(block.styles.len() < block.style_indices.len());
    compare(&model, 0, (1, 1, 9, 3));
}

#[test]
fn block_edges_keep_neighbor_borders_and_sheet_limits() {
    use ironcalc_base::types::{BorderItem, BorderStyle, Color};
    let mut model = UserModel::new_empty("borders", "en", "UTC", "en").unwrap();
    // Put borders outside the requested tile. The engine's ring supplies them.
    for (row, col) in [(1, 2), (2, 1), (2, 4), (4, 2)] {
        let mut style = model.get_cell_style(0, row, col).unwrap();
        let edge = Some(BorderItem {
            style: BorderStyle::Double,
            color: Color::Rgb("#123456".into()),
        });
        style.border.top = edge.clone();
        style.border.right = edge.clone();
        style.border.bottom = edge.clone();
        style.border.left = edge;
        model.set_selected_cell(row, col).unwrap();
        model.on_paste_styles(&[vec![style]]).unwrap();
    }
    for area in [
        (2, 2, 2, 2),
        (1, 1, 2, 2),
        (LAST_ROW - 1, LAST_COLUMN - 1, 2, 2),
    ] {
        compare(&model, 0, area);
    }
}

#[test]
fn block_matches_evaluated_cf_before_and_after_input_change() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cf_tests.xlsx");
    let mut model =
        UserModel::from_model(ironcalc::import::load_from_xlsx(path, "en", "UTC", "en").unwrap());
    for sheet in 0..7 {
        compare(&model, sheet, (1, 1, 12, 24));
    }
    // This range contains icons with hidden values and gradient/solid data bars.
    assert!(
        model
            .get_extended_cell_style(0, 17, 2)
            .unwrap()
            .icon
            .is_some()
    );
    assert!(
        model
            .get_extended_cell_style(0, 2, 8)
            .unwrap()
            .data_bar
            .is_some()
    );
    model.set_user_input(0, 2, 8, "12345").unwrap();
    model.evaluate();
    compare(&model, 0, (1, 1, 12, 24));
}
