//! Reads the EXIF fields Folio shows and groups by: when the photo was taken,
//! with what, and how. Works on TIFF-based RAWs directly and on the TIFF block
//! inside a JPEG's APP1 segment.

use std::io::{self, Read, Seek};

use crate::tiff::{self, ByteOrder, Directory};

/// A capture time as the camera's clock recorded it. EXIF stores local time
/// without a zone, so this is a plain calendar time. Fields are ordered so
/// the derived ordering is chronological.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CaptureTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl CaptureTime {
    /// Parses EXIF's `YYYY:MM:DD HH:MM:SS`. Cameras with a reset clock write
    /// zeros or garbage; those come back as `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() < 19 {
            return None;
        }
        let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<u16>().ok();
        let time = Self {
            year: number(0..4)?,
            month: u8::try_from(number(5..7)?).ok()?,
            day: u8::try_from(number(8..10)?).ok()?,
            hour: u8::try_from(number(11..13)?).ok()?,
            minute: u8::try_from(number(14..16)?).ok()?,
            second: u8::try_from(number(17..19)?).ok()?,
        };
        let valid = time.year >= 1900
            && (1..=12).contains(&time.month)
            && (1..=31).contains(&time.day)
            && time.hour < 24
            && time.minute < 60
            && time.second < 61;
        valid.then_some(time)
    }

    /// Seconds since 1970-01-01 in the camera's local time. Only differences
    /// between two of these mean anything, which is all grouping needs.
    pub fn seconds(self) -> i64 {
        let days = days_from_civil(i64::from(self.year), i64::from(self.month), i64::from(self.day));
        days * 86_400 + i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second)
    }
}

impl std::fmt::Display for CaptureTime {
    /// `Oct 20 2024, 14:02`
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let month = MONTHS[usize::from(self.month.clamp(1, 12)) - 1];
        write!(f, "{month} {} {}, {:02}:{:02}", self.day, self.year, self.hour, self.minute)
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The EXIF fields Folio uses. Each one is independently optional: plenty of
/// files carry a time but no lens, or a camera but no time. Values are kept
/// as stored, and the methods below word them for people.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exif {
    pub captured: Option<CaptureTime>,
    /// Minutes east of UTC when the photo was taken (OffsetTimeOriginal).
    /// Cameras that know their time zone write it, many don't.
    pub utc_offset: Option<i32>,
    pub make: Option<String>,
    pub camera: Option<String>,
    pub serial: Option<String>,
    pub lens: Option<String>,
    /// Exposure time in seconds, as a fraction.
    pub exposure: Option<(u32, u32)>,
    pub f_number: Option<(u32, u32)>,
    pub focal_length: Option<(u32, u32)>,
    /// The focal length that would give the same view on full frame.
    pub focal_length_35: Option<u32>,
    pub iso: Option<u32>,
    /// Exposure compensation in EV, as a signed fraction.
    pub exposure_bias: Option<(i64, u32)>,
    /// EXIF ExposureProgram, MeteringMode, Flash and WhiteBalance codes.
    pub exposure_program: Option<u32>,
    pub metering: Option<u32>,
    pub flash: Option<u32>,
    pub white_balance: Option<u32>,
    /// Latitude and longitude in degrees, north and east positive.
    pub gps: Option<(f64, f64)>,
    pub software: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    /// How to turn the pixels upright (EXIF Orientation, 1 to 8).
    pub orientation: Option<u32>,
}

