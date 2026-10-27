use iron_canvas_core::{
    CanvasTheme, CellCoord, PixelRect, Point, painter::TextMetrics, scene::Primitive,
};
use iron_canvas_ironcalc::{
    CellPlacement, PreparedWorksheet, WorksheetCompiler, WorksheetViewport,
};
use ironcalc_base::UserModel;
use ironcalc_base::expressions::types::Area;
use ironcalc_base::types as ic;
struct Metrics;
impl TextMetrics for Metrics {
    fn measure_text_width(&self, text: &str, _: &str) -> f64 {
        text.chars().count() as f64 * 8.0
    }
}
fn rect() -> PixelRect {
    PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: 100,
        height: 30,
    }
}

/// Compile a small viewport with the shared metrics/theme the other tests use.
fn prepared(workbook: &UserModel<'_>) -> PreparedWorksheet {
    WorksheetCompiler::new()
        .prepare(
            workbook,
            0,
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 100,
                height: 30,
            },
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap()
}

fn external(target: &str) -> ic::Link {
    ic::Link::External {
        target: target.to_string(),
        tooltip: None,
    }
}

#[test]
fn viewport_placements_keep_pane_order_and_full_bounds() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook.set_rows_height(0, 1, 10, 24.0).unwrap();
    workbook.set_columns_width(0, 1, 10, 60.0).unwrap();
    workbook.set_frozen_rows_count(0, 2).unwrap();
    workbook.set_frozen_columns_count(0, 2).unwrap();
    workbook.set_rows_hidden(0, 2, 2, true).unwrap();
    workbook.set_columns_hidden(0, 2, 2, true).unwrap();
    let viewport = WorksheetViewport {
        first_cell: CellCoord { row: 5, col: 5 },
        width: 180,
        height: 82,
    };
    let prepared = WorksheetCompiler::new()
        .prepare(&workbook, 0, viewport, &Metrics, &CanvasTheme::light())
        .unwrap();

    // The last row and column cross the viewport edge. Their placements
    // retain full bounds so callers can position text before clipping.
    let expected: Vec<_> = [
        (1, 1, 48, 24),
        (1, 5, 108, 24),
        (1, 6, 168, 24),
        (5, 1, 48, 48),
        (6, 1, 48, 72),
        (5, 5, 108, 48),
        (5, 6, 168, 48),
        (6, 5, 108, 72),
        (6, 6, 168, 72),
    ]
    .into_iter()
    .map(|(row, col, x, y)| CellPlacement {
        coordinate: CellCoord { row, col },
        bounds: PixelRect {
            top_left: Point { x, y },
            width: 60,
            height: 24,
        },
    })
    .collect();
    assert_eq!(prepared.cell_count(), expected.len());
    assert_eq!(prepared.cells().collect::<Vec<_>>(), expected);

    let reused = WorksheetCompiler::new()
        .prepare_reusing_geometry(
            &workbook,
            0,
            viewport,
            &Metrics,
            &CanvasTheme::light(),
            prepared.geometry.as_ref(),
        )
        .unwrap();
    assert!(std::rc::Rc::ptr_eq(
        prepared.geometry.as_ref().unwrap(),
        reused.geometry.as_ref().unwrap(),
    ));
    drop(prepared);
    assert_eq!(reused.clone().cells().collect::<Vec<_>>(), expected);
}

#[test]
fn explicit_placements_keep_input_order_rectangles_and_repeated_coordinates() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    let first = CellPlacement {
        coordinate: CellCoord { row: 2, col: 1 },
        bounds: rect(),
    };
    let second = CellPlacement {
        coordinate: CellCoord { row: 1, col: 1 },
        bounds: rect(),
    };
    let third = CellPlacement {
        bounds: PixelRect {
            top_left: Point { x: -20, y: 10 },
            width: 140,
            height: 40,
        },
        ..first
    };
    let placements = [first, second, third];
    let prepared = WorksheetCompiler::new()
        .prepare_cells(
            &workbook,
            0,
            rect(),
            placements,
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap();
    assert!(prepared.geometry.is_none());
    assert!(prepared.metadata().is_none());
    assert_eq!(prepared.extended_style_reads, 2);
    assert_eq!(prepared.cell_count(), placements.len());
    let cloned = prepared.clone();
    drop(prepared);
    assert_eq!(cloned.cells().collect::<Vec<_>>(), placements);
}

