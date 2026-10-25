//! Moving photos into their folders, with undo that survives a crash.
//!
//! `plan` is pure: accepted outings in, the exact list of file moves out,
//! sidecars included and name clashes resolved. The rest is the edge that
//! touches the disk. Every move batch gets a journal in `.folio/moves/`,
//! written and flushed before the first file moves:
//!
//! ```text
//! folio moves 1
//! 2024/2024-10-20/_DSC0001.NEF<TAB>Events/Fair/_DSC0001.NEF
//! 2024/2024-10-20/_DSC0001.xmp<TAB>Events/Fair/_DSC0001.xmp
//! done
//! ```
//!
//! After the moves, state lines are appended, never rewritten: `done`,
//! `undoing`, `undone`, `redoing`. The last one wins. A journal that ends
//! mid-way (no state yet, `redoing` or `undoing`) was interrupted, and
//! `recover` finishes it in the same direction. Each move is checked against
//! the disk before it runs, so finishing twice is harmless.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::conflicts::conflict_original;
use crate::library::{FileMove, Library, absolute, key};
use crate::outings::SortPlan;

/// One folder that photos move into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFolder {
    /// Relative to the library root.
    pub folder: String,
    /// False when the folder already holds photos, so this adds to it.
    pub is_new: bool,
    pub photos: Vec<PlannedPhoto>,
    /// Videos shot during the outings going here.
    pub videos: Vec<FileMove>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPhoto {
    pub photo: usize,
    pub file: FileMove,
    /// Moves with the photo. `None` when the photo has no sidecar, or shares
    /// one with a photo planned earlier (a RAW and JPEG pair).
    pub sidecar: Option<FileMove>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MovePlan {
    pub folders: Vec<PlannedFolder>,
}

impl MovePlan {
    pub fn photo_count(&self) -> usize {
        self.folders.iter().map(|f| f.photos.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }

    pub fn video_count(&self) -> usize {
        self.folders.iter().map(|f| f.videos.len()).sum()
    }

    /// Every file move, photos with their sidecars, then videos.
    pub fn moves(&self) -> Vec<FileMove> {
        self.folders
            .iter()
            .flat_map(|f| f.photos.iter().flat_map(|p| std::iter::once(&p.file).chain(&p.sidecar)).chain(&f.videos))
            .cloned()
            .collect()
    }
}

/// Plans the moves for every accepted outing in `sort`, with the videos
/// shot during it. `taken` holds the keys (see `library::key`) of files that
/// already exist in the target folders, so nothing is ever overwritten. A
/// clashing name gets `-2`, `-3` and so on, and its sidecar gets the same
/// new name. Folders reuse the spelling they already have on disk.
pub fn plan(library: &Library, sort: &SortPlan, taken: &HashSet<String>) -> MovePlan {
    let mut taken = taken.clone();
    taken.extend(library.photos().iter().flat_map(|p| std::iter::once(key(&p.relative)).chain(p.sidecar.as_deref().map(key))));
    taken.extend(library.videos().iter().map(|v| key(&v.relative)));
    let mut moved_sidecars: HashSet<String> = HashSet::new();
    let mut plan = MovePlan::default();

    for (outing, videos) in sort.outings().iter().zip(sort.videos(library)).filter(|(o, _)| o.is_accepted()) {
        let Some(wanted) = outing.folder() else { continue };
        let folder = existing_spelling(library, &wanted);
        let is_new = library.existing_folder(&folder).is_none();
        let index = match plan.folders.iter().position(|f| f.folder == folder) {
            Some(index) => index,
            None => {
                plan.folders.push(PlannedFolder { folder: folder.clone(), is_new, photos: Vec::new(), videos: Vec::new() });
                plan.folders.len() - 1
            }
        };

        for &photo in outing.photos() {
            let source = library.photo(photo);
            let (stem, extension) = split_name(source.file_name());
            let sidecar = source.sidecar.as_deref().filter(|s| !moved_sidecars.contains(&key(s)));
            let sidecar_extension = sidecar.map(|s| split_name(file_name(s)).1);

            let target_stem = (1..)
                .map(|n| if n == 1 { stem.to_owned() } else { format!("{stem}-{n}") })
                .find(|candidate| {
                    let free = |ext: &str| !taken.contains(&key(&format!("{folder}/{candidate}{ext}")));
                    free(extension) && sidecar_extension.is_none_or(free)
                })
                .unwrap_or_else(|| stem.to_owned());

            let to = format!("{folder}/{target_stem}{extension}");
            taken.insert(key(&to));
            let sidecar = sidecar.zip(sidecar_extension).map(|(from, ext)| {
                moved_sidecars.insert(key(from));
                let to = format!("{folder}/{target_stem}{ext}");
                taken.insert(key(&to));
                FileMove { from: from.to_owned(), to }
            });
            plan.folders[index].photos.push(PlannedPhoto {
                photo,
                file: FileMove { from: source.relative.clone(), to },
                sidecar,
            });
        }

        for video in videos {
            let source = &library.videos()[video];
            let (stem, extension) = split_name(source.file_name());
            let to = (1..)
                .map(|n| if n == 1 { format!("{folder}/{stem}{extension}") } else { format!("{folder}/{stem}-{n}{extension}") })
                .find(|candidate| !taken.contains(&key(candidate)))
                .unwrap_or_else(|| format!("{folder}/{stem}{extension}"));
            taken.insert(key(&to));
            plan.folders[index].videos.push(FileMove { from: source.relative.clone(), to });
        }
    }
    plan.folders.retain(|f| !f.photos.is_empty());
    plan
}

/// `Category/Outing` with each part spelled the way it already is on disk.
fn existing_spelling(library: &Library, wanted: &str) -> String {
    match library.existing_folder(wanted) {
        Some(existing) => existing.to_owned(),
        None => match wanted.split_once('/') {
            Some((category, rest)) => {
                let category = library.existing_folder(category).unwrap_or(category);
                format!("{category}/{rest}")
            }
            None => wanted.to_owned(),
        },
    }
}

fn file_name(relative: &str) -> &str {
    relative.rsplit_once('/').map_or(relative, |(_, name)| name)
}

/// `_DSC0001.NEF` into `_DSC0001` and `.NEF`.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// Where a journal is in its life. Appended as the last line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalState {
    /// Written, but the moves never finished.
    Started,
    Done,
    Undoing,
    Undone,
    Redoing,
}

impl JournalState {
    fn line(self) -> Option<&'static str> {
        match self {
            Self::Started => None,
            Self::Done => Some("done"),
            Self::Undoing => Some("undoing"),
            Self::Undone => Some("undone"),
            Self::Redoing => Some("redoing"),
        }
    }

    fn from_line(line: &str) -> Option<Self> {
        [Self::Done, Self::Undoing, Self::Undone, Self::Redoing].into_iter().find(|s| s.line() == Some(line))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    pub path: PathBuf,
    pub moves: Vec<FileMove>,
    pub state: JournalState,
}

const HEADER: &str = "folio moves 1";

/// Reads a journal's text. `None` if it isn't one, or is damaged.
pub fn parse_journal(text: &str) -> Option<(Vec<FileMove>, JournalState)> {
    let mut lines = text.lines();
    if lines.next()? != HEADER {
        return None;
    }
    let mut moves = Vec::new();
    let mut state = JournalState::Started;
    for line in lines.filter(|l| !l.is_empty()) {
        if let Some((from, to)) = line.split_once('\t') {
            moves.push(FileMove { from: from.to_owned(), to: to.to_owned() });
        } else {
            state = JournalState::from_line(line)?;
        }
    }
    Some((moves, state))
}

/// What happened to one batch: which moves ran and which couldn't.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub moved: Vec<FileMove>,
    pub failed: Vec<(FileMove, String)>,
}

