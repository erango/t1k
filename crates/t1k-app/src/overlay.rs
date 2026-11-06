//! Overlays: ⌘P switcher, ⌘O overview, rename field, and the tab context menu.

use std::time::{Duration, SystemTime};

use gpui::{
    div, prelude::*, px, AnyElement, ClipboardItem, Context, Div, FontWeight, KeyDownEvent, MouseButton, Pixels,
    Point, SharedString, Window,
};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as MatchConfig, Matcher, Utf32Str};
use t1k_agent::Status;

use crate::groups;
use crate::input::{self, Key};
use crate::workspace::Workspace;

pub enum Overlay {
    None,
    Switcher { query: String, selected: usize },
    Overview { selected: usize },
    Rename { tab: usize, text: String },
    RenameGroup { key: std::path::PathBuf, text: String },
    Menu { tab: usize, at: Point<Pixels> },
    Settings { section: crate::settings::Section },
}

const CARD_W: f32 = 264.;
const CARD_H: f32 = 164.;
const GAP: f32 = 12.;

impl Workspace {
    /// Everything we know about a tab, as one searchable line.
    fn haystack(&self, i: usize) -> String {
        let t = &self.tabs[i];
        let mut s = format!("{} {} {}", t.title(), self.group_label(&t.group), t.subtitle());
        if let Some(a) = &t.agent {
            for x in [&a.branch, &a.first_prompt, &a.recap, &a.goal].into_iter().flatten() {
                s.push(' ');
                s.push_str(x);
            }
            for (n, _) in &a.prs {
                s.push_str(&format!(" #{n} PR{n}"));
            }
        }
        if let Some(c) = &t.cwd {
            s.push(' ');
            s.push_str(&groups::tilde(c));
        }
        s
    }

    /// Switcher ranking: match score, then needs-you first, then most recently used.
    pub fn switcher_results(&self, query: &str) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.tabs.len()).collect();
        if query.trim().is_empty() {
            idx.sort_by_key(|&i| (self.tabs[i].urgency().min(2), std::cmp::Reverse(self.tabs[i].last_active)));
            return idx;
        }
        let hays: Vec<String> = idx.iter().map(|&i| self.haystack(i)).collect();
        let mut scored: Vec<(u32, usize)> =
            fuzzy(query, &hays).into_iter().map(|(s, k)| (s, idx[k])).collect();
        scored.sort_by_key(|&(s, i)| {
            (std::cmp::Reverse(s), self.tabs[i].urgency().min(2), std::cmp::Reverse(self.tabs[i].last_active))
        });
        scored.into_iter().map(|(_, i)| i).collect()
    }

    pub fn open_overlay(&mut self, o: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = o;
        window.focus(&self.overlay_focus);
        cx.notify();
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = Overlay::None;
        if let Some(t) = self.tabs.get(self.active) {
            let h = gpui::Focusable::focus_handle(t.view.read(cx), cx);
            window.focus(&h);
        }
        cx.notify();
    }

    fn overview_cols(window: &Window) -> usize {
        let w = f32::from(window.viewport_size().width) - 64.;
        ((w + GAP) / (CARD_W + GAP)).floor().max(1.) as usize
    }

    pub fn overlay_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        // Work on the overlay by value, then put it back unless the key closed it.
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        let next = match overlay {
            Overlay::None => None,
            Overlay::Switcher { mut query, mut selected } => match input::handle(&mut query, ev) {
                Key::Cancel => None,
                Key::Submit => {
                    let pick = self.switcher_results(&query).get(selected).copied();
                    self.close_overlay(window, cx);
                    if let Some(i) = pick {
                        self.activate(i, window, cx);
                    }
                    return;
                }
                key => {
                    let n = self.switcher_results(&query).len();
                    match key {
                        Key::Edited => selected = 0,
                        Key::Up => selected = selected.saturating_sub(1),
                        Key::Down => selected = (selected + 1).min(n.saturating_sub(1)),
                        _ => {}
                    }
                    Some(Overlay::Switcher { query, selected })
                }
            },
            Overlay::Overview { mut selected } => {
                let order = self.visual_order();
                let cols = Self::overview_cols(window);
                let last = order.len().saturating_sub(1);
                match ev.keystroke.key.as_str() {
                    "escape" => None,
                    "enter" => {
                        let pick = order.get(selected).copied();
                        self.close_overlay(window, cx);
                        if let Some(i) = pick {
                            self.activate(i, window, cx);
                        }
                        return;
                    }
                    k => {
                        match k {
                            "left" => selected = selected.saturating_sub(1),
                            "right" => selected = (selected + 1).min(last),
                            "up" => selected = selected.saturating_sub(cols),
                            "down" => selected = (selected + cols).min(last),
                            _ => {
                                // Typing in the overview starts a search.
                                let mut q = String::new();
                                if let Key::Edited = input::handle(&mut q, ev) {
                                    self.overlay = Overlay::Switcher { query: q, selected: 0 };
                                    cx.notify();
                                    return;
                                }
                            }
                        }
                        Some(Overlay::Overview { selected })
                    }
                }
            }
            Overlay::Rename { tab, mut text } => match input::handle(&mut text, ev) {
                Key::Submit => {
                    let text = text.trim().to_string();
                    if let Some(t) = self.tabs.get_mut(tab) {
                        t.manual_title = (!text.is_empty()).then_some(text);
                    }
                    self.save();
                    None
                }
                Key::Cancel => None,
                _ => Some(Overlay::Rename { tab, text }),
            },
            Overlay::RenameGroup { key, mut text } => match input::handle(&mut text, ev) {
                Key::Submit => {
                    let text = text.trim().to_string();
                    if text.is_empty() || text == groups::label(&key) {
                        self.group_names.remove(&key);
                    } else {
                        self.group_names.insert(key, text);
                    }
                    self.save();
                    None
                }
                Key::Cancel => None,
                _ => Some(Overlay::RenameGroup { key, text }),
            },
            Overlay::Menu { tab, at } => (ev.keystroke.key != "escape").then_some(Overlay::Menu { tab, at }),
            Overlay::Settings { section } => (ev.keystroke.key != "escape").then_some(Overlay::Settings { section }),
        };
        match next {
            Some(o) => {
                self.overlay = o;
                cx.notify();
            }
            None => self.close_overlay(window, cx),
        }
    }

    pub fn start_rename(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.tabs[tab].title();
        self.open_overlay(Overlay::Rename { tab, text }, window, cx);
    }

    pub fn set_reminder(&mut self, tab: usize, at: Option<SystemTime>, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get_mut(tab) {
            t.reminder_at = at;
            t.reminder_due = false;
        }
        self.save();
        cx.notify();
    }
}

