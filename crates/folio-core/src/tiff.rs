//! Just enough TIFF to walk directories and read values. RAW files (NEF, DNG,
//! ARW, CR2, PEF) are TIFF containers, and so is the EXIF block inside JPEGs.
//! `preview` and `exif` build on this.

use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Copy, Debug)]
pub(crate) enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    pub(crate) fn u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }

    pub(crate) fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }
}

/// One directory entry as stored: the value is inline if it fits in four
/// bytes, otherwise `value` holds an offset to it.
pub(crate) struct Entry {
    pub(crate) tag: u16,
    pub(crate) kind: u16,
    pub(crate) count: u32,
    pub(crate) value: [u8; 4],
}

pub(crate) struct Directory {
    pub(crate) entries: Vec<Entry>,
    pub(crate) next: u64,
}

// TIFF type codes.
pub(crate) const KIND_ASCII: u16 = 2;
pub(crate) const KIND_SHORT: u16 = 3;
pub(crate) const KIND_LONG: u16 = 4;
pub(crate) const KIND_RATIONAL: u16 = 5;
pub(crate) const KIND_SRATIONAL: u16 = 10;
pub(crate) const KIND_IFD: u16 = 13;

// Real files have a handful of directories with a few dozen entries each.
// These caps stop a corrupt or hostile file from looping or allocating forever.
pub(crate) const MAX_IFDS: usize = 32;
const MAX_ENTRIES: u16 = 1024;
const MAX_ASCII: u32 = 256;

/// Reads the 8-byte header at the reader's start. `None` if it isn't TIFF.
pub(crate) fn read_header<R: Read + Seek>(reader: &mut R) -> io::Result<Option<(ByteOrder, u64)>> {
    let mut header = [0u8; 8];
    reader.seek(SeekFrom::Start(0))?;
    if reader.read_exact(&mut header).is_err() {
        return Ok(None);
    }
    let order = match &header[0..4] {
        b"II*\0" => ByteOrder::Little,
        b"MM\0*" => ByteOrder::Big,
        _ => return Ok(None),
    };
    Ok(Some((order, u64::from(order.u32([header[4], header[5], header[6], header[7]])))))
}

pub(crate) fn read_directory<R: Read + Seek>(reader: &mut R, order: ByteOrder, offset: u64) -> io::Result<Directory> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut count = [0u8; 2];
    reader.read_exact(&mut count)?;
    let count = order.u16(count).min(MAX_ENTRIES);

    let mut entries = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let mut raw = [0u8; 12];
        reader.read_exact(&mut raw)?;
        entries.push(Entry {
            tag: order.u16([raw[0], raw[1]]),
            kind: order.u16([raw[2], raw[3]]),
            count: order.u32([raw[4], raw[5], raw[6], raw[7]]),
            value: [raw[8], raw[9], raw[10], raw[11]],
        });
    }
    let mut next = [0u8; 4];
    reader.read_exact(&mut next)?;
    Ok(Directory { entries, next: u64::from(order.u32(next)) })
}

impl Directory {
    pub(crate) fn find(&self, tag: u16) -> Option<&Entry> {
        self.entries.iter().find(|e| e.tag == tag)
    }

    /// An entry holding exactly one SHORT or LONG.
    pub(crate) fn single(&self, order: ByteOrder, tag: u16) -> Option<u32> {
        let entry = self.find(tag)?;
        if entry.count != 1 {
            return None;
        }
        match entry.kind {
            KIND_SHORT => Some(u32::from(order.u16([entry.value[0], entry.value[1]]))),
            KIND_LONG | KIND_IFD => Some(order.u32(entry.value)),
            _ => None,
        }
    }
}

