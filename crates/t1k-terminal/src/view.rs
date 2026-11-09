//! GPUI view that renders a `Terminal` grid and routes input to it.

use std::collections::HashMap;
use std::path::PathBuf;

use alacritty_terminal::event::Event as AlacEvent;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Rgb};
use futures::StreamExt;
use gpui::{
    actions, canvas, div, fill, point, prelude::*, px, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight, Hsla, KeyDownEvent,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba,
    ScrollDelta, ScrollWheelEvent, SharedString, StrikethroughStyle, Task, TextRun,
    UnderlineStyle, Window,
};

use crate::input::{key_up, to_esc_held};
use crate::links::{find_path_candidates, find_pr_refs, find_tickets, find_urls, Link, LinkRules};
use crate::palette::{dim, Palette};
use crate::scrollbar::{Fade, Metrics, BUTTON_DELAY, IDLE};
use crate::terminal::{GridSize, SpawnOptions, Terminal};

actions!(terminal, [Copy, Paste, ClearScrollback, ScrollPageUp, ScrollPageDown, ShowLinkHints, Find, FindNext, FindPrevious, PreviousPrompt, NextPrompt, ScrollToBottom, CopyLastOutput, CopyLastCommand, SelectLastOutput]);

mod copy_ui;

/// Events the terminal view emits to its container (tab/workspace).
#[derive(Clone, Debug)]
pub enum TerminalEvent {
    TitleChanged(String),
    /// ⌘-click (or hint mode) on a link. The app opens it: browser, editor, PR or tracker.
    OpenLink(Link),
    /// Right-click: the app shows the terminal menu at `at` (window coordinates).
    ContextMenu { at: Point<Pixels>, link: Option<Link>, has_selection: bool },
    Bell,
    /// A program asked for a desktop notification (OSC 9 / OSC 777).
    Notify(crate::notify_scan::Notice),
    Exited,
}

#[derive(Clone)]
pub struct TerminalStyle {
    pub font_family: SharedString,
    /// Tried in order for glyphs the main font lacks (Powerline / Nerd Font icons, emoji).
    pub font_fallbacks: Vec<String>,
    pub font_size: Pixels,
    pub line_height: f32,
    /// Vertical and horizontal padding around the grid.
    pub padding: Pixels,
    pub padding_x: Pixels,
    pub option_as_meta: bool,
    /// Copy the selection to the clipboard as soon as the mouse is released.
    pub copy_on_select: bool,
    /// Background alpha (1.0 solid); the window behind shows through the rest.
    pub bg_alpha: f32,
    /// Whether colors programs paint behind text take `bg_alpha` too, or stay solid.
    pub cell_alpha: bool,
}

impl TerminalStyle {
    pub fn font(&self) -> Font {
        Font {
            fallbacks: Some(gpui::FontFallbacks::from_fonts(self.font_fallbacks.clone())),
            ..gpui::font(self.font_family.clone())
        }
    }
}

impl Default for TerminalStyle {
    fn default() -> Self {
        Self {
            font_family: "JetBrains Mono".into(),
            font_fallbacks: vec![
                "Symbols Nerd Font Mono".into(),
                "MesloLGS NF".into(),
                "Meslo LG L DZ for Powerline".into(),
                "Menlo".into(),
                "Apple Color Emoji".into(),
            ],
            font_size: px(13.),
            line_height: 1.45,
            padding: px(14.),
            padding_x: px(20.),
            option_as_meta: false,
            copy_on_select: false,
            bg_alpha: 1.,
            cell_alpha: false,
        }
    }
}

/// Makes the bytes for a screen of (columns, rows).
pub type FeedOnSize = Box<dyn FnOnce(usize, usize) -> Vec<u8>>;

pub struct TerminalView {
    pub terminal: Terminal,
    focus_handle: FocusHandle,
    pub palette: Palette,
    pub style: TerminalStyle,
    /// Last painted content bounds and grid geometry, used for mouse → cell mapping.
    layout: Option<(Bounds<Pixels>, GridSize)>,
    scroll_accum: f32,
    /// Cell of the last plain single click, to tell a click from a drag on release.
    press_cell: Option<(usize, usize)>,
    /// The selection's moving end, for shift-arrow (grid coordinates).
    sel_head: Option<alacritty_terminal::index::Point>,
    /// Links found in the last painted frame: viewport (col, row) -> index into `links`.
    link_map: HashMap<(usize, usize), usize>,
    links: Vec<Link>,
    /// Which plain-text references count as links (set by the app per tab).
    pub link_rules: LinkRules,
    /// Path existence checks, so file links don't stat every frame.
    path_cache: std::cell::RefCell<HashMap<PathBuf, Option<PathBuf>>>,
    /// Viewport cell under the mouse, and whether cmd is held (cmd-hover arms a link).
    hover_cell: Option<(usize, usize)>,
    cmd_down: bool,
    /// Resting on a link without ⌘: (link, since, pointer) for the "⌘ + click to open" tip.
    link_tip: Option<(usize, std::time::Instant, Point<Pixels>)>,
    link_tip_task: Option<Task<()>>,
    /// Link hint mode: the letters typed so far (labels are drawn on every visible link).
    hints: Option<String>,
    /// Find bar state while it's open (⌘F).
    search: Option<crate::search::Search>,
    /// Labels from the last frame in hint mode: (label, link index).
    hint_labels: Vec<(String, usize)>,
    /// Size the pane wants but the grid hasn't taken yet (see `request_resize`).
    pending_size: Option<GridSize>,
    resize_task: Option<Task<()>>,
    /// Output to show once the grid has its real size, made for that size (demo screens,
    /// whose full-width rules must fit the pane).
    pub feed_on_size: Option<FeedOnSize>,
    scroll_ui: ScrollUi,
    copy_ui: copy_ui::CopyUi,
    _events: Task<()>,
}

/// The overlay scrollbar and the scroll-to-bottom button (see `scrollbar`).
struct ScrollUi {
    bar: Fade,
    /// The wider hover track, while the pointer is near the right edge.
    wide: Fade,
    button: Fade,
    near_edge: bool,
    /// Last scroll or edge hover: the scrollbar fades out `IDLE` after it.
    last_active: std::time::Instant,
    hide_armed: bool,
    hide_task: Option<Task<()>>,
    button_task: Option<Task<()>>,
    /// A thumb drag: how far below the thumb's top it was grabbed.
    drag: Option<f32>,
    /// Display offset at the last sync. Output while scrolled up raises the real one (alacritty
    /// keeps the view still), and that difference is the button's "new lines".
    seen_offset: usize,
    new_lines: usize,
    /// Prompt starts for the hover track's ticks; None after new output.
    prompts: Option<Vec<i32>>,
    metrics: Metrics,
    /// Main screen with some history: the only time either one shows.
    eligible: bool,
    /// T1K_DEBUG_SCROLL, until it's applied; then everything stays shown (`pinned`).
    debug: Option<DebugScroll>,
    pinned: bool,
}

struct DebugScroll {
    up: usize,
    new: usize,
    hover: bool,
    /// The shell's startup output has to land first, or the view scrolls into the blank lines
    /// that bottom gravity puts in history.
    after: std::time::Instant,
}

/// How long T1K_DEBUG_SCROLL waits for the shell's startup output.
const DEBUG_SCROLL_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

