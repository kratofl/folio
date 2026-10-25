//! A one-line text being edited: the text, the selection, and the part an
//! input method is still composing (a dead key's accent, or Japanese before
//! it's confirmed). Pure, so the editing rules are tested without a window.
//!
//! Offsets are UTF-8 byte offsets on character boundaries. Input methods
//! count in UTF-16, so the conversions live here too.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextField {
    text: String,
    /// `start..end`. Empty is a plain caret.
    selection: Range<usize>,
    /// The caret is at `start` rather than `end`, after selecting leftwards.
    reversed: bool,
    /// The input method's text that isn't confirmed yet.
    marked: Option<Range<usize>>,
}

/// How far a caret move goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Char,
    /// To the start or end of a word, like ⌥-arrow on macOS.
    Word,
    /// To the start or end of the text.
    All,
}

impl TextField {
    /// `text` with the caret after it.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self { text, selection: end..end, reversed: false, marked: None }
    }

    /// `text` selected as a whole, so the first key typed replaces it.
    pub fn selected(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self { text, selection: 0..end, reversed: false, marked: None }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn is_reversed(&self) -> bool {
        self.reversed
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Where the caret is: the moving end of the selection.
    pub fn caret(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    /// Replaces `range`, or else the composing text, or else the
    /// selection, with confirmed text. The caret ends up after it.
    pub fn replace(&mut self, range: Option<Range<usize>>, new: &str) {
        let range = self.target(range);
        self.text.replace_range(range.clone(), new);
        let caret = range.start + new.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
    }

    /// Like `replace`, but the new text is still being composed. `selected`
    /// is the input method's selection within `new`, in UTF-16 units, if it
    /// has one.
    pub fn replace_and_mark(&mut self, range: Option<Range<usize>>, new: &str, selected: Option<Range<usize>>) {
        let range = self.target(range);
        self.text.replace_range(range.clone(), new);
        let start = range.start;
        self.marked = (!new.is_empty()).then(|| start..start + new.len());
        self.selection = match selected {
            Some(selected) => {
                let at = |utf16: usize| start + utf16_to_byte(new, utf16);
                at(selected.start)..at(selected.end.max(selected.start))
            }
            None => start + new.len()..start + new.len(),
        };
        self.reversed = false;
    }

    /// Confirms the composing text as it is.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.selection = self.clamp(range.start)..self.clamp(range.end.max(range.start));
        self.reversed = false;
    }

    pub fn select_all(&mut self) {
        self.select(0..self.text.len());
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    /// Moves the caret left or right by `step`. With `extend` the selection
    /// grows or shrinks instead; without it, a selection collapses to the
    /// side the caret is moving to.
    pub fn move_caret(&mut self, forward: bool, step: Step, extend: bool) {
        if !extend && !self.selection.is_empty() && step == Step::Char {
            let to = if forward { self.selection.end } else { self.selection.start };
            self.selection = to..to;
            self.reversed = false;
            return;
        }
        let to = self.target_of(self.caret(), forward, step);
        if extend {
            let anchor = if self.reversed { self.selection.end } else { self.selection.start };
            self.reversed = to < anchor;
            self.selection = anchor.min(to)..anchor.max(to);
        } else {
            self.selection = to..to;
            self.reversed = false;
        }
    }

    /// Backspace: deletes the selection, or `step` back from the caret.
    pub fn delete_backward(&mut self, step: Step) {
        if self.selection.is_empty() {
            let from = self.target_of(self.caret(), false, step);
            self.selection = from..self.caret();
        }
        self.replace(None, "");
    }

    /// Forward delete: deletes the selection, or `step` on from the caret.
    pub fn delete_forward(&mut self, step: Step) {
        if self.selection.is_empty() {
            let to = self.target_of(self.caret(), true, step);
            self.selection = self.caret()..to;
        }
        self.replace(None, "");
    }

    /// Byte offset to the UTF-16 offset input methods use.
    pub fn to_utf16(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].encode_utf16().count()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.to_utf16(range.start)..self.to_utf16(range.end)
    }

    /// UTF-16 offset to a byte offset. One inside a surrogate pair lands
    /// before the character.
    pub fn from_utf16(&self, utf16: usize) -> usize {
        utf16_to_byte(&self.text, utf16)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.from_utf16(range.start)..self.from_utf16(range.end.max(range.start))
    }

    /// The range an edit applies to when the input method names none.
    fn target(&self, range: Option<Range<usize>>) -> Range<usize> {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection.clone());
        self.clamp(range.start)..self.clamp(range.end.max(range.start))
    }

    /// The offset `step` away from `from`.
    fn target_of(&self, from: usize, forward: bool, step: Step) -> usize {
        match (step, forward) {
            (Step::All, true) => self.text.len(),
            (Step::All, false) => 0,
            (Step::Char, true) => self.text[from..].chars().next().map_or(from, |c| from + c.len_utf8()),
            (Step::Char, false) => self.text[..from].chars().next_back().map_or(from, |c| from - c.len_utf8()),
            // Skip spaces and punctuation, then the word.
            (Step::Word, true) => {
                let rest = &self.text[from..];
                let start = rest.find(char::is_alphanumeric).unwrap_or(rest.len());
                let end = rest[start..].find(|c: char| !c.is_alphanumeric()).map_or(rest.len(), |end| start + end);
                from + end
            }
            (Step::Word, false) => {
                let before = &self.text[..from];
                let end = before.rfind(char::is_alphanumeric).map_or(0, |i| i + before[i..].chars().next().map_or(0, char::len_utf8));
                before[..end].rfind(|c: char| !c.is_alphanumeric()).map_or(0, |i| i + before[i..].chars().next().map_or(0, char::len_utf8))
            }
        }
    }

    /// Clamps to the text and rounds down to a character boundary.
    fn clamp(&self, offset: usize) -> usize {
        floor_boundary(&self.text, offset.min(self.text.len()))
    }
}

