//! Terminal model: owns the pty, the alacritty `Term` grid, and the reader thread.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::Arc;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{test::TermSize, Config, Term, TermMode};
use alacritty_terminal::tty;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

/// Forwards alacritty events from the pty thread to the UI thread.
#[derive(Clone)]
pub struct Listener(UnboundedSender<AlacEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: AlacEvent) {
        let _ = self.0.unbounded_send(event);
    }
}

/// Grid geometry in cells plus cell size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSize {
    pub cols: usize,
    pub rows: usize,
    pub cell_width: f32,
    pub cell_height: f32,
}

impl GridSize {
    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.rows as u16,
            num_cols: self.cols as u16,
            cell_width: self.cell_width as u16,
            cell_height: self.cell_height as u16,
        }
    }
}

pub struct SpawnOptions {
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub tab_id: String,
    /// Start with the cursor on the bottom row, so the first prompt sits at the bottom of the
    /// pane ("prompt gravity: bottom").
    pub start_at_bottom: bool,
    /// Scrollback saved from the last run (`Terminal::snapshot`), replayed before the shell
    /// prints anything.
    pub restore: Option<Vec<u8>>,
}

pub struct Terminal {
    pub term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    pub size: GridSize,
    pub title: Option<String>,
    pub exited: bool,
    /// Master side of the pty (owned by the event loop; valid while it runs).
    pty_fd: RawFd,
    shell_pid: i32,
}

/// The user's shell as a login session, without the "Last login" banner.
///
/// Mirrors alacritty's macOS default (`login -flp user zsh -fc 'exec -a -<shell> <shell>'`) but
/// always passes `-q`, as Ghostty and Terminal.app do. `-q` also hides the `tty??` that `login`
/// prints when the pty isn't in utmp.
fn login_shell() -> Option<tty::Shell> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let user = std::env::var("USER").ok()?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let name = shell.rsplit('/').next().unwrap_or("zsh").to_string();
    let exec = format!("exec -a -{name} {shell}");
    Some(tty::Shell::new(
        "/usr/bin/login".into(),
        vec!["-qflp".into(), user, "/bin/zsh".into(), "-fc".into(), exec],
    ))
}

impl Terminal {
    /// Spawn the user's login shell in a new pty. Returns the terminal and the event stream.
    pub fn spawn(opts: SpawnOptions) -> anyhow::Result<(Self, UnboundedReceiver<AlacEvent>)> {
        let size = GridSize { cols: 80, rows: 24, cell_width: 8.0, cell_height: 16.0 };
        let (tx, rx) = unbounded();
        let listener = Listener(tx);

        let mut env = opts.env;
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        env.insert("TERM_PROGRAM".into(), "t1k".into());
        env.insert("TERM_PROGRAM_VERSION".into(), env!("CARGO_PKG_VERSION").into());
        env.insert("T1K_TAB_ID".into(), opts.tab_id);

        let pty_opts = tty::Options {
            shell: login_shell(),
            working_directory: opts.working_directory,
            drain_on_exit: true,
            env,
        };
        let pty = tty::new(&pty_opts, size.window_size(), 0)?;
        let pty_fd = pty.file().as_raw_fd();
        let shell_pid = pty.child().id() as i32;

        let config = Config { scrolling_history: 10_000, ..Config::default() };
        let term = Term::new(config, &TermSize::new(size.cols, size.rows), listener.clone());
        let mut term = term;
        if opts.start_at_bottom {
            // Feed blank lines before the shell prints anything: the cursor reaches the last row
            // and the rest scroll into history, so when the pane grows to its real height the
            // resize pulls those blanks back down and the cursor stays on the bottom row.
            use alacritty_terminal::vte::ansi::Handler;
            for _ in 0..400 {
                term.linefeed();
            }
        }
        if let Some(bytes) = opts.restore.as_deref().filter(|b| !b.is_empty()) {
            let mut p: alacritty_terminal::vte::ansi::Processor<alacritty_terminal::vte::ansi::StdSyncHandler> = Default::default();
            p.advance(&mut term, bytes);
        }
        let term = Arc::new(FairMutex::new(term));

        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();

        Ok((Self { term, notifier, size, title: None, exited: false, pty_fd, shell_pid }, rx))
    }

    /// Hang up the foreground program, as closing a terminal window does (SIGHUP to its
    /// process group). Never the shell's own group.
    pub fn hangup_foreground(&self) {
        let pgid = unsafe { libc::tcgetpgrp(self.pty_fd) };
        if pgid > 0 && pgid != self.shell_pid && unsafe { libc::getpgid(self.shell_pid) } != pgid {
            unsafe { libc::killpg(pgid, libc::SIGHUP) };
        }
    }