#[test]
fn empty_viewports_and_explicit_input_have_no_placements() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    for (width, height) in [(0, 0), (0, 100), (100, 0), (48, 100), (100, 24)] {
        let prepared = WorksheetCompiler::new()
            .prepare(
                &workbook,
                0,
                WorksheetViewport {
                    first_cell: CellCoord { row: 1, col: 1 },
                    width,
                    height,
                },
                &Metrics,
                &CanvasTheme::light(),
            )
            .unwrap();
        assert_eq!(prepared.cell_count(), 0);
        assert_eq!(prepared.cells().next(), None);
    }
    let explicit = WorksheetCompiler::new()
        .prepare_cells(&workbook, 0, rect(), [], &Metrics, &CanvasTheme::light())
        .unwrap();
    assert_eq!(explicit.cell_count(), 0);
    assert_eq!(explicit.cells().next(), None);
}

#[test]
fn compiler_reads_each_distinct_cell_once_without_changing_workbook_or_view() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook.set_user_input(0, 1, 1, "42").unwrap();
    let before = workbook.to_bytes();
    let placement = CellPlacement {
        coordinate: CellCoord { row: 1, col: 1 },
        bounds: rect(),
    };
    let prepared = WorksheetCompiler::new()
        .prepare_cells(
            &workbook,
            0,
            rect(),
            [placement, placement],
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap();
    assert_eq!(prepared.extended_style_reads, 1);
    assert_eq!(prepared.cell_count(), 2);
    assert_eq!(workbook.to_bytes(), before);
    assert!(
        prepared.scene.passes[4]
            .iter()
            .any(|p| matches!(p, Primitive::Text {text, ..} if text == "42"))
    );
}

#[test]
fn explicit_black_font_color_is_preserved_on_dark_theme() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook.set_user_input(0, 1, 1, "explicit black").unwrap();
    workbook.set_user_input(0, 1, 2, "default color").unwrap();
    workbook
        .update_range_style(
            &Area {
                sheet: 0,
                row: 1,
                column: 1,
                width: 1,
                height: 1,
            },
            "font.color",
            "#000000",
        )
        .unwrap();

    let theme = CanvasTheme::dark();
    let prepared = WorksheetCompiler::new()
        .prepare_cells(
            &workbook,
            0,
            rect(),
            [
                CellPlacement {
                    coordinate: CellCoord { row: 1, col: 1 },
                    bounds: rect(),
                },
                CellPlacement {
                    coordinate: CellCoord { row: 1, col: 2 },
                    bounds: rect(),
                },
            ],
            &Metrics,
            &theme,
        )
        .unwrap();
    let text_colors: Vec<_> = prepared.scene.passes[4]
        .iter()
        .filter_map(|primitive| match primitive {
            Primitive::Text { text, color, .. } => Some((text.as_str(), color.as_ref())),
            _ => None,
        })
        .collect();

    assert!(text_colors.contains(&("explicit black", "#000000")));
    assert!(text_colors.contains(&("default color", theme.default_text_color.as_ref())));
}

#[test]
fn invalid_sheet_is_rejected_even_without_cells() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    assert!(
        WorksheetCompiler::new()
            .prepare_cells(
                &workbook,
                u32::MAX,
                rect(),
                [],
                &Metrics,
                &CanvasTheme::light()
            )
            .is_err()
    );
    assert!(
        WorksheetCompiler::new()
            .prepare(
                &workbook,
                u32::MAX,
                WorksheetViewport {
                    first_cell: CellCoord { row: 1, col: 1 },
                    width: 0,
                    height: 0
                },
                &Metrics,
                &CanvasTheme::light()
            )
            .is_err()
    );
}

#[test]
fn invalid_late_coordinate_returns_an_error_instead_of_partial_scene() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    let cells = [
        CellPlacement {
            coordinate: CellCoord { row: 1, col: 1 },
            bounds: rect(),
        },
        CellPlacement {
            coordinate: CellCoord { row: 0, col: 1 },
            bounds: rect(),
        },
    ];
    let error = WorksheetCompiler::new()
        .prepare_cells(&workbook, 0, rect(), cells, &Metrics, &CanvasTheme::light())
        .unwrap_err();
    assert_eq!(error.cell.row, 0);
}

