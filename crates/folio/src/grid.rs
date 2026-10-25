//! The photo area: photos edge to edge in justified rows, with a big heading
//! per section. In the Unsorted view each section is an outing, and its
//! heading carries the sorting controls.

use folio_core::culling::Flag;
use folio_core::layout::Block;
use folio_core::outings::{Outing, gap_text};
use folio_core::text::TextField;
use gpui::{
    AnyElement, ClickEvent, Context, PinchEvent, FontWeight, ImageSource, IntoElement, ObjectFit, SharedString, div, img, list, prelude::*, px,
};

use crate::app::{Field, Folio, GAP, Mode, Thumb, View, count, typed_prefix};
use crate::theme;

/// How strongly accepted outings and rejected photos fade.
const FADED: f32 = 0.35;

impl Folio {
    pub fn render_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.layout.order().is_empty();
        div()
            .relative()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            // Pinching makes the thumbnails bigger or smaller.
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| this.zoom_grid(1. + event.delta, cx)))
            .when(!self.filter.is_empty(), |main| main.child(self.render_filter_strip(cx)))
            .when(self.view == View::Unsorted && !empty, |main| main.child(self.render_gap_strip(cx)))
            .when(!empty, |main| {
                main.child(
                    list(self.list.clone(), cx.processor(|this, index: usize, _window, cx| this.render_block(index, cx)))
                        .flex_1()
                        .w_full()
                        .pb(px(80.)),
                )
            })
            .when(empty, |main| main.child(self.render_empty()))
            .child(self.render_bottom(cx))
    }

    /// How long a break starts a new outing, with buttons to change it.
    fn render_gap_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.filter.is_empty();
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(18.))
            .pt(px(if first { 44. } else { 4. }))
            .pb(px(4.))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::TEXT_DIM)
            .child(div().mr(px(4.)).child(format!("New outing after {} without photos", gap_text(self.settings.outing_gap_minutes))))
            .child(chip("Shorter  [".into()).id("gap-shorter").on_click(cx.listener(|this, _, _, cx| this.change_gap(false, cx))))
            .child(chip("Longer  ]".into()).id("gap-longer").on_click(cx.listener(|this, _, _, cx| this.change_gap(true, cx))))
    }

    fn render_empty(&self) -> impl IntoElement {
        let (title, detail) = match self.view {
            _ if !self.filter.is_empty() => ("No photos match", "Esc clears the filters."),
            View::Unsorted => ("Everything is sorted", "Photos in date folders show up here, grouped into outings."),
            _ => ("No photos here", ""),
        };
        div()
            .w_full()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .child(div().font_family(theme::FONT_DISPLAY).font_weight(FontWeight::EXTRA_BOLD).text_size(px(26.)).child(title))
            .child(div().text_size(px(13.)).text_color(theme::TEXT_DIM).child(detail))
    }

    fn render_block(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        match &self.layout.blocks()[index] {
            Block::Header { section } => {
                // A strip above the list, when there is one, already clears
                // the window's drag area at the top.
                let strip = !self.filter.is_empty() || self.view == View::Unsorted;
                let top = match index {
                    0 if !strip => 52.,
                    0 => 14.,
                    _ => 34.,
                };
                let header = match self.view {
                    View::Unsorted => self.render_outing_header(self.section_outings[*section], cx).into_any_element(),
                    _ => self.render_folder_header(*section).into_any_element(),
                };
                div().pt(px(top)).pb(px(12.)).px(px(18.)).child(header).into_any_element()
            }
            Block::Row { height, items } => div()
                .flex()
                .gap(px(GAP))
                .pb(px(GAP))
                .children(items.iter().map(|placed| self.render_tile(placed.photo, placed.width, *height, cx)))
                .into_any_element(),
        }
    }

    fn render_folder_header(&self, section: usize) -> impl IntoElement {
        let section = &self.sections[section];
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(title(section.title.clone()))
            .child(caption(count(section.photos.len(), "photo")))
    }

    fn render_outing_header(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let outing = &self.plan.outings()[index];
        let typing = match &self.mode {
            Mode::Typing { outing, field, text } if *outing == index => Some((*field, text)),
            _ => None,
        };
        let selected = self.selected_outing() == Some(index) && matches!(self.mode, Mode::Browse);

        let name = match typing {
            Some((Field::Name, text)) => self.render_input(text, cx).into_any_element(),
            _ => title(outing.name().to_owned()).into_any_element(),
        };
        let destination = match typing {
            Some((Field::Category { .. }, text)) => self.render_category_input(text, cx).into_any_element(),
            _ => destination(outing).into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(outing.is_accepted(), |header| header.opacity(0.6))
            .child(div().flex().items_center().gap(px(14.)).child(name).child(destination))
            .child(caption(outing_summary(self, outing, self.outing_videos.get(index).copied().unwrap_or(0))))
            .when(selected, |header| header.child(self.render_sort_actions(outing, cx)))
    }

    /// The typed category plus matching categories to pick from.
    fn render_category_input(&self, text: &TextField, cx: &mut Context<Self>) -> impl IntoElement {
        // While the old value is selected, every category is a candidate.
        let suggestions = self.plan.category_suggestions(&self.library, typed_prefix(text));
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(caption("Category".into()))
            .child(self.render_input(text, cx))
            .children(suggestions.into_iter().take(6).enumerate().map(|(i, name)| {
                let picked = name.clone();
                chip(if i == 0 { format!("{name}  Tab") } else { name })
                    .id(SharedString::from(format!("suggestion-{i}")))
                    .on_click(cx.listener(move |this, _, _, cx| this.commit_typing(Some(picked.clone()), cx)))
            }))
    }

    /// Buttons for the selected outing, each with its key.
    fn render_sort_actions(&self, outing: &Outing, cx: &mut Context<Self>) -> impl IntoElement {
        let accept = if outing.is_accepted() { "Reopen" } else { "Accept" };
        let action = |id: &'static str, label: &str, key: &str| {
            chip(format!("{label}  {key}")).id(id)
        };
        div()
            .flex()
            .gap(px(6.))
            .pt(px(4.))
            .child(
                action("accept", accept, "⏎")
                    .on_click(cx.listener(|this, _, _, cx| this.accept_or_ask(cx))),
            )
            .child(
                action("rename", "Rename", "R")
                    .on_click(cx.listener(|this, _, _, cx| this.start_typing(Field::Name, cx))),
            )
            .child(
                action("category", "Category", "C")
                    .on_click(cx.listener(|this, _, _, cx| this.start_typing(Field::Category { then_accept: false }, cx))),
            )
            .child(action("split", "Split here", "S").on_click(cx.listener(|this, _, _, cx| this.split(cx))))
            .child(action("merge", "Merge up", "M").on_click(cx.listener(|this, _, _, cx| this.merge_up(cx))))
    }

    fn render_tile(&self, index: usize, width: f32, height: f32, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let picture = match &self.thumbs[index] {
            Thumb::Ready(image) => img(ImageSource::Render(image.clone())).size_full().object_fit(ObjectFit::Cover).into_any_element(),
            Thumb::Loading => div().into_any_element(),
            Thumb::Failed => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(theme::TEXT_DIM)
                .child("No preview")
                .into_any_element(),
        };
        let marks = self.culling.marks(index);
        let faded = self.faded[index] || marks.flag == Some(Flag::Reject);

        div()
            .id(index)
            .relative()
            .flex_none()
            .w(px(width))
            .h(px(height))
            .bg(theme::TILE_EMPTY)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.select(index, cx);
                if event.click_count() == 2 {
                    this.open_loupe(window, cx);
                }
            }))
            .child(div().size_full().when(faded, |d| d.opacity(FADED)).child(picture))
            .when(marks.stars.is_some() || marks.flag == Some(Flag::Pick), |tile| {
                tile.child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .right(px(6.))
                        .flex()
                        .gap(px(4.))
                        .px(px(5.))
                        .rounded(px(4.))
                        .bg(theme::SCRIM)
                        .text_xs()
                        .text_color(theme::ACCENT)
                        .when(marks.flag == Some(Flag::Pick), |b| b.child(div().text_color(theme::TEXT).child("Pick")))
                        .when_some(marks.stars, |b, stars| b.child("★".repeat(stars.count()))),
                )
            })
            .when_some(marks.label, |tile, label| {
                tile.child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .left(px(8.))
                        .size(px(10.))
                        .rounded_full()
                        .border_1()
                        .border_color(theme::SCRIM)
                        .bg(theme::label(label)),
                )
            })
            .when(index == self.culling.selected(), |tile| {
                tile.child(div().absolute().inset_0().border(px(3.)).border_color(theme::ACCENT))
            })
    }
}

