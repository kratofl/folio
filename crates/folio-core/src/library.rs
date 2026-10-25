//! The library as data: every photo with its path relative to the library
//! root, and the folder structure read from those paths. Folders are the
//! organisation. A photo in `Category/Outing/` is sorted, a photo in a date
//! folder like `2024/2024-10-20/` (what cameras and importers create) is not.
//! Pure: the app builds `Photo`s from the scan and hands them in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::culling::Marks;
use crate::exif::{CaptureTime, Exif};
use crate::layout::Section;

#[derive(Debug, Clone, PartialEq)]
pub struct Photo {
    /// Where the file is on disk.
    pub path: PathBuf,
    /// The path from the library root, NFC, with `/` separators.
    pub relative: String,
    /// The XMP sidecar, if there is one, relative like `relative`.
    pub sidecar: Option<String>,
    /// Width over height, upright.
    pub aspect: f32,
    /// Pixels, upright, if the headers say (see `info::PhotoInfo::size`).
    pub size: Option<(u32, u32)>,
    pub exif: Exif,
    /// What the sidecar said when the library opened.
    pub marks: Marks,
}

impl Photo {
    /// The folder part of `relative`. Empty for photos in the library root.
    pub fn folder(&self) -> &str {
        self.relative.rsplit_once('/').map_or("", |(folder, _)| folder)
    }

    pub fn file_name(&self) -> &str {
        self.relative.rsplit_once('/').map_or(&self.relative, |(_, name)| name)
    }
}

/// A video. Folio doesn't show videos, it moves them with their outing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    /// The path from the library root, NFC, with `/` separators.
    pub relative: String,
    pub captured: Option<CaptureTime>,
}

impl Video {
    pub fn file_name(&self) -> &str {
        self.relative.rsplit_once('/').map_or(&self.relative, |(_, name)| name)
    }
}

/// One file moved from one place in the library to another. Both paths are
/// relative to the root, NFC, with `/` separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMove {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    root: PathBuf,
    photos: Vec<Photo>,
    videos: Vec<Video>,
}

impl Library {
    /// Photo indices are stable for the life of the library: moves update a
    /// photo's paths in place, so selection, marks and thumbnails keyed by
    /// index stay valid.
    pub fn new(root: PathBuf, photos: Vec<Photo>) -> Self {
        Self { root, photos, videos: Vec::new() }
    }

    pub fn with_videos(self, videos: Vec<Video>) -> Self {
        Self { videos, ..self }
    }

    pub fn videos(&self) -> &[Video] {
        &self.videos
    }

