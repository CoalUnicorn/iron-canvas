//! Pure keyboard classification. The host applies each returned intent.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationKey {
    Up,
    Down,
    Left,
    Right,
    Tab,
    Enter,
    PageUp,
    PageDown,
    Home,
    End,
    A,
    Space,
    F8,
    Escape,
}

impl NavigationKey {
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "ArrowUp" => Self::Up,
            "ArrowDown" => Self::Down,
            "ArrowLeft" => Self::Left,
            "ArrowRight" => Self::Right,
            "Tab" => Self::Tab,
            "Enter" => Self::Enter,
            "PageUp" => Self::PageUp,
            "PageDown" => Self::PageDown,
            "Home" => Self::Home,
            "End" => Self::End,
            "a" | "A" => Self::A,
            " " => Self::Space,
            "F8" => Self::F8,
            "Escape" => Self::Escape,
            _ => return None,
        })
    }
}

/// Primary means Control or Command under the host's platform policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NavigationModifiers {
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
    pub alt_graph: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingProfile {
    Basic,
    Extended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalAxis {
    Rows,
    Columns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationIntent {
    Step(Direction),
    ExtendStep(Direction),
    DataEdge(Direction),
    Traverse {
        axis: TraversalAxis,
        direction: TraversalDirection,
    },
    Page(PageDirection),
    RowStart,
    RowEnd,
    SheetStart,
    UsedEnd,
    SelectUsedRange,
    Cancel,
}

/// Classify grid keys only after the host handles editor and IME input.
/// Extended bindings remain unavailable until their engine gate passes.
pub fn classify_navigation(
    key: NavigationKey,
    modifiers: NavigationModifiers,
    profile: BindingProfile,
) -> Option<NavigationIntent> {
    use NavigationIntent as I;
    use NavigationKey as K;
    // Extended bindings need an engine support gate. Do not report Basic
    // behavior under the Extended name before that gate passes.
    if profile == BindingProfile::Extended {
        return None;
    }
    if modifiers.alt || modifiers.alt_graph {
        return None;
    }
    let NavigationModifiers { primary, shift, .. } = modifiers;
    let arrow = match key {
        K::Up => Some(Direction::Up),
        K::Down => Some(Direction::Down),
        K::Left => Some(Direction::Left),
        K::Right => Some(Direction::Right),
        _ => None,
    };
    if let Some(direction) = arrow {
        return match (primary, shift) {
            (false, false) => Some(I::Step(direction)),
            (false, true) => Some(I::ExtendStep(direction)),
            (true, false) => Some(I::DataEdge(direction)),
            (true, true) => None,
        };
    }
    match (key, primary, shift) {
        (K::Tab | K::Enter, false, _) => Some(I::Traverse {
            axis: if key == K::Tab {
                TraversalAxis::Rows
            } else {
                TraversalAxis::Columns
            },
            direction: if shift {
                TraversalDirection::Backward
            } else {
                TraversalDirection::Forward
            },
        }),
        (K::PageUp, false, false) => Some(I::Page(PageDirection::Up)),
        (K::PageDown, false, false) => Some(I::Page(PageDirection::Down)),
        (K::Home, false, false) => Some(I::RowStart),
        (K::End, false, false) => Some(I::RowEnd),
        (K::Home, true, false) => Some(I::SheetStart),
        (K::End, true, false) => Some(I::UsedEnd),
        (K::A, true, false) => Some(I::SelectUsedRange),
        (K::Escape, false, false) => Some(I::Cancel),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_matrix_rejects_every_unsupported_modifier_combination() {
        for profile in [BindingProfile::Basic, BindingProfile::Extended] {
            for key in [
                NavigationKey::Up,
                NavigationKey::Down,
                NavigationKey::Left,
                NavigationKey::Right,
                NavigationKey::Tab,
                NavigationKey::Enter,
                NavigationKey::PageUp,
                NavigationKey::PageDown,
                NavigationKey::Home,
                NavigationKey::End,
                NavigationKey::A,
                NavigationKey::Space,
                NavigationKey::F8,
                NavigationKey::Escape,
            ] {
                for bits in 0..16 {
                    let m = NavigationModifiers {
                        primary: bits & 1 != 0,
                        shift: bits & 2 != 0,
                        alt: bits & 4 != 0,
                        alt_graph: bits & 8 != 0,
                    };
                    let supported = profile == BindingProfile::Basic
                        && !m.alt
                        && !m.alt_graph
                        && match key {
                            NavigationKey::Up
                            | NavigationKey::Down
                            | NavigationKey::Left
                            | NavigationKey::Right => !(m.primary && m.shift),
                            NavigationKey::Tab | NavigationKey::Enter => !m.primary,
                            NavigationKey::Home | NavigationKey::End => !m.shift,
                            NavigationKey::PageUp
                            | NavigationKey::PageDown
                            | NavigationKey::Escape => !m.primary && !m.shift,
                            NavigationKey::A => m.primary && !m.shift,
                            NavigationKey::Space | NavigationKey::F8 => false,
                        };
                    assert_eq!(
                        classify_navigation(key, m, profile).is_some(),
                        supported,
                        "{key:?} {m:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn basic_targets_follow_the_standalone_keymap() {
        use NavigationIntent as I;
        let cases = [
            ("ArrowUp", false, false, I::Step(Direction::Up)),
            ("ArrowDown", true, false, I::DataEdge(Direction::Down)),
            ("ArrowLeft", false, true, I::ExtendStep(Direction::Left)),
            ("ArrowRight", false, false, I::Step(Direction::Right)),
            (
                "Tab",
                false,
                false,
                I::Traverse {
                    axis: TraversalAxis::Rows,
                    direction: TraversalDirection::Forward,
                },
            ),
            (
                "Tab",
                false,
                true,
                I::Traverse {
                    axis: TraversalAxis::Rows,
                    direction: TraversalDirection::Backward,
                },
            ),
            (
                "Enter",
                false,
                false,
                I::Traverse {
                    axis: TraversalAxis::Columns,
                    direction: TraversalDirection::Forward,
                },
            ),
            (
                "Enter",
                false,
                true,
                I::Traverse {
                    axis: TraversalAxis::Columns,
                    direction: TraversalDirection::Backward,
                },
            ),
            ("PageUp", false, false, I::Page(PageDirection::Up)),
            ("PageDown", false, false, I::Page(PageDirection::Down)),
            ("Home", false, false, I::RowStart),
            ("End", false, false, I::RowEnd),
            ("Home", true, false, I::SheetStart),
            ("End", true, false, I::UsedEnd),
            ("a", true, false, I::SelectUsedRange),
            ("Escape", false, false, I::Cancel),
        ];
        for (key, primary, shift, expected) in cases {
            assert_eq!(
                classify_navigation(
                    NavigationKey::from_key(key).unwrap(),
                    NavigationModifiers {
                        primary,
                        shift,
                        ..Default::default()
                    },
                    BindingProfile::Basic
                ),
                Some(expected)
            );
        }
        assert_eq!(NavigationKey::from_key("F9"), None);
    }
}
