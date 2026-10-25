//! The inspector: a panel on the right with the selected photo's histogram,
//! exposure, marks and every metadata field Folio reads. I shows and hides
//! it, in the grid and in the loupe.
//!
//! The histogram comes from the cached thumbnail, read off the UI thread
//! whenever the selection changes while the panel is open.

use std::path::Path;

use folio_core::culling::{Flag, Label, Stars};
use folio_core::histogram::Histogram;
use folio_core::library::Photo;
use folio_core::thumbnail::load_thumbnail;
use folio_core::thumbnail_cache::ThumbnailCache;
use gpui::{
    AnyElement, Bounds, Context, FontWeight, IntoElement, PathBuilder, Pixels, SharedString, Window, canvas, div, point,
    prelude::*, px,
};

use crate::app::{Folio, THUMB_PX};
use crate::theme;

pub const INSPECTOR_WIDTH: f32 = 300.;
/// How many bars the histogram is squeezed into. About one per two points.
const BINS: usize = 128;
const HISTOGRAM_HEIGHT: f32 = 92.;

pub struct Inspector {
    pub open: bool,
    /// Groups folded away. Kept while the window is open.
    folded: Vec<Group>,
    pub details: Details,
}

/// What's worked out off the UI thread for one photo.
pub enum Details {
    /// The panel hasn't asked for anything yet.
    None,
    Loading { photo: usize },
    Ready { photo: usize, histogram: Option<Curves>, file_size: Option<u64> },
}

/// A histogram squeezed for drawing: red, green, blue and brightness from 0
/// to 1, plus the shares of clipped shadows and highlights.
pub struct Curves {
    channels: [Vec<f32>; 4],
    clipped: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Exposure,
    Gear,
    File,
    Location,
    Copyright,
}

impl Inspector {
    pub fn new() -> Self {
        Self { open: false, folded: Vec::new(), details: Details::None }
    }

    /// The photo the details are for, or are being worked out for.
    fn photo(&self) -> Option<usize> {
        match self.details {
            Details::None => None,
            Details::Loading { photo } | Details::Ready { photo, .. } => Some(photo),
        }
    }

    #[cfg(feature = "screenshot")]
    pub fn is_loading(&self) -> bool {
        self.open && matches!(self.details, Details::Loading { .. })
    }
}

