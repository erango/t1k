//! Find in the terminal (⌘F): literal, smart-case search over the scrollback and screen.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::Term;

/// More matches than this aren't worth listing; the count shows "1000+".
pub const MAX_MATCHES: usize = 1000;

#[derive(Default)]
pub struct Search {
    pub query: String,
    pub matches: Vec<Match>,
    /// Index into `matches` of the selected one.
    pub current: Option<usize>,
    /// Content changed since the last scan.
    pub dirty: bool,
}

/// Escape regex syntax so the query matches literally.
fn literal(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 2);
    for c in q.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Search {
    /// Rescan the whole buffer. Keeps the selection on the same match when it still exists,
    /// else picks the match nearest the bottom (the newest output).
    pub fn scan<T>(&mut self, term: &Term<T>) {
        self.dirty = false;
        let previous = self.current.and_then(|i| self.matches.get(i).cloned());
        self.matches.clear();
        self.current = None;
        if self.query.is_empty() {
            return;
        }
        let Ok(mut regex) = RegexSearch::new(&literal(&self.query)) else { return };
        let start = Point::new(term.topmost_line(), Column(0));
        let end = Point::new(term.bottommost_line(), term.last_column());
        self.matches = RegexIter::new(start, end, Direction::Right, term, &mut regex).take(MAX_MATCHES).collect();
        if self.matches.is_empty() {
            return;
        }
        self.current = Some(match previous {
            Some(p) => self.matches.iter().position(|m| *m == p).unwrap_or(self.matches.len() - 1),
            None => self.matches.len() - 1,
        });
    }

    /// Step to the next (`forward`) or previous match, wrapping around.
    pub fn step(&mut self, forward: bool) -> Option<&Match> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        let i = self.current.unwrap_or(n - 1);
        let next = if forward { (i + 1) % n } else { (i + n - 1) % n };
        self.current = Some(next);
        self.matches.get(next)
    }

    /// Whether `p` is inside a match, and whether that match is the selected one.
    pub fn hit(&self, p: Point) -> Option<bool> {
        // Matches are sorted; binary search for the last match starting at or before p.
        let idx = self.matches.partition_point(|m| *m.start() <= p);
        let i = idx.checked_sub(1)?;
        let m = &self.matches[i];
        (p <= *m.end()).then(|| self.current == Some(i))
    }

    pub fn label(&self) -> String {
        match (self.current, self.matches.len()) {
            (_, 0) if self.query.is_empty() => String::new(),
            (_, 0) => "No matches".into(),
            (Some(i), n) if n >= MAX_MATCHES => format!("{}/{MAX_MATCHES}+", i + 1),
            (Some(i), n) => format!("{}/{n}", i + 1),
            (None, n) => format!("{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::literal;

    #[test]
    fn escapes_regex_syntax() {
        assert_eq!(literal("a.b(c)"), "a\\.b\\(c\\)");
        assert_eq!(literal("v2.1-x"), "v2\\.1\\-x");
    }
}
