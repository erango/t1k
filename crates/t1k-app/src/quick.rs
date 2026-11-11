//! Quick terminal (canvas v3-ay): a panel over whatever app you're in, on a global shortcut.
//! It opens on the tab that has waited longest for you, so you answer in place (⌘J: the next
//! one); with nothing waiting it is a quick shell of its own. The same shortcut puts it away.
//!
//! The panel shows the tab's real terminal (the same view and pty), so while it's there the
//! tab's pane in its window says "Shown in the quick terminal". Hiding never closes anything.
//! Esc doesn't dismiss it: Esc belongs to the programs.

use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Global, MouseButton, Subscription, Window, WindowBounds, WindowHandle,
    WindowKind, WindowOptions,
};
use t1k_agent::Status;
use t1k_terminal::{TerminalEvent, TerminalView};

use crate::config::QuickCfg;
use crate::split::{PaneId, MAIN};
use crate::workspace::Workspace;

/// What the shortcut's registration came to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Hotkey {
    #[default]
    Off,
    On,
    /// macOS refused it: another app has it.
    Taken,
    /// Not a shortcut we can register (no modifier, or an unknown key).
    Invalid,
}

#[derive(Clone)]
enum Target {
    /// The quick shell.
    Shell,
    /// A tab's pane, in a window.
    Tab { window: WindowHandle<Workspace>, tab: String, pane: PaneId },
}

#[derive(Default)]
pub struct Quick {
    cfg: QuickCfg,
    /// The shortcut registered now, and how that went.
    registered: Option<String>,
    pub hotkey: Hotkey,
    shell: Option<(Entity<TerminalView>, String, Subscription)>,
    shells: u32,
    panel: Option<WindowHandle<Panel>>,
    target: Option<Target>,
    /// The terminal the panel is showing (its window shows a placeholder meanwhile).
    shown: Option<Entity<TerminalView>>,
    pinned: bool,
    /// Sliding out: the panel goes in a moment; the shortcut waits for it.
    leaving: bool,
}

impl Global for Quick {}

/// Is this terminal in the quick panel right now? Its window must not draw it too.
pub fn shows(view: &Entity<TerminalView>, cx: &App) -> bool {
    cx.try_global::<Quick>().is_some_and(|q| q.shown.as_ref() == Some(view))
}

pub fn hotkey_state(cx: &App) -> Hotkey {
    cx.try_global::<Quick>().map(|q| q.hotkey).unwrap_or_default()
}

/// Follow the settings: register or release the shortcut. Cheap to call again.
pub fn configure(cfg: &QuickCfg, cx: &mut App) {
    if crate::demo::on() {
        return;
    }
    if !cx.has_global::<Quick>() {
        cx.set_global(Quick::default());
    }
    let q = cx.global_mut::<Quick>();
    let want = cfg.enabled.then(|| cfg.shortcut.clone());
    let changed = q.registered != want || (q.cfg.enabled != cfg.enabled);
    q.cfg = cfg.clone();
    if !changed {
        return;
    }
    q.registered = want.clone();
    q.hotkey = match want {
        None => {
            crate::platform::hotkey_unregister();
            Hotkey::Off
        }
        Some(s) if crate::platform::hotkey_parse(&s).is_none() => {
            crate::platform::hotkey_unregister();
            Hotkey::Invalid
        }
        Some(s) => {
            if crate::platform::hotkey_register(&s) {
                Hotkey::On
            } else {
                Hotkey::Taken
            }
        }
    };
    log::info!("quick terminal shortcut: {:?}", q.hotkey);
    if !cfg.enabled {
        hide(cx);
    }
}

/// Every T1K window repaints (a pane swaps between its terminal and the placeholder).
fn repaint_windows(cx: &mut App) {
    for w in cx.windows() {
        if let Some(w) = w.downcast::<Workspace>() {
            let _ = w.update(cx, |_, _, cx| cx.notify());
        }
    }
}

