//! Bring your tabs over (canvas v3-v, layout B): the import sheet, grouped by the T1K group
//! each tab lands in; the welcome card; and the "still open in the other terminal" state of a
//! Claude session that resumes here once it exits there.

use std::path::PathBuf;
use std::time::Instant;

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, KeyDownEvent, MouseButton, SharedString, Window};

use t1k_themes::UiColors;

use crate::groups;
use crate::import::{self, Found, Source};
use crate::overlay::Overlay;
use crate::workspace::Workspace;

pub struct Row {
    pub found: Found,
    pub group: PathBuf,
    /// Already in T1K (same Claude session, imported from this tab before, or same folder and
    /// program): shown dimmed and unticked.
    pub already: bool,
    pub checked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// No iTerm2 or Terminal tabs: nothing to ask.
    None,
    Pending,
    Skipped,
    Done,
}

pub struct Sheet {
    pub rows: Vec<Row>,
    pub ask: Ask,
}

/// A Claude session still running in the terminal it came from. It resumes here once it exits
/// there: one session can't run in two places without mixing up its history.
#[derive(Clone, Debug)]
pub struct Waiting {
    pub pid: i32,
    pub session_id: String,
    pub source: Source,
    pub tty: String,
    pub busy: bool,
}

/// Where an imported tab came from ("Warp ttys004"), so a second import skips it.
pub fn origin(f: &Found) -> String {
    format!("{} {}", f.source.label(), f.tty)
}

/// What a found tab runs, in the terms T1K uses for its own tabs' `program`.
fn program_of(f: &Found) -> Option<String> {
    if f.claude.is_some() {
        return Some("claude".into());
    }
    f.running.as_ref().and_then(|r| r.split_whitespace().next()).map(str::to_string)
}

fn title_of(f: &Found) -> String {
    if let Some(t) = &f.title {
        return t.clone();
    }
    if let Some(n) = f.claude.as_ref().and_then(|c| c.name.clone()).filter(|n| !n.is_empty()) {
        return n;
    }
    if let Some(r) = &f.running {
        return r.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    }
    f.cwd.as_deref().map(groups::label).unwrap_or_else(|| "~".into())
}