impl Exif {
    /// The shooting details as one line, such as
    /// `NIKON Z fc · 35 mm · 1/400 s · f/8 · ISO 100`. Missing fields are left out.
    pub fn summary(&self) -> String {
        [self.camera.clone(), self.focal(), self.shutter(), self.aperture(), self.iso.map(|iso| format!("ISO {iso}"))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// `1/400 s` or `2 s`.
    pub fn shutter(&self) -> Option<String> {
        let (n, d) = self.exposure?;
        let seconds = decimal((n, d))?;
        Some(if seconds >= 1. || n == 0 { format!("{} s", trim(seconds)) } else { format!("1/{} s", (1. / seconds).round()) })
    }

    /// `f/3.5`
    pub fn aperture(&self) -> Option<String> {
        self.f_number.and_then(decimal).map(|f| format!("f/{}", trim(f)))
    }

    /// `35 mm`
    pub fn focal(&self) -> Option<String> {
        self.focal_length.and_then(decimal).map(|mm| format!("{} mm", trim(mm)))
    }

    /// `+0.7 EV`, `-1 EV` or `0 EV`.
    pub fn bias(&self) -> Option<String> {
        let (n, d) = self.exposure_bias?;
        let ev = n as f64 / f64::from(d.max(1));
        let ev = (ev * 10.).round() / 10.;
        Some(match ev {
            0. => "0 EV".to_owned(),
            ev if ev > 0. => format!("+{} EV", trim(ev)),
            ev => format!("{} EV", trim(ev)),
        })
    }

    /// `Aperture priority`
    pub fn program(&self) -> Option<&'static str> {
        Some(match self.exposure_program? {
            1 => "Manual",
            2 => "Program",
            3 => "Aperture priority",
            4 => "Shutter priority",
            5 => "Creative",
            6 => "Action",
            7 => "Portrait",
            8 => "Landscape",
            _ => return None,
        })
    }

    /// `Matrix`
    pub fn metering_mode(&self) -> Option<&'static str> {
        Some(match self.metering? {
            1 => "Average",
            2 => "Center-weighted",
            3 => "Spot",
            4 => "Multi-spot",
            5 => "Matrix",
            6 => "Partial",
            _ => return None,
        })
    }

    /// `Fired` or `Off`. Bit 0 of the Flash code says whether it fired.
    pub fn flash_fired(&self) -> Option<&'static str> {
        self.flash.map(|flash| if flash & 1 == 1 { "Fired" } else { "Off" })
    }

    /// `Auto` or `Manual`.
    pub fn white_balance_mode(&self) -> Option<&'static str> {
        Some(match self.white_balance? {
            0 => "Auto",
            1 => "Manual",
            _ => return None,
        })
    }

    /// `UTC+2` or `UTC-3:30`.
    pub fn zone(&self) -> Option<String> {
        let minutes = self.utc_offset?;
        let sign = if minutes < 0 { '-' } else { '+' };
        let (hours, rest) = (minutes.abs() / 60, minutes.abs() % 60);
        Some(if rest == 0 { format!("UTC{sign}{hours}") } else { format!("UTC{sign}{hours}:{rest:02}") })
    }

    /// `48.7758° N, 9.1829° E`
    pub fn location(&self) -> Option<String> {
        let (latitude, longitude) = self.gps?;
        let north = if latitude >= 0. { 'N' } else { 'S' };
        let east = if longitude >= 0. { 'E' } else { 'W' };
        Some(format!("{:.4}° {north}, {:.4}° {east}", latitude.abs(), longitude.abs()))
    }
}

fn decimal((n, d): (u32, u32)) -> Option<f64> {
    (d != 0).then(|| f64::from(n) / f64::from(d))
}

/// `3.5`, and `35` rather than `35.0`.
fn trim(value: f64) -> String {
    format!("{value:.1}").trim_end_matches(".0").to_owned()
}

/// `+02:00` into minutes east of UTC.
fn parse_offset(text: &str) -> Option<i32> {
    let (sign, rest) = match text.trim().as_bytes().first()? {
        b'+' => (1, &text.trim()[1..]),
        b'-' => (-1, &text.trim()[1..]),
        _ => return None,
    };
    let (hours, minutes) = rest.split_once(':')?;
    let (hours, minutes) = (hours.parse::<i32>().ok()?, minutes.parse::<i32>().ok()?);
    (hours <= 14 && minutes < 60).then_some(sign * (hours * 60 + minutes))
}