fn journals_dir(root: &Path) -> PathBuf {
    root.join(".folio").join("moves")
}

/// Runs a plan: writes the journal, moves every file and marks it done.
/// `id` names the journal and must sort after earlier ones (the app uses the
/// current time in milliseconds). Files that can't move are reported and
/// skipped. Folders left empty are removed.
pub fn execute(root: &Path, id: &str, moves: &[FileMove]) -> io::Result<Outcome> {
    if moves.iter().any(|m| [&m.from, &m.to].iter().any(|p| p.contains(['\t', '\n', '\r']))) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a file name contains a tab or line break"));
    }
    let dir = journals_dir(root);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.tsv"));
    let mut text = format!("{HEADER}\n");
    for m in moves {
        text.push_str(&format!("{}\t{}\n", m.from, m.to));
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;

    let outcome = run(root, moves, Direction::Forward);
    append_state(&path, JournalState::Done)?;
    Ok(outcome)
}

/// Undoes the newest batch that's done. `None` when there's nothing to undo.
pub fn undo(root: &Path) -> io::Result<Option<(Journal, Outcome)>> {
    let Some(journal) = journals(root)?.into_iter().rev().find(|j| j.state == JournalState::Done) else { return Ok(None) };
    append_state(&journal.path, JournalState::Undoing)?;
    let outcome = run(root, &journal.moves, Direction::Back);
    append_state(&journal.path, JournalState::Undone)?;
    Ok(Some((journal, outcome)))
}

