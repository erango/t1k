//! Panes (canvas v3-ap): up to four terminals in a tab, as a tree of right (⌘D) and down (⌘⇧D)
//! splits with draggable dividers, zoom (⌘⇧↵) and ⌘⌥ arrows to move between them.
//! The main pane (id 0) is the tab's identity: its title, agent and folder drive the sidebar row,
//! the strip, ⌘J and notifications. The others are helpers for a server, tests or logs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    canvas, div, prelude::*, px, relative, AnyElement, Bounds, Context, CursorStyle, Entity, Focusable, MouseButton, Pixels, SharedString, Subscription, Window,
};
use serde::{Deserialize, Serialize};
use t1k_terminal::view::{TerminalEvent, TerminalView};

use crate::workspace::{Tab, Workspace};

pub type PaneId = u32;
/// The tab's own terminal.
pub const MAIN: PaneId = 0;
pub const MAX_PANES: usize = 4;
const HEADER_H: f32 = 22.;
/// Each side of a divider keeps at least this many columns and rows.
const MIN_COLS: f32 = 20.;
const MIN_ROWS: f32 = 5.;

/// Where each pane sits: a leaf, or two parts side by side (`down`: one above the other).
/// `ratio` is the first part's share.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Layout {
    Leaf(PaneId),
    Split { down: bool, ratio: f32, a: Box<Layout>, b: Box<Layout> },
}

impl Layout {
    /// Pane ids in reading order (left to right, top to bottom).
    pub fn leaves(&self) -> Vec<PaneId> {
        match self {
            Layout::Leaf(id) => vec![*id],
            Layout::Split { a, b, .. } => {
                let mut v = a.leaves();
                v.extend(b.leaves());
                v
            }
        }
    }

    /// Halve the pane `target`, putting `new` after it. False when `target` isn't here.
    fn split_leaf(&mut self, target: PaneId, new: PaneId, down: bool) -> bool {
        match self {
            Layout::Leaf(id) if *id == target => {
                *self = Layout::Split { down, ratio: 0.5, a: Box::new(Layout::Leaf(target)), b: Box::new(Layout::Leaf(new)) };
                true
            }
            Layout::Leaf(_) => false,
            Layout::Split { a, b, .. } => a.split_leaf(target, new, down) || b.split_leaf(target, new, down),
        }
    }

    /// The tree without pane `id`: its sibling takes its parent's place. None when it was the
    /// only pane.
    fn without(self, id: PaneId) -> Option<Layout> {
        match self {
            Layout::Leaf(x) if x == id => None,
            Layout::Leaf(_) => Some(self),
            Layout::Split { down, ratio, a, b } => match (a.without(id), b.without(id)) {
                (Some(a), Some(b)) => Some(Layout::Split { down, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(x), None) | (None, Some(x)) => Some(x),
                (None, None) => None,
            },
        }
    }

    fn rename(&mut self, from: PaneId, to: PaneId) {
        match self {
            Layout::Leaf(id) if *id == from => *id = to,
            Layout::Leaf(_) => {}
            Layout::Split { a, b, .. } => {
                a.rename(from, to);
                b.rename(from, to);
            }
        }
    }

    /// The split at `path` (false: first part, true: second).
    fn node_mut(&mut self, path: &[bool]) -> Option<&mut Layout> {
        match (path.split_first(), self) {
            (None, me) => Some(me),
            (Some((side, rest)), Layout::Split { a, b, .. }) => if *side { b } else { a }.node_mut(rest),
            _ => None,
        }
    }
}

/// A pane besides the main one.
pub struct Helper {
    pub id: PaneId,
    pub view: Entity<TerminalView>,
    /// Folder it's in (saved for restore; refreshed while polling).
    pub cwd: Option<PathBuf>,
    /// Program running in it ("npm"), None at the shell prompt (refreshed while polling).
    pub program: Option<String>,
    /// That program's command line ("npm run dev"), its header title.
    pub command: Option<String>,
    pub osc_title: Option<String>,
    _sub: Subscription,
}

pub struct Panes {
    pub helpers: Vec<Helper>,
    pub layout: Layout,
    /// One pane filling the tab.
    pub zoomed: Option<PaneId>,
    next: PaneId,
}

impl Default for Panes {
    fn default() -> Self {
        Panes { helpers: Vec::new(), layout: Layout::Leaf(MAIN), zoomed: None, next: 1 }
    }
}

impl Panes {
    pub fn count(&self) -> usize {
        1 + self.helpers.len()
    }

