//! What T1K shows for updates (canvas board V3Updates): the mercury Update pill, the update
//! dialog, and the quit prompt. Sparkle does the work; `updater` holds the state.
//!
//! The rule behind all of it: restarting a terminal stops every shell and agent in it, so T1K
//! never restarts on its own and always says what a restart would interrupt.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, SharedString, Window};
use t1k_agent::Status;

use crate::overlay::Overlay;
use crate::updater::{self, Phase, Snapshot};
use crate::workspace::Workspace;

const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// ⌘Q asks once per run while an update waits.
static QUIT_ASKED: AtomicBool = AtomicBool::new(false);

fn ago(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

fn mb(b: u64) -> String {
    format!("{:.1}", b as f64 / 1_000_000.)
}

/// One line for About and Settings: "Up to date · checked 2h ago", "Downloading 42%", ...
pub fn status_words(s: &Snapshot) -> String {
    if !updater::available() {
        return "Updates work in the installed app".into();
    }
    let v = s.version.clone().unwrap_or_default();
    match &s.phase {
        Phase::Checking => "Checking…".into(),
        Phase::Available => format!("{v} is available"),
        Phase::Downloading { got, total } if *total > 0 => format!("Downloading {v} · {}%", got * 100 / total),
        Phase::Downloading { .. } => format!("Downloading {v}…"),
        Phase::Extracting => format!("Verifying {v}…"),
        Phase::Ready => format!("{v} is ready · Restart to update"),
        Phase::Installing => format!("Installing {v}…"),
        Phase::Error(_) => "Couldn't check · will try again".into(),
        Phase::UpToDate | Phase::Idle => match s.last_check {
            Some(t) => format!("Up to date · checked {}", ago(t)),
            None => "Not checked yet".into(),
        },
    }
}

impl Workspace {
    /// Agents a restart or quit would stop: (working, needs you).
    pub fn agent_counts(&self) -> (usize, usize) {
        let mut working = 0;
        let mut needs = 0;
        for t in &self.tabs {
            match t.agent.as_ref().map(|a| &a.status) {
                Some(Status::Working) => working += 1,
                Some(Status::NeedsInput { .. }) => needs += 1,
                _ => {}
            }
        }
        (working, needs)
    }

    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        updater::check_now();
        self.open_overlay(Overlay::Update, window, cx);
    }

    pub fn open_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_overlay(Overlay::Update, window, cx);
    }

    /// Close the dialog, telling Sparkle the answer was seen (or deferred).
    pub fn close_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match updater::snapshot().phase {
            Phase::UpToDate | Phase::Error(_) => updater::acknowledge(),
            Phase::Checking => updater::cancel(),
            Phase::Available => updater::later(),
            _ => {}
        }
        self.close_overlay(window, cx);
    }

    /// ⌘Q: with an update waiting, ask once; otherwise quit.
    pub fn should_confirm_quit() -> bool {
        updater::snapshot().phase == Phase::Ready && !QUIT_ASKED.swap(true, Ordering::Relaxed)
    }
}

/// "3 working agents and 1 that needs you", or None when nothing would stop.
fn running(ws: &Workspace) -> Option<String> {
    let (w, n) = ws.agent_counts();
    let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    match (w, n) {
        (0, 0) => None,
        (w, 0) => Some(plural(w, "working agent", "working agents")),
        (0, n) => Some(plural(n, "agent that needs you", "agents that need you")),
        (w, n) => Some(format!("{} and {n} that {}", plural(w, "working agent", "working agents"), if n == 1 { "needs you" } else { "need you" })),
    }
}

/// The sidebar's top strip: a mercury pill while an update waits. News, not a problem, so
/// mercury (never amber) and it never animates.
pub fn pill(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !updater::waiting() {
        return None;
    }
    let ui = ws.ui;
    let v = updater::snapshot().version.unwrap_or_default();
    Some(
        div()
            .id("update-pill")
            .h(px(22.))
            .px(px(9.))
            .mr(px(4.))
            .flex()
            .items_center()
            .rounded(px(11.))
            .bg(ui.accent.opacity(0.16))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(ui.accent)
            .hover(move |d| d.bg(ui.accent.opacity(0.26)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.open_update(window, cx)))
            .tooltip(crate::tooltip::text(ui, if v.is_empty() { "An update is ready" } else { "A new version is ready" }))
            .child("Update")
            .into_any_element(),
    )
}

