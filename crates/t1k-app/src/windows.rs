//! More than one window (canvas v3-aq). Each window is a `Workspace` with its own sidebar of
//! tabs; what belongs to the app rather than a window lives here: the agents' hook socket
//! (one per process, each message handed to the window that holds its tab), session.json
//! (a list of windows), the Needs-you queue across windows (⌘J, the Dock badge), plan usage,
//! clicked banners, and the order windows are asked in at quit.
//!
//! A plain mutex rather than a GPUI global: windows report in from inside their own updates,
//! where reading another entity is fine but re-entering themselves is not, so nothing here
//! calls back into a workspace while the lock is held.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use gpui::{prelude::*, px, size, App, Bounds, EntityId, WindowBounds, WindowHandle, WindowOptions};
use t1k_agent::agents::AgentKind;
use t1k_agent::server::Envelope;
use t1k_agent::statusline::Usage;

use crate::session::{self, Session};
use crate::split::PaneId;
use crate::workspace::Workspace;

/// A tab (or one of its panes) waiting on a person, and for how long.
#[derive(Clone, Debug)]
pub struct Wait {
    pub tab: String,
    pub pane: PaneId,
    pub secs: u64,
}

struct Win {
    id: EntityId,
    handle: WindowHandle<Workspace>,
    session: Option<Session>,
    name: String,
    tabs: usize,
    waiting: Vec<Wait>,
}

/// Another window, as menus and the sidebar describe it.
#[derive(Clone)]
pub struct Other {
    pub id: EntityId,
    pub handle: WindowHandle<Workspace>,
    pub name: String,
    pub tabs: usize,
    pub waiting: usize,
}

#[derive(Default)]
struct Hub {
    rx: Option<Receiver<Envelope>>,
    socket: Option<PathBuf>,
    socket_tried: bool,
    /// Messages no window has claimed yet (its tab may be mid-move between windows).
    buf: Vec<(Instant, Envelope)>,
    clicked: Vec<(Instant, String)>,
    snoozed: Vec<(Instant, String, u64)>,
    wins: Vec<Win>,
    usage: HashMap<AgentKind, Usage>,
    usage_rev: u64,
    /// Quitting: windows that have been asked and are done.
    asked: HashSet<EntityId>,
    quitting: bool,
    /// Only this window is closing (its close button, with other windows open).
    closing: Option<EntityId>,
}

static HUB: LazyLock<Mutex<Hub>> = LazyLock::new(|| Mutex::new(Hub::default()));
/// Unclaimed messages are dropped after this long (the tab is gone).
const KEEP: Duration = Duration::from_secs(10);

fn hub() -> std::sync::MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|e| e.into_inner())
}

/// The hook socket, started with the first window. None in the demo or if it can't listen.
pub fn socket() -> Option<PathBuf> {
    let mut h = hub();
    if !h.socket_tried {
        h.socket_tried = true;
        if !crate::demo::on() {
            if let Some(dir) = session::support_dir() {
                t1k_agent::server::remove_stale(&dir);
                let path = t1k_agent::server::socket_path(&dir);
                let (tx, rx) = channel();
                match t1k_agent::server::serve(&path, tx, || {}) {
                    Ok(()) => {
                        h.rx = Some(rx);
                        h.socket = Some(path);
                    }
                    Err(e) => log::error!("agent socket {}: {e}", path.display()),
                }
            }
        }
    }
    h.socket.clone()
}

pub fn register(id: EntityId, handle: WindowHandle<Workspace>) {
    hub().wins.push(Win { id, handle, session: None, name: String::new(), tabs: 0, waiting: Vec::new() });
}

/// A window went away. While others stay open it leaves the saved session too; the last
/// one (and every window at quit) stays saved, to come back next launch.
pub fn remove(id: EntityId) {
    let mut h = hub();
    if h.quitting || h.wins.len() <= 1 {
        return;
    }
    h.wins.retain(|w| w.id != id);
    write(&h);
}

/// Quit is under way: windows closing now are not being closed by the person.
pub fn set_quitting() {
    hub().quitting = true;
}

pub fn count() -> usize {
    hub().wins.len()
}

/// Keep this window's state and write session.json: the first window's session, with the
/// others inside it.
pub fn store(id: EntityId, s: Session) {
    let mut h = hub();
    if let Some(w) = h.wins.iter_mut().find(|w| w.id == id) {
        w.session = Some(s);
    }
    write(&h);
}

fn write(h: &Hub) {
    let mut all = h.wins.iter().filter_map(|w| w.session.clone());
    let Some(mut first) = all.next() else { return };
    first.windows = all.collect();
    session::save(&first);
}