    pub fn multi(&self) -> bool {
        !self.helpers.is_empty()
    }

    pub fn helper(&self, id: PaneId) -> Option<&Helper> {
        self.helpers.iter().find(|h| h.id == id)
    }

    pub fn saved(&self) -> Option<SavedPanes> {
        self.multi().then(|| SavedPanes { layout: self.layout.clone(), cwds: self.helpers.iter().map(|h| (h.id, h.cwd.clone())).collect() })
    }
}

/// Panes as session.json keeps them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedPanes {
    pub layout: Layout,
    pub cwds: Vec<(PaneId, Option<PathBuf>)>,
}

/// Carried while a divider is dragged: which split it belongs to.
#[derive(Clone)]
pub struct PaneDrag {
    path: Vec<bool>,
}

/// Where each pane of the active tab was drawn last frame, for ⌘⌥ arrows.
pub type PaneBounds = Rc<RefCell<HashMap<PaneId, Bounds<Pixels>>>>;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

fn cached(view: &Entity<TerminalView>) -> gpui::AnyView {
    gpui::AnyView::from(view.clone()).cached(gpui::StyleRefinement::default().size_full())
}

/// What a pane's header and the hover card call it, and the program for its icon.
pub fn pane_label(t: &Tab, id: PaneId) -> (String, Option<String>) {
    if id == MAIN {
        return (t.title(), t.program.clone());
    }
    let Some(h) = t.panes.helper(id) else { return (String::new(), None) };
    let title = match (&h.command, &h.osc_title) {
        (Some(c), _) => c.clone(),
        (None, Some(o)) => o.clone(),
        (None, None) => h.cwd.as_deref().map(crate::groups::tilde).unwrap_or_else(|| "shell".into()),
    };
    (title, h.program.clone())
}

/// The icon in a pane's header: the tab's icon with its status badge for the main pane,
/// the running program's for a helper.
pub fn pane_icon(ws: &Workspace, t: &Tab, id: PaneId, size: f32) -> AnyElement {
    if id == MAIN {
        crate::sidebar::status_icon(t, ws.ui, ws.ui.bg_terminal, true, size).into_any_element()
    } else {
        let program = t.panes.helper(id).and_then(|h| h.program.clone());
        crate::app_icon::program_badge(program.as_deref(), ws.ui, size).into_any_element()
    }
}

/// The active tab's terminal area: one pane, or the tree of them with hairlines between.
/// Each pane takes file drops (`drop::pane`).
pub fn render_panes(ws: &Workspace, t: &Tab, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    if !t.panes.multi() {
        return crate::drop::pane(ws, MAIN, cached(&t.view), cx);
    }
    let focused = ws.focused_pane(window, cx);
    if let Some(z) = t.panes.zoomed {
        return leaf(ws, t, z, focused, false, cx);
    }
    ws.pane_bounds.borrow_mut().clear();
    node(ws, t, &t.panes.layout, &mut Vec::new(), focused, cx)
}

