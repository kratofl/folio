//! Videos shot during an outing. Folio doesn't show them, but moves them
//! with the outing's photos so a day's clips don't stay behind in the date
//! folder. Their capture time comes from the QuickTime movie header, which
//! MOV and MP4 share.
//!
//! The header's time is UTC, while photo times are local without a zone. To
//! compare them, a video takes the time zone of the photo shot closest to
//! it, from the photo's EXIF offset. That's right while traveling, as long
//! as the camera knows its zone. Only without such a photo nearby does the
//! computer's own time zone stand in.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{Datelike, TimeZone, Timelike};

use crate::exif::CaptureTime;

/// Seconds from 1904, where QuickTime counts from, to 1970.
const QUICKTIME_TO_UNIX: i64 = 2_082_844_800;

/// A photo further away than this in time doesn't lend a video its zone.
const NEARBY_SECONDS: i64 = 24 * 3600;

/// When the video was shot, in Unix seconds (UTC). `None` if the file has no
/// usable movie header.
pub fn read_video_utc(path: &Path) -> Option<i64> {
    let mut file = BufReader::new(File::open(path).ok()?);
    let created = movie_created(&mut file).ok()??;
    Some(i64::try_from(created).ok()? - QUICKTIME_TO_UNIX)
}

/// A video's local time, comparable with photo times. `zoned_photos` are
/// the photos that know their zone, as (UTC seconds, minutes east of UTC).
pub fn local_video_time(utc: i64, zoned_photos: &[(i64, i32)]) -> Option<CaptureTime> {
    match offset_near(utc, zoned_photos) {
        Some(minutes) => local_time(utc, &chrono::FixedOffset::east_opt(minutes * 60)?),
        None => local_time(utc, &chrono::Local),
    }
}

/// The zone of the photo closest in time, if one is close enough. Pure.
fn offset_near(utc: i64, zoned_photos: &[(i64, i32)]) -> Option<i32> {
    zoned_photos
        .iter()
        .map(|&(photo, offset)| ((photo - utc).abs(), offset))
        .filter(|&(distance, _)| distance <= NEARBY_SECONDS)
        .min_by_key(|&(distance, _)| distance)
        .map(|(_, offset)| offset)
}

/// Unix seconds as a calendar time in `zone`.
fn local_time<Tz: TimeZone>(unix: i64, zone: &Tz) -> Option<CaptureTime> {
    let local = zone.timestamp_opt(unix, 0).single()?.naive_local();
    Some(CaptureTime {
        year: u16::try_from(local.year()).ok()?,
        month: u8::try_from(local.month()).ok()?,
        day: u8::try_from(local.day()).ok()?,
        hour: u8::try_from(local.hour()).ok()?,
        minute: u8::try_from(local.minute()).ok()?,
        second: u8::try_from(local.second()).ok()?,
    })
}

/// Finds `moov/mvhd` and returns its creation time in seconds since 1904.
/// `None` for a zero time, which means the camera didn't set it. Skips over
/// other atoms by seeking, so a 4 GB `mdat` costs nothing.
fn movie_created<R: Read + Seek>(reader: &mut R) -> io::Result<Option<u64>> {
    let Some(moov_end) = find_atom(reader, b"moov", u64::MAX)? else { return Ok(None) };
    if find_atom(reader, b"mvhd", moov_end)?.is_none() {
        return Ok(None);
    }
    let mut version = [0u8; 4];
    reader.read_exact(&mut version)?;
    let created = if version[0] == 1 {
        let mut bytes = [0u8; 8];
        reader.read_exact(&mut bytes)?;
        u64::from_be_bytes(bytes)
    } else {
        let mut bytes = [0u8; 4];
        reader.read_exact(&mut bytes)?;
        u64::from(u32::from_be_bytes(bytes))
    };
    Ok((created != 0).then_some(created))
}

/// Walks atoms from the reader's position up to `end` and stops inside the
/// first one called `name`, returning where it ends.
fn find_atom<R: Read + Seek>(reader: &mut R, name: &[u8; 4], end: u64) -> io::Result<Option<u64>> {
    let mut start = reader.stream_position()?;
    while start + 8 <= end {
        let mut header = [0u8; 8];
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        let size = u64::from(u32::from_be_bytes([header[0], header[1], header[2], header[3]]));
        let size = match size {
            // A 64-bit size follows the name.
            1 => {
                let mut large = [0u8; 8];
                reader.read_exact(&mut large)?;
                u64::from_be_bytes(large)
            }
            // The atom runs to the end of the file.
            0 => end.saturating_sub(start),
            size => size,
        };
        if size < 8 {
            return Ok(None);
        }
        let atom_end = start.saturating_add(size);
        if &header[4..] == name {
            return Ok(Some(atom_end.min(end)));
        }
        start = atom_end;
        reader.seek(SeekFrom::Start(start))?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn atom(name: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(body);
        bytes
    }

    /// A movie shaped like a camera's: `ftyp`, a big `mdat`, then `moov`
    /// holding a track before the movie header.
    fn movie(mvhd_body: &[u8]) -> Vec<u8> {
        let mut bytes = atom(b"ftyp", b"qt  \0\0\0\0");
        bytes.extend(atom(b"mdat", &[0u8; 1000]));
        let mut moov = atom(b"trak", &[0u8; 20]);
        moov.extend(atom(b"mvhd", mvhd_body));
        bytes.extend(atom(b"moov", &moov));
        bytes
    }

    #[test]
    fn reads_the_creation_time_from_either_header_version() {
        // Version 0: four-byte times. 0xe33a7d38 is 2024-10-20 09:26:16 UTC,
        // from a Nikon Z fc clip.
        let v0 = [&[0u8, 0, 0, 0][..], &0xe33a_7d38u32.to_be_bytes(), &[0u8; 20]].concat();
        assert_eq!(movie_created(&mut Cursor::new(movie(&v0))).unwrap(), Some(0xe33a_7d38));

        let v1 = [&[1u8, 0, 0, 0][..], &0xe33a_7d38u64.to_be_bytes(), &[0u8; 20]].concat();
        assert_eq!(movie_created(&mut Cursor::new(movie(&v1))).unwrap(), Some(0xe33a_7d38));

        let unset = [&[0u8; 8][..], &[0u8; 20]].concat();
        assert_eq!(movie_created(&mut Cursor::new(movie(&unset))).unwrap(), None);
        assert_eq!(movie_created(&mut Cursor::new(atom(b"ftyp", b"isom"))).unwrap(), None);
    }

    #[test]
    fn a_video_takes_the_zone_of_the_photo_shot_closest_to_it() {
        // 2024-10-20 09:26:16 UTC, from a Nikon Z fc clip.
        let utc = 0xe33a_7d38 - QUICKTIME_TO_UNIX;
        // A photo two minutes earlier in Stuttgart (UTC+2), one a week
        // earlier in New York (UTC-4).
        let photos = [(utc - 120, 120), (utc - 7 * 86_400, -240)];
        let time = local_video_time(utc, &photos).unwrap();
        assert_eq!((time.year, time.month, time.day, time.hour, time.minute, time.second), (2024, 10, 20, 11, 26, 16));

        assert_eq!(offset_near(utc, &[(utc + 3600, -240), (utc - 600, 540)]), Some(540), "the closer photo wins");
        assert_eq!(offset_near(utc, &[(utc - 3 * 86_400, 120)]), None, "too far away to say");
    }
}
