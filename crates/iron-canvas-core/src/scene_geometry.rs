//! Bounded axis sampling for a model-independent grid viewport.
mod composition;
pub use composition::*;

/// One grid coordinate. Both axes are 1-based; no sheet identity is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellCoord {
    pub row: i32,
    pub col: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisCell {
    pub index: i32,
    pub start: i32,
    pub extent: i32,
}

#[derive(Debug, PartialEq)]
pub enum AxisError<E> {
    Bounds,
    Extent { index: i32 },
    Read { index: i32, error: E },
}

/// Sample only the visible window. Zero extents are skipped within `last`.
/// The final cell keeps its full extent so text aligns before viewport clipping.
pub fn sample_axis<E>(
    first: i32,
    last: i32,
    pixels: i32,
    mut extent: impl FnMut(i32) -> Result<f64, E>,
) -> Result<Vec<AxisCell>, AxisError<E>> {
    if first < 1 || first > last || pixels < 0 {
        return Err(AxisError::Bounds);
    }
    let mut cells = Vec::new();
    let mut cursor = 0_i32;
    for index in first..=last {
        if cursor >= pixels {
            break;
        }
        let value = extent(index).map_err(|error| AxisError::Read { index, error })?;
        if !value.is_finite() || value < 0.0 || value.round() > f64::from(i32::MAX) {
            return Err(AxisError::Extent { index });
        }
        let size = value.round() as i32;
        let end = cursor
            .checked_add(size)
            .ok_or(AxisError::Extent { index })?;
        if size > 0 {
            cells.push(AxisCell {
                index,
                start: cursor,
                extent: size,
            });
        }
        cursor = end;
    }
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sampling_is_bounded_and_preserves_the_final_full_cell() {
        let mut reads = Vec::new();
        let slots = sample_axis(2, 100, 25, |i| {
            reads.push(i);
            Ok::<_, ()>(if i == 2 { 0.0 } else { 20.0 })
        })
        .unwrap();
        assert_eq!(reads, [2, 3, 4]);
        assert_eq!(
            slots.last(),
            Some(&AxisCell {
                index: 4,
                start: 20,
                extent: 20
            })
        );
        let mut reads = 0;
        assert!(
            sample_axis(1, 4, 20, |_| {
                reads += 1;
                Ok::<_, ()>(0.0)
            })
            .unwrap()
            .is_empty()
        );
        assert_eq!(reads, 4);
    }
    #[test]
    fn invalid_extents_and_overflow_are_errors() {
        for extent in [-1.0, f64::NAN, f64::INFINITY, f64::from(i32::MAX) + 1.0] {
            assert!(sample_axis(1, 2, 20, |_| Ok::<_, ()>(extent)).is_err());
        }
        assert!(sample_axis(1, 2, i32::MAX, |_| Ok::<_, ()>(f64::from(i32::MAX - 1))).is_err());
    }
}
