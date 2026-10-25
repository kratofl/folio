//! The welcome screen, shown when Folio starts without a library: open a
//! folder or pick a recently opened library.

use std::path::PathBuf;

use gpui::{
    Context, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render, SharedString, Window, div, prelude::*, px,
};

use crate::chrome::{command, titlebar};
use crate::open;
use crate::theme;

pub struct Welcome {
    recent: Vec<PathBuf>,
    state: State,
    focus: FocusHandle,
    /// A titlebar press that may become a drag (see `chrome::titlebar`).
    drag_pending: bool,
}

enum State {
    Idle,
    Opening(PathBuf),
    Failed(String),
}

impl Welcome {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let recent = folio_core::dirs::data_dir().map(|dir| folio_core::recent::load_recent(&dir)).unwrap_or_default();
        Self { recent, state: State::Idle, focus, drag_pending: false }
    }

    /// Asks for a folder, then opens it.
    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            if let Some(root) = open::choose_folder(cx).await {
                let _ = this.update_in(cx, |this, window, cx| this.open(root, window, cx));
            }
        })
        .detach();
    }

    /// Opens `root` in its own window and closes this one once it's there.
    fn open(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.state, State::Opening(_)) {
            return;
        }
        self.state = State::Opening(root.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| match open::open(root, cx).await {
            Ok(_) => {
                let _ = cx.update(|window, _| window.remove_window());
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    this.state = State::Failed(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "o" if keystroke.modifiers.secondary(cx) => self.choose(window, cx),
            // Enter reopens the last library.
            "enter" => {
                if let Some(root) = self.recent.first().cloned() {
                    self.open(root, window, cx);
                }
            }
            _ => {}
        }
    }
}

impl Render for Welcome {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = match &self.state {
            State::Idle => None,
            State::Opening(root) => Some((theme::TEXT_MUTED, format!("Opening {}…", folder_name(root)))),
            State::Failed(error) => Some((theme::ACCENT_SOFT, error.clone())),
        };

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::SURFACE)
            .font_family(theme::FONT_UI)
            .text_color(theme::TEXT)
            .child(
                div()
                    .w(px(420.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().font_family(theme::FONT_DISPLAY).text_size(px(40.)).font_weight(FontWeight::BOLD).child("Folio"))
                            .child(div().text_color(theme::TEXT_MUTED).child("Open a folder of photos. It becomes your library.")),
                    )
                    .child(
                        div()
                            .id("open-folder")
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(16.))
                            .py(px(11.))
                            .rounded(px(10.))
                            .bg(theme::ACCENT)
                            .hover(|button| button.bg(theme::ACCENT_HOVER))
                            .cursor_pointer()
                            .text_color(theme::ON_ACCENT)
                            .font_weight(FontWeight::BOLD)
                            .child("Open folder…")
                            .child(command("O"))
                            .on_click(cx.listener(|this, _, window, cx| this.choose(window, cx))),
                    )
                    .when_some(status, |column, (color, text)| column.child(div().text_size(px(13.)).text_color(color).child(text)))
                    .when(!self.recent.is_empty(), |column| column.child(self.render_recent(cx))),
            )
            .child(titlebar(cx, |this: &mut Self| &mut this.drag_pending))
    }
}

impl Welcome {
    fn render_recent(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().pb(px(6.)).text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme::TEXT_DIM).child("Recent"))
            .children(self.recent.iter().enumerate().map(|(index, root)| {
                let open_root = root.clone();
                let path: SharedString = root.parent().map(|p| p.display().to_string()).unwrap_or_default().into();
                div()
                    .id(index)
                    .flex()
                    .flex_col()
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|row| row.bg(theme::RAISED))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(folder_name(root)))
                    .child(div().text_size(px(12.)).text_color(theme::TEXT_DIM).child(path))
                    .on_click(cx.listener(move |this, _, window, cx| this.open(open_root.clone(), window, cx)))
            }))
    }
}

fn folder_name(root: &std::path::Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
}