/// What this window shows of itself to the others: its name, tab count and who waits.
pub fn publish(id: EntityId, name: String, tabs: usize, waiting: Vec<Wait>) {
    let mut h = hub();
    if let Some(w) = h.wins.iter_mut().find(|w| w.id == id) {
        w.name = name;
        w.tabs = tabs;
        w.waiting = waiting;
    }
}

/// Every other window, in the order they opened.
pub fn others(id: EntityId) -> Vec<Other> {
    hub().wins.iter().filter(|w| w.id != id).map(|w| Other { id: w.id, handle: w.handle, name: w.name.clone(), tabs: w.tabs, waiting: w.waiting.len() }).collect()
}

/// Tabs waiting on a person in every window (the Dock badge).
pub fn total_waiting() -> usize {
    hub().wins.iter().map(|w| w.waiting.len()).sum()
}

/// The Needs-you queue across windows, longest wait first.
pub fn queue() -> Vec<(EntityId, WindowHandle<Workspace>, Wait)> {
    let h = hub();
    let mut v: Vec<_> = h.wins.iter().flat_map(|w| w.waiting.iter().map(move |x| (w.id, w.handle, x.clone()))).collect();
    v.sort_by_key(|(_, _, w)| std::cmp::Reverse(w.secs));
    v
}

/// Hook messages for tabs this window holds (`owns` answers for a hook id).
pub fn take_envelopes(owns: impl Fn(&str) -> bool) -> Vec<Envelope> {
    let mut h = hub();
    let now = Instant::now();
    let fresh: Vec<Envelope> = h.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
    h.buf.extend(fresh.into_iter().map(|e| (now, e)));
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut h.buf).into_iter().partition(|(_, e)| owns(&e.tab_id));
    h.buf = rest.into_iter().filter(|(at, _)| at.elapsed() < KEEP).collect();
    mine.into_iter().map(|(_, e)| e).collect()
}

/// Banners clicked for tabs this window holds.
pub fn take_clicked(owns: impl Fn(&str) -> bool) -> Vec<String> {
    let mut h = hub();
    let now = Instant::now();
    h.clicked.extend(crate::platform::take_clicked().into_iter().map(|id| (now, id)));
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut h.clicked).into_iter().partition(|(_, id)| owns(id));
    h.clicked = rest.into_iter().filter(|(at, _)| at.elapsed() < KEEP).collect();
    mine.into_iter().map(|(_, id)| id).collect()
}

/// Reminder banners snoozed for tabs this window holds: (tab, minutes).
pub fn take_snoozed(owns: impl Fn(&str) -> bool) -> Vec<(String, u64)> {
    let mut h = hub();
    let now = Instant::now();
    h.snoozed.extend(crate::platform::take_snoozed().into_iter().map(|(id, m)| (now, id, m)));
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut h.snoozed).into_iter().partition(|(_, id, _)| owns(id));
    h.snoozed = rest.into_iter().filter(|(at, _, _)| at.elapsed() < KEEP).collect();
    mine.into_iter().map(|(_, id, m)| (id, m)).collect()
}

/// Plan usage limits are per account: one window hears them, every window shows them.
pub fn set_usage(kind: AgentKind, usage: Usage) {
    let mut h = hub();
    h.usage.insert(kind, usage);
    h.usage_rev += 1;
}

/// The shared usage if it's newer than `rev`.
pub fn usage_since(rev: u64) -> Option<(u64, HashMap<AgentKind, Usage>)> {
    let h = hub();
    (h.usage_rev != rev).then(|| (h.usage_rev, h.usage.clone()))
}

/// The saved frame if it still lands on a connected display (a monitor may be gone).
fn restored_bounds(s: &Session, cx: &App) -> Option<WindowBounds> {
    use session::WindowState;
    let w = s.window?;
    if w.w < 200. || w.h < 150. {
        return None;
    }
    let b = Bounds::new(gpui::point(px(w.x), px(w.y)), size(px(w.w), px(w.h)));
    // At least a 120×60 corner of it must be on some display.
    let visible = cx.displays().iter().any(|d| {
        let s = d.bounds();
        let ix = f32::from(b.right().min(s.right())) - f32::from(b.left().max(s.left()));
        let iy = f32::from(b.bottom().min(s.bottom())) - f32::from(b.top().max(s.top()));
        ix >= 120. && iy >= 60.
    });
    visible.then_some(match w.state {
        WindowState::Windowed => WindowBounds::Windowed(b),
        WindowState::Maximized => WindowBounds::Maximized(b),
        WindowState::Fullscreen => WindowBounds::Fullscreen(b),
    })
}

