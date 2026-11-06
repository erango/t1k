//! Terminal titles from programs that put a status glyph in front, as Claude Code does
//! ("✳ Fix login bug" when idle, a spinning "◐ …" or braille frame while working). T1K strips
//! the glyph and draws status its own way.

use t1k_agent::Status;

/// What a title's leading glyph says about the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleState {
    Working,
    Idle,
}

pub static WORKING: Status = Status::Working;
pub static IDLE: Status = Status::Idle;

impl TitleState {
    pub fn status(self) -> &'static Status {
        match self {
            TitleState::Working => &WORKING,
            TitleState::Idle => &IDLE,
        }
    }
}

fn glyph_state(c: char) -> Option<TitleState> {
    match c {
        // Spinner frames: half circles, braille dots, the record dot.
        '◐' | '◓' | '◑' | '◒' | '◴' | '◵' | '◶' | '◷' | '⏺' | '●' => Some(TitleState::Working),
        '\u{2800}'..='\u{28FF}' => Some(TitleState::Working),
        // Resting marks.
        '✳' | '✻' | '✽' | '✶' | '✢' | '✱' | '❋' | '·' | '∗' => Some(TitleState::Idle),
        _ => None,
    }
}

/// Split a known status glyph off the front of a title: `("◐ Fix it")` → `(Some(Working), "Fix it")`.
/// Titles without one come back unchanged.
pub fn split(title: &str) -> (Option<TitleState>, &str) {
    let t = title.trim_start();
    let mut chars = t.chars();
    let Some(first) = chars.next() else { return (None, title) };
    match glyph_state(first) {
        Some(state) => {
            // Drop the glyph, an optional variation selector, and the spaces after it.
            let rest = chars.as_str().trim_start_matches('\u{FE0F}').trim_start();
            (Some(state), rest)
        }
        None => (None, title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_claude_status_glyphs() {
        assert_eq!(split("✳ Project overview question"), (Some(TitleState::Idle), "Project overview question"));
        assert_eq!(split("◐ Project overview question"), (Some(TitleState::Working), "Project overview question"));
        assert_eq!(split("⠂ Claude Code"), (Some(TitleState::Working), "Claude Code"));
        assert_eq!(split("vim notes.md"), (None, "vim notes.md"));
        assert_eq!(split("✳"), (Some(TitleState::Idle), ""));
    }
}
