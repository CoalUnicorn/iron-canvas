//! XLSX loading, worksheet-range preparation, and file export.
//!
//! Worksheet scenes are still compiled by `iron-canvas-ironcalc`; this module
//! sizes the requested range and sends that prepared scene to the SVG or PDF
//! backend.

use std::{
    error::Error,
    fmt, io,
    path::{Path, PathBuf},
};

use iron_canvas_core::{CanvasTheme, CellCoord};
#[cfg(any(feature = "svg", feature = "pdf"))]
use iron_canvas_core::{painter::TextMetrics, scene::RenderBackend};
use iron_canvas_ironcalc::WorksheetCompileError;
#[cfg(any(feature = "svg", feature = "pdf"))]
use iron_canvas_ironcalc::{PreparedWorksheet, WorksheetCompiler, WorksheetViewport};
use ironcalc::import::load_from_xlsx;
use ironcalc_base::UserModel;
#[cfg(any(feature = "svg", feature = "pdf"))]
use std::fs;

const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;
#[cfg(any(feature = "svg", feature = "pdf"))]
const HEADER_HEIGHT: i32 = 24;
const MAX_VISIBLE_CELLS: i64 = 50_000;
#[cfg(any(feature = "svg", feature = "pdf"))]
const MAX_PAGE_DIMENSION: i32 = 20_000;
#[cfg(any(feature = "svg", feature = "pdf"))]
const MAX_PAGE_PIXELS: i64 = 100_000_000;

/// An inclusive rectangular worksheet selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRange {
    first: CellCoord,
    last: CellCoord,
}

impl CellRange {
    /// Create a checked, inclusive cell range.
    pub fn new(first: CellCoord, last: CellCoord) -> Result<Self, String> {
        if !(1..=LAST_ROW).contains(&first.row)
            || !(1..=LAST_ROW).contains(&last.row)
            || !(1..=LAST_COLUMN).contains(&first.col)
            || !(1..=LAST_COLUMN).contains(&last.col)
        {
            return Err("cell range is outside worksheet bounds".to_owned());
        }
        if first.row > last.row || first.col > last.col {
            return Err("range start must be above and left of its end".to_owned());
        }
        let cells = (i64::from(last.row) - i64::from(first.row) + 1)
            * (i64::from(last.col) - i64::from(first.col) + 1);
        if cells > MAX_VISIBLE_CELLS {
            return Err(format!(
                "range contains more than {MAX_VISIBLE_CELLS} cells"
            ));
        }
        Ok(Self { first, last })
    }

    pub const fn first(self) -> CellCoord {
        self.first
    }

    pub const fn last(self) -> CellCoord {
        self.last
    }
}

impl std::str::FromStr for CellRange {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        parse_range(input)
    }
}

/// Errors raised while loading, compiling, or writing an XLSX export.
#[derive(Debug)]
pub enum WorkbookExportError {
    UnsupportedOutput(PathBuf),
    FeatureDisabled(&'static str),
    OpenWorkbook { path: PathBuf, message: String },
    SheetSelection(String),
    Viewport(String),
    Compile(WorksheetCompileError),
    Backend(&'static str),
    Write { path: PathBuf, source: io::Error },
}

impl fmt::Display for WorkbookExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOutput(path) => {
                write!(
                    f,
                    "output path must end in .svg or .pdf: {}",
                    path.display()
                )
            }
            Self::FeatureDisabled(format) => {
                write!(f, "the {format} export feature is not enabled")
            }
            Self::OpenWorkbook { path, message } => {
                write!(f, "could not open {}: {message}", path.display())
            }
            Self::SheetSelection(message) | Self::Viewport(message) => f.write_str(message),
            Self::Compile(error) => write!(f, "could not compile worksheet: {error}"),
            Self::Backend(message) => f.write_str(message),
            Self::Write { path, source } => {
                write!(f, "could not write {}: {source}", path.display())
            }
        }
    }
}

impl Error for WorkbookExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Load an `.xlsx` workbook and export the selected worksheet range.
///
/// `sheet` accepts a case-insensitive worksheet name or a zero-based index.
/// When omitted, the first worksheet is exported. The output extension chooses
/// SVG or PDF; the workbook's frozen rows and columns remain visible.
///
/// Returns the zero-based index of the worksheet exported.
pub fn export_xlsx(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    sheet: Option<&str>,
    range: CellRange,
    theme: &CanvasTheme,
) -> Result<u32, WorkbookExportError> {
    let input_path = input_path.as_ref();
    let output_path = output_path.as_ref();
    let format = output_format(output_path)?;
    let mut workbook = load_from_xlsx(&input_path.to_string_lossy(), "en", "UTC", "en")
        .map(UserModel::from_model)
        .map_err(|error| WorkbookExportError::OpenWorkbook {
            path: input_path.to_owned(),
            message: error.to_string(),
        })?;
    workbook.evaluate();
    let sheet = resolve_sheet(&workbook, sheet).map_err(WorkbookExportError::SheetSelection)?;

    match format {
        OutputFormat::Svg => write_svg(&workbook, sheet, range, theme, output_path)?,
        OutputFormat::Pdf => write_pdf(&workbook, sheet, range, theme, output_path)?,
    }
    Ok(sheet)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Svg,
    Pdf,
}