fn button(ws: &Workspace, id: &'static str, label: &str, primary: bool) -> gpui::Stateful<gpui::Div> {
    let ui = ws.ui;
    div()
        .id(id)
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .text_size(px(13.))
        .map(|d| if primary { d.bg(ui.accent).text_color(ui.bg_sidebar).font_weight(FontWeight::MEDIUM) } else { d.bg(ui.bg_control).text_color(ui.text_primary) })
        .hover(|d| d.opacity(0.9))
        .child(label.to_string())
}

fn card(ws: &Workspace) -> gpui::Div {
    let ui = ws.ui;
    div()
        .w(px(440.))
        .mt(px(110.))
        .p(px(22.))
        .rounded(px(14.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(12.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn heading(ws: &Workspace, title: String, sub: Option<String>) -> gpui::Div {
    let ui = ws.ui;
    div()
        .flex()
        .gap(px(12.))
        .items_center()
        .child(img_face())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(title))
                .children(sub.map(|s| div().text_size(px(12.)).text_color(ui.text_tertiary).child(s))),
        )
}

fn img_face() -> impl IntoElement {
    static FACE: std::sync::LazyLock<std::sync::Arc<gpui::Image>> = std::sync::LazyLock::new(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, include_bytes!("../../../resources/icon-256.png").to_vec()))
    });
    gpui::img(FACE.clone()).size(px(40.)).flex_none()
}

fn row() -> gpui::Div {
    div().pt(px(6.)).flex().items_center().gap(px(8.))
}

