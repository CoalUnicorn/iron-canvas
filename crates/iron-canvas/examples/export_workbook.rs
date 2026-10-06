//! Export a selected worksheet range from an `.xlsx` workbook to SVG or PDF.
//!
//! ```text
//! cargo run -p iron-canvas --example export_workbook -- book.xlsx \
//!   --range A1:H30 --output /tmp/book.svg
//! cargo run -p iron-canvas --example export_workbook -- book.xlsx \
//!   --sheet "Quarterly Results" --range B3:J28 --output /tmp/results.pdf
//! ```

use std::{path::PathBuf, process};

use clap::Parser;
use iron_canvas_core::CanvasTheme;
use iron_canvas_export::{CellRange, export_xlsx};

#[derive(Debug, Parser)]
#[command(
    about = "Export a worksheet range from an .xlsx workbook to SVG or PDF.",
    after_help = "Frozen rows and columns remain visible. Exports are limited to 50,000 visible cells and a 20,000 by 20,000 pixel page."
)]
struct Cli {
    /// Input .xlsx workbook.
    #[arg(value_name = "WORKBOOK.xlsx")]
    input: PathBuf,

    /// Worksheet name or zero-based index (default: first sheet).
    #[arg(long, value_name = "NAME|INDEX")]
    sheet: Option<String>,

    /// First and last visible cells, e.g. A1:H30. A single cell is also valid.
    #[arg(long, value_name = "A1:H30")]
    range: CellRange,

    /// Output path ending in .svg or .pdf.
    #[arg(long, value_name = "FILE.svg|FILE.pdf")]
    output: PathBuf,
}

fn main() {
    let options = Cli::parse();
    match export_xlsx(
        &options.input,
        &options.output,
        options.sheet.as_deref(),
        options.range,
        &CanvasTheme::light(),
    ) {
        Ok(sheet) => println!(
            "Exported {} sheet {} to {}",
            options.input.display(),
            sheet,
            options.output.display()
        ),
        Err(error) => {
            eprintln!("Export failed: {error}");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_workbook_range_output_and_optional_sheet() {
        let cli = Cli::try_parse_from([
            "export_workbook",
            "book.xlsx",
            "--range",
            "b3:J28",
            "--output",
            "out.svg",
            "--sheet",
            "Sales",
        ])
        .expect("valid CLI arguments");
        assert_eq!(cli.input, PathBuf::from("book.xlsx"));
        assert_eq!(
            cli.range.first(),
            iron_canvas_core::CellCoord { row: 3, col: 2 }
        );
        assert_eq!(
            cli.range.last(),
            iron_canvas_core::CellCoord { row: 28, col: 10 }
        );
        assert_eq!(cli.sheet.as_deref(), Some("Sales"));
    }
}
