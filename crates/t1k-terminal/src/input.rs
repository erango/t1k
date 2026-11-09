//! Translate GPUI keystrokes into the byte sequences a terminal program expects.

use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers};
use std::cell::Cell;

thread_local! {
    /// Caps lock, as last seen by the focused terminal: keystrokes don't carry it, and the kitty
    /// protocol reports it as a modifier bit.
    static CAPS: Cell<bool> = const { Cell::new(false) };
}

/// Record caps lock's state for the kitty protocol's lock bit.
pub fn set_caps_lock(on: bool) {
    CAPS.with(|c| c.set(on));
}

/// The kitty protocol's caps_lock modifier bit.
const CAPS_BIT: u32 = 64;

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn modifier_param(k: &Keystroke) -> u8 {
    1 + k.modifiers.shift as u8 + 2 * k.modifiers.alt as u8 + 4 * k.modifiers.control as u8
}

/// Returns the bytes to send to the pty, or `None` if the key isn't for the terminal
/// (e.g. cmd shortcuts, which are handled as app actions).
pub fn to_esc(k: &Keystroke, mode: TermMode, option_as_meta: bool) -> Option<Vec<u8>> {
    // macOS line editing, as Terminal.app and iTerm2 do it: ⌘⌫ kills the line, ⌘← / ⌘→ go to
    // the start / end, ⌘⌦ kills to the end. Other ⌘ keys are app shortcuts.
    if k.modifiers.platform {
        if k.modifiers.control || k.modifiers.alt || k.modifiers.shift {
            return None;
        }
        return match k.key.as_str() {
            "backspace" => Some(vec![0x15]),
            "left" => Some(vec![0x01]),
            "right" => Some(vec![0x05]),
            "delete" => Some(vec![0x0b]),
            _ => None,
        };
    }
    // ⌥← / ⌥→ move by word (readline's meta-b / meta-f), which shells and Claude Code understand.
    if k.modifiers.alt && !k.modifiers.control && !k.modifiers.shift {
        match k.key.as_str() {
            "left" => return Some(b"\x1bb".to_vec()),
            "right" => return Some(b"\x1bf".to_vec()),
            _ => {}
        }
    }
    if mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        return kitty(k, mode, option_as_meta, Event::Press);
    }
    let m = modifier_param(k);
    let has_mods = m > 1;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);

    // Cursor keys + home/end: CSI or SS3 depending on mode/modifiers.
    let cursor = |c: char| -> Vec<u8> {
        if has_mods {
            format!("\x1b[1;{m}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    // Tilde keys: CSI n ~ / CSI n ; m ~
    let tilde = |n: u8| -> Vec<u8> {
        if has_mods { format!("\x1b[{n};{m}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    // F1-F4 use SS3 P..S unmodified.
    let ss3 = |c: char| -> Vec<u8> {
        if has_mods { format!("\x1b[1;{m}{c}").into_bytes() } else { format!("\x1bO{c}").into_bytes() }
    };

    let alt = k.modifiers.alt;
    let ctrl = k.modifiers.control;
    let shift = k.modifiers.shift;

    let bytes = match k.key.as_str() {
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "f1" => ss3('P'),
        "f2" => ss3('Q'),
        "f3" => ss3('R'),
        "f4" => ss3('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "enter" => {
            if alt { b"\x1b\r".to_vec() } else { b"\r".to_vec() }
        }
        "backspace" => {
            let b: &[u8] = if ctrl { b"\x08" } else { b"\x7f" };
            if alt { [b"\x1b".as_slice(), b].concat() } else { b.to_vec() }
        }
        "tab" => {
            if shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() }
        }
        "escape" => b"\x1b".to_vec(),
        "space" if ctrl => vec![0],
        key => {
            if ctrl {
                let c = key.chars().next()?;
                if key.chars().count() != 1 {
                    return None;
                }
                let code = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 - b'a' + 1,
                    '@' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '/' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return None,
                };
                if alt { vec![0x1b, code] } else { vec![code] }
            } else if alt && option_as_meta {
                let mut v = vec![0x1b];
                let s = if shift { key.to_uppercase() } else { key.to_string() };
                v.extend_from_slice(if key == "space" { b" " } else { s.as_bytes() });
                v
            } else if let Some(ch) = &k.key_char {
                ch.as_bytes().to_vec()
            } else if key == "space" {
                b" ".to_vec()
            } else {
                return None;
            }
        }
    };
    Some(bytes)
}

/// A key event, for the kitty protocol's event types (flag 2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    Press,
    Repeat,
    Release,
}

/// A held key's repeats: marked as repeats for programs that asked for event types.
pub fn to_esc_held(k: &Keystroke, mode: TermMode, option_as_meta: bool, held: bool) -> Option<Vec<u8>> {
    if held && mode.contains(TermMode::REPORT_EVENT_TYPES) && !k.modifiers.platform {
        return kitty(k, mode, option_as_meta, Event::Repeat).or_else(|| to_esc(k, mode, option_as_meta));
    }
    to_esc(k, mode, option_as_meta)
}

/// A key release, for programs that asked for them (kitty flag 2). None otherwise.
pub fn key_up(k: &Keystroke, mode: TermMode, option_as_meta: bool) -> Option<Vec<u8>> {
    if !mode.contains(TermMode::REPORT_EVENT_TYPES) || k.modifiers.platform {
        return None;
    }
    kitty(k, mode, option_as_meta, Event::Release)
}

/// Pressing or releasing a modifier key on its own (shift, ctrl, ⌥, ⌘, caps lock), for
/// programs that asked for every key (kitty flag 8); releases only with event types (flag 2).
/// macOS doesn't say which side was pressed, so these report the left key.
pub fn modifier_keys(old: (Modifiers, bool), new: (Modifiers, bool), mode: TermMode, option_as_meta: bool) -> Vec<u8> {
    if !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
        return Vec::new();
    }
    let events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let (m, caps) = new;
    // The state after the event: a pressed modifier's bit is set, a released one's cleared.
    let mods = 1
        + m.shift as u32
        + 2 * (m.alt && option_as_meta) as u32
        + 4 * m.control as u32
        + 8 * m.platform as u32
        + if caps { CAPS_BIT } else { 0 };
    let keys = [
        (old.0.shift, m.shift, 57441),
        (old.0.control, m.control, 57442),
        (old.0.alt, m.alt, 57443),
        (old.0.platform, m.platform, 57444),
    ];
    let mut out = String::new();
    let mut emit = |code: u32, release: bool| {
        if release && !events {
            return;
        }
        let e = if release { ":3" } else { "" };
        let tail = if mods > 1 || release { format!(";{mods}{e}") } else { String::new() };
        out.push_str(&format!("\x1b[{code}{tail}u"));
    };
    for (was, is, code) in keys {
        if was != is {
            emit(code, was);
        }
    }
    // Caps lock is a toggle: only the switch is visible, so report it as a press and release.
    if old.1 != caps {
        emit(57358, false);
        emit(57358, true);
    }
    out.into_bytes()
}

/// The kitty keyboard protocol (https://sw.kovidgoyal.net/kitty/keyboard-protocol/), for
/// programs that turned it on (CSI > flags u): keys as CSI code ; modifiers u, so they can
/// tell shift+enter from enter, ctrl+i from tab, and esc from alt prefixes.
fn kitty(k: &Keystroke, mode: TermMode, option_as_meta: bool, ev: Event) -> Option<Vec<u8>> {
    let all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let alternates = mode.contains(TermMode::REPORT_ALTERNATE_KEYS);
    let with_text = mode.contains(TermMode::REPORT_ASSOCIATED_TEXT);
    let m = &k.modifiers;
    // ⌥ is a modifier only when it's Meta; otherwise it types characters (é, ∑).
    let alt = m.alt && option_as_meta;
    let mods = 1 + m.shift as u32 + 2 * alt as u32 + 4 * m.control as u32;
    // Caps lock rides along on CSI u keys, but not on keys that type text unless every key is
    // wanted (so it doesn't break programs reading text keys), nor on the legacy-form keys
    // (arrows, F-keys), which have no encoding for it.
    let lock = if CAPS.with(Cell::get) { CAPS_BIT } else { 0 };
    let ev_n = match ev {
        Event::Press => 1,
        Event::Repeat => 2,
        Event::Release => 3,
    };
    if ev != Event::Press && !events {
        return None;
    }
    // "; mods[:event]", left out when both are the defaults.
    let tail = |force: bool, mods: u32| -> String {
        let e = if events && ev_n != 1 { format!(":{ev_n}") } else { String::new() };
        if mods > 1 || !e.is_empty() || force { format!(";{mods}{e}") } else { String::new() }
    };
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let letter = |c: char| -> Vec<u8> {
        let t = tail(false, mods);
        if t.is_empty() {
            if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") }.into_bytes()
        } else {
            format!("\x1b[1{t}{c}").into_bytes()
        }
    };
    let tilde = |n: u32| format!("\x1b[{n}{}~", tail(false, mods)).into_bytes();
    let csi_u = |code: u32, extra: &str, text: &str, lock: u32| -> Vec<u8> {
        let mut t = tail(!text.is_empty(), mods + lock);
        if !text.is_empty() {
            t.push(';');
            t.push_str(text);
        }
        format!("\x1b[{code}{extra}{t}u").into_bytes()
    };
    let key = k.key.as_str();
    let out = match key {
        "up" => letter('A'),
        "down" => letter('B'),
        "right" => letter('C'),
        "left" => letter('D'),
        "home" => letter('H'),
        "end" => letter('F'),
        "f1" => letter('P'),
        "f2" => letter('Q'),
        "f4" => letter('S'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f3" => tilde(13),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "escape" => csi_u(27, "", "", lock),
        // Plain enter, tab and backspace stay legacy (so `reset` still works after a crashed
        // program left the mode on), unless every key is wanted as an escape.
        "enter" | "tab" | "backspace" => {
            let code = match key {
                "enter" => 13,
                "tab" => 9,
                _ => 127,
            };
            if all || mods > 1 {
                csi_u(code, "", "", lock)
            } else if ev != Event::Press {
                return None;
            } else {
                match key {
                    "enter" => b"\r".to_vec(),
                    "tab" => b"\t".to_vec(),
                    _ => b"\x7f".to_vec(),
                }
            }
        }
        _ => {
            // A text key: its code is the unshifted character, lowercase.
            let base = if key == "space" { ' ' } else {
                let mut cs = key.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) => c.to_ascii_lowercase(),
                    _ => return None,
                }
            };
            let text = k.key_char.clone().unwrap_or_else(|| if key == "space" { " ".into() } else { String::new() });
            let shifted = text.chars().next().filter(|&c| m.shift && c != base && text.chars().count() == 1);
            let extra = match shifted {
                Some(c) if alternates => format!(":{}", c as u32),
                _ => String::new(),
            };
            if all {
                let assoc = if with_text && !m.control && !alt && ev != Event::Release && !text.is_empty() {
                    text.chars().map(|c| (c as u32).to_string()).collect::<Vec<_>>().join(":")
                } else {
                    String::new()
                };
                csi_u(base as u32, &extra, &assoc, lock)
            } else if m.control || alt {
                csi_u(base as u32, &extra, "", 0)
            } else if ev != Event::Press {
                // Keys that type text report only the press, unless every key is wanted.
                return None;
            } else if !text.is_empty() {
                text.into_bytes()
            } else {
                return None;
            }
        }
    };
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_line_editing() {
        let m = TermMode::empty();
        assert_eq!(to_esc(&ks("cmd-backspace"), m, false), Some(vec![0x15]));
        assert_eq!(to_esc(&ks("cmd-left"), m, false), Some(vec![0x01]));
        assert_eq!(to_esc(&ks("cmd-right"), m, false), Some(vec![0x05]));
        assert_eq!(to_esc(&ks("alt-left"), m, false), Some(b"\x1bb".to_vec()));
        assert_eq!(to_esc(&ks("alt-backspace"), m, false), Some(b"\x1b\x7f".to_vec()));
        assert_eq!(to_esc(&ks("cmd-t"), m, false), None);
    }

    fn ks(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn arrows() {
        assert_eq!(to_esc(&ks("up"), TermMode::NONE, false).unwrap(), b"\x1b[A");
        assert_eq!(to_esc(&ks("up"), TermMode::APP_CURSOR, false).unwrap(), b"\x1bOA");
        assert_eq!(to_esc(&ks("ctrl-left"), TermMode::NONE, false).unwrap(), b"\x1b[1;5D");
    }

    #[test]
    fn ctrl_keys() {
        assert_eq!(to_esc(&ks("ctrl-c"), TermMode::NONE, false).unwrap(), vec![3]);
        assert_eq!(to_esc(&ks("ctrl-["), TermMode::NONE, false).unwrap(), vec![0x1b]);
    }

    #[test]
    fn cmd_is_app() {
        assert!(to_esc(&ks("cmd-c"), TermMode::NONE, false).is_none());
    }

    #[test]
    fn kitty_disambiguate() {
        let m = TermMode::DISAMBIGUATE_ESC_CODES;
        let k = |s: &str| to_esc(&ks(s), m, true).map(|b| String::from_utf8(b).unwrap());
        assert_eq!(k("escape").unwrap(), "\x1b[27u");
        assert_eq!(k("shift-enter").unwrap(), "\x1b[13;2u");
        assert_eq!(k("enter").unwrap(), "\r");
        assert_eq!(k("ctrl-c").unwrap(), "\x1b[99;5u");
        assert_eq!(k("alt-x").unwrap(), "\x1b[120;3u");
        assert_eq!(k("up").unwrap(), "\x1b[A");
        assert_eq!(k("ctrl-up").unwrap(), "\x1b[1;5A");
        assert_eq!(k("shift-delete").unwrap(), "\x1b[3;2~");
        assert_eq!(k("cmd-left").unwrap(), "\x01");
    }

    #[test]
    fn kitty_text_and_all_keys() {
        let mut a = ks("a");
        a.key_char = Some("a".into());
        assert_eq!(to_esc(&a, TermMode::DISAMBIGUATE_ESC_CODES, true).unwrap(), b"a");
        let all = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ASSOCIATED_TEXT;
        assert_eq!(to_esc(&a, all, true).unwrap(), b"\x1b[97;1;97u");
        assert_eq!(to_esc(&ks("enter"), all, true).unwrap(), b"\x1b[13u");
        let mut sa = ks("shift-a");
        sa.key_char = Some("A".into());
        let alt = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ALTERNATE_KEYS;
        assert_eq!(to_esc(&sa, alt, true).unwrap(), b"\x1b[97:65;2u");
    }

    #[test]
    fn kitty_modifier_keys() {
        let all = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC;
        let ev = all | TermMode::REPORT_EVENT_TYPES;
        let none = Modifiers::default();
        let shift = Modifiers { shift: true, ..Default::default() };
        let s = |b: Vec<u8>| String::from_utf8(b).unwrap();
        assert_eq!(s(modifier_keys((none, false), (shift, false), all, true)), "\x1b[57441;2u");
        assert_eq!(s(modifier_keys((shift, false), (none, false), all, true)), "");
        assert_eq!(s(modifier_keys((shift, false), (none, false), ev, true)), "\x1b[57441;1:3u");
        assert_eq!(s(modifier_keys((none, false), (none, true), ev, true)), "\x1b[57358;65u\x1b[57358;65:3u");
        assert!(modifier_keys((none, false), (shift, false), TermMode::DISAMBIGUATE_ESC_CODES, true).is_empty());
    }

    #[test]
    fn kitty_caps_lock_bit() {
        let m = TermMode::DISAMBIGUATE_ESC_CODES;
        set_caps_lock(true);
        assert_eq!(to_esc(&ks("escape"), m, true).unwrap(), b"\x1b[27;65u");
        assert_eq!(to_esc(&ks("ctrl-c"), m, true).unwrap(), b"\x1b[99;5u");
        assert_eq!(to_esc(&ks("up"), m, true).unwrap(), b"\x1b[A");
        let mut a = ks("a");
        a.key_char = Some("a".into());
        assert_eq!(to_esc(&a, m, true).unwrap(), b"a");
        assert_eq!(to_esc(&a, m | TermMode::REPORT_ALL_KEYS_AS_ESC, true).unwrap(), b"\x1b[97;65u");
        set_caps_lock(false);
    }

    #[test]
    fn kitty_releases() {
        let m = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_EVENT_TYPES;
        assert_eq!(key_up(&ks("escape"), m, true).unwrap(), b"\x1b[27;1:3u");
        assert_eq!(key_up(&ks("up"), m, true).unwrap(), b"\x1b[1;1:3A");
        let mut a = ks("a");
        a.key_char = Some("a".into());
        assert!(key_up(&a, m, true).is_none());
        assert!(key_up(&ks("escape"), TermMode::DISAMBIGUATE_ESC_CODES, true).is_none());
    }
}