/// Redoes the newest undone batch, as long as nothing was moved after it.
pub fn redo(root: &Path) -> io::Result<Option<(Journal, Outcome)>> {
    let Some(journal) = newest_redoable(root)? else { return Ok(None) };
    append_state(&journal.path, JournalState::Redoing)?;
    let outcome = run(root, &journal.moves, Direction::Forward);
    append_state(&journal.path, JournalState::Done)?;
    Ok(Some((journal, outcome)))
}

fn newest_redoable(root: &Path) -> io::Result<Option<Journal>> {
    // Walking back from the newest: undone batches can be redone in reverse
    // order of undoing. A batch that's done ends the redo stack.
    let journals = journals(root)?;
    let mut candidate = None;
    for journal in journals.into_iter().rev() {
        match journal.state {
            JournalState::Undone => candidate = Some(journal),
            _ => break,
        }
    }
    Ok(candidate)
}

/// What opening a library did about earlier move batches.
pub struct Recovery {
    /// Batches a crash had interrupted, now finished.
    pub finished: Vec<Journal>,
    /// Journal conflict copies that couldn't be merged.
    pub problems: Vec<String>,
}

/// Merges journal conflict copies, then finishes batches a crash
/// interrupted, in the direction they were going.
pub fn recover(root: &Path) -> io::Result<Recovery> {
    let problems = merge_journal_copies(root);
    let mut finished = Vec::new();
    for journal in journals(root)? {
        let (direction, end) = match journal.state {
            JournalState::Started | JournalState::Redoing => (Direction::Forward, JournalState::Done),
            JournalState::Undoing => (Direction::Back, JournalState::Undone),
            JournalState::Done | JournalState::Undone => continue,
        };
        run(root, &journal.moves, direction);
        append_state(&journal.path, end)?;
        finished.push(journal);
    }
    Ok(Recovery { finished, problems })
}

/// Merges sync conflict copies of journals (`1729871234123 2.tsv`). They
/// happen when two machines undo or redo the same batch while offline. The
/// moves in both are the same, written once, so only the state can differ,
/// and the disk says which state is true: Folio checks where the files are
/// and records that. A copy the disk can't settle is kept and reported.
fn merge_journal_copies(root: &Path) -> Vec<String> {
    let dir = journals_dir(root);
    let mut problems = Vec::new();
    for (copy, original) in journal_copies(&dir) {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !original.exists() {
            if let Err(error) = fs::rename(&copy, &original) {
                problems.push(format!("Couldn't rename the move log {}: {error}", name(&copy)));
            }
            continue;
        }
        let read = |p: &Path| fs::read_to_string(p).ok().and_then(|t| parse_journal(&t));
        let (Some((moves, state)), Some((copy_moves, copy_state))) = (read(&original), read(&copy)) else {
            problems.push(format!("The move log {} couldn't be read, so it was kept", name(&copy)));
            continue;
        };
        if moves != copy_moves {
            problems.push(format!("The move logs {} and {} list different moves, so both were kept", name(&original), name(&copy)));
            continue;
        }
        if state != copy_state {
            let Some(settled) = state_on_disk(root, &moves) else {
                problems.push(format!("The move logs {} and {} disagree and the files are half moved, so both were kept", name(&original), name(&copy)));
                continue;
            };
            if let Err(error) = append_state(&original, settled) {
                problems.push(format!("Couldn't update the move log {}: {error}", name(&original)));
                continue;
            }
        }
        if let Err(error) = fs::remove_file(&copy) {
            problems.push(format!("Merged the move log {}, but couldn't remove it: {error}", name(&copy)));
        }
    }
    problems
}

