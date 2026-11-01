use iron_canvas::{
    AreaId, SelectionArea, SelectionError, SelectionKind, SelectionMode, SelectionOperation,
    SelectionSnapshot, SelectionTarget, propose_selection,
};
use iron_canvas_core::{CellCoord, scene_geometry::GridRange};
use std::collections::BTreeSet;

fn cell(row: i32, col: i32) -> CellCoord {
    CellCoord { row, col }
}
fn target(r1: i32, c1: i32, r2: i32, c2: i32) -> SelectionTarget {
    SelectionTarget::new(
        SelectionKind::Cells,
        cell(r1, c1),
        cell(r2, c2),
        GridRange {
            first: cell(r1, c1),
            last: cell(r2, c2),
        },
    )
    .unwrap()
}
fn snapshot(target: SelectionTarget) -> SelectionSnapshot {
    SelectionSnapshot::new(
        1,
        0,
        0,
        vec![SelectionArea::new(AreaId(1), target)],
        AreaId(1),
        target.focus(),
        SelectionMode::Normal,
    )
    .unwrap()
}
fn coverage(snapshot: &SelectionSnapshot) -> BTreeSet<(i32, i32)> {
    let mut result = BTreeSet::new();
    for area in snapshot.areas() {
        let r = area.range();
        for row in r.first.row..=r.last.row {
            for col in r.first.col..=r.last.col {
                result.insert((row, col));
            }
        }
    }
    result
}

#[test]
fn exhaustive_small_grid_add_and_subtract_match_a_cell_set_oracle() {
    let rectangles: Vec<_> = (1..=3)
        .flat_map(|r1| {
            (1..=3).flat_map(move |c1| {
                (r1..=3).flat_map(move |r2| (c1..=3).map(move |c2| target(r1, c1, r2, c2)))
            })
        })
        .collect();
    for a in &rectangles {
        for b in &rectangles {
            let original = snapshot(*a);
            let cells_a = coverage(&original);
            let cells_b = coverage(&snapshot(*b));
            let added = propose_selection(&original, SelectionOperation::Add, *b).unwrap();
            assert_eq!(coverage(&added), cells_a.union(&cells_b).copied().collect());
            let removed = propose_selection(&original, SelectionOperation::Subtract, *b).unwrap();
            let expected: BTreeSet<_> = cells_a.difference(&cells_b).copied().collect();
            if expected.is_empty() {
                assert_eq!(coverage(&removed), cells_b);
            } else {
                assert_eq!(coverage(&removed), expected);
            }
            assert_eq!(coverage(&original), cells_a);
            assert!(
                removed
                    .areas()
                    .iter()
                    .find(|area| area.id() == removed.active_area())
                    .unwrap()
                    .range()
                    .contains(removed.active_cell())
            );
        }
    }
}

#[test]
fn subtraction_keeps_focus_piece_identity_and_uses_a_deterministic_nearest_cell() {
    let original = snapshot(target(1, 1, 3, 3));
    let split =
        propose_selection(&original, SelectionOperation::Subtract, target(1, 2, 3, 2)).unwrap();
    assert_eq!(split.active_area(), AreaId(1));
    assert_eq!(split.active_cell(), cell(3, 3));
    assert_eq!(split.areas().len(), 2);
    let removed_focus =
        propose_selection(&original, SelectionOperation::Subtract, target(3, 3, 3, 3)).unwrap();
    assert_eq!(removed_focus.active_cell(), cell(2, 3));
    let with_next = removed_focus.clone().with_next_revision().unwrap();
    assert_eq!(with_next.revision(), 1);
    assert_eq!(original.revision(), 0);
    let invalid_anchor = target(2, 2, 3, 3);
    assert_eq!(
        propose_selection(&original, SelectionOperation::ExtendActive, invalid_anchor),
        Err(SelectionError::InvalidArea)
    );
}