/// In 140 ms (ease-out), out in 100 ms (ease-in), 16 pt from its edge; none with Reduce
/// Motion.
const IN_MS: f32 = 140.;
const OUT_MS: f32 = 100.;

pub fn hide(cx: &mut App) {
    let Some(q) = cx.try_global::<Quick>() else { return };
    let Some(panel) = q.panel.filter(|_| !q.leaving) else { return };
    let animate = !crate::platform::reduce_motion();
    cx.global_mut::<Quick>().leaving = true;
    let _ = panel.update(cx, |p, _, cx| {
        p.leaving = Some(std::time::Instant::now());
        cx.notify();
    });
    let gone = move |cx: &mut App| {
        let q = cx.global_mut::<Quick>();
        q.panel = None;
        q.target = None;
        q.shown = None;
        q.leaving = false;
        let _ = panel.update(cx, |_, window, _| window.remove_window());
        repaint_windows(cx);
    };
    if animate {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(OUT_MS as u64)).await;
            let _ = cx.update(gone);
        })
        .detach();
    } else {
        gone(cx);
    }
}

/// The shortcut: show the panel, or put it away.
pub fn toggle(cx: &mut App) {
    if !cx.has_global::<Quick>() {
        return;
    }
    if cx.global::<Quick>().leaving {
        return;
    }
    if cx.global::<Quick>().panel.is_some() {
        hide(cx);
        return;
    }
    if !cx.global::<Quick>().cfg.enabled {
        return;
    }
    let target = pick(None, cx);
    show(target, true, cx);
}

/// Debug aid (`T1K_DEBUG_QUICK=needs|shell[,top|bottom|left|right][,s|l]`): show the panel
/// as the shortcut would, pinned, without taking the keyboard.
pub fn debug_show(spec: &str, cx: &mut App) {
    if !cx.has_global::<Quick>() {
        cx.set_global(Quick::default());
    }
    let q = cx.global_mut::<Quick>();
    q.cfg.enabled = true;
    q.pinned = true;
    for part in spec.split(',') {
        match part {
            "needs" | "shell" => q.cfg.shows = part.into(),
            "top" | "bottom" | "left" | "right" | "panel" => q.cfg.position = part.into(),
            "s" | "m" | "l" => q.cfg.size = part.into(),
            _ => {}
        }
    }
    q.cfg.screen = "main".into();
    let target = pick(None, cx);
    show(target, false, cx);
    // `confirm`: ⌘W's question in the quick shell.
    if spec.split(',').any(|p| p == "confirm") {
        if let Some(p) = cx.global::<Quick>().panel {
            let _ = p.update(cx, |p, _, cx| {
                p.confirm = true;
                cx.notify();
            });
        }
    }
}

/// A T1K window to borrow from (the look, a place to start the shell): the frontmost.
fn any_window(cx: &App) -> Option<WindowHandle<Workspace>> {
    cx.active_window().and_then(|w| w.downcast::<Workspace>()).or_else(|| cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()))
}

/// What to show: the longest-waiting tab (after `after`, for ⌘J), else the quick shell.
fn pick(after: Option<(&str, PaneId)>, cx: &App) -> Target {
    if cx.global::<Quick>().cfg.shows != "shell" {
        let q = crate::windows::queue();
        let at = after.and_then(|(tab, pane)| q.iter().position(|(_, _, w)| w.tab == tab && w.pane == pane));
        let next = match at {
            Some(p) if q.len() > 1 => q.get((p + 1) % q.len()),
            Some(_) => None,
            None => q.first(),
        };
        if let Some((_, window, w)) = next {
            return Target::Tab { window: *window, tab: w.tab.clone(), pane: w.pane };
        }
    }
    Target::Shell
}