/// Every journal conflict copy in `dir`, with the journal it copies.
fn journal_copies(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tsv"))
        .filter_map(|copy| {
            let stem = copy.file_stem()?.to_str()?;
            let original = dir.join(format!("{}.tsv", conflict_original(stem)?));
            Some((copy, original))
        })
        .collect()
}

/// `Done` if every file is where the batch put it, `Undone` if every file
/// is back where it started. `None` when it's mixed.
fn state_on_disk(root: &Path, moves: &[FileMove]) -> Option<JournalState> {
    let at = |m: &FileMove| (resolve(root, &m.from).is_some(), resolve(root, &m.to).is_some());
    if moves.iter().all(|m| at(m) == (false, true)) {
        Some(JournalState::Done)
    } else if moves.iter().all(|m| at(m) == (true, false)) {
        Some(JournalState::Undone)
    } else {
        None
    }
}

/// Every readable journal, oldest first. Damaged ones are skipped with a
/// warning rather than blocking the library.
fn journals(root: &Path) -> io::Result<Vec<Journal>> {
    let dir = journals_dir(root);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    // Conflict copies left after merging (reported at open) aren't batches
    // of their own.
    let copies: Vec<PathBuf> = journal_copies(&dir).into_iter().map(|(copy, _)| copy).collect();
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tsv") && !copies.contains(p))
        .collect();
    paths.sort();
    let mut journals = Vec::new();
    for path in paths {
        let parsed = fs::read_to_string(&path).ok().and_then(|text| parse_journal(&text));
        match parsed {
            Some((moves, state)) => journals.push(Journal { path, moves, state }),
            None => eprintln!("skipping damaged move journal {}", path.display()),
        }
    }
    Ok(journals)
}

fn append_state(path: &Path, state: JournalState) -> io::Result<()> {
    let Some(line) = state.line() else { return Ok(()) };
    let mut file = OpenOptions::new().append(true).open(path)?;
    file.write_all(format!("{line}\n").as_bytes())?;
    file.sync_all()
}

#[derive(Clone, Copy)]
enum Direction {
    Forward,
    Back,
}

/// Moves files one by one. Each move checks the disk first: if the file is
/// already where it should end up, it's skipped, and an existing file at the
/// destination is never overwritten. Source folders left empty are removed.
fn run(root: &Path, moves: &[FileMove], direction: Direction) -> Outcome {
    let mut outcome = Outcome::default();
    let mut emptied: Vec<PathBuf> = Vec::new();
    let ordered: Box<dyn Iterator<Item = &FileMove>> = match direction {
        Direction::Forward => Box::new(moves.iter()),
        Direction::Back => Box::new(moves.iter().rev()),
    };
    for m in ordered {
        let (from, to) = match direction {
            Direction::Forward => (&m.from, &m.to),
            Direction::Back => (&m.to, &m.from),
        };
        let step = FileMove { from: from.clone(), to: to.clone() };
        match move_file(root, from, to) {
            Ok(Some(source)) => {
                if let Some(parent) = source.parent() {
                    emptied.push(parent.to_path_buf());
                }
                outcome.moved.push(step);
            }
            Ok(None) => {}
            Err(error) => outcome.failed.push((step, error.to_string())),
        }
    }
    emptied.sort();
    emptied.dedup();
    for folder in emptied {
        remove_empty_folders(root, &folder);
    }
    outcome
}

/// Moves one file. `Ok(None)` when there's nothing to do because it already
/// moved. Returns the source path it moved from.
fn move_file(root: &Path, from: &str, to: &str) -> io::Result<Option<PathBuf>> {
    let source = resolve(root, from);
    let target = resolve(root, to);
    match (source, target) {
        (None, Some(_)) => Ok(None),
        (None, None) => Err(io::Error::new(io::ErrorKind::NotFound, format!("{from} is gone"))),
        (Some(_), Some(_)) => Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{to} already exists"))),
        (Some(source), None) => {
            let target = absolute(root, to);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&source, &target)?;
            Ok(Some(source))
        }
    }
}