fn node(ws: &Workspace, t: &Tab, n: &Layout, path: &mut Vec<bool>, focused: PaneId, cx: &mut Context<Workspace>) -> AnyElement {
    let (down, ratio, a, b) = match n {
        Layout::Leaf(id) => return leaf(ws, t, *id, focused, true, cx),
        Layout::Split { down, ratio, a, b } => (*down, *ratio, a, b),
    };
    path.push(false);
    let ea = node(ws, t, a, path, focused, cx);
    path.pop();
    path.push(true);
    let eb = node(ws, t, b, path, focused, cx);
    path.pop();
    let part = |e: AnyElement, grow: f32| {
        let mut d = div().flex_basis(px(0.)).min_w_0().min_h_0().overflow_hidden().flex().child(e);
        d.style().flex_grow = Some(grow);
        d
    };
    let ui = ws.ui;
    let key: String = path.iter().map(|s| if *s { 'b' } else { 'a' }).collect();
    let here = path.clone();
    let drag = path.clone();
    let even = path.clone();
    let group = SharedString::from(format!("divider-{key}"));
    // The 1 pt line sits in the flow; its 7 pt grab area floats over both sides, drawn last
    // so the panes don't cover it.
    let line = if down { div().h(px(1.)).w_full() } else { div().w(px(1.)).h_full() }.flex_none().bg(ui.border);
    let grab = div()
        .id(SharedString::from(format!("grab-{key}")))
        .absolute()
        .group(group.clone())
        .cursor(if down { CursorStyle::ResizeUpDown } else { CursorStyle::ResizeLeftRight })
        .map(|d| if down { d.left_0().right_0().top(px(-3.)).h(px(7.)) } else { d.top_0().bottom_0().left(px(-3.)).w(px(7.)) })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_drag(PaneDrag { path: drag }, |_, _, _, cx| cx.new(|_| crate::rail::NoGhost))
        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
            if ev.click_count() == 2 {
                this.set_pane_ratio(&even, 0.5, cx);
                this.save();
            }
        }))
        .child(
            div()
                .absolute()
                .map(|d| if down { d.left_0().right_0().top(px(2.5)).h(px(2.)) } else { d.top_0().bottom_0().left(px(2.5)).w(px(2.)) })
                .group_hover(group, move |s| s.bg(ui.accent.opacity(0.7))),
        );
    let anchor = div().absolute().map(|d| if down { d.left_0().right_0().top(relative(ratio)).h(px(0.)) } else { d.top_0().bottom_0().left(relative(ratio)).w(px(0.)) }).child(grab);
    let min = ws.pane_min(down);
    div()
        .id(SharedString::from(format!("split-{key}")))
        .relative()
        .size_full()
        .flex()
        .when(down, |d| d.flex_col())
        .on_drag_move::<PaneDrag>(cx.listener(move |this, ev: &gpui::DragMoveEvent<PaneDrag>, _, cx| {
            if ev.drag(cx).path != here {
                return;
            }
            let (b, p) = (ev.bounds, ev.event.position);
            let (at, size) = if down { (p.y - b.origin.y, b.size.height) } else { (p.x - b.origin.x, b.size.width) };
            let size = f32::from(size).max(1.);
            let edge = (min / size).min(0.45);
            this.set_pane_ratio(&here, (f32::from(at) / size).clamp(edge, 1. - edge), cx);
        }))
        .on_drop(cx.listener(|this, _: &PaneDrag, _, _| this.save()))
        .child(part(ea, ratio))
        .child(line)
        .child(part(eb, 1. - ratio))
        .child(anchor)
        .into_any_element()
}

fn leaf(ws: &Workspace, t: &Tab, id: PaneId, focused: PaneId, header: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(view) = ws.pane_view(ws.active, id) else { return div().into_any_element() };
    let bounds = ws.pane_bounds.clone();
    let is_focused = id == focused;
    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        // Clicking into a pane moves focus there; draw the headers again.
        .capture_any_mouse_down(cx.listener(|_, _, _, cx| cx.notify()))
        .child(canvas(move |b, _, _| { bounds.borrow_mut().insert(id, b); }, |_, _, _, _| {}).absolute().inset_0())
        .when(header, |d| d.child(pane_header(ws, t, id, is_focused, cx)))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .bg(ws.term_bg())
                .child(div().size_full().when(header && !is_focused, |d| d.opacity(0.72)).child(crate::drop::pane(ws, id, cached(&view), cx))),
        )
        .into_any_element()
}