fn apps(rows: &[Row]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for r in rows {
        let n = r.found.source.label();
        if !names.contains(&n) {
            names.push(n);
        }
    }
    match names.as_slice() {
        [] => String::new(),
        [a] => a.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl Workspace {
    /// Rows for what a scan found, with tabs already in T1K marked.
    fn sheet_rows(&self, found: Vec<Found>) -> Vec<Row> {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut rows: Vec<Row> = found
            .into_iter()
            .map(|f| {
                let cwd = f.cwd.clone().unwrap_or_else(|| home.clone());
                let group = groups::project_root(&cwd);
                let prog = program_of(&f);
                let key = origin(&f);
                let already = self.tabs.iter().any(|t| {
                    let same_session = f.claude.as_ref().is_some_and(|c| {
                        t.agent.as_ref().is_some_and(|a| a.session_id == c.session_id)
                            || t.last_session.as_deref() == Some(c.session_id.as_str())
                            || t.waiting.as_ref().is_some_and(|w| w.session_id == c.session_id)
                    });
                    let program = if t.agent.as_ref().is_some_and(|a| a.status != t1k_agent::Status::Ended) { Some("claude".to_string()) } else { t.program.clone() };
                    same_session || t.origin.as_deref() == Some(key.as_str()) || (t.cwd.as_deref() == Some(cwd.as_path()) && program == prog)
                });
                Row { found: f, group, already, checked: !already }
            })
            .collect();
        // By group, in order of first appearance; inside a group, the source app's order.
        let mut order: Vec<PathBuf> = Vec::new();
        for r in &rows {
            if !order.contains(&r.group) {
                order.push(r.group.clone());
            }
        }
        rows.sort_by_key(|r| (order.iter().position(|g| *g == r.group), r.found.order, r.found.tty.clone()));
        rows
    }

    pub fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let found = import::scan();
        let rows = self.sheet_rows(found);
        let asks = rows.iter().any(|r| matches!(r.found.source, Source::ITerm2 | Source::Terminal));
        self.import = Some(Sheet { rows, ask: if asks { Ask::Pending } else { Ask::None } });
        self.open_overlay(Overlay::Import { selected: 0 }, window, cx);
    }

    /// Scan in the background (at launch): fills the welcome card and the palette hint, and
    /// offers the import when there are tabs not seen before.
    pub fn scan_for_import(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async { import::scan() });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |ws, cx| {
                let fresh: Vec<Found> = ws.sheet_rows(found).into_iter().filter(|r| !r.already).map(|r| r.found).collect();
                let new = fresh.iter().any(|f| !ws.import_seen.contains(&origin(f)));
                ws.import_found = Some(fresh);
                if new && ws.config.terminal.offer_import && !matches!(ws.overlay, Overlay::Welcome) {
                    ws.import_offer = true;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Remember what was offered, so the launch offer only comes back for new tabs.
    fn mark_seen(&mut self) {
        if let Some(f) = &self.import_found {
            for o in f.iter().map(origin) {
                if !self.import_seen.contains(&o) {
                    self.import_seen.push(o);
                }
            }
        }
        if let Some(s) = &self.import {
            for o in s.rows.iter().map(|r| origin(&r.found)) {
                if !self.import_seen.contains(&o) {
                    self.import_seen.push(o);
                }
            }
        }
        self.import_offer = false;
        self.save();
    }

    pub fn dismiss_import_offer(&mut self, cx: &mut Context<Self>) {
        self.mark_seen();
        cx.notify();
    }

    fn import_allow_titles(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.import.as_mut() else { return };
        s.ask = Ask::Done;
        let mut sources: Vec<Source> = s.rows.iter().map(|r| r.found.source).collect();
        sources.sort();
        sources.dedup();
        let task = cx.background_executor().spawn(async move { import::titles(&sources) });
        cx.spawn(async move |this, cx| {
            let titles = task.await;
            let _ = this.update(cx, |ws, cx| {
                if let Some(s) = ws.import.as_mut() {
                    let mut found: Vec<Found> = s.rows.iter().map(|r| r.found.clone()).collect();
                    import::apply_titles(&mut found, &titles);
                    let got = found.iter().any(|f| f.order.is_some());
                    for (r, f) in s.rows.iter_mut().zip(found) {
                        r.found = f;
                    }
                    if !got {
                        // Denied (or the app said nothing): folder names, and the quiet hint.
                        s.ask = Ask::Skipped;
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_row(&mut self, i: usize) {
        if let Some(r) = self.import.as_mut().and_then(|s| s.rows.get_mut(i)) {
            r.checked = !r.checked;
        }
    }

    fn toggle_import_group(&mut self, g: &PathBuf) {
        let Some(s) = self.import.as_mut() else { return };
        let all = s.rows.iter().filter(|r| &r.group == g).all(|r| r.checked);
        for r in s.rows.iter_mut().filter(|r| &r.group == g) {
            r.checked = !all;
        }
    }

    pub fn import_key(&mut self, mut selected: usize, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        let n = self.import.as_ref().map(|s| s.rows.len()).unwrap_or(0);
        match ev.keystroke.key.as_str() {
            "escape" => {
                self.mark_seen();
                self.import = None;
                return None;
            }
            "up" => selected = selected.saturating_sub(1),
            "down" => selected = (selected + 1).min(n.saturating_sub(1)),
            "space" => self.toggle_row(selected),
            "enter" => {
                self.bring_over(window, cx);
                return None;
            }
            _ => {}
        }
        Some(Overlay::Import { selected })
    }

    /// Open the ticked tabs. Plain shells start in their folder; Claude sessions resume (now,
    /// or once they exit in the other terminal); other programs aren't restarted.
    pub fn bring_over(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mark_seen();
        let Some(sheet) = self.import.take() else { return };
        self.overlay = Overlay::None;
        let first = self.tabs.len();
        let mut n = 0;
        for r in sheet.rows.into_iter().filter(|r| r.checked) {
            let f = r.found;
            // An app's tab name, when it said one; Claude tabs take their session's title.
            let title = f.title.clone().filter(|_| f.claude.is_none());
            self.open_tab(f.cwd.clone(), title, window, cx);
            let ix = self.tabs.len() - 1;
            self.tabs[ix].origin = Some(origin(&f));
            if let Some(c) = &f.claude {
                self.tabs[ix].last_session = Some(c.session_id.clone());
                if import::alive(c.pid) {
                    self.tabs[ix].waiting = Some(Waiting { pid: c.pid, session_id: c.session_id.clone(), source: f.source, tty: f.tty.clone(), busy: c.busy });
                } else {
                    self.tabs[ix].view.read(cx).terminal.write(format!("claude --resume {}\r", c.session_id).into_bytes());
                }
            } else if let Some(r) = &f.running {
                self.tabs[ix].was_running = Some(r.clone());
            }
            n += 1;
        }
        if n > 0 {
            self.activate(first, window, cx);
            self.import_last = Some((n, crate::usage::now()));
            self.notice = Some((format!("Brought {n} tab{} over", if n == 1 { "" } else { "s" }), Instant::now()));
            self.notice_undo = false;
            self.import_found = Some(Vec::new());
        }
        self.save();
        cx.notify();
    }

    /// Poll: a waiting session resumes here the moment it exits there.
    pub fn poll_waiting(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for i in 0..self.tabs.len() {
            let Some(w) = self.tabs[i].waiting.clone() else { continue };
            if import::alive(w.pid) {
                let busy = import::claude_busy(w.pid).unwrap_or(w.busy);
                if busy != w.busy {
                    if let Some(x) = self.tabs[i].waiting.as_mut() {
                        x.busy = busy;
                    }
                    changed = true;
                }
                continue;
            }
            self.tabs[i].waiting = None;
            self.tabs[i].view.read(cx).terminal.write(format!("claude --resume {}\r", w.session_id).into_bytes());
            let title = self.tabs[i].title();
            self.notice = Some((format!("Resumed {title} from {}", w.source.label()), Instant::now()));
            self.notice_undo = false;
            changed = true;
        }
        if self.notice.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs() >= if self.notice_undo { 8 } else { 5 }) {
            self.notice = None;
            changed = true;
        }
        changed
    }
}

// ---- rendering ---------------------------------------------------------------------------------

pub(crate) fn checkbox(ui: UiColors, state: Option<bool>) -> gpui::Div {
    // Some(true) checked, Some(false) unchecked, None mixed.
    let on = state != Some(false);
    div()
        .size(px(14.))
        .flex_none()
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .when(on, |d| d.bg(ui.accent).text_color(gpui::white()))
        .when(!on, |d| d.border_1().border_color(ui.text_disabled))
        .child(match state {
            Some(true) => "✓",
            None => "–",
            Some(false) => "",
        })
}

pub(crate) fn button(ui: UiColors, id: impl Into<SharedString>, label: impl Into<SharedString>, primary: bool, enabled: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .h(px(28.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(7.))
        .text_size(px(12.5))
        .map(|d| if primary { d.bg(ui.accent).text_color(gpui::white()).font_weight(FontWeight::MEDIUM) } else { d.bg(ui.bg_control).text_color(ui.text_primary) })
        .when(!enabled, |d| d.opacity(0.45))
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.opacity(0.88)))
        .child(label.into())
}

pub(crate) fn mono(s: impl Into<SharedString>, c: Hsla) -> gpui::Div {
    div().font_family("JetBrains Mono").text_size(px(11.)).text_color(c).child(s.into())
}

pub fn sheet(ws: &Workspace, selected: usize, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let Some(s) = ws.import.as_ref() else { return div().into_any_element() };
    let total = s.rows.len();
    let picked = s.rows.iter().filter(|r| r.checked).count();
    let mut list = div().flex().flex_col();
    let mut last: Option<&PathBuf> = None;
    for (i, r) in s.rows.iter().enumerate() {
        if last != Some(&r.group) {
            last = Some(&r.group);
            let g = r.group.clone();
            let members: Vec<&Row> = s.rows.iter().filter(|x| x.group == r.group).collect();
            let k = members.iter().filter(|x| x.checked).count();
            let state = if k == 0 { Some(false) } else if k == members.len() { Some(true) } else { None };
            list = list.child(
                div()
                    .id(SharedString::from(format!("imp-g-{i}")))
                    .mt(px(if i == 0 { 2. } else { 12. }))
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_import_group(&g);
                        cx.notify();
                    }))
                    .child(checkbox(ui, state))
                    .child(div().w(px(3.)).h(px(14.)).rounded(px(2.)).bg(ws.group_color(&r.group).unwrap_or(ui.border)))
                    .child(div().flex_1().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(ws.group_label(&r.group)))
                    .child(mono(format!("{k} of {}", members.len()), ui.text_tertiary)),
            );
        }
        let f = &r.found;
        let claude = f.claude.is_some();
        let right: AnyElement = if r.already {
            div().text_size(px(11.5)).text_color(ui.text_disabled).child("Already open in T1K").into_any_element()
        } else if let Some(c) = &f.claude {
            div()
                .text_size(px(11.5))
                .text_color(ui.text_secondary)
                .child(if c.busy { "✳ Claude · resumes after it finishes" } else { "✳ Claude · resumes" })
                .into_any_element()
        } else if let Some(run) = &f.running {
            div()
                .flex()
                .gap(px(6.))
                .items_center()
                .min_w_0()
                .child(div().flex_none().text_size(px(11.5)).text_color(ui.text_tertiary).child("was running"))
                .child(mono(run.clone(), ui.text_secondary).truncate())
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let folder = f.cwd.as_deref().map(groups::tilde).unwrap_or_else(|| "~".into());
        list = list.child(
            div()
                .id(SharedString::from(format!("imp-{i}")))
                .h(px(44.))
                .px(px(10.))
                .ml(px(8.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .gap(px(10.))
                .cursor_pointer()
                .when(i == selected, |d| d.bg(ui.bg_selected))
                .when(i != selected, |d| d.hover(move |d| d.bg(ui.bg_hover)))
                .when(r.already && !r.checked, |d| d.opacity(0.55))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_row(i);
                    this.overlay = Overlay::Import { selected: i };
                    cx.notify();
                }))
                .child(checkbox(ui, Some(r.checked)))
                .child(
                    div()
                        .size(px(22.))
                        .flex_none()
                        .rounded(px(5.))
                        .bg(ui.bg_control)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(if claude { 13. } else { 10. }))
                        .font_family("JetBrains Mono")
                        .text_color(if claude { gpui::rgb(0xd97757).into() } else { ui.text_secondary })
                        .child(if claude { "✳" } else { ">_" }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().min_w_0().truncate().text_size(px(13.)).text_color(ui.text_primary).child(title_of(f)))
                                .child(div().flex_none().px(px(5.)).rounded(px(4.)).bg(ui.bg_control).text_size(px(10.5)).text_color(ui.text_tertiary).child(f.source.label())),
                        )
                        .child(mono(folder, ui.text_tertiary).truncate()),
                )
                .child(div().flex_none().max_w(px(220.)).child(right)),
        );
    }

    let note: Option<AnyElement> = match s.ask {
        Ask::Pending => {
            let which = if s.rows.iter().any(|r| r.found.source == Source::ITerm2) { "iTerm2" } else { "Terminal" };
            Some(
                div()
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(ui.accent.opacity(0.10))
                    .border_1()
                    .border_color(ui.accent.opacity(0.3))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(12.5)).text_color(ui.text_primary).child(format!("Use {which}’s tab names and order?")))
                            .child(div().text_size(px(11.5)).text_color(ui.text_tertiary).child("macOS will ask once. Without it, tabs are named after their folder.")),
                    )
                    .child(button(ui, "imp-skip", "Skip", false, true).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(s) = this.import.as_mut() {
                            s.ask = Ask::Skipped;
                        }
                        cx.notify();
                    })))
                    .child(button(ui, "imp-allow", "Allow…", true, true).on_click(cx.listener(|this, _, _, cx| this.import_allow_titles(cx))))
                    .into_any_element(),
            )
        }
        Ask::Skipped => Some(
            div()
                .text_size(px(11.5))
                .text_color(ui.text_tertiary)
                .child("iTerm2 and Terminal tabs use folder names. Allow in System Settings › Privacy & Security › Automation.")
                .into_any_element(),
        ),
        _ => None,
    };

    let vh = f32::from(window.viewport_size().height);
    div()
        .mt(px(64.))
        .w(px(600.))
        .max_h(px((vh - 120.).max(320.)))
        .rounded(px(12.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .px(px(18.))
                .pt(px(16.))
                .pb(px(10.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Bring your tabs over"))
                        .child(div().text_size(px(12.5)).text_color(ui.text_tertiary).child(if total == 0 {
                            "No tabs found in other terminals.".to_string()
                        } else {
                            format!("{total} tab{} in {}. Nothing in them is closed.", if total == 1 { "" } else { "s" }, apps(&s.rows))
                        })),
                )
                .children(note),
        )
        .child(div().id("imp-list").flex_1().min_h_0().overflow_y_scroll().px(px(8.)).pb(px(8.)).child(list))
        .child(
            div()
                .px(px(18.))
                .py(px(12.))
                .border_t_1()
                .border_color(ui.border)
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().text_size(px(12.)).text_color(ui.text_tertiary).child(format!("{picked} of {total} selected · grouped by repo in T1K")))
                .child(button(ui, "imp-cancel", "Cancel", false, true).on_click(cx.listener(|this, _, window, cx| {
                    this.mark_seen();
                    this.import = None;
                    this.close_overlay(window, cx);
                })))
                .child(
                    button(ui, "imp-go", format!("Bring {picked} tab{}", if picked == 1 { "" } else { "s" }), true, picked > 0)
                        .child(div().text_size(px(11.)).opacity(0.7).child("↵"))
                        .when(picked > 0, |d| d.on_click(cx.listener(|this, _, window, cx| this.bring_over(window, cx)))),
                ),
        )
        .into_any_element()
}