impl Folio {
    pub fn toggle_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inspector.open = !self.inspector.open;
        self.resize(window, cx);
        self.load_details(cx);
        cx.notify();
    }

    /// Starts reading the selected photo's details if the panel shows and
    /// has none for it. Called after every change, so it stays cheap when
    /// nothing moved.
    pub fn load_details(&mut self, cx: &mut Context<Self>) {
        let photo = self.culling.selected();
        if !self.inspector.open || !self.selection_shown() || self.inspector.photo() == Some(photo) {
            return;
        }
        self.inspector.details = Details::Loading { photo };
        let path = self.library.photo(photo).path.clone();
        let cache = self.thumb_cache.clone();
        self.details_load = Some(cx.spawn(async move |this, cx| {
            let (histogram, file_size) = cx.background_spawn(async move { read_details(&path, cache.as_deref()) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.inspector.photo() == Some(photo) {
                    this.inspector.details = Details::Ready { photo, histogram, file_size };
                    cx.notify();
                }
            });
        }));
    }

    fn toggle_group(&mut self, group: Group, cx: &mut Context<Self>) {
        let folded = &mut self.inspector.folded;
        match folded.iter().position(|&g| g == group) {
            Some(index) => {
                folded.remove(index);
            }
            None => folded.push(group),
        }
        cx.notify();
    }

    pub fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = div()
            .id("inspector")
            .w(px(INSPECTOR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme::SURFACE)
            .border_l_1()
            .border_color(theme::LINE)
            .overflow_y_scroll()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM);
        if !self.selection_shown() {
            return panel.pt(px(52.)).px(px(20.)).text_color(theme::TEXT_DIM).child("No photo selected");
        }
        let index = self.culling.selected();
        let photo = self.library.photo(index);
        let (curves, file_size) = match &self.inspector.details {
            Details::Ready { photo, histogram, file_size } if *photo == index => (histogram.as_ref(), *file_size),
            _ => (None, None),
        };
        let exif = &photo.exif;
        let taken = match (exif.captured, exif.zone()) {
            (Some(time), Some(zone)) => format!("{time} · {zone}"),
            (Some(time), None) => time.to_string(),
            (None, _) => "No capture time".to_owned(),
        };

        panel
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pt(px(46.))
                    .px(px(20.))
                    .pb(px(16.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).truncate().child(photo.file_name().to_owned()))
                    .child(div().mt(px(3.)).text_size(px(12.)).text_color(theme::TEXT_DIM).child(taken))
                    .child(histogram(curves))
                    .child(clipping(curves))
                    .child(exposure_strip(photo))
                    .child(self.render_marks(index)),
            )
            .children(self.groups(photo, file_size).into_iter().map(|(group, rows)| self.render_group(group, rows, cx)))
    }

    /// Stars, the color label and the flag, as the photo is marked now.
    fn render_marks(&self, photo: usize) -> impl IntoElement {
        let marks = self.culling.marks(photo);
        let filled = marks.stars.map_or(0, Stars::count);
        div()
            .mt(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .text_size(px(13.))
            .text_color(theme::TEXT_MUTED)
            .child(
                div()
                    .flex()
                    .child(div().text_color(theme::ACCENT).child("★".repeat(filled)))
                    .child(div().text_color(theme::TEXT_OFF).child("★".repeat(5 - filled))),
            )
            .when_some(marks.label, |row, label| {
                row.child(div().flex().items_center().gap(px(6.)).child(div().size(px(10.)).rounded_full().bg(theme::label(label))).child(label_name(label)))
            })
            .child(div().ml_auto().text_color(theme::TEXT_DIM).child(match marks.flag {
                Some(Flag::Pick) => "Picked",
                Some(Flag::Reject) => "Rejected",
                None => "No flag",
            }))
    }

    /// Every field, grouped. Rows without a value are left out.
    fn groups(&self, photo: &Photo, file_size: Option<u64>) -> Vec<(Group, Vec<Row>)> {
        let exif = &photo.exif;
        let focal = exif.focal().map(|focal| match exif.focal_length_35 {
            Some(full_frame) => Row::two("Focal length", focal, format!("{full_frame} mm full frame")),
            None => Row::one("Focal length", focal),
        });
        let size = photo.size.map(|(width, height)| {
            let megapixels = f64::from(width) * f64::from(height) / 1_000_000.;
            let detail = match file_size {
                Some(bytes) => format!("{megapixels:.1} MP · {}", file_size_text(bytes)),
                None => format!("{megapixels:.1} MP"),
            };
            Row::two("Size", format!("{width} × {height}"), detail)
        });
        let sidecar = photo.sidecar.as_deref().map(|s| s.rsplit('/').next().unwrap_or(s).to_owned());
        vec![
            (
                Group::Exposure,
                [
                    exif.bias().map(|v| Row::one("Bias", v)),
                    exif.program().map(|v| Row::one("Program", v)),
                    exif.metering_mode().map(|v| Row::one("Metering", v)),
                    exif.flash_fired().map(|v| Row::one("Flash", v)),
                    exif.white_balance_mode().map(|v| Row::one("White balance", v)),
                ]
                .into_iter()
                .flatten()
                .collect(),
            ),
            (
                Group::Gear,
                [
                    exif.camera.clone().map(|camera| match &exif.serial {
                        Some(serial) => Row::two("Camera", camera, format!("Serial {serial}")),
                        None => Row::one("Camera", camera),
                    }),
                    exif.make.clone().map(|v| Row::one("Make", v)),
                    // GPUI may wrap at the slash in `f/3.5`, so long names
                    // break before the aperture instead.
                    exif.lens.as_deref().map(|v| Row::one("Lens", v.replacen(" f/", "\nf/", 1))),
                    focal,
                    exif.software.clone().map(|v| Row::one("Firmware", v)),
                ]
                .into_iter()
                .flatten()
                .collect(),
            ),
            (
                Group::File,
                [
                    Some(Row::one("Name", photo.file_name().to_owned())),
                    Some(Row::one("Sidecar", sidecar.unwrap_or_else(|| "None yet".to_owned()))),
                    Some(Row::one("Folder", photo.folder().replace('/', " / "))),
                    size,
                ]
                .into_iter()
                .flatten()
                .collect(),
            ),
            (Group::Location, exif.location().map(|v| Row::one("Coordinates", v)).into_iter().collect()),
            (
                Group::Copyright,
                [exif.artist.clone().map(|v| Row::one("Artist", v)), exif.copyright.clone().map(|v| Row::one("Copyright", v))]
                    .into_iter()
                    .flatten()
                    .collect(),
            ),
        ]
    }

    /// A group's header, which folds it, and its rows.
    fn render_group(&self, group: Group, rows: Vec<Row>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let open = !self.inspector.folded.contains(&group);
        let title = match group {
            Group::Exposure => "Exposure",
            Group::Gear => "Camera and lens",
            Group::File => "File",
            Group::Location => "Location",
            Group::Copyright => "Copyright",
        };
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!("group-{title}")))
                    .flex()
                    .justify_between()
                    .px(px(20.))
                    .py(px(8.))
                    .bg(theme::RAISED)
                    .border_t_1()
                    .border_color(theme::LINE)
                    .cursor_pointer()
                    .hover(|header| header.bg(theme::HOVER))
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .child(title)
                    .child(div().text_color(theme::TEXT_DIM).child(if open { "⌄" } else { "›" }))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(group, cx))),
            )
            .when(open, |section| {
                section.child(
                    div()
                        .flex()
                        .flex_col()
                        .px(px(20.))
                        .py(px(6.))
                        .when(rows.is_empty(), |list| list.child(div().py(px(4.)).text_color(theme::TEXT_DIM).child("Not recorded")))
                        .children(rows.into_iter().map(render_row)),
                )
            })
    }
}