impl ScrollUi {
    fn new() -> Self {
        // Debug aid for screenshots: `T1K_DEBUG_SCROLL=up:300,new:128,hover` starts scrolled up
        // 300 lines (once there is that much history) with "128 new lines" on the button.
        let debug = std::env::var("T1K_DEBUG_SCROLL").ok().map(|v| {
            let mut d = DebugScroll { up: 200, new: 0, hover: false, after: std::time::Instant::now() + DEBUG_SCROLL_WAIT };
            for part in v.split(',').map(str::trim) {
                match part.split_once(':') {
                    Some(("up", n)) => d.up = n.parse().unwrap_or(d.up),
                    Some(("new", n)) => d.new = n.parse().unwrap_or(0),
                    _ if part == "hover" => d.hover = true,
                    _ => {}
                }
            }
            d
        });
        Self {
            bar: Fade::new(100., 300.),
            wide: Fade::new(100., 100.),
            button: Fade::new(150., 150.),
            near_edge: false,
            last_active: std::time::Instant::now(),
            hide_armed: false,
            hide_task: None,
            button_task: None,
            drag: None,
            seen_offset: 0,
            new_lines: 0,
            prompts: None,
            metrics: Metrics::default(),
            eligible: false,
            debug,
            pinned: false,
        }
    }
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn hsla(c: Rgb) -> Hsla {
    Rgba { r: c.r as f32 / 255., g: c.g as f32 / 255., b: c.b as f32 / 255., a: 1. }.into()
}

impl TerminalView {
    pub fn new(opts: SpawnOptions, window: &mut Window, cx: &mut Context<Self>) -> anyhow::Result<Self> {
        let (terminal, mut events) = Terminal::spawn(opts)?;
        let focus_handle = cx.focus_handle();

        let task = cx.spawn(async move |this, cx| {
            while let Some(ev) = events.next().await {
                // Drain whatever else is queued so a burst of output is one repaint.
                let mut batch = vec![ev];
                while let Ok(more) = events.try_recv() {
                    batch.push(more);
                }
                let alive = this
                    .update(cx, |view, cx| {
                        for ev in batch {
                            view.handle_event(ev, cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });

        // The workspace caches this view's last frame, so focus changes (hollow vs solid cursor)
        // must ask for a repaint.
        cx.on_focus(&focus_handle, window, |this, window, cx| {
            this.terminal.set_focused(window.is_window_active());
            cx.notify();
        })
        .detach();
        cx.on_blur(&focus_handle, window, |this, _, cx| {
            this.terminal.set_focused(false);
            cx.notify();
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            let focused = window.is_window_active() && this.focus_handle.is_focused(window);
            this.terminal.set_focused(focused);
            cx.notify();
        })
        .detach();

        if std::env::var_os("T1K_DEBUG_SCROLL").is_some() {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DEBUG_SCROLL_WAIT).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }

        Self::debug_copy(cx);
        Ok(Self {
            terminal,
            focus_handle,
            palette: Palette::default(),
            style: TerminalStyle::default(),
            layout: None,
            scroll_accum: 0.,
            press_cell: None,
            sel_head: None,
            link_map: HashMap::new(),
            links: Vec::new(),
            link_rules: LinkRules { files: true, prs: false, tickets: false },
            path_cache: Default::default(),
            hover_cell: None,
            cmd_down: false,
            link_tip: None,
            link_tip_task: None,
            // Debug aid for screenshots without a keyboard: start in hint mode.
            hints: std::env::var_os("T1K_DEBUG_HINTS").map(|_| String::new()),
            hint_labels: Vec::new(),
            // Debug aid for screenshots: open the find bar with this query.
            search: std::env::var("T1K_DEBUG_FIND").ok().map(|q| crate::search::Search { query: q, dirty: true, ..Default::default() }),
            pending_size: None,
            resize_task: None,
            feed_on_size: None,
            copy_ui: Default::default(),
            scroll_ui: ScrollUi::new(),
            _events: task,
        })
    }

    /// Resize the grid once the pane size has settled.
    ///
    /// Every resize reflows the grid and makes full-screen programs redraw. Claude Code, for one,
    /// erases the visible rows and repaints its header; whatever the reflow pushed into
    /// scrollback stays there as a stale copy. Resizing on every frame of a window drag or panel
    /// toggle left a stack of copies, so intermediate sizes are skipped: the grid keeps its old
    /// size (clipped) until the pane has held one size for `SETTLE`.
    fn request_resize(&mut self, g: GridSize, cx: &mut Context<Self>) {
        const SETTLE: std::time::Duration = std::time::Duration::from_millis(120);
        if g == self.terminal.size {
            self.pending_size = None;
            self.resize_task = None;
            return;
        }
        let cell_changed = g.cell_width != self.terminal.size.cell_width || g.cell_height != self.terminal.size.cell_height;
        // First layout and font changes apply at once; they aren't part of a drag.
        if self.layout.is_none() || cell_changed {
            self.pending_size = None;
            self.resize_task = None;
            self.terminal.resize(g);
            if let Some(f) = self.feed_on_size.take() {
                self.terminal.feed(&f(g.cols, g.rows));
                cx.notify();
            }
            self.sync_offset(true);
            return;
        }
        if self.pending_size == Some(g) {
            return;
        }
        self.pending_size = Some(g);
        self.resize_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let _ = this.update(cx, |view, cx| {
                if view.pending_size == Some(g) {
                    view.pending_size = None;
                    view.terminal.resize(g);
                    view.sync_offset(true);
                    cx.notify();
                }
            });
        }));
    }

    fn handle_event(&mut self, ev: AlacEvent, cx: &mut Context<Self>) {
        match ev {
            AlacEvent::Wakeup => {
                for n in self.terminal.take_notices() {
                    cx.emit(TerminalEvent::Notify(n));
                }
                if let Some(s) = self.search.as_mut() {
                    s.dirty = true;
                }
                self.scroll_ui.prompts = None;
                self.sync_offset(false);
                cx.notify();
            }
            AlacEvent::MouseCursorDirty | AlacEvent::CursorBlinkingChange => cx.notify(),
            AlacEvent::Title(t) => {
                self.terminal.title = Some(t.clone());
                cx.emit(TerminalEvent::TitleChanged(t));
            }
            AlacEvent::ResetTitle => {
                self.terminal.title = None;
                cx.emit(TerminalEvent::TitleChanged(String::new()));
            }
            AlacEvent::PtyWrite(s) => self.terminal.write(s.into_bytes()),
            AlacEvent::ClipboardStore(_, text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            AlacEvent::ClipboardLoad(_, fmt) => {
                let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                self.terminal.write(fmt(&text).into_bytes());
            }
            AlacEvent::ColorRequest(idx, fmt) => {
                let term = self.terminal.term.lock();
                let rgb = term.colors()[idx].unwrap_or_else(|| match idx {
                    256 => self.palette.foreground,
                    257 => self.palette.background,
                    258 => self.palette.cursor,
                    i if i < 256 => self.palette.indexed(i as u8),
                    _ => self.palette.foreground,
                });
                drop(term);
                self.terminal.write(fmt(rgb).into_bytes());
            }
            AlacEvent::TextAreaSizeRequest(fmt) => {
                let s = self.terminal.size;
                let ws = alacritty_terminal::event::WindowSize {
                    num_lines: s.rows as u16,
                    num_cols: s.cols as u16,
                    cell_width: s.cell_width as u16,
                    cell_height: s.cell_height as u16,
                };
                self.terminal.write(fmt(ws).into_bytes());
            }
            AlacEvent::Bell => cx.emit(TerminalEvent::Bell),
            AlacEvent::ChildExit(_) | AlacEvent::Exit => {
                if !self.terminal.exited {
                    self.terminal.exited = true;
                    cx.emit(TerminalEvent::Exited);
                }
            }
        }
    }

    // ---- input ---------------------------------------------------------------------------

    fn show_link_hints(&mut self, _: &ShowLinkHints, _: &mut Window, cx: &mut Context<Self>) {
        self.hints = if self.hints.is_some() { None } else { Some(String::new()) };
        cx.notify();
    }

    /// Hint mode keys: letters narrow to a label and open it, ⌫ steps back, anything else exits.
    fn hint_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let typed = self.hints.get_or_insert_with(String::new);
        let plain = !k.modifiers.platform && !k.modifiers.control && !k.modifiers.alt;
        match k.key.as_str() {
            "backspace" if plain => {
                typed.pop();
            }
            key if plain && key.len() == 1 && key.chars().all(|c| c.is_ascii_lowercase()) => {
                typed.push_str(key);
                let typed = typed.clone();
                if let Some((_, id)) = self.hint_labels.iter().find(|(l, _)| *l == typed) {
                    if let Some(link) = self.links.get(*id).cloned() {
                        cx.emit(TerminalEvent::OpenLink(link));
                    }
                    self.hints = None;
                } else if !self.hint_labels.iter().any(|(l, _)| l.starts_with(&typed)) {
                    self.hints = None;
                }
            }
            _ => self.hints = None,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn find(&mut self, _: &Find, _: &mut Window, cx: &mut Context<Self>) {
        // ⌘F with a selection searches for it.
        let seed = self.terminal.selection_text().filter(|t| !t.contains('\n') && t.len() < 200);
        let s = self.search.get_or_insert_with(Default::default);
        if let Some(q) = seed {
            s.query = q;
            s.current = None;
        }
        s.dirty = true;
        cx.notify();
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.search_step(true, cx);
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.search_step(false, cx);
    }

    /// Next is towards newer output (down), previous towards older (up).
    fn search_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(s) = self.search.as_mut() else { return };
        if s.dirty {
            s.scan(&self.terminal.term.lock());
        }
        if let Some(m) = s.step(forward).cloned() {
            self.terminal.term.lock().scroll_to_point(*m.start());
        }
        self.scrolled(cx);
    }

    /// Open the find bar on `query` and go to its newest match (used to show where an output
    /// appeared).
    pub fn find_text(&mut self, query: &str, cx: &mut Context<Self>) {
        let mut s = crate::search::Search { query: query.to_string(), ..Default::default() };
        s.scan(&self.terminal.term.lock());
        if let Some(m) = s.current.and_then(|i| s.matches.get(i)).cloned() {
            self.terminal.term.lock().scroll_to_point(*m.start());
        }
        self.search = Some(s);
        self.scrolled(cx);
    }

    fn previous_prompt(&mut self, _: &PreviousPrompt, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_prompt(false, cx);
    }

    fn next_prompt(&mut self, _: &NextPrompt, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_prompt(true, cx);
    }

    /// Scroll so the previous / next prompt (marked by shell integration) is the top line.
    fn jump_prompt(&mut self, down: bool, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        let offset = term.grid().display_offset() as i32;
        let top = -offset;
        let starts = prompt_starts(&term);
        let target = if down {
            starts.iter().copied().find(|&l| l > top)
        } else {
            starts.iter().copied().rev().find(|&l| l < top)
        };
        match target {
            // Put the prompt line at the top of the view (clamped by the scrollback).
            Some(line) => term.scroll_display(alacritty_terminal::grid::Scroll::Delta(-line - offset)),
            None if down => term.scroll_display(alacritty_terminal::grid::Scroll::Bottom),
            None => {}
        }
        drop(term);
        self.scrolled(cx);
    }

    fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search = None;
        cx.notify();
    }

    /// Find bar keys: typing edits the query, Enter / ⇧Enter step, Esc closes.
    fn search_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let k = &ev.keystroke;
        if k.modifiers.platform || k.modifiers.control {
            return false;
        }
        let Some(s) = self.search.as_mut() else { return false };
        match k.key.as_str() {
            "escape" => {
                self.close_search(cx);
                return true;
            }
            "enter" => {
                self.search_step(!k.modifiers.shift, cx);
                return true;
            }
            "backspace" => {
                if k.modifiers.alt {
                    s.query.clear();
                } else {
                    s.query.pop();
                }
            }
            _ => match k.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                Some(c) => s.query.push_str(c),
                None => return false,
            },
        }
        s.current = None;
        s.scan(&self.terminal.term.lock());
        if let Some(m) = s.current.and_then(|i| s.matches.get(i)).cloned() {
            self.terminal.term.lock().scroll_to_point(*m.start());
        }
        cx.stop_propagation();
        self.scrolled(cx);
        true
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() && self.search_key(ev, cx) {
            cx.stop_propagation();
            return;
        }
        if self.hints.is_some() {
            self.hint_key(ev, cx);
            return;
        }
        // Shift-arrows change a selection, as in any macOS text (⌥ by word, ⌘ to the edge).
        let m = &ev.keystroke.modifiers;
        if m.shift && !m.control && self.terminal.selection_text().is_some() {
            let (dx, dy) = match ev.keystroke.key.as_str() {
                "left" => (-1, 0),
                "right" => (1, 0),
                "up" => (0, -1),
                "down" => (0, 1),
                _ => (0, 0),
            };
            if (dx, dy) != (0, 0) {
                if let Some(head) = self.sel_head {
                    self.sel_head = self.terminal.extend_selection(head, dx, dy, m.alt, m.platform);
                    self.sync_offset(true);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
            }
        }
        let mode = self.terminal.mode();
        if let Some(bytes) = to_esc_held(&ev.keystroke, mode, self.style.option_as_meta, ev.is_held) {
            self.terminal.clear_selection();
            self.terminal.scroll_to_bottom();
            self.sync_offset(true);
            self.terminal.write(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Releases, only for programs that asked for them (kitty keyboard protocol, flag 2).
    fn on_key_up(&mut self, ev: &gpui::KeyUpEvent, _: &mut Window, _: &mut Context<Self>) {
        if let Some(bytes) = key_up(&ev.keystroke, self.terminal.mode(), self.style.option_as_meta) {
            self.terminal.write(bytes);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.terminal.paste(&text);
            self.sync_offset(true);
            cx.notify();
        }
    }

    fn clear_scrollback(&mut self, _: &ClearScrollback, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.term.lock().grid_mut().clear_history();
        // Ask the shell to redraw its prompt on a clean screen (ctrl-l).
        self.terminal.write(&b"\x0c"[..]);
        cx.notify();
    }

    fn page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.terminal.size.rows as i32;
        self.terminal.scroll(rows);
        self.scrolled(cx);
    }

    fn page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.terminal.size.rows as i32;
        self.terminal.scroll(-rows);
        self.scrolled(cx);
    }

    /// Map a window position to (col, row, side) inside the grid.
    fn cell_at(&self, pos: Point<Pixels>) -> Option<(usize, usize, Side)> {
        let (bounds, g) = self.layout?;
        let x = f32::from(pos.x - bounds.origin.x).max(0.);
        let y = f32::from(pos.y - bounds.origin.y).max(0.);
        let col_f = x / g.cell_width;
        let col = (col_f as usize).min(g.cols.saturating_sub(1));
        let row = ((y / g.cell_height) as usize).min(g.rows.saturating_sub(1));
        let side = if col_f.fract() < 0.5 { Side::Left } else { Side::Right };
        Some((col, row, side))
    }

    fn mouse_reporting(&self, shift: bool) -> bool {
        !shift && self.terminal.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn report_mouse(&self, button: u8, col: usize, row: usize, pressed: bool) {
        let mode = self.terminal.mode();
        if mode.contains(TermMode::SGR_MOUSE) {
            let c = if pressed { 'M' } else { 'm' };
            self.terminal.write(format!("\x1b[<{button};{};{}{c}", col + 1, row + 1).into_bytes());
        } else if col < 223 && row < 223 {
            let b = if pressed { button } else { 3 };
            self.terminal.write(vec![0x1b, b'[', b'M', 32 + b, 33 + col as u8, 33 + row as u8]);
        }
    }

    /// Right-click: programs that take the mouse get the click; otherwise the app shows the
    /// terminal menu (copy, paste, link actions, find, split...).
    fn on_right_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let cell = self.cell_at(ev.position);
        if let Some((col, row, _)) = cell {
            if self.mouse_reporting(ev.modifiers.shift) {
                self.report_mouse(2, col, row, true);
                self.report_mouse(2, col, row, false);
                return;
            }
        }
        let link = cell.and_then(|(c, r, _)| self.link_at(c, r));
        let has_selection = self.terminal.selection_text().is_some();
        cx.emit(TerminalEvent::ContextMenu { at: ev.position, link, has_selection });
        cx.stop_propagation();
    }

    pub fn copy_selection(&self, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn paste_clipboard(&self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.terminal.paste(&text);
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.terminal.select_all();
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.terminal.term.lock().grid_mut().clear_history();
        self.terminal.write(&b"\x0c"[..]);
        cx.notify();
    }

    pub fn open_find(&mut self, cx: &mut Context<Self>) {
        self.search.get_or_insert_with(Default::default).dirty = true;
        cx.notify();
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.hints.take().is_some() {
            cx.notify();
        }
        window.focus(&self.focus_handle);
        if self.scrollbar_mouse_down(ev.position, cx) {
            cx.stop_propagation();
            return;
        }
        let Some((col, row, side)) = self.cell_at(ev.position) else { return };
        if ev.modifiers.platform && !ev.modifiers.alt {
            if let Some(link) = self.link_at(col, row) {
                cx.emit(TerminalEvent::OpenLink(link));
                return;
            }
        }
        if self.mouse_reporting(ev.modifiers.shift) {
            self.report_mouse(0, col, row, true);
            return;
        }
        let ty = match ev.click_count {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            // ⌥-drag (Terminal, Ghostty) or ⌘⌥-drag (iTerm2) selects a rectangle: columns.
            _ if ev.modifiers.alt => SelectionType::Block,
            _ => SelectionType::Simple,
        };
        if ev.click_count == 1 {
            self.terminal.clear_selection();
        }
        self.press_cell = (ev.click_count == 1 && ev.modifiers.number_of_modifiers() == 0).then_some((col, row));
        self.terminal.start_selection(col, row, side, ty);
        self.sel_head = Some(self.terminal.grid_point(col, row));
        cx.notify();
    }

    /// A plain click on a link does nothing (it selects, like any text), so resting on one
    /// without ⌘ says how to open it.
    fn update_link_tip(&mut self, cell: Option<(usize, usize)>, cmd: bool, at: Point<Pixels>, cx: &mut Context<Self>) {
        const DELAY: std::time::Duration = std::time::Duration::from_millis(600);
        let link = (!cmd).then(|| cell.and_then(|c| self.link_map.get(&c).copied())).flatten().filter(|&i| matches!(self.links.get(i), Some(Link::Url(_))));
        match (link, self.link_tip) {
            (Some(i), Some((j, _, _))) if i == j => {}
            (Some(i), _) => {
                let shown = self.link_tip.is_some_and(|(_, t, _)| t.elapsed() >= DELAY);
                self.link_tip = Some((i, std::time::Instant::now(), at));
                self.link_tip_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(DELAY).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                }));
                if shown {
                    cx.notify();
                }
            }
            (None, Some((_, t, _))) => {
                self.link_tip = None;
                self.link_tip_task = None;
                if t.elapsed() >= DELAY {
                    cx.notify();
                }
            }
            (None, None) => {}
        }
    }

