//! Terminal model: owns the pty, the alacritty `Term` grid, and the reader thread.

use std::collections::HashMap;
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
}

pub struct Terminal {
    pub term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    pub size: GridSize,
    pub title: Option<String>,
    pub exited: bool,
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
            shell: None,
            working_directory: opts.working_directory,
            drain_on_exit: true,
            env,
        };
        let pty = tty::new(&pty_opts, size.window_size(), 0)?;

        let config = Config { scrolling_history: 10_000, ..Config::default() };
        let term = Term::new(config, &TermSize::new(size.cols, size.rows), listener.clone());
        let term = Arc::new(FairMutex::new(term));

        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();

        Ok((Self { term, notifier, size, title: None, exited: false }, rx))
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

    pub fn update_selection(&self, col: usize, row: usize, side: Side) {
        let point = self.grid_point(col, row);
        if let Some(sel) = self.term.lock().selection.as_mut() {
            sel.update(point, side);
        }
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
