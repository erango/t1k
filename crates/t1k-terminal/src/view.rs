//! GPUI view that renders a `Terminal` grid and routes input to it.

use std::collections::HashMap;

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

use crate::input::to_esc;
use crate::links::find_urls;
use crate::palette::{dim, Palette};
use crate::terminal::{GridSize, SpawnOptions, Terminal};

actions!(terminal, [Copy, Paste, ClearScrollback, ScrollPageUp, ScrollPageDown]);

/// Events the terminal view emits to its container (tab/workspace).
#[derive(Clone, Debug)]
pub enum TerminalEvent {
    TitleChanged(String),
    Bell,
    Exited,
}

#[derive(Clone)]
pub struct TerminalStyle {
    pub font_family: SharedString,
    pub font_size: Pixels,
    pub line_height: f32,
    pub padding: Pixels,
    pub option_as_meta: bool,
}

impl Default for TerminalStyle {
    fn default() -> Self {
        Self {
            font_family: "Menlo".into(),
            font_size: px(13.),
            line_height: 1.3,
            padding: px(10.),
            option_as_meta: false,
        }
    }
}

pub struct TerminalView {
    pub terminal: Terminal,
    focus_handle: FocusHandle,
    pub palette: Palette,
    pub style: TerminalStyle,
    /// Last painted content bounds and grid geometry, used for mouse → cell mapping.
    layout: Option<(Bounds<Pixels>, GridSize)>,
    scroll_accum: f32,
    /// Links found in the last painted frame: viewport (col, row) -> index into `links`.
    link_map: HashMap<(usize, usize), usize>,
    links: Vec<String>,
    /// Viewport cell under the mouse, and whether cmd is held (cmd-hover arms a link).
    hover_cell: Option<(usize, usize)>,
    cmd_down: bool,
    _events: Task<()>,
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

        cx.on_focus(&focus_handle, window, |this, window, _| {
            this.terminal.set_focused(window.is_window_active());
        })
        .detach();
        cx.on_blur(&focus_handle, window, |this, _, _| this.terminal.set_focused(false)).detach();
        cx.observe_window_activation(window, |this, window, _| {
            let focused = window.is_window_active() && this.focus_handle.is_focused(window);
            this.terminal.set_focused(focused);
        })
        .detach();

        Ok(Self {
            terminal,
            focus_handle,
            palette: Palette::default(),
            style: TerminalStyle::default(),
            layout: None,
            scroll_accum: 0.,
            link_map: HashMap::new(),
            links: Vec::new(),
            hover_cell: None,
            cmd_down: false,
            _events: task,
        })
    }

    fn handle_event(&mut self, ev: AlacEvent, cx: &mut Context<Self>) {
        match ev {
            AlacEvent::Wakeup | AlacEvent::MouseCursorDirty | AlacEvent::CursorBlinkingChange => cx.notify(),
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

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mode = self.terminal.mode();
        if let Some(bytes) = to_esc(&ev.keystroke, mode, self.style.option_as_meta) {
            self.terminal.clear_selection();
            self.terminal.scroll_to_bottom();
            self.terminal.write(bytes);
            cx.stop_propagation();
            cx.notify();
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
        cx.notify();
    }

    fn page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.terminal.size.rows as i32;
        self.terminal.scroll(-rows);
        cx.notify();
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

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let Some((col, row, side)) = self.cell_at(ev.position) else { return };
        if ev.modifiers.platform {
            if let Some(url) = self.link_at(col, row) {
                cx.open_url(&url);
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
            _ => SelectionType::Simple,
        };
        if ev.click_count == 1 {
            self.terminal.clear_selection();
        }
        self.terminal.start_selection(col, row, side, ty);
        cx.notify();
    }

    fn link_at(&self, col: usize, row: usize) -> Option<String> {
        self.link_map.get(&(col, row)).map(|&i| self.links[i].clone())
    }

    fn set_hover(&mut self, cell: Option<(usize, usize)>, cmd: bool, cx: &mut Context<Self>) {
        let armed_before = self.cmd_down && self.hover_cell.is_some_and(|c| self.link_map.contains_key(&c));
        self.hover_cell = cell;
        self.cmd_down = cmd;
        let armed_now = cmd && cell.is_some_and(|c| self.link_map.contains_key(&c));
        if armed_before || armed_now {
            cx.notify();
        }
    }

    fn on_modifiers_changed(&mut self, ev: &ModifiersChangedEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.set_hover(self.hover_cell, ev.modifiers.platform, cx);
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let cell = self.cell_at(ev.position).map(|(c, r, _)| (c, r));
        self.set_hover(cell, ev.modifiers.platform, cx);
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
        cx.notify();
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((col, row, _)) = self.cell_at(ev.position) else { return };
        if self.mouse_reporting(ev.modifiers.shift) {
            self.report_mouse(0, col, row, false);
            return;
        }
        if self.terminal.selection_text().is_none() {
            self.terminal.clear_selection();
        }
        cx.notify();
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
            cx.notify();
        }
    }
}

// ---- rendering ---------------------------------------------------------------------------

struct TextBatch {
    col: usize,
    row: usize,
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
    batches: Vec<TextBatch>,
    cursor: Option<(usize, usize, CursorShape, bool)>,
    links: Vec<String>,
    link_map: HashMap<(usize, usize), usize>,
    /// Link under the mouse while cmd is held.
    hovered: Option<usize>,
}

