//! The culling loop: a selection moving through the photo grid, and the marks
//! set from the keyboard (stars, color label, pick or reject). Pure state, no
//! I/O. `apply` reports what changed so the caller can save it.

use crate::layout::{Direction, Layout};

/// A star rating. Unrated photos have no `Stars` at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stars {
    One = 1,
    Two,
    Three,
    Four,
    Five,
}

impl Stars {
    pub fn count(self) -> usize {
        self as usize
    }

    /// The other direction, for reading `xmp:Rating`. 0 and anything out of
    /// range (Bridge writes -1 for rejected) mean unrated.
    pub fn from_count(count: i64) -> Option<Self> {
        match count {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            _ => None,
        }
    }
}

/// Color labels, named the way Lightroom writes them to `xmp:Label`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl Label {
    pub const ALL: [Self; 5] = [Self::Red, Self::Yellow, Self::Green, Self::Blue, Self::Purple];

    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Pick,
    Reject,
}

/// Everything the user can mark on one photo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Marks {
    pub stars: Option<Stars>,
    pub label: Option<Label>,
    pub flag: Option<Flag>,
}

/// What the user asked for. The UI maps keys and clicks to these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Move(Direction),
    Select(usize),
    /// Rate the selected photo. `None` clears the rating.
    Rate(Option<Stars>),
    /// Set the selected photo's label, or clear it if it already has this one.
    ToggleLabel(Label),
    /// Flag the selected photo. `None` clears the flag.
    Flag(Option<Flag>),
}

/// The selection and every photo's marks. `selected` is a library photo
/// index and is always valid, because a `Culling` only exists for a library
/// with at least one photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Culling {
    marks: Vec<Marks>,
    selected: usize,
}

impl Culling {
    /// Starts with the first photo selected. Returns `None` for zero photos.
    pub fn new(marks: Vec<Marks>) -> Option<Self> {
        (!marks.is_empty()).then_some(Self { marks, selected: 0 })
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn marks(&self, photo: usize) -> Marks {
        self.marks.get(photo).copied().unwrap_or_default()
    }

    /// Applies one intent. `layout` is the grid as currently shown, which
    /// decides where the arrow keys go. Returns the photo whose marks changed,
    /// if any, so the caller can save them.
    pub fn apply(&mut self, intent: Intent, layout: &Layout) -> Option<usize> {
        let before = self.marks[self.selected];
        let marks = &mut self.marks[self.selected];
        match intent {
            Intent::Move(direction) => {
                self.selected = layout.neighbor(self.selected, direction);
                return None;
            }
            Intent::Select(photo) => {
                if photo < self.marks.len() {
                    self.selected = photo;
                }
                return None;
            }
            Intent::Rate(stars) => marks.stars = stars,
            Intent::ToggleLabel(label) => {
                marks.label = if marks.label == Some(label) { None } else { Some(label) };
            }
            Intent::Flag(flag) => marks.flag = flag,
        }
        (*marks != before).then_some(self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Metrics, Section};

    fn layout(count: usize) -> Layout {
        let sections = [Section { title: "a".into(), photos: (0..count).collect() }];
        Layout::new(&sections, &vec![1.5; count], Metrics { width: 1000., target_height: 200., gap: 0. })
    }

    fn culling(count: usize) -> Culling {
        Culling::new(vec![Marks::default(); count]).unwrap()
    }

    #[test]
    fn an_empty_library_has_no_culling_state() {
        assert_eq!(Culling::new(Vec::new()), None);
    }

    #[test]
    fn arrows_move_the_selection_through_the_layout() {
        // Four 3:2 photos per row at this width.
        let grid = layout(10);
        let mut c = culling(10);
        assert_eq!(c.apply(Intent::Move(Direction::Down), &grid), None);
        assert_eq!(c.selected(), 4);
        c.apply(Intent::Move(Direction::Right), &grid);
        assert_eq!(c.selected(), 5);
    }

    #[test]
    fn selecting_out_of_range_is_ignored() {
        let mut c = culling(3);
        c.apply(Intent::Select(3), &layout(3));
        assert_eq!(c.selected(), 0);
    }

    #[test]
    fn marks_apply_to_the_selection_and_report_real_changes_only() {
        let grid = layout(3);
        let mut c = culling(3);
        c.apply(Intent::Select(1), &grid);
        assert_eq!(c.apply(Intent::Rate(Some(Stars::Four)), &grid), Some(1));
        assert_eq!(c.apply(Intent::Rate(Some(Stars::Four)), &grid), None, "same rating again changes nothing");
        assert_eq!(c.apply(Intent::Flag(Some(Flag::Pick)), &grid), Some(1));
        assert_eq!(c.marks(1), Marks { stars: Some(Stars::Four), label: None, flag: Some(Flag::Pick) });
        assert_eq!(c.marks(0), Marks::default());

        c.apply(Intent::Rate(None), &grid);
        assert_eq!(c.marks(1).stars, None);
    }

    #[test]
    fn the_same_label_twice_clears_it() {
        let grid = layout(1);
        let mut c = culling(1);
        c.apply(Intent::ToggleLabel(Label::Green), &grid);
        assert_eq!(c.marks(0).label, Some(Label::Green));
        c.apply(Intent::ToggleLabel(Label::Red), &grid);
        assert_eq!(c.marks(0).label, Some(Label::Red));
        c.apply(Intent::ToggleLabel(Label::Red), &grid);
        assert_eq!(c.marks(0).label, None);
    }
}