    /// Videos in unsorted folders, which go with whichever outing they
    /// were shot during.
    pub fn unsorted_videos(&self) -> Vec<usize> {
        (0..self.videos.len()).filter(|&i| is_unsorted_folder(self.videos[i].relative.rsplit_once('/').map_or("", |(f, _)| f))).collect()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    pub fn photo(&self, index: usize) -> &Photo {
        &self.photos[index]
    }

    /// The library folder's own name, used as the title for photos in the root.
    pub fn name(&self) -> String {
        self.root.file_name().map_or_else(|| self.root.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Where a relative path is on disk.
    pub fn absolute(&self, relative: &str) -> PathBuf {
        absolute(&self.root, relative)
    }

    pub fn is_sorted(&self, photo: usize) -> bool {
        !is_unsorted_folder(self.photos[photo].folder())
    }

    /// Photos still waiting to be sorted, in library order.
    pub fn unsorted(&self) -> Vec<usize> {
        (0..self.photos.len()).filter(|&i| !self.is_sorted(i)).collect()
    }

    /// The category a sorted photo is filed under: its top folder.
    pub fn category_of(&self, photo: usize) -> Option<&str> {
        self.is_sorted(photo).then(|| top_folder(self.photos[photo].folder()))
    }

    /// Every category with its photo count, sorted by name.
    pub fn categories(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for photo in 0..self.photos.len() {
            let Some(category) = self.category_of(photo) else { continue };
            match counts.iter_mut().find(|(name, _)| name == category) {
                Some((_, count)) => *count += 1,
                None => counts.push((category.to_owned(), 1)),
            }
        }
        counts.sort_by_key(|(name, _)| name.to_lowercase());
        counts
    }

    /// The spelling a folder already has on disk, matched without case. Lets
    /// a typed `events` land in the existing `Events`.
    pub fn existing_folder(&self, folder: &str) -> Option<&str> {
        let wanted = key(folder);
        self.photos.iter().find_map(|p| {
            let existing = p.folder();
            // A photo in `Events/Market` also proves `Events` exists.
            let mut end = existing.len();
            loop {
                let candidate = &existing[..end];
                if key(candidate) == wanted {
                    return Some(candidate);
                }
                end = candidate.rfind('/')?;
            }
        })
    }

    /// One section per folder, for the photos `include` accepts. Folders are
    /// sorted by path without case, photos by name.
    pub fn folder_sections(&self, include: impl Fn(usize) -> bool) -> Vec<Section> {
        let mut folders: Vec<(&str, Vec<usize>)> = Vec::new();
        for photo in (0..self.photos.len()).filter(|&i| include(i)) {
            let folder = self.photos[photo].folder();
            match folders.iter_mut().find(|(f, _)| *f == folder) {
                Some((_, photos)) => photos.push(photo),
                None => folders.push((folder, vec![photo])),
            }
        }
        folders.sort_by_key(|(folder, _)| folder.to_lowercase());
        folders
            .into_iter()
            .map(|(folder, mut photos)| {
                photos.sort_by_key(|&i| self.photos[i].file_name().to_lowercase());
                let title = if folder.is_empty() { self.name() } else { folder.replace('/', " / ") };
                Section { title, photos }
            })
            .collect()
    }

    /// The photo at a relative path, matched without case or Unicode form.
    pub fn photo_at(&self, relative: &str) -> Option<usize> {
        let wanted = key(relative);
        self.photos.iter().position(|p| key(&p.relative) == wanted)
    }

    /// Records a sidecar Folio just created for a photo.
    pub fn set_sidecar(&mut self, photo: usize, relative: String) {
        self.photos[photo].sidecar = Some(relative);
    }

    /// Updates photo, sidecar and video paths after files moved on disk.
    /// Moves that don't match a known file are ignored.
    pub fn apply_moves(&mut self, moves: &[FileMove]) {
        let to_by_from: HashMap<String, &str> = moves.iter().map(|m| (key(&m.from), m.to.as_str())).collect();
        for photo in &mut self.photos {
            if let Some(to) = to_by_from.get(&key(&photo.relative)) {
                photo.relative = (*to).to_owned();
                photo.path = absolute(&self.root, to);
            }
            if let Some(sidecar) = &mut photo.sidecar
                && let Some(to) = to_by_from.get(&key(sidecar))
            {
                *sidecar = (*to).to_owned();
            }
        }
        for video in &mut self.videos {
            if let Some(to) = to_by_from.get(&key(&video.relative)) {
                video.relative = (*to).to_owned();
            }
        }
    }
}

/// `path` relative to `root`, NFC, with `/` separators. `None` if `path`
/// isn't inside `root` or isn't valid Unicode.
pub fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = path.strip_prefix(root).ok()?.components().map(|c| c.as_os_str().to_str()).collect();
    Some(nfc(&parts?.join("/")))
}

/// The sidecar Folio creates for a photo that has none: same name, `.xmp`.
pub fn new_sidecar(relative: &str) -> String {
    let (folder, name) = relative.rsplit_once('/').map_or(("", relative), |(f, n)| (f, n));
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if folder.is_empty() { format!("{stem}.xmp") } else { format!("{folder}/{stem}.xmp") }
}

pub fn absolute(root: &Path, relative: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(relative.split('/').filter(|part| !part.is_empty()));
    path
}

pub fn nfc(text: &str) -> String {
    text.nfc().collect()
}

/// How two paths compare: NFC and without case, since macOS and Windows both
/// treat `Events` and `events` as the same folder.
pub fn key(path: &str) -> String {
    nfc(path).to_lowercase()
}

/// True for folders that only say when, not what: the library root, date
/// folders (`2024`, `2024-10-20`, `20241020`) and camera folders (`DCIM`,
/// `100NIKON`), at any depth.
pub fn is_unsorted_folder(folder: &str) -> bool {
    folder.is_empty() || folder.split('/').all(|part| is_date_name(part) || is_camera_name(part))
}

fn is_date_name(name: &str) -> bool {
    let year = name.get(..4).and_then(|y| y.parse::<u16>().ok());
    year.is_some_and(|y| (1900..2100).contains(&y))
        && name.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | '_' | '.' | ' '))
}

/// `DCIM` and DCF folders: three digits then five letters, such as `100NIKON`.
fn is_camera_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    name.eq_ignore_ascii_case("dcim")
        || (bytes.len() == 8
            && bytes[..3].iter().all(u8::is_ascii_digit)
            && bytes[3..].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_'))
}

fn top_folder(folder: &str) -> &str {
    folder.split_once('/').map_or(folder, |(top, _)| top)
}

