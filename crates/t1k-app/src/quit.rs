//! Quitting with programs still running (canvas v3-p, option A): a warning that can expand
//! into a live list of what's running, each with Go to tab and Stop; then a graceful
//! shutdown that lets each program finish its last output (saved with the scrollback).

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, SharedString, Window};
use t1k_agent::Status;

use crate::overlay::Overlay;
use crate::update_ui::{button, card, heading, row};
use crate::workspace::Workspace;

const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "login", "nu", "xonsh"];
/// How long programs get to exit (after Quit anyway, or Stop) before we offer force.
const GRACE: Duration = Duration::from_secs(3);

/// One running tab, as the dialog lists it.
pub struct Running {
    pub tab: usize,
    pub name: String,
    pub command: String,
    pub since: Option<SystemTime>,
    pub claude: bool,
}

impl Workspace {
    /// Tabs with a program in the foreground right now, and its name.
    pub fn running_programs(&self, cx: &gpui::App) -> Vec<(usize, String)> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let n = t.view.read(cx).terminal.foreground_name()?;
                (!SHELLS.contains(&n.trim_start_matches('-'))).then_some((i, n))
            })
            .collect()
    }

    fn is_claude(&self, tab: usize) -> bool {
        self.tabs[tab].agent.as_ref().is_some_and(|a| a.kind == t1k_agent::agents::AgentKind::Claude && !a.session_id.is_empty())
    }

    /// Running tabs in the dialog's order: needs-you, then errors, then working, then the
    /// rest, oldest first.
    pub fn running_list(&self, cx: &gpui::App) -> Vec<Running> {
        let mut v: Vec<Running> = self
            .running_programs(cx)
            .into_iter()
            .map(|(tab, name)| {
                let (command, since) = self.tabs[tab].view.read(cx).terminal.foreground_command().unwrap_or((name.clone(), None));
                Running { tab, name, command, since, claude: self.is_claude(tab) }
            })
            .collect();
        let rank = |r: &Running| match self.tabs[r.tab].status() {
            Some(Status::NeedsInput { .. }) => 0,
            Some(Status::Error { .. }) => 1,
            Some(Status::Working) => 2,
            _ => 3,
        };
        v.sort_by_key(|r| (rank(r), r.since));
        v
    }

    /// ⌘Q: ask first when something is still running; otherwise quit.
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quitting.is_some() {
            return;
        }
        if self.running_programs(cx).is_empty() {
            crate::platform::quit(cx);
        } else {
            self.open_overlay(Overlay::QuitRunning { expanded: false, selected: 0 }, window, cx);
        }
    }

    /// Ask one tab's program to stop: Claude gets Ctrl-C twice (it exits cleanly and
    /// resumes next launch), anything else one Ctrl-C. After 3 s the row offers Kill.
    pub fn stop_running(&mut self, tab: usize, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(tab) else { return };
        let id = t.id.clone();
        let claude = self.is_claude(tab);
        if claude {
            self.resume_after_quit.insert(id.clone());
        }
        t.view.read(cx).terminal.write(b"\x03".to_vec());
        self.stopping.insert(id.clone(), Instant::now());
        if claude {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(350)).await;
                let _ = this.update(cx, |ws, cx| {
                    if let Some(t) = ws.tabs.iter().find(|t| t.id == id) {
                        t.view.read(cx).terminal.write(b"\x03".to_vec());
                    }
                });
            })
            .detach();
        }
        self.tick_quit_dialog(cx);
        cx.notify();
    }

    pub fn kill_running(&mut self, tab: usize, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get(tab) {
            t.view.read(cx).terminal.kill_foreground();
        }
        cx.notify();
    }

    /// While the dialog is open, redraw every half second so the list and times stay live.
    fn tick_quit_dialog(&mut self, cx: &mut Context<Self>) {
        if self.quit_ticker.is_some() {
            return;
        }
        self.quit_ticker = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let open = this
                .update(cx, |ws, cx| {
                    let open = matches!(ws.overlay, Overlay::QuitRunning { .. });
                    if open {
                        cx.notify();
                    } else {
                        ws.quit_ticker = None;
                    }
                    open
                })
                .unwrap_or(false);
            if !open {
                break;
            }
        }));
    }

    /// Quit anyway, gently: Claude sessions get Ctrl-C twice (they exit cleanly and can be
    /// resumed), other programs a hangup like closing a terminal window. T1K waits up to 3 s
    /// for them, so their last words land in the saved scrollback, then quits.
    pub fn quit_gracefully(&mut self, cx: &mut Context<Self>) {
        if self.quitting.is_some() {
            return;
        }
        let running = self.running_programs(cx);
        // Remember which Claude sessions were alive, so they resume next launch even though
        // they're about to exit.
        let live: HashSet<String> = running.iter().filter(|(i, _)| self.is_claude(*i)).map(|(i, _)| self.tabs[*i].id.clone()).collect();
        self.resume_after_quit.extend(live.iter().cloned());
        self.quitting = Some(Instant::now());
        for (i, _) in &running {
            let t = &self.tabs[*i];
            let term = &t.view.read(cx).terminal;
            if live.contains(&t.id) {
                term.write(b"\x03".to_vec());
            } else {
                term.hangup_foreground();
            }
        }
        self.tick_quit_dialog(cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            // Claude asks for a second Ctrl-C to exit.
            cx.background_executor().timer(Duration::from_millis(350)).await;
            let _ = this.update(cx, |ws, cx| {
                for t in &ws.tabs {
                    if live.contains(&t.id) {
                        t.view.read(cx).terminal.write(b"\x03".to_vec());
                    }
                }
            });
            // Quit as soon as everything has exited; past the grace period, wait for Force
            // quit (the dialog says what's still running).
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                // Backed out of (Cancel): stop waiting, quit nothing.
                let backed_out = this.update(cx, |ws, _| ws.quitting.is_none()).unwrap_or(true);
                if backed_out {
                    break;
                }
                let done = this.update(cx, |ws, cx| ws.running_programs(cx).is_empty()).unwrap_or(true);
                if done {
                    // A moment for the last output to be drawn into the grid.
                    cx.background_executor().timer(Duration::from_millis(150)).await;
                    let _ = this.update(cx, |_, cx| crate::platform::quit(cx));
                    break;
                }
            }
        })
        .detach();
    }

    /// Keys in the quit dialog: ↑↓ move, ↵ go to tab (or quit, collapsed), ⌫ stop,
    /// ⌘↵ quit anyway (force quit when something hangs), esc collapses then cancels.
    pub fn quit_key(&mut self, expanded: bool, mut selected: usize, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        let k = ev.keystroke.key.as_str();
        let cmd = ev.keystroke.modifiers.platform;
        if let Some(at) = self.quitting {
            if k == "enter" && cmd && at.elapsed() > GRACE {
                crate::platform::quit(cx);
            }
            if k == "escape" {
                self.cancel_quit(cx);
                return None;
            }
            return Some(Overlay::QuitRunning { expanded, selected });
        }
        let list = self.running_list(cx);
        if list.is_empty() {
            return match k {
                "enter" => {
                    crate::platform::quit(cx);
                    None
                }
                "escape" => None,
                _ => Some(Overlay::QuitRunning { expanded, selected }),
            };
        }
        selected = selected.min(list.len() - 1);
        match k {
            "escape" if expanded => return Some(Overlay::QuitRunning { expanded: false, selected: 0 }),
            "escape" => return None,
            "enter" if cmd || !expanded => {
                self.quit_gracefully(cx);
            }
            "enter" => {
                let tab = list[selected].tab;
                self.close_overlay(window, cx);
                self.activate(tab, window, cx);
                return None;
            }
            "up" if expanded => selected = selected.saturating_sub(1),
            "down" if expanded => selected = (selected + 1).min(list.len() - 1),
            "backspace" | "delete" if expanded => {
                let tab = list[selected].tab;
                let id = self.tabs[tab].id.clone();
                if self.stopping.get(&id).is_some_and(|t| t.elapsed() > GRACE) {
                    self.kill_running(tab, cx);
                } else {
                    self.stop_running(tab, cx);
                }
            }
            _ => {}
        }
        Some(Overlay::QuitRunning { expanded, selected })
    }

    /// Back out of quitting: whatever already stopped stays stopped (its tab keeps its
    /// output), the rest keeps running, and T1K stays open.
    /// Claude sessions the quit already stopped stay marked to resume, so the next launch
    /// brings back every session the quit closed, not just the ones still running then.
    pub fn cancel_quit(&mut self, cx: &mut Context<Self>) {
        self.quitting = None;
        self.save();
        self.overlay = Overlay::None;
        cx.notify();
    }

    pub fn show_running(&mut self, cx: &mut Context<Self>) {
        self.overlay = Overlay::QuitRunning { expanded: true, selected: 0 };
        self.tick_quit_dialog(cx);
        cx.notify();
    }
}