fn output_format(path: &Path) -> Result<OutputFormat, WorkbookExportError> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("svg") => Ok(OutputFormat::Svg),
        Some("pdf") => Ok(OutputFormat::Pdf),
        _ => Err(WorkbookExportError::UnsupportedOutput(path.to_owned())),
    }
}

#[cfg(feature = "svg")]
fn write_svg(
    workbook: &UserModel<'_>,
    sheet: u32,
    range: CellRange,
    theme: &CanvasTheme,
    output_path: &Path,
) -> Result<(), WorkbookExportError> {
    let mut backend = crate::SvgSceneBackend::new();
    let prepared = prepare(workbook, sheet, range, backend.metrics(), theme)?;
    let staged = match backend.stage_frame(&prepared.scene) {
        Ok(staged) => staged,
        Err(never) => match never {},
    };
    if let Err(never) = backend.commit_frame(staged) {
        match never {}
    }
    let document = backend.document().ok_or(WorkbookExportError::Backend(
        "SVG backend did not produce a document",
    ))?;
    write_output(output_path, document.as_bytes())
}

#[cfg(not(feature = "svg"))]
fn write_svg(
    _workbook: &UserModel<'_>,
    _sheet: u32,
    _range: CellRange,
    _theme: &CanvasTheme,
    _output_path: &Path,
) -> Result<(), WorkbookExportError> {
    Err(WorkbookExportError::FeatureDisabled("SVG"))
}

#[cfg(feature = "pdf")]
fn write_pdf(
    workbook: &UserModel<'_>,
    sheet: u32,
    range: CellRange,
    theme: &CanvasTheme,
    output_path: &Path,
) -> Result<(), WorkbookExportError> {
    let mut backend = crate::PdfSceneBackend::new();
    let prepared = prepare(workbook, sheet, range, backend.metrics(), theme)?;
    let staged = match backend.stage_frame(&prepared.scene) {
        Ok(staged) => staged,
        Err(never) => match never {},
    };
    if let Err(never) = backend.commit_frame(staged) {
        match never {}
    }
    let document = backend.document().ok_or(WorkbookExportError::Backend(
        "PDF backend did not produce a document",
    ))?;
    write_output(output_path, document)
}

#[cfg(not(feature = "pdf"))]
fn write_pdf(
    _workbook: &UserModel<'_>,
    _sheet: u32,
    _range: CellRange,
    _theme: &CanvasTheme,
    _output_path: &Path,
) -> Result<(), WorkbookExportError> {
    Err(WorkbookExportError::FeatureDisabled("PDF"))
}

#[cfg(any(feature = "svg", feature = "pdf"))]
fn write_output(output_path: &Path, document: &[u8]) -> Result<(), WorkbookExportError> {
    fs::write(output_path, document).map_err(|source| WorkbookExportError::Write {
        path: output_path.to_owned(),
        source,
    })
}

#[cfg(any(feature = "svg", feature = "pdf"))]
fn prepare(
    workbook: &UserModel<'_>,
    sheet: u32,
    range: CellRange,
    metrics: &dyn TextMetrics,
    theme: &CanvasTheme,
) -> Result<PreparedWorksheet, WorkbookExportError> {
    let viewport = viewport_for_range(workbook, sheet, range, metrics)
        .map_err(WorkbookExportError::Viewport)?;
    WorksheetCompiler::new()
        .prepare(workbook, sheet, viewport, metrics, theme)
        .map_err(WorkbookExportError::Compile)
}