/// Open a window on `saved` (its tabs, groups and frame). `at`: its top-left corner, for a
/// tab dragged out; else the saved frame, else a little down and right of the last window.
pub fn open(saved: Session, at: Option<gpui::Point<gpui::Pixels>>, focus: bool, cx: &mut App) -> Option<WindowHandle<Workspace>> {
    let default = size(px(1100.), px(720.));
    let bounds = if crate::demo::on() {
        WindowBounds::Windowed(Bounds::centered(None, size(px(crate::demo::WINDOW.0), px(crate::demo::WINDOW.1)), cx))
    } else if let Some(p) = at {
        WindowBounds::Windowed(Bounds::new(p, default))
    } else if let Some(b) = restored_bounds(&saved, cx) {
        b
    } else {
        // Cascade from the frontmost window, so a new one doesn't hide it.
        let from = cx.active_window().and_then(|w| w.update(cx, |_, window, _| window.bounds()).ok());
        match from {
            Some(b) => WindowBounds::Windowed(Bounds::new(gpui::point(b.origin.x + px(28.), b.origin.y + px(28.)), b.size)),
            None => WindowBounds::Windowed(Bounds::centered(None, default, cx)),
        }
    };
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(bounds),
                titlebar: Some(gpui::TitlebarOptions { title: Some("T1K".into()), appears_transparent: true, traffic_light_position: Some(gpui::point(px(12.), px(15.))) }),
                focus,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Workspace::new(saved, window, cx)),
        )
        .map_err(|e| log::error!("open window: {e}"))
        .ok()?;
    Some(handle)
}

/// Quit, or close one window: ask each window with something running, one after another.
/// `from`: the window that just finished being asked (None to start). When none is left,
/// T1K quits; a window closing by itself just closes.
pub fn quit_next(from: Option<EntityId>, cx: &mut App) {
    let (closing, next) = {
        let mut h = hub();
        if let Some(id) = from {
            h.asked.insert(id);
        }
        let closing = h.closing.take().filter(|c| Some(*c) == from).and_then(|c| h.wins.iter().find(|w| w.id == c).map(|w| w.handle));
        if closing.is_some() {
            h.asked.clear();
        }
        let next = h.wins.iter().find(|w| !h.asked.contains(&w.id)).map(|w| (w.id, w.handle));
        (closing, next)
    };
    if let Some(w) = closing {
        let _ = w.update(cx, |_, window, _| window.remove_window());
        return;
    }
    match next {
        None => {
            hub().asked.clear();
            set_quitting();
            crate::platform::quit(cx);
        }
        Some((id, w)) => {
            // Its turn: with nothing running it passes straight on (deferred, from inside).
            if w.update(cx, |ws, window, cx| ws.ask_quit(window, cx)).is_err() {
                quit_next(Some(id), cx);
            }
        }
    }
}

/// Cancel: nobody has been asked.
pub fn quit_cancelled() {
    let mut h = hub();
    h.asked.clear();
    h.closing = None;
}

/// This window's close button, with other windows open: only it closes.
pub fn closing_only(id: EntityId) {
    hub().closing = Some(id);
}

impl Workspace {
    /// A fresh session for a new window: this window's group names, colors and icons, none of
    /// its tabs.
    fn window_seed(&self) -> Session {
        let mut s = self.session();
        s.tabs.clear();
        s.active = 0;
        s.recent.clear();
        s.window = None;
        s.windows.clear();
        s.welcomed = true;
        s
    }

    /// ⌘N: another window with one shell tab in this tab's folder.
    pub fn new_window(&mut self, cx: &mut Context<Self>) {
        let mut seed = self.window_seed();
        let cwd = self.tabs.get(self.active).and_then(|t| t.cwd.clone());
        seed.tabs.push(session::SavedTab { cwd, ..Default::default() });
        cx.defer(move |cx| {
            open(seed, None, true, cx);
        });
    }

    /// Take a tab out of this window, still running (its terminal, scrollback and session
    /// go with it). The window closes when its last tab leaves.
    fn detach_tab(&mut self, ix: usize, window: &mut gpui::Window, cx: &mut Context<Self>) -> Option<(crate::workspace::Tab, Vec<(String, String)>)> {
        if ix >= self.tabs.len() {
            return None;
        }
        self.stop_broadcast(false, cx);
        self.hide_card(cx);
        let next = if ix == self.active { self.after_close_target(ix) } else { Some(self.active) };
        let tab = self.tabs.remove(ix);
        // Helpers promoted to main keep reporting under their old hook id.
        let aliases: Vec<(String, String)> = self.hook_alias.iter().filter(|(_, t)| **t == tab.id).map(|(k, v)| (k.clone(), v.clone())).collect();
        self.hook_alias.retain(|_, t| *t != tab.id);
        if self.tabs.is_empty() {
            window.remove_window();
        } else {
            let next = next.map(|n| if n > ix { n - 1 } else { n }).unwrap_or(0);
            self.activate(next.min(self.tabs.len() - 1), window, cx);
            self.save();
        }
        cx.notify();
        Some((tab, aliases))
    }

