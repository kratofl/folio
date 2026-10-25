//! Outing drafts: outings the user named, gave a category, accepted, split
//! or merged, kept in `.folio/outings/` until their photos move. Without
//! them that work would be lost on quit. Untouched outings aren't saved,
//! since grouping makes them again.
//!
//! One small file per outing, named after its first photo, so two machines
//! editing different outings never touch the same file:
//!
//! ```text
//! folio outing 1
//! name<TAB>Christmas market
//! category<TAB>Events
//! accepted
//! photo<TAB>2024/2024-12-07/_DSC0001.NEF
//! photo<TAB>2024/2024-12-07/_DSC0002.NEF
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::library::key;
use crate::outings::Draft;
use crate::thumbnail_cache::fnv1a;

const HEADER: &str = "folio outing 1";

pub fn to_text(draft: &Draft) -> String {
    let mut text = format!("{HEADER}\nname\t{}\n", draft.name);
    if let Some(category) = &draft.category {
        text.push_str(&format!("category\t{category}\n"));
    }
    if draft.accepted {
        text.push_str("accepted\n");
    }
    for photo in &draft.photos {
        text.push_str(&format!("photo\t{photo}\n"));
    }
    text
}

/// `None` unless it's a draft with a name and at least one photo. Unknown
/// lines are skipped, so a draft from a newer Folio still opens.
pub fn from_text(text: &str) -> Option<Draft> {
    let mut lines = text.lines();
    if lines.next()?.trim() != HEADER {
        return None;
    }
    let mut draft = Draft { name: String::new(), category: None, accepted: false, photos: Vec::new() };
    for line in lines {
        match line.split_once('\t') {
            Some(("name", name)) => draft.name = name.to_owned(),
            Some(("category", category)) => draft.category = Some(category.to_owned()),
            Some(("photo", photo)) => draft.photos.push(photo.to_owned()),
            None if line == "accepted" => draft.accepted = true,
            _ => {}
        }
    }
    (!draft.name.is_empty() && !draft.photos.is_empty()).then_some(draft)
}

/// Every draft, plus a message for each file that couldn't be read. Two
/// files for the same first photo are a sync conflict copy: the newer one
/// wins and the older is removed. A draft is one decision, the outing as
/// the user last left it, so there's nothing finer to merge.
pub fn load_drafts(root: &Path) -> (Vec<Draft>, Vec<String>) {
    let mut problems = Vec::new();
    let Ok(entries) = fs::read_dir(drafts_dir(root)) else { return (Vec::new(), problems) };

    let mut found: Vec<(Draft, PathBuf, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "txt") || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        match fs::read_to_string(&path).ok().as_deref().and_then(from_text) {
            Some(draft) => found.push((draft, path, modified)),
            None => problems.push(format!("Couldn't read the outing draft {}", entry.file_name().to_string_lossy())),
        }
    }

    // Newest first, so the first draft for each photo is the one to keep.
    found.sort_by_key(|f| std::cmp::Reverse(f.2));
    let mut drafts: Vec<Draft> = Vec::new();
    for (draft, path, _) in found {
        if drafts.iter().any(|d| file_for(d) == file_for(&draft)) {
            if let Err(error) = fs::remove_file(&path) {
                problems.push(format!("Couldn't remove the older copy {}: {error}", path.display()));
            }
        } else {
            drafts.push(draft);
        }
    }
    (drafts, problems)
}

/// Writes the drafts that changed since `previous` and removes the files of
/// drafts that are gone, such as outings whose photos moved. Files this
/// session never knew about are left alone.
pub fn save_drafts(root: &Path, previous: &[Draft], current: &[Draft]) -> io::Result<()> {
    let dir = drafts_dir(root);
    let (write, remove) = changes(previous, current);
    if !write.is_empty() {
        fs::create_dir_all(&dir)?;
    }
    for draft in write {
        let name = file_for(draft);
        let temporary = dir.join(format!(".{name}.folio-tmp"));
        fs::write(&temporary, to_text(draft))?;
        fs::rename(&temporary, dir.join(name))?;
    }
    for name in remove {
        match fs::remove_file(dir.join(name)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// Which drafts to write and which files to remove. Pure.
fn changes<'a>(previous: &[Draft], current: &'a [Draft]) -> (Vec<&'a Draft>, Vec<String>) {
    let write = current.iter().filter(|d| !previous.contains(d)).collect();
    let kept: Vec<String> = current.iter().map(file_for).collect();
    let mut remove: Vec<String> = previous.iter().map(file_for).filter(|f| !kept.contains(f)).collect();
    remove.dedup();
    (write, remove)
}

/// Named after the first photo's path, hashed so any name is a safe file
/// name, and matched without case or Unicode form like every path.
fn file_for(draft: &Draft) -> String {
    let first = draft.photos.first().map(String::as_str).unwrap_or_default();
    format!("{:016x}.txt", fnv1a(key(first).as_bytes()))
}

fn drafts_dir(root: &Path) -> PathBuf {
    root.join(".folio").join("outings")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str, photos: &[&str]) -> Draft {
        Draft { name: name.into(), category: Some("Events".into()), accepted: true, photos: photos.iter().map(|p| (*p).into()).collect() }
    }

    #[test]
    fn drafts_round_trip_through_text() {
        let market = draft("Christmas market", &["2024/2024-12-07/_DSC0001.NEF", "2024/2024-12-07/_DSC0002.NEF"]);
        assert_eq!(from_text(&to_text(&market)), Some(market.clone()));
        let newer = format!("{}colour\tred\n", to_text(&market));
        assert_eq!(from_text(&newer), Some(market), "unknown lines are skipped");
        assert_eq!(from_text("folio outing 1\nname\tEmpty\n"), None);
        assert_eq!(from_text("folio view 1\n"), None);
    }

    #[test]
    fn only_changed_drafts_are_written_and_only_known_ones_removed() {
        let a = draft("A", &["2024/a.NEF"]);
        let b = draft("B", &["2024/b.NEF"]);
        let renamed = draft("B renamed", &["2024/b.NEF"]);
        let current = [a.clone(), renamed.clone()];
        let (write, remove) = changes(&[a.clone(), b.clone()], &current);
        assert_eq!(write, vec![&renamed]);
        assert!(remove.is_empty(), "the renamed draft replaces its own file");

        let (write, remove) = changes(&[a.clone(), b.clone()], std::slice::from_ref(&b));
        assert!(write.is_empty());
        assert_eq!(remove, vec![file_for(&a)]);
    }

    #[test]
    fn a_conflict_copy_loses_to_the_newer_draft() {
        let root = std::env::temp_dir().join(format!("folio-drafts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let old = draft("Old name", &["2024/a.NEF"]);
        let new = draft("New name", &["2024/A.nef"]);
        save_drafts(&root, &[], std::slice::from_ref(&old)).unwrap();
        // A sync service kept the other machine's version next to it.
        let copy = drafts_dir(&root).join(file_for(&old).replace(".txt", " 2.txt"));
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&copy, to_text(&new)).unwrap();

        let (drafts, problems) = load_drafts(&root);
        let remaining = fs::read_dir(drafts_dir(&root)).unwrap().count();
        let _ = fs::remove_dir_all(&root);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(drafts, vec![new]);
        assert_eq!(remaining, 1);
    }
}