/// The quick shell, started on first use in the frontmost tab's folder.
fn shell(cx: &mut App) -> Option<Entity<TerminalView>> {
    if let Some((v, _, _)) = &cx.global::<Quick>().shell {
        return Some(v.clone());
    }
    let window = any_window(cx)?;
    let n = cx.global::<Quick>().shells + 1;
    let id = format!("quick-{n}");
    let hook = id.clone();
    let view = window
        .update(cx, |ws, window, cx| {
            let cwd = ws.tabs.get(ws.active).and_then(|t| t.cwd.clone());
            ws.spawn_view(cwd, hook, window, cx)
        })
        .ok()?;
    // When its shell exits, the quick terminal is gone until next time.
    let sub = cx.subscribe(&view, |_, ev: &TerminalEvent, cx| {
        if matches!(ev, TerminalEvent::Exited) {
            cx.defer(|cx| {
                hide(cx);
                cx.global_mut::<Quick>().shell = None;
            });
        }
    });
    let q = cx.global_mut::<Quick>();
    q.shells = n;
    q.shell = Some((view.clone(), id, sub));
    Some(view)
}

fn view_of(target: &Target, cx: &mut App) -> Option<Entity<TerminalView>> {
    match target {
        Target::Shell => shell(cx),
        Target::Tab { window, tab, pane } => {
            let ws = window.entity(cx).ok()?;
            let ws = ws.read(cx);
            let ix = ws.tabs.iter().position(|t| &t.id == tab)?;
            ws.pane_view(ix, *pane)
        }
    }
}

/// The panel's frame on its screen, from the settings.
fn frame(cfg: &QuickCfg, cx: &App) -> (Bounds<gpui::Pixels>, Option<gpui::DisplayId>) {
    let (pointer, bar) = crate::platform::pointer_and_menu_bar();
    let displays = cx.displays();
    let display = match cfg.screen.as_str() {
        "main" => cx.primary_display(),
        _ => displays.iter().find(|d| d.bounds().contains(&pointer)).cloned().or_else(|| cx.primary_display()),
    };
    let screen = display.as_ref().map(|d| d.bounds()).unwrap_or(Bounds::new(gpui::point(px(0.), px(0.)), size(px(1440.), px(900.))));
    let (sw, sh) = (f32::from(screen.size.width), f32::from(screen.size.height) - bar);
    let (x0, y0) = (f32::from(screen.origin.x), f32::from(screen.origin.y) + bar);
    let share = match cfg.size.as_str() {
        "s" => 0.30,
        "l" => 0.55,
        _ => 0.40,
    };
    let (x, y, w, h) = match cfg.position.as_str() {
        "top" => (x0, y0, sw, sh * share),
        "bottom" => (x0, y0 + sh * (1. - share), sw, sh * share),
        "left" => (x0, y0, sw * share, sh),
        "right" => (x0 + sw * (1. - share), y0, sw * share, sh),
        _ => {
            let (w, h): (f32, f32) = match cfg.size.as_str() {
                "s" => (720., 340.),
                "l" => (1100., 520.),
                _ => (900., 420.),
            };
            let (w, h) = (w.min(sw), h.min(sh));
            (x0 + (sw - w) / 2., y0, w, h)
        }
    };
    (Bounds::new(gpui::point(px(x), px(y)), size(px(w), px(h))), display.map(|d| d.id()))
}

fn show(target: Target, focus: bool, cx: &mut App) {
    let Some(view) = view_of(&target, cx) else { return };
    let Some(look) = any_window(cx) else { return };
    let cfg = cx.global::<Quick>().cfg.clone();
    let (bounds, display_id) = frame(&cfg, cx);
    {
        let q = cx.global_mut::<Quick>();
        q.target = Some(target);
        q.shown = Some(view.clone());
    }
    // The tab's own window stops drawing this terminal before the panel starts.
    repaint_windows(cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            display_id,
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                // Clicking another app puts it away, unless pinned or told to stay.
                cx.observe_window_activation(window, |_: &mut Panel, window, cx| {
                    if !window.is_window_active() {
                        let q = cx.global::<Quick>();
                        if !q.pinned && q.cfg.hide_on_blur {
                            cx.defer(hide);
                        }
                    }
                })
                .detach();
                Panel { look, focus, opened: std::time::Instant::now(), leaving: None, edge: cfg.position.clone(), confirm: false, look_set: None }
            })
        },
    );
    match opened {
        Ok(panel) => {
            cx.global_mut::<Quick>().panel = Some(panel);
            if focus {
                let _ = panel.update(cx, |_, window, cx| {
                    window.activate_window();
                    window.focus(&view.read(cx).focus_handle(cx));
                });
            }
        }
        Err(e) => {
            log::error!("quick terminal: {e}");
            let q = cx.global_mut::<Quick>();
            q.target = None;
            q.shown = None;
            repaint_windows(cx);
        }
    }
}

