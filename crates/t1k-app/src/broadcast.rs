//! Broadcast input (canvas v3-aw): what you type goes to several terminals at once: this
//! tab's panes, a group's tabs, or tabs you tick.
//!
//! Typing lands in terminals you aren't looking at, so the mode is loud: an accent chip in
//! the strip, an accent border on every receiving pane, a glyph on receiving rows. Esc never
//! stops it (Esc belongs to the programs); ⌥⌘I, the chip's Stop, or leaving the set does.
//!
//! Shells and plain programs get live keys, each encoded for its own terminal's modes. When
//! the set holds an agent, nothing is typed anywhere: a compose bar collects one line and
//! Enter sends it whole (busy agents are skipped). A password prompt in any receiving
//! terminal pauses it, and a paste of more than one line asks first.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{div, prelude::*, px, AnyElement, Context, Div, Entity, FontWeight, KeyDownEvent, MouseButton, SharedString, Window};
use t1k_agent::Status;
use t1k_terminal::view::{PasteHold, Typed};
use t1k_terminal::TerminalView;

use crate::overlay::Overlay;
use crate::split::{PaneId, MAIN};
use crate::workspace::Workspace;

/// A receiving terminal: a tab's id and one of its panes.
pub type Target = (String, PaneId);

#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    /// Every pane of one tab.
    Panes,
    /// The tabs of one group.
    Group(PathBuf),
    /// Tabs ticked in the sidebar.
    Picked,
}

#[derive(Clone, Debug)]
pub struct Broadcast {
    pub scope: Scope,
    pub targets: Vec<Target>,
    /// Taken out by hand (its header's glyph): not receiving.
    pub out: HashSet<Target>,
    /// The compose bar's line.
    pub text: String,
}

impl Workspace {
    fn target_view(&self, t: &Target) -> Option<Entity<TerminalView>> {
        let ix = self.tabs.iter().position(|x| x.id == t.0)?;
        self.pane_view(ix, t.1)
    }

    /// Receiving terminals that still exist.
    pub fn broadcast_live(&self) -> Vec<Target> {
        let Some(b) = &self.broadcast else { return Vec::new() };
        b.targets.iter().filter(|t| !b.out.contains(*t) && self.target_view(t).is_some()).cloned().collect()
    }

    pub fn broadcast_receives(&self, tab: &str, pane: PaneId) -> bool {
        self.broadcast.as_ref().is_some_and(|b| b.targets.iter().any(|t| t.0 == tab && t.1 == pane) && !b.out.contains(&(tab.to_string(), pane)))
    }

    /// In the set (receiving or taken out).
    pub fn broadcast_has(&self, tab: &str, pane: PaneId) -> bool {
        self.broadcast.as_ref().is_some_and(|b| b.targets.iter().any(|t| t.0 == tab && t.1 == pane))
    }

    pub fn broadcast_has_tab(&self, tab: &str) -> bool {
        self.broadcast.as_ref().is_some_and(|b| b.targets.iter().any(|t| t.0 == tab))
    }

    fn target_agent(&self, t: &Target) -> Option<&t1k_agent::AgentState> {
        let tab = self.tabs.iter().find(|x| x.id == t.0)?;
        if t.1 == MAIN {
            tab.agent.as_ref().filter(|a| a.status != Status::Ended && tab.program.is_some())
        } else {
            tab.panes.helper(t.1)?.agent.as_ref()
        }
    }

    /// An agent that can't take a prompt now: mid-turn, or waiting on a question.
    pub fn broadcast_busy(&self, t: &Target) -> bool {
        self.target_agent(t).is_some_and(|a| matches!(a.status, Status::Working | Status::NeedsInput { .. }))
    }

    /// Any agent in the set: typing goes to the compose bar and is sent whole on Enter.
    pub fn broadcast_compose(&self) -> bool {
        self.broadcast_live().iter().any(|t| self.target_agent(t).is_some())
    }

    /// A receiving terminal sits at a password prompt: keys go only where you type.
    pub fn broadcast_paused(&self, cx: &gpui::App) -> bool {
        // The demo's placeholder terminals echo nothing, which looks like a password prompt.
        if crate::demo::on() {
            return false;
        }
        self.broadcast_live().iter().any(|t| self.target_view(t).is_some_and(|v| v.read(cx).terminal.password_prompt()))
    }

