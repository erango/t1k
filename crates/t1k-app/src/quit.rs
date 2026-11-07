//! Quitting with programs still running: a warning first, then a graceful shutdown that lets
//! each program finish its last output (saved with the scrollback) before T1K goes.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::{div, prelude::*, px, AnyElement, Context, SharedString, Window};

use crate::overlay::Overlay;
use crate::update_ui::{button, card, heading, row};
use crate::workspace::Workspace;

const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "login", "nu", "xonsh"];
/// How long programs get to exit before T1K quits anyway.
const GRACE: Duration = Duration::from_secs(3);

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

    /// ⌘Q: ask first when something is still running; otherwise quit.
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quitting.is_some() {
            return;
        }
        if self.running_programs(cx).is_empty() {
            cx.quit();
        } else {
            self.open_overlay(Overlay::QuitRunning, window, cx);
        }
    }

    /// Quit anyway, gently: Claude sessions get Ctrl-C twice (they exit cleanly and can be
    /// resumed), other programs a hangup like closing a terminal window. T1K waits up to 3 s
    /// for them, so their last words land in the saved scrollback, then quits.
    pub fn quit_gracefully(&mut self, cx: &mut Context<Self>) {
        let running = self.running_programs(cx);
        // Remember which Claude sessions were alive, so they resume next launch even though
        // they're about to exit.
        self.resume_after_quit = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, t)| running.iter().any(|(r, _)| r == i) && t.agent.as_ref().is_some_and(|a| a.kind == t1k_agent::agents::AgentKind::Claude && !a.session_id.is_empty()))
            .map(|(_, t)| t.id.clone())
            .collect::<HashSet<_>>();
        self.quitting = Some(Instant::now());
        for (i, _) in &running {
            let t = &self.tabs[*i];
            let term = &t.view.read(cx).terminal;
            if self.resume_after_quit.contains(&t.id) {
                term.write(b"\x03".to_vec());
            } else {
                term.hangup_foreground();
            }
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let start = Instant::now();
            // Claude asks for a second Ctrl-C to exit.
            cx.background_executor().timer(Duration::from_millis(350)).await;
            let _ = this.update(cx, |ws, cx| {
                for t in &ws.tabs {
                    if ws.resume_after_quit.contains(&t.id) {
                        t.view.read(cx).terminal.write(b"\x03".to_vec());
                    }
                }
            });
            while start.elapsed() < GRACE {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = this.update(cx, |ws, cx| ws.running_programs(cx).is_empty()).unwrap_or(true);
                if done {
                    // A moment for the last output to be drawn into the grid.
                    cx.background_executor().timer(Duration::from_millis(150)).await;
                    break;
                }
            }
            let _ = this.update(cx, |_, cx| cx.quit());
        })
        .detach();
    }
}

fn names(ws: &Workspace, running: &[(usize, String)]) -> String {
    let claude = running.iter().filter(|(i, _)| ws.tabs[*i].agent.is_some()).count();
    let mut parts: Vec<String> = Vec::new();
    if claude > 0 {
        parts.push(if claude == 1 { "1 Claude session".into() } else { format!("{claude} Claude sessions") });
    }
    for (i, n) in running.iter().filter(|(i, _)| ws.tabs[*i].agent.is_none()) {
        let _ = i;
        parts.push(n.clone());
    }
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => format!("{} and {}", parts[..parts.len() - 1].join(", "), parts[parts.len() - 1]),
    }
}

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let running = ws.running_programs(cx);
    if let Some(at) = ws.quitting {
        let n = running.len();
        return card(ws)
            .child(heading(ws, if n == 0 { "Quitting…".to_string() } else { format!("Closing {n} program{}…", if n == 1 { "" } else { "s" }) }, None))
            .child(div().text_size(px(13.)).text_color(ui.text_secondary).child(SharedString::from(names(ws, &running))))
            .when(at.elapsed() > GRACE, |d| {
                d.child(row().justify_end().child(button(ws, "quit-force", "Force quit", true).on_click(|_, _, cx| cx.quit())))
            })
            .into_any_element();
    }
    let tabs = running.len();
    if tabs == 0 {
        // Everything stopped while the dialog was open.
        return card(ws)
            .child(heading(ws, "Quit T1K?".to_string(), None))
            .child(div().text_size(px(13.)).text_color(ui.text_secondary).child("Nothing is running any more."))
            .child(
                row()
                    .justify_end()
                    .child(button(ws, "quit-cancel", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
                    .child(button(ws, "quit-anyway", "Quit", true).on_click(|_, _, cx| cx.quit())),
            )
            .into_any_element();
    }
    let claude = running.iter().any(|(i, _)| ws.tabs[*i].agent.is_some());
    let other = running.iter().any(|(i, _)| ws.tabs[*i].agent.is_none());
    let after = match (claude, other) {
        (true, true) => " Claude sessions pick up where they left off next time; other programs stop.",
        (true, false) => " They pick up where they left off next time.",
        _ => " They stop when T1K quits.",
    };
    let body = format!("{} {} still running: {}.{after}", tabs, if tabs == 1 { "tab is" } else { "tabs are" }, names(ws, &running));
    card(ws)
        .child(heading(ws, "Quit T1K?".to_string(), None))
        .child(div().text_size(px(13.)).line_height(px(18.)).text_color(ui.text_secondary).child(SharedString::from(body)))
        .child(
            row()
                .child(button(ws, "quit-show", "Show running", false).on_click(cx.listener(|this, _, window, cx| this.show_running(window, cx))))
                .child(div().flex_1())
                .child(button(ws, "quit-cancel", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
                .child(button(ws, "quit-anyway", "Quit anyway", true).on_click(cx.listener(|this, _, _, cx| this.quit_gracefully(cx)))),
        )
        .into_any_element()
}
