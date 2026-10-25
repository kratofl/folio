//! Finds the embedded JPEG preview inside TIFF-based RAW files (NEF, DNG, ARW,
//! CR2, PEF and friends) without reading the RAW data itself.
//!
//! Cameras store one or more JPEGs next to the sensor data. The largest one is
//! usually full size, which is plenty for thumbnails and previews and far
//! cheaper than demosaicing. This module only walks TIFF directories, so it
//! reads a few kilobytes of headers and then exactly the bytes of one JPEG.

use std::io::{self, Read, Seek};

use crate::tiff;

/// Where a JPEG sits inside the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegLocation {
    pub offset: u64,
    pub length: u64,
}

/// EXIF orientation, as the camera recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Normal,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    /// Maps the EXIF value. Anything out of range is treated as normal,
    /// because a wrong tag shouldn't stop a photo from showing.
    pub fn from_exif(value: u32) -> Self {
        match value {
            2 => Self::FlipHorizontal,
            3 => Self::Rotate180,
            4 => Self::FlipVertical,
            5 => Self::Transpose,
            6 => Self::Rotate90,
            7 => Self::Transverse,
            8 => Self::Rotate270,
            _ => Self::Normal,
        }
    }
    /// True when showing the photo upright swaps its width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270)
    }
}

/// What the TIFF directories say about previews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedPreview {
    /// The largest JPEG found, by byte length.
    pub jpeg: JpegLocation,
    pub orientation: Orientation,
    /// Width and height of the largest image any directory describes
    /// (usually the sensor data), before orientation. Gives the photo's shape
    /// without decoding anything.
    pub size: Option<(u32, u32)>,
}

#[derive(Debug)]
pub enum PreviewError {
    Io(io::Error),
    NotTiff,
    NoPreview,
}

impl From<io::Error> for PreviewError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "read failed: {error}"),
            Self::NotTiff => write!(f, "not a TIFF-based file"),
            Self::NoPreview => write!(f, "no embedded JPEG preview"),
        }
    }
}

impl std::error::Error for PreviewError {}

const TAG_IMAGE_WIDTH: u16 = 0x0100;
const TAG_IMAGE_LENGTH: u16 = 0x0101;
const TAG_COMPRESSION: u16 = 0x0103;
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x0117;
const TAG_SUB_IFDS: u16 = 0x014a;
const TAG_JPEG_OFFSET: u16 = 0x0201;
const TAG_JPEG_LENGTH: u16 = 0x0202;

/// Old-style (6) and new-style (7) JPEG compression in TIFF.
const COMPRESSION_JPEG: [u32; 2] = [6, 7];