fn pane_header(ws: &Workspace, t: &Tab, id: PaneId, focused: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let (title, _) = pane_label(t, id);
    let group = SharedString::from(format!("pane-head-{id}"));
    let tab = ws.active;
    let button = |key: &'static str, glyph: &'static str, tip: String| {
        div()
            .id(SharedString::from(format!("pane-{key}-{id}")))
            .size(px(18.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(ui.text_tertiary)
            .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
            .tooltip(crate::tooltip::text(ui, tip))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(glyph)
    };
    let keys = &ws.config.keys;
    div()
        .id(SharedString::from(format!("pane-head-{id}")))
        .group(group.clone())
        .h(px(HEADER_H))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .bg(ws.term_bg())
        .border_b_1()
        .border_color(ui.border)
        .child(pane_icon(ws, t, id, 14.))
        .child(div().flex_1().min_w_0().truncate().text_size(px(11.5)).text_color(if focused { ui.text_primary } else { ui.text_tertiary }).child(title))
        .child(
            div()
                .flex()
                .gap(px(2.))
                .invisible()
                .group_hover(group, |s| s.visible())
                .child(button("zoom", "⤢", format!("Zoom pane  {}", crate::keymap::display_for(&crate::actions::ZoomPane, keys))).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.focus_pane(tab, id, window, cx);
                        this.toggle_zoom(window, cx);
                    },
                )))
                .child(button("close", "×", "Close pane  ⌘W".into()).on_click(cx.listener(move |this, _, window, cx| this.close_pane(tab, id, window, cx)))),
        )
        .on_click(cx.listener(move |this, _, window, cx| this.focus_pane(tab, id, window, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
            this.focus_pane(tab, id, window, cx);
            this.open_overlay(crate::overlay::Overlay::PaneMenu { tab, pane: id, at: ev.position }, window, cx);
        }))
        .into_any_element()
}