/// The welcome overlay's card when the launch scan found tabs.
pub fn welcome_card(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let found = ws.import_found.as_ref().filter(|f| !f.is_empty())?;
    let n = found.len();
    let claude = found.iter().filter(|f| f.claude.is_some()).count();
    let mut names: Vec<&str> = Vec::new();
    for f in found {
        if !names.contains(&f.source.label()) {
            names.push(f.source.label());
        }
    }
    let body = if claude > 0 {
        format!("{claude} {} Claude sessions, which pick up where they left off. Nothing in the other terminals is closed.", if claude == 1 { "has" } else { "have" })
    } else {
        "You choose which ones. Nothing in the other terminals is closed.".to_string()
    };
    Some(
        div()
            .w_full()
            .p(px(14.))
            .rounded(px(10.))
            .bg(ui.bg_terminal)
            .border_1()
            .border_color(ui.border)
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child(format!("Found {n} tab{} in {}", if n == 1 { "" } else { "s" }, names.join(" and "))))
            .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_tertiary).child(body))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_1())
                    .child(button(ui, "welcome-imp-no", "Not now", false, true).on_click(cx.listener(|this, _, _, cx| {
                        this.import_found = Some(Vec::new());
                        this.dismiss_import_offer(cx);
                    })))
                    .child(button(ui, "welcome-imp", "Choose tabs to bring…", true, true).on_click(cx.listener(|this, _, window, cx| this.open_import(window, cx)))),
            )
            .into_any_element(),
    )
}

