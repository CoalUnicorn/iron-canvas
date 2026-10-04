//! Read-only compilation of requested worksheet cells from IronCalc.
//!
//! The compiler takes the workbook by shared reference and returns owned
//! paint-facing records. Repeated cell coordinates in one request are compiled
//! only once, so one `ExtendedStyle` read supplies both ordinary style and CF
//! decoration for that cell.

use std::collections::HashSet;
use std::fmt;

use iron_canvas_core::{CellCoord, CellDecoration, CellKind, CellStyle};
use ironcalc_base::UserModel;

use crate::color_resolver;
use crate::convert::{cell_decoration_from_extended, cell_type_to_kind, style_to_core};

/// One distinct cell compiled from a worksheet.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCell {
    pub coordinate: CellCoord,
    pub formatted_value: String,
    pub cell_type: CellKind,
    pub style: CellStyle,
    pub decoration: Option<CellDecoration>,
}

/// The workbook read that prevented a cell from being prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellReadOperation {
    ExtendedStyle,
    CellType,
    FormattedValue,
}

/// A cell-scoped failure while compiling a requested worksheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorksheetCompileError {
    pub sheet: u32,
    pub cell: CellCoord,
    pub operation: CellReadOperation,
    pub message: String,
}

impl fmt::Display for WorksheetCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to read {:?} for sheet {}, cell ({}, {}): {}",
            self.operation, self.sheet, self.cell.row, self.cell.col, self.message
        )
    }
}

impl std::error::Error for WorksheetCompileError {}

/// Compiles requested worksheet cells using read-only IronCalc access.
#[derive(Debug, Default, Clone, Copy)]
pub struct WorksheetCompiler;

impl WorksheetCompiler {
    pub const fn new() -> Self {
        Self
    }

    /// Compile each distinct requested coordinate once, preserving the order
    /// of its first appearance. Callers include any visible cells, merge
    /// anchors, or active-cell restoration cells needed by their preparation.
    ///
    /// All returned content is owned; no reference into `workbook` is retained.
    pub fn prepare_cells(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        coordinates: impl IntoIterator<Item = CellCoord>,
    ) -> Result<Vec<PreparedCell>, WorksheetCompileError> {
        compile_distinct(coordinates, |coordinate| {
            self.prepare_cell(workbook, sheet, coordinate)
        })
    }

    fn prepare_cell(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        coordinate: CellCoord,
    ) -> Result<PreparedCell, WorksheetCompileError> {
        let extended_style = workbook
            .get_extended_cell_style(sheet, coordinate.row, coordinate.col)
            .map_err(|error| {
                read_error(sheet, coordinate, CellReadOperation::ExtendedStyle, error)
            })?;
        let cell_type = workbook
            .get_cell_type(sheet, coordinate.row, coordinate.col)
            .map_err(|error| read_error(sheet, coordinate, CellReadOperation::CellType, error))?;
        let formatted_value = workbook
            .get_formatted_cell_value(sheet, coordinate.row, coordinate.col)
            .map_err(|error| {
                read_error(sheet, coordinate, CellReadOperation::FormattedValue, error)
            })?;

        let resolve = color_resolver(workbook);
        let decoration = cell_decoration_from_extended(&extended_style, &resolve);
        let style = style_to_core(extended_style.style, &resolve);

        Ok(PreparedCell {
            coordinate,
            formatted_value,
            cell_type: cell_type_to_kind(cell_type),
            style,
            decoration,
        })
    }
}

fn read_error(
    sheet: u32,
    cell: CellCoord,
    operation: CellReadOperation,
    error: impl fmt::Debug,
) -> WorksheetCompileError {
    WorksheetCompileError {
        sheet,
        cell,
        operation,
        message: format!("{error:?}"),
    }
}

fn compile_distinct<T, E>(
    coordinates: impl IntoIterator<Item = CellCoord>,
    mut compile_one: impl FnMut(CellCoord) -> Result<T, E>,
) -> Result<Vec<T>, E> {
    let mut seen = HashSet::new();
    let mut prepared = Vec::new();
    for coordinate in coordinates {
        if seen.insert((coordinate.row, coordinate.col)) {
            prepared.push(compile_one(coordinate)?);
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_distinct_requested_coordinate_is_compiled_once_in_input_order() {
        let first = CellCoord { row: 2, col: 4 };
        let second = CellCoord { row: 3, col: 1 };
        let mut reads = Vec::new();

        let compiled = compile_distinct([first, second, first], |coordinate| {
            reads.push(coordinate);
            Ok::<_, ()>(coordinate)
        })
        .expect("test compilation succeeds");

        assert_eq!(reads, [first, second]);
        assert_eq!(compiled, [first, second]);
    }
}