#[test]
fn zero_width_placement_at_max_coordinate_is_ignored_safely() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    let prepared = WorksheetCompiler::new()
        .prepare_cells(
            &workbook,
            0,
            rect(),
            [CellPlacement {
                coordinate: CellCoord { row: 1, col: 1 },
                bounds: PixelRect {
                    top_left: Point { x: i32::MAX, y: 0 },
                    width: 0,
                    height: 30,
                },
            }],
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap();

    assert_eq!(prepared.cell_count(), 1);
}

#[test]
fn one_pixel_placement_at_max_coordinate_is_ignored_safely() {
    let workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    for bounds in [
        PixelRect {
            top_left: Point {
                x: i32::MAX - 1,
                y: 0,
            },
            width: 1,
            height: 30,
        },
        PixelRect {
            top_left: Point {
                x: 0,
                y: i32::MAX - 1,
            },
            width: 100,
            height: 1,
        },
    ] {
        WorksheetCompiler::new()
            .prepare_cells(
                &workbook,
                0,
                rect(),
                [CellPlacement {
                    coordinate: CellCoord { row: 1, col: 1 },
                    bounds,
                }],
                &Metrics,
                &CanvasTheme::light(),
            )
            .unwrap();
    }
}

#[test]
fn explicit_sheet_does_not_change_selection() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook.new_sheet().unwrap();
    workbook.set_user_input(0, 1, 1, "first sheet").unwrap();
    workbook.set_user_input(1, 1, 1, "second sheet").unwrap();
    let before = workbook.to_bytes();
    let result = WorksheetCompiler::new()
        .prepare(
            &workbook,
            0,
            WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 100,
                height: 30,
            },
            &Metrics,
            &CanvasTheme::light(),
        )
        .unwrap();
    assert!(
        result.scene.passes[4]
            .iter()
            .any(|p| matches!(p, Primitive::Text {text, ..} if text == "first sheet"))
    );
    assert_eq!(workbook.to_bytes(), before);
}

/// The compiler surfaces the engine's worksheet links: a static link and a
/// formula link are both reported, the static one wins at an address that has
/// both, and `dynamic` distinguishes them.
#[test]
fn prepared_worksheet_reports_static_and_formula_links_with_static_precedence() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook
        .set_cell_link(0, 1, 1, external("https://static.example"), None)
        .expect("static link write");
    workbook
        .set_user_input(0, 2, 1, "=HYPERLINK(\"https://formula.example\",\"x\")")
        .expect("formula write");
    workbook
        .set_user_input(0, 3, 1, "=HYPERLINK(\"https://dynamic.example\",\"y\")")
        .expect("formula write");
    workbook.evaluate();
    workbook
        .set_cell_link(0, 3, 1, external("https://static-wins.example"), None)
        .expect("static link write");
    workbook.evaluate();

    let links = prepared(&workbook).links;
    let at = |row, column| {
        links
            .iter()
            .find(|link| link.row == row && link.column == column)
            .unwrap_or_else(|| panic!("no link at ({row}, {column})"))
    };

    let static_link = at(1, 1);
    assert!(!static_link.dynamic);
    assert_eq!(static_link.link, external("https://static.example"));

    let dynamic_link = at(2, 1);
    assert!(dynamic_link.dynamic);
    assert_eq!(
        dynamic_link.link,
        external("https://formula.example"),
        "a formula link keeps its dynamic flag"
    );

    let both = at(3, 1);
    assert!(!both.dynamic, "a worksheet link takes precedence");
    assert_eq!(both.link, external("https://static-wins.example"));
}

/// An internal formula target (`#Sheet1!A5`) keeps its kind — the engine
/// strips the `#` — and stays dynamic.
#[test]
fn prepared_worksheet_preserves_internal_formula_link_kind() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook
        .set_user_input(0, 2, 1, "=HYPERLINK(\"#Sheet1!A5\",\"go\")")
        .expect("formula write");
    workbook.evaluate();

    let links = prepared(&workbook).links;
    let link = links
        .iter()
        .find(|link| (link.row, link.column) == (2, 1))
        .expect("the formula link is reported");
    assert!(link.dynamic);
    assert_eq!(
        link.link,
        ic::Link::Internal {
            location: "Sheet1!A5".to_string(),
            tooltip: None,
        }
    );
}