/// Detect links in the viewport: OSC 8 hyperlinks first, then plain-text URLs.
/// URLs that soft-wrap across rows are joined and detected as one link.
fn detect_links(
    cells: &[alacritty_terminal::grid::Indexed<&alacritty_terminal::term::cell::Cell>],
    offset: i32,
    rows: usize,
    cols: usize,
) -> (Vec<String>, HashMap<(usize, usize), usize>) {
    let mut grid = vec![vec!['\0'; cols]; rows];
    let mut wraps = vec![false; rows];
    let mut links: Vec<String> = Vec::new();
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
        if let Some(h) = ind.cell.hyperlink() {
            let uri = h.uri().to_string();
            let id = *osc_ids.entry(uri.clone()).or_insert_with(|| {
                links.push(uri);
                links.len() - 1
            });
            map.insert((col, row), id);
        }
    }

    let mut r = 0;
    while r < rows {
        let mut chars = Vec::with_capacity(cols);
        let mut pos = Vec::with_capacity(cols);
        loop {
            for c in 0..cols {
                chars.push(grid[r][c]);
                pos.push((c, r));
            }
            let wrapped = wraps[r];
            r += 1;
            if !wrapped || r >= rows {
                break;
            }
        }
        for (a, b) in find_urls(&chars) {
            if pos[a..b].iter().any(|p| map.contains_key(p)) {
                continue;
            }
            links.push(chars[a..b].iter().collect());
            let id = links.len() - 1;
            for &p in &pos[a..b] {
                map.insert(p, id);
            }
        }
    }
    (links, map)
}

/// Snapshot the visible grid into paint-ready batches. Holds the term lock briefly.
fn build_frame(view: &TerminalView, focused: bool) -> Frame {
    let term = view.terminal.term.lock();
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
    let (links, link_map) = detect_links(&cells, offset, term.screen_lines(), term.columns());
    let hovered = if view.cmd_down { view.hover_cell.and_then(|c| link_map.get(&c).copied()) } else { None };

    let mut bg_rects: Vec<BgRect> = Vec::new();
    let mut batches: Vec<TextBatch> = Vec::new();

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
        let is_cursor = show_cursor && focused && cursor.shape == CursorShape::Block && (row_i, col) == cursor_vp;
        let link = link_map.get(&(col, row)).copied();
        if link.is_some() && !selected {
            fg = palette.link;
        }
        if is_cursor {
            fg = palette.cursor_text;
        }

        if bg != default_bg {
            let color = hsla(bg);
            match bg_rects.last_mut() {
                Some(r) if r.row == row && r.col + r.cols == col && r.color == color => r.cols += width,
                _ => bg_rects.push(BgRect { col, row, cols: width, color }),
            }
        }

        if cell.c == ' ' || cell.c == '\0' || flags.contains(Flags::HIDDEN) {
            continue;
        }
        let fg_h = hsla(fg);
        let style_flags = flags & (Flags::BOLD | Flags::ITALIC | Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
        let underline_color = cell.underline_color().map(|c| hsla(palette.resolve(c, colors)));
        let can_join = width == 1
            && cell.zerowidth().is_none()
            && batches.last().is_some_and(|b| {
                b.row == row
                    && b.col + b.text.chars().count() == col
                    && b.fg == fg_h
                    && b.flags == style_flags
                    && b.underline_color == underline_color
                    && b.link == link
                    && !b.text.chars().any(|c| c.len_utf8() > 1 && unicode_wide(c))
            });
        if can_join {
            batches.last_mut().unwrap().text.push(cell.c);
        } else {
            let mut text = String::new();
            text.push(cell.c);
            if let Some(zw) = cell.zerowidth() {
                text.extend(zw);
            }
            batches.push(TextBatch { col, row, text, fg: fg_h, flags: style_flags, underline_color, link });
        }
    }

    let cursor = (show_cursor && cursor_vp.0 >= 0)
        .then(|| (cursor_vp.1, cursor_vp.0 as usize, cursor.shape, focused));
    Frame { bg_rects, batches, cursor, links, link_map, hovered }
}

/// Wide glyphs (CJK, emoji) are painted on their own so they don't shift neighbors.
fn unicode_wide(c: char) -> bool {
    (c as u32) >= 0x1100
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window) && window.is_window_active();
        let frame = build_frame(self, focused);
        self.links = frame.links.clone();
        self.link_map = frame.link_map.clone();
        let pointer = frame.hovered.is_some();
        let palette = self.palette.clone();
        let style = self.style.clone();
        let entity = cx.entity();

        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(hsla(palette.background))
            .p(style.padding)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear_scrollback))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
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
                            let font = gpui::font(style.font_family.clone());
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
                            entity.update(cx, |view, _| {
                                view.terminal.resize(g);
                                view.layout = Some((bounds, g));
                            });
                            g
                        }
                    },
                    move |bounds, g, window, cx| {
                        paint_frame(bounds, g, &frame, &palette, &style, window, cx);
                    },
                )
                .size_full(),
            )
    }
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

    let base = gpui::font(style.font_family.clone());
    for b in &frame.batches {
        if b.row >= g.rows {
            continue;
        }
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
        let strikethrough = b
            .flags
            .contains(Flags::STRIKEOUT)
            .then(|| StrikethroughStyle { thickness: px(1.), color: Some(b.fg) });
        let run = TextRun {
            len: b.text.len(),
            font,
            color: b.fg,
            background_color: None,
            underline,
            strikethrough,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(b.text.clone()),
            style.font_size,
            &[run],
            Some(px(cw)),
        );
        let _ = shaped.paint(at(b.col, b.row), px(ch), window, cx);
    }

    // Links always carry an underline so they scan at a glance; cmd-hover makes it solid.
    let mut spans: Vec<(usize, usize, usize)> = frame.link_map.iter().map(|(&(c, r), &id)| (r, c, id)).collect();
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
}
