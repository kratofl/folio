//! Saved views: a named filter, one small text file each in `.folio/views/`,
//! so editing one view never touches another's file.
//!
//! ```text
//! folio view 1
//! name<TAB>Best of October
//! rating<TAB>>=3
//! flag<TAB>pick
//! lens<TAB>NIKKOR Z DX 16-50mm f/3.5-6.3 VR
//! ```
//!
//! Lines Folio doesn't understand are skipped, so a newer Folio's views still
//! open. When file sync leaves two files with the same view name (a conflict
//! copy such as `Best of October 2.txt`), the newer one wins and the other is
//! removed. A view is one value, its list of conditions, so there's nothing
//! finer to merge.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::culling::{Flag, Label, Stars};
use crate::filter::{Condition, DateSpan, Filter, Rating};
use crate::library::folder_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedView {
    pub name: String,
    pub filter: Filter,
}

const HEADER: &str = "folio view 1";

pub fn to_text(view: &SavedView) -> String {
    let mut text = format!("{HEADER}\nname\t{}\n", one_line(&view.name));
    for condition in view.filter.conditions() {
        let (field, value) = match condition {
            Condition::Rating(Rating::AtLeast(stars)) => ("rating", format!(">={}", stars.count())),
            Condition::Rating(Rating::Exactly(stars)) => ("rating", format!("={}", stars.count())),
            Condition::Rating(Rating::Unrated) => ("rating", "none".into()),
            Condition::Flag(Some(Flag::Pick)) => ("flag", "pick".into()),
            Condition::Flag(Some(Flag::Reject)) => ("flag", "reject".into()),
            Condition::Flag(None) => ("flag", "none".into()),
            Condition::Label(label) => ("label", label.map_or("none", Label::name).into()),
            Condition::Camera(camera) => ("camera", one_line(camera)),
            Condition::Lens(lens) => ("lens", one_line(lens)),
            Condition::Folder(folder) => ("folder", one_line(folder)),
            Condition::Date(DateSpan::Year(year)) => ("date", year.to_string()),
            Condition::Date(DateSpan::Month(year, month)) => ("date", format!("{year}-{month:02}")),
        };
        text.push_str(&format!("{field}\t{value}\n"));
    }
    text
}

/// Reads a view file. `None` if it isn't one.
pub fn from_text(text: &str) -> Option<SavedView> {
    let mut lines = text.lines();
    if lines.next()?.trim() != HEADER {
        return None;
    }
    let mut name = None;
    let mut conditions = Vec::new();
    for line in lines {
        let Some((field, value)) = line.split_once('\t') else { continue };
        if field == "name" {
            name = Some(value.to_owned());
        } else if let Some(condition) = parse_condition(field, value) {
            conditions.push(condition);
        }
    }
    Some(SavedView { name: name?, filter: Filter::new(conditions) })
}

fn parse_condition(field: &str, value: &str) -> Option<Condition> {
    let stars = |v: &str| v.parse::<i64>().ok().and_then(Stars::from_count);
    Some(match (field, value) {
        ("rating", "none") => Condition::Rating(Rating::Unrated),
        ("rating", v) if v.starts_with(">=") => Condition::Rating(Rating::AtLeast(stars(&v[2..])?)),
        ("rating", v) if v.starts_with('=') => Condition::Rating(Rating::Exactly(stars(&v[1..])?)),
        ("flag", "pick") => Condition::Flag(Some(Flag::Pick)),
        ("flag", "reject") => Condition::Flag(Some(Flag::Reject)),
        ("flag", "none") => Condition::Flag(None),
        ("label", "none") => Condition::Label(None),
        ("label", v) => Condition::Label(Some(Label::from_name(v)?)),
        ("camera", v) => Condition::Camera(v.to_owned()),
        ("lens", v) => Condition::Lens(v.to_owned()),
        ("folder", v) => Condition::Folder(v.to_owned()),
        ("date", v) => match v.split_once('-') {
            Some((year, month)) => Condition::Date(DateSpan::Month(year.parse().ok()?, month.parse().ok()?)),
            None => Condition::Date(DateSpan::Year(v.parse().ok()?)),
        },
        _ => return None,
    })
}

fn one_line(text: &str) -> String {
    text.replace(['\t', '\n', '\r'], " ")
}

fn views_dir(root: &Path) -> PathBuf {
    root.join(".folio").join("views")
}