    /// The pointer rests on a web link (shown underlined without ⌘): it gets a hand cursor.
    fn on_url(&self) -> bool {
        self.hover_cell.and_then(|c| self.link_map.get(&c)).is_some_and(|&i| matches!(self.links.get(i), Some(Link::Url(_))))
    }

    fn link_at(&self, col: usize, row: usize) -> Option<Link> {
        self.link_map.get(&(col, row)).map(|&i| self.links[i].clone())
    }

    fn set_hover(&mut self, cell: Option<(usize, usize)>, cmd: bool, cx: &mut Context<Self>) {
        let was_cmd = self.cmd_down;
        let armed_before = self.cmd_down && self.hover_cell.is_some_and(|c| self.link_map.contains_key(&c));
        let url_before = self.on_url();
        self.hover_cell = cell;
        self.cmd_down = cmd;
        let armed_now = cmd && cell.is_some_and(|c| self.link_map.contains_key(&c));
        // ⌘ reveals file, PR and ticket links anywhere on screen, so a change always repaints;
        // moving onto or off a web link repaints for the hand cursor.
        if armed_before || armed_now || cmd != was_cmd || url_before != self.on_url() {
            cx.notify();
        }
    }

    fn on_modifiers_changed(&mut self, ev: &ModifiersChangedEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.set_hover(self.hover_cell, ev.modifiers.platform, cx);
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_ui.drag.is_some() {
            return;
        }
        if ev.pressed_button.is_none() {
            let near = self.pane_bounds().is_some_and(|p| crate::scrollbar::near_edge(p, ev.position));
            self.set_near_edge(near, cx);
        }
        let cell = self.cell_at(ev.position).map(|(c, r, _)| (c, r));
        self.copy_hover(cell.map(|c| c.1), ev.pressed_button.is_some(), cx);
        self.set_hover(cell, ev.modifiers.platform, cx);
        self.update_link_tip(cell, ev.modifiers.platform, ev.position, cx);
        if ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some((col, row, side)) = self.cell_at(ev.position) else { return };
        if self.mouse_reporting(ev.modifiers.shift) {
            if self.terminal.mode().intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                self.report_mouse(32, col, row, true);
            }
            return;
        }
        self.terminal.update_selection(col, row, side);
        self.sel_head = Some(self.terminal.grid_point(col, row));
        cx.notify();
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_ui.drag.is_some() {
            self.end_drag(ev.position, cx);
            return;
        }
        let Some((col, row, _)) = self.cell_at(ev.position) else { return };
        if self.mouse_reporting(ev.modifiers.shift) {
            self.report_mouse(0, col, row, false);
            return;
        }
        // A plain click (no drag) on the line being typed moves the cursor there.
        if self.press_cell.take() == Some((col, row)) && self.terminal.selection_text().is_none_or(|t| t.is_empty()) {
            self.click_to_cursor(col, row);
        }
        match self.terminal.selection_text() {
            None => self.terminal.clear_selection(),
            Some(text) if self.style.copy_on_select && !text.is_empty() => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            Some(_) => {}
        }
        cx.notify();
    }

    /// Send ← or → so the cursor lands on the clicked cell, when the click is on the cursor's
    /// line or a line soft-wrapped into it (a long command), the view isn't scrolled back,
    /// and no full-screen program owns the screen. Shells and Claude Code's input both move
    /// one character per arrow.
    fn click_to_cursor(&self, col: usize, row: usize) {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::cell::Flags;
        let (delta, app_cursor) = {
            let term = self.terminal.term.lock();
            let mode = *term.mode();
            if mode.contains(TermMode::ALT_SCREEN) || term.grid().display_offset() != 0 {
                return;
            }
            let cols = term.columns() as i64;
            let cur = term.grid().cursor.point;
            let (click, at) = (row as i32, cur.line.0);
            let wraps = |l: i32| term.grid()[Line(l)][Column(cols as usize - 1)].flags.contains(Flags::WRAPLINE);
            // Every line between the two must continue into the next.
            let (lo, hi) = if click <= at { (click, at) } else { (at, click) };
            if !(lo..hi).all(wraps) {
                return;
            }
            let delta = (click - at) as i64 * cols + col as i64 - cur.column.0 as i64;
            (delta, mode.contains(TermMode::APP_CURSOR))
        };
        if delta == 0 || delta.unsigned_abs() > 2000 {
            return;
        }
        let key: &[u8] = match (delta > 0, app_cursor) {
            (true, true) => b"\x1bOC",
            (true, false) => b"\x1b[C",
            (false, true) => b"\x1bOD",
            (false, false) => b"\x1b[D",
        };
        let bytes: Vec<u8> = key.iter().copied().cycle().take(key.len() * delta.unsigned_abs() as usize).collect();
        self.terminal.write(bytes);
    }

    // ---- scrollbar and scroll-to-bottom button ----------------------------------------------

    /// The whole pane (grid plus padding), where the overlays are placed.
    fn pane_bounds(&self) -> Option<Bounds<Pixels>> {
        let (b, _) = self.layout?;
        let (pad_x, pad_y) = (self.style.padding_x, self.style.padding);
        Some(Bounds::new(point(b.origin.x - pad_x, b.origin.y - pad_y), size(b.size.width + pad_x * 2., b.size.height + pad_y * 2.)))
    }

    /// Note the display offset. `user`: the change was a scroll; otherwise a rise is output
    /// arriving while scrolled up (alacritty keeps the view still) and counts as new lines.
    fn sync_offset(&mut self, user: bool) {
        let off = self.terminal.term.lock().grid().display_offset();
        let s = &mut self.scroll_ui;
        if !user && s.seen_offset > 0 && off > s.seen_offset {
            s.new_lines += off - s.seen_offset;
        }
        s.seen_offset = off;
        if off == 0 {
            s.new_lines = 0;
        }
    }

    /// The user scrolled: show the scrollbar and restart its idle timer.
    fn scrolled(&mut self, cx: &mut Context<Self>) {
        self.sync_offset(true);
        self.scroll_ui.last_active = std::time::Instant::now();
        self.update_bar(cx);
        cx.notify();
    }

    /// Show or hide the scrollbar for the current state, and arm the idle timer while it shows
    /// only because of a recent scroll.
    fn update_bar(&mut self, cx: &mut Context<Self>) {
        let (metrics, eligible) = {
            let term = self.terminal.term.lock();
            let grid = term.grid();
            // Rows that bottom gravity hides aren't content: leave them out, so the thumb sits
            // where the view is (and at the bottom when the view is).
            let (hidden, shift) = if self.terminal.bottom_gravity { (crate::terminal::gravity_hidden(&term), crate::terminal::gravity_shift(&term)) } else { (0, 0) };
            let m = Metrics { history: grid.history_size() - hidden, rows: term.screen_lines(), offset: (grid.display_offset() + shift).saturating_sub(hidden) };
            (m, !term.mode().contains(TermMode::ALT_SCREEN) && m.history > 0)
        };
        let finding = self.search.as_ref().is_some_and(|f| !f.matches.is_empty());
        let s = &mut self.scroll_ui;
        s.metrics = metrics;
        s.eligible = eligible;
        if !eligible {
            // A full-screen program took over: nothing to scroll, nothing to fade.
            s.bar.snap(false);
            s.wide.snap(false);
            s.drag = None;
            return;
        }
        let held = s.near_edge || s.drag.is_some() || finding || s.pinned;
        let recent = s.last_active.elapsed() < IDLE;
        if s.bar.set(held || recent) {
            cx.notify();
        }
        if recent && !held && !s.hide_armed {
            s.hide_armed = true;
            s.hide_task = Some(cx.spawn(async move |this, cx| {
                let mut wait = IDLE;
                loop {
                    cx.background_executor().timer(wait).await;
                    let Ok(left) = this.update(cx, |v, _| IDLE.saturating_sub(v.scroll_ui.last_active.elapsed())) else { return };
                    if left.is_zero() {
                        break;
                    }
                    wait = left;
                }
                let _ = this.update(cx, |v, cx| {
                    v.scroll_ui.hide_armed = false;
                    v.update_bar(cx);
                    cx.notify();
                });
            }));
        }
    }

    fn set_near_edge(&mut self, near: bool, cx: &mut Context<Self>) {
        let s = &mut self.scroll_ui;
        let near = near && s.eligible;
        if s.near_edge == near || s.pinned {
            return;
        }
        s.near_edge = near;
        s.wide.set(near);
        // Leaving the edge starts the idle countdown.
        s.last_active = std::time::Instant::now();
        self.update_bar(cx);
        cx.notify();
    }

    /// A press on the scrollbar: grab the thumb, or page toward the press. Returns whether the
    /// scrollbar took it.
    fn scrollbar_mouse_down(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        use crate::scrollbar::{hit, Hit};
        let s = &self.scroll_ui;
        if !s.eligible || !(s.near_edge || s.bar.on()) {
            return false;
        }
        let Some(pane) = self.pane_bounds() else { return false };
        let Some(h) = hit(pane, s.metrics, pos) else { return false };
        let rows = self.terminal.size.rows as i32;
        match h {
            Hit::Thumb(grab) => {
                self.scroll_ui.drag = Some(grab);
                self.scroll_ui.last_active = std::time::Instant::now();
                self.update_bar(cx);
                cx.notify();
            }
            Hit::Above => {
                self.terminal.scroll(rows);
                self.scrolled(cx);
            }
            Hit::Below => {
                self.terminal.scroll(-rows);
                self.scrolled(cx);
            }
        }
        true
    }

    fn drag_to(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let (Some(grab), Some(pane)) = (self.scroll_ui.drag, self.pane_bounds()) else { return };
        let target = crate::scrollbar::drag_offset(pane, self.scroll_ui.metrics, grab, y) as i32;
        let current = self.terminal.term.lock().grid().display_offset() as i32;
        if target != current {
            self.terminal.scroll(target - current);
            self.scrolled(cx);
        }
    }

    fn end_drag(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        if self.scroll_ui.drag.take().is_none() {
            return;
        }
        // Released away from the edge: the track narrows and the idle countdown starts.
        let near = self.pane_bounds().is_some_and(|p| crate::scrollbar::near_edge(p, pos));
        self.scroll_ui.near_edge = !near;
        self.set_near_edge(near, cx);
        cx.notify();
    }

    fn scroll_to_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_to_bottom(cx);
    }

    /// The button and ⌘⇧↓: back to the newest output, at once.
    fn jump_to_bottom(&mut self, cx: &mut Context<Self>) {
        let was_up = self.terminal.term.lock().grid().display_offset() > 0;
        self.terminal.scroll_to_bottom();
        if was_up {
            self.scrolled(cx);
        }
    }

    /// Per-frame upkeep: count new lines, apply the debug state, show or hide the button.
    fn sync_scroll_ui(&mut self, cx: &mut Context<Self>) {
        self.sync_offset(false);
        if let Some(d) = self.scroll_ui.debug.as_ref() {
            let history = self.terminal.term.lock().grid().history_size();
            if history >= d.up && std::time::Instant::now() >= d.after {
                let (up, new, hover) = (d.up, d.new, d.hover);
                self.scroll_ui.debug = None;
                self.terminal.scroll(up as i32);
                self.sync_offset(true);
                let s = &mut self.scroll_ui;
                s.new_lines = new;
                s.pinned = true;
                s.near_edge = hover;
                s.wide.snap(hover);
                s.bar.snap(true);
                s.button.snap(true);
            }
        }
        self.update_bar(cx);
        let s = &mut self.scroll_ui;
        if s.metrics.offset == 0 || !s.eligible {
            s.button.set(false);
            s.button_task = None;
            s.pinned = false;
        } else if !s.button.on() && s.button_task.is_none() {
            s.button_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(BUTTON_DELAY).await;
                let _ = this.update(cx, |v, cx| {
                    if v.terminal.term.lock().grid().display_offset() > 0 {
                        v.scroll_ui.button.set(true);
                        cx.notify();
                    }
                });
            }));
        }
    }

    /// The scrollbar's frame, or None while it's invisible.
    fn scrollbar_paint(&mut self) -> Option<crate::scrollbar::Paint> {
        let opacity = self.scroll_ui.bar.value();
        if !self.scroll_ui.eligible || opacity <= 0. {
            return None;
        }
        let wide = self.scroll_ui.wide.value();
        // Prompt ticks only show on the hover track, so the scan waits until it's there.
        let prompts = if wide > 0. {
            let term = &self.terminal.term;
            self.scroll_ui.prompts.get_or_insert_with(|| prompt_starts(&term.lock())).clone()
        } else {
            Vec::new()
        };
        let matches = self
            .search
            .as_ref()
            .map(|f| f.matches.iter().enumerate().map(|(i, m)| (m.start().line.0, f.current == Some(i))).collect())
            .unwrap_or_default();
        let s = &self.scroll_ui;
        Some(crate::scrollbar::Paint {
            metrics: s.metrics,
            opacity,
            wide,
            dragging: s.drag.is_some(),
            fg: hsla(self.palette.foreground),
            accent: hsla(self.palette.link),
            prompts,
            matches,
        })
    }

    /// Distance of the button's bottom from the pane's bottom edge.
    fn button_bottom(&self) -> f32 {
        let pad = f32::from(self.style.padding);
        let mut bottom = pad + 16.;
        let Some(pane) = self.pane_bounds() else { return bottom };
        // Bottom-center belongs to the drop label and the resume pill; in a narrow pane they
        // reach the corner, so the button steps up instead.
        if f32::from(pane.size.width) < 360. {
            bottom += 40.;
        }
        // Never cover the line being typed when it's on screen.
        let term = self.terminal.term.lock();
        let row = term.grid().cursor.point.line.0 + term.grid().display_offset() as i32;
        if row >= 0 && (row as usize) < term.screen_lines() {
            let ch = self.terminal.size.cell_height;
            let pane_h = f32::from(pane.size.height);
            let line_top = pad + row as f32 * ch;
            let button_top = pane_h - bottom - 30.;
            if line_top < pane_h - bottom && line_top + ch > button_top {
                bottom = pane_h - line_top + 8.;
            }
        }
        bottom
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_h = self.terminal.size.cell_height.max(1.);
        let dy = match ev.delta {
            ScrollDelta::Pixels(p) => f32::from(p.y) / line_h,
            ScrollDelta::Lines(l) => l.y,
        };
        self.scroll_accum += dy;
        let lines = self.scroll_accum.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_accum -= lines as f32;

        let mode = self.terminal.mode();
        if mode.intersects(TermMode::MOUSE_MODE) {
            if let Some((col, row, _)) = self.cell_at(ev.position) {
                let button = if lines > 0 { 64 } else { 65 };
                for _ in 0..lines.unsigned_abs() {
                    self.report_mouse(button, col, row, true);
                }
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let bytes: Vec<u8> = key.iter().copied().cycle().take(key.len() * lines.unsigned_abs() as usize).collect();
            self.terminal.write(bytes);
        } else {
            self.terminal.scroll(lines);
            self.scrolled(cx);
        }
    }
}