/// Reads an array of LONG offsets, inline if there's one and out of line if
/// there are more.
pub(crate) fn read_longs<R: Read + Seek>(reader: &mut R, order: ByteOrder, entry: &Entry) -> io::Result<Vec<u64>> {
    if !matches!(entry.kind, KIND_LONG | KIND_IFD) {
        return Ok(Vec::new());
    }
    if entry.count == 1 {
        return Ok(vec![u64::from(order.u32(entry.value))]);
    }
    let count = entry.count.min(MAX_IFDS as u32);
    reader.seek(SeekFrom::Start(u64::from(order.u32(entry.value))))?;
    let mut longs = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut raw = [0u8; 4];
        reader.read_exact(&mut raw)?;
        longs.push(u64::from(order.u32(raw)));
    }
    Ok(longs)
}

/// Reads an ASCII entry, trimmed of its NUL terminator and padding.
pub(crate) fn read_ascii<R: Read + Seek>(reader: &mut R, order: ByteOrder, entry: &Entry) -> io::Result<Option<String>> {
    if entry.kind != KIND_ASCII || entry.count == 0 || entry.count > MAX_ASCII {
        return Ok(None);
    }
    let bytes = if entry.count <= 4 {
        entry.value[..entry.count as usize].to_vec()
    } else {
        reader.seek(SeekFrom::Start(u64::from(order.u32(entry.value))))?;
        let mut bytes = vec![0u8; entry.count as usize];
        reader.read_exact(&mut bytes)?;
        bytes
    };
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_end_matches('\0').trim();
    Ok((!text.is_empty()).then(|| text.to_string()))
}

/// Reads a single unsigned RATIONAL as (numerator, denominator).
pub(crate) fn read_rational<R: Read + Seek>(reader: &mut R, order: ByteOrder, entry: &Entry) -> io::Result<Option<(u32, u32)>> {
    if entry.kind != KIND_RATIONAL || entry.count < 1 {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(u64::from(order.u32(entry.value))))?;
    let mut raw = [0u8; 8];
    reader.read_exact(&mut raw)?;
    let numerator = order.u32([raw[0], raw[1], raw[2], raw[3]]);
    let denominator = order.u32([raw[4], raw[5], raw[6], raw[7]]);
    Ok((denominator != 0).then_some((numerator, denominator)))
}

/// Reads up to `max` RATIONAL or SRATIONAL values, such as a GPS
/// coordinate's degrees, minutes and seconds. Signed values keep their sign
/// in the numerator. Values with a zero denominator are dropped.
pub(crate) fn read_rationals<R: Read + Seek>(reader: &mut R, order: ByteOrder, entry: &Entry, max: u32) -> io::Result<Vec<(i64, u32)>> {
    if !matches!(entry.kind, KIND_RATIONAL | KIND_SRATIONAL) || entry.count < 1 {
        return Ok(Vec::new());
    }
    reader.seek(SeekFrom::Start(u64::from(order.u32(entry.value))))?;
    let mut values = Vec::new();
    for _ in 0..entry.count.min(max) {
        let mut raw = [0u8; 8];
        reader.read_exact(&mut raw)?;
        let (numerator, denominator) = (order.u32([raw[0], raw[1], raw[2], raw[3]]), order.u32([raw[4], raw[5], raw[6], raw[7]]));
        let value = if entry.kind == KIND_SRATIONAL {
            // Both halves are signed: move the sign onto the numerator.
            let (n, d) = (i64::from(numerator as i32), i64::from(denominator as i32));
            (d.signum() * n, d.unsigned_abs() as u32)
        } else {
            (i64::from(numerator), denominator)
        };
        if value.1 != 0 {
            values.push(value);
        }
    }
    Ok(values)
}

/// A view of a reader that starts at `base`, so a TIFF block embedded in a
/// JPEG can be parsed with offsets relative to its own header.
pub(crate) struct Embedded<R> {
    inner: R,
    base: u64,
}

impl<R: Seek> Embedded<R> {
    pub(crate) fn new(mut inner: R, base: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(base))?;
        Ok(Self { inner, base })
    }
}

impl<R: Read> Read for Embedded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for Embedded<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(offset) => SeekFrom::Start(self.base + offset),
            other => other,
        };
        Ok(self.inner.seek(pos)?.saturating_sub(self.base))
    }
}
