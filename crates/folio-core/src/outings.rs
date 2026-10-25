//! Sorting unsorted photos: group them into outings wherever there's a big
//! gap in capture time, then let the user name each outing, give it a
//! category, and accept it. Accepted outings become `Category/Outing/`
//! folders once the moves are approved (see `moves`). Pure.

use std::collections::{HashMap, HashSet};

use crate::exif::CaptureTime;
use crate::layout::Section;
use crate::library::{FileMove, Library, folder_name, key};

/// The gaps, in minutes, that the user steps through to start a new
/// outing sooner or later. The library's setting may be any other value.
const GAP_STEPS: [i64; 10] = [15, 30, 60, 120, 180, 240, 360, 480, 720, 1440];

/// The next step from `minutes`, shorter or longer. `None` past either end.
pub fn step_gap(minutes: i64, longer: bool) -> Option<i64> {
    if longer { GAP_STEPS.iter().copied().find(|&step| step > minutes) } else { GAP_STEPS.iter().rev().copied().find(|&step| step < minutes) }
}

/// `90 minutes`, `1 hour`, `2 hours`, `2.5 hours`.
pub fn gap_text(minutes: i64) -> String {
    match minutes {
        60 => "1 hour".to_owned(),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        m if m > 60 && m % 30 == 0 => format!("{}.5 hours", m / 60),
        m => format!("{m} minutes"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outing {
    /// Library photo indices in capture order.
    photos: Vec<usize>,
    name: String,
    category: Option<String>,
    /// Only an outing with a category can be accepted.
    accepted: bool,
    /// The user changed it, so it's kept as a draft until its photos move
    /// (see `drafts`). Untouched outings are simply grouped again.
    edited: bool,
}

impl Outing {
    /// An outing for photos that came back from `folder` (`Category/Name`)
    /// through undo, so the user's names come back too.
    pub fn restored(photos: Vec<usize>, folder: &str) -> Self {
        let (category, name) = folder.split_once('/').map_or((None, folder), |(c, n)| (Some(c.to_owned()), n));
        Self { photos, name: name.to_owned(), category, accepted: false, edited: true }
    }

    pub fn photos(&self) -> &[usize] {
        &self.photos
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    pub fn is_accepted(&self) -> bool {
        self.accepted
    }

    /// Where this outing's photos go: `Category/Name`.
    pub fn folder(&self) -> Option<String> {
        self.category.as_ref().map(|category| format!("{category}/{}", self.name))
    }
}

/// An edited outing as it's saved, with photos by path since indices only
/// last one session. See `drafts` for the file format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub category: Option<String>,
    pub accepted: bool,
    /// Relative paths, in the outing's order. Never empty.
    pub photos: Vec<String>,
}

/// What the user can do to the outings. The UI maps keys and clicks to these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortIntent {
    Rename { outing: usize, name: String },
    SetCategory { outing: usize, category: String },
    /// Accept an outing that has a category, or reopen an accepted one.
    ToggleAccepted { outing: usize },
    /// Start a new outing at this photo.
    SplitAt { photo: usize },
    /// Merge this outing into the one above it.
    MergeUp { outing: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SortPlan {
    outings: Vec<Outing>,
    /// The gap the photos were grouped by, which also decides how far from
    /// an outing a video may be and still belong to it.
    gap_seconds: i64,
}

impl SortPlan {
    /// Groups every unsorted photo in `library`. Photos with a capture time
    /// are sorted by it and split at gaps longer than `gap_seconds`. Photos
    /// without one can't be placed in time, so they stay together by folder,
    /// after the dated outings.
    pub fn group(library: &Library, gap_seconds: i64) -> Self {
        let mut dated: Vec<(CaptureTime, usize)> = Vec::new();
        let mut undated: Vec<usize> = Vec::new();
        for photo in library.unsorted() {
            match library.photo(photo).exif.captured {
                Some(time) => dated.push((time, photo)),
                None => undated.push(photo),
            }
        }
        dated.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| library.photo(a.1).relative.cmp(&library.photo(b.1).relative)));

        let mut groups: Vec<Vec<(CaptureTime, usize)>> = Vec::new();
        for (time, photo) in dated {
            match groups.last_mut() {
                Some(group) if group.last().is_some_and(|(last, _)| time.seconds() - last.seconds() <= gap_seconds) => {
                    group.push((time, photo));
                }
                _ => groups.push(vec![(time, photo)]),
            }
        }

        let starts: Vec<CaptureTime> = groups.iter().map(|g| g[0].0).collect();
        let mut outings: Vec<Outing> = groups
            .iter()
            .enumerate()
            .map(|(i, group)| Outing {
                photos: group.iter().map(|(_, photo)| *photo).collect(),
                name: suggest_name(&starts, i),
                category: None,
                accepted: false,
                edited: false,
            })
            .collect();

        for section in library.folder_sections(|i| undated.contains(&i)) {
            let name = section.title.rsplit(" / ").next().map(str::to_owned).unwrap_or_default();
            outings.push(Outing { photos: section.photos, name, category: None, accepted: false, edited: false });
        }
        Self { outings, gap_seconds }
    }

    pub fn outings(&self) -> &[Outing] {
        &self.outings
    }

    /// The unsorted videos that go with each outing, indexed like
    /// `outings`. A video joins the outing closest to it in time, as long as
    /// that's within the grouping gap. Videos without a capture time, or
    /// shot far from every outing, belong to none and stay where they are.
    pub fn videos(&self, library: &Library) -> Vec<Vec<usize>> {
        let spans: Vec<Option<(i64, i64)>> = self
            .outings
            .iter()
            .map(|o| {
                let times = o.photos.iter().filter_map(|&p| library.photo(p).exif.captured).map(CaptureTime::seconds);
                times.fold(None, |span, t| Some(span.map_or((t, t), |(start, end): (i64, i64)| (start.min(t), end.max(t)))))
            })
            .collect();
        let mut videos = vec![Vec::new(); self.outings.len()];
        for video in library.unsorted_videos() {
            let Some(time) = library.videos()[video].captured.map(CaptureTime::seconds) else { continue };
            let distance = |&(start, end): &(i64, i64)| if time < start { start - time } else { (time - end).max(0) };
            let closest = spans
                .iter()
                .enumerate()
                .filter_map(|(i, span)| span.as_ref().map(|span| (distance(span), i)))
                .min();
            if let Some((distance, outing)) = closest
                && distance <= self.gap_seconds
            {
                videos[outing].push(video);
            }
        }
        videos
    }

    pub fn outing_of(&self, photo: usize) -> Option<usize> {
        self.outings.iter().position(|o| o.photos.contains(&photo))
    }

    pub fn accepted(&self) -> impl Iterator<Item = &Outing> {
        self.outings.iter().filter(|o| o.accepted)
    }

    /// Every category set on an outing, for suggestions while typing.
    pub fn categories(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for category in self.outings.iter().filter_map(Outing::category) {
            if !seen.contains(&category) {
                seen.push(category);
            }
        }
        seen
    }

    /// Categories to offer while typing: the ones on disk, then the ones
    /// already given to outings, that start with `typed` (without case).
    pub fn category_suggestions(&self, library: &Library, typed: &str) -> Vec<String> {
        let typed = typed.trim().to_lowercase();
        let mut found: Vec<String> = Vec::new();
        let known = library.categories().into_iter().map(|(name, _)| name).chain(self.categories().into_iter().map(str::to_owned));
        for name in known {
            let lower = name.to_lowercase();
            if lower.starts_with(&typed) && !found.iter().any(|f| f.to_lowercase() == lower) {
                found.push(name);
            }
        }
        found
    }

    /// One section per outing, in plan order.
    pub fn sections(&self) -> Vec<Section> {
        self.outings.iter().map(|o| Section { title: o.name.clone(), photos: o.photos.clone() }).collect()
    }

    /// Applies one intent. Returns false if it changed nothing, such as
    /// accepting an outing that has no category or splitting at the first
    /// photo of an outing.
    pub fn apply(&mut self, intent: SortIntent) -> bool {
        match intent {
            SortIntent::Rename { outing, name } => {
                let (Some(target), Some(name)) = (self.outings.get_mut(outing), folder_name(&name)) else { return false };
                target.name = name;
                target.edited = true;
            }
            SortIntent::SetCategory { outing, category } => {
                let (Some(target), Some(category)) = (self.outings.get_mut(outing), folder_name(&category)) else {
                    return false;
                };
                target.category = Some(category);
                target.edited = true;
            }
            SortIntent::ToggleAccepted { outing } => {
                let Some(target) = self.outings.get_mut(outing) else { return false };
                if target.category.is_none() {
                    return false;
                }
                target.accepted = !target.accepted;
                target.edited = true;
            }
            SortIntent::SplitAt { photo } => {
                let Some(outing) = self.outing_of(photo) else { return false };
                let at = self.outings[outing].photos.iter().position(|&p| p == photo).unwrap_or(0);
                if at == 0 {
                    return false;
                }
                let original = &mut self.outings[outing];
                original.edited = true;
                let rest = original.photos.split_off(at);
                // The new part keeps the category, since a split usually
                // separates two parts of the same kind of day.
                let split = Outing {
                    photos: rest,
                    name: format!("{} part 2", original.name),
                    category: original.category.clone(),
                    accepted: false,
                    edited: true,
                };
                self.outings.insert(outing + 1, split);
            }
            SortIntent::MergeUp { outing } => {
                if outing == 0 || outing >= self.outings.len() {
                    return false;
                }
                let merged = self.outings.remove(outing);
                self.outings[outing - 1].photos.extend(merged.photos);
                self.outings[outing - 1].edited = true;
            }
        }
        true
    }

    /// Drops photos that left the unsorted pile (they moved). Outings left
    /// empty disappear.
    pub fn remove(&mut self, photos: &HashSet<usize>) {
        for outing in &mut self.outings {
            outing.photos.retain(|p| !photos.contains(p));
        }
        self.outings.retain(|o| !o.photos.is_empty());
    }

    /// Drops every photo that's no longer unsorted, after moves ran.
    pub fn forget_sorted(&mut self, library: &Library) {
        let sorted: HashSet<usize> = self.outings.iter().flat_map(|o| o.photos.iter().copied()).filter(|&p| library.is_sorted(p)).collect();
        self.remove(&sorted);
    }

    /// Puts photos that an undo brought back to the unsorted pile into the
    /// plan, one outing per folder they came back from, named after it.
    /// `moved` are the undo's moves (from the sorted folder back to the date
    /// folder), already applied to `library`.
    pub fn restore(&mut self, library: &Library, moved: &[FileMove]) {
        let mut by_folder: Vec<(&str, Vec<usize>)> = Vec::new();
        for m in moved {
            let Some(photo) = library.photo_at(&m.to) else { continue };
            if library.is_sorted(photo) || self.outing_of(photo).is_some() {
                continue;
            }
            let folder = m.from.rsplit_once('/').map_or("", |(folder, _)| folder);
            match by_folder.iter_mut().find(|(f, _)| *f == folder) {
                Some((_, photos)) => photos.push(photo),
                None => by_folder.push((folder, vec![photo])),
            }
        }
        for (folder, mut photos) in by_folder {
            photos.sort_by(|&a, &b| {
                let (a, b) = (library.photo(a), library.photo(b));
                a.exif.captured.cmp(&b.exif.captured).then_with(|| a.relative.cmp(&b.relative))
            });
            self.insert(Outing::restored(photos, folder), library);
        }
    }

    /// The outings the user changed, to save as drafts.
    pub fn drafts(&self, library: &Library) -> Vec<Draft> {
        self.outings
            .iter()
            .filter(|o| o.edited)
            .map(|o| Draft {
                name: o.name.clone(),
                category: o.category.clone(),
                accepted: o.accepted,
                photos: o.photos.iter().map(|&p| library.photo(p).relative.clone()).collect(),
            })
            .collect()
    }

    /// Brings back saved drafts after grouping. Each draft takes its photos
    /// out of the grouped outings. Photos that moved or are gone are
    /// skipped, and a draft left with none is dropped.
    pub fn apply_drafts(&mut self, library: &Library, drafts: &[Draft]) {
        let by_key: HashMap<String, usize> = library.photos().iter().enumerate().map(|(i, p)| (key(&p.relative), i)).collect();
        for draft in drafts {
            let photos: Vec<usize> =
                draft.photos.iter().filter_map(|relative| by_key.get(&key(relative)).copied()).filter(|&p| !library.is_sorted(p)).collect();
            if photos.is_empty() {
                continue;
            }
            self.remove(&photos.iter().copied().collect());
            let category = draft.category.clone();
            let accepted = draft.accepted && category.is_some();
            self.insert(Outing { photos, name: draft.name.clone(), category, accepted, edited: true }, library);
        }
    }

    /// Adds an outing back, before the first outing whose first photo is
    /// later, so the plan stays in time order.
    pub fn insert(&mut self, outing: Outing, library: &Library) {
        let start = |o: &Outing| o.photos.first().and_then(|&p| library.photo(p).exif.captured);
        let at = match start(&outing) {
            Some(time) => self.outings.iter().position(|o| start(o).is_none_or(|t| t > time)),
            None => None,
        };
        self.outings.insert(at.unwrap_or(self.outings.len()), outing);
    }
}

/// `2024-10-20` for an outing that has its day to itself. When a day has
/// several, each gets the part of the day too, or the start time if that
/// still doesn't tell them apart.
fn suggest_name(starts: &[CaptureTime], index: usize) -> String {
    let start = starts[index];
    let date = format!("{:04}-{:02}-{:02}", start.year, start.month, start.day);
    let same_day: Vec<&CaptureTime> =
        starts.iter().filter(|s| (s.year, s.month, s.day) == (start.year, start.month, start.day)).collect();
    if same_day.len() == 1 {
        return date;
    }
    let part = |time: &CaptureTime| match time.hour {
        0..12 => "morning",
        12..17 => "afternoon",
        _ => "evening",
    };
    let clashes = same_day.iter().filter(|s| part(s) == part(&start)).count() > 1;
    if clashes { format!("{date} {:02}.{:02}", start.hour, start.minute) } else { format!("{date} {}", part(&start)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_GAP_SECONDS: i64 = 2 * 3600;

    #[test]
    fn gap_steps_stop_at_the_ends_and_join_from_any_value() {
        assert_eq!(step_gap(120, true), Some(180));
        assert_eq!(step_gap(120, false), Some(60));
        assert_eq!(step_gap(90, false), Some(60), "an odd value steps to its neighbour");
        assert_eq!(step_gap(1440, true), None);
        assert_eq!(step_gap(15, false), None);
        assert_eq!([gap_text(30), gap_text(60), gap_text(150), gap_text(1440)], ["30 minutes", "1 hour", "2.5 hours", "24 hours"]);
    }
    use crate::library::tests::photo;

    fn library(photos: &[(&str, Option<&str>)]) -> Library {
        let photos = photos
            .iter()
            .map(|(path, time)| {
                let mut p = photo(path);
                p.exif.captured = time.and_then(CaptureTime::parse);
                p
            })
            .collect();
        Library::new("/lib".into(), photos)
    }

    fn fixture() -> Library {
        library(&[
            ("2024/2024-10-20/a.NEF", Some("2024:10:20 09:00:00")),
            ("2024/2024-10-20/b.NEF", Some("2024:10:20 10:30:00")),
            // Four hours later: a new outing on the same day.
            ("2024/2024-10-20/c.NEF", Some("2024:10:20 14:30:00")),
            ("2024/2024-10-21/d.NEF", Some("2024:10:21 08:00:00")),
            ("Events/Fair/e.NEF", Some("2024:10:20 09:10:00")),
            ("2024/2024-10-22/f.NEF", None),
        ])
    }

    fn photos(plan: &SortPlan) -> Vec<Vec<usize>> {
        plan.outings().iter().map(|o| o.photos().to_vec()).collect()
    }

    #[test]
    fn unsorted_photos_split_at_time_gaps_and_get_dated_names() {
        let plan = SortPlan::group(&fixture(), DEFAULT_GAP_SECONDS);
        assert_eq!(photos(&plan), vec![vec![0, 1], vec![2], vec![3], vec![5]], "sorted photo 4 is left alone");
        let names: Vec<&str> = plan.outings().iter().map(Outing::name).collect();
        assert_eq!(names, ["2024-10-20 morning", "2024-10-20 afternoon", "2024-10-21", "2024-10-22"]);
    }

    #[test]
    fn two_outings_in_the_same_part_of_a_day_are_told_apart_by_time() {
        let lib = library(&[("a.NEF", Some("2024:10:20 07:00:00")), ("b.NEF", Some("2024:10:20 10:00:00"))]);
        let plan = SortPlan::group(&lib, DEFAULT_GAP_SECONDS);
        let names: Vec<&str> = plan.outings().iter().map(Outing::name).collect();
        assert_eq!(names, ["2024-10-20 07.00", "2024-10-20 10.00"]);
    }

    #[test]
    fn only_an_outing_with_a_category_can_be_accepted() {
        let mut plan = SortPlan::group(&fixture(), DEFAULT_GAP_SECONDS);
        assert!(!plan.apply(SortIntent::ToggleAccepted { outing: 0 }));
        assert!(plan.apply(SortIntent::SetCategory { outing: 0, category: " Events ".into() }));
        assert!(plan.apply(SortIntent::Rename { outing: 0, name: "Fair".into() }));
        assert!(plan.apply(SortIntent::ToggleAccepted { outing: 0 }));
        assert_eq!(plan.outings()[0].folder().as_deref(), Some("Events/Fair"));
        assert_eq!(plan.accepted().count(), 1);

        assert!(plan.apply(SortIntent::ToggleAccepted { outing: 0 }), "accepting again reopens");
        assert_eq!(plan.accepted().count(), 0);
        assert!(!plan.apply(SortIntent::Rename { outing: 0, name: " / ".replace('/', "") }), "blank names are refused");
    }

    #[test]
    fn category_suggestions_come_from_disk_and_the_plan() {
        let lib = fixture();
        let mut plan = SortPlan::group(&lib, DEFAULT_GAP_SECONDS);
        plan.apply(SortIntent::SetCategory { outing: 0, category: "Nature".into() });
        plan.apply(SortIntent::SetCategory { outing: 1, category: "events".into() });
        assert_eq!(plan.category_suggestions(&lib, ""), ["Events", "Nature"]);
        assert_eq!(plan.category_suggestions(&lib, "na"), ["Nature"]);
        assert!(plan.category_suggestions(&lib, "x").is_empty());
    }

    #[test]
    fn split_and_merge_are_inverses() {
        let mut plan = SortPlan::group(&fixture(), DEFAULT_GAP_SECONDS);
        plan.apply(SortIntent::SetCategory { outing: 0, category: "Events".into() });
        assert!(!plan.apply(SortIntent::SplitAt { photo: 0 }), "can't split before the first photo");
        assert!(plan.apply(SortIntent::SplitAt { photo: 1 }));
        assert_eq!(photos(&plan)[..2], [vec![0], vec![1]]);
        assert_eq!(plan.outings()[1].category(), Some("Events"));

        assert!(plan.apply(SortIntent::MergeUp { outing: 1 }));
        assert_eq!(photos(&plan)[0], vec![0, 1]);
        assert!(!plan.apply(SortIntent::MergeUp { outing: 0 }), "nothing above the first outing");
    }

    #[test]
    fn drafts_bring_back_edited_outings_after_a_reopen() {
        let lib = fixture();
        let mut plan = SortPlan::group(&lib, DEFAULT_GAP_SECONDS);
        plan.apply(SortIntent::Rename { outing: 0, name: "Walk".into() });
        plan.apply(SortIntent::SetCategory { outing: 0, category: "Nature".into() });
        plan.apply(SortIntent::ToggleAccepted { outing: 0 });
        plan.apply(SortIntent::MergeUp { outing: 2 });
        let drafts = plan.drafts(&lib);
        assert_eq!(drafts.len(), 2, "only edited outings are drafts");
        assert_eq!(drafts[0].photos, ["2024/2024-10-20/a.NEF", "2024/2024-10-20/b.NEF"]);

        let mut reopened = SortPlan::group(&lib, DEFAULT_GAP_SECONDS);
        reopened.apply_drafts(&lib, &drafts);
        assert_eq!(reopened, plan);
    }

    #[test]
    fn moved_photos_leave_and_restored_ones_return_in_time_order() {
        let lib = fixture();
        let mut plan = SortPlan::group(&lib, DEFAULT_GAP_SECONDS);
        plan.remove(&HashSet::from([2]));
        assert_eq!(photos(&plan), vec![vec![0, 1], vec![3], vec![5]]);

        plan.insert(Outing::restored(vec![2], "Events/Market"), &lib);
        assert_eq!(photos(&plan), vec![vec![0, 1], vec![2], vec![3], vec![5]]);
        assert_eq!(plan.outings()[1].folder().as_deref(), Some("Events/Market"));
        assert!(!plan.outings()[1].is_accepted());
    }

    #[test]
    fn a_move_and_its_undo_leave_and_restore_the_outing() {
        let mut lib = fixture();
        let mut plan = SortPlan::group(&lib, DEFAULT_GAP_SECONDS);
        let there = [
            FileMove { from: "2024/2024-10-20/a.NEF".into(), to: "Events/Fair/a.NEF".into() },
            FileMove { from: "2024/2024-10-20/b.NEF".into(), to: "Events/Fair/b.NEF".into() },
        ];
        lib.apply_moves(&there);
        plan.forget_sorted(&lib);
        assert_eq!(photos(&plan), vec![vec![2], vec![3], vec![5]]);

        let back: Vec<FileMove> = there.iter().rev().map(|m| FileMove { from: m.to.clone(), to: m.from.clone() }).collect();
        lib.apply_moves(&back);
        plan.restore(&lib, &back);
        assert_eq!(photos(&plan), vec![vec![0, 1], vec![2], vec![3], vec![5]]);
        assert_eq!(plan.outings()[0].folder().as_deref(), Some("Events/Fair"));
    }
}