#[cfg(any(feature = "svg", feature = "pdf"))]
fn viewport_for_range(
    workbook: &UserModel<'_>,
    sheet: u32,
    range: CellRange,
    metrics: &dyn TextMetrics,
) -> Result<WorksheetViewport, String> {
    let frozen_rows = workbook
        .get_frozen_rows_count(sheet)
        .map_err(|error| format!("could not read frozen rows: {error}"))?;
    let frozen_columns = workbook
        .get_frozen_columns_count(sheet)
        .map_err(|error| format!("could not read frozen columns: {error}"))?;
    if !(0..=LAST_ROW).contains(&frozen_rows) || !(0..=LAST_COLUMN).contains(&frozen_columns) {
        return Err("workbook has invalid frozen pane bounds".to_owned());
    }

    let visible_rows = visible_axis_count(range.first.row, range.last.row, frozen_rows);
    let visible_columns = visible_axis_count(range.first.col, range.last.col, frozen_columns);
    if visible_rows * visible_columns > MAX_VISIBLE_CELLS {
        return Err(format!(
            "range expands to more than {MAX_VISIBLE_CELLS} visible cells (including frozen panes)"
        ));
    }

    let rows = summed_axis_extent(
        workbook,
        sheet,
        true,
        range.first.row,
        range.last.row,
        frozen_rows,
    )?;
    let columns = summed_axis_extent(
        workbook,
        sheet,
        false,
        range.first.col,
        range.last.col,
        frozen_columns,
    )?;
    let largest_row_label = range.last.row.max(frozen_rows).to_string();
    let measured_header = metrics.measure_text_width(&largest_row_label, "12px sans-serif");
    if !measured_header.is_finite() || measured_header < 0.0 {
        return Err("text metrics returned an invalid row-header width".to_owned());
    }
    let measured_header = measured_header.ceil();
    if measured_header > f64::from(i32::MAX - 16) {
        return Err("row-header width is too large".to_owned());
    }
    let header_width = (measured_header as i32 + 16).max(48);
    let width = header_width
        .checked_add(columns)
        .ok_or_else(|| "page width overflows".to_owned())?;
    let height = HEADER_HEIGHT
        .checked_add(rows)
        .ok_or_else(|| "page height overflows".to_owned())?;
    if width <= 0
        || height <= 0
        || width > MAX_PAGE_DIMENSION
        || height > MAX_PAGE_DIMENSION
        || i64::from(width) * i64::from(height) > MAX_PAGE_PIXELS
    {
        return Err(format!(
            "requested page is {width}x{height}px; maximum is {MAX_PAGE_DIMENSION}x{MAX_PAGE_DIMENSION}px"
        ));
    }

    Ok(WorksheetViewport {
        first_cell: range.first,
        width,
        height,
    })
}

#[cfg(any(feature = "svg", feature = "pdf"))]
fn visible_axis_count(first: i32, last: i32, frozen: i32) -> i64 {
    let frozen_count = i64::from(frozen);
    let scrolling_first = first.max(frozen.saturating_add(1));
    let scrolling_count = if scrolling_first <= last {
        i64::from(last) - i64::from(scrolling_first) + 1
    } else {
        0
    };
    frozen_count + scrolling_count
}

#[cfg(any(feature = "svg", feature = "pdf"))]
fn summed_axis_extent(
    workbook: &UserModel<'_>,
    sheet: u32,
    rows: bool,
    first: i32,
    last: i32,
    frozen: i32,
) -> Result<i32, String> {
    let mut total = 0_i32;
    if frozen > 0 {
        total = sum_extent(workbook, sheet, rows, 1, frozen)?;
    }
    let scrolling_first = first.max(frozen.saturating_add(1));
    if scrolling_first <= last {
        total = total
            .checked_add(sum_extent(workbook, sheet, rows, scrolling_first, last)?)
            .ok_or_else(|| "worksheet dimensions overflow".to_owned())?;
    }
    Ok(total)
}

#[cfg(any(feature = "svg", feature = "pdf"))]
fn sum_extent(
    workbook: &UserModel<'_>,
    sheet: u32,
    rows: bool,
    first: i32,
    last: i32,
) -> Result<i32, String> {
    let mut total = 0_i32;
    for index in first..=last {
        let extent = if rows {
            workbook.get_row_height(sheet, index)
        } else {
            workbook.get_column_width(sheet, index)
        }
        .map_err(|error| {
            let axis = if rows { "row height" } else { "column width" };
            format!("could not read {axis} for index {index}: {error}")
        })?;
        if !extent.is_finite() || extent < 0.0 || extent.round() > f64::from(i32::MAX) {
            return Err(format!("invalid worksheet extent at index {index}"));
        }
        total = total
            .checked_add(extent.round() as i32)
            .ok_or_else(|| "worksheet dimensions overflow".to_owned())?;
    }
    Ok(total)
}

