//! Thumbnails on disk, so a library opens with its grid already filled in
//! and photos scrolled out of memory come back in a couple of milliseconds.
//!
//! Each thumbnail is a small JPEG in the OS cache folder, never in the
//! library. The key is the photo's file name, size and modification time,
//! not its path, so a thumbnail survives Folio moving the photo into an
//! outing folder. Everything here can be deleted at any time.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use image::RgbaImage;
use image::codecs::jpeg::JpegEncoder;

use crate::thumbnail::{ThumbnailError, decode_jpeg, load_thumbnail};

/// Bump when thumbnails change shape, so old ones are ignored.
const VERSION: &str = "thumbnails-1";
const QUALITY: u8 = 85;

#[derive(Debug, Clone)]
pub struct ThumbnailCache {
    dir: PathBuf,
}

impl ThumbnailCache {
    /// The cache in `dir`, for example `dirs::cache_dir()`.
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.join(VERSION) }
    }

    /// The thumbnail for `photo`, from the cache if it's there, otherwise
    /// made from the photo and stored. A cache that can't be written still
    /// returns the thumbnail. Blocking: call it off the UI thread.
    pub fn load(&self, photo: &Path, max_edge: u32) -> Result<RgbaImage, ThumbnailError> {
        let Some(entry) = self.entry(photo, max_edge) else { return load_thumbnail(photo, max_edge) };
        if let Ok(bytes) = fs::read(&entry)
            && let Ok(image) = decode_jpeg(&bytes, max_edge)
        {
            return Ok(image);
        }
        let image = load_thumbnail(photo, max_edge)?;
        if let Err(error) = store(&entry, &image) {
            eprintln!("can't cache the thumbnail for {}: {error}", photo.display());
        }
        Ok(image)
    }

    /// Where the thumbnail for `photo` goes. `None` if the photo can't be
    /// read, in which case there's nothing to key it by.
    fn entry(&self, photo: &Path, max_edge: u32) -> Option<PathBuf> {
        let meta = fs::metadata(photo).ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
        let name = photo.file_name()?.to_string_lossy().to_lowercase();
        let key = fnv1a(format!("{name}\0{}\0{modified}\0{max_edge}", meta.len()).as_bytes());
        let hex = format!("{key:016x}");
        // 256 subfolders keep any one folder small.
        Some(self.dir.join(&hex[..2]).join(format!("{hex}.jpg")))
    }
}

/// Writes through a temporary file, so a crash never leaves half a JPEG.
fn store(entry: &Path, image: &RgbaImage) -> io::Result<()> {
    let dir = entry.parent().ok_or_else(|| io::Error::other("cache entry has no folder"))?;
    fs::create_dir_all(dir)?;
    let rgb = image::DynamicImage::ImageRgba8(image.clone()).into_rgb8();
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, QUALITY).encode_image(&rgb).map_err(io::Error::other)?;
    let temporary = entry.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, entry)
}

/// FNV-1a: tiny and stable across Rust versions, unlike `DefaultHasher`, so
/// cache keys and draft file names stay the same after an update.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn jpeg(path: &Path, width: u32, height: u32) {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 200]));
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes).encode_image(&image).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn the_second_load_comes_from_disk_and_survives_a_move() {
        let root = TempDir(std::env::temp_dir().join(format!("folio-thumbcache-{}", std::process::id())));
        let _ = fs::remove_dir_all(&root.0);
        fs::create_dir_all(root.0.join("a")).unwrap();
        let photo = root.0.join("a/photo.jpg");
        jpeg(&photo, 800, 600);
        let cache = ThumbnailCache::new(&root.0.join("cache"));

        let first = cache.load(&photo, 200).unwrap();
        assert_eq!(first.dimensions(), (200, 150));
        let entry = cache.entry(&photo, 200).unwrap();
        assert!(entry.exists());

        // Moved into an outing folder: same name, size and time, same entry.
        fs::create_dir_all(root.0.join("Events/Fair")).unwrap();
        let moved = root.0.join("Events/Fair/photo.jpg");
        fs::rename(&photo, &moved).unwrap();
        assert_eq!(cache.entry(&moved, 200), Some(entry.clone()));

        // Swap the stored thumbnail for a marker: the next load must return
        // it, which proves it came from disk and not from the photo.
        jpeg(&entry, 20, 10);
        assert_eq!(cache.load(&moved, 200).unwrap().dimensions(), (20, 10));
    }

    #[test]
    fn keys_match_the_published_fnv1a_values() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