/// Fuzzy-match `query` against each haystack. Returns (score, index) for matches.
/// Space-separated words must all match, in any order.
pub fn fuzzy(query: &str, hays: &[String]) -> Vec<(u32, usize)> {
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(MatchConfig::DEFAULT);
    let mut buf = Vec::new();
    hays.iter()
        .enumerate()
        .filter_map(|(i, h)| pattern.score(Utf32Str::new(h, &mut buf), &mut matcher).map(|s| (s, i)))
        .collect()
}

/// Seconds until 9:00 tomorrow, local time.
fn until_tomorrow_9() -> Duration {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_mday += 1;
        tm.tm_hour = 9;
        tm.tm_min = 0;
        tm.tm_sec = 0;
        let target = libc::mktime(&mut tm);
        Duration::from_secs((target - now).max(60) as u64)
    }
}

fn scrim(ws: &Workspace, close_on_click: bool, cx: &mut Context<Workspace>) -> gpui::Stateful<Div> {
    div()
        .id("overlay")
        .absolute()
        .inset_0()
        .track_focus(&ws.overlay_focus)
        .on_key_down(cx.listener(Workspace::overlay_key))
        .when(close_on_click, |d| d.on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
}

pub fn render(ws: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    match &ws.overlay {
        Overlay::None => None,
        Overlay::Switcher { query, selected } => {
            let results = ws.switcher_results(query);
            let selected = *selected;
            let rows = results.iter().take(9).enumerate().map(|(n, &i)| {
                let t = &ws.tabs[i];
                let is_sel = n == selected;
                div()
                    .id(SharedString::from(format!("sw-{i}")))
                    .h(px(54.))
                    .px(px(14.))
                    .rounded(px(8.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(2.))
                    .when(is_sel, |d| d.bg(ui.bg_selected))
                    .hover(move |d| d.bg(ui.bg_hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_overlay(window, cx);
                        this.activate(i, window, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .items_center()
                            .child(status_dot(t.status(), t.unread || t.reminder_due, ui))
                            .child(div().flex_1().truncate().text_size(px(14.)).text_color(ui.text_primary).child(t.title()))
                            .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(ws.group_label(&t.group))),
                    )
                    .child(div().pl(px(16.)).truncate().text_size(px(12.)).text_color(ui.text_quiet).child(t.subtitle()))
            });
            let panel = div()
                .w(px(720.))
                .max_h(px(560.))
                .mt(px(90.))
                .p(px(8.))
                .rounded(px(12.))
                .bg(ui.bg_raised)
                .border_1()
                .border_color(ui.border)
                .shadow_lg()
                .flex()
                .flex_col()
                .gap(px(4.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(44.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .text_size(px(16.))
                        .text_color(if query.is_empty() { ui.text_tertiary } else { ui.text_primary })
                        .child(if query.is_empty() { "Find a tab: title, project, branch, PR, file…".to_string() } else { format!("{query}▏") }),
                )
                .child(div().h(px(1.)).bg(ui.border).mb(px(4.)))
                .children(rows)
                .when(results.is_empty(), |d| {
                    d.child(div().p(px(14.)).text_size(px(13.)).text_color(ui.text_tertiary).child("No matching tabs"))
                });
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.25)).flex().justify_center().child(panel).into_any_element())
        }
        Overlay::Overview { selected } => {
            let order = ws.visual_order();
            let sel = order.get(*selected).copied();
            let mut grid = div().flex().flex_wrap().gap(px(GAP)).content_start();
            for &i in &order {
                grid = grid.child(card(ws, i, sel == Some(i), cx));
            }
            Some(
                scrim(ws, false, cx)
                    .bg(ui.bg_chrome.opacity(0.97))
                    .flex()
                    .flex_col()
                    .p(px(32.))
                    .pt(px(48.))
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Overview"))
                            .child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("Arrows to move · Enter to open · type to search · Esc")),
                    )
                    .child(div().id("overview-scroll").flex_1().overflow_y_scroll().child(grid))
                    .into_any_element(),
            )
        }
        Overlay::Rename { .. } | Overlay::RenameGroup { .. } => {
            let (title, text, hint) = match &ws.overlay {
                Overlay::Rename { tab, text } => (
                    format!("Rename tab in {}", ws.tabs.get(*tab).map(|t| ws.group_label(&t.group)).unwrap_or_default()),
                    text,
                    "Enter to save · empty resets to the automatic title · Esc",
                ),
                Overlay::RenameGroup { key, text } => (
                    format!("Rename group · folder {}", groups::tilde(key)),
                    text,
                    "Enter to save · empty resets to the folder name · Esc",
                ),
                _ => unreachable!(),
            };
            let panel = div()
                .w(px(420.))
                .mt(px(120.))
                .p(px(14.))
                .rounded(px(12.))
                .bg(ui.bg_raised)
                .border_1()
                .border_color(ui.border)
                .shadow_lg()
                .flex()
                .flex_col()
                .gap(px(8.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().text_size(px(12.)).text_color(ui.text_tertiary).child(title))
                .child(
                    div()
                        .h(px(34.))
                        .px(px(10.))
                        .rounded(px(6.))
                        .bg(ui.bg_control)
                        .flex()
                        .items_center()
                        .text_size(px(14.))
                        .text_color(ui.text_primary)
                        .child(format!("{text}▏")),
                )
                .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(hint));
            Some(scrim(ws, true, cx).flex().justify_center().child(panel).into_any_element())
        }
        Overlay::Settings { section } => {
            let panel = crate::settings::render(ws, *section, window, cx);
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.3)).flex().justify_center().child(panel).into_any_element())
        }
        Overlay::Menu { tab, at } => {
            let m = menu(ws, *tab, *at, window, cx);
            Some(scrim(ws, true, cx).child(m).into_any_element())
        }
    }
}