const TAG_MAKE: u16 = 0x010f;
const TAG_MODEL: u16 = 0x0110;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_SOFTWARE: u16 = 0x0131;
const TAG_DATE_TIME: u16 = 0x0132;
const TAG_ARTIST: u16 = 0x013b;
const TAG_COPYRIGHT: u16 = 0x8298;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_EXPOSURE_TIME: u16 = 0x829a;
const TAG_F_NUMBER: u16 = 0x829d;
const TAG_EXPOSURE_PROGRAM: u16 = 0x8822;
const TAG_ISO: u16 = 0x8827;
const TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
const TAG_OFFSET_TIME_ORIGINAL: u16 = 0x9011;
const TAG_EXPOSURE_BIAS: u16 = 0x9204;
const TAG_METERING: u16 = 0x9207;
const TAG_FLASH: u16 = 0x9209;
const TAG_FOCAL_LENGTH: u16 = 0x920a;
const TAG_WHITE_BALANCE: u16 = 0xa403;
const TAG_FOCAL_LENGTH_35: u16 = 0xa405;
const TAG_BODY_SERIAL: u16 = 0xa431;
const TAG_LENS_MODEL: u16 = 0xa434;
// In the GPS directory.
const TAG_GPS_LATITUDE_REF: u16 = 0x0001;
const TAG_GPS_LATITUDE: u16 = 0x0002;
const TAG_GPS_LONGITUDE_REF: u16 = 0x0003;
const TAG_GPS_LONGITUDE: u16 = 0x0004;

/// Reads EXIF from a TIFF stream (a RAW file, or the block inside a JPEG).
/// Returns empty `Exif` if the stream isn't TIFF.
pub fn read_tiff_exif<R: Read + Seek>(reader: &mut R) -> io::Result<Exif> {
    let Some((order, first_ifd)) = tiff::read_header(reader)? else { return Ok(Exif::default()) };
    let ifd0 = tiff::read_directory(reader, order, first_ifd)?;

    let mut exif = Exif {
        make: ascii(reader, order, &ifd0, TAG_MAKE)?,
        camera: ascii(reader, order, &ifd0, TAG_MODEL)?,
        captured: ascii(reader, order, &ifd0, TAG_DATE_TIME)?.and_then(|t| CaptureTime::parse(&t)),
        orientation: ifd0.single(order, TAG_ORIENTATION),
        software: ascii(reader, order, &ifd0, TAG_SOFTWARE)?,
        artist: ascii(reader, order, &ifd0, TAG_ARTIST)?,
        copyright: ascii(reader, order, &ifd0, TAG_COPYRIGHT)?,
        ..Exif::default()
    };

    if let Some(offset) = ifd0.single(order, TAG_EXIF_IFD) {
        let sub = tiff::read_directory(reader, order, u64::from(offset))?;
        // DateTimeOriginal is when the shutter fired. IFD0's DateTime can be
        // when a program last saved the file, so the original wins.
        if let Some(original) = ascii(reader, order, &sub, TAG_DATE_TIME_ORIGINAL)?.and_then(|t| CaptureTime::parse(&t)) {
            exif.captured = Some(original);
        }
        exif.lens = ascii(reader, order, &sub, TAG_LENS_MODEL)?;
        exif.exposure = rational(reader, order, &sub, TAG_EXPOSURE_TIME)?;
        exif.f_number = rational(reader, order, &sub, TAG_F_NUMBER)?;
        exif.focal_length = rational(reader, order, &sub, TAG_FOCAL_LENGTH)?;
        exif.iso = sub.single(order, TAG_ISO);
        exif.utc_offset = ascii(reader, order, &sub, TAG_OFFSET_TIME_ORIGINAL)?.and_then(|t| parse_offset(&t));
        exif.serial = ascii(reader, order, &sub, TAG_BODY_SERIAL)?;
        exif.focal_length_35 = sub.single(order, TAG_FOCAL_LENGTH_35);
        exif.exposure_program = sub.single(order, TAG_EXPOSURE_PROGRAM);
        exif.metering = sub.single(order, TAG_METERING);
        exif.flash = sub.single(order, TAG_FLASH);
        exif.white_balance = sub.single(order, TAG_WHITE_BALANCE);
        exif.exposure_bias = match sub.find(TAG_EXPOSURE_BIAS) {
            Some(entry) => tiff::read_rationals(reader, order, entry, 1)?.first().copied(),
            None => None,
        };
    }
    if let Some(offset) = ifd0.single(order, TAG_GPS_IFD) {
        let gps = tiff::read_directory(reader, order, u64::from(offset))?;
        exif.gps = read_gps(reader, order, &gps)?;
    }
    Ok(exif)
}