#[test]
fn limits_full_sheet_and_exhausted_ids_reject_without_widening() {
    let full = SelectionTarget::new(
        SelectionKind::Sheet,
        cell(1, 1),
        cell(1_048_576, 16_384),
        GridRange {
            first: cell(1, 1),
            last: cell(1_048_576, 16_384),
        },
    )
    .unwrap();
    let original = snapshot(full);
    let cut =
        propose_selection(&original, SelectionOperation::Subtract, target(2, 2, 2, 2)).unwrap();
    assert_eq!(cut.areas().len(), 4);
    assert!(
        !cut.areas()
            .iter()
            .any(|area| area.range().contains(cell(2, 2)))
    );
    let mut many = snapshot(target(1, 1, 1, 1));
    for row in 2..=256 {
        many = propose_selection(&many, SelectionOperation::Add, target(row, 1, row, 1)).unwrap();
    }
    let before = many.clone();
    assert_eq!(
        propose_selection(&many, SelectionOperation::Add, target(257, 1, 257, 1)),
        Err(SelectionError::AreaLimit)
    );
    assert_eq!(many, before);
    let max_id = SelectionSnapshot::new(
        1,
        0,
        u64::MAX,
        vec![SelectionArea::new(AreaId(u64::MAX), target(1, 1, 1, 1))],
        AreaId(u64::MAX),
        cell(1, 1),
        SelectionMode::Normal,
    )
    .unwrap();
    assert_eq!(
        propose_selection(&max_id, SelectionOperation::Add, target(2, 2, 2, 2)),
        Err(SelectionError::IdentityExhausted)
    );
    assert_eq!(
        max_id.with_next_revision(),
        Err(SelectionError::RevisionExhausted)
    );
    assert_eq!(
        SelectionTarget::new(
            SelectionKind::Cells,
            cell(0, 1),
            cell(1, 1),
            GridRange {
                first: cell(0, 1),
                last: cell(1, 1)
            }
        ),
        Err(SelectionError::Bounds)
    );
    assert_eq!(
        SelectionSnapshot::new(
            1,
            0,
            0,
            vec![SelectionArea::new(AreaId(1), target(1, 1, 1, 1))],
            AreaId(1),
            cell(2, 2),
            SelectionMode::Normal
        ),
        Err(SelectionError::InvalidActiveCell)
    );
}

#[test]
fn removed_area_ids_are_not_reused_by_later_proposals() {
    let original = snapshot(target(1, 1, 1, 1));
    let added = propose_selection(&original, SelectionOperation::Add, target(3, 3, 3, 3)).unwrap();
    let removed =
        propose_selection(&added, SelectionOperation::Subtract, target(3, 3, 3, 3)).unwrap();
    let next = propose_selection(&removed, SelectionOperation::Add, target(4, 4, 4, 4)).unwrap();
    assert!(next.active_area().0 > added.active_area().0);
}

#[test]
fn intersecting_full_axis_shapes_reject_piece_growth_before_the_area_limit() {
    let rows = SelectionTarget::new(
        SelectionKind::Rows,
        cell(5, 1),
        cell(6, 16_384),
        GridRange {
            first: cell(5, 1),
            last: cell(6, 16_384),
        },
    )
    .unwrap();
    assert_eq!(rows.kind(), SelectionKind::Rows);
    let columns = SelectionTarget::new(
        SelectionKind::Columns,
        cell(1, 5),
        cell(1_048_576, 6),
        GridRange {
            first: cell(1, 5),
            last: cell(1_048_576, 6),
        },
    )
    .unwrap();
    assert!(propose_selection(&snapshot(rows), SelectionOperation::Add, columns).is_ok());
    let mut current = snapshot(target(1, 1, 1, 33));
    for row in (3..=31).step_by(2) {
        current =
            propose_selection(&current, SelectionOperation::Add, target(row, 1, row, 33)).unwrap();
    }
    let mut rejected = false;
    for col in (1..=31).step_by(2) {
        let before = current.clone();
        match propose_selection(&current, SelectionOperation::Add, target(1, col, 33, col)) {
            Ok(next) => current = next,
            Err(error) => {
                assert_eq!(error, SelectionError::PieceLimit);
                assert_eq!(current, before);
                assert!(current.areas().len() < 256);
                rejected = true;
                break;
            }
        }
    }
    assert!(rejected);
}