/// ⌘J in the panel: the next tab that needs you (the quick shell when none is left).
fn next(cx: &mut App) {
    let Some(current) = cx.global::<Quick>().target.clone() else { return };
    let target = match &current {
        Target::Tab { tab, pane, .. } => pick(Some((tab, *pane)), cx),
        Target::Shell => pick(None, cx),
    };
    let Some(view) = view_of(&target, cx) else { return };
    let q = cx.global_mut::<Quick>();
    q.target = Some(target);
    q.shown = Some(view.clone());
    let panel = q.panel;
    repaint_windows(cx);
    if let Some(p) = panel {
        let _ = p.update(cx, |_, window, cx| {
            window.focus(&view.read(cx).focus_handle(cx));
            cx.notify();
        });
    }
}

/// "Open in window": the shown tab in its own window; the quick shell becomes a tab of the
/// frontmost window.
fn open_in_window(cx: &mut App) {
    let Some(target) = cx.global::<Quick>().target.clone() else { return };
    hide(cx);
    cx.activate(true);
    match target {
        Target::Tab { window, tab, pane } => {
            let _ = window.update(cx, |ws, window, cx| {
                window.activate_window();
                if let Some(ix) = ws.tabs.iter().position(|t| t.id == tab) {
                    ws.jump_to_pane(ix, pane, window, cx);
                }
            });
        }
        Target::Shell => {
            let Some((view, id, _)) = cx.global_mut::<Quick>().shell.take() else { return };
            if let Some(window) = any_window(cx) {
                let _ = window.update(cx, |ws, window, cx| {
                    window.activate_window();
                    let cwd = view.read(cx).terminal.cwd();
                    ws.adopt_view(id, view, cwd, window, cx);
                });
            }
        }
    }
}

/// The panel's root: a 34 pt strip saying what this is, over the terminal.
pub struct Panel {
    /// A window to take the look (colors, text scale) from.
    look: WindowHandle<Workspace>,
    focus: FocusHandle,
    opened: std::time::Instant,
    leaving: Option<std::time::Instant>,
    /// "panel" or the edge it slides from.
    edge: String,
    /// ⌘W in the quick shell: "End the quick shell?" in the strip.
    confirm: bool,
    look_set: Option<gpui::WindowBackgroundAppearance>,
}

impl Panel {
    /// How far in it is, 0 to 1, and whether it's still moving.
    fn progress(&self) -> (f32, bool) {
        if crate::platform::reduce_motion() {
            return (if self.leaving.is_some() { 0. } else { 1. }, false);
        }
        match self.leaving {
            Some(t) => {
                let x = (t.elapsed().as_secs_f32() * 1000. / OUT_MS).min(1.);
                // Ease-in: slow to start, then away.
                (1. - x * x, x < 1.)
            }
            None => {
                let x = (self.opened.elapsed().as_secs_f32() * 1000. / IN_MS).min(1.);
                // Ease-out (cubic): quick, then settling.
                (1. - (1. - x).powi(3), x < 1.)
            }
        }
    }
}

