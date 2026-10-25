//! Finds the photos and videos in a library folder.

use std::io;
use std::path::{Path, PathBuf};

use crate::conflicts::conflict_original;

/// Extensions Folio can show a thumbnail for: TIFF-based RAWs and JPEG.
/// Matched case-insensitively, since cameras write `.NEF` and phones `.jpg`.
const PHOTO_EXTENSIONS: [&str; 8] = ["nef", "nrw", "dng", "arw", "cr2", "pef", "jpg", "jpeg"];
/// Videos aren't shown, but move with their outing (see `video`).
const VIDEO_EXTENSIONS: [&str; 3] = ["mov", "mp4", "m4v"];

/// A photo file and its XMP sidecar, if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    pub path: PathBuf,
    pub sidecar: Option<PathBuf>,
    /// Sync conflict copies of the sidecar, such as `_DSC0001 2.xmp`, to be
    /// merged (see `conflicts`).
    pub sidecar_copies: Vec<PathBuf>,
}

/// What a scan found, each list grouped by folder and sorted by name.
#[derive(Debug, Default)]
pub struct Found {
    pub photos: Vec<Scanned>,
    pub videos: Vec<PathBuf>,
}

/// Finds every photo under `root`, each with its sidecar (`name.xmp` next
/// to `name.NEF`), and every video. Hidden files and folders (including
/// `.folio/`) are skipped. Unreadable subfolders are skipped too, so one bad
/// folder doesn't hide the rest.
pub fn scan(root: &Path) -> io::Result<Found> {
    let mut photos = Vec::new();
    let mut videos = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut is_root = true;

    while let Some(dir) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // The root itself must be readable. Below that, keep going.
            Err(error) if is_root => return Err(error),
            Err(_) => continue,
        };
        is_root = false;

        let mut here = Vec::new();
        let mut sidecars = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && is_photo(&path) {
                here.push(path);
            } else if kind.is_file() && has_extension(&path, "xmp") {
                sidecars.push(path);
            } else if kind.is_file() && VIDEO_EXTENSIONS.iter().any(|known| has_extension(&path, known)) {
                videos.push(path);
            }
        }
        photos.extend(here.iter().map(|path| {
            let sidecar = sidecars.iter().find(|s| same_stem(s, path)).cloned();
            // A sidecar that belongs to no photo by its own name, but would
            // after dropping a conflict suffix, is a copy of this one.
            let sidecar_copies = sidecars
                .iter()
                .filter(|s| !here.iter().any(|p| same_stem(s, p)))
                .filter(|s| {
                    let stem = s.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let photo_stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    conflict_original(&stem).is_some_and(|original| original.eq_ignore_ascii_case(&photo_stem))
                })
                .cloned()
                .collect();
            Scanned { path: path.clone(), sidecar, sidecar_copies }
        }));
    }

    // Folder first, then name, so each folder's photos sit together even
    // when it also has subfolders.
    photos.sort_by(|a, b| (a.path.parent(), a.path.file_name()).cmp(&(b.path.parent(), b.path.file_name())));
    videos.sort_by(|a, b| (a.parent(), a.file_name()).cmp(&(b.parent(), b.file_name())));
    Ok(Found { photos, videos })
}

fn same_stem(a: &Path, b: &Path) -> bool {
    match (a.file_stem(), b.file_stem()) {
        (Some(a), Some(b)) => a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

fn has_extension(path: &Path, wanted: &str) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(wanted))
}

fn is_photo(path: &Path) -> bool {
    PHOTO_EXTENSIONS.iter().any(|known| has_extension(path, known))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_photo_extensions_in_any_case() {
        assert!(is_photo(Path::new("a/_DSC1140.NEF")));
        assert!(is_photo(Path::new("a/IMG_1.jpg")));
        assert!(!is_photo(Path::new("a/_DSC1140.xmp")));
        assert!(!is_photo(Path::new("a/clip.MOV")));
        assert!(!is_photo(Path::new("a/NEF")));
    }

    #[test]
    fn finds_videos_and_sidecars_by_stem_in_any_case_with_their_conflict_copies() {
        let dir = std::env::temp_dir().join(format!("folio-scan-{}", std::process::id()));
        let sub = dir.join("2024-10-20");
        std::fs::create_dir_all(&sub).unwrap();
        for name in ["_DSC1.NEF", "_DSC1.XMP", "_DSC1 2.xmp", "_DSC2.NEF", "notes.txt", "Photo 2.NEF", "Photo 2.xmp", "DSC_0003.MOV"] {
            std::fs::write(sub.join(name), b"").unwrap();
        }
        std::fs::create_dir_all(dir.join(".folio")).unwrap();
        std::fs::write(dir.join(".folio/hidden.jpg"), b"").unwrap();

        let Found { photos: found, videos } = scan(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(videos, vec![sub.join("DSC_0003.MOV")]);
        assert_eq!(found.len(), 3, "photos only, nothing from .folio/");
        assert_eq!(found[0].path.file_name().unwrap(), "Photo 2.NEF");
        assert_eq!(found[0].sidecar.as_deref(), Some(sub.join("Photo 2.xmp").as_path()));
        assert!(found[0].sidecar_copies.is_empty(), "a sidecar named after a photo is never a copy");
        assert_eq!(found[1].sidecar.as_deref(), Some(sub.join("_DSC1.XMP").as_path()));
        assert_eq!(found[1].sidecar_copies, vec![sub.join("_DSC1 2.xmp")]);
        assert_eq!(found[2].sidecar, None);
    }
}