// ---- rendering ---------------------------------------------------------------------------

struct TextBatch {
    col: usize,
    row: usize,
    /// Cells covered (tracked so joining doesn't recount chars).
    cells: usize,
    /// Wide or combining glyphs are shaped on their own so they can't shift neighbours.
    solo: bool,
    text: String,
    fg: Hsla,
    flags: Flags,
    underline_color: Option<Hsla>,
    link: Option<usize>,
}

struct BgRect {
    col: usize,
    row: usize,
    cols: usize,
    color: Hsla,
}

struct Frame {
    bg_rects: Vec<BgRect>,
    /// Find matches, painted over cell backgrounds.
    highlights: Vec<BgRect>,
    batches: Vec<TextBatch>,
    cursor: Option<(usize, usize, CursorShape, bool)>,
    /// Block and box-drawing cells painted as shapes (see `glyphs`), not font glyphs.
    drawn: Vec<(usize, usize, char, Hsla)>,
    links: Vec<Link>,
    link_map: HashMap<(usize, usize), usize>,
    /// URLs are always marked; other links only while ⌘ is held.
    cmd_down: bool,
    /// Link under the mouse while cmd is held.
    hovered: Option<usize>,
    /// Hint mode: typed prefix and (label, link id, first cell) per link.
    hints: Option<(String, Vec<HintLabel>)>,
}

