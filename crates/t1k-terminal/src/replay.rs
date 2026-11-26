//! A terminal's state written out as the escape sequences that rebuild it.
//!
//! The session daemon keeps a copy of every tab's screen. When the app attaches to a tab that
//! was already running (after a crash, a freeze or an update), the daemon sends this first,
//! so the app's fresh grid ends up with the same history, screen, colors, links, cursor and
//! modes, and the live output carries on from there.
//!
//! Not carried: the main screen while a full-screen program has the alternate screen up
//! (alacritty doesn't expose it; the program's own redraw on the attach resize fills the
//! alternate screen back in), the scroll region, tab stops, charsets and palette changes.

use std::fmt::Write;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor};

/// The attributes a run of cells shares.
#[derive(Clone, PartialEq)]
struct Pen {
    fg: Color,
    bg: Color,
    flags: Flags,
    underline: Option<Color>,
}

const STYLE: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::UNDERLINE)
    .union(Flags::DOUBLE_UNDERLINE)
    .union(Flags::UNDERCURL)
    .union(Flags::DOTTED_UNDERLINE)
    .union(Flags::DASHED_UNDERLINE)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

impl Pen {
    fn of(cell: &Cell) -> Self {
        Pen { fg: cell.fg, bg: cell.bg, flags: cell.flags & STYLE, underline: cell.underline_color() }
    }

    fn plain() -> Self {
        Pen { fg: Color::Named(NamedColor::Foreground), bg: Color::Named(NamedColor::Background), flags: Flags::empty(), underline: None }
    }

    fn sgr(&self, out: &mut String) {
        let mut s = String::from("0");
        for (flag, code) in [
            (Flags::BOLD, "1"),
            (Flags::DIM, "2"),
            (Flags::ITALIC, "3"),
            (Flags::UNDERLINE, "4"),
            (Flags::DOUBLE_UNDERLINE, "4:2"),
            (Flags::UNDERCURL, "4:3"),
            (Flags::DOTTED_UNDERLINE, "4:4"),
            (Flags::DASHED_UNDERLINE, "4:5"),
            (Flags::INVERSE, "7"),
            (Flags::HIDDEN, "8"),
            (Flags::STRIKEOUT, "9"),
        ] {
            if self.flags.contains(flag) {
                s.push(';');
                s.push_str(code);
            }
        }
        if let Some(c) = color(self.fg, 30) {
            let _ = write!(s, ";{c}");
        }
        if let Some(c) = color(self.bg, 40) {
            let _ = write!(s, ";{c}");
        }
        match self.underline {
            Some(Color::Spec(rgb)) => {
                let _ = write!(s, ";58:2::{}:{}:{}", rgb.r, rgb.g, rgb.b);
            }
            Some(Color::Indexed(i)) => {
                let _ = write!(s, ";58:5:{i}");
            }
            Some(Color::Named(n)) if (n as usize) < 16 => {
                let _ = write!(s, ";58:5:{}", n as usize);
            }
            _ => {}
        }
        let _ = write!(out, "\x1b[{s}m");
    }
}