/// One line of the panel: a name, a value, and maybe a dimmer detail
/// under the value.
struct Row {
    name: &'static str,
    value: String,
    detail: Option<String>,
}

impl Row {
    fn one(name: &'static str, value: impl Into<String>) -> Self {
        Self { name, value: value.into(), detail: None }
    }

    fn two(name: &'static str, value: impl Into<String>, detail: String) -> Self {
        Self { name, value: value.into(), detail: Some(detail) }
    }
}

fn render_row(row: Row) -> AnyElement {
    div()
        .flex()
        .gap(px(10.))
        .py(px(4.))
        .child(div().w(px(96.)).flex_none().text_color(theme::TEXT_DIM).child(row.name))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(row.value)
                .when_some(row.detail, |value, detail| value.child(div().mt(px(1.)).text_size(px(11.5)).text_color(theme::TEXT_DIM).child(detail))),
        )
        .into_any_element()
}

/// The channels drawn over each other, brightness as a line on top, with
/// quarter marks behind. Empty while it's being worked out.
fn histogram(curves: Option<&Curves>) -> impl IntoElement {
    let channels = curves.map(|c| c.channels.clone());
    div()
        .mt(px(14.))
        .p(px(10.))
        .rounded(px(10.))
        .bg(theme::CANVAS)
        .child(canvas(|_, _, _| {}, move |bounds, _, window, _| paint_histogram(bounds, channels, window)).w_full().h(px(HISTOGRAM_HEIGHT)))
}