/// The pane in direction `dir` from `from`: the nearest one past that edge that lines up with it.
fn neighbor(bounds: &HashMap<PaneId, Bounds<Pixels>>, from: PaneId, dir: Dir) -> Option<PaneId> {
    let me = bounds.get(&from)?;
    let f = |p: Pixels| f32::from(p);
    let (l, r, t, b) = (f(me.left()), f(me.right()), f(me.top()), f(me.bottom()));
    bounds
        .iter()
        .filter(|(id, _)| **id != from)
        .filter_map(|(id, o)| {
            let (ol, or, ot, ob) = (f(o.left()), f(o.right()), f(o.top()), f(o.bottom()));
            let (gap, overlap, center) = match dir {
                Dir::Left => (l - or, ob.min(b) - ot.max(t), ((ot + ob) - (t + b)).abs()),
                Dir::Right => (ol - r, ob.min(b) - ot.max(t), ((ot + ob) - (t + b)).abs()),
                Dir::Up => (t - ob, or.min(r) - ol.max(l), ((ol + or) - (l + r)).abs()),
                Dir::Down => (ot - b, or.min(r) - ol.max(l), ((ol + or) - (l + r)).abs()),
            };
            (gap > -3. && overlap > 0.).then_some((*id, gap, center))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|(id, ..)| id)
}

impl Workspace {
    pub fn pane_view(&self, ix: usize, id: PaneId) -> Option<Entity<TerminalView>> {
        let t = self.tabs.get(ix)?;
        if id == MAIN {
            Some(t.view.clone())
        } else {
            t.panes.helper(id).map(|h| h.view.clone())
        }
    }

    /// The active tab's focused pane (the main one when none has focus).
    pub fn focused_pane(&self, window: &Window, cx: &gpui::App) -> PaneId {
        let Some(t) = self.tabs.get(self.active) else { return MAIN };
        t.panes.helpers.iter().find(|h| h.view.read(cx).focus_handle(cx).is_focused(window)).map(|h| h.id).unwrap_or(MAIN)
    }

    /// The view that has focus in the active tab, else its main one.
    pub fn focused_view(&self, window: &Window, cx: &gpui::App) -> Option<Entity<TerminalView>> {
        self.pane_view(self.active, self.focused_pane(window, cx))
    }

    pub fn focus_pane(&mut self, ix: usize, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(v) = self.pane_view(ix, id) {
            let h = v.read(cx).focus_handle(cx);
            window.focus(&h);
        }
        cx.notify();
    }

    /// Smallest size, in px, either side of a divider may take.
    fn pane_min(&self, down: bool) -> f32 {
        let s = self.terminal_style();
        let fs = f32::from(s.font_size);
        if down {
            MIN_ROWS * fs * s.line_height + HEADER_H + f32::from(s.padding) * 2.
        } else {
            MIN_COLS * fs * 0.6 + f32::from(s.padding_x) * 2.
        }
    }

    pub fn set_pane_ratio(&mut self, path: &[bool], ratio: f32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        if let Some(Layout::Split { ratio: r, .. }) = t.panes.layout.node_mut(path) {
            if (*r - ratio).abs() > f32::EPSILON {
                *r = ratio;
                cx.notify();
            }
        }
    }

    /// ⌘D / ⌘⇧D: halve the focused pane (the main one for a tab in the background).
    /// At four panes nothing happens.
    pub fn open_split(&mut self, ix: usize, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(ix) else { return };
        if t.panes.count() >= MAX_PANES {
            return;
        }
        let target = if ix == self.active { self.focused_pane(window, cx) } else { MAIN };
        let cwd = self.pane_view(ix, target).and_then(|v| v.read(cx).terminal.cwd()).or_else(|| t.cwd.clone());
        let id = self.add_helper(ix, None, cwd, window, cx);
        let t = &mut self.tabs[ix];
        t.panes.layout.split_leaf(target, id, down);
        t.panes.zoomed = None;
        if ix == self.active {
            self.focus_pane(ix, id, window, cx);
        }
        self.save();
        cx.notify();
    }

    /// Start a helper's shell in tab `ix` (not placed in the layout yet).
    fn add_helper(&mut self, ix: usize, id: Option<PaneId>, cwd: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> PaneId {
        let t = &mut self.tabs[ix];
        let id = id.unwrap_or(t.panes.next);
        t.panes.next = t.panes.next.max(id + 1);
        let cwd = cwd.filter(|c| c.is_dir()).or_else(|| t.cwd.clone());
        // Its own hook id, so an agent in a helper doesn't take over the tab's card; a pane
        // moved to a tab of its own keeps it as that tab's id.
        let hook_id = format!("{}-p{id}", t.id);
        let view = self.spawn_view(cwd.clone(), hook_id, window, cx);
        let sub = cx.subscribe_in(&view, window, |this: &mut Self, view, ev: &TerminalEvent, window, cx| {
            let Some((ix, id)) = this.tabs.iter().enumerate().find_map(|(i, t)| t.panes.helpers.iter().find(|h| &h.view == view).map(|h| (i, h.id))) else {
                return;
            };
            match ev {
                TerminalEvent::Exited => this.close_pane(ix, id, window, cx),
                TerminalEvent::OpenLink(link) => this.open_link(ix, link, cx),
                TerminalEvent::ContextMenu { at, link, has_selection } => {
                    let o = crate::overlay::Overlay::TerminalMenu { tab: ix, pane: id, at: *at, link: link.clone(), has_selection: *has_selection, apps: crate::overlay::file_apps(link) };
                    this.open_overlay(o, window, cx);
                }
                TerminalEvent::Notify(n) => this.program_notice(ix, n, false, window, cx),
                TerminalEvent::TitleChanged(s) => {
                    let (_, clean) = crate::title::split(s);
                    if let Some(h) = this.tabs[ix].panes.helpers.iter_mut().find(|h| h.id == id) {
                        h.osc_title = (!clean.is_empty()).then(|| clean.to_string());
                    }
                    cx.notify();
                }
                TerminalEvent::Bell => {}
            }
        });
        self.tabs[ix].panes.helpers.push(Helper { id, view, cwd, program: None, command: None, osc_title: None, _sub: sub });
        id
    }

    /// Bring back a tab's panes from session.json.
    pub fn restore_panes(&mut self, ix: usize, saved: SavedPanes, window: &mut Window, cx: &mut Context<Self>) {
        let mut ids: Vec<PaneId> = saved.cwds.iter().map(|(id, _)| *id).filter(|id| *id != MAIN).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(MAX_PANES - 1);
        let mut want = ids.clone();
        want.push(MAIN);
        want.sort_unstable();
        let mut leaves = saved.layout.leaves();
        leaves.sort_unstable();
        // A layout that doesn't match its panes: stack them side by side instead.
        let layout = if leaves == want {
            saved.layout
        } else {
            ids.iter().fold(Layout::Leaf(MAIN), |acc, id| Layout::Split { down: false, ratio: 0.5, a: Box::new(acc), b: Box::new(Layout::Leaf(*id)) })
        };
        for id in ids {
            let cwd = saved.cwds.iter().find(|(i, _)| *i == id).and_then(|(_, c)| c.clone());
            self.add_helper(ix, Some(id), cwd, window, cx);
        }
        self.tabs[ix].panes.layout = layout;
    }

    /// ⌘W in a tab with panes, a header's ×, or a helper's shell exiting: close that pane.
    /// The main pane hands the tab to the next pane, unless a program is running in it.
    pub fn close_pane(&mut self, ix: usize, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(ix) else { return };
        if !t.panes.multi() {
            if id == MAIN {
                self.request_close(ix, window, cx);
            }
            return;
        }
        if id == MAIN {
            if let Some(p) = self.foreground_program(ix, cx) {
                self.notice = Some((format!("{p} is running in this pane. Quit it first, or close the tab."), std::time::Instant::now()));
                self.notice_undo = false;
                self.notice_brief = false;
                cx.notify();
                return;
            }
        }
        let was_focused = ix == self.active && self.focused_pane(window, cx) == id;
        let t = &mut self.tabs[ix];
        let order = t.panes.layout.leaves();
        let pos = order.iter().position(|p| *p == id).unwrap_or(0);
        let layout = std::mem::replace(&mut t.panes.layout, Layout::Leaf(MAIN));
        t.panes.layout = layout.without(id).unwrap_or(Layout::Leaf(MAIN));
        if t.panes.zoomed == Some(id) {
            t.panes.zoomed = None;
        }
        let mut next = order.get(pos.saturating_sub(1)).copied().filter(|p| *p != id).or_else(|| order.get(pos + 1).copied()).unwrap_or(MAIN);
        if id == MAIN {
            // The first remaining pane becomes the tab's main one.
            let heir = t.panes.layout.leaves()[0];
            let h = t.panes.helpers.remove(t.panes.helpers.iter().position(|h| h.id == heir).unwrap_or(0));
            t.panes.layout.rename(heir, MAIN);
            if t.panes.zoomed == Some(heir) {
                t.panes.zoomed = Some(MAIN);
            }
            if next == heir {
                next = MAIN;
            }
            t.osc_title = h.osc_title.clone();
            t.cwd = h.cwd.clone().or(t.cwd.take());
            self.promote_view(ix, h.view, window, cx);
        } else {
            t.panes.helpers.retain(|h| h.id != id);
        }
        if was_focused || ix == self.active && id == MAIN {
            self.focus_pane(ix, next, window, cx);
        }
        self.pane_bounds.borrow_mut().remove(&id);
        self.save();
        cx.notify();
    }

    /// Move a helper into a tab of its own, right after this one.
    pub fn pane_to_new_tab(&mut self, ix: usize, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(ix) else { return };
        let Some(pos) = t.panes.helpers.iter().position(|h| h.id == id) else { return };
        let h = t.panes.helpers.remove(pos);
        let layout = std::mem::replace(&mut t.panes.layout, Layout::Leaf(MAIN));
        t.panes.layout = layout.without(id).unwrap_or(Layout::Leaf(MAIN));
        if t.panes.zoomed == Some(id) {
            t.panes.zoomed = None;
        }
        let hook_id = format!("{}-p{id}", t.id);
        self.adopt_view(hook_id, h.view, h.cwd, window, cx);
    }

    /// ⌘⇧↵: the focused pane fills the tab, or the panes come back.
    pub fn toggle_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.focused_pane(window, cx);
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        if !t.panes.multi() {
            return;
        }
        t.panes.zoomed = if t.panes.zoomed.is_some() { None } else { Some(focused) };
        let id = t.panes.zoomed.unwrap_or(focused);
        self.focus_pane(self.active, id, window, cx);
    }

    /// ⌘⌥ arrows: focus the pane on that side (unzooming first).
    pub fn focus_dir(&mut self, dir: Dir, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.focused_pane(window, cx);
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        if !t.panes.multi() {
            return;
        }
        let zoomed = t.panes.zoomed.take().is_some();
        let to = neighbor(&self.pane_bounds.borrow(), from, dir);
        match to {
            Some(id) => self.focus_pane(self.active, id, window, cx),
            None if zoomed => cx.notify(),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> Layout {
        let mut l = Layout::Leaf(MAIN);
        assert!(l.split_leaf(MAIN, 1, false));
        assert!(l.split_leaf(1, 2, true));
        l
    }

    #[test]
    fn split_and_remove() {
        let l = three();
        assert_eq!(l.leaves(), vec![0, 1, 2]);
        assert!(!three().split_leaf(9, 3, false));
        // Removing a pane gives its space to its sibling.
        let r = l.clone().without(1).unwrap();
        assert_eq!(r.leaves(), vec![0, 2]);
        assert!(matches!(&r, Layout::Split { down: false, b, .. } if **b == Layout::Leaf(2)));
        let r = l.without(0).unwrap();
        assert!(matches!(&r, Layout::Split { down: true, .. }));
        assert_eq!(Layout::Leaf(MAIN).without(MAIN), None);
    }

    #[test]
    fn rename_and_ratio() {
        let mut l = three();
        l.rename(2, 0);
        assert_eq!(l.leaves(), vec![0, 1, 0]);
        let mut l = three();
        if let Some(Layout::Split { ratio, .. }) = l.node_mut(&[true]) {
            *ratio = 0.3;
        }
        assert!(matches!(l.node_mut(&[true]), Some(Layout::Split { ratio, .. }) if (*ratio - 0.3).abs() < 1e-6));
        assert!(l.node_mut(&[false, true]).is_none());
    }

    #[test]
    fn saved_layout_round_trips() {
        let s = SavedPanes { layout: three(), cwds: vec![(1, None), (2, Some("/tmp".into()))] };
        let j = serde_json::to_string(&s).unwrap();
        let back: SavedPanes = serde_json::from_str(&j).unwrap();
        assert_eq!(back.layout, s.layout);
    }

    #[test]
    fn neighbors_by_direction() {
        use gpui::{point, size};
        let b = |x: f32, y: f32, w: f32, h: f32| Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) };
        // Main on the left; 1 above 2 on the right.
        let m: HashMap<PaneId, Bounds<Pixels>> = [(0, b(0., 0., 100., 200.)), (1, b(101., 0., 100., 100.)), (2, b(101., 101., 100., 99.))].into();
        assert_eq!(neighbor(&m, 0, Dir::Right), Some(1));
        assert_eq!(neighbor(&m, 2, Dir::Left), Some(0));
        assert_eq!(neighbor(&m, 1, Dir::Down), Some(2));
        assert_eq!(neighbor(&m, 2, Dir::Up), Some(1));
        assert_eq!(neighbor(&m, 0, Dir::Left), None);
    }
}