/// Turns typed text into a folder name that works on macOS and Windows:
/// characters Windows forbids become `-`, trailing dots and spaces go, and the
/// result is NFC. `None` if nothing usable is left.
pub fn folder_name(typed: &str) -> Option<String> {
    let cleaned: String =
        typed.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '-' } else { c }).collect();
    let cleaned = nfc(cleaned.trim().trim_end_matches(['.', ' ']));
    (!cleaned.is_empty() && cleaned != "." && cleaned != "..").then_some(cleaned)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A photo for tests. Only the path matters to most of the library.
    pub(crate) fn photo(relative: &str) -> Photo {
        Photo {
            path: absolute(Path::new("/lib"), relative),
            relative: relative.into(),
            sidecar: None,
            aspect: 1.5,
            size: None,
            exif: Exif::default(),
            marks: Marks::default(),
        }
    }

    pub(crate) fn library(paths: &[&str]) -> Library {
        Library::new("/lib".into(), paths.iter().map(|p| photo(p)).collect())
    }

    #[test]
    fn date_and_camera_folders_are_unsorted() {
        for folder in ["", "2024", "2024/2024-10-20", "20241020", "DCIM/100NIKON", "2024/2024_10_20"] {
            assert!(is_unsorted_folder(folder), "{folder:?}");
        }
        for folder in ["Events", "Events/Christmas market", "2024/Stuttgart", "2024-10-20 Stuttgart", "1234"] {
            assert!(!is_unsorted_folder(folder), "{folder:?}");
        }
    }

    #[test]
    fn categories_are_top_folders_of_sorted_photos() {
        let lib = library(&["2024/2024-10-20/a.NEF", "Events/Market/b.NEF", "events/Fair/c.NEF", "Nature/d.NEF", "Events/Market/e.NEF"]);
        assert_eq!(lib.unsorted(), vec![0]);
        assert_eq!(lib.category_of(1), Some("Events"));
        assert_eq!(lib.category_of(0), None);
        assert_eq!(
            lib.categories(),
            vec![("Events".into(), 2), ("events".into(), 1), ("Nature".into(), 1)],
            "names stay as spelled on disk"
        );
    }

    #[test]
    fn existing_folders_match_without_case() {
        let lib = library(&["Events/Christmas Market/a.NEF"]);
        assert_eq!(lib.existing_folder("events"), Some("Events"));
        assert_eq!(lib.existing_folder("EVENTS/christmas market"), Some("Events/Christmas Market"));
        assert_eq!(lib.existing_folder("Nature"), None);
    }

    #[test]
    fn folder_sections_group_by_folder_and_sort() {
        let lib = library(&["b/2.NEF", "a/x/1.NEF", "b/1.NEF", "root.NEF"]);
        let sections = lib.folder_sections(|_| true);
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["lib", "a / x", "b"]);
        assert_eq!(sections[2].photos, vec![2, 0]);
        assert_eq!(lib.folder_sections(|i| i == 1).len(), 1);
    }

    #[test]
    fn relative_paths_are_nfc_with_forward_slashes() {
        // "Märkte" with a decomposed ä, as macOS often returns it.
        let decomposed = "Ma\u{0308}rkte";
        let path = Path::new("/lib").join(decomposed).join("a.NEF");
        assert_eq!(relative_path(Path::new("/lib"), &path).as_deref(), Some("M\u{e4}rkte/a.NEF"));
        assert_eq!(relative_path(Path::new("/other"), &path), None);
    }

    #[test]
    fn moves_update_photos_and_sidecars_in_place() {
        let mut lib = library(&["2024/a.NEF", "2024/b.NEF"]);
        lib.photos[0].sidecar = Some("2024/a.xmp".into());
        lib.apply_moves(&[
            FileMove { from: "2024/A.nef".into(), to: "Events/Fair/a.NEF".into() },
            FileMove { from: "2024/a.xmp".into(), to: "Events/Fair/a.xmp".into() },
        ]);
        assert_eq!(lib.photo(0).relative, "Events/Fair/a.NEF");
        assert_eq!(lib.photo(0).path, Path::new("/lib/Events/Fair/a.NEF"));
        assert_eq!(lib.photo(0).sidecar.as_deref(), Some("Events/Fair/a.xmp"));
        assert_eq!(lib.photo(1).relative, "2024/b.NEF");
    }

    #[test]
    fn new_sidecars_sit_next_to_their_photo() {
        assert_eq!(new_sidecar("2024/a.b/_DSC1.NEF"), "2024/a.b/_DSC1.xmp");
        assert_eq!(new_sidecar("photo.jpg"), "photo.xmp");
    }

    #[test]
    fn typed_folder_names_are_made_safe() {
        assert_eq!(folder_name("  Christmas market  ").as_deref(), Some("Christmas market"));
        assert_eq!(folder_name("Cities/Stuttgart: day 1...").as_deref(), Some("Cities-Stuttgart- day 1"));
        assert_eq!(folder_name(" .. "), None);
        assert_eq!(folder_name("   "), None);
    }
}
