//! Auto-fit measurement over an explicit used-range span.
//!
//! Engine-side port of the legacy `model/autofit.rs`: `fit_width` scans a
//! column across a `[first_row, last_row]` span for the widest formatted
//! value; `fit_height` scans a row across `[first_col, last_col]` for the
//! tallest wrapped text block. The caller supplies the span (the used-range
//! query lives a layer up). No mutation: the returned extent is what the
//! host would apply through a resize.
//!
//! Font CSS is built with the same rule as [`crate::compiler`]'s text pass,
//! so measured widths match painted widths.

use crate::compiler::CfDecorationPaint;
use iron_canvas_core::{
    PixelRect, Point,
    painter::{TextAlign, TextMetrics},
    text_layout::{
        CELL_PADDING, LINE_HEIGHT_FACTOR, TEXT_V_INSET_PX, TextLayoutOptions, TextLine,
        TextVerticalAlign, layout_text_into,
    },
};
use ironcalc_base::{UserModel, types::Style};

/// Slack added to the measured extent so glyphs do not touch the cell border.
pub const FIT_PADDING: f64 = 2.0 * CELL_PADDING;

/// Floor so an auto-fit never collapses a row or column to a sliver.
const MIN_EXTENT: f64 = 5.0;

/// Vertical extent used when only the line count matters. The wrap width
/// decides that, so this value cannot change the result.
const UNMEASURED_HEIGHT: i32 = 1_000_000;

/// Upper bound on cells scanned per auto-fit.
pub const FIT_SCAN_CAP: i32 = 20_000;

/// Why an auto-fit could not produce a measurement. `Ok(None)` is an empty
/// span; these are failed reads, which a host must not treat as "no content".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoFitError {
    /// No committed frame, so the sheet is unknown.
    Sheet,
    /// A column width could not be read as a valid slot extent.
    ColumnExtent,
    /// A row height could not be read as a valid slot extent.
    RowExtent,
    /// A formatted cell value could not be read.
    CellValue,
    /// A non-empty cell's style could not be read.
    CellStyle,
}

impl std::fmt::Display for AutoFitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Sheet => "no sheet is committed to measure",
            Self::ColumnExtent => "a column width could not be read as a valid extent",
            Self::RowExtent => "a row height could not be read as a valid extent",
            Self::CellValue => "a formatted cell value could not be read",
            Self::CellStyle => "a cell style could not be read",
        };
        f.write_str(message)
    }
}

impl std::error::Error for AutoFitError {}

/// Bound a used-range span to at most `FIT_SCAN_CAP` entries, anchored at
/// `first`. An inverted span is returned unchanged.
fn capped_range(first: i32, last: i32) -> (i32, i32) {
    if first > last {
        return (first, last);
    }
    let cap_last = first.saturating_add(FIT_SCAN_CAP - 1);
    (first, last.min(cap_last))
}

/// `ctx.font` string for a cell style, matching the text pass.
fn font_css(style: &Style) -> String {
    let font = &style.font;
    let size = f64::from(font.sz);
    let family = font.name.trim();
    let family = if family.is_empty() { "Calibri" } else { family };
    let family = family.replace(['"', '\\', '\n', '\r'], "");
    format!(
        "{}{}{size}px \"{family}\"",
        if font.b { "bold " } else { "" },
        if font.i { "italic " } else { "" }
    )
}

/// Widest formatted value across `col` over `[first_row, last_row]`, plus
/// padding. `Ok(None)` when every scanned cell is empty.
pub fn fit_width(
    workbook: &UserModel<'_>,
    metrics: &dyn TextMetrics,
    sheet: u32,
    col: i32,
    first_row: i32,
    last_row: i32,
) -> Result<Option<f64>, AutoFitError> {
    let (first, last) = capped_range(first_row, last_row);
    let mut max = 0.0_f64;
    for row in first..=last {
        let text = workbook
            .get_formatted_cell_value(sheet, row, col)
            .map_err(|_| AutoFitError::CellValue)?;
        if text.is_empty() {
            continue;
        }
        let extended = workbook
            .get_extended_cell_style(sheet, row, col)
            .map_err(|_| AutoFitError::CellStyle)?;
        let reserved_left = if extended.icon.is_some() || extended.rating.is_some() {
            let height = workbook
                .get_row_height(sheet, row)
                .map_err(|_| AutoFitError::RowExtent)?;
            if !height.is_finite() || height < 0.0 || height.round() > f64::from(i32::MAX) {
                return Err(AutoFitError::RowExtent);
            }
            let rect = PixelRect {
                top_left: Point::default(),
                // Do not let the current column width clip the needed reservation.
                width: i32::MAX,
                height: height.round() as i32,
            };
            CfDecorationPaint::resolve(&extended, workbook).reserved_left(rect)
        } else {
            0
        };
        let width = metrics.measure_text_width(&text, &font_css(&extended.style))
            + f64::from(reserved_left);
        if width > max {
            max = width;
        }
    }
    Ok((max > 0.0).then(|| (max + FIT_PADDING).max(MIN_EXTENT)))
}

