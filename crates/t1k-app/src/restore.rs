//! Tabs coming back after a quit (canvas v3-ab, option A). Shells start in a fraction of a
//! second, but Claude sessions take a few: `claude --resume` clears the screen and redraws, and
//! until its hooks fire again every row looks idle, so quiet folding would tuck the tabs away
//! and the sidebar looked emptied. While a restore runs, resuming tabs stay out of "+N quiet",
//! their rows say "resuming", the sidebar's top strip counts them back, and the selected pane
//! says what's happening instead of going blank.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight};
use t1k_themes::UiColors;

use crate::workspace::Workspace;

/// The count shows only when a restore is still running after this long (no flash on fast
/// launches).
const SHOW_AFTER: Duration = Duration::from_millis(400);
/// A resuming tab that hasn't reported by then is checked: running something counts as back,
/// otherwise it couldn't resume.
const GIVE_UP: Duration = Duration::from_secs(10);
/// "All 31 tabs back" stays this long, then fades.
const DONE_SHOWN: Duration = Duration::from_secs(2);
const FADE: Duration = Duration::from_millis(300);
/// Quiet folding waits this long after the restore, so nothing collapses right as it ends.
const FOLD_HOLD: Duration = Duration::from_secs(5);

pub struct Restore {
    pub started: Instant,
    pub total: usize,
    /// Tabs whose Claude session is resuming (ids).
    pub resuming: HashSet<String>,
    /// Every tab that was resuming, for the folding hold.
    pub all: HashSet<String>,
    /// Tabs that couldn't come back: (id, reason).
    pub failed: Vec<(String, &'static str)>,
    pub done_at: Option<Instant>,
}

impl Restore {
    pub fn new(total: usize, resuming: HashSet<String>, failed: Vec<(String, &'static str)>) -> Self {
        let done_at = resuming.is_empty().then(Instant::now);
        Self { started: Instant::now(), total, all: resuming.clone(), resuming, failed, done_at }
    }

    fn back(&self) -> usize {
        self.total - self.resuming.len() - self.failed.len()
    }
}

impl Workspace {
    /// Start the restore display after reopening tabs, and keep repainting until it's over.
    pub fn start_restore(&mut self, resuming: HashSet<String>, failed: Vec<(String, &'static str)>, cx: &mut Context<Self>) {
        if resuming.is_empty() && failed.is_empty() {
            return;
        }
        self.restore = Some(Restore::new(self.tabs.len(), resuming, failed));
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            let more = this.update(cx, |ws, cx| ws.tick_restore(cx)).unwrap_or(false);
            if !more {
                break;
            }
        })
        .detach();
    }

    /// Returns whether the restore display still needs repaints.
    fn tick_restore(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(r) = &mut self.restore else { return false };
        if r.done_at.is_none() && r.started.elapsed() > GIVE_UP {
            for id in r.resuming.drain().collect::<Vec<_>>() {
                let running = self.tabs.iter().find(|t| t.id == id).is_some_and(|t| t.program.is_some());
                if !running {
                    r.failed.push((id, "Couldn't resume"));
                }
            }
        }
        if r.done_at.is_none() && r.resuming.is_empty() {
            r.done_at = Some(Instant::now());
        }
        cx.notify();
        match r.done_at {
            // Failures keep the line up until they're handled; nothing else moves, so stop
            // ticking.
            Some(_) if !r.failed.is_empty() => false,
            Some(t) => t.elapsed() < DONE_SHOWN + FADE + FOLD_HOLD,
            None => true,
        }
    }

    /// A hook arrived from tab `id`: its session is live again.
    pub fn restore_heard(&mut self, id: &str) {
        if let Some(r) = &mut self.restore {
            r.resuming.remove(id);
        }
    }

    /// The user looked at a failed tab, or closed it: it no longer needs a look.
    pub fn restore_handled(&mut self, id: &str) {
        if let Some(r) = &mut self.restore {
            r.failed.retain(|(f, _)| f != id);
        }
    }

    pub fn is_resuming(&self, id: &str) -> bool {
        self.restore.as_ref().is_some_and(|r| r.resuming.contains(id))
    }

    pub fn restore_failure(&self, id: &str) -> Option<&'static str> {
        self.restore.as_ref()?.failed.iter().find(|(f, _)| f == id).map(|(_, why)| *why)
    }

    /// A tab that was resuming stays out of quiet folding until the restore is over and a few
    /// seconds more, so nothing collapses as it comes back.
    pub fn restore_holds(&self, id: &str) -> bool {
        self.restore_failure(id).is_some()
            || self.restore.as_ref().is_some_and(|r| r.all.contains(id) && r.done_at.is_none_or(|t| t.elapsed() < DONE_SHOWN + FADE + FOLD_HOLD))
    }
}