/// Finds the EXIF block in a JPEG and reads it. The reader must sit just past
/// the SOI marker. Returns empty `Exif` if there's none.
pub fn read_jpeg_exif<R: Read + Seek>(reader: &mut R) -> io::Result<Exif> {
    loop {
        let mut marker = [0u8; 4];
        if reader.read_exact(&mut marker).is_err() || marker[0] != 0xff {
            return Ok(Exif::default());
        }
        let length = u16::from_be_bytes([marker[2], marker[3]]);
        // EXIF lives in APP1, which comes before the image data. Anything
        // else that looks like image data means there's no EXIF.
        match marker[1] {
            0xe1 => {
                let mut header = [0u8; 6];
                reader.read_exact(&mut header)?;
                if &header == b"Exif\0\0" {
                    let base = reader.stream_position()?;
                    let mut embedded = tiff::Embedded::new(&mut *reader, base)?;
                    return read_tiff_exif(&mut embedded);
                }
                reader.seek(io::SeekFrom::Current(i64::from(length) - 8))?;
            }
            0xda | 0xd9 => return Ok(Exif::default()),
            _ => {
                reader.seek(io::SeekFrom::Current(i64::from(length) - 2))?;
            }
        }
    }
}

/// Latitude and longitude from the GPS directory. Each is three rationals,
/// degrees, minutes and seconds, plus a reference letter for the hemisphere.
fn read_gps<R: Read + Seek>(reader: &mut R, order: ByteOrder, gps: &Directory) -> io::Result<Option<(f64, f64)>> {
    let mut coordinate = |value: u16, reference: u16, negative: char| -> io::Result<Option<f64>> {
        let Some(entry) = gps.find(value) else { return Ok(None) };
        let parts = tiff::read_rationals(reader, order, entry, 3)?;
        if parts.len() != 3 {
            return Ok(None);
        }
        let [degrees, minutes, seconds] = [parts[0], parts[1], parts[2]].map(|(n, d)| n as f64 / f64::from(d));
        let flip = ascii(reader, order, gps, reference)?.is_some_and(|r| r.starts_with(negative));
        let degrees = degrees + minutes / 60. + seconds / 3600.;
        Ok(Some(if flip { -degrees } else { degrees }))
    };
    let latitude = coordinate(TAG_GPS_LATITUDE, TAG_GPS_LATITUDE_REF, 'S')?;
    let longitude = coordinate(TAG_GPS_LONGITUDE, TAG_GPS_LONGITUDE_REF, 'W')?;
    Ok(latitude.zip(longitude))
}

fn ascii<R: Read + Seek>(reader: &mut R, order: ByteOrder, dir: &Directory, tag: u16) -> io::Result<Option<String>> {
    match dir.find(tag) {
        Some(entry) => tiff::read_ascii(reader, order, entry),
        None => Ok(None),
    }
}

