//! Marks in XMP sidecars, where other apps can read them. The sidecar is the
//! source of truth for marks: `xmp:Rating` (1 to 5, 0 for none),
//! `xmp:Label` (`Red`, `Yellow`, `Green`, `Blue`, `Purple`) and
//! `folio:Flag` (`pick` or `reject`).
//!
//! Sidecars from Lightroom and Camera Raw carry hundreds of edit settings, so
//! writing never rebuilds the file. It patches the three properties in the
//! text and leaves every other byte alone. No XML parser: XMP properties are
//! either `prefix:Name="value"` attributes or `<prefix:Name>value</prefix:Name>`
//! elements, and that's all this needs to find. Only the usual `xmp` prefix
//! is recognised.

use std::fs;
use std::io;
use std::path::Path;

use crate::culling::{Flag, Label, Marks, Stars};

const XMP_NS: &str = "http://ns.adobe.com/xap/1.0/";
const FOLIO_NS: &str = "https://github.com/kratofl/folio/xmp/1.0/";

const RATING: &str = "xmp:Rating";
const LABEL: &str = "xmp:Label";
const FLAG: &str = "folio:Flag";

/// Reads the marks in a sidecar's text. Anything missing or unreadable is
/// unmarked. A rating of -1, which Bridge and darktable write for rejected
/// photos, counts as a reject unless Folio's own flag says otherwise.
pub fn read_marks(text: &str) -> Marks {
    let rating = find(text, RATING).and_then(|p| p.value.trim().parse::<f64>().ok()).map(|r| r.round() as i64);
    let flag = match find(text, FLAG).map(|p| p.value.trim().to_ascii_lowercase()).as_deref() {
        Some("pick") => Some(Flag::Pick),
        Some("reject") => Some(Flag::Reject),
        _ if rating == Some(-1) => Some(Flag::Reject),
        _ => None,
    };
    Marks {
        stars: rating.and_then(Stars::from_count),
        label: find(text, LABEL).and_then(|p| label_from_name(p.value.trim())),
        flag,
    }
}

/// Lightroom writes label names in the language of its interface, so the
/// German names are understood too. Folio always writes the English ones.
fn label_from_name(name: &str) -> Option<Label> {
    Label::from_name(name).or(match name.to_lowercase().as_str() {
        "rot" => Some(Label::Red),
        "gelb" => Some(Label::Yellow),
        "grün" => Some(Label::Green),
        "blau" => Some(Label::Blue),
        "lila" | "violett" => Some(Label::Purple),
        _ => None,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum XmpError {
    /// The file has no `rdf:Description` to add properties to.
    NoDescription,
}

/// `text` with the marks set, or a new minimal sidecar when there's no text.
/// Everything else in the text stays exactly as it was.
pub fn with_marks(text: Option<&str>, marks: Marks) -> Result<String, XmpError> {
    let mut text = text.map_or_else(empty_sidecar, str::to_owned);
    let rating = marks.stars.map_or(0, Stars::count).to_string();
    text = set(text, RATING, Some(&rating), XMP_NS)?;
    text = set(text, LABEL, marks.label.map(Label::name), XMP_NS)?;
    let flag = marks.flag.map(|f| match f {
        Flag::Pick => "pick",
        Flag::Reject => "reject",
    });
    set(text, FLAG, flag, FOLIO_NS)
}

/// Writes marks into a photo's sidecar, creating it if needed. The new text
/// goes to a hidden temporary file first and then replaces the sidecar in
/// one rename, so a crash never leaves half a file.
pub fn write_marks(sidecar: &Path, marks: Marks) -> io::Result<()> {
    let existing = match fs::read_to_string(sidecar) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let text = with_marks(existing.as_deref(), marks)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the sidecar has no rdf:Description"))?;
    let name = sidecar.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = sidecar.with_file_name(format!(".{name}.folio-tmp"));
    fs::write(&temporary, text)?;
    fs::rename(&temporary, sidecar)
}

/// `text` with Folio's three properties and namespace taken out. Two
/// sidecars that are equal after this differ only in marks, so they can be
/// merged without losing anything another app wrote.
pub fn without_marks(text: &str) -> String {
    let mut text = text.to_owned();
    for name in [RATING, LABEL, FLAG] {
        while let Some(whole) = find(&text, name).map(|p| p.whole) {
            text.replace_range(whole, "");
        }
    }
    text.replace(&format!(" xmlns:folio=\"{FOLIO_NS}\""), "")
}

fn empty_sidecar() -> String {
    format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\" xmlns:xmp=\"{XMP_NS}\">\n  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n"
    )
}

/// Where a property sits in the text.
struct Property<'a> {
    value: &'a str,
    /// The value's byte range.
    value_at: std::ops::Range<usize>,
    /// The whole property, including the whitespace before an attribute.
    whole: std::ops::Range<usize>,
}

fn find<'a>(text: &'a str, name: &str) -> Option<Property<'a>> {
    find_attribute(text, name).or_else(|| find_element(text, name))
}

