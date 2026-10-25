//! Library settings, in `.folio/settings.txt`. They belong to the library,
//! not the machine, so every machine groups the same photos the same way.
//! Today that's only the gap that starts a new outing.
//!
//! A conflict copy (`settings 2.txt`) merges per value: each setting comes
//! from the newest file that has it. Saving re-reads the file and changes
//! only the one line, so a value another machine wrote isn't overwritten
//! from stale memory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::conflicts::conflict_original;

const HEADER: &str = "folio settings 1";
const OUTING_GAP: &str = "outing-gap";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Minutes without photos that start a new outing.
    pub outing_gap_minutes: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { outing_gap_minutes: 120 }
    }
}

/// The `key<TAB>value` lines of a settings file. `None` if it isn't one.
fn parse(text: &str) -> Option<Vec<(&str, &str)>> {
    let mut lines = text.lines();
    if lines.next()?.trim() != HEADER {
        return None;
    }
    Some(lines.filter_map(|line| line.split_once('\t')).map(|(key, value)| (key.trim(), value.trim())).collect())
}

/// Settings from files read newest first. Each value comes from the first
/// file that has a valid one; unknown keys are skipped, so a file from a
/// newer Folio still works.
fn merge(files: &[Vec<(&str, &str)>]) -> Settings {
    let find = |key: &str| files.iter().flat_map(|lines| lines.iter()).find(|(k, _)| *k == key).map(|(_, v)| *v);
    let mut settings = Settings::default();
    if let Some(minutes) = find(OUTING_GAP).and_then(|v| v.parse::<i64>().ok()).filter(|&m| m > 0) {
        settings.outing_gap_minutes = minutes;
    }
    settings
}

fn settings_path(root: &Path) -> PathBuf {
    root.join(".folio").join("settings.txt")
}

/// The library's settings, plus a message for each copy that couldn't be
/// read. Conflict copies are merged into `settings.txt` and removed.
pub fn load_settings(root: &Path) -> (Settings, Vec<String>) {
    let mut problems = Vec::new();
    let Ok(entries) = fs::read_dir(root.join(".folio")) else { return (Settings::default(), problems) };
    let mut found: Vec<(PathBuf, SystemTime, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
        let is_settings = path.extension().is_some_and(|e| e == "txt") && (stem == "settings" || conflict_original(&stem) == Some("settings"));
        if !is_settings {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        match fs::read_to_string(&path) {
            Ok(text) if parse(&text).is_some() => found.push((path, modified, text)),
            _ => problems.push(format!("Couldn't read the library settings in {}", entry.file_name().to_string_lossy())),
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.1));
    let files: Vec<Vec<(&str, &str)>> = found.iter().filter_map(|f| parse(&f.2)).collect();
    let settings = merge(&files);

    let main = settings_path(root);
    let copies: Vec<&PathBuf> = found.iter().map(|f| &f.0).filter(|p| **p != main).collect();
    if !copies.is_empty() {
        match save_settings(root, &settings) {
            Ok(()) => {
                for copy in copies {
                    if let Err(error) = fs::remove_file(copy) {
                        problems.push(format!("Couldn't remove the settings copy {}: {error}", copy.display()));
                    }
                }
            }
            Err(error) => problems.push(format!("Couldn't merge the library settings: {error}")),
        }
    }
    (settings, problems)
}

/// Writes the settings, keeping lines this Folio doesn't know.
pub fn save_settings(root: &Path, settings: &Settings) -> io::Result<()> {
    let path = settings_path(root);
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = vec![HEADER.to_owned()];
    if let Some(parsed) = parse(&existing) {
        lines.extend(parsed.into_iter().filter(|(key, _)| *key != OUTING_GAP).map(|(key, value)| format!("{key}\t{value}")));
    }
    lines.push(format!("{OUTING_GAP}\t{}", settings.outing_gap_minutes));
    let dir = root.join(".folio");
    fs::create_dir_all(&dir)?;
    let temporary = dir.join(".settings.txt.folio-tmp");
    fs::write(&temporary, lines.join("\n") + "\n")?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("folio-settings-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join(".folio")).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_at(path: &Path, text: &str, age: u64) {
        fs::write(path, text).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age)).unwrap();
    }

    #[test]
    fn missing_or_unknown_values_fall_back_to_two_hours() {
        assert_eq!(merge(&[]), Settings::default());
        let text = "folio settings 1\ntheme\tdark\nouting-gap\tsoon\n";
        assert_eq!(merge(&[parse(text).unwrap()]).outing_gap_minutes, 120);
        assert_eq!(parse("not settings"), None);
    }

    #[test]
    fn saving_keeps_lines_from_a_newer_folio() {
        let root = TempDir::new("keep");
        fs::write(settings_path(&root.0), "folio settings 1\ntheme\tdark\nouting-gap\t60\n").unwrap();
        save_settings(&root.0, &Settings { outing_gap_minutes: 240 }).unwrap();
        let text = fs::read_to_string(settings_path(&root.0)).unwrap();
        assert_eq!(text, "folio settings 1\ntheme\tdark\nouting-gap\t240\n");
    }

    #[test]
    fn conflict_copies_merge_to_the_newest_value_and_corrupt_ones_are_reported() {
        let root = TempDir::new("conflict");
        let folio = root.0.join(".folio");
        write_at(&folio.join("settings.txt"), "folio settings 1\nouting-gap\t60\n", 100);
        write_at(&folio.join("settings 2.txt"), "folio settings 1\nouting-gap\t240\n", 10);
        write_at(&folio.join("settings (1).txt"), "garbage", 5);
        let (settings, problems) = load_settings(&root.0);
        assert_eq!(settings.outing_gap_minutes, 240, "the newer copy wins");
        assert!(!folio.join("settings 2.txt").exists(), "the merged copy is removed");
        assert!(folio.join("settings (1).txt").exists(), "the unreadable copy is left alone");
        assert_eq!(problems.len(), 1);
        assert_eq!(load_settings(&root.0).0.outing_gap_minutes, 240, "the merge was saved");
    }
}
