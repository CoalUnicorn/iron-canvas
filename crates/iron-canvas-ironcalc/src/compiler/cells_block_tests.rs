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

fn compile_with_blocks(
    model: &UserModel<'_>,
    sheet: u32,
    viewport: WorksheetViewport,
    limits: Option<cells::BlockLimits>,
) -> PreparedWorksheet {
    let theme = CanvasTheme::light();
    let mut cache = TextLayoutCache::default();
    let mut shaper = TextShaper::new(&Metrics, TextMetricsKey::new(0, 0), &mut cache);
    composition::prepare_with_blocks(
        WorksheetContext {
            workbook: model,
            sheet,
            theme: &theme,
        },
        viewport,
        &mut shaper,
        WorksheetReuse::default(),
        limits,
    )
    .unwrap()
}

fn compare_frames(model: &UserModel<'_>, sheet: u32, viewport: WorksheetViewport) {
    let reference = compile_with_blocks(model, sheet, viewport, None);
    for limits in [
        cells::BlockLimits::default(),
        cells::BlockLimits {
            cells_per_attempt: 4,
            width: 2,
            height: 2,
            ..cells::BlockLimits::default()
        },
    ] {
        let candidate = compile_with_blocks(model, sheet, viewport, Some(limits));
        assert_eq!(candidate.scene, reference.scene);
        assert_eq!(candidate.geometry, reference.geometry);
        assert_eq!(candidate.merges, reference.merges);
        assert_eq!(candidate.links, reference.links);
        assert_eq!(
            candidate.cells().collect::<Vec<_>>(),
            reference.cells().collect::<Vec<_>>()
        );
    }
}

#[test]
fn block_frames_match_individual_frames_with_fallback_merges_and_hidden_gaps() {
    let mut model = UserModel::new_empty("frames", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "anchor").unwrap();
    model
        .merge_cells(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 8,
            height: 4,
        })
        .unwrap();
    model
        .set_user_input(0, 5, 3, "offscreen spill source")
        .unwrap();
    model.set_user_input(0, 6, 6, "=\"\"").unwrap();
    model.set_columns_hidden(0, 9, 10, true).unwrap();
    model.set_rows_hidden(0, 8, 9, true).unwrap();
    let viewport = WorksheetViewport {
        first_cell: CellCoord { row: 3, col: 5 },
        width: 500,
        height: 300,
    };
    compare_frames(&model, 0, viewport);
    model.set_frozen_rows_count(0, 1).unwrap();
    model.set_frozen_columns_count(0, 1).unwrap();
    compare_frames(&model, 0, viewport);
}

#[test]
fn block_frames_match_individual_cf_frames() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cf_tests.xlsx");
    let model =
        UserModel::from_model(ironcalc::import::load_from_xlsx(path, "en", "UTC", "en").unwrap());
    for sheet in 0..7 {
        compare_frames(
            &model,
            sheet,
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 700,
                height: 600,
            },
        );
    }
}

#[test]
fn block_records_preserve_occupancy_and_count_styles_and_fallback_once() {
    let mut model = UserModel::new_empty("counts", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "=\"\"").unwrap();
    let viewport = WorksheetViewport {
        first_cell: CellCoord { row: 1, col: 1 },
        width: 350,
        height: 150,
    };
    let prepared = compile_with_blocks(&model, 0, viewport, None);
    let theme = CanvasTheme::light();
    let mut cache = TextLayoutCache::default();
    let mut shaper = TextShaper::new(&Metrics, TextMetricsKey::new(0, 0), &mut cache);
    let mut attempt = Attempt::new(
        WorksheetContext {
            workbook: &model,
            sheet: 0,
            theme: &theme,
        },
        &mut shaper,
        None,
    )
    .unwrap();
    attempt
        .load_blocks(
            prepared.geometry.as_ref().unwrap(),
            cells::BlockLimits::default(),
        )
        .unwrap();
    let style_count = attempt.reads;
    assert!(attempt.read_stats.block_calls > 0);
    assert!(style_count < attempt.read_stats.block_cells);
    assert_eq!(style_count, attempt.read_stats.block_styles);
    for _ in 0..2 {
        attempt.read(CellCoord { row: 1, col: 1 }).unwrap();
        attempt.read(CellCoord { row: 100, col: 100 }).unwrap();
    }
    assert!(attempt.cached.view(CellCoord { row: 1, col: 1 }).occupied);
    assert_eq!(attempt.cached.view(CellCoord { row: 1, col: 1 }).value, "");
    assert_eq!(attempt.read_stats.fallback_cells, 1);
    assert_eq!(attempt.reads, style_count + 1);
}

#[test]
fn failed_block_read_reports_bounds_and_does_not_retry_individual_cells() {
    let model = UserModel::new_empty("errors", "en", "UTC", "en").unwrap();
    let viewport = WorksheetViewport {
        first_cell: CellCoord { row: 1, col: 1 },
        width: 200,
        height: 100,
    };
    let prepared = compile_with_blocks(&model, 0, viewport, None);
    let theme = CanvasTheme::light();
    let mut cache = TextLayoutCache::default();
    let mut shaper = TextShaper::new(&Metrics, TextMetricsKey::new(0, 0), &mut cache);
    let mut attempt = Attempt::new(
        WorksheetContext {
            workbook: &model,
            sheet: 0,
            theme: &theme,
        },
        &mut shaper,
        None,
    )
    .unwrap();
    // Use an absent sheet after reading the valid sheet metadata. This isolates
    // block error mapping from earlier sheet validation.
    attempt.context.sheet = 99;
    let error = attempt
        .load_blocks(
            prepared.geometry.as_ref().unwrap(),
            cells::BlockLimits::default(),
        )
        .unwrap_err();
    assert_eq!(error.operation, CellReadOperation::CellsBlock);
    assert_eq!(error.sheet, 99);
    assert_eq!(error.cell, CellCoord { row: 1, col: 1 });
    assert!(error.message.starts_with("block "));
    assert_eq!(attempt.read_stats.fallback_cells, 0);
    assert_eq!(attempt.read_stats.block_calls, 0);
}
