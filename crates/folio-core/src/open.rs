//! Opening a folder as a library: finish any moves a crash interrupted,
//! merge sync conflict copies, scan the photos and read what the app shows
//! before thumbnails exist (shape, EXIF, marks from the sidecars) and the
//! videos' capture times. The edge
//! where a folder on disk becomes a `Library`.

use std::fs;
use std::io;
use std::path::Path;

use crate::conflicts::merge_sidecar_copies;
use crate::culling::Marks;
use crate::info::read_info;
use crate::library::{Library, Photo, Video, relative_path};
use crate::moves;
use crate::scan::scan;
use crate::video::{local_video_time, read_video_utc};
use crate::xmp::read_marks;

/// Shape for photos whose headers don't say: the common 3:2 sensor.
const DEFAULT_ASPECT: f32 = 1.5;

pub struct Opened {
    pub library: Library,
    /// Move batches a crash had interrupted, now finished.
    pub recovered: usize,
    /// Photos skipped because their path isn't valid Unicode.
    pub skipped: usize,
    /// Things the user should hear about, such as conflict copies that
    /// couldn't be merged.
    pub problems: Vec<String>,
}

pub fn open_library(root: &Path) -> io::Result<Opened> {
    let root = fs::canonicalize(root)?;
    let recovery = moves::recover(&root).unwrap_or_else(|error| moves::Recovery {
        finished: Vec::new(),
        problems: vec![format!("Couldn't finish interrupted moves: {error}")],
    });
    let recovered = recovery.finished.len();

    let mut photos = Vec::new();
    let mut skipped = 0;
    let mut problems = recovery.problems;
    let found = scan(&root)?;
    for mut scanned in found.photos {
        if !scanned.sidecar_copies.is_empty() {
            let (sidecar, trouble) = merge_sidecar_copies(&scanned.path, scanned.sidecar.take(), &scanned.sidecar_copies);
            scanned.sidecar = sidecar;
            let folder = scanned.path.parent().and_then(|f| relative_path(&root, f)).unwrap_or_default();
            problems.extend(trouble.into_iter().map(|t| if folder.is_empty() { t } else { format!("In {folder}: {t}") }));
        }
        let Some(relative) = relative_path(&root, &scanned.path) else {
            skipped += 1;
            continue;
        };
        let sidecar = scanned.sidecar.as_deref().and_then(|s| relative_path(&root, s));
        let marks = match &scanned.sidecar {
            Some(path) => fs::read_to_string(path).map(|text| read_marks(&text)).unwrap_or_default(),
            None => Marks::default(),
        };
        let info = read_info(&scanned.path);
        photos.push(Photo {
            path: scanned.path,
            relative,
            sidecar,
            aspect: info.aspect().unwrap_or(DEFAULT_ASPECT),
            size: info.size,
            exif: info.exif,
            marks,
        });
    }
    // Photos that know their time zone, as (UTC seconds, offset), for
    // placing videos in time (see `video`).
    let zoned: Vec<(i64, i32)> = photos
        .iter()
        .filter_map(|p: &Photo| Some((p.exif.captured?.seconds() - i64::from(p.exif.utc_offset?) * 60, p.exif.utc_offset?)))
        .collect();
    let videos = found
        .videos
        .iter()
        .filter_map(|path| {
            let captured = read_video_utc(path).and_then(|utc| local_video_time(utc, &zoned));
            Some(Video { relative: relative_path(&root, path)?, captured })
        })
        .collect();
    Ok(Opened { library: Library::new(root, photos).with_videos(videos), recovered, skipped, problems })
}
