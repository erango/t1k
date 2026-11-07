//! Single-line text input for overlays. `handle`: the cursor stays at the end (search
//! fields); `edit`: a real field with cursor and selection (rename).

use gpui::KeyDownEvent;

pub enum Key {
    Edited,
    Submit,
    Cancel,
    Up,
    Down,
    Left,
    Right,
    Ignored,
}

pub fn handle(text: &mut String, ev: &KeyDownEvent) -> Key {
    let ks = &ev.keystroke;
    let m = &ks.modifiers;
    match ks.key.as_str() {
        "escape" => return Key::Cancel,
        "enter" => return Key::Submit,
        "up" => return Key::Up,
        "down" => return Key::Down,
        "left" => return Key::Left,
        "right" => return Key::Right,
        "backspace" => {
            if m.platform || m.alt {
                // Delete a word (alt) or everything (cmd).
                if m.platform {
                    text.clear();
                } else {
                    let trimmed = text.trim_end().len();
                    let cut = text[..trimmed].rfind(' ').map(|i| i + 1).unwrap_or(0);
                    text.truncate(cut);
                }
            } else {
                text.pop();
            }
            return Key::Edited;
        }
        _ => {}
    }
    if m.control && ks.key == "n" {
        return Key::Down;
    }
    if m.control && ks.key == "p" {
        return Key::Up;
    }
    if m.platform || m.control || m.function {
        return Key::Ignored;
    }
    match &ks.key_char {
        Some(c) if !c.chars().any(char::is_control) => {
            text.push_str(c);
            Key::Edited
        }
        _ => Key::Ignored,
    }
}

/// Cursor and selection in an editable field, as byte offsets into its text. `anchor` is
/// where the selection started; `cursor` is the end that moves.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Sel {
    pub anchor: usize,
    pub cursor: usize,
}

impl Sel {
    pub fn all(text: &str) -> Self {
        Sel { anchor: 0, cursor: text.len() }
    }
    pub fn range(&self) -> std::ops::Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    fn caret(i: usize) -> Self {
        Sel { anchor: i, cursor: i }
    }
}

fn prev_char(text: &str, i: usize) -> usize {
    text[..i].char_indices().next_back().map(|(j, _)| j).unwrap_or(0)
}
fn next_char(text: &str, i: usize) -> usize {
    text[i..].chars().next().map(|c| i + c.len_utf8()).unwrap_or(text.len())
}
/// Start of the word before `i` (skipping spaces first), as ⌥← does.
pub fn prev_word(text: &str, i: usize) -> usize {
    let head = &text[..i];
    let t = head.trim_end_matches(|c: char| !c.is_alphanumeric());
    t.rfind(|c: char| !c.is_alphanumeric()).map(|j| next_char(t, j)).unwrap_or(0)
}
/// End of the word after `i`, as ⌥→ does.
pub fn next_word(text: &str, i: usize) -> usize {
    let tail = &text[i..];
    let skip = tail.len() - tail.trim_start_matches(|c: char| !c.is_alphanumeric()).len();
    let rest = &tail[skip..];
    i + skip + rest.find(|c: char| !c.is_alphanumeric()).unwrap_or(rest.len())
}
/// The word around byte `i` (double-click).
pub fn word_at(text: &str, i: usize) -> Sel {
    let i = i.min(text.len());
    let start = text[..i].rfind(|c: char| !c.is_alphanumeric()).map(|j| next_char(text, j)).unwrap_or(0);
    let end = text[i..].find(|c: char| !c.is_alphanumeric()).map(|j| i + j).unwrap_or(text.len());
    Sel { anchor: start, cursor: end.max(start) }
}

fn replace(text: &mut String, sel: &mut Sel, with: &str) {
    let r = sel.range();
    text.replace_range(r.clone(), with);
    *sel = Sel::caret(r.start + with.len());
}

/// A single-line field with a cursor and selection, as in any macOS text field: arrows (⇧
/// extends, ⌥ by word, ⌘ to the ends), ⌫ / ⌦ (⌥ word, ⌘ to the start), ⌘A, ⌘C / ⌘X / ⌘V,
/// and typing replaces the selection.
pub fn edit(text: &mut String, sel: &mut Sel, ev: &KeyDownEvent, cx: &mut gpui::App) -> Key {
    let ks = &ev.keystroke;
    let m = &ks.modifiers;
    sel.anchor = sel.anchor.min(text.len());
    sel.cursor = sel.cursor.min(text.len());
    let moved = |sel: &mut Sel, to: usize| {
        if m.shift {
            sel.cursor = to;
        } else {
            *sel = Sel::caret(to);
        }
        Key::Ignored
    };
    match ks.key.as_str() {
        "escape" => return Key::Cancel,
        "enter" => return Key::Submit,
        "up" | "down" if !m.shift => return if ks.key == "up" { Key::Up } else { Key::Down },
        "left" | "up" | "home" => {
            let r = sel.range();
            let to = if m.platform || ks.key != "left" {
                0
            } else if m.alt {
                prev_word(text, sel.cursor)
            } else if !m.shift && !r.is_empty() {
                r.start
            } else {
                prev_char(text, sel.cursor)
            };
            return moved(sel, to);
        }
        "right" | "down" | "end" => {
            let r = sel.range();
            let to = if m.platform || ks.key != "right" {
                text.len()
            } else if m.alt {
                next_word(text, sel.cursor)
            } else if !m.shift && !r.is_empty() {
                r.end
            } else {
                next_char(text, sel.cursor)
            };
            return moved(sel, to);
        }
        "backspace" | "delete" => {
            if sel.range().is_empty() {
                let back = ks.key == "backspace";
                let to = match (back, m.platform, m.alt) {
                    (true, true, _) => 0,
                    (true, _, true) => prev_word(text, sel.cursor),
                    (true, _, _) => prev_char(text, sel.cursor),
                    (false, true, _) => text.len(),
                    (false, _, true) => next_word(text, sel.cursor),
                    (false, _, _) => next_char(text, sel.cursor),
                };
                sel.anchor = to;
            }
            replace(text, sel, "");
            return Key::Edited;
        }
        _ => {}
    }
    if m.platform {
        match ks.key.as_str() {
            "a" => *sel = Sel::all(text),
            "c" | "x" => {
                let r = sel.range();
                if !r.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text[r].to_string()));
                    if ks.key == "x" {
                        replace(text, sel, "");
                        return Key::Edited;
                    }
                }
            }
            "v" => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let line: String = t.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
                    replace(text, sel, &line);
                    return Key::Edited;
                }
            }
            _ => {}
        }
        return Key::Ignored;
    }
    if m.control && (ks.key == "n" || ks.key == "p") {
        return if ks.key == "n" { Key::Down } else { Key::Up };
    }
    if m.control || m.function {
        return Key::Ignored;
    }
    match &ks.key_char {
        Some(c) if !c.chars().any(char::is_control) => {
            replace(text, sel, c);
            Key::Edited
        }
        _ => Key::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        let t = "fix the login bug";
        assert_eq!(prev_word(t, t.len()), 14);
        assert_eq!(prev_word(t, 14), 8);
        assert_eq!(next_word(t, 0), 3);
        assert_eq!(next_word(t, 3), 7);
        assert_eq!(word_at(t, 9), Sel { anchor: 8, cursor: 13 });
    }

    #[test]
    fn replace_selection() {
        let mut t = "hello world".to_string();
        let mut s = Sel { anchor: 6, cursor: 11 };
        replace(&mut t, &mut s, "there");
        assert_eq!(t, "hello there");
        assert_eq!(s, Sel { anchor: 11, cursor: 11 });
    }
}