    /// "3 panes", "4 tabs in t1k", "4 tabs".
    pub fn broadcast_label(&self) -> String {
        let n = self.broadcast_live().len();
        match self.broadcast.as_ref().map(|b| &b.scope) {
            Some(Scope::Panes) => format!("{n} panes"),
            Some(Scope::Group(g)) => format!("{n} tabs in {}", self.group_label(g)),
            _ => format!("{n} tabs"),
        }
    }

    fn start_broadcast(&mut self, scope: Scope, targets: Vec<Target>, window: &mut Window, cx: &mut Context<Self>) {
        if targets.len() < 2 {
            self.say("Broadcast needs at least two terminals", cx);
            return;
        }
        // You type in one of them: come to the set if you're outside it.
        let here = self.tabs.get(self.active).map(|t| t.id.clone()).unwrap_or_default();
        if !targets.iter().any(|t| t.0 == here) {
            if let Some(ix) = self.tabs.iter().position(|t| t.id == targets[0].0) {
                self.activate(ix, window, cx);
            }
        }
        self.broadcast_resume = None;
        self.broadcast = Some(Broadcast { scope, targets, out: HashSet::new(), text: String::new() });
        self.sync_broadcast(cx);
        cx.notify();
    }

    pub fn broadcast_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let targets = t.panes.layout.leaves().into_iter().map(|p| (t.id.clone(), p)).collect();
        self.start_broadcast(Scope::Panes, targets, window, cx);
    }

    pub fn broadcast_group(&mut self, key: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.tabs.iter().filter(|t| t.group == key).map(|t| (t.id.clone(), MAIN)).collect();
        self.start_broadcast(Scope::Group(key), targets, window, cx);
    }

    /// Start ticking tabs in the sidebar (this tab is ticked already).
    pub fn broadcast_pick(&mut self, cx: &mut Context<Self>) {
        self.stop_broadcast(false, cx);
        self.broadcast_picking = Some(self.tabs.get(self.active).map(|t| t.id.clone()).into_iter().collect());
        if !matches!(self.sidebar_shown(), crate::rail::Shown::Full(_)) {
            self.sidebar_open = true;
        }
        cx.notify();
    }

    pub fn broadcast_pick_done(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picked) = self.broadcast_picking.take() else { return };
        let targets: Vec<Target> = self.tabs.iter().filter(|t| picked.contains(&t.id)).map(|t| (t.id.clone(), MAIN)).collect();
        self.start_broadcast(Scope::Picked, targets, window, cx);
        cx.notify();
    }

    /// ⌥⌘I: stop; or start on this tab's panes; with one pane, the palette's broadcast entries.
    pub fn toggle_broadcast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.broadcast.is_some() {
            self.stop_broadcast(false, cx);
        } else if self.broadcast_picking.take().is_some() {
            cx.notify();
        } else if self.tabs.get(self.active).is_some_and(|t| t.panes.multi()) {
            self.broadcast_panes(window, cx);
        } else {
            self.open_overlay(Overlay::Palette { query: "broadcast".into(), selected: 0 }, window, cx);
        }
    }

    /// `left`: you switched to a tab outside the set, so a toast offers Resume.
    pub fn stop_broadcast(&mut self, left: bool, cx: &mut Context<Self>) {
        let Some(b) = self.broadcast.take() else { return };
        self.sync_broadcast(cx);
        if left {
            self.broadcast_resume = Some(b);
            self.notice = Some(("Broadcast stopped: you left the set".into(), Instant::now()));
            self.notice_undo = false;
            self.notice_brief = false;
        }
        cx.notify();
    }

    pub fn resume_broadcast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.broadcast_resume.take() else { return };
        self.notice = None;
        self.start_broadcast(b.scope, b.targets, window, cx);
    }

    fn say(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some((text.into(), Instant::now()));
        self.notice_undo = false;
        self.notice_brief = false;
        self.broadcast_resume = None;
        cx.notify();
    }

    /// Called after switching tabs: leaving the set stops the broadcast.
    pub fn broadcast_tab_changed(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.get(self.active).map(|t| t.id.clone()) else { return };
        if self.broadcast.is_some() && !self.broadcast_has_tab(&id) {
            self.stop_broadcast(true, cx);
        }
    }

    /// Every poll: closed terminals leave the set (down to one, it stops), and each view
    /// learns which pastes to hold.
    pub fn sync_broadcast(&mut self, cx: &mut Context<Self>) {
        if self.broadcast.is_some() && self.broadcast_live().len() < 2 {
            self.broadcast = None;
            self.say("Broadcast stopped: one terminal left", cx);
        }
        let compose = self.broadcast_compose();
        for t in &self.tabs {
            let views = std::iter::once((MAIN, t.view.clone())).chain(t.panes.helpers.iter().map(|h| (h.id, h.view.clone())));
            for (pane, view) in views {
                let hold = match self.broadcast_receives(&t.id, pane) {
                    true if compose => PasteHold::All,
                    true => PasteHold::Multiline,
                    false => PasteHold::None,
                };
                if view.read(cx).paste_hold != hold {
                    view.update(cx, |v, _| v.paste_hold = hold);
                }
            }
        }
    }

    /// The person typed in `from`: repeat it in the other receiving terminals.
    pub fn broadcast_typed(&mut self, from: &Entity<TerminalView>, typed: &Typed, cx: &mut Context<Self>) {
        if self.broadcast.is_none() || self.broadcast_compose() {
            return;
        }
        let live = self.broadcast_live();
        let views: Vec<Entity<TerminalView>> = live.iter().filter_map(|t| self.target_view(t)).collect();
        if !views.contains(from) || self.broadcast_paused(cx) {
            return;
        }
        for v in views.iter().filter(|v| *v != from) {
            v.update(cx, |v, cx| v.typed_elsewhere(typed, cx));
        }
    }

    /// A paste the view held: into the compose bar, or (more than one line) ask first.
    pub fn broadcast_paste_held(&mut self, from: &Entity<TerminalView>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.broadcast_compose() {
            if let Some(b) = &mut self.broadcast {
                b.text.push_str(&text.trim_end().replace(['\n', '\r'], " "));
            }
            cx.notify();
            return;
        }
        if self.broadcast.is_none() {
            // The mode ended between the keystroke and here: paste as usual.
            from.read(cx).terminal.paste(text);
            return;
        }
        self.open_overlay(Overlay::BroadcastPaste { text: text.to_string(), from: from.clone() }, window, cx);
    }

    /// The paste confirm's answer: everywhere, or only where it was pasted.
    pub fn broadcast_paste(&mut self, text: &str, from: &Entity<TerminalView>, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlay(window, cx);
        from.read(cx).terminal.paste(text);
        if all {
            self.broadcast_typed(from, &Typed::Paste(text.to_string()), cx);
        }
    }

    /// Enter in the compose bar: the line goes to every receiving terminal that can take it.
    fn broadcast_send(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.broadcast.as_mut().map(|b| std::mem::take(&mut b.text)).filter(|t| !t.trim().is_empty()) else { return };
        let (mut sent, mut skipped) = (Vec::new(), 0);
        for t in self.broadcast_live() {
            if self.broadcast_busy(&t) {
                skipped += 1;
            } else if let Some(v) = self.target_view(&t) {
                v.read(cx).terminal.paste(&text);
                sent.push(v);
            }
        }
        // Enter goes a moment later: agents read a line and its Enter arriving together as a
        // paste with a newline in it, and don't submit.
        let n = sent.len();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            for v in sent {
                let _ = v.update(cx, |v, _| v.terminal.write(&b"\r"[..]));
            }
        })
        .detach();
        let noun = if matches!(self.broadcast.as_ref().map(|b| &b.scope), Some(Scope::Panes)) { "pane" } else { "tab" };
        let s = if n == 1 { "" } else { "s" };
        let skip = if skipped > 0 { format!(" · {skipped} skipped (busy)") } else { String::new() };
        self.say(format!("Sent to {n} {noun}{s}{skip}"), cx);
    }

    /// Keys before the terminal sees them: the compose bar's typing, and Enter / Esc while
    /// ticking tabs. Shortcuts pass through.
    pub fn broadcast_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.overlay, Overlay::None) {
            return;
        }
        let ks = &ev.keystroke;
        if self.broadcast_picking.is_some() {
            match ks.key.as_str() {
                "enter" => self.broadcast_pick_done(window, cx),
                "escape" => {
                    self.broadcast_picking = None;
                    cx.notify();
                }
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if self.broadcast.is_none() || !self.broadcast_compose() {
            return;
        }
        // Only where typing would have reached a receiving terminal.
        let here = self.tabs.get(self.active).map(|t| (t.id.clone(), self.focused_pane(window, cx)));
        if !here.is_some_and(|(t, p)| self.broadcast_receives(&t, p)) {
            return;
        }
        let m = &ks.modifiers;
        if (m.platform && ks.key != "backspace") || m.control || m.function {
            return;
        }
        let Some(b) = &mut self.broadcast else { return };
        match crate::input::handle(&mut b.text, ev) {
            crate::input::Key::Submit => self.broadcast_send(cx),
            // Esc clears a line you started; with nothing typed it goes to the program.
            crate::input::Key::Cancel => {
                if b.text.is_empty() {
                    return;
                }
                b.text.clear();
            }
            crate::input::Key::Ignored => return,
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub fn broadcast_toggle_target(&mut self, tab: &str, pane: PaneId, cx: &mut Context<Self>) {
        let Some(b) = &mut self.broadcast else { return };
        let t = (tab.to_string(), pane);
        if !b.out.remove(&t) {
            b.out.insert(t);
        }
        self.sync_broadcast(cx);
        cx.notify();
    }
}

// ---------- drawing ----------

/// The broadcast glyph (a dot with waves), struck through for "not receiving".
pub fn glyph(color: gpui::Hsla, size: f32, struck: bool) -> impl IntoElement {
    crate::icons::artifact(if struck { "x-broadcast-off" } else { "x-broadcast" }, color, size)
}

/// The strip's chip: "Broadcasting to 3 panes  Stop ⌥⌘I"; an outline while a password
/// prompt pauses it.
pub fn chip(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    ws.broadcast.as_ref()?;
    let ui = ws.ui;
    let paused = ws.broadcast_paused(cx);
    let on = ui.on_accent;
    let (fg, text) = if paused { (ui.accent, "Paused: password prompt".to_string()) } else { (on, format!("Broadcasting to {}", ws.broadcast_label())) };
    let key = crate::keymap::display_for(&crate::actions::ToggleBroadcast, &ws.config.keys);
    Some(
        div()
            .id("broadcast-chip")
            .flex_none()
            .h(px(24.))
            .pl(px(8.))
            .pr(px(3.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .when(!paused, |d| d.bg(ui.accent))
            .when(paused, |d| d.border_1().border_color(ui.accent))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(glyph(fg, 13., false))
            .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(fg).child(text))
            .child(
                div()
                    .id("broadcast-stop")
                    .h(px(18.))
                    .px(px(6.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(px(11.5))
                    .text_color(fg)
                    .bg(fg.opacity(0.16))
                    .hover(move |d| d.bg(fg.opacity(0.28)))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.stop_broadcast(false, cx)))
                    .child("Stop")
                    .child(div().font_family("JetBrains Mono").text_size(px(10.5)).opacity(0.8).child(key)),
            )
            .into_any_element(),
    )
}

/// The accent frame on a receiving pane's terminal.
pub fn frame(ws: &Workspace) -> Div {
    div().absolute().inset_0().border_color(ws.ui.accent.opacity(0.75)).border(px(1.5))
}

/// "Receiving" at the top right of a receiving tab with one pane (it has no header).
pub fn receiving_chip(ws: &Workspace) -> Div {
    let ui = ws.ui;
    div()
        .absolute()
        .top(px(8.))
        .right(px(10.))
        .h(px(20.))
        .px(px(7.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .gap(px(5.))
        .bg(gpui::Hsla::blend(ui.bg_terminal, ui.accent.opacity(0.18)))
        .child(glyph(ui.accent, 12., false))
        .child(div().text_size(px(11.)).text_color(ui.accent).child("Receiving"))
}

/// The glyph in a pane's header: click takes the pane out of the set, or back in.
pub fn header_glyph(ws: &Workspace, tab: &str, pane: PaneId, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !ws.broadcast_has(tab, pane) {
        return None;
    }
    let ui = ws.ui;
    let on = ws.broadcast_receives(tab, pane);
    let busy = on && ws.broadcast_busy(&(tab.to_string(), pane));
    let id = tab.to_string();
    Some(
        div()
            .id(SharedString::from(format!("broadcast-pane-{pane}")))
            .flex_none()
            .h(px(18.))
            .px(px(4.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .hover(move |d| d.bg(ui.bg_hover))
            .cursor_pointer()
            .tooltip(crate::tooltip::text(ui, if on { "Receiving what you type. Click to take this pane out." } else { "Not receiving. Click to add it back." }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| this.broadcast_toggle_target(&id, pane, cx)))
            .child(glyph(if on && !busy { ui.accent } else { ui.text_tertiary }, 13., !on || busy))
            .when(!on, |d| d.child(div().text_size(px(10.5)).text_color(ui.text_tertiary).child("not receiving")))
            .when(busy, |d| d.child(div().text_size(px(10.5)).text_color(ui.text_tertiary).child("busy")))
            .into_any_element(),
    )
}

/// A row's right-end mark: the glyph for a receiving tab, "busy" for an agent that would be
/// skipped; a checkbox while tabs are being ticked.
pub fn row_mark(ws: &Workspace, tab: &str, k: f32) -> Option<AnyElement> {
    let ui = ws.ui;
    if let Some(picked) = &ws.broadcast_picking {
        let on = picked.contains(tab);
        return Some(
            div()
                .flex_none()
                .size(px(14.))
                .rounded(px(3.5))
                .flex()
                .items_center()
                .justify_center()
                .when(on, |d| d.bg(ui.accent))
                .when(!on, |d| d.border_1().border_color(ui.text_tertiary))
                .when(on, |d| d.child(div().text_size(px(10.)).font_weight(FontWeight::BOLD).text_color(ui.on_accent).child("✓")))
                .into_any_element(),
        );
    }
    if !ws.broadcast_has_tab(tab) {
        return None;
    }
    let live = ws.broadcast_live();
    let mine: Vec<&Target> = live.iter().filter(|t| t.0 == tab).collect();
    let busy = !mine.is_empty() && mine.iter().all(|t| ws.broadcast_busy(t));
    let off = mine.is_empty() || busy;
    Some(
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .when(busy, |d| d.child(div().text_size(px(10.5 * k)).text_color(ui.text_tertiary).child("busy")))
            .child(glyph(if off { ui.text_tertiary } else { ui.accent }, 13., off))
            .into_any_element(),
    )
}

/// While ticking tabs: the bar at the bottom of the sidebar.
pub fn pick_bar(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let picked = ws.broadcast_picking.as_ref()?;
    let ui = ws.ui;
    let n = picked.len();
    let button = |id: &'static str, label: String, key: &'static str, primary: bool| {
        div()
            .id(id)
            .h(px(24.))
            .px(px(9.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .cursor_pointer()
            .when(primary, |d| d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::MEDIUM))
            .when(!primary, |d| d.bg(ui.bg_control).text_color(ui.text_primary))
            .hover(|d| d.opacity(0.85))
            .child(label)
            .child(div().font_family("JetBrains Mono").text_size(px(10.5)).opacity(0.7).child(key))
    };
    Some(
        div()
            .flex_none()
            .mx(px(8.))
            .mb(px(8.))
            .p(px(10.))
            .rounded(px(8.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(12.)).text_color(ui.text_secondary).child("Tick the tabs to type into."))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        button("broadcast-pick-go", format!("Broadcast to {n} tab{}", if n == 1 { "" } else { "s" }), "⏎", true)
                            .when(n < 2, |d| d.opacity(0.45))
                            .on_click(cx.listener(|this, _, window, cx| this.broadcast_pick_done(window, cx))),
                    )
                    .child(button("broadcast-pick-cancel", "Cancel".into(), "esc", false).on_click(cx.listener(|this, _, _, cx| {
                        this.broadcast_picking = None;
                        cx.notify();
                    }))),
            )
            .into_any_element(),
    )
}

/// The compose bar at the bottom of the window (only while the set holds an agent).
pub fn compose_bar(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let b = ws.broadcast.as_ref()?;
    if !ws.broadcast_compose() {
        return None;
    }
    let ui = ws.ui;
    let live = ws.broadcast_live();
    let ready = live.iter().filter(|t| !ws.broadcast_busy(t)).count();
    let noun = if b.scope == Scope::Panes { "pane" } else { "tab" };
    let caret = (ws.started.elapsed().as_millis() / 530).is_multiple_of(2);
    Some(
        div()
            .id("broadcast-compose")
            .flex_none()
            .h(px(64.))
            .px(px(14.))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(5.))
            .bg(ui.bg_raised)
            .border_t_1()
            .border_color(ui.accent.opacity(0.75))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(glyph(ui.accent, 14., false))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .font_family(ws.config.terminal.font.family.clone())
                            .text_size(px(13.))
                            .text_color(if b.text.is_empty() { ui.text_tertiary } else { ui.text_primary })
                            .child(div().min_w_0().truncate().child(if b.text.is_empty() { "Type once, send to all".to_string() } else { b.text.clone() }))
                            .child(div().w(px(1.5)).h(px(15.)).ml(px(1.)).flex_none().bg(if caret { ui.accent } else { gpui::transparent_black() })),
                    )
                    .child(
                        div()
                            .id("broadcast-send")
                            .flex_none()
                            .h(px(22.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .bg(ui.accent)
                            .text_color(ui.on_accent)
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .when(b.text.trim().is_empty(), |d| d.opacity(0.45))
                            .on_click(cx.listener(|this, _, _, cx| this.broadcast_send(cx)))
                            .child(format!("Send to {ready} {noun}{}", if ready == 1 { "" } else { "s" }))
                            .child(div().font_family("JetBrains Mono").text_size(px(10.5)).opacity(0.8).child("⏎")),
                    ),
            )
            .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("Shells run it as a command. Claude tabs get it as one prompt. Busy tabs are skipped."))
            .into_any_element(),
    )
}

/// "Paste 12 lines into 4 terminals?"
pub fn paste_confirm(ws: &Workspace, text: &str, from: &Entity<TerminalView>, cx: &mut Context<Workspace>) -> Div {
    let ui = ws.ui;
    let lines = text.trim_end().lines().count();
    let n = ws.broadcast_live().len();
    let button = |id: &'static str, label: String, primary: bool| {
        div()
            .id(id)
            .h(px(26.))
            .px(px(10.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .text_size(px(12.5))
            .cursor_pointer()
            .when(primary, |d| d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::MEDIUM))
            .when(!primary, |d| d.bg(ui.bg_control).text_color(ui.text_primary))
            .hover(|d| d.opacity(0.85))
            .child(label)
    };
    let (t1, f1) = (text.to_string(), from.clone());
    let (t2, f2) = (text.to_string(), from.clone());
    div()
        .w(px(380.))
        .mt(px(160.))
        .p(px(16.))
        .rounded(px(10.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .font_family(".SystemUIFont")
        .flex()
        .flex_col()
        .gap(px(12.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(format!("Paste {lines} lines into {n} terminals?")))
        .child(div().text_size(px(12.)).text_color(ui.text_secondary).child("Broadcast is on, so each line would run in every receiving terminal."))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.))
                .child(button("bp-cancel", "Cancel".into(), false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
                .child(button("bp-here", "Paste here only".into(), false).on_click(cx.listener(move |this, _, window, cx| this.broadcast_paste(&t1, &f1, false, window, cx))))
                .child(button("bp-all", format!("Paste into {n}  ⏎"), true).on_click(cx.listener(move |this, _, window, cx| this.broadcast_paste(&t2, &f2, true, window, cx)))),
        )
}