/// Walks every TIFF directory reachable from the header (the IFD chain and
/// SubIFDs) and returns the largest embedded JPEG and the photo's size.
pub fn find_preview<R: Read + Seek>(reader: &mut R) -> Result<EmbeddedPreview, PreviewError> {
    let (order, first_ifd) = tiff::read_header(reader)?.ok_or(PreviewError::NotTiff)?;

    let mut pending = vec![first_ifd];
    let mut visited = Vec::new();
    let mut best: Option<JpegLocation> = None;
    let mut orientation: Option<Orientation> = None;
    let mut size: Option<(u32, u32)> = None;

    while let Some(offset) = pending.pop() {
        if offset == 0 || visited.contains(&offset) || visited.len() >= tiff::MAX_IFDS {
            continue;
        }
        visited.push(offset);
        let dir = tiff::read_directory(reader, order, offset)?;
        let single = |tag| dir.single(order, tag);

        // IFD0's orientation applies to the whole image. Later directories
        // sometimes carry their own, but those describe the raw data.
        if orientation.is_none() {
            orientation = single(TAG_ORIENTATION).map(Orientation::from_exif);
        }

        let mut jpegs = Vec::new();
        if let (Some(offset), Some(length)) = (single(TAG_JPEG_OFFSET), single(TAG_JPEG_LENGTH)) {
            jpegs.push(JpegLocation { offset: offset.into(), length: length.into() });
        }
        // DNG often stores its preview as a single JPEG-compressed strip instead.
        if single(TAG_COMPRESSION).is_some_and(|c| COMPRESSION_JPEG.contains(&c))
            && let (Some(offset), Some(length)) = (single(TAG_STRIP_OFFSETS), single(TAG_STRIP_BYTE_COUNTS))
        {
            jpegs.push(JpegLocation { offset: offset.into(), length: length.into() });
        }
        for jpeg in jpegs.into_iter().filter(|j| j.length > 0) {
            if best.is_none_or(|b| jpeg.length > b.length) {
                best = Some(jpeg);
            }
        }

        if let (Some(w), Some(h)) = (single(TAG_IMAGE_WIDTH), single(TAG_IMAGE_LENGTH))
            && w > 0
            && h > 0
            && size.is_none_or(|(bw, bh)| u64::from(w) * u64::from(h) > u64::from(bw) * u64::from(bh))
        {
            size = Some((w, h));
        }

        if let Some(entry) = dir.find(TAG_SUB_IFDS) {
            pending.extend(tiff::read_longs(reader, order, entry)?);
        }
        pending.push(dir.next);
    }

    best.map(|jpeg| EmbeddedPreview { jpeg, orientation: orientation.unwrap_or_default(), size })
        .ok_or(PreviewError::NoPreview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiff::{KIND_LONG, KIND_SHORT};
    use std::io::Cursor;

    /// Builds a little-endian TIFF. Each IFD is a list of (tag, kind, count,
    /// value) entries; IFDs are laid out back to back and chained in order
    /// unless `chain` is false.
    struct TiffBuilder {
        bytes: Vec<u8>,
    }

    impl TiffBuilder {
        fn new() -> Self {
            let mut bytes = b"II*\0".to_vec();
            bytes.extend_from_slice(&8u32.to_le_bytes());
            Self { bytes }
        }

        /// Appends an IFD at the current end and returns its offset.
        fn ifd(&mut self, entries: &[(u16, u16, u32, u32)], next: u32) -> u32 {
            let offset = self.bytes.len() as u32;
            self.bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for &(tag, kind, count, value) in entries {
                self.bytes.extend_from_slice(&tag.to_le_bytes());
                self.bytes.extend_from_slice(&kind.to_le_bytes());
                self.bytes.extend_from_slice(&count.to_le_bytes());
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            self.bytes.extend_from_slice(&next.to_le_bytes());
            offset
        }

        fn len(&self) -> u32 {
            self.bytes.len() as u32
        }
    }

    #[test]
    fn picks_the_largest_jpeg_across_ifd0_and_sub_ifds() {
        // The layout of a Nikon NEF: IFD0 with a small JPEG and a SubIFD
        // pointer, SubIFD0 with the full-size JPEG.
        let mut tiff = TiffBuilder::new();
        let ifd0_size = 2 + 6 * 12 + 4;
        let sub_ifd = 8 + ifd0_size;
        tiff.ifd(
            &[
                (TAG_IMAGE_WIDTH, KIND_SHORT, 1, 160),
                (TAG_IMAGE_LENGTH, KIND_SHORT, 1, 120),
                (TAG_ORIENTATION, KIND_SHORT, 1, 6),
                (TAG_SUB_IFDS, KIND_LONG, 1, sub_ifd),
                (TAG_JPEG_OFFSET, KIND_LONG, 1, 1000),
                (TAG_JPEG_LENGTH, KIND_LONG, 1, 50),
            ],
            0,
        );
        assert_eq!(tiff.len(), sub_ifd);
        tiff.ifd(
            &[
                (TAG_IMAGE_WIDTH, KIND_LONG, 1, 6000),
                (TAG_IMAGE_LENGTH, KIND_LONG, 1, 4000),
                (TAG_JPEG_OFFSET, KIND_LONG, 1, 2000),
                (TAG_JPEG_LENGTH, KIND_LONG, 1, 900),
            ],
            0,
        );

        let preview = find_preview(&mut Cursor::new(tiff.bytes)).unwrap();
        assert_eq!(preview.jpeg, JpegLocation { offset: 2000, length: 900 });
        assert_eq!(preview.orientation, Orientation::Rotate90);
        assert_eq!(preview.size, Some((6000, 4000)), "the sensor size, not the small thumbnail's");
    }

    #[test]
    fn reads_a_jpeg_compressed_strip_as_in_dng() {
        let mut tiff = TiffBuilder::new();
        tiff.ifd(
            &[
                (TAG_COMPRESSION, KIND_SHORT, 1, 7),
                (TAG_STRIP_OFFSETS, KIND_LONG, 1, 4096),
                (TAG_STRIP_BYTE_COUNTS, KIND_LONG, 1, 300),
            ],
            0,
        );
        let preview = find_preview(&mut Cursor::new(tiff.bytes)).unwrap();
        assert_eq!(preview.jpeg, JpegLocation { offset: 4096, length: 300 });
        assert_eq!(preview.orientation, Orientation::Normal);
    }

    #[test]
    fn ignores_uncompressed_strips() {
        // IFD0 of a NEF holds an uncompressed RGB thumbnail as a strip.
        let mut tiff = TiffBuilder::new();
        tiff.ifd(
            &[
                (TAG_COMPRESSION, KIND_SHORT, 1, 1),
                (TAG_STRIP_OFFSETS, KIND_LONG, 1, 4096),
                (TAG_STRIP_BYTE_COUNTS, KIND_LONG, 1, 300),
            ],
            0,
        );
        assert!(matches!(find_preview(&mut Cursor::new(tiff.bytes)), Err(PreviewError::NoPreview)));
    }

    #[test]
    fn survives_an_ifd_chain_that_loops_back_on_itself() {
        let mut tiff = TiffBuilder::new();
        tiff.ifd(&[(TAG_JPEG_OFFSET, KIND_LONG, 1, 100), (TAG_JPEG_LENGTH, KIND_LONG, 1, 10)], 8);
        let preview = find_preview(&mut Cursor::new(tiff.bytes)).unwrap();
        assert_eq!(preview.jpeg.length, 10);
    }

    #[test]
    fn rejects_files_that_are_not_tiff() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0];
        assert!(matches!(find_preview(&mut Cursor::new(jpeg)), Err(PreviewError::NotTiff)));
    }
}
