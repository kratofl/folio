//! Text editing for the window's one active field (an outing's name or
//! category, the ⌘K bar, a view's name). Typed text arrives through the
//! platform's input handler, so dead keys and input methods compose like
//! anywhere else on the system. Editing keys (arrows, Backspace, ⌘A, ⌘V)
//! come as key presses. The editing rules live in `folio_core::text`.

use std::ops::Range;

use folio_core::text::{Step, TextField};
use gpui::{
    Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, IntoElement, Keystroke, Pixels, Point,
    UTF16Selection, Window, canvas, div, prelude::*, px,
};

use crate::app::{Folio, Mode};
use crate::theme;

impl Folio {
    /// The field being edited, if any.
    fn field(&self) -> Option<&TextField> {
        match &self.mode {
            Mode::Typing { text, .. } | Mode::Filtering { text, .. } | Mode::NamingView { text } => Some(text),
            _ => None,
        }
    }

    fn field_mut(&mut self) -> Option<&mut TextField> {
        match &mut self.mode {
            Mode::Typing { text, .. } | Mode::Filtering { text, .. } | Mode::NamingView { text } => Some(text),
            _ => None,
        }
    }

    /// What follows a change to the text: fresh suggestions in the ⌘K bar,
    /// a re-measured header for an outing.
    fn text_changed(&mut self, cx: &mut Context<Self>) {
        if let Mode::Filtering { highlighted, .. } = &mut self.mode {
            *highlighted = 0;
        }
        self.after_text_change();
        cx.notify();
    }

    /// The editing keys every field shares. Returns false for keys it
    /// doesn't use. Printable keys never get here as text: they arrive
    /// through the input handler.
    pub fn edit_key(&mut self, keystroke: &Keystroke, secondary: bool, cx: &mut Context<Self>) -> bool {
        let modifiers = &keystroke.modifiers;
        // ⌥ jumps words and ⌘ the whole line on macOS. Windows uses Ctrl for
        // words and Home and End for the line.
        let step = if cfg!(target_os = "macos") && modifiers.platform {
            Step::All
        } else if (cfg!(target_os = "macos") && modifiers.alt) || (!cfg!(target_os = "macos") && modifiers.control) {
            Step::Word
        } else {
            Step::Char
        };
        let extend = modifiers.shift;
        let Some(field) = self.field_mut() else { return false };
        let before = field.text().to_owned();
        match keystroke.key.as_str() {
            "backspace" => field.delete_backward(step),
            "delete" => field.delete_forward(step),
            "left" => field.move_caret(false, step, extend),
            "right" => field.move_caret(true, step, extend),
            "home" => field.move_caret(false, Step::All, extend),
            "end" => field.move_caret(true, Step::All, extend),
            "a" if secondary => field.select_all(),
            "c" if secondary => {
                cx.write_to_clipboard(ClipboardItem::new_string(field.selected_text().to_owned()));
            }
            "x" if secondary => {
                cx.write_to_clipboard(ClipboardItem::new_string(field.selected_text().to_owned()));
                field.replace(None, "");
            }
            "v" if secondary => {
                let pasted = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
                // One line only.
                field.replace(None, &pasted.replace(['\r', '\n'], " "));
            }
            _ => return false,
        }
        if field.text() != before {
            self.text_changed(cx);
        } else {
            cx.notify();
        }
        true
    }

    /// The text of the active field with its selection, composing text and
    /// caret, plus the hook that lets the platform's input handler reach it.
    /// The caret doesn't blink on purpose: no animation that keeps repainting.
    pub fn render_field(&self, field: &TextField, cx: &mut Context<Self>) -> impl IntoElement {
        let text = field.text();
        let selection = field.selection();
        let marked = field.marked();
        let caret = field.caret();
        let mut cuts = vec![0, text.len(), selection.start, selection.end, caret];
        cuts.extend(marked.iter().flat_map(|m| [m.start, m.end]));
        cuts.sort_unstable();
        cuts.dedup();

        let mut row = div().relative().flex().items_center().whitespace_nowrap();
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if selection.is_empty() && start == caret {
                row = row.child(caret_bar());
            }
            let selected = selection.start <= start && end <= selection.end && !selection.is_empty();
            let composing = marked.as_ref().is_some_and(|m| m.start <= start && end <= m.end);
            row = row.child(
                div()
                    .when(selected, |segment| segment.bg(theme::HOVER).rounded(px(2.)))
                    .when(composing, |segment| segment.underline())
                    .child(text[start..end].to_owned()),
            );
        }
        if selection.is_empty() && caret == text.len() {
            row = row.child(caret_bar());
        }
        let entity = cx.entity();
        let focus = self.focus.clone();
        row.child(
            canvas(|_, _, _| {}, move |bounds, _, window, cx| window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx))
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
    }
}

fn caret_bar() -> impl IntoElement {
    div().w(px(2.)).h(px(17.)).mx(px(-1.)).bg(theme::ACCENT)
}

/// The platform's side of editing. Input methods count in UTF-16; the
/// field converts. With no field active everything is ignored, which also
/// covers text arriving just after Enter closed the field.
impl EntityInputHandler for Folio {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let field = self.field()?;
        let range = field.range_from_utf16(&range);
        adjusted_range.replace(field.range_to_utf16(&range));
        Some(field.text()[range].to_owned())
    }

    fn selected_text_range(&mut self, _ignore_disabled_input: bool, _window: &mut Window, _cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let field = self.field()?;
        Some(UTF16Selection { range: field.range_to_utf16(&field.selection()), reversed: field.is_reversed() })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        let field = self.field()?;
        field.marked().map(|marked| field.range_to_utf16(&marked))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.field_mut() {
            field.unmark();
            cx.notify();
        }
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.field_mut() else { return };
        let range = range.map(|range| field.range_from_utf16(&range));
        field.replace(range, text);
        self.text_changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.field_mut() else { return };
        let range = range.map(|range| field.range_from_utf16(&range));
        field.replace_and_mark(range, new_text, new_selected_range);
        self.text_changed(cx);
    }

    fn set_selected_text_range(&mut self, range: Range<usize>, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.field_mut() else { return };
        let range = field.range_from_utf16(&range);
        field.select(range);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        let field = self.field()?;
        Some(field.to_utf16(field.text().len()))
    }

    /// The whole field, so the input method's candidate window opens just
    /// below it.
    fn bounds_for_range(&mut self, _range: Range<usize>, bounds: Bounds<Pixels>, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        self.field().map(|_| bounds)
    }

    fn character_index_for_point(&mut self, _point: Point<Pixels>, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        None
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.field().is_some()
    }
}