fn utf16_to_byte(text: &str, utf16: usize) -> usize {
    let mut count = 0;
    for (offset, c) in text.char_indices() {
        if count + c.len_utf16() > utf16 {
            return offset;
        }
        count += c.len_utf16();
    }
    text.len()
}

fn floor_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_a_selected_old_value() {
        let mut field = TextField::selected("Untitled");
        field.replace(None, "M");
        field.replace(None, "arket");
        assert_eq!(field.text(), "Market");
        assert_eq!(field.selection(), 6..6);
    }

    #[test]
    fn caret_moves_and_selects_by_character_word_and_all() {
        let mut field = TextField::new("Christmas market 2024");
        field.move_caret(false, Step::Word, false);
        assert_eq!(field.caret(), 17, "back over 2024");
        field.move_caret(false, Step::Word, true);
        assert_eq!(field.selected_text(), "market ");
        assert!(field.is_reversed());
        field.move_caret(true, Step::Char, true);
        assert_eq!(field.selected_text(), "arket ", "shrinks from the caret's side");
        field.move_caret(true, Step::Char, false);
        assert_eq!(field.caret(), 17, "an arrow collapses the selection to that side");
        field.move_caret(false, Step::All, true);
        assert_eq!(field.selected_text(), "Christmas market ");
    }

    #[test]
    fn deleting_respects_characters_wider_than_a_byte() {
        let mut field = TextField::new("Schäfer");
        field.move_caret(false, Step::Char, false);
        field.move_caret(false, Step::Char, false);
        field.move_caret(false, Step::Char, false);
        field.delete_backward(Step::Char);
        assert_eq!(field.text(), "Schfer");
        field.delete_backward(Step::Word);
        assert_eq!(field.text(), "fer");
        field.delete_forward(Step::All);
        assert_eq!(field.text(), "");
    }

    #[test]
    fn a_dead_key_composes_then_confirms() {
        // ⌥E then E on a Mac: the accent shows marked, then the letter replaces it.
        let mut field = TextField::new("Caf");
        field.replace_and_mark(None, "´", Some(1..1));
        assert_eq!(field.marked(), Some(3..5));
        assert_eq!(field.text(), "Caf´");
        assert_eq!(field.caret(), 5, "the input method put its caret after the accent");
        field.replace(None, "é");
        assert_eq!(field.text(), "Café");
        assert_eq!(field.marked(), None);
        assert_eq!(field.caret(), 5);
    }

    #[test]
    fn utf16_offsets_round_trip_past_emoji_and_umlauts() {
        let field = TextField::new("ä📷b");
        assert_eq!(field.to_utf16(field.text().len()), 4, "ä is one unit, the camera two");
        assert_eq!(field.from_utf16(2), 2, "inside the surrogate pair lands before it");
        assert_eq!(field.range_from_utf16(&(1..4)), 2..7);
        assert_eq!(field.range_to_utf16(&(2..7)), 1..4);
    }
}