/// A hint-mode label: (label, link id, first cell as (col, row)).
type HintLabel = (String, usize, (usize, usize));

/// Hint labels in reading order: one letter each while they fit, else two.
fn hint_labels(link_map: &HashMap<(usize, usize), usize>) -> Vec<(String, usize, (usize, usize))> {
    const KEYS: &[u8] = b"asdfjklghqweruioptyzxcvbnm";
    let mut first: HashMap<usize, (usize, usize)> = HashMap::new();
    for (&(c, r), &id) in link_map {
        let e = first.entry(id).or_insert((c, r));
        if (r, c) < (e.1, e.0) {
            *e = (c, r);
        }
    }
    let mut order: Vec<(usize, (usize, usize))> = first.into_iter().collect();
    order.sort_by_key(|&(_, (c, r))| (r, c));
    let n = order.len();
    order
        .into_iter()
        .enumerate()
        .map(|(i, (id, cell))| {
            let label = if n <= KEYS.len() {
                (KEYS[i] as char).to_string()
            } else {
                format!("{}{}", KEYS[(i / KEYS.len()) % KEYS.len()] as char, KEYS[i % KEYS.len()] as char)
            };
            (label, id, cell)
        })
        .collect()
}

/// Detect links in the viewport: OSC 8 hyperlinks first, then plain-text URLs.
/// URLs that soft-wrap across rows are joined and detected as one link.
fn detect_links(
    cells: &[alacritty_terminal::grid::Indexed<&alacritty_terminal::term::cell::Cell>],
    offset: i32,
    rows: usize,
    cols: usize,
    rules: LinkRules,
    resolve: &mut dyn FnMut(&str) -> Option<PathBuf>,
) -> (Vec<Link>, HashMap<(usize, usize), usize>) {
    let mut grid = vec![vec!['\0'; cols]; rows];
    let mut wraps = vec![false; rows];
    // Rows that are a shell prompt (marked by shell integration): no file/PR/ticket links there.
    let mut prompt_rows = vec![false; rows];
    let mut links: Vec<Link> = Vec::new();
    let mut map: HashMap<(usize, usize), usize> = HashMap::new();
    let mut osc_ids: HashMap<String, usize> = HashMap::new();

    for ind in cells {
        let row_i = ind.point.line.0 + offset;
        let col = ind.point.column.0;
        if row_i < 0 || row_i as usize >= rows || col >= cols {
            continue;
        }
        let row = row_i as usize;
        let flags = ind.cell.flags;
        if !flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            grid[row][col] = ind.cell.c;
        }
        if flags.contains(Flags::WRAPLINE) {
            wraps[row] = true;
        }
        if ind.cell.hyperlink().is_some_and(|h| h.uri().starts_with(PROMPT_MARK)) {
            prompt_rows[row] = true;
        }
        if let Some(h) = ind.cell.hyperlink().filter(|h| !h.uri().starts_with(PROMPT_MARK)) {
            let uri = h.uri().to_string();
            let id = *osc_ids.entry(uri.clone()).or_insert_with(|| {
                links.push(Link::Url(uri));
                links.len() - 1
            });
            map.insert((col, row), id);
        }
    }

    let mut r = 0;
    while r < rows {
        let mut chars = Vec::with_capacity(cols);
        let mut pos = Vec::with_capacity(cols);
        let mut is_prompt = false;
        loop {
            is_prompt |= prompt_rows[r];
            for (c, ch) in grid[r].iter().enumerate().take(cols) {
                chars.push(*ch);
                pos.push((c, r));
            }
            let wrapped = wraps[r];
            r += 1;
            if !wrapped || r >= rows {
                break;
            }
        }
        // Earlier kinds win where spans overlap: OSC 8, URLs, files, PR refs, tickets.
        let add = |a: usize, b: usize, link: Link, links: &mut Vec<Link>, map: &mut HashMap<(usize, usize), usize>| {
            if pos[a..b].iter().any(|p| map.contains_key(p)) {
                return;
            }
            links.push(link);
            let id = links.len() - 1;
            for &p in &pos[a..b] {
                map.insert(p, id);
            }
        };
        for (a, b) in find_urls(&chars) {
            add(a, b, Link::Url(chars[a..b].iter().collect()), &mut links, &mut map);
        }
        if rules.files && !is_prompt {
            for c in find_path_candidates(&chars) {
                if let Some(path) = resolve(&c.path) {
                    add(c.start, c.end, Link::File { path, line: c.line, col: c.col }, &mut links, &mut map);
                }
            }
        }
        if rules.prs && !is_prompt {
            for (a, b, n) in find_pr_refs(&chars) {
                add(a, b, Link::Pr(n), &mut links, &mut map);
            }
        }
        if rules.tickets && !is_prompt {
            for (a, b, k) in find_tickets(&chars) {
                add(a, b, Link::Ticket(k), &mut links, &mut map);
            }
        }
    }
    (links, map)
}

/// Snapshot the visible grid into paint-ready batches. Holds the term lock briefly.
fn build_frame(view: &TerminalView, focused: bool) -> Frame {
    let mut term = view.terminal.term.lock();
    // Bottom gravity: draw as if scrolled up past blank rows under the cursor (reverted below,
    // under the same lock, so the terminal's real scroll position never changes).
    // Scrolled back, the shift already shows the hidden rows' worth of extra history, so the
    // last steps up to the very top would move nothing: stop there instead.
    if view.terminal.bottom_gravity {
        use alacritty_terminal::grid::Dimensions;
        let top = term.grid().history_size() - crate::terminal::gravity_hidden(&term);
        let over = term.grid().display_offset().saturating_sub(top);
        if over > 0 {
            term.scroll_display(alacritty_terminal::grid::Scroll::Delta(-(over as i32)));
        }
    }
    let shift = if view.terminal.bottom_gravity { crate::terminal::gravity_shift(&term) } else { 0 };
    if shift > 0 {
        term.scroll_display(alacritty_terminal::grid::Scroll::Delta(shift as i32));
    }
    view.terminal.shift.store(shift, std::sync::atomic::Ordering::Relaxed);
    let frame = build_frame_locked(view, &term, focused);
    if shift > 0 {
        term.scroll_display(alacritty_terminal::grid::Scroll::Delta(-(shift as i32)));
    }
    frame
}

