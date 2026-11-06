//! Minimal single-line text input for overlays (switcher query, rename). Cursor stays at the end.

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