    /// The foreground program's command line and when it started (quit dialog).
    pub fn foreground_command(&self) -> Option<(String, Option<std::time::SystemTime>)> {
        let pid = self.foreground_pid();
        Some((crate::process::command(pid)?, crate::process::started(pid)))
    }

    /// Force the foreground program to stop (SIGKILL to its process group), never the shell.
    pub fn kill_foreground(&self) {
        let pgid = unsafe { libc::tcgetpgrp(self.pty_fd) };
        if pgid > 0 && pgid != self.shell_pid && unsafe { libc::getpgid(self.shell_pid) } != pgid {
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
        }
    }

    /// Pid of the foreground process (falls back to the shell).
    pub fn foreground_pid(&self) -> i32 {
        crate::process::foreground_pid(self.pty_fd).unwrap_or(self.shell_pid)
    }

    /// Working directory of the foreground process, else of the shell.
    pub fn cwd(&self) -> Option<PathBuf> {
        crate::process::cwd(self.foreground_pid()).or_else(|| crate::process::cwd(self.shell_pid))
    }

    /// Name of the foreground process ("zsh", "claude", "nvim").
    pub fn foreground_name(&self) -> Option<String> {
        if self.exited {
            return None;
        }
        crate::process::name(self.foreground_pid())
    }

    pub fn write(&self, bytes: impl Into<std::borrow::Cow<'static, [u8]>>) {
        self.notifier.notify(bytes);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size || size.cols == 0 || size.rows == 0 {
            return;
        }
        self.size = size;
        self.term.lock().resize(TermSize::new(size.cols, size.rows));
        self.notifier.on_resize(size.window_size());
    }

    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    /// Paste text, honoring bracketed paste mode.
    pub fn paste(&self, text: &str) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        // Normalize newlines to carriage returns, as terminals expect.
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        let data = if bracketed {
            let clean = body.replace('\x1b', "");
            format!("\x1b[200~{clean}\x1b[201~")
        } else {
            body
        };
        self.scroll_to_bottom();
        self.write(data.into_bytes());
    }

    /// Report focus change to programs that asked for it (DECSET 1004).
    /// Claude Code uses this to know the user is away and to write `away_summary` recaps.
    pub fn set_focused(&self, focused: bool) {
        let mut term = self.term.lock();
        term.is_focused = focused;
        let wants = term.mode().contains(TermMode::FOCUS_IN_OUT);
        drop(term);
        if wants {
            self.write(if focused { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
        }
    }

    /// Convert a viewport cell (col, row) into a grid point accounting for scrollback.
    pub fn grid_point(&self, col: usize, row: usize) -> GridPoint {
        let term = self.term.lock();
        let offset = term.grid().display_offset() as i32;
        let col = col.min(term.columns().saturating_sub(1));
        GridPoint::new(Line(row as i32 - offset), Column(col))
    }

    pub fn start_selection(&self, col: usize, row: usize, side: Side, ty: SelectionType) {
        let point = self.grid_point(col, row);
        self.term.lock().selection = Some(Selection::new(ty, point, side));
    }

    /// Shift-arrow on a selection (as in macOS text): move its moving end from `head` by a
    /// character (`dx`), a line (`dy`), a word (`word`) or to the line's edge (`edge`).
    /// Returns the new head, or None when there's no selection.
    pub fn extend_selection(&self, head: GridPoint, dx: i32, dy: i32, word: bool, edge: bool) -> Option<GridPoint> {
        use alacritty_terminal::grid::Dimensions;
        let mut term = self.term.lock();
        term.selection.as_ref()?;
        let cols = term.columns() as i32;
        let (top, bottom) = (term.topmost_line().0, term.bottommost_line().0);
        let cell = |term: &Term<Listener>, l: i32, c: i32| term.grid()[Line(l)][Column(c as usize)].c;
        let (mut line, mut col) = (head.line.0, head.column.0 as i32);
        if dy != 0 {
            line = (line + dy).clamp(top, bottom);
        } else if edge {
            col = if dx < 0 { 0 } else { cols - 1 };
        } else if word {
            // Skip spaces, then the word, in the direction of travel.
            let step = |line: &mut i32, col: &mut i32| -> bool {
                *col += dx;
                if *col < 0 {
                    if *line == top { *col = 0; return false; }
                    *line -= 1;
                    *col = cols - 1;
                } else if *col >= cols {
                    if *line == bottom { *col = cols - 1; return false; }
                    *line += 1;
                    *col = 0;
                }
                true
            };
            while cell(&term, line, col).is_whitespace() && step(&mut line, &mut col) {}
            while !cell(&term, line, col).is_whitespace() && step(&mut line, &mut col) {}
        } else {
            col += dx;
            if col < 0 && line > top {
                line -= 1;
                col = cols - 1;
            } else if col >= cols && line < bottom {
                line += 1;
                col = 0;
            }
            col = col.clamp(0, cols - 1);
        }
        let point = GridPoint::new(Line(line), Column(col as usize));
        let side = if dx < 0 || dy < 0 { Side::Left } else { Side::Right };
        if let Some(sel) = term.selection.as_mut() {
            sel.update(point, side);
        }
        term.scroll_to_point(point);
        Some(point)
    }

    pub fn update_selection(&self, col: usize, row: usize, side: Side) {
        let point = self.grid_point(col, row);
        if let Some(sel) = self.term.lock().selection.as_mut() {
            sel.update(point, side);
        }
    }

    /// Select the whole scrollback and screen.
    pub fn select_all(&self) {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Point};
        let mut term = self.term.lock();
        let start = Point::new(term.topmost_line(), Column(0));
        let end = Point::new(term.bottommost_line(), term.last_column());
        let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
        sel.update(end, Side::Right);
        term.selection = Some(sel);
    }

    /// History and screen as text with colors (SGR), up to the cursor's line and at most
    /// `max_lines` lines, for saving across restarts. Wrapped lines stay joined so the text
    /// reflows to the next window's width. Empty when the screen is an alternate one (vim,
    /// htop), whose contents mean nothing once the program is gone.
    pub fn snapshot(&self, max_lines: usize) -> Vec<u8> {
        // At a shell prompt, stop before it: the next launch prints a fresh one, and saving
        // it would stack one more prompt per restart.
        let idle = self.foreground_name().is_none_or(|n| SHELLS.contains(&n.trim_start_matches('-')));
        let term = self.term.lock();
        let cut = idle.then(|| {
            let cursor = term.grid().cursor.point.line.0;
            crate::view::prompt_starts(&*term).into_iter().filter(|&l| l <= cursor).last().unwrap_or(cursor)
        });
        snapshot_term(&term, max_lines, cut)
    }

    /// The last `n` non-blank lines of the screen (for previews), oldest first.
    pub fn last_lines(&self, n: usize) -> Vec<String> {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Line};
        let term = self.term.lock();
        let grid = term.grid();
        let cols = term.columns();
        let mut out = Vec::new();
        let mut line = term.bottommost_line().0;
        let top = term.topmost_line().0.max(-200);
        while line >= top && out.len() < n {
            let row = &grid[Line(line)];
            let text: String = (0..cols).map(|c| row[Column(c)].c).collect::<String>().replace('\0', " ");
            let text = text.trim_end().to_string();
            if !text.trim().is_empty() {
                out.push(text);
            }
            line -= 1;
        }
        out.reverse();
        out
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn shutdown(&self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// See `Terminal::snapshot`.
/// The line put under restored scrollback; never saved again.
pub const RESTORED_BANNER: &str = "── restored from the last session ──";

const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "login", "nu", "xonsh"];

fn plain(ansi: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for c in ansi.chars() {
        match (esc, c) {
            (false, '\x1b') => esc = true,
            (true, c) if c.is_ascii_alphabetic() => esc = false,
            (true, _) => {}
            (false, c) => out.push(c),
        }
    }
    out
}

/// `cut`: a line to stop before (the shell's current prompt), if any.
pub fn snapshot_term<T>(term: &Term<T>, max_lines: usize, cut: Option<i32>) -> Vec<u8> {
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::term::TermMode;
    use alacritty_terminal::vte::ansi::{Color, NamedColor};
    use std::fmt::Write;
    if term.mode().contains(TermMode::ALT_SCREEN) {
        return Vec::new();
    }
    let grid = term.grid();
    let cols = term.columns();
    // Down to the last line with text (a TUI's status line can sit below the cursor).
    let blank = |l: i32| (0..cols).all(|c| matches!(grid[Line(l)][Column(c)].c, ' ' | '\0'));
    let mut last = cut.map(|c| c - 1).unwrap_or(term.bottommost_line().0);
    if last < term.topmost_line().0 {
        return Vec::new();
    }
    while last > term.topmost_line().0 && blank(last) {
        last -= 1;
    }
    let first = term.topmost_line().0.max(last - max_lines as i32 + 1);
    let color = |c: Color, fg: bool| -> String {
        let base = if fg { 30 } else { 40 };
        match c {
            Color::Named(n) if (n as usize) < 8 => format!("{}", base + n as usize),
            Color::Named(n) if (n as usize) < 16 => format!("{}", base + 60 + n as usize - 8),
            Color::Named(_) => format!("{}", base + 9),
            Color::Indexed(i) => format!("{};5;{i}", base + 8),
            Color::Spec(rgb) => format!("{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b),
        }
    };
    let mut out = String::new();
    let mut started = false;
    let mut style = None;
    for line in first..=last {
        let row = &grid[Line(line)];
        // Skip blank lines before the first text (prompt gravity pads with them).
        let len = (0..cols).rposition(|c| {
            let cell = &row[Column(c)];
            cell.c != ' ' && cell.c != '\0' || !matches!(cell.bg, Color::Named(NamedColor::Background))
        });
        if !started && len.is_none() {
            continue;
        }
        started = true;
        let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        let end = if wrapped { cols } else { len.map_or(0, |l| l + 1) };
        for c in 0..end {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let key = (cell.fg, cell.bg, cell.flags & (Flags::BOLD | Flags::ITALIC | Flags::UNDERLINE | Flags::DIM | Flags::INVERSE | Flags::STRIKEOUT));
            if style != Some(key) {
                let mut sgr = String::from("0");
                let f = key.2;
                for (flag, code) in [(Flags::BOLD, "1"), (Flags::DIM, "2"), (Flags::ITALIC, "3"), (Flags::UNDERLINE, "4"), (Flags::INVERSE, "7"), (Flags::STRIKEOUT, "9")] {
                    if f.contains(flag) {
                        sgr.push(';');
                        sgr.push_str(code);
                    }
                }
                if !matches!(cell.fg, Color::Named(NamedColor::Foreground)) {
                    let _ = write!(sgr, ";{}", color(cell.fg, true));
                }
                if !matches!(cell.bg, Color::Named(NamedColor::Background)) {
                    let _ = write!(sgr, ";{}", color(cell.bg, false));
                }
                let _ = write!(out, "\x1b[{sgr}m");
                style = Some(key);
            }
            out.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                out.extend(extra.iter());
            }
        }
        if !wrapped {
            out.push_str("\x1b[0m\r\n");
            style = None;
        }
    }
    // Drop banners from earlier restores, and the stale prompt saved just above each one
    // by older versions, so restarting never piles them up.
    let prompt = cut.map(|l| {
        let row = &grid[Line(l)];
        (0..cols).map(|c| row[Column(c)].c).collect::<String>().replace('\0', " ").trim().to_string()
    });
    let mut kept: Vec<&str> = Vec::new();
    for line in out.split_inclusive("\r\n") {
        let text = plain(line);
        if text.trim() == RESTORED_BANNER {
            if kept.last().is_some_and(|p| prompt.as_deref().is_some_and(|q| !q.is_empty() && plain(p).trim() == q)) {
                kept.pop();
            }
            continue;
        }
        kept.push(line);
    }
    // Leading blank lines left behind by what was dropped.
    while kept.first().is_some_and(|l| plain(l).trim().is_empty()) {
        kept.remove(0);
    }
    kept.concat().into_bytes()
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term_with(bytes: &[u8], cols: usize) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &TermSize::new(cols, 10), VoidListener);
        let mut p: Processor<StdSyncHandler> = Default::default();
        p.advance(&mut t, bytes);
        t
    }

    #[test]
    fn snapshot_round_trips_text_and_color() {
        let src = term_with(b"plain\r\n\x1b[31mred\x1b[0m and \x1b[1mbold\x1b[0m\r\n$ ", 40);
        let snap = snapshot_term(&src, 100, None);
        let text = String::from_utf8(snap.clone()).unwrap();
        assert!(text.contains("plain"));
        assert!(text.contains("\x1b[0;31mred"), "{text:?}");
        assert!(text.contains("\x1b[0;1mbold"), "{text:?}");
        // Replaying the snapshot shows the same lines.
        let again = term_with(&snap, 40);
        assert_eq!(String::from_utf8(snapshot_term(&again, 100, None)).unwrap(), text);
    }

    #[test]
    fn restarts_do_not_pile_up_prompts_and_banners() {
        // What an older version saved: output, the prompt, the banner, and again.
        let old = format!("ls\r\nfile.txt\r\n$ \r\n{RESTORED_BANNER}\r\n$ \r\n{RESTORED_BANNER}\r\n$ ");
        let src = term_with(old.as_bytes(), 40);
        let cursor = src.grid().cursor.point.line.0;
        let text = String::from_utf8(snapshot_term(&src, 100, Some(cursor))).unwrap();
        let p = plain(&text);
        assert!(p.contains("file.txt"), "{p:?}");
        assert!(!p.contains("restored"), "{p:?}");
        assert!(!p.contains('$'), "{p:?}");
    }

    #[test]
    fn wrapped_lines_stay_joined() {
        let long = "x".repeat(25);
        let src = term_with(format!("{long}\r\n").as_bytes(), 10);
        let text = String::from_utf8(snapshot_term(&src, 100, None)).unwrap();
        assert!(text.contains(&long), "{text:?}");
    }
}
