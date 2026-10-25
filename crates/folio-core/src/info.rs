//! Reads what Folio needs to know about a photo from its headers alone: its
//! shape (so the grid can lay out rows before any thumbnail exists) and its
//! EXIF. One quick pass per photo when a library opens.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::exif::{self, Exif};
use crate::preview::{self, Orientation};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoInfo {
    /// Width and height in pixels, upright: the JPEG's for a JPEG, the
    /// biggest embedded preview's for a RAW. `None` if the headers don't
    /// say, in which case the caller picks a default shape.
    pub size: Option<(u32, u32)>,
    pub exif: Exif,
}

impl PhotoInfo {
    /// Width divided by height, upright.
    pub fn aspect(&self) -> Option<f32> {
        self.size.and_then(|(width, height)| (width > 0 && height > 0).then(|| width as f32 / height as f32))
    }
}

/// Reads a photo's shape and EXIF. Unreadable or unknown files come back with
/// nothing filled in, so one bad file never stops a library from opening.
pub fn read_info(path: &Path) -> PhotoInfo {
    let Ok(file) = File::open(path) else { return PhotoInfo::default() };
    let mut file = BufReader::new(file);
    let mut magic = [0u8; 2];
    if file.read_exact(&mut magic).is_err() {
        return PhotoInfo::default();
    }

    if magic == [0xff, 0xd8] {
        // Plain JPEG. A sideways-stored portrait reports its upright shape.
        let exif = exif::read_jpeg_exif(&mut file).unwrap_or_default();
        let size = file.seek(SeekFrom::Start(2)).ok().and_then(|_| jpeg_size(&mut file));
        let upright = exif.orientation.is_some_and(|o| Orientation::from_exif(o).swaps_axes());
        let size = size.map(|(w, h)| if upright { (h, w) } else { (w, h) });
        return PhotoInfo { size, exif };
    }

    let size = preview::find_preview(&mut file).ok().and_then(|found| {
        let (w, h) = found.size?;
        Some(if found.orientation.swaps_axes() { (h, w) } else { (w, h) })
    });
    let exif = exif::read_tiff_exif(&mut file).unwrap_or_default();
    PhotoInfo { size, exif }
}

/// Walks JPEG markers up to the first start-of-frame and returns its size.
/// The reader must sit just past the SOI marker.
fn jpeg_size<R: Read + Seek>(reader: &mut R) -> Option<(u32, u32)> {
    loop {
        let mut marker = [0u8; 2];
        reader.read_exact(&mut marker).ok()?;
        if marker[0] != 0xff {
            return None;
        }
        // Markers may be padded with extra 0xff bytes.
        let mut kind = marker[1];
        while kind == 0xff {
            let mut next = [0u8; 1];
            reader.read_exact(&mut next).ok()?;
            kind = next[0];
        }

        let mut length = [0u8; 2];
        reader.read_exact(&mut length).ok()?;
        let length = u16::from_be_bytes(length);

        // SOF0 to SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if matches!(kind, 0xc0..=0xcf) && !matches!(kind, 0xc4 | 0xc8 | 0xcc) {
            let mut frame = [0u8; 5];
            reader.read_exact(&mut frame).ok()?;
            let height = u16::from_be_bytes([frame[1], frame[2]]);
            let width = u16::from_be_bytes([frame[3], frame[4]]);
            return Some((width.into(), height.into()));
        }
        reader.seek(SeekFrom::Current(i64::from(length) - 2)).ok()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;
    use std::io::Cursor;

    #[test]
    fn reads_the_frame_size_of_a_jpeg() {
        let image = image::RgbImage::new(300, 200);
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes).encode_image(&image).unwrap();

        let mut reader = Cursor::new(bytes);
        reader.seek(SeekFrom::Start(2)).unwrap();
        assert_eq!(jpeg_size(&mut reader), Some((300, 200)));
    }
}