fn find_attribute<'a>(text: &'a str, name: &str) -> Option<Property<'a>> {
    let pattern = format!("{name}=");
    let mut from = 0;
    while let Some(found) = text[from..].find(&pattern) {
        let start = from + found;
        from = start + pattern.len();
        let before = text[..start].chars().next_back();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let quote = text[from..].chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value_start = from + 1;
        let value_end = value_start + text[value_start..].find(quote)?;
        let whole_start = text[..start].trim_end().len();
        return Some(Property {
            value: &text[value_start..value_end],
            value_at: value_start..value_end,
            whole: whole_start..value_end + 1,
        });
    }
    None
}

fn find_element<'a>(text: &'a str, name: &str) -> Option<Property<'a>> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = text.find(&open)?;
    let value_start = start + open.len();
    let value_end = value_start + text[value_start..].find(&close)?;
    Some(Property {
        value: &text[value_start..value_end],
        value_at: value_start..value_end,
        whole: start..value_end + close.len(),
    })
}

/// Sets or removes one property, declaring its namespace if it's new.
fn set(mut text: String, name: &str, value: Option<&str>, namespace: &str) -> Result<String, XmpError> {
    match (find(&text, name).map(|p| (p.value_at, p.whole)), value) {
        (Some((value_at, _)), Some(value)) => text.replace_range(value_at, value),
        (Some((_, whole)), None) => text.replace_range(whole, ""),
        (None, None) => {}
        (None, Some(value)) => {
            let prefix = name.split_once(':').map_or(name, |(prefix, _)| prefix);
            let mut attributes = format!(" {name}=\"{value}\"");
            if !text.contains(&format!("xmlns:{prefix}=")) {
                attributes = format!(" xmlns:{prefix}=\"{namespace}\"{attributes}");
            }
            let at = description_tag_end(&text).ok_or(XmpError::NoDescription)?;
            text.insert_str(at, &attributes);
        }
    }
    Ok(text)
}