fn ago(since: Option<SystemTime>) -> String {
    let Some(s) = since.and_then(|s| s.elapsed().ok()).map(|d| d.as_secs()) else { return String::new() };
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

/// "2 Claude sessions and npm run dev": Claude sessions counted, other programs named, at
/// most two named before "and N more".
fn names(list: &[Running]) -> String {
    let claude = list.iter().filter(|r| r.claude).count();
    let mut parts: Vec<String> = Vec::new();
    if claude > 0 {
        parts.push(if claude == 1 { "1 Claude session".into() } else { format!("{claude} Claude sessions") });
    }
    let others: Vec<&Running> = list.iter().filter(|r| !r.claude).collect();
    for r in others.iter().take(2) {
        parts.push(short(&r.command));
    }
    if others.len() > 2 {
        parts.push(format!("{} more", others.len() - 2));
    }
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => format!("{} and {}", parts[..parts.len() - 1].join(", "), parts[parts.len() - 1]),
    }
}

fn short(command: &str) -> String {
    let s: String = command.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
    if s.chars().count() > 28 { s.chars().take(27).collect::<String>() + "…" } else { s }
}

/// What stopping a program means, when it's not just "it stops".
fn consequence(r: &Running) -> &'static str {
    if r.claude {
        return "resumes next time";
    }
    match r.name.as_str() {
        "ssh" | "mosh" | "mosh-client" => "closes the connection",
        "vim" | "nvim" | "vi" | "nano" | "emacs" | "hx" | "helix" | "micro" => "unsaved changes are lost",
        _ => "stops on quit",
    }
}