/// Every saved view, sorted by name, plus a message for each file that
/// couldn't be read. Conflict copies are merged here (see the module docs).
pub fn load_views(root: &Path) -> (Vec<SavedView>, Vec<String>) {
    let mut problems = Vec::new();
    let Ok(entries) = fs::read_dir(views_dir(root)) else { return (Vec::new(), problems) };

    let mut found: Vec<(SavedView, PathBuf, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_view = path.extension().is_some_and(|e| e == "txt") && !entry.file_name().to_string_lossy().starts_with('.');
        if !is_view {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        match fs::read_to_string(&path).ok().as_deref().and_then(from_text) {
            Some(view) => found.push((view, path, modified)),
            None => problems.push(format!("Couldn't read the saved view {}", entry.file_name().to_string_lossy())),
        }
    }

    // Newest first, so the first of each name is the one to keep.
    found.sort_by_key(|f| std::cmp::Reverse(f.2));
    let mut views: Vec<SavedView> = Vec::new();
    for (view, path, _) in found {
        if views.iter().any(|v| v.name.to_lowercase() == view.name.to_lowercase()) {
            if let Err(error) = fs::remove_file(&path) {
                problems.push(format!("Couldn't remove the older copy {}: {error}", path.display()));
            }
        } else {
            views.push(view);
        }
    }
    views.sort_by_key(|v| v.name.to_lowercase());
    (views, problems)
}

/// Saves a view, replacing any view with the same name.
pub fn save_view(root: &Path, view: &SavedView) -> io::Result<()> {
    let dir = views_dir(root);
    fs::create_dir_all(&dir)?;
    remove_named(&dir, &view.name, Some(&file_for(&view.name)))?;
    let path = dir.join(file_for(&view.name));
    let temporary = dir.join(format!(".{}.folio-tmp", file_for(&view.name)));
    fs::write(&temporary, to_text(view))?;
    fs::rename(&temporary, path)
}

pub fn delete_view(root: &Path, name: &str) -> io::Result<()> {
    remove_named(&views_dir(root), name, None)
}

/// `Best of October.txt`, safe on macOS and Windows.
fn file_for(name: &str) -> String {
    format!("{}.txt", folder_name(name).unwrap_or_else(|| "view".into()))
}

/// Removes every file holding a view called `name`, except `keep`.
fn remove_named(dir: &Path, name: &str, keep: Option<&str>) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(dir) else { return Ok(()) };
    for entry in entries.flatten() {
        if keep.is_some_and(|k| entry.file_name().to_string_lossy() == k) {
            continue;
        }
        let same = fs::read_to_string(entry.path()).ok().as_deref().and_then(from_text).is_some_and(|v| v.name.eq_ignore_ascii_case(name));
        if same {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(name: &str) -> SavedView {
        SavedView {
            name: name.into(),
            filter: Filter::new(vec![
                Condition::Rating(Rating::AtLeast(Stars::Three)),
                Condition::Flag(Some(Flag::Pick)),
                Condition::Label(Some(Label::Green)),
                Condition::Lens("NIKKOR Z DX 16-50mm".into()),
                Condition::Folder("Home/Fair".into()),
                Condition::Date(DateSpan::Month(2024, 10)),
                Condition::Date(DateSpan::Year(2023)),
            ]),
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("folio-views-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn views_round_trip_through_text() {
        let original = view("Best of October");
        assert_eq!(from_text(&to_text(&original)), Some(original));
    }

    #[test]
    fn unknown_lines_are_skipped_and_damage_is_refused() {
        let text = "folio view 1\nname\tA\nrating\t>=4\nsharpness\thigh\n";
        let read = from_text(text).unwrap();
        assert_eq!(read.filter.conditions(), &[Condition::Rating(Rating::AtLeast(Stars::Four))]);
        assert_eq!(from_text("garbage"), None);
        assert_eq!(from_text("folio view 1\nrating\t>=4\n"), None, "a view needs a name");
    }

    #[test]
    fn saving_replaces_and_deleting_removes() {
        let root = TempDir::new("save");
        save_view(&root.0, &view("Best")).unwrap();
        let mut changed = view("best");
        changed.filter = Filter::new(vec![Condition::Flag(None)]);
        save_view(&root.0, &changed).unwrap();

        let (views, problems) = load_views(&root.0);
        assert!(problems.is_empty());
        assert_eq!(views, vec![changed]);

        delete_view(&root.0, "BEST").unwrap();
        assert!(load_views(&root.0).0.is_empty());
    }

    #[test]
    fn conflict_copies_merge_to_the_newest_and_corrupt_files_are_reported() {
        let root = TempDir::new("conflict");
        let dir = views_dir(&root.0);
        fs::create_dir_all(&dir).unwrap();
        let older = view("Best");
        let mut newer = view("Best");
        newer.filter = Filter::new(vec![Condition::Flag(Some(Flag::Reject))]);
        fs::write(dir.join("Best.txt"), to_text(&older)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join("Best 2.txt"), to_text(&newer)).unwrap();
        fs::write(dir.join("Broken.txt"), "not a view").unwrap();

        let (views, problems) = load_views(&root.0);
        assert_eq!(views, vec![newer]);
        assert_eq!(problems, vec!["Couldn't read the saved view Broken.txt".to_owned()]);
        assert!(!dir.join("Best.txt").exists(), "the older copy is gone once merged");
    }
}
