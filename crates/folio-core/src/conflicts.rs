//! Sync conflict copies. When two machines change the same file while
//! offline, sync services keep both: iCloud as `name 2.xmp`, Dropbox and
//! Nextcloud as `name (conflicted copy ...).xmp`, Google Drive as
//! `name (1).xmp`, Syncthing as `name.sync-conflict-....xmp`. Folio merges
//! them without asking and only speaks up when a copy can't be merged.
//!
//! Sidecars merge per mark. A mark set in only one copy is kept, so a rating
//! from one machine and a label from the other both survive. A mark set to
//! different values in both takes the value from the newer file. Anything
//! else differing between the copies (Lightroom edits, say) means Folio
//! can't merge without losing someone's work, so both files stay and the
//! user hears about it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::culling::Marks;
use crate::xmp::{read_marks, without_marks, write_marks};

/// The name a conflict copy was copied from: `_DSC0001` for
/// `_DSC0001 2` or `_DSC0001 (conflicted copy 2024-10-20)`. Takes a file
/// stem, without the extension. `None` if the name doesn't look like a copy.
pub fn conflict_original(stem: &str) -> Option<&str> {
    if let Some((original, _)) = stem.split_once(".sync-conflict-") {
        return Some(original);
    }
    if let Some(inner) = stem.strip_suffix(')')
        && let Some(open) = inner.rfind(" (")
    {
        let note = &inner[open + 2..];
        let is_copy = note.to_lowercase().contains("conflict") || (!note.is_empty() && note.bytes().all(|b| b.is_ascii_digit()));
        return is_copy.then(|| &inner[..open]).filter(|o| !o.is_empty());
    }
    let (original, number) = stem.rsplit_once(' ')?;
    (!original.is_empty() && !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then_some(original)
}

/// Merges two sets of marks, mark by mark. `newer` says which side wins when
/// both set the same mark to different values.
pub fn merge_marks(original: Marks, copy: Marks, newer: Newer) -> Marks {
    fn pick<T: PartialEq>(original: Option<T>, copy: Option<T>, newer: Newer) -> Option<T> {
        match (original, copy) {
            (Some(o), Some(c)) if o != c => Some(if newer == Newer::Copy { c } else { o }),
            (o, c) => o.or(c),
        }
    }
    Marks {
        stars: pick(original.stars, copy.stars, newer),
        label: pick(original.label, copy.label, newer),
        flag: pick(original.flag, copy.flag, newer),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newer {
    Original,
    Copy,
}

/// Merges a photo's sidecar conflict copies into its sidecar and removes the
/// copies that merged. If the sidecar itself is missing, the newest copy
/// takes its place. Returns where the sidecar is now, and a message for each
/// copy that couldn't be merged.
pub fn merge_sidecar_copies(photo: &Path, sidecar: Option<PathBuf>, copies: &[PathBuf]) -> (Option<PathBuf>, Vec<String>) {
    let mut problems = Vec::new();
    let mut copies: Vec<&PathBuf> = copies.iter().collect();
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    let sidecar = match sidecar {
        Some(sidecar) => sidecar,
        None => {
            // Newest first, so the newest copy becomes the sidecar.
            copies.sort_by_key(|c| std::cmp::Reverse(modified(c)));
            let Some(newest) = copies.first().copied() else { return (None, problems) };
            let target = photo.with_extension("xmp");
            if let Err(error) = fs::rename(newest, &target) {
                problems.push(format!("Couldn't rename {} to {}: {error}", name(newest), name(&target)));
                return (None, problems);
            }
            copies.remove(0);
            target
        }
    };

    for copy in copies {
        if let Err(problem) = merge_one(&sidecar, copy) {
            problems.push(format!("{} and {} {problem}", name(&sidecar), name(copy)));
        }
    }
    (Some(sidecar), problems)
}

/// Merges one copy into the sidecar, then removes the copy. The sidecar is
/// written before the copy goes, so a crash in between just merges again
/// next time.
fn merge_one(sidecar: &Path, copy: &Path) -> Result<(), String> {
    let original_text = fs::read_to_string(sidecar).map_err(|e| format!("couldn't be read: {e}"))?;
    let copy_text = fs::read_to_string(copy).map_err(|e| format!("couldn't be read: {e}"))?;
    if without_marks(&original_text) != without_marks(&copy_text) {
        return Err("differ in more than ratings, labels and flags, so both are kept".into());
    }
    let newer = if modified(copy) > modified(sidecar) { Newer::Copy } else { Newer::Original };
    let merged = merge_marks(read_marks(&original_text), read_marks(&copy_text), newer);
    if merged != read_marks(&original_text) {
        write_marks(sidecar, merged).map_err(|e| format!("couldn't be merged: {e}"))?;
    }
    fs::remove_file(copy).map_err(|e| format!("merged, but the copy couldn't be removed: {e}"))
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::culling::{Flag, Label, Stars};
    use crate::xmp::with_marks;

    #[test]
    fn recognises_the_copy_names_sync_services_use() {
        assert_eq!(conflict_original("_DSC0001 2"), Some("_DSC0001"));
        assert_eq!(conflict_original("_DSC0001 (conflicted copy 2024-10-20 143022)"), Some("_DSC0001"));
        assert_eq!(conflict_original("_DSC0001 (MacBook's conflicted copy 2024-10-20)"), Some("_DSC0001"));
        assert_eq!(conflict_original("_DSC0001 (1)"), Some("_DSC0001"));
        assert_eq!(conflict_original("_DSC0001.sync-conflict-20241020-143022-ABCDEFG"), Some("_DSC0001"));
        assert_eq!(conflict_original("Best of October 2"), Some("Best of October"));

        assert_eq!(conflict_original("_DSC0001"), None);
        assert_eq!(conflict_original("_DSC0001-2"), None);
        assert_eq!(conflict_original("Trip (Italy)"), None);
        assert_eq!(conflict_original(" 2"), None);
    }

    #[test]
    fn marks_merge_one_by_one_and_the_newer_file_breaks_ties() {
        let rated = Marks { stars: Some(Stars::Four), label: None, flag: None };
        let labelled = Marks { stars: None, label: Some(Label::Green), flag: Some(Flag::Pick) };
        let both = Marks { stars: Some(Stars::Four), label: Some(Label::Green), flag: Some(Flag::Pick) };
        assert_eq!(merge_marks(rated, labelled, Newer::Original), both);

        let two = Marks { stars: Some(Stars::Two), ..Marks::default() };
        assert_eq!(merge_marks(rated, two, Newer::Copy).stars, Some(Stars::Two));
        assert_eq!(merge_marks(rated, two, Newer::Original).stars, Some(Stars::Four));
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("folio-conflicts-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const LIGHTROOM: &str = "<x:xmpmeta><rdf:RDF><rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" crs:Exposure2012=\"-0.44\"/></rdf:RDF></x:xmpmeta>";

    fn sidecar(marks: Marks, exposure: &str) -> String {
        with_marks(Some(&LIGHTROOM.replace("-0.44", exposure)), marks).unwrap()
    }

    #[test]
    fn copies_that_differ_only_in_marks_merge_into_the_sidecar() {
        let dir = TempDir::new("merge");
        let (photo, original, copy) = (dir.0.join("_DSC1.NEF"), dir.0.join("_DSC1.xmp"), dir.0.join("_DSC1 2.xmp"));
        fs::write(&original, sidecar(Marks { stars: Some(Stars::Four), ..Marks::default() }, "-0.44")).unwrap();
        fs::write(&copy, sidecar(Marks { label: Some(Label::Red), ..Marks::default() }, "-0.44")).unwrap();

        let (now, problems) = merge_sidecar_copies(&photo, Some(original.clone()), std::slice::from_ref(&copy));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(now.as_deref(), Some(original.as_path()));
        assert!(!copy.exists());
        let merged = read_marks(&fs::read_to_string(&original).unwrap());
        assert_eq!(merged, Marks { stars: Some(Stars::Four), label: Some(Label::Red), flag: None });
    }

    #[test]
    fn copies_with_other_edits_are_kept_and_reported() {
        let dir = TempDir::new("keep");
        let (photo, original, copy) = (dir.0.join("_DSC1.NEF"), dir.0.join("_DSC1.xmp"), dir.0.join("_DSC1 2.xmp"));
        fs::write(&original, sidecar(Marks::default(), "-0.44")).unwrap();
        fs::write(&copy, sidecar(Marks::default(), "+1.00")).unwrap();

        let (_, problems) = merge_sidecar_copies(&photo, Some(original.clone()), std::slice::from_ref(&copy));
        assert_eq!(problems, vec!["_DSC1.xmp and _DSC1 2.xmp differ in more than ratings, labels and flags, so both are kept"]);
        assert!(copy.exists() && original.exists());
    }

    #[test]
    fn a_lone_copy_becomes_the_sidecar() {
        let dir = TempDir::new("lone");
        let (photo, copy) = (dir.0.join("_DSC1.NEF"), dir.0.join("_DSC1 2.xmp"));
        fs::write(&copy, sidecar(Marks { stars: Some(Stars::Five), ..Marks::default() }, "-0.44")).unwrap();

        let (now, problems) = merge_sidecar_copies(&photo, None, std::slice::from_ref(&copy));
        assert!(problems.is_empty());
        assert_eq!(now, Some(dir.0.join("_DSC1.xmp")));
        assert!(!copy.exists());
    }
}