fn rational<R: Read + Seek>(reader: &mut R, order: ByteOrder, dir: &Directory, tag: u16) -> io::Result<Option<(u32, u32)>> {
    match dir.find(tag) {
        Some(entry) => tiff::read_rational(reader, order, entry),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_exif_times_and_rejects_reset_clocks() {
        let time = CaptureTime::parse("2024:10:20 14:02:33").unwrap();
        assert_eq!((time.year, time.month, time.day, time.hour, time.minute, time.second), (2024, 10, 20, 14, 2, 33));
        assert_eq!(CaptureTime::parse("0000:00:00 00:00:00"), None);
        assert_eq!(CaptureTime::parse("    :  :     :  :  "), None);
        assert_eq!(CaptureTime::parse("2024:10:20"), None);
    }

    #[test]
    fn seconds_count_across_month_and_year_ends() {
        let a = CaptureTime::parse("2024:12:31 23:59:00").unwrap();
        let b = CaptureTime::parse("2025:01:01 00:01:00").unwrap();
        assert_eq!(b.seconds() - a.seconds(), 120);
        let leap = CaptureTime::parse("2024:02:28 12:00:00").unwrap();
        let after = CaptureTime::parse("2024:03:01 12:00:00").unwrap();
        assert_eq!(after.seconds() - leap.seconds(), 2 * 86_400);
        assert_eq!(CaptureTime::parse("1970:01:01 00:00:00").unwrap().seconds(), 0);
    }

    #[test]
    fn summarises_shooting_details_for_people() {
        let exif = Exif {
            camera: Some("NIKON Z fc".into()),
            focal_length: Some((350, 10)),
            exposure: Some((10, 4000)),
            f_number: Some((35, 10)),
            iso: Some(100),
            ..Exif::default()
        };
        assert_eq!(exif.summary(), "NIKON Z fc · 35 mm · 1/400 s · f/3.5 · ISO 100");
        assert_eq!(Exif { exposure: Some((2, 1)), ..Exif::default() }.summary(), "2 s");
        assert_eq!(Exif::default().summary(), "");
        assert_eq!(CaptureTime::parse("2024:10:20 14:02:33").unwrap().to_string(), "Oct 20 2024, 14:02");
    }

    /// Little-endian TIFF with IFD0 (model, EXIF pointer) and an EXIF IFD
    /// (DateTimeOriginal, ISO, exposure). Values that don't fit inline go in a
    /// data area after the directories.
    fn tiff_with_exif() -> Vec<u8> {
        let ifd0 = 8u32;
        let ifd0_len = 2 + 2 * 12 + 4;
        let exif_ifd = ifd0 + ifd0_len;
        let exif_len = 2 + 3 * 12 + 4;
        let data = exif_ifd + exif_len;
        let model = b"NIKON Z fc\0";
        let date = b"2024:10:20 14:02:33\0";
        let model_at = data;
        let date_at = model_at + model.len() as u32;
        let exposure_at = date_at + date.len() as u32;

        let mut b = b"II*\0".to_vec();
        b.extend_from_slice(&ifd0.to_le_bytes());
        let entry = |b: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32| {
            b.extend_from_slice(&tag.to_le_bytes());
            b.extend_from_slice(&kind.to_le_bytes());
            b.extend_from_slice(&count.to_le_bytes());
            b.extend_from_slice(&value.to_le_bytes());
        };
        b.extend_from_slice(&2u16.to_le_bytes());
        entry(&mut b, TAG_MODEL, tiff::KIND_ASCII, model.len() as u32, model_at);
        entry(&mut b, TAG_EXIF_IFD, tiff::KIND_LONG, 1, exif_ifd);
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        entry(&mut b, TAG_EXPOSURE_TIME, tiff::KIND_RATIONAL, 1, exposure_at);
        entry(&mut b, TAG_ISO, tiff::KIND_SHORT, 1, 400);
        entry(&mut b, TAG_DATE_TIME_ORIGINAL, tiff::KIND_ASCII, date.len() as u32, date_at);
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(model);
        b.extend_from_slice(date);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&250u32.to_le_bytes());
        b
    }

    #[test]
    fn reads_exif_from_a_raw_file() {
        let exif = read_tiff_exif(&mut Cursor::new(tiff_with_exif())).unwrap();
        assert_eq!(exif.camera.as_deref(), Some("NIKON Z fc"));
        assert_eq!(exif.captured, CaptureTime::parse("2024:10:20 14:02:33"));
        assert_eq!(exif.exposure, Some((1, 250)));
        assert_eq!(exif.iso, Some(400));
        assert_eq!(exif.lens, None);
    }

    #[test]
    fn reads_exif_from_a_jpeg_app1_segment() {
        let tiff = tiff_with_exif();
        let mut jpeg = vec![0xff, 0xd8];
        // An APP0 segment first, as most JPEGs have one.
        jpeg.extend_from_slice(&[0xff, 0xe0, 0x00, 0x04, 0x00, 0x00]);
        jpeg.extend_from_slice(&[0xff, 0xe1]);
        jpeg.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);

        let mut reader = Cursor::new(jpeg);
        reader.set_position(2);
        let exif = read_jpeg_exif(&mut reader).unwrap();
        assert_eq!(exif.camera.as_deref(), Some("NIKON Z fc"));
        assert_eq!(exif.iso, Some(400));
    }

    enum Value {
        Ascii(&'static str),
        Short(u16),
        Rationals(Vec<(u32, u32)>),
        SRational(i32, i32),
        /// Points at another directory, by index.
        Directory(usize),
    }

    /// A little-endian TIFF from directories of tagged values, the first one
    /// being IFD0. Values longer than four bytes go in a data area after the
    /// directories.
    fn tiff(directories: Vec<Vec<(u16, Value)>>) -> Vec<u8> {
        let sizes: Vec<u32> = directories.iter().map(|d| 2 + 12 * d.len() as u32 + 4).collect();
        let starts: Vec<u32> = sizes.iter().scan(8, |at, size| Some(std::mem::replace(at, *at + size))).collect();
        let mut data_at = 8 + sizes.iter().sum::<u32>();
        let (mut bytes, mut data) = (b"II*\0".to_vec(), Vec::new());
        bytes.extend_from_slice(&8u32.to_le_bytes());
        for directory in &directories {
            bytes.extend_from_slice(&(directory.len() as u16).to_le_bytes());
            for (tag, value) in directory {
                let (kind, count, payload): (u16, u32, Vec<u8>) = match value {
                    Value::Ascii(text) => (tiff::KIND_ASCII, text.len() as u32 + 1, [text.as_bytes(), &[0]].concat()),
                    Value::Short(n) => (tiff::KIND_SHORT, 1, n.to_le_bytes().to_vec()),
                    Value::Rationals(values) => {
                        (tiff::KIND_RATIONAL, values.len() as u32, values.iter().flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat()).collect())
                    }
                    Value::SRational(n, d) => (tiff::KIND_SRATIONAL, 1, [n.to_le_bytes(), d.to_le_bytes()].concat()),
                    Value::Directory(index) => (tiff::KIND_LONG, 1, starts[*index].to_le_bytes().to_vec()),
                };
                bytes.extend_from_slice(&tag.to_le_bytes());
                bytes.extend_from_slice(&kind.to_le_bytes());
                bytes.extend_from_slice(&count.to_le_bytes());
                if payload.len() <= 4 {
                    bytes.extend_from_slice(&payload);
                    bytes.resize(bytes.len() + 4 - payload.len(), 0);
                } else {
                    bytes.extend_from_slice(&data_at.to_le_bytes());
                    data_at += payload.len() as u32;
                    data.extend_from_slice(&payload);
                }
            }
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        bytes.extend_from_slice(&data);
        bytes
    }

    #[test]
    fn reads_what_the_inspector_shows() {
        let file = tiff(vec![
            vec![
                (TAG_MAKE, Value::Ascii("NIKON CORPORATION")),
                (TAG_MODEL, Value::Ascii("NIKON Z fc")),
                (TAG_EXIF_IFD, Value::Directory(1)),
                (TAG_GPS_IFD, Value::Directory(2)),
            ],
            vec![
                (TAG_EXPOSURE_PROGRAM, Value::Short(3)),
                (TAG_OFFSET_TIME_ORIGINAL, Value::Ascii("+02:00")),
                (TAG_EXPOSURE_BIAS, Value::SRational(-2, 3)),
                (TAG_METERING, Value::Short(5)),
                (TAG_FLASH, Value::Short(16)),
                (TAG_WHITE_BALANCE, Value::Short(0)),
                (TAG_FOCAL_LENGTH_35, Value::Short(75)),
            ],
            vec![
                (TAG_GPS_LATITUDE_REF, Value::Ascii("N")),
                (TAG_GPS_LATITUDE, Value::Rationals(vec![(48, 1), (46, 1), (3240, 100)])),
                (TAG_GPS_LONGITUDE_REF, Value::Ascii("W")),
                (TAG_GPS_LONGITUDE, Value::Rationals(vec![(9, 1), (10, 1), (5880, 100)])),
            ],
        ]);
        let exif = read_tiff_exif(&mut Cursor::new(file)).unwrap();
        assert_eq!(exif.make.as_deref(), Some("NIKON CORPORATION"));
        assert_eq!(exif.program(), Some("Aperture priority"));
        assert_eq!(exif.zone().as_deref(), Some("UTC+2"));
        assert_eq!(exif.bias().as_deref(), Some("-0.7 EV"));
        assert_eq!(exif.metering_mode(), Some("Matrix"));
        assert_eq!(exif.flash_fired(), Some("Off"));
        assert_eq!(exif.white_balance_mode(), Some("Auto"));
        assert_eq!(exif.focal_length_35, Some(75));
        assert_eq!(exif.location().as_deref(), Some("48.7757° N, 9.1830° W"));
    }

    #[test]
    fn offsets_parse_both_ways_and_reject_nonsense() {
        assert_eq!(parse_offset("+02:00"), Some(120));
        assert_eq!(parse_offset("-03:30"), Some(-210));
        assert_eq!(parse_offset("   :  "), None);
        assert_eq!(parse_offset("+25:00"), None);
    }
}