    /// A tab arriving from another window: it joins its project group here and is selected.
    pub fn attach_tab(&mut self, mut tab: crate::workspace::Tab, aliases: Vec<(String, String)>, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.adopt_tab_subscriptions(&mut tab, window, cx);
        self.resubscribe_helpers(&mut tab, window, cx);
        self.hook_alias.extend(aliases);
        // It repaints in this window, with this window's look.
        let views = std::iter::once(tab.view.clone()).chain(tab.panes.helpers.iter().map(|h| h.view.clone())).collect::<Vec<_>>();
        self.tabs.push(tab);
        for v in views {
            v.update(cx, |_, cx| cx.notify());
        }
        let ix = self.tabs.len() - 1;
        self.activate(ix, window, cx);
        self.save();
        window.activate_window();
        cx.notify();
    }

    /// Move a tab to another window, or (`to` None) to a new one, at `at` if given.
    pub fn move_tab_to_window(&mut self, ix: usize, to: Option<WindowHandle<Workspace>>, at: Option<gpui::Point<gpui::Pixels>>, window: &mut gpui::Window, cx: &mut Context<Self>) {
        // A window's only tab in a new window is the same window.
        if to.is_none() && self.tabs.len() < 2 {
            return;
        }
        let seed = self.window_seed();
        let Some((tab, aliases)) = self.detach_tab(ix, window, cx) else { return };
        // Deferred: the target may be a window that doesn't exist yet, and this one is mid-update.
        cx.defer(move |cx| {
            let target = to.or_else(|| {
                let mut seed = seed;
                seed.adopt = true;
                open(seed, at, true, cx)
            });
            if let Some(w) = target {
                let _ = w.update(cx, |ws, window, cx| ws.attach_tab(tab, aliases, window, cx));
            }
        });
    }

    /// A row dragged out of the window and dropped at `screen`: into the T1K window under
    /// that point, else a new window there.
    pub fn drop_tab_outside(&mut self, id: &str, screen: gpui::Point<gpui::Pixels>, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else { return };
        let me = self.hub_id;
        cx.defer_in(window, move |this, window, cx| {
            let under = others(me).into_iter().find(|o| o.handle.update(cx, |_, w, _| w.bounds().contains(&screen)).unwrap_or(false));
            let at = gpui::point(screen.x - px(120.), screen.y - px(20.));
            this.move_tab_to_window(ix, under.map(|o| o.handle), Some(at), window, cx);
        });
    }

    /// ⌘J across windows: the next waiting tab after where you are, longest wait first;
    /// in another window, that window comes forward. False when nothing waits anywhere.
    pub fn jump_needs_everywhere(&mut self, dir: isize, window: &mut gpui::Window, cx: &mut Context<Self>) -> bool {
        let q = queue();
        if q.is_empty() {
            return false;
        }
        let n = q.len() as isize;
        let here_tab = self.tabs.get(self.active).map(|t| t.id.clone()).unwrap_or_default();
        let here_pane = self.focused_pane(window, cx);
        let at = q.iter().position(|(w, _, x)| *w == self.hub_id && x.tab == here_tab && x.pane == here_pane).or_else(|| {
            let mine: Vec<usize> = q.iter().enumerate().filter(|(_, (w, _, x))| *w == self.hub_id && x.tab == here_tab).map(|(i, _)| i).collect();
            (mine.len() == 1).then(|| mine[0])
        });
        let (win, handle, wait) = match at {
            Some(p) => q[((p as isize + dir).rem_euclid(n)) as usize].clone(),
            None if dir > 0 => q[0].clone(),
            None => q[(n - 1) as usize].clone(),
        };
        if win == self.hub_id {
            if let Some(ix) = self.tabs.iter().position(|t| t.id == wait.tab) {
                self.jump_to_pane(ix, wait.pane, window, cx);
            }
        } else {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |ws, window, cx| {
                    window.activate_window();
                    if let Some(ix) = ws.tabs.iter().position(|t| t.id == wait.tab) {
                        ws.jump_to_pane(ix, wait.pane, window, cx);
                    }
                });
            });
        }
        true
    }
}
