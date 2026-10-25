//! The chrome around the photos: the sidebar with views and categories, the
//! dock for the selected photo, notices and the window drag strip.

use folio_core::culling::{Flag, Stars};
use gpui::{
    Context, FontWeight, IntoElement, MouseButton, SharedString, WindowControlArea, div, prelude::*, px,
};

use crate::app::{Folio, Mode, NoticeAction, SIDEBAR_WIDTH, View, count};
use crate::theme;

/// The strip at the top where the window can be dragged, under the traffic lights.
const TITLEBAR_HEIGHT: f32 = 38.;

impl Folio {
    pub fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let unsorted: usize = self.plan.outings().iter().map(|o| o.photos().len()).sum();
        let accepted: usize = self.plan.accepted().map(|o| o.photos().len()).sum();
        let categories = self.library.categories();
        let active_view = self.active_view().map(|v| v.name.clone());
        let in_library = self.view == View::Library && active_view.is_none();

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme::SURFACE)
            .border_r_1()
            .border_color(theme::LINE)
            .pt(px(52.))
            .pb(px(14.))
            .px(px(10.))
            .gap(px(2.))
            .text_size(px(13.))
            .child(self.nav_item(View::Library, "Library".into(), self.library.photos().len(), false, in_library, cx))
            .child(self.nav_item(View::Unsorted, "Unsorted".into(), unsorted, unsorted > 0, self.view == View::Unsorted, cx))
            .child(
                div()
                    .id("sidebar-lists")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(!categories.is_empty(), |list| list.child(heading("Categories")))
                    .children(categories.into_iter().map(|(name, photos)| {
                        let view = View::Category(name.clone());
                        let active = self.view == view;
                        self.nav_item(view, name.into(), photos, false, active, cx)
                    }))
                    .when(!self.views.is_empty(), |list| list.child(heading("Views")))
                    .children(self.views.iter().enumerate().map(|(i, view)| {
                        let active = active_view.as_deref() == Some(view.name.as_str());
                        let photos = self.view_counts.get(i).copied().unwrap_or(0);
                        nav_row(SharedString::from(format!("view-{i}")), view.name.clone().into(), photos, false, active)
                            .on_click(cx.listener(move |this, _, _, cx| this.open_view(i, cx)))
                    })),
            )
            .when(accepted > 0, |bar| {
                bar.child(
                    div()
                        .id("review")
                        .mt(px(10.))
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(px(8.))
                        .bg(theme::ACCENT)
                        .hover(|b| b.bg(theme::ACCENT_HOVER))
                        .cursor_pointer()
                        .text_color(theme::ON_ACCENT)
                        .font_weight(FontWeight::BOLD)
                        .flex()
                        .justify_between()
                        .child(format!("Review {}", count(accepted, "photo")))
                        .child(div().font_weight(FontWeight::MEDIUM).child(command("⏎")))
                        .on_click(cx.listener(|this, _, _, cx| this.open_review(cx))),
                )
            })
            .child(
                div()
                    .pt(px(12.))
                    .px(px(8.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::TEXT_DIM)
                    .truncate()
                    .child(self.library.name()),
            )
    }

    /// A sidebar entry that switches the grid to `view`, without filters.
    fn nav_item(
        &self,
        view: View,
        label: SharedString,
        photos: usize,
        highlight: bool,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        nav_row(SharedString::from(format!("nav-{label}")), label, photos, highlight, active).on_click(cx.listener(
            move |this, _, _, cx| {
                this.clear_filter(cx);
                this.show(view.clone(), cx);
            },
        ))
    }

    /// The bottom of the photo area: a notice if there is one, otherwise
    /// the dock for the selected photo.
    pub fn render_bottom(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.notice {
            // The loupe shows notices without their actions, which would
            // change the grid behind it.
            Some(notice) => {
                let action = notice.action.clone().filter(|_| !matches!(self.mode, Mode::Loupe(_)));
                Some(self.render_notice(&notice.text, action, cx).into_any_element())
            }
            None if self.selection_shown() && matches!(self.mode, Mode::Browse | Mode::Loupe(_)) => Some(self.render_dock(cx).into_any_element()),
            None => None,
        };
        div().absolute().bottom(px(18.)).left_0().right_0().flex().justify_center().children(content)
    }

    fn render_notice(&self, text: &str, action: Option<NoticeAction>, cx: &mut Context<Self>) -> impl IntoElement {
        pill().text_color(theme::TEXT).child(text.to_owned()).when_some(action, |pill, action| {
            let label = match action {
                NoticeAction::Undo => format!("Undo  {}", command("Z")),
                NoticeAction::Redo => format!("Redo  {}", shift_command("Z")),
                NoticeAction::Restore(_) => "Restore".to_owned(),
            };
            pill.child(separator()).child(
                div()
                    .id("notice-action")
                    .cursor_pointer()
                    .text_color(theme::ACCENT)
                    .font_weight(FontWeight::BOLD)
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| match &action {
                        NoticeAction::Undo => this.undo(cx),
                        NoticeAction::Redo => this.redo(cx),
                        NoticeAction::Restore(view) => this.restore_view(view.clone(), cx),
                    })),
            )
        })
    }

    fn render_dock(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.culling.selected();
        let photo = self.library.photo(selected);
        let marks = self.culling.marks(selected);
        let filled = marks.stars.map_or(0, Stars::count);
        let summary = photo.exif.summary();

        pill()
            .child(
                div()
                    .flex()
                    .child(div().text_color(theme::ACCENT).child("★".repeat(filled)))
                    .child(div().text_color(theme::TEXT_OFF).child("★".repeat(5 - filled))),
            )
            .when_some(marks.label, |dock, label| dock.child(div().size(px(10.)).rounded_full().bg(theme::label(label))))
            .when_some(marks.flag, |dock, flag| {
                dock.child(match flag {
                    Flag::Pick => div().text_color(theme::TEXT).child("Pick"),
                    Flag::Reject => div().text_color(theme::TEXT_DIM).child("Rejected"),
                })
            })
            .child(separator())
            .child(div().text_color(theme::TEXT).child(photo.file_name().to_owned()))
            .when(!summary.is_empty(), |dock| dock.child(separator()).child(summary))
            .when_some(photo.exif.captured, |dock, time| dock.child(separator()).child(time.to_string()))
            .when(!self.thumbnails_settled(), |dock| dock.child(separator()).child("Loading thumbnails"))
            // Filters change the grid, which the loupe hides.
            .when(matches!(self.mode, Mode::Browse), |dock| dock.child(separator()).child(self.render_filter_button(cx)))
            .child(self.render_info_button(cx))
    }

    /// Shows or hides the inspector. Lit while it's open.
    fn render_info_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.inspector.open;
        div()
            .id("dock-info")
            .px(px(8.))
            .py(px(2.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_color(theme::TEXT)
            .when(!open, |b| b.bg(theme::LINE).hover(|b| b.bg(theme::HOVER)))
            .when(open, |b| b.bg(theme::HOVER))
            .child("Info  I")
            .on_click(cx.listener(|this, _, window, cx| this.toggle_inspector(window, cx)))
    }

    /// Opens the ⌘K bar. Shows how many filters are on, so a narrowed grid
    /// never looks like the whole library.
    fn render_filter_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.filter.conditions().len();
        div()
            .id("dock-filter")
            .px(px(8.))
            .py(px(2.))
            .rounded(px(6.))
            .cursor_pointer()
            .when(active == 0, |b| b.bg(theme::LINE).text_color(theme::TEXT).hover(|b| b.bg(theme::HOVER)))
            .when(active > 0, |b| b.bg(theme::ACCENT).text_color(theme::ON_ACCENT).font_weight(FontWeight::BOLD).hover(|b| b.bg(theme::ACCENT_HOVER)))
            .child(if active == 0 { format!("Filter  {}", command("K")) } else { format!("Filters · {active}  {}", command("K")) })
            .on_click(cx.listener(|this, _, _, cx| this.open_filter(cx)))
    }

}

