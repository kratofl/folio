//! Filtering: the ⌘K bar that suggests conditions as you type, the strip of
//! active conditions above the grid, and the prompt that saves them as a view.

use folio_core::text::TextField;
use gpui::{Context, FontWeight, IntoElement, SharedString, div, prelude::*, px};

use crate::app::{BarItem, Folio, Mode, SIDEBAR_WIDTH};
use crate::chrome::command;
use crate::theme;

/// Suggestion rows shown at once. Arrow keys scroll through the rest.
const VISIBLE_SUGGESTIONS: usize = 14;

impl Folio {
    /// The bar or the save prompt, floating over the photos just below the
    /// window's drag area.
    pub fn render_filter_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.mode {
            Mode::Filtering { text, highlighted, suggestions } => {
                Some(self.render_bar(text, *highlighted, suggestions, cx).into_any_element())
            }
            Mode::NamingView { text } => Some(self.render_naming(text, cx).into_any_element()),
            _ => None,
        };
        div().absolute().top(px(44.)).left(px(SIDEBAR_WIDTH)).right_0().flex().justify_center().children(content)
    }

    fn render_bar(&self, text: &TextField, highlighted: usize, suggestions: &[BarItem], cx: &mut Context<Self>) -> impl IntoElement {
        let start = highlighted.saturating_sub(VISIBLE_SUGGESTIONS - 1);
        let field = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .py(px(10.))
            .text_size(px(14.))
            .children(self.filter.conditions().iter().enumerate().map(|(i, condition)| {
                token(condition.label())
                    .id(SharedString::from(format!("bar-token-{i}")))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_condition(i, cx)))
            }))
            .when(text.is_empty() && self.filter.is_empty(), |field| {
                field.child(div().text_color(theme::TEXT_DIM).child("Filter by rating, flag, label, camera, lens, folder or date"))
            })
            .child(self.render_field(text, cx));

        let rows = suggestions.iter().enumerate().skip(start).take(VISIBLE_SUGGESTIONS).map(|(i, suggestion)| {
            let on = i == highlighted;
            div()
                .id(SharedString::from(format!("suggestion-{i}")))
                .flex()
                .justify_between()
                .px(px(10.))
                .py(px(6.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(if on { theme::TEXT } else { theme::TEXT_MUTED })
                .when(on, |row| row.bg(theme::HOVER))
                .when(!on, |row| row.hover(|row| row.bg(theme::LINE)))
                .child(match suggestion {
                    BarItem::View(view) => div()
                        .flex()
                        .gap(px(8.))
                        .child(div().text_color(theme::TEXT_DIM).child("View"))
                        .child(self.views[*view].name.clone()),
                    BarItem::Condition(suggestion) => div().child(suggestion.condition.label()),
                })
                .child(div().text_color(theme::TEXT_DIM).child(match suggestion {
                    BarItem::View(view) => self.view_counts.get(*view).copied().unwrap_or(0).to_string(),
                    BarItem::Condition(suggestion) => suggestion.count.to_string(),
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.add_suggestion(i, cx)))
        });

        floating(560.)
            .child(field)
            .child(
                div()
                    .border_t_1()
                    .border_color(theme::LINE)
                    .p(px(4.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .children(rows)
                    .when(suggestions.is_empty(), |list| {
                        list.child(div().px(px(10.)).py(px(6.)).text_color(theme::TEXT_DIM).child("Nothing matches"))
                    }),
            )
            .child(hints(format!("⏎ add or open   ⌫ remove last   ↑↓ choose   {} save as view   Esc close", command("S"))))
    }

    /// The active conditions above the grid, each removable, with the save
    /// or delete action for views.
    pub fn render_filter_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active_view().map(|v| v.name.clone());
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .px(px(18.))
            .pt(px(44.))
            .pb(px(4.))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .children(self.filter.conditions().iter().enumerate().map(|(i, condition)| {
                token(format!("{}  ×", condition.label()))
                    .id(SharedString::from(format!("strip-token-{i}")))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_condition(i, cx)))
            }))
            .child(link("Clear  Esc", "strip-clear").on_click(cx.listener(|this, _, _, cx| this.clear_filter(cx))))
            .child(match active {
                Some(name) => div()
                    .flex()
                    .gap(px(10.))
                    .items_center()
                    .child(div().text_color(theme::TEXT_DIM).child(format!("View: {name}")))
                    .child(link(format!("Delete view  {}", command("⌫")), "strip-delete").on_click(cx.listener(|this, _, _, cx| this.delete_active_view(cx))))
                    .into_any_element(),
                None => link(format!("Save as view  {}", command("S")), "strip-save")
                    .on_click(cx.listener(|this, _, _, cx| this.start_naming(cx)))
                    .into_any_element(),
            })
    }

    /// The prompt for a view's name.
    fn render_naming(&self, text: &TextField, cx: &mut Context<Self>) -> impl IntoElement {
        floating(360.)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme::TEXT_DIM).child("Save view as"))
                    .child(self.render_input(text, cx)),
            )
            .child(hints("⏎ save   Esc cancel"))
    }
}

/// The raised panel the bar and the save prompt share.
fn floating(width: f32) -> gpui::Div {
    div()
        .w(px(width))
        .flex()
        .flex_col()
        .rounded(px(12.))
        .bg(theme::RAISED)
        .border_1()
        .border_color(theme::LINE)
        .shadow_lg()
        .text_color(theme::TEXT)
}

fn hints(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .border_t_1()
        .border_color(theme::LINE)
        .px(px(12.))
        .py(px(7.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::TEXT_DIM)
        .child(text.into())
}

/// One active condition. Clicking it removes it.
fn token(text: String) -> gpui::Div {
    div()
        .px(px(8.))
        .py(px(3.))
        .rounded(px(6.))
        .bg(theme::HOVER)
        .text_size(px(12.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme::ACCENT_SOFT)
        .cursor_pointer()
        .hover(|token| token.bg(theme::LINE))
        .child(text)
}

fn link(text: impl Into<SharedString>, id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(4.))
        .cursor_pointer()
        .text_color(theme::TEXT_DIM)
        .hover(|link| link.text_color(theme::TEXT))
        .child(text.into())
}