/// Where new attributes go in the first `rdf:Description` start tag: before
/// its `>`, or before `/>` if it's self-closing. Skips `>` inside quoted values.
fn description_tag_end(text: &str) -> Option<usize> {
    let start = text
        .match_indices("<rdf:Description")
        .find(|(i, m)| text[i + m.len()..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/'))?
        .0;
    let mut quote = None;
    for (offset, c) in text[start..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                let end = start + offset;
                return Some(if text[..end].ends_with('/') { end - 1 } else { end });
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Lightroom and Camera Raw write: everything as attributes.
    const LIGHTROOM: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   xmp:Rating="0" crs:Exposure2012="-0.44" crs:Note="a > b">
   <dc:creator><rdf:Seq><rdf:li>KRATOFL</rdf:li></rdf:Seq></dc:creator>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

    const MARKED: Marks = Marks { stars: Some(Stars::Four), label: Some(Label::Green), flag: Some(Flag::Pick) };

    #[test]
    fn reads_marks_from_attributes_and_elements() {
        assert_eq!(read_marks(LIGHTROOM), Marks::default(), "rating 0 is unrated");
        let elements = "<rdf:Description><xmp:Rating>3</xmp:Rating><xmp:Label>Rot</xmp:Label></rdf:Description>";
        assert_eq!(read_marks(elements), Marks { stars: Some(Stars::Three), label: Some(Label::Red), flag: None });
        let bridge_reject = r#"<rdf:Description xmp:Rating="-1"/>"#;
        assert_eq!(read_marks(bridge_reject).flag, Some(Flag::Reject));
    }

    #[test]
    fn writing_patches_marks_and_keeps_everything_else() {
        let written = with_marks(Some(LIGHTROOM), MARKED).unwrap();
        assert_eq!(read_marks(&written), MARKED);
        assert!(written.contains(r#"xmp:Rating="4""#));
        assert!(written.contains(r#"xmlns:folio="https://github.com/kratofl/folio/xmp/1.0/""#));
        assert!(written.contains(r#"crs:Note="a > b""#), "a > inside a value doesn't end the tag");
        assert!(written.contains("<rdf:li>KRATOFL</rdf:li>"));

        // Clearing gives back the original text, apart from the namespace.
        let cleared = with_marks(Some(&written), Marks::default()).unwrap();
        let without_namespace = cleared.replace(&format!(r#" xmlns:folio="{FOLIO_NS}""#), "");
        assert_eq!(without_namespace, LIGHTROOM);
    }

    #[test]
    fn element_values_are_replaced_in_place() {
        let text = "<rdf:Description rdf:about=\"\"><xmp:Rating>3</xmp:Rating></rdf:Description>";
        let written = with_marks(Some(text), Marks { stars: Some(Stars::Five), ..Marks::default() }).unwrap();
        assert!(written.contains("<xmp:Rating>5</xmp:Rating>"));
    }

    #[test]
    fn a_self_closing_description_takes_new_attributes() {
        let text = r#"<x:xmpmeta><rdf:RDF><rdf:Description rdf:about=""/></rdf:RDF></x:xmpmeta>"#;
        let written = with_marks(Some(text), MARKED).unwrap();
        assert!(written.ends_with(r#"folio:Flag="pick"/></rdf:RDF></x:xmpmeta>"#), "{written}");
        assert!(written.contains(r#"xmlns:xmp="http://ns.adobe.com/xap/1.0/""#));
        assert_eq!(read_marks(&written), MARKED);
    }

    #[test]
    fn marks_can_be_taken_out_to_compare_the_rest() {
        let marked = with_marks(Some(LIGHTROOM), MARKED).unwrap();
        assert_eq!(without_marks(&marked), without_marks(LIGHTROOM));
        assert!(without_marks(&marked).contains(r#"crs:Exposure2012="-0.44""#));
        assert!(!without_marks(&marked).contains("Rating"));
    }

    #[test]
    fn new_sidecars_and_broken_ones() {
        assert_eq!(read_marks(&with_marks(None, MARKED).unwrap()), MARKED);
        assert_eq!(with_marks(Some("not xmp"), MARKED), Err(XmpError::NoDescription));
    }

    #[test]
    fn writes_go_to_disk_atomically() {
        let dir = std::env::temp_dir().join(format!("folio-xmp-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let sidecar = dir.join("_DSC1.xmp");
        write_marks(&sidecar, MARKED).unwrap();
        let text = fs::read_to_string(&sidecar).unwrap();
        let leftovers = fs::read_dir(&dir).unwrap().count();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(read_marks(&text), MARKED);
        assert_eq!(leftovers, 1, "no temporary file left behind");
    }
}
