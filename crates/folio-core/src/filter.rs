//! Filters: conditions on marks and metadata that narrow what a view shows,
//! and the suggestions the filter bar offers while typing. Conditions of the
//! same kind widen (two lenses show photos from either), different kinds
//! narrow (a lens and a rating show photos that have both). Pure.

use crate::culling::{Flag, Label, Marks, Stars};
use crate::library::{Library, Photo, key};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    AtLeast(Stars),
    Exactly(Stars),
    Unrated,
}

/// A month or a whole year of capture time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSpan {
    Year(u16),
    Month(u16, u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    Rating(Rating),
    /// `None` means unflagged.
    Flag(Option<Flag>),
    /// `None` means no label.
    Label(Option<Label>),
    Camera(String),
    Lens(String),
    /// A folder relative to the library root, with everything below it.
    Folder(String),
    Date(DateSpan),
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

impl Condition {
    pub fn matches(&self, photo: &Photo, marks: Marks) -> bool {
        match self {
            Self::Rating(Rating::AtLeast(stars)) => marks.stars.is_some_and(|s| s >= *stars),
            Self::Rating(Rating::Exactly(stars)) => marks.stars == Some(*stars),
            Self::Rating(Rating::Unrated) => marks.stars.is_none(),
            Self::Flag(flag) => marks.flag == *flag,
            Self::Label(label) => marks.label == *label,
            Self::Camera(camera) => photo.exif.camera.as_ref() == Some(camera),
            Self::Lens(lens) => photo.exif.lens.as_ref() == Some(lens),
            Self::Folder(folder) => {
                let (here, wanted) = (key(photo.folder()), key(folder));
                here == wanted || here.starts_with(&format!("{wanted}/"))
            }
            Self::Date(span) => photo.exif.captured.is_some_and(|t| match span {
                DateSpan::Year(year) => t.year == *year,
                DateSpan::Month(year, month) => (t.year, t.month) == (*year, *month),
            }),
        }
    }

    /// How the condition reads in the bar and on its chip.
    pub fn label(&self) -> String {
        match self {
            Self::Rating(Rating::AtLeast(stars)) => format!("★ {} or more", stars.count()),
            Self::Rating(Rating::Exactly(stars)) => format!("★ exactly {}", stars.count()),
            Self::Rating(Rating::Unrated) => "Unrated".into(),
            Self::Flag(Some(Flag::Pick)) => "Picks".into(),
            Self::Flag(Some(Flag::Reject)) => "Rejects".into(),
            Self::Flag(None) => "Unflagged".into(),
            Self::Label(Some(label)) => format!("{} label", label.name()),
            Self::Label(None) => "No label".into(),
            Self::Camera(camera) => format!("Camera {camera}"),
            Self::Lens(lens) => format!("Lens {lens}"),
            Self::Folder(folder) => format!("Folder {}", folder.replace('/', " / ")),
            Self::Date(DateSpan::Year(year)) => year.to_string(),
            Self::Date(DateSpan::Month(year, month)) => format!("{} {year}", MONTHS[usize::from(month.clamp(&1, &12) - 1)]),
        }
    }

    /// Conditions of the same kind widen each other, different kinds narrow.
    fn kind(&self) -> usize {
        match self {
            Self::Rating(_) => 0,
            Self::Flag(_) => 1,
            Self::Label(_) => 2,
            Self::Camera(_) => 3,
            Self::Lens(_) => 4,
            Self::Folder(_) => 5,
            Self::Date(_) => 6,
        }
    }

    /// Extra words that find a condition by what it is, so typing `rating`
    /// or `lens` lists those.
    fn kind_words(&self) -> &'static str {
        ["rating stars", "flag", "label color colour", "camera", "lens", "folder", "date"][self.kind()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    conditions: Vec<Condition>,
}

impl Filter {
    pub fn new(conditions: Vec<Condition>) -> Self {
        let mut filter = Self::default();
        for condition in conditions {
            filter.add(condition);
        }
        filter
    }

    pub fn conditions(&self) -> &[Condition] {
        &self.conditions
    }

    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    /// Adds a condition. Returns false if it's already there.
    pub fn add(&mut self, condition: Condition) -> bool {
        if self.conditions.contains(&condition) {
            return false;
        }
        self.conditions.push(condition);
        true
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.conditions.len() {
            self.conditions.remove(index);
        }
    }

    pub fn pop(&mut self) -> Option<Condition> {
        self.conditions.pop()
    }

    pub fn matches(&self, photo: &Photo, marks: Marks) -> bool {
        let mut kinds = self.conditions.iter().map(Condition::kind).collect::<Vec<_>>();
        kinds.sort_unstable();
        kinds.dedup();
        kinds.iter().all(|&kind| self.conditions.iter().filter(|c| c.kind() == kind).any(|c| c.matches(photo, marks)))
    }

    /// The photos out of `photos` that pass.
    pub fn apply(&self, library: &Library, photos: &[usize], marks: impl Fn(usize) -> Marks) -> Vec<usize> {
        photos.iter().copied().filter(|&p| self.matches(library.photo(p), marks(p))).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub condition: Condition,
    /// How many of the view's photos the filter would show with it added.
    pub count: usize,
}

/// What the filter bar offers for `typed`, for a view showing `photos`. Every
/// word typed must appear in a suggestion. With nothing typed, the two best
/// of each kind show, so the empty bar doubles as an overview. Suggestions
/// that would show no photos, or are already in the filter, are left out.
pub fn suggest(
    library: &Library,
    photos: &[usize],
    marks: impl Fn(usize) -> Marks,
    filter: &Filter,
    typed: &str,
) -> Vec<Suggestion> {
    let words: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    let mut found: Vec<Suggestion> = candidates(library, photos)
        .into_iter()
        .filter(|c| !filter.conditions.contains(c))
        .filter(|c| {
            let text = format!("{} {}", c.label(), c.kind_words()).to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
        })
        .map(|condition| {
            let mut with = filter.clone();
            with.add(condition.clone());
            let count = photos.iter().filter(|&&p| with.matches(library.photo(p), marks(p))).count();
            Suggestion { condition, count }
        })
        .filter(|s| s.count > 0)
        .collect();

    // Fixed kinds (rating, flag, label) keep their natural order. The rest go
    // by how many photos they'd show.
    found.sort_by(|a, b| {
        a.condition.kind().cmp(&b.condition.kind()).then_with(|| match a.condition.kind() {
            0..=2 => std::cmp::Ordering::Equal,
            _ => b.count.cmp(&a.count),
        })
    });
    if words.is_empty() {
        let mut per_kind = [0; 7];
        found.retain(|s| {
            per_kind[s.condition.kind()] += 1;
            per_kind[s.condition.kind()] <= 2
        });
    }
    found
}

/// Everything that could be filtered on among `photos`.
fn candidates(library: &Library, photos: &[usize]) -> Vec<Condition> {
    let mut all: Vec<Condition> = Vec::new();
    for count in [3, 4, 5, 2, 1] {
        all.extend(Stars::from_count(count).map(|s| Condition::Rating(Rating::AtLeast(s))));
    }
    all.extend((1..=5).filter_map(Stars::from_count).map(|s| Condition::Rating(Rating::Exactly(s))));
    all.push(Condition::Rating(Rating::Unrated));
    all.extend([Some(Flag::Pick), Some(Flag::Reject), None].map(Condition::Flag));
    all.extend(Label::ALL.map(|l| Condition::Label(Some(l))));
    all.push(Condition::Label(None));

    let mut push = |condition: Condition| {
        if !all.contains(&condition) {
            all.push(condition);
        }
    };
    for &photo in photos {
        let photo = library.photo(photo);
        if let Some(camera) = &photo.exif.camera {
            push(Condition::Camera(camera.clone()));
        }
        if let Some(lens) = &photo.exif.lens {
            push(Condition::Lens(lens.clone()));
        }
        let folder = photo.folder();
        let mut end = folder.len();
        while end > 0 {
            push(Condition::Folder(folder[..end].to_owned()));
            end = folder[..end].rfind('/').unwrap_or(0);
        }
        if let Some(time) = photo.exif.captured {
            push(Condition::Date(DateSpan::Year(time.year)));
            push(Condition::Date(DateSpan::Month(time.year, time.month)));
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif::CaptureTime;
    use crate::library::tests::photo;

    fn library() -> Library {
        let mut photos = vec![photo("Home/Fair/a.NEF"), photo("Home/Fair/b.NEF"), photo("Nature/c.NEF")];
        photos[0].exif.lens = Some("NIKKOR Z DX 16-50mm".into());
        photos[1].exif.lens = Some("VR 150-600mm".into());
        photos[2].exif.lens = Some("NIKKOR Z DX 16-50mm".into());
        photos[0].exif.captured = CaptureTime::parse("2024:10:20 10:00:00");
        photos[2].exif.captured = CaptureTime::parse("2024:11:02 10:00:00");
        Library::new("/lib".into(), photos)
    }

    fn marks(photo: usize) -> Marks {
        match photo {
            0 => Marks { stars: Stars::from_count(4), flag: Some(Flag::Pick), label: None },
            1 => Marks { stars: Stars::from_count(2), flag: None, label: Some(Label::Red) },
            _ => Marks::default(),
        }
    }

    const ALL: [usize; 3] = [0, 1, 2];

    #[test]
    fn same_kind_widens_and_different_kinds_narrow() {
        let lib = library();
        let lens = |l: &str| Condition::Lens(l.into());
        let filter = Filter::new(vec![lens("NIKKOR Z DX 16-50mm"), lens("VR 150-600mm")]);
        assert_eq!(filter.apply(&lib, &ALL, marks), vec![0, 1, 2]);

        let filter = Filter::new(vec![lens("NIKKOR Z DX 16-50mm"), Condition::Rating(Rating::AtLeast(Stars::Three))]);
        assert_eq!(filter.apply(&lib, &ALL, marks), vec![0]);
        assert_eq!(Filter::default().apply(&lib, &ALL, marks), ALL, "no conditions shows everything");
    }

    #[test]
    fn folders_include_subfolders_and_dates_match_by_month() {
        let lib = library();
        assert_eq!(Filter::new(vec![Condition::Folder("home".into())]).apply(&lib, &ALL, marks), vec![0, 1]);
        assert_eq!(Filter::new(vec![Condition::Folder("Home/Fa".into())]).apply(&lib, &ALL, marks), Vec::<usize>::new());
        assert_eq!(Filter::new(vec![Condition::Date(DateSpan::Month(2024, 11))]).apply(&lib, &ALL, marks), vec![2]);
        assert_eq!(Filter::new(vec![Condition::Flag(None)]).apply(&lib, &ALL, marks), vec![1, 2]);
    }

    #[test]
    fn typing_finds_suggestions_with_counts_and_skips_dead_ends() {
        let lib = library();
        let found = suggest(&lib, &ALL, marks, &Filter::default(), "16");
        assert_eq!(found, vec![Suggestion { condition: Condition::Lens("NIKKOR Z DX 16-50mm".into()), count: 2 }]);

        let picks = suggest(&lib, &ALL, marks, &Filter::default(), "pi");
        assert_eq!(picks[0].condition, Condition::Flag(Some(Flag::Pick)));

        let ratings = suggest(&lib, &ALL, marks, &Filter::default(), "rating");
        assert!(ratings.iter().all(|s| matches!(s.condition, Condition::Rating(_))));
        assert!(!ratings.iter().any(|s| s.condition == Condition::Rating(Rating::AtLeast(Stars::Five))), "no photo has five stars");
    }

    #[test]
    fn counts_reflect_the_current_filter_and_the_empty_bar_is_an_overview() {
        let lib = library();
        let filter = Filter::new(vec![Condition::Folder("Home".into())]);
        let found = suggest(&lib, &ALL, marks, &filter, "lens");
        let counts: Vec<usize> = found.iter().map(|s| s.count).collect();
        assert_eq!(counts, vec![1, 1], "one photo each within Home");
        assert!(!suggest(&lib, &ALL, marks, &filter, "folder home").iter().any(|s| s.condition == Condition::Folder("Home".into())));

        let overview = suggest(&lib, &ALL, marks, &Filter::default(), "");
        for kind in 0..7 {
            assert!(overview.iter().filter(|s| s.condition.kind() == kind).count() <= 2);
        }
        assert!(overview.iter().any(|s| matches!(s.condition, Condition::Lens(_))));
    }
}