/// A precedent edit changes the formula link's target while the displayed
/// label stays put, and an erroring formula drops the link entirely — the
/// engine rebuilds `Model::links` on every evaluation.
#[test]
fn precedent_edit_moves_the_link_target_and_an_error_drops_it() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    workbook
        .set_user_input(0, 1, 1, "https://one.example")
        .expect("precedent write");
    workbook
        .set_user_input(0, 3, 1, "=HYPERLINK(A1,\"label\")")
        .expect("formula write");
    workbook.evaluate();

    let target_at = |workbook: &UserModel<'_>, row, column| {
        prepared(workbook)
            .links
            .iter()
            .find(|link| (link.row, link.column) == (row, column))
            .map(|link| link.link.clone())
    };
    let dynamic = target_at(&workbook, 3, 1).expect("the formula link is reported");
    assert_eq!(dynamic, external("https://one.example"));

    // Move the precedent: the label is unchanged, so only the target moves.
    workbook
        .set_user_input(0, 1, 1, "https://two.example")
        .expect("precedent write");
    workbook.evaluate();
    assert_eq!(
        target_at(&workbook, 3, 1),
        Some(external("https://two.example"))
    );
    assert_eq!(
        workbook
            .get_formatted_cell_value(0, 3, 1)
            .expect("the label is readable"),
        "label"
    );

    workbook
        .set_user_input(0, 1, 1, "=1/0")
        .expect("precedent write");
    workbook.evaluate();
    assert!(
        target_at(&workbook, 3, 1).is_none(),
        "an erroring HYPERLINK attaches no link at the formula cell"
    );
}

/// The compiler reports the engine's merged list: a real `merge_cells` shows
/// up, and unmerging clears it.
#[test]
fn prepared_worksheet_reports_the_engine_merged_list() {
    let mut workbook = UserModel::new_empty("workbook", "en", "UTC", "en").unwrap();
    let area = Area {
        sheet: 0,
        row: 2,
        column: 2,
        width: 3,
        height: 2,
    };
    workbook.merge_cells(&area).expect("merge must succeed");

    let merges = prepared(&workbook).merges;
    assert_eq!(merges.len(), 1);
    assert_eq!(
        (
            merges[0].row,
            merges[0].column,
            merges[0].width,
            merges[0].height
        ),
        (2, 2, 3, 2)
    );

    workbook.unmerge_cells(&area).expect("unmerge must succeed");
    assert!(prepared(&workbook).merges.is_empty());
}

/// Both link kinds look like links: hyperlink color and an underline.
///
/// A static link carries that look in the cell style. A formula link does not,
/// so the text pass must supply it.
#[test]
fn linked_cells_paint_the_link_color_and_an_underline() {
    let mut workbook = UserModel::new_empty("links", "en", "UTC", "en").unwrap();
    workbook.set_user_input(0, 1, 1, "static").unwrap();
    workbook
        .set_user_input(0, 2, 1, "=HYPERLINK(\"https://example.com\",\"formula\")")
        .unwrap();
    workbook.set_user_input(0, 3, 1, "plain").unwrap();
    workbook
        .set_cell_link(0, 1, 1, external("https://example.com"), None)
        .expect("link write");
    workbook.evaluate();

    let theme = CanvasTheme::light();
    let paint = |row: i32| -> (String, usize, usize) {
        let prepared = WorksheetCompiler::new()
            .prepare_cells(
                &workbook,
                0,
                rect(),
                [CellPlacement {
                    coordinate: CellCoord { row, col: 1 },
                    bounds: rect(),
                }],
                &Metrics,
                &theme,
            )
            .expect("cell preparation");
        let mut color = String::new();
        let mut texts = 0;
        let mut underlines = 0;
        for op in prepared.scene.passes[4].iter() {
            match op {
                Primitive::Text {
                    color: op_color, ..
                } => {
                    color = op_color.to_string();
                    texts += 1;
                }
                Primitive::TextLine { .. } => underlines += 1,
                _ => {}
            }
        }
        (color, texts, underlines)
    };

    let (static_color, static_texts, static_underlines) = paint(1);
    let (formula_color, formula_texts, formula_underlines) = paint(2);
    let (plain_color, plain_texts, plain_underlines) = paint(3);
    assert_eq!((static_texts, formula_texts, plain_texts), (1, 1, 1));
    assert_eq!(
        (static_underlines, formula_underlines, plain_underlines),
        (1, 1, 0)
    );
    assert_eq!(static_color, formula_color);
    assert_ne!(static_color, plain_color);
    assert_eq!(plain_color, theme.default_text_color.as_ref());
}