/// The update dialog, for whatever phase Sparkle is in.
pub fn dialog(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let s = updater::snapshot();
    let v = s.version.clone().unwrap_or_default();
    let close = |label: &'static str, primary: bool| {
        button(ws, "upd-close", label, primary).on_click(cx.listener(|this, _, window, cx| this.close_update(window, cx)))
    };
    let notes = |s: &Snapshot| {
        let mut d = div().flex().flex_col().gap(px(5.));
        if !s.notes.is_empty() {
            d = d.child(div().pt(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child("WHAT'S NEW"));
            for n in s.notes.iter().take(8) {
                d = d.child(
                    div().flex().gap(px(8.)).text_size(px(13.)).line_height(px(18.)).text_color(ui.text_secondary)
                        .child(div().flex_none().text_color(ui.text_quiet).child("•"))
                        .child(div().flex_1().min_w_0().child(n.clone())),
                );
            }
        }
        if let Some(url) = s.notes_url.clone() {
            d = d.child(
                div().id("upd-notes").pt(px(2.)).text_size(px(12.)).text_color(ui.accent).cursor_pointer()
                    .hover(|d| d.underline())
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .child("Full release notes ↗"),
            );
        }
        d
    };
    let skip = |label: &'static str| {
        div()
            .id("upd-skip")
            .text_size(px(12.))
            .text_color(ui.text_tertiary)
            .cursor_pointer()
            .hover(move |d| d.text_color(ui.text_primary))
            .on_click(cx.listener(|this, _, window, cx| {
                updater::skip();
                this.close_overlay(window, cx);
            }))
            .child(label)
    };

    if !updater::available() {
        return card(ws)
            .child(heading(ws, "Updates work in the installed app".into(), Some(format!("You have {CURRENT}"))))
            .child(div().text_size(px(13.)).text_color(ui.text_secondary).child("This copy of T1K runs outside T1K.app, so it can't update itself. Build and install with scripts/bundle.sh --install."))
            .child(row().justify_end().child(close("Close", true)))
            .into_any_element();
    }

    let body = match &s.phase {
        Phase::Idle | Phase::Checking => card(ws)
            .child(heading(ws, "Checking for updates…".into(), Some(format!("You have {CURRENT}"))))
            .child(row().justify_end().child(close("Cancel", false))),
        Phase::UpToDate => card(ws)
            .child(heading(ws, "T1K is up to date".into(), Some(format!("You have {CURRENT}, the latest version."))))
            .child(row().justify_end().child(close("OK", true))),
        Phase::Error(msg) => card(ws)
            .child(heading(ws, "Couldn't update".into(), None))
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(ui.text_secondary).child(format!("{msg} T1K will try again at the next check.")))
            .child(
                row()
                    .justify_end()
                    .child(button(ws, "upd-retry", "Try again", false).on_click(cx.listener(|this, _, window, cx| this.check_for_updates(window, cx))))
                    .child(close("Close", true)),
            ),
        Phase::Available => card(ws)
            .child(heading(ws, format!("T1K {v} is available"), Some(format!("You have {CURRENT}"))))
            .child(notes(&s))
            .child(
                row()
                    .child(skip("Skip this version"))
                    .child(div().flex_1())
                    .child(close("Later", false))
                    .child(button(ws, "upd-download", "Download", true).on_click(cx.listener(|_, _, _, cx| {
                        updater::download();
                        cx.notify();
                    }))),
            ),
        Phase::Downloading { got, total } => {
            let frac = if *total > 0 { (*got as f32 / *total as f32).min(1.) } else { 0. };
            card(ws)
                .child(heading(ws, format!("Downloading {v}"), Some(if *total > 0 { format!("{} / {} MB", mb(*got), mb(*total)) } else { "Starting…".into() })))
                .child(div().h(px(4.)).rounded(px(2.)).bg(ui.bg_control).child(div().h_full().rounded(px(2.)).bg(ui.accent).w(gpui::relative(frac))))
                .child(
                    row()
                        .child(div().flex_1().text_size(px(12.)).text_color(ui.text_tertiary).child("Keeps going in the background"))
                        .child(button(ws, "upd-hide", "Hide", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))),
                )
        }
        Phase::Extracting | Phase::Installing => card(ws)
            .child(heading(ws, if s.phase == Phase::Extracting { format!("Verifying {v}…") } else { format!("Installing {v}…") }, None))
            .child(row().justify_end().child(button(ws, "upd-hide", "Hide", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))),
        Phase::Ready => {
            let busy = running(ws);
            // With agents working, the safe choice is the default.
            let restart_primary = busy.is_none();
            card(ws)
                .child(heading(ws, format!("T1K {v} is ready"), Some(format!("You have {CURRENT} · downloaded and verified"))))
                .child(notes(&s))
                .children(busy.map(|b| {
                    div()
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(ui.status_needs.opacity(0.10))
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(ui.status_needs)
                        .child(format!("Restarting stops {b}. Tabs come back after; work in progress in those turns is lost."))
                }))
                .child(
                    row()
                        .when(updater::can_skip(), |d| d.child(skip("Skip this version")))
                        .child(div().flex_1())
                        .child(button(ws, "upd-onquit", "Install on quit", !restart_primary).on_click(cx.listener(|this, _, window, cx| {
                            updater::install_on_quit();
                            this.close_overlay(window, cx);
                        })))
                        .child(button(ws, "upd-restart", "Restart now", restart_primary).on_click(cx.listener(|this, _, _, cx| {
                            this.save();
                            updater::install_now();
                            cx.notify();
                        }))),
                )
        }
    };
    body.into_any_element()
}

/// ⌘Q with an update waiting.
pub fn quit_prompt(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let v = updater::snapshot().version.unwrap_or_default();
    let body = match running(ws) {
        Some(b) => format!("{} {} when T1K quits; tabs come back next time.", capitalize(&b), if b.starts_with("1 ") && !b.contains(" and ") { "stops" } else { "stop" }),
        None => "It installs as T1K quits; tabs come back next time.".into(),
    };
    card(ws)
        .child(heading(ws, format!("Quit T1K and install {v}?"), None))
        .child(div().text_size(px(13.)).line_height(px(18.)).text_color(ui.text_secondary).child(SharedString::from(body)))
        .child(
            row()
                .justify_end()
                .child(button(ws, "quit-cancel", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
                .child(button(ws, "quit-update", "Quit and update", true).on_click(|_, _, cx| cx.quit())),
        )
        .into_any_element()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