fn paint_histogram(bounds: Bounds<Pixels>, channels: Option<[Vec<f32>; 4]>, window: &mut Window) {
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    for quarter in 1..4 {
        let x = width * quarter as f32 / 4.;
        let mut line = PathBuilder::stroke(px(1.));
        line.move_to(at(x, 0.));
        line.line_to(at(x, height));
        if let Ok(path) = line.build() {
            window.paint_path(path, theme::LINE);
        }
    }
    let Some([red, green, blue, luma]) = channels else { return };
    let x_of = |i: usize, len: usize| width * i as f32 / (len.max(2) - 1) as f32;
    let y_of = |value: f32| height - value * height * 0.95;
    for (values, color) in [(red, Label::Red), (green, Label::Green), (blue, Label::Blue)] {
        let mut area = PathBuilder::fill();
        area.move_to(at(0., height));
        for (i, &value) in values.iter().enumerate() {
            area.line_to(at(x_of(i, values.len()), y_of(value)));
        }
        area.line_to(at(width, height));
        area.close();
        if let Ok(path) = area.build() {
            window.paint_path(path, theme::histogram_fill(color));
        }
    }
    let mut line = PathBuilder::stroke(px(1.2));
    for (i, &value) in luma.iter().enumerate() {
        let to = at(x_of(i, luma.len()), y_of(value));
        if i == 0 { line.move_to(to) } else { line.line_to(to) }
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, theme::TEXT_MUTED);
    }
}

/// How much of the photo is pure black or pure white.
fn clipping(curves: Option<&Curves>) -> impl IntoElement {
    let percent = |share: f32| format!("{:.1}%", share * 100.);
    let (shadows, highlights) = curves.map_or((String::new(), String::new()), |c| {
        (format!("Shadows clipped {}", percent(c.clipped.0)), format!("Highlights {}", percent(c.clipped.1)))
    });
    div().mt(px(6.)).h(px(14.)).flex().justify_between().text_size(px(11.)).text_color(theme::TEXT_DIM).child(shadows).child(highlights)
}

/// Shutter, aperture, ISO and focal length, big.
fn exposure_strip(photo: &Photo) -> impl IntoElement {
    let exif = &photo.exif;
    let shutter = exif.shutter().map(|s| s.trim_end_matches(" s").to_owned());
    let aperture = exif.aperture();
    let iso = exif.iso.map(|iso| iso.to_string());
    let focal = exif.focal().map(|f| f.trim_end_matches(" mm").to_owned());
    let cells = [(shutter, "s"), (aperture, ""), (iso, "ISO"), (focal, "mm")];
    div()
        .mt(px(16.))
        .flex()
        .gap(px(1.))
        .rounded(px(10.))
        .overflow_hidden()
        .bg(theme::LINE)
        .children(cells.into_iter().map(|(value, unit)| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(10.))
                .pb(px(9.))
                .bg(theme::RAISED)
                .child(
                    div()
                        .font_family(theme::FONT_DISPLAY)
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_size(px(20.))
                        .text_color(if value.is_some() { theme::TEXT } else { theme::TEXT_OFF })
                        .child(value.unwrap_or_else(|| "-".to_owned())),
                )
                .child(div().mt(px(2.)).h(px(13.)).text_size(px(10.5)).text_color(theme::TEXT_DIM).child(unit))
        }))
}

fn label_name(label: Label) -> &'static str {
    match label {
        Label::Red => "Red",
        Label::Yellow => "Yellow",
        Label::Green => "Green",
        Label::Blue => "Blue",
        Label::Purple => "Purple",
    }
}

/// `23.8 MB`, `640 KB`.
fn file_size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1_000_000.),
        b => format!("{} KB", b.div_ceil(1000)),
    }
}

/// Runs off the UI thread: the histogram of the cached thumbnail and the
/// file's size. Either can be missing without the other.
fn read_details(path: &Path, cache: Option<&ThumbnailCache>) -> (Option<Curves>, Option<u64>) {
    let file_size = std::fs::metadata(path).ok().map(|m| m.len());
    let image = match cache {
        Some(cache) => cache.load(path, THUMB_PX),
        None => load_thumbnail(path, THUMB_PX),
    };
    let curves = match image {
        Ok(image) => {
            let histogram = Histogram::of(&image);
            Some(Curves { channels: histogram.curves(BINS), clipped: histogram.clipped() })
        }
        Err(error) => {
            eprintln!("no histogram for {}: {error}", path.display());
            None
        }
    };
    (curves, file_size)
}