/// ⌘W in the panel: the quick shell asks first (ending it ends what runs there); a tab's
/// terminal is only put away (the tab stays in its window).
fn close_key(panel: &mut Panel, cx: &mut Context<Panel>) {
    match cx.global::<Quick>().target {
        Some(Target::Shell) => {
            panel.confirm = true;
            cx.notify();
        }
        _ => cx.defer(hide),
    }
}

/// End the quick shell (its terminal goes, and with it what ran there).
fn end_shell(cx: &mut App) {
    let shell = cx.global_mut::<Quick>().shell.take();
    hide(cx);
    drop(shell);
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (t, moving) = self.progress();
        if moving {
            window.request_animation_frame();
        }
        let q = cx.global::<Quick>();
        let (target, shown, pinned) = (q.target.clone(), q.shown.clone(), q.pinned);
        let Some(ws) = self.look.entity(cx).ok() else { return div() };
        let ui = ws.read(cx).ui;
        // See-through and blurred like the windows (Settings › Appearance › Window).
        let (bg, look) = {
            let w = ws.read(cx);
            let look = match (w.translucent(), w.config.window.background_blur) {
                (false, _) => gpui::WindowBackgroundAppearance::Opaque,
                (true, true) => gpui::WindowBackgroundAppearance::Blurred,
                (true, false) => gpui::WindowBackgroundAppearance::Transparent,
            };
            (w.term_bg(), look)
        };
        if self.look_set != Some(look) {
            window.set_background_appearance(look);
            self.look_set = Some(look);
        }
        let waiting = crate::windows::total_waiting();
        let mono = |s: String, c: gpui::Hsla| div().flex_none().font_family("JetBrains Mono").text_size(px(11.)).text_color(c).child(s);

        // Left: what this is. A tab: its icon with the badge, title, "Needs you · 4m".
        let mut left = div().flex_1().min_w_0().flex().items_center().gap(px(8.));
        let mut others = waiting;
        match &target {
            Some(Target::Tab { window, tab, pane }) => {
                if let Ok(w) = window.entity(cx) {
                    let w = w.read(cx);
                    if let Some(t) = w.tabs.iter().find(|t| &t.id == tab) {
                        let agent = if *pane == MAIN { t.agent.as_ref() } else { t.panes.helper(*pane).and_then(|h| h.agent.as_ref()) };
                        let (state, tone) = match agent.map(|a| &a.status) {
                            Some(Status::NeedsInput { .. }) => ("Needs you", ui.status_needs),
                            Some(Status::Error { .. }) => ("Error", ui.status_error),
                            Some(Status::Working) => ("Working", ui.text_secondary),
                            _ => ("Done", ui.text_tertiary),
                        };
                        let asking = agent.is_some_and(|a| a.status.needs_you());
                        if asking {
                            others = others.saturating_sub(1);
                        }
                        let since = agent.map(|a| crate::usage::span(a.since.elapsed().as_secs() as i64));
                        left = left
                            .child(crate::split::pane_icon(w, t, *pane, 16.))
                            .child(div().min_w_0().truncate().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child(t.title()))
                            .child(div().flex_none().text_size(px(12.)).text_color(tone).child(match since {
                                Some(s) => format!("{state} · {s}"),
                                None => state.to_string(),
                            }));
                    }
                }
            }
            _ => {
                let folder = shown.as_ref().and_then(|v| v.read(cx).terminal.cwd()).map(|p| crate::groups::tilde(&p)).unwrap_or_default();
                left = left
                    .child(div().flex_none().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child("Quick terminal"))
                    .child(div().min_w_0().truncate().text_size(px(12.)).text_color(ui.text_tertiary).child(folder));
            }
        }
        let more = match (&target, others) {
            (Some(Target::Tab { .. }), 0) => None,
            (Some(Target::Tab { .. }), n) => Some((format!("{n} more waiting"), ui.status_needs)),
            (_, 0) => Some(("Nothing needs you".to_string(), ui.text_tertiary)),
            (_, n) => Some((format!("{n} need{} you", if n == 1 { "s" } else { "" }), ui.status_needs)),
        };
        let button = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .h(px(22.))
                .px(px(7.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .text_size(px(11.5))
                .text_color(if on { ui.text_primary } else { ui.text_secondary })
                .when(on, |d| d.bg(ui.bg_control))
                .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label)
        };
        if self.confirm {
            let what = shown
                .as_ref()
                .and_then(|v| v.read(cx).terminal.foreground_command().map(|(c, _)| c))
                .filter(|c| !crate::close::SHELLS.contains(&c.split_whitespace().next().unwrap_or("").trim_start_matches('-')))
                .map(|c| format!("End the quick shell? {} stops.", c.split_whitespace().take(3).collect::<Vec<_>>().join(" ")))
                .unwrap_or_else(|| "End the quick shell?".into());
            left = div().flex_1().min_w_0().truncate().text_size(px(12.5)).text_color(ui.text_primary).child(what);
        }
        let confirming = self.confirm;
        let strip = div()
            .h(px(34.))
            .flex_none()
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(ui.border)
            .font_family(".SystemUIFont")
            .child(left)
            .when(confirming, |d| {
                d.child(button("quick-cancel", "Cancel", false).on_click(cx.listener(|p, _, _, cx| {
                    p.confirm = false;
                    cx.notify();
                })))
                .child(
                    button("quick-end", "End", false)
                        .bg(ui.status_error)
                        .text_color(t1k_themes::on_hsla(ui.status_error))
                        .on_click(|_, _, cx| cx.defer(end_shell)),
                )
            })
            .when(!confirming, |d| d.children(more.map(|(text, tone)| {
                div()
                    .id("quick-next")
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .on_click(|_, _, cx| cx.defer(next))
                    .child(div().text_size(px(11.5)).text_color(tone).child(text))
                    .when(others > 0, |d| d.child(mono("⌘J".into(), ui.text_tertiary)))
            })))
            .when(!confirming, |d| d.child(button("quick-pin", if pinned { "Pinned" } else { "Keep open" }, pinned).on_click(cx.listener(|_, _, _, cx| {
                let q = cx.global_mut::<Quick>();
                q.pinned = !q.pinned;
                cx.notify();
            }))))
            .when(!confirming, |d| d.child(button("quick-open", "Open in window", false).on_click(|_, _, cx| cx.defer(open_in_window))));

        // Slide 16 pt in from its edge, fading in.
        let off = 16. * (1. - t);
        let (dx, dy) = match self.edge.as_str() {
            "bottom" => (0., off),
            "left" => (-off, 0.),
            "right" => (off, 0.),
            _ => (0., -off),
        };

        div().size_full().relative().child(
            div()
                .absolute()
                .top(px(dy))
                .left(px(dx))
                .size_full()
                .opacity(t)
                .track_focus(&self.focus)
                .flex()
                .flex_col()
                .bg(bg)
                .border_1()
                .border_color(ui.border)
                // ⌘J here walks the tabs that need you, in place.
                .on_action(|_: &crate::actions::JumpToNeeds, _, cx| cx.defer(next))
                .on_action(cx.listener(|p, _: &crate::actions::CloseTab, _, cx| close_key(p, cx)))
                .child(strip)
                .child(div().flex_1().min_h_0().children(shown)),
        )
    }
}