fn status_dot(status: Option<&Status>, loud_accent: bool, ui: t1k_themes::UiColors) -> Div {
    let color = match status {
        Some(Status::NeedsInput { .. }) => Some(ui.status_needs),
        Some(Status::Error { .. }) => Some(ui.status_error),
        Some(Status::Working) => Some(ui.status_working),
        _ if loud_accent => Some(ui.accent),
        _ => None,
    };
    let d = div().size(px(8.)).flex_none().rounded_full();
    match color {
        Some(c) => d.bg(c),
        None => d.border_1().border_color(ui.text_disabled),
    }
}

fn status_text(t: &crate::workspace::Tab) -> String {
    match t.status() {
        Some(Status::Working) => "Working".into(),
        Some(Status::NeedsInput { .. }) => "Needs you".into(),
        Some(Status::Error { .. }) => "Error".into(),
        Some(Status::Done) if t.unread => "Done · unread".into(),
        Some(Status::Done) => "Done".into(),
        Some(Status::Idle) => "Idle".into(),
        Some(Status::Ended) => "Ended".into(),
        None if t.program.is_some() => t.program.clone().unwrap_or_default(),
        None => "Shell".into(),
    }
}

fn card(ws: &Workspace, i: usize, selected: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let t = &ws.tabs[i];
    let since = t.agent.as_ref().map(|a| {
        let s = a.since.elapsed().as_secs();
        if s < 60 { format!("{s}s") } else if s < 3600 { format!("{}m", s / 60) } else { format!("{}h", s / 3600) }
    });
    let mut facts: Vec<String> = Vec::new();
    if let Some(a) = &t.agent {
        if let Some(b) = &a.branch {
            facts.push(format!("⎇ {b}"));
        }
        for (n, _) in a.prs.iter().take(3) {
            facts.push(format!("PR #{n}"));
        }
    }
    let rim = match t.status() {
        Some(Status::NeedsInput { .. }) => Some(ui.status_needs),
        Some(Status::Error { .. }) => Some(ui.status_error),
        _ => None,
    };
    div()
        .id(SharedString::from(format!("card-{}", t.id)))
        .w(px(CARD_W))
        .h(px(CARD_H))
        .p(px(12.))
        .rounded(px(9.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(if selected { ui.accent } else { rim.map(|c| c.opacity(0.45)).unwrap_or(ui.border) })
        .when(selected, |d| d.shadow_md())
        .hover(move |d| d.bg(ui.bg_hover))
        .flex()
        .flex_col()
        .gap(px(6.))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            this.activate(i, window, cx);
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(ui.text_tertiary)
                .child(status_dot(t.status(), t.unread || t.reminder_due, ui))
                .child(status_text(t))
                .child(div().flex_1())
                .children(since)
                .child(div().child(ws.group_label(&t.group))),
        )
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui.text_primary)
                .line_clamp(2)
                .child(t.title()),
        )
        .child(div().flex_1().text_size(px(12.)).text_color(ui.text_secondary).line_clamp(3).child(t.subtitle()))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .text_size(px(10.))
                .font_family("JetBrains Mono")
                .text_color(ui.text_tertiary)
                .children(facts),
        )
        .into_any_element()
}

