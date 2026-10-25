//! Turns a photo file into a small RGBA thumbnail.
//!
//! RAW files go through their embedded JPEG (see `preview`). The JPEG is
//! decoded at a reduced scale (libjpeg-style DCT scaling), so a 20 MP preview
//! never gets decoded at full size just to end up as a 300 px tile.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use image::{RgbaImage, imageops};

use crate::preview::{self, Orientation, PreviewError};

#[derive(Debug)]
pub enum ThumbnailError {
    Io(io::Error),
    Preview(PreviewError),
    Decode(jpeg_decoder::Error),
    UnsupportedPixelFormat,
}

impl std::fmt::Display for ThumbnailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "read failed: {error}"),
            Self::Preview(error) => write!(f, "{error}"),
            Self::Decode(error) => write!(f, "JPEG decode failed: {error}"),
            Self::UnsupportedPixelFormat => write!(f, "unsupported JPEG pixel format"),
        }
    }
}

impl std::error::Error for ThumbnailError {}

impl From<io::Error> for ThumbnailError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Loads a thumbnail whose longer edge is at most `max_edge` pixels, already
/// rotated upright. Blocking: call it off the UI thread.
pub fn load_thumbnail(path: &Path, max_edge: u32) -> Result<RgbaImage, ThumbnailError> {
    let mut file = BufReader::new(File::open(path)?);
    let (jpeg, orientation) = read_jpeg(&mut file)?;
    decode_jpeg(&jpeg, max_edge).map(|image| orient(image, orientation))
}

/// Returns the bytes of the JPEG to show: the file itself if it's a JPEG, or
/// the largest preview embedded in a TIFF-based RAW.
fn read_jpeg<R: Read + Seek>(reader: &mut R) -> Result<(Vec<u8>, Orientation), ThumbnailError> {
    let mut magic = [0u8; 2];
    reader.read_exact(&mut magic)?;
    if magic == [0xff, 0xd8] {
        // Plain JPEG. Phones store portrait shots sideways plus an EXIF
        // orientation, so read that before taking the bytes.
        let orientation = crate::exif::read_jpeg_exif(reader).ok().and_then(|e| e.orientation).map_or(Orientation::Normal, Orientation::from_exif);
        reader.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        return Ok((bytes, orientation));
    }

    let found = preview::find_preview(reader).map_err(ThumbnailError::Preview)?;
    reader.seek(SeekFrom::Start(found.jpeg.offset))?;
    let mut bytes = Vec::new();
    reader.take(found.jpeg.length).read_to_end(&mut bytes)?;
    Ok((bytes, found.orientation))
}

/// Decodes JPEG bytes to RGBA, at most `max_edge` on the longer side.
pub(crate) fn decode_jpeg(bytes: &[u8], max_edge: u32) -> Result<RgbaImage, ThumbnailError> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    // Ask for the smallest DCT scale that still covers `max_edge` on the
    // short side, so the final resize only ever shrinks.
    let request = u16::try_from(max_edge).unwrap_or(u16::MAX);
    decoder.read_info().map_err(ThumbnailError::Decode)?;
    let (width, height) = decoder.scale(request, request).map_err(ThumbnailError::Decode)?;
    let pixels = decoder.decode().map_err(ThumbnailError::Decode)?;
    let format = decoder.info().map(|info| info.pixel_format);

    let rgba: Vec<u8> = match format {
        Some(jpeg_decoder::PixelFormat::RGB24) => {
            pixels.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect()
        }
        Some(jpeg_decoder::PixelFormat::L8) => pixels.iter().flat_map(|&l| [l, l, l, 255]).collect(),
        _ => return Err(ThumbnailError::UnsupportedPixelFormat),
    };
    let image = RgbaImage::from_raw(width.into(), height.into(), rgba)
        .ok_or(ThumbnailError::UnsupportedPixelFormat)?;

    let (fit_width, fit_height) = fit_within(image.width(), image.height(), max_edge);
    if (fit_width, fit_height) == image.dimensions() {
        return Ok(image);
    }
    Ok(imageops::thumbnail(&image, fit_width, fit_height))
}