fn resolve_sheet(workbook: &UserModel<'_>, selector: Option<&str>) -> Result<u32, String> {
    let sheets = workbook.get_worksheets_properties();
    if sheets.is_empty() {
        return Err("workbook has no worksheets".to_owned());
    }
    let Some(selector) = selector else {
        return Ok(0);
    };

    if let Ok(index) = selector.parse::<usize>() {
        return if index < sheets.len() {
            Ok(index as u32)
        } else {
            Err(format!(
                "worksheet index {index} is out of range; workbook has {} sheets",
                sheets.len()
            ))
        };
    }
    sheets
        .iter()
        .position(|properties| properties.name.eq_ignore_ascii_case(selector))
        .map(|index| index as u32)
        .ok_or_else(|| {
            let available = sheets
                .iter()
                .map(|properties| properties.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("worksheet {selector:?} was not found; available sheets: {available}")
        })
}

fn parse_range(input: &str) -> Result<CellRange, String> {
    let mut endpoints = input.split(':');
    let first = endpoints
        .next()
        .ok_or_else(|| format!("invalid range: {input}"))?;
    let last = endpoints.next().unwrap_or(first);
    if endpoints.next().is_some() {
        return Err(format!("invalid range {input:?}; expected A1 or A1:B5"));
    }
    CellRange::new(parse_cell(first)?, parse_cell(last)?)
}

fn parse_cell(input: &str) -> Result<CellCoord, String> {
    let first_digit = input
        .find(|character: char| character.is_ascii_digit())
        .ok_or_else(|| format!("invalid cell reference {input:?}; expected a value like B12"))?;
    let (column_label, row_label) = input.split_at(first_digit);
    if column_label.is_empty()
        || !column_label.bytes().all(|byte| byte.is_ascii_alphabetic())
        || row_label.is_empty()
        || !row_label.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "invalid cell reference {input:?}; expected a value like B12"
        ));
    }

    let mut col = 0_i32;
    for byte in column_label.bytes() {
        let digit = i32::from(byte.to_ascii_uppercase() - b'A' + 1);
        col = col
            .checked_mul(26)
            .and_then(|value| value.checked_add(digit))
            .ok_or_else(|| format!("column is out of bounds in cell reference {input:?}"))?;
    }
    let row = row_label
        .parse::<i32>()
        .map_err(|_| format!("row is out of bounds in cell reference {input:?}"))?;
    if !(1..=LAST_ROW).contains(&row) || !(1..=LAST_COLUMN).contains(&col) {
        return Err(format!(
            "cell reference {input:?} is outside worksheet bounds"
        ));
    }
    Ok(CellCoord { row, col })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironcalc::export::save_to_xlsx;
    use ironcalc_base::UserModel;

    #[test]
    fn parses_single_cells_and_ranges_case_insensitively() {
        let range = "b3:J28".parse::<CellRange>().expect("valid range");
        assert_eq!(range.first(), CellCoord { row: 3, col: 2 });
        assert_eq!(range.last(), CellCoord { row: 28, col: 10 });

        let last_cell = "XFD1048576".parse::<CellRange>().expect("valid cell");
        assert_eq!(
            last_cell.first(),
            CellCoord {
                row: LAST_ROW,
                col: LAST_COLUMN
            }
        );
        assert_eq!(last_cell.last(), last_cell.first());
    }

    #[test]
    fn rejects_reversed_and_out_of_bounds_ranges() {
        assert!("B4:A1".parse::<CellRange>().is_err());
        assert!("XFE1".parse::<CellRange>().is_err());
        assert!("A0".parse::<CellRange>().is_err());
        assert!("A1:B2:C3".parse::<CellRange>().is_err());
    }

    #[test]
    fn infers_format_from_output_extension() {
        assert_eq!(
            output_format(Path::new("out.SVG")).unwrap(),
            OutputFormat::Svg
        );
        assert_eq!(
            output_format(Path::new("out.pdf")).unwrap(),
            OutputFormat::Pdf
        );
        assert!(output_format(Path::new("out.png")).is_err());
    }

    #[test]
    fn xlsx_export_recalculates_stale_formula_results() {
        let stem = format!("iron-canvas-formula-export-{}", std::process::id());
        let input = std::env::temp_dir().join(format!("{stem}.xlsx"));
        let output = std::env::temp_dir().join(format!("{stem}.svg"));
        let _ = fs::remove_file(&input);
        let _ = fs::remove_file(&output);

        let mut source =
            UserModel::new_empty("formula-export", "en", "UTC", "en").expect("create workbook");
        source
            .set_user_input(0, 1, 2, "1")
            .expect("set initial formula input");
        source
            .set_user_input(0, 1, 1, "=B1*2")
            .expect("set formula");
        source.pause_evaluation();
        source
            .set_user_input(0, 1, 2, "3")
            .expect("change formula input without evaluating");
        assert_eq!(
            source
                .get_formatted_cell_value(0, 1, 1)
                .expect("read stale result"),
            "2"
        );
        save_to_xlsx(source.get_model(), &input.to_string_lossy()).expect("save stale cache");

        export_xlsx(
            &input,
            &output,
            None,
            CellRange::new(CellCoord { row: 1, col: 1 }, CellCoord { row: 1, col: 1 })
                .expect("valid range"),
            &CanvasTheme::light(),
        )
        .expect("export workbook");
        let svg = fs::read_to_string(&output).expect("read exported SVG");

        let _ = fs::remove_file(&input);
        let _ = fs::remove_file(&output);
        assert!(
            svg.contains(">6</text>"),
            "recalculated formula result is missing"
        );
    }
}