fn menu(ws: &Workspace, tab: usize, at: Point<Pixels>, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let Some(t) = ws.tabs.get(tab) else { return div().into_any_element() };
    let item = |id: &str, label: String| {
        div()
            .id(SharedString::from(format!("menu-{id}")))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(ui.text_primary)
            .hover(move |d| d.bg(ui.accent.opacity(0.25)))
            .child(label)
    };
    let sep = || div().h(px(1.)).my(px(4.)).mx(px(6.)).bg(ui.border);
    let session = t.agent.as_ref().map(|a| a.session_id.clone()).filter(|s| !s.is_empty()).or_else(|| t.last_session.clone());
    let ended = t.agent.as_ref().is_none_or(|a| a.status == Status::Ended) && t.program.is_none();
    let branch = t.agent.as_ref().and_then(|a| a.branch.clone());
    let cwd = t.cwd.clone();

    let mut m = div()
        .absolute()
        .left(at.x)
        .top(at.y)
        .w(px(240.))
        .p(px(5.))
        .rounded(px(8.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    let unread = t.unread;
    m = m.child(item("unread", if unread { "Mark as read".into() } else { "Mark as unread".into() }).on_click(
        cx.listener(move |this, _, window, cx| {
            if let Some(t) = this.tabs.get_mut(tab) {
                t.unread = !unread;
            }
            this.close_overlay(window, cx);
        }),
    ));
    m = m.child(item("rename", "Rename…".into()).on_click(cx.listener(move |this, _, window, cx| this.start_rename(tab, window, cx))));
    if t.manual_title.is_some() {
        m = m.child(item("reset", "Reset to automatic title".into()).on_click(cx.listener(move |this, _, window, cx| {
            if let Some(t) = this.tabs.get_mut(tab) {
                t.manual_title = None;
            }
            this.save();
            this.close_overlay(window, cx);
        })));
    }
    m = m.child(sep());
    m = m.child(div().px(px(10.)).pt(px(2.)).pb(px(4.)).text_size(px(11.)).text_color(ui.text_tertiary).child("Remind me in"));
    let mut chips = div().flex().flex_wrap().gap(px(4.)).px(px(8.)).pb(px(4.));
    let presets: [(&str, Duration); 5] = [
        ("15m", Duration::from_secs(15 * 60)),
        ("30m", Duration::from_secs(30 * 60)),
        ("1h", Duration::from_secs(3600)),
        ("3h", Duration::from_secs(3 * 3600)),
        ("Tomorrow 9:00", until_tomorrow_9()),
    ];
    for (label, d) in presets {
        chips = chips.child(
            div()
                .id(SharedString::from(format!("rem-{label}")))
                .px(px(8.))
                .h(px(22.))
                .rounded(px(5.))
                .bg(ui.bg_control)
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(ui.text_primary)
                .hover(move |d| d.bg(ui.accent.opacity(0.3)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_reminder(tab, Some(SystemTime::now() + d), cx);
                    this.close_overlay(window, cx);
                }))
                .child(label),
        );
    }
    m = m.child(chips);
    if t.reminder_at.is_some() || t.reminder_due {
        m = m.child(item("clear-rem", "Clear reminder".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.set_reminder(tab, None, cx);
            this.close_overlay(window, cx);
        })));
    }
    m = m.child(sep());
    if let Some(sid) = session.clone() {
        if ended {
            let sid2 = sid.clone();
            m = m.child(item("resume", "Resume Claude session".into()).on_click(cx.listener(move |this, _, window, cx| {
                this.resume_session(tab, &sid2, cx);
                this.close_overlay(window, cx);
            })));
        }
        m = m.child(item("copy-resume", "Copy resume command".into()).on_click(cx.listener(move |this, _, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(format!("claude --resume {sid}")));
            this.close_overlay(window, cx);
        })));
    }
    if let Some(b) = branch {
        m = m.child(item("copy-branch", "Copy branch".into()).on_click(cx.listener(move |this, _, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(b.clone()));
            this.close_overlay(window, cx);
        })));
    }
    if let Some(dir) = cwd {
        m = m.child(item("reveal", "Reveal in Finder".into()).on_click(cx.listener(move |this, _, window, cx| {
            cx.reveal_path(&dir);
            this.close_overlay(window, cx);
        })));
    }
    m = m.child(sep());
    m = m.child(item("close", "Close tab".into()).on_click(cx.listener(move |this, _, window, cx| {
        this.close_overlay(window, cx);
        this.close_tab_at(tab, window, cx);
    })));

    // Keep the menu inside the window.
    let vh = f32::from(window.viewport_size().height);
    let top = f32::from(at.y).min(vh - 340.).max(8.);
    m.top(px(top)).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::fuzzy;

    #[test]
    fn finds_tabs_by_any_field() {
        let hays = vec![
            "Fix login bug auth ~/git-repos/web feature/login".to_string(),
            "Build calm sidebar t1k Editing src/sidebar.rs #23 PR23".to_string(),
            "~ zsh".to_string(),
        ];
        let best = |q: &str| {
            let mut r = fuzzy(q, &hays);
            r.sort_by(|a, b| b.0.cmp(&a.0));
            r.first().map(|x| x.1)
        };
        assert_eq!(best("login"), Some(0));
        assert_eq!(best("pr23"), Some(1));
        assert_eq!(best("side t1k"), Some(1));
        assert_eq!(best("zzzz"), None);
    }

    #[test]
    fn tomorrow_is_in_the_future() {
        let d = super::until_tomorrow_9().as_secs();
        assert!(d > 0 && d <= 33 * 3600);
    }
}
