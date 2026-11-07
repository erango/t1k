//! Closing tabs from the sidebar (canvas v3-r, option a′): an × on hover, a gentle close for
//! running programs, an undo toast, and ⌘⇧T to reopen what you closed.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Window};
use t1k_agent::Status;

use crate::workspace::Workspace;

const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "login", "nu", "xonsh"];
const TOAST: Duration = Duration::from_secs(6);
const KEEP: usize = 20;

/// Enough of a closed tab to bring it back.
#[derive(Clone, Debug)]
pub struct ClosedTab {
    pub cwd: Option<PathBuf>,
    pub manual_title: Option<String>,
    pub title: String,
    pub pinned_group: Option<PathBuf>,
    pub session: Option<String>,
    /// Its scrollback, replayed when it reopens (no banner).
    pub scrollback: Vec<u8>,
}

impl Workspace {
    fn foreground_program(&self, ix: usize, cx: &gpui::App) -> Option<String> {
        let n = self.tabs.get(ix)?.view.read(cx).terminal.foreground_name()?;
        (!SHELLS.contains(&n.trim_start_matches('-'))).then_some(n)
    }

    /// The × (and middle-click, and the menu's Close tab): a running program asks first, in
    /// the row (a Claude session only while it's working or waiting on you); an idle Claude
    /// session ends cleanly and can come back from Recent. Undo works either way.
    pub fn request_close(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(ix) else { return };
        let id = t.id.clone();
        if self.foreground_program(ix, cx).is_none() {
            self.close_with_undo(ix, window, cx);
            return;
        }
        let claude = self.claude_tab(ix);
        let busy = matches!(t.status(), Some(Status::Working) | Some(Status::NeedsInput { .. }));
        if claude && !busy {
            self.close_claude_gently(id, window, cx);
        } else {
            self.confirm_close = Some(id);
            cx.notify();
        }
    }

    pub fn claude_tab(&self, ix: usize) -> bool {
        self.tabs[ix].agent.as_ref().is_some_and(|a| a.kind == t1k_agent::agents::AgentKind::Claude && !a.session_id.is_empty())
    }

    /// Stop in the row's confirm: Claude exits cleanly first; anything else just closes
    /// (closing the tab hangs it up, as closing a terminal window does).
    pub fn confirm_stop(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_close = None;
        if self.claude_tab(ix) {
            let id = self.tabs[ix].id.clone();
            self.close_claude_gently(id, window, cx);
        } else {
            self.close_with_undo(ix, window, cx);
        }
    }