/// Tallest wrapped text block across `row` over `[first_col, last_col]`, plus
/// padding. `Ok(None)` when every scanned cell is empty.
pub fn fit_height(
    workbook: &UserModel<'_>,
    metrics: &dyn TextMetrics,
    sheet: u32,
    row: i32,
    first_col: i32,
    last_col: i32,
) -> Result<Option<f64>, AutoFitError> {
    let (first, last) = capped_range(first_col, last_col);
    let mut lines: Vec<TextLine> = Vec::new();
    let mut scratch = String::new();
    let mut max_height = 0.0_f64;
    for col in first..=last {
        let text = workbook
            .get_formatted_cell_value(sheet, row, col)
            .map_err(|_| AutoFitError::CellValue)?;
        if text.is_empty() {
            continue;
        }
        let style = workbook
            .get_extended_cell_style(sheet, row, col)
            .map_err(|_| AutoFitError::CellStyle)?
            .style;
        let raw = workbook
            .get_column_width(sheet, col)
            .map_err(|_| AutoFitError::ColumnExtent)?;
        if !raw.is_finite() || raw < 0.0 {
            return Err(AutoFitError::ColumnExtent);
        }
        let width = (raw.round() as i32).max(1);
        let size = f64::from(style.font.sz);
        let bounds = PixelRect {
            top_left: iron_canvas_core::Point { x: 0, y: 0 },
            width,
            height: UNMEASURED_HEIGHT,
        };
        lines.clear();
        layout_text_into(
            metrics,
            &font_css(&style),
            &text,
            TextLayoutOptions {
                bounds,
                clip: bounds,
                font_size_px: size,
                horizontal: TextAlign::Start,
                vertical: TextVerticalAlign::Top,
                wrap: style.alignment.as_ref().is_some_and(|a| a.wrap_text),
                horizontal_padding: CELL_PADDING,
                vertical_inset: TEXT_V_INSET_PX,
                line_height_factor: LINE_HEIGHT_FACTOR,
                approx_char_width: size,
            },
            &mut lines,
            &mut scratch,
        );
        let height = line_block_height(lines.len(), size, size * LINE_HEIGHT_FACTOR);
        if height > max_height {
            max_height = height;
        }
    }
    Ok((max_height > 0.0).then(|| max_height.max(MIN_EXTENT)))
}

/// Fitted pixel height of one cell's text block.
fn line_block_height(line_count: usize, size_px: f64, line_height: f64) -> f64 {
    if line_count == 0 {
        return 0.0;
    }
    let gaps = (line_count - 1) as f64;
    size_px + gaps * line_height + FIT_PADDING
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironcalc_base::cf_types::{CfRuleInput, Cfvo, Icon, IconThreshold};
    use ironcalc_base::types::Color;

    struct FixedWidth;
    impl TextMetrics for FixedWidth {
        fn measure_text_width(&self, text: &str, _font_css: &str) -> f64 {
            text.chars().count() as f64 * 10.0
        }
    }

    #[test]
    fn single_line_height_is_font_plus_padding() {
        assert_eq!(line_block_height(1, 14.0, 28.0), 14.0 + FIT_PADDING);
    }

    #[test]
    fn fit_width_scans_formatted_values_and_pads() {
        let mut workbook = UserModel::new_empty("fit", "en", "UTC", "en").unwrap();
        workbook.set_user_input(0, 1, 1, "abc").unwrap();
        workbook.set_user_input(0, 3, 1, "abcdef").unwrap();
        let width = fit_width(&workbook, &FixedWidth, 0, 1, 1, 3).unwrap();
        assert_eq!(width, Some(60.0 + FIT_PADDING));
        // An empty span has nothing to fit to.
        assert_eq!(fit_width(&workbook, &FixedWidth, 0, 5, 1, 3).unwrap(), None);
    }

    #[test]
    fn fit_width_includes_visible_conditional_formatting_icon_space() {
        let mut workbook = UserModel::new_empty("fit", "en", "UTC", "en").unwrap();
        workbook.set_user_input(0, 1, 1, "1234").unwrap();
        workbook.set_rows_height(0, 1, 1, 26.0).unwrap();
        workbook
            .add_conditional_formatting(
                0,
                "A1:A1",
                CfRuleInput::IconSet {
                    thresholds: vec![
                        IconThreshold {
                            icon: Icon::ArrowDown,
                            cfvo: Cfvo::Min,
                            color: Color::Rgb("#e43400".into()),
                            is_strict: false,
                        },
                        IconThreshold {
                            icon: Icon::ArrowRight,
                            cfvo: Cfvo::Percent(33.0),
                            color: Color::Rgb("#ffeb84".into()),
                            is_strict: false,
                        },
                        IconThreshold {
                            icon: Icon::ArrowUp,
                            cfvo: Cfvo::Percent(67.0),
                            color: Color::Rgb("#84cb1f".into()),
                            is_strict: false,
                        },
                    ],
                    show_value: true,
                },
            )
            .unwrap();

        let extended = workbook.get_extended_cell_style(0, 1, 1).unwrap();
        assert!(extended.icon.is_some(), "the test cell must have a CF icon");
        let height = workbook.get_row_height(0, 1).unwrap().round() as i32;
        let rect = PixelRect {
            top_left: Point::default(),
            width: i32::MAX,
            height,
        };
        let icon_space = CfDecorationPaint::resolve(&extended, &workbook).reserved_left(rect);
        assert!(icon_space > 0, "the test icon must reserve text space");
        let value = workbook.get_formatted_cell_value(0, 1, 1).unwrap();
        let expected = FixedWidth.measure_text_width(&value, &font_css(&extended.style))
            + f64::from(icon_space)
            + FIT_PADDING;

        assert_eq!(
            fit_width(&workbook, &FixedWidth, 0, 1, 1, 1).unwrap(),
            Some(expected)
        );
    }

    #[test]
    fn fit_height_reads_one_wrapped_block() {
        let mut workbook = UserModel::new_empty("fit", "en", "UTC", "en").unwrap();
        workbook.set_user_input(0, 1, 1, "one line").unwrap();
        let height = fit_height(&workbook, &FixedWidth, 0, 1, 1, 1).unwrap();
        // A single line is font height + padding, floored at MIN_EXTENT.
        assert!(height.is_some_and(|h| h >= MIN_EXTENT));
    }
}
