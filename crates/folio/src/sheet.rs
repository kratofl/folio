//! The review sheet: every folder that moves will create or fill and every
//! file that moves, sidecars included, before anything touches the disk.

use folio_core::moves::{MovePlan, PlannedFolder};
use gpui::{Context, FontWeight, IntoElement, div, prelude::*, px};

use crate::app::{Folio, Mode, count};
use crate::theme;

/// Files listed per folder before the rest are summed up.
const FILES_SHOWN: usize = 4;

impl Folio {
    pub fn render_review(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Mode::Review(plan) = &self.mode else { return div().into_any_element() };
        let files = files(plan.photo_count(), plan.video_count());

        div()
            .id("review-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::SCRIM)
            // Clicks on the backdrop don't reach the grid underneath.
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .w(px(560.))
                    .max_h(px(620.))
                    .flex()
                    .flex_col()
                    .rounded(px(14.))
                    .bg(theme::RAISED)
                    .border_1()
                    .border_color(theme::LINE)
                    .shadow_lg()
                    .child(
                        div()
                            .px(px(22.))
                            .pt(px(20.))
                            .pb(px(14.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!("Move {files} into {}", count(plan.folders.len(), "folder"))),
                            )
                            .child(div().text_size(px(12.)).text_color(theme::TEXT_DIM).child(sources(self, plan))),
                    )
                    .child(
                        div()
                            .id("review-folders")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(22.))
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .children(plan.folders.iter().map(render_folder)),
                    )
                    .child(
                        div()
                            .px(px(22.))
                            .py(px(16.))
                            .mt(px(8.))
                            .border_t_1()
                            .border_color(theme::LINE)
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(11.))
                                    .text_color(theme::TEXT_DIM)
                                    .child("The undo log goes to .folio/moves/ before anything moves."),
                            )
                            .child(
                                button("Cancel  Esc")
                                    .id("review-cancel")
                                    .hover(|b| b.bg(theme::HOVER))
                                    .on_click(cx.listener(|this, _, _, cx| this.close_review(cx))),
                            )
                            .child(
                                button(&format!("Move {files}  ⏎"))
                                    .id("review-move")
                                    .bg(theme::ACCENT)
                                    .text_color(theme::ON_ACCENT)
                                    .hover(|b| b.bg(theme::ACCENT_HOVER))
                                    .on_click(cx.listener(|this, _, _, cx| this.run_moves(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// `From 2024/2024-11-09 and 2024/2024-12-05`
fn sources(folio: &Folio, plan: &MovePlan) -> String {
    let mut folders: Vec<&str> = Vec::new();
    for planned in plan.folders.iter().flat_map(|f| &f.photos) {
        let folder = folio.library.photo(planned.photo).folder();
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    let named: Vec<&str> = folders.iter().take(3).map(|f| if f.is_empty() { "the library root" } else { f }).collect();
    match folders.len() {
        0 => String::new(),
        1 => format!("From {}", named[0]),
        2 => format!("From {} and {}", named[0], named[1]),
        3 => format!("From {}, {} and {}", named[0], named[1], named[2]),
        n => format!("From {}, {}, {} and {} more", named[0], named[1], named[2], n - 3),
    }
}

/// `12 photos`, or `12 photos and 2 videos`.
fn files(photos: usize, videos: usize) -> String {
    match videos {
        0 => count(photos, "photo"),
        _ => format!("{} and {}", count(photos, "photo"), count(videos, "video")),
    }
}

fn render_folder(folder: &PlannedFolder) -> impl IntoElement {
    let rest = folder.photos.len().saturating_sub(FILES_SHOWN);
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .child(folder.folder.replace('/', " / "))
                .when(folder.is_new, |row| {
                    row.child(
                        div()
                            .px(px(5.))
                            .rounded(px(4.))
                            .bg(theme::ACCENT)
                            .text_color(theme::ON_ACCENT)
                            .text_size(px(10.))
                            .child("NEW"),
                    )
                })
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::TEXT_DIM)
                        .child(files(folder.photos.len(), folder.videos.len())),
                ),
        )
        .children(folder.photos.iter().take(FILES_SHOWN).map(|planned| {
            let name = planned.file.to.rsplit_once('/').map_or(planned.file.to.as_str(), |(_, n)| n).to_owned();
            div()
                .pl(px(12.))
                .text_size(px(12.))
                .text_color(theme::TEXT_MUTED)
                .child(if planned.sidecar.is_some() { format!("{name}  + .xmp") } else { name })
        }))
        .when(rest > 0, |list| {
            list.child(div().pl(px(12.)).text_size(px(12.)).text_color(theme::TEXT_DIM).child(format!("and {rest} more")))
        })
}

fn button(label: &str) -> gpui::Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .px(px(14.))
        .py(px(7.))
        .rounded(px(8.))
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme::TEXT)
        .child(label.to_owned())
}