/// The launch offer, at the bottom of the sidebar, when new tabs turned up in other terminals.
pub fn offer(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    if !ws.import_offer {
        return None;
    }
    let found = ws.import_found.as_ref().filter(|f| !f.is_empty())?;
    let new = found.iter().filter(|f| !ws.import_seen.contains(&origin(f))).count();
    let app = found.first().map(|f| f.source.label()).unwrap_or("other terminals");
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
            .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child(format!("Bring your {app} tabs into T1K?")))
            .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_tertiary).child(format!(
                "{new} {} open in {app}. Pick which to open here; nothing in {app} is closed.",
                if new == 1 { "tab is" } else { "tabs are" }
            )))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(div().flex_1())
                    .child(button(ui, "offer-no", "Not now", false, true).on_click(cx.listener(|this, _, _, cx| this.dismiss_import_offer(cx))))
                    .child(button(ui, "offer-yes", "Choose tabs to bring…", true, true).on_click(cx.listener(|this, _, window, cx| this.open_import(window, cx)))),
            )
            .into_any_element(),
    )
}

/// A short message at the bottom of the sidebar ("Brought 11 tabs over").
pub fn notice(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let (text, _) = ws.notice.as_ref()?;
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
            .text_size(px(12.5))
            .gap(px(8.))
            .text_color(ui.text_secondary)
            .child(div().flex_1().min_w_0().truncate().child(text.clone()))
            .when(ws.notice_undo, |d| {
                d.child(
                    div()
                        .id("notice-undo")
                        .flex_none()
                        .flex()
                        .gap(px(6.))
                        .text_color(ui.accent)
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.undo_move(cx)))
                        .child("Undo")
                        .child(mono(crate::keymap::display_for(&crate::actions::UndoMove, &ws.config.keys), ui.text_tertiary)),
                )
            })
            .into_any_element(),
    )
}