/// In a tab's window, where its terminal would be while the panel has it.
pub fn placeholder(ws: &Workspace) -> gpui::Div {
    let ui = ws.ui;
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(ws.term_bg())
        .font_family(".SystemUIFont")
        .text_size(px(12.5))
        .text_color(ui.text_tertiary)
        .child("Shown in the quick terminal")
}

/// `Workspace::key_recording` while Settings records the quick terminal's shortcut.
pub const RECORD: &str = "quick_shortcut";

/// Under "Shortcut" in Settings: what it's for, or why it isn't working.
pub fn shortcut_note(ws: &Workspace, cx: &App) -> &'static str {
    if !ws.config.quick.enabled {
        return "Works in every app once the quick terminal is on";
    }
    match hotkey_state(cx) {
        Hotkey::Taken => "In use by another app. Pick another shortcut.",
        Hotkey::Invalid => "Needs ctrl, alt, shift or cmd with one key",
        _ => "Works in every app. Not opening? Another app may take the key first: try another shortcut.",
    }
}

/// The shortcut in Settings: click, then press the new keys.
pub fn recorder(ws: &Workspace, cx: &mut Context<Workspace>) -> gpui::AnyElement {
    let ui = ws.ui;
    let recording = ws.key_recording == Some(RECORD);
    let bad = ws.config.quick.enabled && matches!(hotkey_state(cx), Hotkey::Taken | Hotkey::Invalid);
    let text = if recording { "Press keys…".to_string() } else { crate::keymap::display(&ws.config.quick.shortcut) };
    div()
        .id("quick-shortcut")
        .h(px(24.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .bg(ui.bg_sidebar)
        .border_1()
        .border_color(if recording { ui.accent } else if bad { ui.status_needs } else { ui.border })
        .text_size(px(12.))
        .text_color(if recording { ui.accent } else { ui.text_secondary })
        .cursor_pointer()
        .hover(move |d| d.bg(ui.bg_hover))
        .on_click(cx.listener(|this, _, _, cx| {
            this.key_recording = if this.key_recording == Some(RECORD) { None } else { Some(RECORD) };
            cx.notify();
        }))
        .child(text)
        .into_any_element()
}

/// The first-run hint, once there are agents to answer: "Answer agents from any app".
pub fn offer(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<gpui::AnyElement> {
    let ui = ws.ui;
    let busy = ws.import_offer || (!ws.match_hidden && ws.match_plan().is_some()) || !matches!(ws.overlay, crate::overlay::Overlay::None);
    if ws.quick_hint_done || ws.config.quick.enabled || busy || crate::demo::on() || !ws.tabs.iter().any(|t| t.agent.is_some()) {
        return None;
    }
    let key = crate::keymap::display(&ws.config.quick.shortcut);
    let button = |id: &'static str, label: &'static str, primary: bool| {
        div()
            .id(id)
            .h(px(24.))
            .px(px(9.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .text_size(px(12.))
            .cursor_pointer()
            .when(primary, |d| d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::MEDIUM))
            .when(!primary, |d| d.bg(ui.bg_control).text_color(ui.text_primary))
            .hover(|d| d.opacity(0.85))
            .child(label)
    };
    let done = |this: &mut Workspace, cx: &mut Context<Workspace>| {
        this.quick_hint_done = true;
        this.save();
        cx.notify();
    };
    Some(
        div()
            .absolute()
            .left(px(10.))
            .right(px(10.))
            .bottom(px(10.))
            .p(px(10.))
            .rounded(px(8.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child("Answer agents from any app"))
            .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_secondary).child(format!("{key} shows the tab that needs you over whatever you're doing. Press it again to put it away.")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(button("quick-try", "Try it", true).on_click(cx.listener(move |this, _, _, cx| {
                        crate::config::set_value(&["quick", "enabled"], true.into());
                        this.config.quick.enabled = true;
                        configure(&this.config.quick.clone(), cx);
                        done(this, cx);
                        let key = crate::keymap::display(&this.config.quick.shortcut);
                        this.notice = Some((format!("Quick terminal is on: press {key} in any app"), std::time::Instant::now()));
                        this.notice_undo = false;
                        this.notice_brief = false;
                    })))
                    .child(button("quick-change", "Change shortcut…", false).on_click(cx.listener(move |this, _, window, cx| {
                        done(this, cx);
                        this.open_overlay(crate::overlay::Overlay::Settings { section: crate::settings::Section::Quick }, window, cx);
                    })))
                    .child(button("quick-later", "Not now", false).on_click(cx.listener(move |this, _, _, cx| done(this, cx)))),
            )
            .into_any_element(),
    )
}