    /// Claude asks for a second Ctrl-C to exit; then close, so its session is saved for Recent.
    fn close_claude_gently(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.iter().find(|t| t.id == id) {
            t.view.read(cx).terminal.write(b"\x03".to_vec());
        }
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(350)).await;
            let _ = this.update(cx, |ws, cx| {
                if let Some(t) = ws.tabs.iter().find(|t| t.id == id) {
                    t.view.read(cx).terminal.write(b"\x03".to_vec());
                }
            });
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = this.update_in(cx, |ws, window, cx| {
                if let Some(ix) = ws.tabs.iter().position(|t| t.id == id) {
                    ws.close_with_undo(ix, window, cx);
                }
            });
        })
        .detach();
    }

    /// Close and remember the tab so Undo / ⌘⇧T can bring it back.
    pub fn close_with_undo(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(ix) else { return };
        let closed = ClosedTab {
            cwd: t.view.read(cx).terminal.cwd().or_else(|| t.cwd.clone()),
            manual_title: t.manual_title.clone(),
            title: t.title(),
            pinned_group: t.pinned_group.clone(),
            session: t.agent.as_ref().map(|a| a.session_id.clone()).filter(|s| !s.is_empty()).or_else(|| t.last_session.clone()),
            scrollback: t.view.read(cx).terminal.snapshot(5_000),
        };
        if self.confirm_close.as_deref() == Some(t.id.as_str()) {
            self.confirm_close = None;
        }
        self.closed.push(closed.clone());
        if self.closed.len() > KEEP {
            self.closed.remove(0);
        }
        self.close_tab_at(ix, window, cx);
        self.close_toast = Some((closed.title, Instant::now()));
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST).await;
            let _ = this.update(cx, |ws, cx| {
                if ws.close_toast.as_ref().is_some_and(|(_, at)| at.elapsed() >= TOAST) {
                    ws.close_toast = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// Undo / ⌘⇧T: reopen the last closed tab in its folder and group, with its scrollback;
    /// a Claude tab resumes its session. A program that was stopped isn't run again.
    pub fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.closed.pop() else { return };
        self.close_toast = None;
        let cwd = c.cwd.filter(|p| p.is_dir());
        // Its output comes back; the shell starts fresh (its history file still has what
        // you typed, so ↑ finds it), with nothing typed in.
        self.restore_next = Some(c.scrollback);
        self.open_tab(cwd, c.manual_title, window, cx);
        let ix = self.tabs.len() - 1;
        if let Some(g) = c.pinned_group {
            self.move_tab(ix, Some(g), cx);
        }
        if let Some(sid) = c.session {
            self.tabs[ix].last_session = Some(sid.clone());
            self.tabs[ix].view.read(cx).terminal.write(format!("claude --resume {sid}\r").into_bytes());
            self.recent.retain(|r| r.session_id != sid);
        }
        self.save();
        cx.notify();
    }
}

/// The × at a row's right end, shown while the row is hovered.
pub fn close_button(ws: &Workspace, ix: usize, row_group: gpui::SharedString, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let keys = crate::keymap::display_for(&crate::actions::CloseTab, &ws.config.keys);
    div()
        .id(gpui::SharedString::from(format!("close-{ix}")))
        // No width at rest, so titles keep their room; on hover it takes 20 px and the
        // right-end content (wait time, spinner) shifts left instead of being covered.
        .w(px(0.))
        .h(px(20.))
        .flex_none()
        .overflow_hidden()
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .invisible()
        .group_hover(row_group, |s| s.visible().w(px(20.)))
        .hover(move |d| d.bg(ui.bg_selected))
        .tooltip(crate::tooltip::text(ui, format!("Close tab  {keys}")))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.request_close(ix, window, cx);
        }))
        .child(crate::icons::close(ui.text_secondary))
}

/// "Stop npm run dev?" in place of a row's title, with Stop and Cancel.
pub fn confirm_row(ws: &Workspace, ix: usize, k: f32, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let what = if ws.claude_tab(ix) {
        "this Claude session".to_string()
    } else {
        let c = ws.tabs[ix].view.read(cx).terminal.foreground_command().map(|(c, _)| c).unwrap_or_else(|| "this program".into());
        c.split_whitespace().take(3).collect::<Vec<_>>().join(" ")
    };
    let small = |id: &'static str, label: &'static str, primary: bool| {
        div()
            .id(id)
            .h(px(22.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .text_size(px(11.5 * k))
            .when(primary, |d| d.bg(ui.status_error).text_color(gpui::white()).font_weight(FontWeight::MEDIUM))
            .when(!primary, |d| d.bg(ui.bg_control).text_color(ui.text_primary))
            .hover(|d| d.opacity(0.85))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(label)
    };
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.5 * k)).text_color(ui.text_primary).child(format!("Stop {what}?")))
        .child(small("confirm-cancel", "Cancel", false).on_click(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            this.confirm_close = None;
            cx.notify();
        })))
        .child(small("confirm-stop", "Stop", true).on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.confirm_stop(ix, window, cx);
        })))
        .into_any_element()
}

/// "Closed <title> · Undo ⌘⇧T" at the bottom of the sidebar for a few seconds.
pub fn toast(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let (title, _) = ws.close_toast.as_ref()?;
    let keys = crate::keymap::display_for(&crate::actions::ReopenClosedTab, &ws.config.keys);
    Some(
        div()
            .absolute()
            .left(px(10.))
            .right(px(10.))
            .bottom(px(10.))
            .h(px(34.))
            .px(px(12.))
            .rounded(px(8.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .shadow_lg()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .child(div().flex_1().min_w_0().truncate().text_color(ui.text_secondary).child(format!("Closed {title}")))
            .child(
                div()
                    .id("undo-close")
                    .flex_none()
                    .flex()
                    .gap(px(6.))
                    .text_color(ui.accent)
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.reopen_closed(window, cx)))
                    .child("Undo")
                    .child(div().font_family("JetBrains Mono").text_size(px(11.)).text_color(ui.text_tertiary).child(keys)),
            )
            .into_any_element(),
    )
}