/// A transparent strip over the top of a window that drags it, since the
/// titlebar is hidden so photos can run edge to edge. `pending` is the
/// view's flag for a press that may turn into a drag: set on mouse down,
/// the first move then drags. Waiting for the move keeps double clicks
/// working.
pub fn titlebar<V: 'static>(cx: &mut Context<V>, pending: fn(&mut V) -> &mut bool) -> impl IntoElement {
    div()
        .id("titlebar")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(TITLEBAR_HEIGHT))
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, cx.listener(move |view, _, _, _| *pending(view) = true))
        .on_mouse_up(MouseButton::Left, cx.listener(move |view, _, _, _| *pending(view) = false))
        .on_mouse_move(cx.listener(move |view, _, window, _| {
            if std::mem::take(pending(view)) {
                window.start_window_move();
            }
        }))
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.titlebar_double_click();
            }
        })
}

/// A clickable sidebar row: a name and a photo count.
fn nav_row(id: SharedString, label: SharedString, photos: usize, highlight: bool, active: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .justify_between()
        .items_center()
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .cursor_pointer()
        .font_weight(FontWeight::MEDIUM)
        .text_color(if active { theme::TEXT } else { theme::TEXT_MUTED })
        .when(active, |item| item.bg(theme::RAISED))
        .when(!active, |item| item.hover(|item| item.bg(theme::HOVER)))
        .child(div().truncate().child(label))
        .child(
            div()
                .text_size(px(12.))
                .text_color(if highlight { theme::ACCENT } else { theme::TEXT_DIM })
                .child(photos.to_string()),
        )
}

fn heading(text: &'static str) -> impl IntoElement {
    div()
        .pt(px(16.))
        .pb(px(4.))
        .px(px(10.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::TEXT_DIM)
        .child(text)
}

/// The floating rounded bar the dock and notices share.
pub fn pill() -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(18.))
        .py(px(9.))
        .rounded_full()
        .bg(theme::RAISED)
        .border_1()
        .border_color(theme::LINE)
        .shadow_lg()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::TEXT_MUTED)
        .whitespace_nowrap()
}

/// A shortcut label with the platform's command key: `⌘K` on macOS,
/// `Ctrl+K` on Windows.
pub fn command(key: &str) -> String {
    if cfg!(target_os = "macos") { format!("⌘{key}") } else { format!("Ctrl+{}", key_name(key)) }
}

/// The same with Shift: `⇧⌘Z` on macOS, `Ctrl+Shift+Z` on Windows.
pub fn shift_command(key: &str) -> String {
    if cfg!(target_os = "macos") { format!("⇧⌘{key}") } else { format!("Ctrl+Shift+{}", key_name(key)) }
}

/// Windows spells out the keys macOS draws as symbols.
fn key_name(key: &str) -> &str {
    match key {
        "⏎" => "Enter",
        "⌫" => "Backspace",
        key => key,
    }
}

fn separator() -> impl IntoElement {
    div().w(px(1.)).h(px(16.)).bg(theme::LINE)
}