fn build_frame_locked(view: &TerminalView, term: &alacritty_terminal::Term<crate::terminal::Listener>, focused: bool) -> Frame {
    let content = term.renderable_content();
    let palette = &view.palette;
    let colors = content.colors;
    let offset = content.display_offset as i32;
    let default_bg = palette.resolve(AnsiColor::Named(NamedColor::Background), colors);
    let cursor = content.cursor;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && cursor.shape != CursorShape::Hidden;
    let cursor_vp = (cursor.point.line.0 + offset, cursor.point.column.0);
    let selection = content.selection;
    let cells: Vec<_> = content.display_iter.collect();
    let cwd = if view.link_rules.files { view.terminal.cwd() } else { None };
    let mut cache = view.path_cache.borrow_mut();
    if cache.len() > 4096 {
        cache.clear();
    }
    let mut resolve = |text: &str| -> Option<PathBuf> {
        // Absolute and ~ paths resolve without knowing the folder.
        let relative = !text.starts_with('/') && !text.starts_with("~/");
        if relative && cwd.is_none() {
            return None;
        }
        let base = cwd.clone().unwrap_or_default();
        let key = base.join(text);
        let found = cache.entry(key).or_insert_with(|| resolve_path(text, &base)).clone();
        // The tab's own folder is what prompts print; it isn't worth a link.
        found.filter(|p| cwd.as_deref().is_none_or(|c| p.canonicalize().ok().as_deref() != c.canonicalize().ok().as_deref()))
    };
    let (links, link_map) = detect_links(&cells, offset, term.screen_lines(), term.columns(), view.link_rules, &mut resolve);
    drop(cache);
    let hovered = if view.cmd_down { view.hover_cell.and_then(|c| link_map.get(&c).copied()) } else { None };

    let mut bg_rects: Vec<BgRect> = Vec::new();
    let mut batches: Vec<TextBatch> = Vec::new();
    let mut drawn = Vec::new();
    let mut highlights: Vec<BgRect> = Vec::new();

    for indexed in &cells {
        let cell = &indexed.cell;
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let row_i = indexed.point.line.0 + offset;
        if row_i < 0 {
            continue;
        }
        let row = row_i as usize;
        let col = indexed.point.column.0;
        let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

        let mut fg = palette.resolve(cell.fg, colors);
        let mut bg = palette.resolve(cell.bg, colors);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = dim(fg, palette.background);
        }
        let selected = selection.is_some_and(|s| s.contains(indexed.point));
        if selected {
            bg = palette.selection_bg;
            if let Some(sfg) = palette.selection_fg {
                fg = sfg;
            }
        }
        // Find matches: amber wash, the selected one solid.
        if let Some(hit) = view.search.as_ref().and_then(|s| s.hit(indexed.point)) {
            let color = if hit { gpui::hsla(0.09, 0.95, 0.55, 0.85) } else { gpui::hsla(0.13, 0.9, 0.55, 0.32) };
            match highlights.last_mut() {
                Some(r) if r.row == row && r.col + r.cols == col && r.color == color => r.cols += width,
                _ => highlights.push(BgRect { col, row, cols: width, color }),
            }
            if hit {
                fg = palette.background;
            }
        }
        let is_cursor = show_cursor && focused && cursor.shape == CursorShape::Block && (row_i, col) == cursor_vp;
        // URLs always show; files, PRs and tickets light up while ⌘ is held.
        let reveal = view.cmd_down || view.hints.is_some();
        let link = link_map.get(&(col, row)).copied().filter(|&i| reveal || matches!(links[i], Link::Url(_)));
        // Hint mode: everything that isn't a link steps back.
        if view.hints.is_some() && link.is_none() {
            fg = dim(fg, palette.background);
            fg = dim(fg, palette.background);
        }
        if link.is_some() && !selected {
            fg = palette.link;
        }
        if is_cursor {
            fg = palette.cursor_text;
        }

        if bg != default_bg {
            let color = if view.style.cell_alpha { hsla(bg).opacity(view.style.bg_alpha) } else { hsla(bg) };
            match bg_rects.last_mut() {
                Some(r) if r.row == row && r.col + r.cols == col && r.color == color => r.cols += width,
                _ => bg_rects.push(BgRect { col, row, cols: width, color }),
            }
        }

        let blank = cell.c == ' ' || cell.c == '\0' || flags.contains(Flags::HIDDEN);
        let fg_h = hsla(fg);
        if !blank && crate::glyphs::is_builtin(cell.c) {
            drawn.push((col, row, cell.c, fg_h));
            continue;
        }
        let style_flags = flags & (Flags::BOLD | Flags::ITALIC | Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
        let contiguous = |b: &TextBatch| b.row == row && b.col + b.cells == col && !b.solo;
        if blank {
            // Spaces only matter between glyphs: they extend the current run (whatever its
            // color) unless they carry an underline or strikethrough of their own. A run
            // never starts with a space, so blank rows cost nothing.
            let decorated = style_flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
            if let Some(b) = batches.last_mut().filter(|b| contiguous(b) && !decorated && b.link == link) {
                b.text.push(' ');
                b.cells += 1;
            }
            continue;
        }
        let underline_color = cell.underline_color().map(|c| hsla(palette.resolve(c, colors)));
        let solo = width == 2 || cell.zerowidth().is_some() || unicode_wide(cell.c);
        let can_join = !solo
            && batches.last().is_some_and(|b| {
                contiguous(b) && b.fg == fg_h && b.flags == style_flags && b.underline_color == underline_color && b.link == link
            });
        if can_join {
            let b = batches.last_mut().unwrap();
            b.text.push(cell.c);
            b.cells += 1;
        } else {
            let mut text = String::new();
            text.push(cell.c);
            if let Some(zw) = cell.zerowidth() {
                text.extend(zw);
            }
            batches.push(TextBatch { col, row, cells: width, solo, text, fg: fg_h, flags: style_flags, underline_color, link });
        }
    }

    // Trailing spaces carry nothing.
    for b in &mut batches {
        let trimmed = b.text.trim_end_matches(' ').len();
        b.cells -= b.text.len() - trimmed;
        b.text.truncate(trimmed);
    }

    let cursor = (show_cursor && cursor_vp.0 >= 0)
        .then_some((cursor_vp.1, cursor_vp.0 as usize, cursor.shape, focused));
    let hints = view.hints.as_ref().map(|typed| (typed.clone(), hint_labels(&link_map)));
    Frame { bg_rects, highlights, batches, drawn, cursor, links, link_map, cmd_down: view.cmd_down || view.hints.is_some(), hovered, hints }
}

/// With T1K_PROFILE=1, log average and worst build/paint times every 120 frames.
pub mod profile {
    use std::cell::RefCell;
    use std::time::Duration;

    thread_local! {
        static STATS: RefCell<std::collections::HashMap<&'static str, (u32, Duration, Duration)>> = RefCell::default();
    }

    pub fn record(what: &'static str, d: Duration) {
        if std::env::var_os("T1K_PROFILE").is_none() {
            return;
        }
        STATS.with(|s| {
            let mut s = s.borrow_mut();
            let e = s.entry(what).or_insert((0, Duration::ZERO, Duration::ZERO));
            e.0 += 1;
            e.1 += d;
            e.2 = e.2.max(d);
            if e.0 == 30 {
                eprintln!("[profile] {what}: avg {:?}, max {:?}", e.1 / e.0, e.2);
                *e = (0, Duration::ZERO, Duration::ZERO);
            }
        });
    }
}

/// URI prefix of the prompt marks written by T1K's shell integration (never shown as links).
const PROMPT_MARK: &str = "t1k://prompt";

/// Lines (grid coordinates, history negative) where a marked prompt begins, oldest first.
pub(crate) fn prompt_starts<T>(term: &alacritty_terminal::Term<T>) -> Vec<i32> {
    use alacritty_terminal::index::{Column, Line};
    let grid = term.grid();
    let cols = term.columns().min(16);
    let marked = |l: i32| (0..cols).any(|c| grid[Line(l)][Column(c)].hyperlink().is_some_and(|h| h.uri().starts_with(PROMPT_MARK)));
    let top = term.topmost_line().0;
    let bottom = term.bottommost_line().0;
    let mut out = Vec::new();
    let mut prev = false;
    for l in top..=bottom {
        let m = marked(l);
        if m && !prev {
            out.push(l);
        }
        prev = m;
    }
    out
}

/// Resolve a path-looking token against the tab's folder: `~/`, absolute, relative, and git's
/// `a/` / `b/` diff prefixes. Only existing paths become links.
fn resolve_path(text: &str, cwd: &std::path::Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut tries: Vec<PathBuf> = Vec::new();
    if let Some(rest) = text.strip_prefix("~/") {
        tries.push(home?.join(rest));
    } else if text.starts_with('/') {
        tries.push(PathBuf::from(text));
    } else {
        tries.push(cwd.join(text));
        if let Some(rest) = text.strip_prefix("a/").or_else(|| text.strip_prefix("b/")) {
            tries.push(cwd.join(rest));
        }
    }
    tries.into_iter().find(|p| p.exists())
}

/// Wide glyphs (CJK, emoji) are painted on their own so they don't shift neighbors.
fn unicode_wide(c: char) -> bool {
    (c as u32) >= 0x1100
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(s) = self.search.as_mut().filter(|s| s.dirty) {
            s.scan(&self.terminal.term.lock());
        }
        let focused = self.focus_handle.is_focused(window) && window.is_window_active();
        self.sync_scroll_ui(cx);
        let t0 = std::time::Instant::now();
        let frame = build_frame(self, focused);
        profile::record("build_frame", t0.elapsed());
        let scrollbar = self.scrollbar_paint();
        let dragging = self.scroll_ui.drag.is_some();
        let button = self.scroll_ui.button.value();
        let button = (self.scroll_ui.eligible && button > 0.).then(|| scroll_button(self.scroll_ui.new_lines, button, self.button_bottom(), &self.palette, cx));
        let s = &self.scroll_ui;
        if !(s.bar.settled() && s.wide.settled() && s.button.settled()) {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        self.links = frame.links.clone();
        self.link_map = frame.link_map.clone();
        self.hint_labels = frame.hints.as_ref().map(|(_, l)| l.iter().map(|(s, id, _)| (s.clone(), *id)).collect()).unwrap_or_default();
        let pointer = frame.hovered.is_some() || self.on_url();
        let palette = self.palette.clone();
        let style = self.style.clone();
        let entity = cx.entity();

        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(hsla(palette.background).opacity(style.bg_alpha))
            .py(style.padding)
            .px(style.padding_x)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear_scrollback))
            .on_action(cx.listener(Self::show_link_hints))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::previous_prompt))
            .on_action(cx.listener(Self::next_prompt))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::copy_last_output))
            .on_action(cx.listener(Self::copy_last_command))
            .on_action(cx.listener(Self::select_last_output))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .when(pointer, |d| d.cursor(CursorStyle::PointingHand))
            .child(
                canvas(
                    {
                        let style = style.clone();
                        let entity = entity.clone();
                        move |bounds, window, cx| {
                            let font = style.font();
                            let ts = window.text_system();
                            let font_id = ts.resolve_font(&font);
                            let cell_w = ts
                                .advance(font_id, style.font_size, 'm')
                                .map(|s| f32::from(s.width))
                                .unwrap_or(f32::from(style.font_size) * 0.6);
                            let cell_h = (f32::from(style.font_size) * style.line_height).round();
                            let g = GridSize {
                                cols: ((f32::from(bounds.size.width) / cell_w).floor() as usize).max(2),
                                rows: ((f32::from(bounds.size.height) / cell_h).floor() as usize).max(1),
                                cell_width: cell_w,
                                cell_height: cell_h,
                            };
                            // Paint with the grid's actual size; it may lag the pane while a
                            // resize settles (the pane clips the difference).
                            entity.update(cx, |view, cx| {
                                view.request_resize(g, cx);
                                let applied = view.terminal.size;
                                view.layout = Some((bounds, applied));
                                applied
                            })
                        }
                    },
                    move |bounds, g, window, cx| {
                        let t0 = std::time::Instant::now();
                        paint_frame(bounds, g, &frame, &palette, &style, window, cx);
                        if let Some(sb) = &scrollbar {
                            let pane = Bounds::new(
                                point(bounds.origin.x - style.padding_x, bounds.origin.y - style.padding),
                                size(bounds.size.width + style.padding_x * 2., bounds.size.height + style.padding * 2.),
                            );
                            crate::scrollbar::paint(pane, sb, window);
                        }
                        profile::record("paint", t0.elapsed());
                        // A thumb drag follows the pointer anywhere in the window, not just over
                        // this pane.
                        if dragging {
                            let e = entity.clone();
                            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                                if phase == gpui::DispatchPhase::Bubble {
                                    e.update(cx, |v, cx| v.drag_to(ev.position.y, cx));
                                }
                            });
                            let e = entity.clone();
                            window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                                if phase == gpui::DispatchPhase::Bubble {
                                    e.update(cx, |v, cx| v.end_drag(ev.position, cx));
                                }
                            });
                        }
                    },
                )
                .size_full(),
            )
            .children(self.copy_overlay(cx))
            .children(button)
            .children(self.search.as_ref().map(|s| find_bar(s, &self.palette, cx)))
            .children(self.hints.as_ref().map(|typed| hint_bar(typed, self.hint_labels.len(), &self.palette)))
            .children(
                self.link_tip
                    .filter(|(_, t, _)| !self.cmd_down && t.elapsed() >= std::time::Duration::from_millis(600))
                    .map(|(_, _, at)| link_tip(at, &self.palette)),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.link_tip.take().is_some() {
                    this.link_tip_task = None;
                    cx.notify();
                }
                if !hovered && this.scroll_ui.drag.is_none() {
                    this.set_near_edge(false, cx);
                }
                if !hovered {
                    this.copy_hover(None, false, cx);
                }
            }))
    }
}