/// SGR parameters for a color (`base` 30 for the foreground, 40 for the background); None for
/// the default.
fn color(c: Color, base: usize) -> Option<String> {
    match c {
        Color::Named(n) if (n as usize) < 8 => Some(format!("{}", base + n as usize)),
        Color::Named(n) if (n as usize) < 16 => Some(format!("{}", base + 60 + n as usize - 8)),
        Color::Named(_) => None,
        Color::Indexed(i) => Some(format!("{};5;{i}", base + 8)),
        Color::Spec(rgb) => Some(format!("{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b)),
    }
}

/// Private modes written back after the screen, as `(mode, DECSET number)`.
const DECSET: &[(TermMode, u16)] = &[
    (TermMode::APP_CURSOR, 1),
    (TermMode::LINE_WRAP, 7),
    (TermMode::SHOW_CURSOR, 25),
    (TermMode::MOUSE_REPORT_CLICK, 1000),
    (TermMode::MOUSE_DRAG, 1002),
    (TermMode::MOUSE_MOTION, 1003),
    (TermMode::FOCUS_IN_OUT, 1004),
    (TermMode::UTF8_MOUSE, 1005),
    (TermMode::SGR_MOUSE, 1006),
    (TermMode::ALTERNATE_SCROLL, 1007),
    (TermMode::URGENCY_HINTS, 1042),
    (TermMode::BRACKETED_PASTE, 2004),
];

/// The kitty keyboard flags, in protocol bit order.
const KITTY: &[TermMode] = &[
    TermMode::DISAMBIGUATE_ESC_CODES,
    TermMode::REPORT_EVENT_TYPES,
    TermMode::REPORT_ALTERNATE_KEYS,
    TermMode::REPORT_ALL_KEYS_AS_ESC,
    TermMode::REPORT_ASSOCIATED_TEXT,
];

/// Escape sequences that turn a new terminal of the same size into a copy of `term`.
pub fn replay<T: EventListener>(term: &Term<T>, title: Option<&str>) -> Vec<u8> {
    let mut out = String::new();
    let mode = *term.mode();
    if mode.contains(TermMode::ALT_SCREEN) {
        out.push_str("\x1b[?1049h");
    }
    let grid = term.grid();
    let cols = term.columns();
    let top = term.topmost_line().0;
    let bottom = term.bottommost_line().0;
    let mut pen = Pen::plain();
    let mut link: Option<(String, String)> = None;
    out.push_str("\x1b[0m");
    for l in top..=bottom {
        let row = &grid[Line(l)];
        let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        // Up to the last cell that shows something; a wrapped row is written in full so the
        // next one wraps onto it as before.
        let end = if wrapped {
            cols
        } else {
            (0..cols)
                .rposition(|c| {
                    let cell = &row[Column(c)];
                    !matches!(cell.c, ' ' | '\0') || Pen::of(cell) != Pen::plain() || cell.hyperlink().is_some()
                })
                .map_or(0, |c| c + 1)
        };
        for c in 0..end {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let p = Pen::of(cell);
            if p != pen {
                p.sgr(&mut out);
                pen = p;
            }
            let here = cell.hyperlink().map(|h| (h.id().to_string(), h.uri().to_string()));
            if here != link {
                match &here {
                    Some((id, uri)) => {
                        let _ = write!(out, "\x1b]8;id={id};{uri}\x1b\\");
                    }
                    None => out.push_str("\x1b]8;;\x1b\\"),
                }
                link = here;
            }
            out.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                out.extend(extra.iter());
            }
        }
        if !wrapped && l < bottom {
            // A default background for the line break, so erasing doesn't paint the new line.
            if pen.bg != Color::Named(NamedColor::Background) {
                Pen::plain().sgr(&mut out);
                pen = Pen::plain();
            }
            out.push_str("\r\n");
        }
    }
    if link.is_some() {
        out.push_str("\x1b]8;;\x1b\\");
    }
    // The cursor, and the pen programs left it with.
    let cursor = &grid.cursor;
    let _ = write!(out, "\x1b[{};{}H", cursor.point.line.0 + 1, cursor.point.column.0 + 1);
    Pen::of(&cursor.template).sgr(&mut out);
    for &(m, n) in DECSET {
        let _ = write!(out, "\x1b[?{n}{}", if mode.contains(m) { 'h' } else { 'l' });
    }
    out.push_str(if mode.contains(TermMode::APP_KEYPAD) { "\x1b=" } else { "\x1b>" });
    out.push_str(if mode.contains(TermMode::INSERT) { "\x1b[4h" } else { "\x1b[4l" });
    out.push_str(if mode.contains(TermMode::LINE_FEED_NEW_LINE) { "\x1b[20h" } else { "\x1b[20l" });
    let kitty: u32 = KITTY.iter().enumerate().filter(|(_, m)| mode.contains(**m)).map(|(i, _)| 1 << i).sum();
    if kitty != 0 {
        let _ = write!(out, "\x1b[={kitty};1u");
    }
    if let Some(t) = title {
        let t: String = t.chars().filter(|c| !c.is_control()).collect();
        let _ = write!(out, "\x1b]2;{t}\x1b\\");
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{test::TermSize, Config};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term(bytes: &[u8], cols: usize, rows: usize) -> Term<VoidListener> {
        let config = Config { kitty_keyboard: true, ..Config::default() };
        let mut t = Term::new(config, &TermSize::new(cols, rows), VoidListener);
        let mut p: Processor<StdSyncHandler> = Default::default();
        p.advance(&mut t, bytes);
        t
    }

    /// Every cell, the cursor and the modes match.
    fn assert_same(a: &Term<VoidListener>, b: &Term<VoidListener>) {
        assert_eq!(a.grid().history_size(), b.grid().history_size(), "history");
        assert_eq!(a.grid().cursor.point, b.grid().cursor.point, "cursor");
        assert_eq!(a.mode(), b.mode(), "modes");
        let cols = a.columns();
        for l in a.topmost_line().0..=a.bottommost_line().0 {
            for c in 0..cols {
                let (x, y) = (&a.grid()[Line(l)][Column(c)], &b.grid()[Line(l)][Column(c)]);
                let at = format!("line {l} col {c}");
                assert_eq!(x.c, y.c, "{at}");
                assert_eq!(x.fg, y.fg, "{at}");
                assert_eq!(x.bg, y.bg, "{at}");
                assert_eq!(x.flags, y.flags, "{at}");
                assert_eq!(x.zerowidth(), y.zerowidth(), "{at}");
                assert_eq!(x.underline_color(), y.underline_color(), "{at}");
                assert_eq!(x.hyperlink().map(|h| h.uri().to_string()), y.hyperlink().map(|h| h.uri().to_string()), "{at}");
            }
        }
    }

    fn round_trip(bytes: &[u8], cols: usize, rows: usize) {
        let src = term(bytes, cols, rows);
        let copy = term(&replay(&src, None), cols, rows);
        assert_same(&src, &copy);
    }

    #[test]
    fn history_colors_and_wraps() {
        let mut s = String::new();
        for i in 0..40 {
            let _ = write!(s, "\x1b[3{}mline {i}\x1b[0m \x1b[1;4:3;58:2::255:0:0mbold\x1b[0m\r\n", i % 8);
        }
        s.push_str(&"wrapped text ".repeat(5));
        s.push_str("\r\n\x1b[48;2;10;20;30m bg \x1b[0m\x1b[38;5;200mindexed\x1b[0m  ");
        round_trip(s.as_bytes(), 20, 8);
    }

    #[test]
    fn wide_chars_links_and_combining() {
        let s = "日本語のテキスト折り返し\r\ne\u{301}\u{302} \x1b]8;;t1k://prompt/cmd\x1b\\ls -la\x1b]8;;\x1b\\ end\r\n$ ";
        round_trip(s.as_bytes(), 9, 4);
    }

    #[test]
    fn modes_and_cursor() {
        round_trip(b"\x1b[?2004h\x1b[?1004h\x1b[?1h\x1b=\x1b[?1000h\x1b[?1006h\x1b[?25l\x1b[>1uhello\x1b[3;5H\x1b[31m", 30, 6);
    }

    #[test]
    fn alternate_screen() {
        let src = term(b"shell\r\n\x1b[?1049h\x1b[2J\x1b[Hvim screen\x1b[2;1H~", 20, 5);
        let copy = term(&replay(&src, None), 20, 5);
        assert!(copy.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(copy.grid()[Line(0)][Column(0)].c, 'v');
        assert_eq!(copy.grid().cursor.point, src.grid().cursor.point);
    }
}