pub fn render(ws: &Workspace, expanded: bool, selected: usize, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let list = ws.running_list(cx);
    let n = list.len();

    // After Quit anyway: what's closing, and Force quit if something hangs.
    if let Some(at) = ws.quitting {
        let late = at.elapsed() > GRACE;
        let title = match (n, late) {
            (0, _) => "Quitting…".to_string(),
            (1, true) => format!("{} isn't stopping", short(&list[0].command)),
            (_, true) => format!("{n} programs aren't stopping"),
            (n, false) => format!("Closing {n} program{}…", if n == 1 { "" } else { "s" }),
        };
        return card(ws)
            .w(px(if expanded { 540. } else { 440. }))
            .child(heading(ws, title, None))
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(ui.text_secondary).child(SharedString::from(if late {
                "Force quit stops them without waiting; anything they haven't saved is lost.".to_string()
            } else {
                format!("Waiting for {} to finish.", names(&list))
            })))
            .child(
                row()
                    .justify_end()
                    .child(button(ws, "quit-cancel", "Cancel  esc", false).on_click(cx.listener(|this, _, _, cx| this.cancel_quit(cx))))
                    .when(late, |d| d.child(button(ws, "quit-force", "Force quit  ⌘↵", true).on_click(|_, _, cx| crate::platform::quit(cx)))),
            )
            .into_any_element();
    }

    if n == 0 {
        // Everything stopped while the dialog was open; never quits by itself.
        return card(ws)
            .w(px(if expanded { 540. } else { 440. }))
            .child(heading(ws, "Nothing is running".to_string(), None))
            .child(div().text_size(px(13.)).text_color(ui.text_secondary).child("Everything has stopped. Tabs come back next time."))
            .child(
                row()
                    .justify_end()
                    .child(button(ws, "quit-cancel", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
                    .child(button(ws, "quit-anyway", "Quit now  ↵", true).on_click(|_, _, cx| crate::platform::quit(cx))),
            )
            .into_any_element();
    }

    let claude = list.iter().any(|r| r.claude);
    let other: Vec<&Running> = list.iter().filter(|r| !r.claude).collect();
    let body = if n == 1 {
        let what = if list[0].claude { ws.tabs[list[0].tab].title() } else { short(&list[0].command) };
        let after = if list[0].claude { "It resumes next time." } else { "It stops when T1K quits." };
        format!("{what} is still running. {after}")
    } else {
        let after = match (claude, other.len()) {
            (true, 0) => "They resume next time.".to_string(),
            (true, 1) => format!("Claude sessions resume next time; {} stops.", short(&other[0].command)),
            (true, _) => "Claude sessions resume next time; other programs stop.".to_string(),
            _ => "They stop when T1K quits.".to_string(),
        };
        format!("{n} tabs are still running: {}. {after}", names(&list))
    };

    let mut c = card(ws)
        .w(px(if expanded { 540. } else { 440. }))
        .child(heading(ws, "Quit T1K?".to_string(), None))
        .child(div().text_size(px(13.)).line_height(px(18.)).text_color(ui.text_secondary).child(SharedString::from(body)));

    if expanded {
        let selected = selected.min(n - 1);
        let mut rows = div().flex().flex_col().gap(px(2.));
        for (i, r) in list.iter().enumerate() {
            let t = &ws.tabs[r.tab];
            let on = i == selected;
            let (state, tone) = match t.status() {
                Some(Status::NeedsInput { .. }) => ("Needs you", ui.status_needs),
                Some(Status::Error { .. }) => ("Error", ui.status_error),
                Some(Status::Working) => ("Working", ui.text_secondary),
                _ => ("Running", ui.text_tertiary),
            };
            let stopping = ws.stopping.get(&t.id).copied();
            let tab = r.tab;
            let small = |id: String, label: String| {
                div()
                    .id(SharedString::from(id))
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .bg(ui.bg_control)
                    .text_size(px(11.5))
                    .text_color(ui.text_primary)
                    .hover(|d| d.opacity(0.85))
                    .child(label)
            };
            let right: AnyElement = match stopping {
                Some(at) if at.elapsed() > GRACE => small(format!("kill-{i}"), "Kill  ⌫".into())
                    .on_click(cx.listener(move |this, _, _, cx| this.kill_running(tab, cx)))
                    .into_any_element(),
                Some(_) => div().text_size(px(11.5)).text_color(ui.text_tertiary).child("Stopping…").into_any_element(),
                None if on => div()
                    .flex()
                    .gap(px(6.))
                    .child(small(format!("go-{i}"), "Go to tab  ↵".into()).on_click(cx.listener(move |this, _, window, cx| {
                        this.close_overlay(window, cx);
                        this.activate(tab, window, cx);
                    })))
                    .child(small(format!("stop-{i}"), "Stop  ⌫".into()).on_click(cx.listener(move |this, _, _, cx| this.stop_running(tab, cx))))
                    .into_any_element(),
                None => div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(1.))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .child(div().text_color(tone).child(state))
                            .child(div().font_family("JetBrains Mono").text_color(ui.text_tertiary).child(ago(r.since))),
                    )
                    .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(consequence(r)))
                    .into_any_element(),
            };
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("run-{i}")))
                    .h(px(44.))
                    .px(px(10.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .when(on, |d| d.bg(ui.bg_selected))
                    .when(!on, |d| d.hover(move |d| d.bg(ui.bg_hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Overlay::QuitRunning { expanded, .. } = this.overlay {
                            this.overlay = Overlay::QuitRunning { expanded, selected: i };
                            cx.notify();
                        }
                    }))
                    .child(crate::app_icon::badge(t, ui, 18.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child(t.title()))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(5.))
                                    .text_size(px(11.5))
                                    .text_color(ui.text_tertiary)
                                    .child(div().flex_none().child(ws.group_label(&t.group)))
                                    .child("·")
                                    .child(div().min_w_0().truncate().font_family("JetBrains Mono").child(r.command.clone())),
                            ),
                    )
                    .child(div().flex_none().child(right)),
            );
        }
        c = c.child(rows).child(
            div().text_size(px(11.)).text_color(ui.text_tertiary).child("↑↓ move · ↵ go to tab · ⌫ stop · ⌘↵ quit anyway · esc back"),
        );
    }

    c.child(
        row()
            .when(!expanded, |d| {
                d.child(
                    div()
                        .id("quit-show")
                        .text_size(px(13.))
                        .text_color(ui.text_secondary)
                        .hover(move |d| d.text_color(ui.text_primary))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.show_running(cx)))
                        .child("Show running"),
                )
            })
            .child(div().flex_1())
            .child(button(ws, "quit-cancel", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
            .child(button(ws, "quit-anyway", if expanded { "Quit anyway  ⌘↵" } else { "Quit anyway  ↵" }, true).on_click(cx.listener(|this, _, _, cx| this.quit_gracefully(cx)))),
    )
    .into_any_element()
}
