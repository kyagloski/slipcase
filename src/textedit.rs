//! A single-line text editor: cursor, selection, clipboard, undo, and the usual keyboard
//! shortcuts. The settings menu shows it in a dialog (see `settings.rs`).

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;

/// What a key press asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum EditResult {
    None,
    Commit,
    Cancel,
}

pub struct TextEdit {
    /// What is being edited (the dialog's title).
    pub title: String,
    text: Vec<char>,
    /// Cursor position, 0..=len (in characters).
    pub cursor: usize,
    /// Other end of the selection, if any.
    pub anchor: Option<usize>,
    undo: Vec<(Vec<char>, usize)>,
    redo: Vec<(Vec<char>, usize)>,
    /// Horizontal scroll of the field (pixels), kept by the dialog.
    pub scroll: f32,
    /// Time of the last change or movement, for the caret blink.
    pub last_input: f32,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextEdit {
    /// Starts with everything selected, so typing replaces it and arrows keep it.
    pub fn new(title: impl Into<String>, text: &str) -> Self {
        let text: Vec<char> = text.chars().collect();
        let len = text.len();
        TextEdit { title: title.into(), text, cursor: len, anchor: (len > 0).then_some(0), undo: vec![], redo: vec![], scroll: 0.0, last_input: 0.0 }
    }

    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Selected range (start, end), if anything is selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.text[a..b].iter().collect())
    }

    /// Text before character `i` (for measuring where things go).
    pub fn prefix(&self, i: usize) -> String {
        self.text[..i.min(self.text.len())].iter().collect()
    }

    fn snapshot(&mut self) {
        self.undo.push((self.text.clone(), self.cursor));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else { return false };
        self.text.drain(a..b);
        self.cursor = a;
        self.anchor = None;
        true
    }

    /// Types or pastes `s` over the selection.
    pub fn insert(&mut self, s: &str) {
        let s: Vec<char> = s.chars().filter(|c| !c.is_control()).collect();
        if s.is_empty() && self.selection().is_none() {
            return;
        }
        self.snapshot();
        self.delete_selection();
        self.anchor = None;
        let n = s.len();
        self.text.splice(self.cursor..self.cursor, s);
        self.cursor += n;
    }

    fn word_left(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && !is_word(self.text[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.text[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self, from: usize) -> usize {
        let mut i = from;
        let n = self.text.len();
        while i < n && !is_word(self.text[i]) {
            i += 1;
        }
        while i < n && is_word(self.text[i]) {
            i += 1;
        }
        i
    }

    /// Moves the cursor; with `select` the selection grows from where it was.
    pub fn move_to(&mut self, to: usize, select: bool) {
        let to = to.min(self.text.len());
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = to;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    /// Selects the word (or run of other characters) around character `i`.
    pub fn select_word_at(&mut self, i: usize) {
        let n = self.text.len();
        if n == 0 {
            return;
        }
        let i = i.min(n - 1);
        let kind = is_word(self.text[i]);
        let (mut a, mut b) = (i, i + 1);
        while a > 0 && is_word(self.text[a - 1]) == kind {
            a -= 1;
        }
        while b < n && is_word(self.text[b]) == kind {
            b += 1;
        }
        self.anchor = Some(a);
        self.cursor = b;
    }

    fn erase(&mut self, forward: bool, word: bool) {
        if self.selection().is_some() {
            self.snapshot();
            self.delete_selection();
            return;
        }
        let (a, b) = if forward {
            (self.cursor, if word { self.word_right(self.cursor) } else { (self.cursor + 1).min(self.text.len()) })
        } else {
            (if word { self.word_left(self.cursor) } else { self.cursor.saturating_sub(1) }, self.cursor)
        };
        if a == b {
            return;
        }
        self.snapshot();
        self.text.drain(a..b);
        self.cursor = a;
        self.anchor = None;
    }

    fn undo(&mut self) {
        if let Some((t, c)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.text, t), self.cursor));
            self.cursor = c.min(self.text.len());
            self.anchor = None;
        }
    }

    fn redo(&mut self) {
        if let Some((t, c)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.text, t), self.cursor));
            self.cursor = c.min(self.text.len());
            self.anchor = None;
        }
    }

    /// Handles one key press (`ctrl` is Ctrl, or Cmd on a Mac). Repeats arrive as presses too.
    pub fn key(&mut self, e: &KeyboardInput, ctrl: bool, shift: bool, now: f32) -> EditResult {
        if e.state != ButtonState::Pressed {
            return EditResult::None;
        }
        self.last_input = now;
        let n = self.text.len();
        match &e.logical_key {
            Key::Enter => return EditResult::Commit,
            Key::Escape => return EditResult::Cancel,
            Key::ArrowLeft => {
                // Without shift, a selection collapses to its left end.
                let to = match (self.selection(), shift, ctrl) {
                    (Some((a, _)), false, false) => a,
                    (_, _, true) => self.word_left(self.cursor),
                    _ => self.cursor.saturating_sub(1),
                };
                self.move_to(to, shift);
            }
            Key::ArrowRight => {
                let to = match (self.selection(), shift, ctrl) {
                    (Some((_, b)), false, false) => b,
                    (_, _, true) => self.word_right(self.cursor),
                    _ => (self.cursor + 1).min(n),
                };
                self.move_to(to, shift);
            }
            Key::Home | Key::ArrowUp => self.move_to(0, shift),
            Key::End | Key::ArrowDown => self.move_to(n, shift),
            Key::Backspace => self.erase(false, ctrl),
            Key::Delete => self.erase(true, ctrl),
            Key::Space if !ctrl => self.insert(" "),
            Key::Character(s) if ctrl => match s.to_lowercase().as_str() {
                "a" => self.select_all(),
                "c" => {
                    if let Some(t) = self.selected_text() {
                        crate::clipboard::copy(&t);
                    }
                }
                "x" => {
                    if let Some(t) = self.selected_text() {
                        crate::clipboard::copy(&t);
                        self.erase(false, false);
                    }
                }
                "v" => {
                    if let Some(t) = crate::clipboard::paste() {
                        self.insert(&t);
                    }
                }
                "z" if shift => self.redo(),
                "z" => self.undo(),
                "y" => self.redo(),
                _ => {}
            },
            Key::Character(s) => self.insert(s.as_str()),
            _ => {}
        }
        EditResult::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_selection_words_undo() {
        let mut e = TextEdit::new("t", "hello world");
        // Starts all selected: typing replaces it.
        e.insert("abc def");
        assert_eq!(e.text(), "abc def");
        e.move_to(0, false);
        e.move_to(3, true);
        assert_eq!(e.selected_text().as_deref(), Some("abc"));
        e.insert("X");
        assert_eq!(e.text(), "X def");
        e.erase(false, false);
        assert_eq!(e.text(), " def");
        e.move_to(e.len(), false);
        e.erase(false, true);
        assert_eq!(e.text(), " ");
        e.undo();
        assert_eq!(e.text(), " def");
        e.redo();
        assert_eq!(e.text(), " ");
        let mut w = TextEdit::new("t", "/home/me/my_game.iso");
        w.select_word_at(10);
        assert_eq!(w.selected_text().as_deref(), Some("my_game"));
        assert_eq!(w.word_left(9), 6);
        assert_eq!(w.word_right(0), 5);
    }
}
