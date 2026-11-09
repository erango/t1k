//! Commands and their output, from the prompt marks shell integration leaves on cells: the
//! prompt and command line carry an OSC 8 link to `t1k://prompt`, and the typed command
//! itself `t1k://prompt/cmd` (zle-line-init switches the link). Everything after a marked run
//! and before the next one is that command's output.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;

pub const PROMPT_MARK: &str = "t1k://prompt";
const CMD_MARK: &str = "t1k://prompt/cmd";

/// One command: its marked prompt lines and the output lines after them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Block {
    /// First and last marked line (prompt and command).
    pub prompt: (i32, i32),
    /// Output lines, inclusive; `None` when it printed nothing.
    pub output: Option<(i32, i32)>,
    /// Still running: the last block, with the cursor past its prompt.
    pub running: bool,
}

pub fn marked<T>(term: &Term<T>, l: i32) -> bool {
    let grid = term.grid();
    (0..term.columns().min(16)).any(|c| grid[Line(l)][Column(c)].hyperlink().is_some_and(|h| h.uri().starts_with(PROMPT_MARK)))
}

fn has_cmd<T>(term: &Term<T>, l: i32) -> bool {
    let row = &term.grid()[Line(l)];
    (0..term.columns()).any(|c| row[Column(c)].hyperlink().is_some_and(|h| h.uri() == CMD_MARK))
}

/// The line's first marked cell is the prompt itself, not a wrapped or continued command.
fn starts_prompt<T>(term: &Term<T>, l: i32) -> bool {
    let row = &term.grid()[Line(l)];
    (0..term.columns().min(16)).find_map(|c| row[Column(c)].hyperlink()).is_some_and(|h| h.uri() == PROMPT_MARK)
}

/// Every command in the history, oldest first. The prompt waiting for input (cursor on it) is
/// not a command yet and is left out.
pub fn blocks<T>(term: &Term<T>) -> Vec<Block> {
    let (top, bottom) = (term.topmost_line().0, term.bottommost_line().0);
    let mut runs: Vec<(i32, i32)> = Vec::new();
    let mut l = top;
    while l <= bottom {
        if marked(term, l) {
            let start = l;
            // A command line followed straight away by a prompt (a command that printed
            // nothing) starts a new command.
            while l < bottom && marked(term, l + 1) && !(has_cmd(term, l) && starts_prompt(term, l + 1)) {
                l += 1;
            }
            runs.push((start, l));
        }
        l += 1;
    }
    let cursor = term.grid().cursor.point.line.0;
    let mut out = Vec::new();
    for (i, &(a, b)) in runs.iter().enumerate() {
        let next = runs.get(i + 1).map(|r| r.0);
        let waiting = next.is_none() && (a..=b).contains(&cursor);
        if waiting {
            break;
        }
        let end = match next {
            Some(n) => n - 1,
            None => cursor,
        };
        let output = trim_blank(term, b + 1, end);
        out.push(Block { prompt: (a, b), output, running: next.is_none() });
    }
    out
}

/// Drops blank lines at the end of `start..=end`.
fn trim_blank<T>(term: &Term<T>, start: i32, mut end: i32) -> Option<(i32, i32)> {
    while end >= start && line_text(term, end).trim().is_empty() {
        end -= 1;
    }
    (end >= start).then_some((start, end))
}

fn line_text<T>(term: &Term<T>, l: i32) -> String {
    let last = Column(term.columns().saturating_sub(1));
    term.bounds_to_string(Point::new(Line(l), Column(0)), Point::new(Line(l), last))
}

/// The block's output as text: wrapped lines joined, trailing spaces trimmed.
pub fn output_text<T>(term: &Term<T>, b: &Block) -> String {
    let Some((s, e)) = b.output else { return String::new() };
    let last = Column(term.columns().saturating_sub(1));
    let text = term.bounds_to_string(Point::new(Line(s), Column(0)), Point::new(Line(e), last));
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n")
}

/// The command as typed: the cells under the command mark. Shells without it (or a prompt
/// redrawn after the mark switched) fall back to the last marked line minus a prompt glyph.
pub fn command_text<T>(term: &Term<T>, b: &Block) -> String {
    let grid = term.grid();
    let mut text = String::new();
    for l in b.prompt.0..=b.prompt.1 {
        let row = &grid[Line(l)];
        let mut line = String::new();
        for c in 0..term.columns() {
            let cell = &row[Column(c)];
            if cell.hyperlink().is_some_and(|h| h.uri() == CMD_MARK) {
                line.push(cell.c);
            }
        }
        let line = line.trim_end();
        if !line.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(line);
        }
    }
    let text = if text.is_empty() { line_text(term, b.prompt.1) } else { text };
    strip_prompt(text.trim()).to_string()
}

/// "❯ git status" → "git status": a prompt drawn under the command mark (a transient prompt)
/// or the fallback's whole line.
fn strip_prompt(s: &str) -> &str {
    for glyph in ["❯", "›", "➜", "→", "λ", "$", "%", "#", ">"] {
        if let Some(i) = s.rfind(&format!("{glyph} ")) {
            return s[i + glyph.len() + 1..].trim_start();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::strip_prompt;

    #[test]
    fn strips_prompt_glyphs() {
        assert_eq!(strip_prompt("~/code ❯ git status"), "git status");
        assert_eq!(strip_prompt("user@host % ls -la"), "ls -la");
        assert_eq!(strip_prompt("cargo test"), "cargo test");
    }
}

#[cfg(test)]
mod term_tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{test::TermSize, Config};
    use alacritty_terminal::vte::ansi::Processor;

    fn term(bytes: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &TermSize::new(40, 10), VoidListener);
        let mut p: Processor = Processor::new();
        p.advance(&mut t, bytes.replace('\n', "\r\n").as_bytes());
        t
    }

    const P: &str = "\x1b]8;;t1k://prompt\x1b\\~ % \x1b]8;;t1k://prompt/cmd\x1b\\";
    const END: &str = "\x1b]8;;\x1b\\";

    #[test]
    fn finds_commands_and_output() {
        let t = term(&format!("{P}seq 3{END}\n1\n2\n3\n\n{P}true{END}\n{P}"));
        let b = blocks(&t);
        assert_eq!(b.len(), 2);
        assert_eq!(output_text(&t, &b[0]), "1\n2\n3");
        assert_eq!(command_text(&t, &b[0]), "seq 3");
        assert!(!b[0].running);
        assert_eq!(b[1].output, None);
        assert_eq!(command_text(&t, &b[1]), "true");
    }

    #[test]
    fn running_command_so_far() {
        let t = term(&format!("{P}npm run dev{END}\nready\nlistening"));
        let b = blocks(&t);
        assert_eq!(b.len(), 1);
        assert!(b[0].running);
        assert_eq!(output_text(&t, &b[0]), "ready\nlistening");
    }

    #[test]
    fn no_marks_no_blocks() {
        assert!(blocks(&term("plain\noutput\n")).is_empty());
    }
}