/// Mix `t` of the way from `a` to `b`.
fn mix(a: Rgb, b: Rgb, t: f32) -> Hsla {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    hsla(Rgb { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b) })
}

/// The scroll-to-bottom button, bottom-right: a round ↓, widening into "↓ 128 new lines"
/// when output arrived while scrolled up.
fn scroll_button(new_lines: usize, opacity: f32, bottom: f32, palette: &Palette, cx: &mut Context<TerminalView>) -> impl IntoElement {
    let fg = hsla(palette.foreground);
    let rest = mix(palette.background, palette.foreground, 0.10);
    let hover = mix(palette.background, palette.foreground, 0.16);
    let pill = new_lines > 0;
    div()
        .id("scroll-to-bottom")
        .absolute()
        .right(px(24.))
        .bottom(px(bottom))
        .h(px(30.))
        .min_w(px(30.))
        .when(pill, |d| d.pl(px(10.)).pr(px(12.)).gap(px(6.)))
        .rounded(px(15.))
        .bg(rest)
        .hover(move |d| d.bg(hover))
        .border_1()
        .border_color(fg.opacity(0.12))
        .shadow(vec![gpui::BoxShadow {
            color: gpui::hsla(0., 0., 0., 0.25),
            offset: point(px(0.), px(4.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
        }])
        .opacity(opacity)
        .flex()
        .items_center()
        .justify_center()
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .text_color(fg)
        .cursor(CursorStyle::PointingHand)
        .child(div().text_color(fg.opacity(0.85)).child("↓"))
        .when(pill, |d| d.child(crate::scrollbar::new_lines_label(new_lines)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|v, _, _, cx| v.jump_to_bottom(cx)))
}

/// "⌘ + click to open", just below the pointer, while it rests on a link.
fn link_tip(at: Point<Pixels>, palette: &Palette) -> impl IntoElement {
    let fg = hsla(palette.foreground);
    let bg = hsla(palette.background);
    let raised = gpui::Hsla { l: if bg.l < 0.5 { bg.l + 0.1 } else { bg.l - 0.06 }, ..bg };
    gpui::deferred(
        gpui::anchored().position(point(at.x + px(10.), at.y + px(16.))).snap_to_window_with_margin(px(8.)).child(
            div()
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .bg(raised)
                .border_1()
                .border_color(fg.opacity(0.12))
                .shadow_md()
                .font_family(".SystemUIFont")
                .text_size(px(11.5))
                .text_color(fg.opacity(0.85))
                .child("⌘ + click to open"),
        ),
    )
    .with_priority(1)
}

/// Hint mode's bottom bar: what you've typed and how to get out.
fn hint_bar(typed: &str, count: usize, palette: &Palette) -> impl IntoElement {
    let fg = hsla(palette.foreground);
    let bg = hsla(palette.background);
    let raised = gpui::Hsla { l: if bg.l < 0.5 { bg.l + 0.07 } else { bg.l - 0.05 }, ..bg };
    let amber = gpui::hsla(0.11, 0.85, 0.6, 1.);
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(12.))
        .bg(raised)
        .border_t_1()
        .border_color(fg.opacity(0.12))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .text_color(fg.opacity(0.6))
        .child(div().text_color(fg).font_weight(FontWeight::SEMIBOLD).child("Hint mode"))
        .when(!typed.is_empty(), |d| {
            d.child(div().px(px(6.)).h(px(18.)).flex().items_center().rounded(px(4.)).bg(amber).text_color(bg).font_family("JetBrains Mono").child(format!("{}_", typed.to_uppercase())))
        })
        .child(format!("{count} link{} · type a label to open", if count == 1 { "" } else { "s" }))
        .child(div().flex_1())
        .child("⌫ back · esc")
}

/// The find bar: floats at the top right of the terminal.
fn find_bar(s: &crate::search::Search, palette: &Palette, cx: &mut Context<TerminalView>) -> impl IntoElement {
    let fg = hsla(palette.foreground);
    let bg = hsla(palette.background);
    let raised = gpui::Hsla { l: if bg.l < 0.5 { bg.l + 0.07 } else { bg.l - 0.05 }, ..bg };
    let quiet = fg.opacity(0.55);
    let button = |id: &'static str, label: &'static str| {
        div()
            .id(id)
            .w(px(24.))
            .h(px(22.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(quiet)
            .hover(move |d| d.bg(fg.opacity(0.1)).text_color(fg))
            .child(label)
    };
    let empty = s.query.is_empty();
    let none = !empty && s.matches.is_empty();
    div()
        .absolute()
        .top(px(8.))
        .right(px(16.))
        .w(px(340.))
        .h(px(34.))
        .px(px(10.))
        .rounded(px(8.))
        .bg(raised)
        .border_1()
        .border_color(fg.opacity(0.14))
        .shadow_lg()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(13.))
        .font_family(".SystemUIFont")
        .child(div().text_color(quiet).child("⌕"))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if empty { quiet } else { fg })
                .child(if empty { "Find".to_string() } else { format!("{}▏", s.query) }),
        )
        .child(div().flex_none().text_size(px(12.)).text_color(if none { gpui::red().opacity(0.8) } else { quiet }).child(s.label()))
        .child(button("find-prev", "↑").on_click(cx.listener(|v, _, _, cx| v.search_step(false, cx))))
        .child(button("find-next", "↓").on_click(cx.listener(|v, _, _, cx| v.search_step(true, cx))))
        .child(button("find-close", "✕").on_click(cx.listener(|v, _, _, cx| v.close_search(cx))))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn paint_frame(
    bounds: Bounds<Pixels>,
    g: GridSize,
    frame: &Frame,
    palette: &Palette,
    style: &TerminalStyle,
    window: &mut Window,
    cx: &mut App,
) {
    let cw = g.cell_width;
    let ch = g.cell_height;
    let origin = bounds.origin;
    let at = |col: usize, row: usize| point(origin.x + px(col as f32 * cw), origin.y + px(row as f32 * ch));

    for r in &frame.bg_rects {
        if r.row >= g.rows {
            continue;
        }
        window.paint_quad(fill(Bounds::new(at(r.col, r.row), size(px(r.cols as f32 * cw), px(ch))), r.color));
    }
    for r in &frame.highlights {
        if r.row < g.rows {
            window.paint_quad(fill(Bounds::new(at(r.col, r.row), size(px(r.cols as f32 * cw), px(ch))), r.color).corner_radii(px(2.)));
        }
    }

    let cursor_color = hsla(palette.cursor);
    if let Some((col, row, shape, focused)) = frame.cursor {
        if row < g.rows && focused && shape == CursorShape::Block {
            window.paint_quad(fill(Bounds::new(at(col, row), size(px(cw), px(ch))), cursor_color));
        }
    }

    let link_color = hsla(palette.link);
    if let Some(h) = frame.hovered {
        for b in frame.batches.iter().filter(|b| b.link == Some(h) && b.row < g.rows) {
            let cols = b.text.chars().count();
            window.paint_quad(fill(
                Bounds::new(at(b.col, b.row), size(px(cols as f32 * cw), px(ch))),
                link_color.opacity(0.16),
            ));
        }
    }

    let fs = f32::from(style.font_size);
    for &(col, row, c, color) in &frame.drawn {
        if row >= g.rows {
            continue;
        }
        // Snap the cell to whole pixels so neighbouring blocks meet without seams.
        let o = at(col, row);
        let (x0, y0) = (f32::from(o.x).round(), f32::from(o.y).round());
        let next = at(col + 1, row + 1);
        let (w, h) = (f32::from(next.x).round() - x0, f32::from(next.y).round() - y0);
        for piece in crate::glyphs::pieces(c, w, h, fs).unwrap_or_default() {
            match piece {
                crate::glyphs::Piece::Rect { x, y, w, h, alpha } => window.paint_quad(fill(
                    Bounds::new(point(px(x0 + x), px(y0 + y)), size(px(w), px(h))),
                    color.opacity(alpha),
                )),
                crate::glyphs::Piece::Arc { right, down, t } => {
                    // A quad with two borders and one rounded corner: its edges run from the cell
                    // center out to the cell edges, matching the straight lines next to it.
                    let (cx, cy) = (x0 + (w / 2.).floor(), y0 + (h / 2.).floor());
                    let lo = |c: f32| c - (t / 2.).floor();
                    let hi = |c: f32| c - (t / 2.).floor() + t;
                    let (left, wd) = if right { (lo(cx), x0 + w - lo(cx)) } else { (x0, hi(cx) - x0) };
                    let (top, ht) = if down { (lo(cy), y0 + h - lo(cy)) } else { (y0, hi(cy) - y0) };
                    let r = px((w / 2.).min(h / 2.).round());
                    let mut radii = gpui::Corners::all(px(0.));
                    let mut edges = gpui::Edges::all(px(0.));
                    match (right, down) {
                        (true, true) => (radii.top_left, edges.top, edges.left) = (r, px(t), px(t)),
                        (false, true) => (radii.top_right, edges.top, edges.right) = (r, px(t), px(t)),
                        (false, false) => (radii.bottom_right, edges.bottom, edges.right) = (r, px(t), px(t)),
                        (true, false) => (radii.bottom_left, edges.bottom, edges.left) = (r, px(t), px(t)),
                    }
                    window.paint_quad(gpui::quad(
                        Bounds::new(point(px(left), px(top)), size(px(wd), px(ht))),
                        radii,
                        gpui::transparent_black(),
                        edges,
                        color,
                        gpui::BorderStyle::Solid,
                    ));
                }
            }
        }
    }

    // One shaped line per row: every batch on the row becomes a styled run, and the gaps
    // between batches become plain spaces. Wide and combining glyphs stay on their own.
    let base = style.font();
    let run_for = |b: &TextBatch| {
        let font = Font {
            weight: if b.flags.contains(Flags::BOLD) { FontWeight::BOLD } else { FontWeight::NORMAL },
            style: if b.flags.contains(Flags::ITALIC) { FontStyle::Italic } else { FontStyle::Normal },
            ..base.clone()
        };
        // Link underlines are painted per span below, so they don't break at spaces.
        let underline = if b.link.is_some() {
            None
        } else {
            b.flags.intersects(Flags::ALL_UNDERLINES).then(|| UnderlineStyle {
                thickness: px(1.),
                color: Some(b.underline_color.unwrap_or(b.fg)),
                wavy: b.flags.contains(Flags::UNDERCURL),
            })
        };
        let strikethrough = b.flags.contains(Flags::STRIKEOUT).then(|| StrikethroughStyle { thickness: px(1.), color: Some(b.fg) });
        TextRun { len: b.text.len(), font, color: b.fg, background_color: None, underline, strikethrough }
    };
    let mut i = 0;
    let batches = &frame.batches;
    while i < batches.len() {
        let first = &batches[i];
        if first.row >= g.rows {
            i += 1;
            continue;
        }
        let mut text = first.text.clone();
        let mut runs = vec![run_for(first)];
        let mut end = first.col + first.cells;
        let mut j = i + 1;
        if !first.solo {
            while j < batches.len() && batches[j].row == first.row && !batches[j].solo && batches[j].col >= end {
                let b = &batches[j];
                let gap = b.col - end;
                if gap > 0 {
                    text.extend(std::iter::repeat_n(' ', gap));
                    runs.push(TextRun { len: gap, font: base.clone(), color: b.fg, background_color: None, underline: None, strikethrough: None });
                }
                text.push_str(&b.text);
                runs.push(run_for(b));
                end = b.col + b.cells;
                j += 1;
            }
        }
        let shaped = window.text_system().shape_line(SharedString::from(text), style.font_size, &runs, Some(px(cw)));
        let _ = shaped.paint(at(first.col, first.row), px(ch), window, cx);
        i = j;
    }

    // Links always carry an underline so they scan at a glance; cmd-hover makes it solid.
    let mut spans: Vec<(usize, usize, usize)> = frame
        .link_map
        .iter()
        .filter(|(_, &id)| frame.cmd_down || matches!(frame.links[id], Link::Url(_)))
        .map(|(&(c, r), &id)| (r, c, id))
        .collect();
    spans.sort_unstable();
    let mut i = 0;
    while i < spans.len() {
        let (row, start, id) = spans[i];
        let mut end = start + 1;
        while i + 1 < spans.len() && spans[i + 1] == (row, end, id) {
            end += 1;
            i += 1;
        }
        i += 1;
        if row >= g.rows {
            continue;
        }
        let alpha = if frame.hovered == Some(id) { 1. } else { 0.5 };
        let o = at(start, row);
        window.paint_quad(fill(
            Bounds::new(point(o.x, o.y + px(ch - 3.)), size(px((end - start) as f32 * cw), px(1.))),
            link_color.opacity(alpha),
        ));
    }

    if let Some((col, row, shape, focused)) = frame.cursor {
        if row < g.rows {
            let o = at(col, row);
            match (shape, focused) {
                (CursorShape::Beam, true) => {
                    window.paint_quad(fill(Bounds::new(o, size(px(2.), px(ch))), cursor_color))
                }
                (CursorShape::Underline, true) => window.paint_quad(fill(
                    Bounds::new(point(o.x, o.y + px(ch - 2.)), size(px(cw), px(2.))),
                    cursor_color,
                )),
                (CursorShape::Block, true) => {}
                // Unfocused or hollow: outline box.
                _ => window.paint_quad(gpui::outline(
                    Bounds::new(o, size(px(cw), px(ch))),
                    cursor_color,
                    gpui::BorderStyle::Solid,
                )),
            }
        }
    }

    // Hint badges: a label on each link's first cell. The typed part fades; labels that no
    // longer match disappear.
    if let Some((typed, labels)) = &frame.hints {
        // Labels still in play turn amber once you start typing.
        let bg = if typed.is_empty() { hsla(palette.link) } else { gpui::hsla(0.11, 0.85, 0.6, 1.) };
        let fg = hsla(palette.background);
        for (label, _, (col, row)) in labels {
            if *row >= g.rows || !label.starts_with(typed.as_str()) {
                continue;
            }
            let o = at(*col, *row);
            let w = label.chars().count() as f32 * cw + 4.;
            window.paint_quad(
                fill(Bounds::new(point(o.x - px(2.), o.y + px(1.)), size(px(w), px(ch - 2.))), bg).corner_radii(px(3.)),
            );
            let font = Font { weight: FontWeight::BOLD, ..style.font() };
            let runs = [
                TextRun { len: typed.len(), font: font.clone(), color: fg.opacity(0.45), background_color: None, underline: None, strikethrough: None },
                TextRun { len: label.len() - typed.len(), font, color: fg, background_color: None, underline: None, strikethrough: None },
            ];
            let runs: Vec<TextRun> = runs.into_iter().filter(|r| r.len > 0).collect();
            let shaped = window.text_system().shape_line(SharedString::from(label.to_uppercase()), style.font_size, &runs, Some(px(cw)));
            let _ = shaped.paint(o, px(ch), window, cx);
        }
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::prompt_starts;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{test::TermSize, Config, Term};
    use alacritty_terminal::vte::ansi::Processor;

    #[test]
    fn finds_marked_prompts() {
        let mut term = Term::new(Config::default(), &TermSize::new(40, 10), VoidListener);
        let mut parser: Processor = Processor::new();
        let open = "\x1b]8;;t1k://prompt\x1b\\";
        let close = "\x1b]8;;\x1b\\";
        let text = format!("{open}$ ls{close}\r\na\r\nb\r\n{open}$ git status{close}\r\nclean\r\n{open}$ ");
        parser.advance(&mut term, text.as_bytes());
        assert_eq!(prompt_starts(&term), vec![0, 3, 5]);
    }

    /// The scroll-to-bottom button's count relies on this: output while scrolled up leaves the
    /// view where it is and raises the display offset by the lines that went into history.
    #[test]
    fn output_while_scrolled_up_keeps_the_view() {
        use alacritty_terminal::grid::{Dimensions, Scroll};
        use alacritty_terminal::index::{Column, Line};
        let mut term = Term::new(Config::default(), &TermSize::new(20, 10), VoidListener);
        let mut parser: Processor = Processor::new();
        let lines = |a: usize, b: usize| (a..b).map(|i| format!("line {i}\r\n")).collect::<String>();
        parser.advance(&mut term, lines(0, 100).as_bytes());
        term.scroll_display(Scroll::Delta(30));
        let top = |t: &Term<VoidListener>| {
            let line = Line(-(t.grid().display_offset() as i32));
            (0..8).map(|c| t.grid()[line][Column(c)].c).collect::<String>()
        };
        let before = top(&term);
        parser.advance(&mut term, lines(100, 128).as_bytes());
        assert_eq!(term.grid().display_offset(), 30 + 28);
        assert_eq!(top(&term), before);
    }

    /// A TUI that redraws a short screen from the top (Claude Code after compacting) still gets
    /// the gap filled; a clear that leaves the prompt at the top doesn't.
    #[test]
    fn gravity_fills_short_redraw_not_clear() {
        let mut term = Term::new(Config::default(), &TermSize::new(20, 30), VoidListener);
        let mut parser: Processor = Processor::new();
        let text = (0..100).map(|i| format!("line {i}\r\n")).collect::<String>();
        parser.advance(&mut term, text.as_bytes());
        parser.advance(&mut term, b"\x1b[2J\x1b[Hone\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\n> ");
        assert_eq!(crate::terminal::gravity_hidden(&term), 23);
        parser.advance(&mut term, b"\x1b[2J\x1b[H% ");
        assert_eq!(crate::terminal::gravity_hidden(&term), 0);
    }

    /// Scrolling back from a gravity-shifted bottom moves one line at a time from what was
    /// shown; it used to jump to the blank rows first.
    #[test]
    fn gravity_shift_carries_into_scrollback() {
        use alacritty_terminal::grid::{Dimensions, Scroll};
        let mut term = Term::new(Config::default(), &TermSize::new(20, 10), VoidListener);
        let mut parser: Processor = Processor::new();
        let text = (0..100).map(|i| format!("line {i}\r\n")).collect::<String>();
        parser.advance(&mut term, text.as_bytes());
        // A TUI shrinks its live region: up 5 rows, erase below.
        parser.advance(&mut term, b"\x1b[5A\x1b[J");
        let shift = crate::terminal::gravity_shift(&term);
        assert_eq!(shift, 5);
        let shown = |t: &Term<VoidListener>| t.grid().display_offset() + crate::terminal::gravity_shift(t);
        let at_bottom = shown(&term);
        for step in 1..=3 {
            term.scroll_display(Scroll::Delta(1));
            assert_eq!(shown(&term), at_bottom + step);
        }
        // The top of history is reached 5 steps early, with the whole shift still applied.
        let hidden = crate::terminal::gravity_hidden(&term);
        assert_eq!(hidden, 5);
        term.scroll_display(Scroll::Top);
        let top = term.grid().history_size() - hidden;
        term.scroll_display(Scroll::Delta(top as i32 - term.grid().display_offset() as i32));
        assert_eq!(shown(&term), term.grid().history_size());
    }
}