/// `12 photos · 2 videos · Oct 20 2024, 09:00 to 10:30`
fn outing_summary(folio: &Folio, outing: &Outing, videos: usize) -> String {
    let times: Vec<_> = outing.photos().iter().filter_map(|&p| folio.library.photo(p).exif.captured).collect();
    let mut photos = count(outing.photos().len(), "photo");
    if videos > 0 {
        photos = format!("{photos} · {}", count(videos, "video"));
    }
    match (times.first(), times.last()) {
        (Some(first), Some(last)) if first == last => format!("{photos} · {first}"),
        (Some(first), Some(last)) if (first.year, first.month, first.day) == (last.year, last.month, last.day) => {
            format!("{photos} · {first} to {:02}:{:02}", last.hour, last.minute)
        }
        (Some(first), Some(last)) => format!("{photos} · {first} to {last}"),
        _ => format!("{photos} · no capture time"),
    }
}

/// Where an outing goes, or a hint that it needs a category first.
fn destination(outing: &Outing) -> impl IntoElement {
    let row = div().flex().items_center().gap(px(8.)).text_size(px(13.)).font_weight(FontWeight::MEDIUM);
    match outing.category() {
        Some(category) => row
            .child(div().text_color(theme::TEXT_DIM).child("into"))
            .child(div().text_color(theme::TEXT).child(format!("{category} / {}", outing.name())))
            .when(outing.is_accepted(), |row| row.child(div().text_color(theme::ACCENT_SOFT).child("Accepted"))),
        None => row.child(div().text_color(theme::TEXT_DIM).child("No category yet")),
    }
}

fn title(text: String) -> impl IntoElement {
    div()
        .font_family(theme::FONT_DISPLAY)
        .text_size(px(26.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .line_height(px(30.))
        .child(text)
}

fn caption(text: String) -> impl IntoElement {
    div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme::TEXT_DIM).child(text)
}

/// A small button-like label on the grid.
pub fn chip(text: String) -> gpui::Div {
    div()
        .px(px(9.))
        .py(px(4.))
        .rounded(px(6.))
        .bg(theme::RAISED)
        .border_1()
        .border_color(theme::LINE)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::TEXT_MUTED)
        .cursor_pointer()
        .hover(|chip| chip.bg(theme::HOVER).text_color(theme::TEXT))
        .child(text)
}

impl Folio {
    /// A boxed field for typing a name.
    pub fn render_input(&self, field: &TextField, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .min_w(px(160.))
            .px(px(10.))
            .py(px(5.))
            .rounded(px(6.))
            .bg(theme::RAISED)
            .border_1()
            .border_color(theme::ACCENT)
            .text_size(px(15.))
            .font_weight(FontWeight::MEDIUM)
            .child(self.render_field(field, cx))
    }
}