/// Scales `width` x `height` down so the longer edge is at most `max_edge`,
/// keeping the aspect ratio. Never scales up.
fn fit_within(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let longer = width.max(height);
    if longer <= max_edge || longer == 0 {
        return (width, height);
    }
    let scale = |side: u32| ((u64::from(side) * u64::from(max_edge)) / u64::from(longer)).max(1) as u32;
    (scale(width), scale(height))
}

fn orient(image: RgbaImage, orientation: Orientation) -> RgbaImage {
    match orientation {
        Orientation::Normal => image,
        Orientation::FlipHorizontal => imageops::flip_horizontal(&image),
        Orientation::Rotate180 => imageops::rotate180(&image),
        Orientation::FlipVertical => imageops::flip_vertical(&image),
        Orientation::Transpose => imageops::flip_horizontal(&imageops::rotate90(&image)),
        Orientation::Rotate90 => imageops::rotate90(&image),
        Orientation::Transverse => imageops::flip_horizontal(&imageops::rotate270(&image)),
        Orientation::Rotate270 => imageops::rotate270(&image),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;
    use std::io::Cursor;

    fn encode_jpeg(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 100, 50]));
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes).encode_image(&image).unwrap();
        bytes
    }

    /// A minimal NEF-shaped TIFF: IFD0 with orientation and one embedded JPEG.
    fn raw_with_preview(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let entries: [(u16, u16, u32, u32); 3] = [
            (0x0112, 3, 1, u32::from(orientation)),
            (0x0201, 4, 1, 0), // patched below
            (0x0202, 4, 1, jpeg.len() as u32),
        ];
        let jpeg_offset = 8 + 2 + entries.len() as u32 * 12 + 4;
        let mut bytes = b"II*\0".to_vec();
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            let value = if tag == 0x0201 { jpeg_offset } else { value };
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(jpeg);
        bytes
    }

    #[test]
    fn decodes_an_embedded_preview_upright_and_shrunk() {
        let raw = raw_with_preview(&encode_jpeg(1600, 1000), 6);
        let (jpeg, orientation) = read_jpeg(&mut Cursor::new(raw)).unwrap();
        let thumb = orient(decode_jpeg(&jpeg, 320).unwrap(), orientation);
        // Landscape sensor data, rotated 90 degrees by the orientation tag.
        assert_eq!(thumb.dimensions(), (200, 320));
        assert_eq!(thumb.get_pixel(100, 160)[3], 255);
    }

    #[test]
    fn reads_a_plain_jpeg_file() {
        let (jpeg, orientation) = read_jpeg(&mut Cursor::new(encode_jpeg(400, 300))).unwrap();
        assert_eq!(orientation, Orientation::Normal);
        assert_eq!(decode_jpeg(&jpeg, 200).unwrap().dimensions(), (200, 150));
    }

    #[test]
    fn a_phone_jpeg_comes_out_upright() {
        // A portrait phone shot: landscape pixels plus EXIF orientation 6 in
        // an APP1 segment right after SOI.
        let plain = encode_jpeg(400, 300);
        let tiff = raw_with_preview(&[], 6);
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&plain[2..]);

        let (bytes, orientation) = read_jpeg(&mut Cursor::new(jpeg)).unwrap();
        assert_eq!(orientation, Orientation::Rotate90);
        assert_eq!(orient(decode_jpeg(&bytes, 400).unwrap(), orientation).dimensions(), (300, 400));
    }

    #[test]
    fn small_images_are_not_scaled_up() {
        assert_eq!(fit_within(100, 50, 320), (100, 50));
        assert_eq!(fit_within(6000, 4000, 300), (300, 200));
    }
}
