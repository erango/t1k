//! Translate GPUI keystrokes into the byte sequences a terminal program expects.

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn modifier_param(k: &Keystroke) -> u8 {
    1 + k.modifiers.shift as u8 + 2 * k.modifiers.alt as u8 + 4 * k.modifiers.control as u8
}

/// Returns the bytes to send to the pty, or `None` if the key isn't for the terminal
/// (e.g. cmd shortcuts, which are handled as app actions).
pub fn to_esc(k: &Keystroke, mode: TermMode, option_as_meta: bool) -> Option<Vec<u8>> {
    if k.modifiers.platform {
        return None;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