/// In place of the terminal while a Claude session is still open in the terminal it came from.
pub fn waiting_pane(ws: &Workspace, ix: usize, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let w = ws.tabs.get(ix)?.waiting.clone()?;
    let app = w.source.label();
    let can_exit = matches!(w.source, Source::ITerm2 | Source::Terminal);
    let body = if w.busy {
        "Claude is working there right now. T1K resumes it here as soon as it finishes and you exit it.".to_string()
    } else {
        "T1K resumes it here as soon as it exits there. One session can’t run in two places without mixing up its history.".to_string()
    };
    let caption = if !can_exit {
        format!("Type /exit in {app} to move it here.")
    } else if w.busy {
        "Exit it for me is available once Claude is idle.".to_string()
    } else {
        format!("Sends /exit to the idle prompt in {app}.")
    };
    let (src, tty) = (w.source, w.tty.clone());
    Some(
        div()
            .size_full()
            .bg(ui.bg_terminal)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(420.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(format!("This Claude session is still open in {app}")))
                    .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(ui.text_tertiary).text_center().child(body))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .pt(px(4.))
                            .child(button(ui, "wait-show", format!("Show in {app}"), false, true).on_click(move |_, _, _| import::show_app(src)))
                            .when(can_exit, |d| {
                                d.child(button(ui, "wait-exit", "Exit it for me", true, !w.busy).when(!w.busy, |b| {
                                    b.on_click(cx.listener(move |_, _, _, cx| {
                                        let tty = tty.clone();
                                        cx.background_executor().spawn(async move { import::send_exit(src, &tty) }).detach();
                                    }))
                                }))
                            }),
                    )
                    .child(div().text_size(px(11.5)).text_color(ui.text_disabled).child(caption)),
            )
            .into_any_element(),
    )
}

/// "was running npm run dev · Run again" in the title bar of an imported tab.
pub fn was_running(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let t = ws.tabs.get(ws.active)?;
    let cmd = t.was_running.clone()?;
    let ix = ws.active;
    Some(
        div()
            .flex_shrink()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_none().text_size(px(12.5)).text_color(ui.text_tertiary).child("was running"))
            .child(mono(cmd.clone(), ui.text_secondary).text_size(px(12.)).truncate())
            .child(
                div()
                    .id("run-again")
                    .flex_none()
                    .px(px(8.))
                    .h(px(22.))
                    .rounded(px(5.))
                    .bg(ui.bg_control)
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(ui.text_primary)
                    .cursor_pointer()
                    .hover(|d| d.opacity(0.85))
                    // Types it without pressing Enter: you look before it runs.
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(t) = this.tabs.get_mut(ix) {
                            t.was_running = None;
                            t.view.read(cx).terminal.write(cmd.clone().into_bytes());
                        }
                        this.refocus_terminal(window, cx);
                        cx.notify();
                    }))
                    .child("Run again"),
            )
            .into_any_element(),
    )
}