/// "Restoring 12 of 31" in the sidebar's top strip, then "✓ All 31 tabs back", fading out.
pub fn progress_label(ws: &Workspace) -> Option<AnyElement> {
    let ui = ws.ui;
    let r = ws.restore.as_ref()?;
    if r.started.elapsed() < SHOW_AFTER && r.done_at.is_none() {
        return None;
    }
    let (text, opacity, color) = match r.done_at {
        None => (format!("Restoring {} of {}", r.back(), r.total), 1., ui.text_tertiary),
        Some(_) if !r.failed.is_empty() => {
            let n = r.failed.len();
            (format!("{} of {} back · {n} need{} a look", r.back(), r.total, if n == 1 { "s" } else { "" }), 1., ui.status_error)
        }
        Some(t) => {
            let e = t.elapsed();
            // A restore that was over before the count ever showed needs no "all back".
            if t.duration_since(r.started) < SHOW_AFTER || e > DONE_SHOWN + FADE {
                return None;
            }
            let o = if e < DONE_SHOWN { 1. } else { 1. - (e - DONE_SHOWN).as_secs_f32() / FADE.as_secs_f32() };
            (format!("✓ All {} tabs back", r.total), o.clamp(0., 1.), ui.text_tertiary)
        }
    };
    // Over the strip's buttons, clear of the traffic lights: a narrow sidebar has no room
    // beside them, and the count matters more for these few seconds. Fading reveals them.
    Some(
        div()
            .absolute()
            .left(px(74.))
            .right_0()
            .top_0()
            .bottom(px(2.))
            .bg(ui.bg_sidebar)
            .flex()
            .items_center()
            .justify_end()
            .pr(px(14.))
            .opacity(opacity)
            .child(div().truncate().text_size(px(11.5)).text_color(color).child(text))
            .into_any_element(),
    )
}

/// The 2 px bar under the top strip while tabs come back.
pub fn progress_bar(ws: &Workspace) -> Option<AnyElement> {
    let ui = ws.ui;
    let r = ws.restore.as_ref()?;
    if r.done_at.is_some() || r.started.elapsed() < SHOW_AFTER || r.total == 0 {
        return None;
    }
    let f = r.back() as f32 / r.total as f32;
    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(2.))
            .bg(ui.text_primary.opacity(0.08))
            .child(div().h_full().w(gpui::relative(f)).bg(ui.accent.opacity(0.7)))
            .into_any_element(),
    )
}

/// The right end of a row while its tab comes back: "resuming", or why it couldn't.
pub fn row_note(ws: &Workspace, id: &str, ui: UiColors) -> Option<gpui::Div> {
    if let Some(why) = ws.restore_failure(id) {
        return Some(div().flex_none().text_size(px(11.)).text_color(ui.status_error).child(why));
    }
    ws.is_resuming(id).then(|| div().flex_none().text_size(px(11.)).text_color(ui.text_tertiary).child("resuming"))
}

/// Over the selected pane while its Claude session resumes: the old output stays, dimmed, with
/// a calm line saying what's happening (Claude's screen clear never reads as a blank pane).
pub fn pane_overlay(ws: &Workspace, id: &str) -> Option<AnyElement> {
    let ui = ws.ui;
    let failed = ws.restore_failure(id);
    if !ws.is_resuming(id) && failed.is_none() {
        return None;
    }
    let text = match failed {
        Some("Folder missing") => "This tab's folder is gone, so it opened in your home folder.",
        Some(_) => "Couldn't resume this Claude session. Run claude --resume to pick one.",
        None => "Resuming Claude session…",
    };
    Some(
        div()
            .absolute()
            .inset_0()
            .bg(ui.bg_terminal.opacity(0.45))
            .flex()
            .flex_col()
            .justify_end()
            .items_center()
            .pb(px(18.))
            .child(
                div()
                    .h(px(30.))
                    .px(px(14.))
                    .rounded(px(15.))
                    .bg(ui.bg_raised)
                    .border_1()
                    .border_color(if failed.is_some() { ui.status_error.opacity(0.5) } else { ui.border })
                    .shadow_md()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .font_family(".SystemUIFont")
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(ui.text_secondary)
                    .when(failed.is_none(), |d| d.child(crate::sidebar::spinner_sized(ui.text_tertiary, ws.started, 5.)))
                    .child(text),
            )
            .into_any_element(),
    )
}