/// Finds a relative path on disk, matching each part without case and
/// Unicode form when the exact spelling isn't there. A file renamed `a.nef`
/// on one machine, or stored decomposed by another, is still found.
fn resolve(root: &Path, relative: &str) -> Option<PathBuf> {
    let direct = absolute(root, relative);
    if direct.exists() {
        return Some(direct);
    }
    let mut path = root.to_path_buf();
    for part in relative.split('/').filter(|p| !p.is_empty()) {
        let wanted = key(part);
        let found = fs::read_dir(&path).ok()?.flatten().find(|e| e.file_name().to_str().is_some_and(|n| key(n) == wanted))?;
        path = found.path();
    }
    Some(path)
}

/// Removes `folder` and its parents while they're empty, up to the root.
/// A folder holding anything at all, even `.DS_Store`, stays.
fn remove_empty_folders(root: &Path, folder: &Path) {
    let mut current = Some(folder);
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) || fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

/// Every file name already in the folders a plan would move into, as keys,
/// so `plan` won't overwrite files the library doesn't know about (videos,
/// documents). Missing folders are simply empty.
pub fn existing_files(root: &Path, folders: impl IntoIterator<Item = String>) -> HashSet<String> {
    let mut taken = HashSet::new();
    for folder in folders {
        let Some(dir) = resolve(root, &folder) else { continue };
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                taken.insert(key(&format!("{folder}/{name}")));
            }
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif::CaptureTime;
    use crate::library::Video;
    use crate::library::tests::photo;
    use crate::outings::SortIntent;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap();
    }

    /// A temporary library folder, removed when dropped.
    struct TempLibrary(PathBuf);

    impl TempLibrary {
        fn new(name: &str, files: &[&str]) -> Self {
            let root = std::env::temp_dir().join(format!("folio-moves-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            for file in files {
                touch(&absolute(&root, file));
            }
            Self(root)
        }

        fn has(&self, relative: &str) -> bool {
            absolute(&self.0, relative).exists()
        }
    }

    impl Drop for TempLibrary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn moves(pairs: &[(&str, &str)]) -> Vec<FileMove> {
        pairs.iter().map(|(from, to)| FileMove { from: (*from).into(), to: (*to).into() }).collect()
    }

    /// Accepts every outing into `folder`.
    fn plan_into(library: &Library, folder: &str, taken: &HashSet<String>) -> MovePlan {
        let mut sort = SortPlan::group(library, i64::MAX);
        let (category, name) = folder.split_once('/').unwrap();
        sort.apply(SortIntent::SetCategory { outing: 0, category: category.into() });
        sort.apply(SortIntent::Rename { outing: 0, name: name.into() });
        sort.apply(SortIntent::ToggleAccepted { outing: 0 });
        plan(library, &sort, taken)
    }

    fn dated(paths: &[&str]) -> Library {
        let photos = paths
            .iter()
            .map(|p| {
                let mut photo = photo(p);
                photo.exif.captured = CaptureTime::parse("2024:10:20 10:00:00");
                photo
            })
            .collect();
        Library::new("/lib".into(), photos)
    }

    #[test]
    fn sidecars_move_with_their_photo_and_clashes_get_a_suffix() {
        let mut lib = dated(&["2024/2024-10-20/_DSC1.NEF", "2024/2024-10-21/_DSC1.NEF", "2024/2024-10-21/_DSC1.JPG", "Events/Old/x.NEF"]);
        let mut photos = lib.photos().to_vec();
        photos[0].sidecar = Some("2024/2024-10-20/_DSC1.xmp".into());
        photos[1].sidecar = Some("2024/2024-10-21/_DSC1.xmp".into());
        photos[2].sidecar = Some("2024/2024-10-21/_DSC1.xmp".into());
        lib = Library::new("/lib".into(), photos);

        let plan = plan_into(&lib, "events/Fair", &HashSet::new());
        assert_eq!(plan.folders.len(), 1);
        assert_eq!(plan.folders[0].folder, "Events/Fair", "the category keeps its spelling on disk");
        assert!(plan.folders[0].is_new);
        assert_eq!(
            plan.moves(),
            moves(&[
                ("2024/2024-10-20/_DSC1.NEF", "Events/Fair/_DSC1.NEF"),
                ("2024/2024-10-20/_DSC1.xmp", "Events/Fair/_DSC1.xmp"),
                // The pair in the second folder sorts JPEG first. It takes the
                // shared sidecar, and both keep one stem so they stay a pair.
                ("2024/2024-10-21/_DSC1.JPG", "Events/Fair/_DSC1-2.JPG"),
                ("2024/2024-10-21/_DSC1.xmp", "Events/Fair/_DSC1-2.xmp"),
                ("2024/2024-10-21/_DSC1.NEF", "Events/Fair/_DSC1-2.NEF"),
            ])
        );
    }

    #[test]
    fn videos_go_with_the_outing_they_were_shot_during() {
        let photos = [("2024/2024-10-20/a.NEF", "2024:10:20 10:00:00"), ("2024/2024-10-20/b.NEF", "2024:10:20 18:00:00")]
            .map(|(path, time)| {
                let mut p = photo(path);
                p.exif.captured = CaptureTime::parse(time);
                p
            });
        let video = |relative: &str, time: Option<&str>| Video { relative: relative.into(), captured: time.and_then(CaptureTime::parse) };
        let lib = Library::new("/lib".into(), photos.to_vec()).with_videos(vec![
            video("2024/2024-10-20/clip.MOV", Some("2024:10:20 10:30:00")),
            video("2024/2024-10-20/late.MOV", Some("2024:10:20 17:00:00")),
            video("2024/2024-10-20/night.MOV", Some("2024:10:21 03:00:00")),
            video("2024/2024-10-20/undated.MOV", None),
            video("Events/Old/kept.MOV", Some("2024:10:20 10:00:00")),
        ]);
        let mut sort = SortPlan::group(&lib, 2 * 3600);
        assert_eq!(sort.videos(&lib), vec![vec![0], vec![1]], "night is too far from both, undated and sorted ones stay");

        sort.apply(SortIntent::SetCategory { outing: 0, category: "Events".into() });
        sort.apply(SortIntent::ToggleAccepted { outing: 0 });
        let taken = HashSet::from([key("Events/2024-10-20 morning/clip.mov")]);
        let plan = plan(&lib, &sort, &taken);
        assert_eq!(plan.video_count(), 1);
        assert_eq!(
            plan.moves(),
            moves(&[
                ("2024/2024-10-20/a.NEF", "Events/2024-10-20 morning/a.NEF"),
                ("2024/2024-10-20/clip.MOV", "Events/2024-10-20 morning/clip-2.MOV"),
            ])
        );
    }

    #[test]
    fn files_already_in_the_target_folder_are_never_overwritten() {
        let lib = dated(&["2024/a.NEF"]);
        let taken = HashSet::from([key("Events/Fair/A.nef")]);
        let plan = plan_into(&lib, "Events/Fair", &taken);
        assert_eq!(plan.moves(), moves(&[("2024/a.NEF", "Events/Fair/a-2.NEF")]));
    }

    #[test]
    fn journals_parse_and_reject_damage() {
        let (moves_read, state) = parse_journal("folio moves 1\na\tb\ndone\nundoing\n").unwrap();
        assert_eq!(moves_read, moves(&[("a", "b")]));
        assert_eq!(state, JournalState::Undoing);
        assert_eq!(parse_journal("folio moves 1\na\tb\n").unwrap().1, JournalState::Started);
        assert_eq!(parse_journal("something else\n"), None);
        assert_eq!(parse_journal("folio moves 1\ngarbage\n"), None);
    }

    #[test]
    fn execute_undo_and_redo_round_trip_and_clean_up_empty_folders() {
        let lib = TempLibrary::new("round-trip", &["2024/2024-10-20/a.NEF", "2024/2024-10-20/a.xmp", "2024/2024-10-21/b.NEF"]);
        let batch = moves(&[
            ("2024/2024-10-20/a.NEF", "Events/Fair/a.NEF"),
            ("2024/2024-10-20/a.xmp", "Events/Fair/a.xmp"),
        ]);

        let outcome = execute(&lib.0, "1", &batch).unwrap();
        assert_eq!(outcome.moved.len(), 2);
        assert!(outcome.failed.is_empty());
        assert!(lib.has("Events/Fair/a.NEF") && lib.has("Events/Fair/a.xmp"));
        assert!(!lib.has("2024/2024-10-20"), "the emptied date folder is gone");
        assert!(lib.has("2024/2024-10-21/b.NEF"));

        let (journal, outcome) = undo(&lib.0).unwrap().unwrap();
        assert_eq!(journal.moves, batch);
        assert_eq!(outcome.moved.len(), 2);
        assert!(lib.has("2024/2024-10-20/a.NEF") && lib.has("2024/2024-10-20/a.xmp"));
        assert!(!lib.has("Events"), "undo leaves no empty folders either");
        assert_eq!(undo(&lib.0).unwrap(), None, "nothing left to undo");

        redo(&lib.0).unwrap().unwrap();
        assert!(lib.has("Events/Fair/a.NEF"));
        assert_eq!(redo(&lib.0).unwrap(), None, "nothing left to redo");
    }

    #[test]
    fn a_new_batch_ends_the_redo_stack() {
        let lib = TempLibrary::new("redo-stack", &["a.NEF", "b.NEF"]);
        execute(&lib.0, "1", &moves(&[("a.NEF", "X/Y/a.NEF")])).unwrap();
        undo(&lib.0).unwrap();
        execute(&lib.0, "2", &moves(&[("b.NEF", "X/Y/b.NEF")])).unwrap();
        assert_eq!(redo(&lib.0).unwrap(), None);
    }

    #[test]
    fn an_existing_file_is_never_overwritten() {
        let lib = TempLibrary::new("no-overwrite", &["a.NEF", "X/Y/a.NEF"]);
        let outcome = execute(&lib.0, "1", &moves(&[("a.NEF", "X/Y/a.NEF")])).unwrap();
        assert!(outcome.moved.is_empty());
        assert_eq!(outcome.failed.len(), 1);
        assert!(lib.has("a.NEF"));
    }

    #[test]
    fn an_interrupted_batch_is_finished_on_recovery() {
        // The crash came after the journal and the first move.
        let lib = TempLibrary::new("recover", &["X/Y/a.NEF", "b.NEF"]);
        let dir = journals_dir(&lib.0);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("1.tsv"), "folio moves 1\na.NEF\tX/Y/a.NEF\nb.NEF\tX/Y/b.NEF\n").unwrap();

        let finished = recover(&lib.0).unwrap().finished;
        assert_eq!(finished.len(), 1);
        assert!(lib.has("X/Y/a.NEF") && lib.has("X/Y/b.NEF"));
        assert!(!lib.has("b.NEF"));
        assert!(recover(&lib.0).unwrap().finished.is_empty(), "recovery runs once");
        assert!(undo(&lib.0).unwrap().is_some(), "a recovered batch can be undone");
    }

    #[test]
    fn a_journal_conflict_copy_merges_to_what_the_disk_says() {
        // One machine undid the batch, the other's log still says done.
        let lib = TempLibrary::new("journal-copy", &["a.NEF"]);
        let dir = journals_dir(&lib.0);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("1.tsv"), "folio moves 1\na.NEF\tX/Y/a.NEF\ndone\n").unwrap();
        fs::write(dir.join("1 2.tsv"), "folio moves 1\na.NEF\tX/Y/a.NEF\ndone\nundoing\nundone\n").unwrap();

        let recovery = recover(&lib.0).unwrap();
        assert!(recovery.problems.is_empty(), "{:?}", recovery.problems);
        assert!(!dir.join("1 2.tsv").exists());
        assert_eq!(journals(&lib.0).unwrap()[0].state, JournalState::Undone);
        assert_eq!(undo(&lib.0).unwrap(), None, "nothing is undone twice");
    }

    #[test]
    fn a_journal_copy_with_other_moves_is_kept_and_reported() {
        let lib = TempLibrary::new("journal-other", &[]);
        let dir = journals_dir(&lib.0);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("1.tsv"), "folio moves 1\na.NEF\tX/a.NEF\ndone\n").unwrap();
        fs::write(dir.join("1 2.tsv"), "folio moves 1\nb.NEF\tX/b.NEF\ndone\n").unwrap();

        let recovery = recover(&lib.0).unwrap();
        assert_eq!(recovery.problems.len(), 1);
        assert!(dir.join("1 2.tsv").exists());
        assert_eq!(journals(&lib.0).unwrap().len(), 1, "the copy isn't a batch of its own");
    }

    #[test]
    fn paths_resolve_without_case_or_unicode_form() {
        let lib = TempLibrary::new("resolve", &["Scha\u{0308}ferlauf/A.NEF"]);
        assert!(resolve(&lib.0, "sch\u{e4}ferlauf/a.nef").is_some());
        assert!(resolve(&lib.0, "other/a.nef").is_none());
    }
}
