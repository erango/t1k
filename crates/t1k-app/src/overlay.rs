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
    /// Sidebar view options (button in the sidebar's top strip).
    SidebarView { at: Point<Pixels> },
    /// The plan usage popover, pinned by clicking the meter.
    Usage { at: Point<Pixels> },
    /// T1K menu ▸ About T1K.
    About,
    /// First launch.
    Welcome,
    /// Bring your tabs over (the sheet's rows live in `Workspace::import`).
    Import { selected: usize },
    /// "Match your terminal" (canvas v3-z).
    Match,
    /// A theme file dropped on a pane: import it, or insert its path (canvas v3-aa).
    DropTheme { path: std::path::PathBuf, split: bool, at: gpui::Point<gpui::Pixels> },
    /// The update dialog (any phase).
    Update,
    /// ⌘Q while an update waits.
    QuitUpdate,
    /// ⌘Q with programs still running.
    QuitRunning { expanded: bool, selected: usize },
    /// The session strip's ⋯ menu.
    MoreMenu { at: Point<Pixels> },
    /// Command palette (⌘⇧P).
    Palette { query: String, selected: usize },
    Switcher { query: String, selected: usize },
    Overview { selected: usize },
    Rename { tab: usize, text: String },
    RenameGroup { key: std::path::PathBuf, text: String },
    Menu { tab: usize, at: Point<Pixels> },
    GroupMenu { key: std::path::PathBuf, at: Point<Pixels> },
    /// Group icon picker: suggestions, search and the emoji palette (`cat` is the category).
    IconPicker { key: crate::group_icons::IconFor, at: Point<Pixels>, query: String, selected: usize, cat: usize },
    /// Right-click in a terminal pane (`split`: the tab's split pane rather than its main one).
    TerminalMenu {
        tab: usize,
        split: bool,
        at: Point<Pixels>,
        link: Option<t1k_terminal::links::Link>,
        has_selection: bool,
        /// For a file link: the apps macOS offers under Open With (looked up when the menu opens).
        apps: Vec<(String, std::path::PathBuf)>,
    },
    Settings { section: crate::settings::Section },
    /// Tab menu › Remind me › Custom…: a typed time.
    RemindCustom { tab: usize, text: String },
    /// Theme picker: `dark` is the slot being chosen.
    ThemePicker { query: String, selected: usize, dark: bool },
}

/// Switcher filter chips.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SwitcherFilter {
    #[default]
    All,
    NeedsYou,
    Working,
    Unread,
    Project,
}

impl SwitcherFilter {
    pub const ALL: [SwitcherFilter; 5] =
        [SwitcherFilter::All, SwitcherFilter::NeedsYou, SwitcherFilter::Working, SwitcherFilter::Unread, SwitcherFilter::Project];
    pub fn label(self) -> &'static str {
        match self {
            SwitcherFilter::All => "All",
            SwitcherFilter::NeedsYou => "Needs you",
            SwitcherFilter::Working => "Working",
            SwitcherFilter::Unread => "Unread",
            SwitcherFilter::Project => "This project",
        }
    }
    fn step(self, back: bool) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if back { (i + n - 1) % n } else { (i + 1) % n }]
    }
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

    /// Which field a query matched best, for the switcher's tag ("branch", "recap"...).
    pub fn matched_field(&self, i: usize, query: &str) -> Option<&'static str> {
        if query.trim().is_empty() {
            return None;
        }
        let t = &self.tabs[i];
        let a = t.agent.as_ref();
        let fields: Vec<(&'static str, String)> = vec![
            ("title", t.title()),
            ("project", self.group_label(&t.group)),
            ("branch", t.branch().unwrap_or_default()),
            ("PR", a.map(|a| a.prs.iter().map(|(n, _)| format!("#{n} PR{n}")).collect::<Vec<_>>().join(" ")).unwrap_or_default()),
            ("now", a.and_then(|a| a.now.clone()).unwrap_or_default()),
            ("recap", a.and_then(|a| a.recap.clone()).unwrap_or_default()),
            ("prompt", a.and_then(|a| a.first_prompt.clone()).unwrap_or_default()),
            ("path", t.cwd.as_deref().map(groups::tilde).unwrap_or_default()),
        ];
        let hays: Vec<String> = fields.iter().map(|(_, v)| v.clone()).collect();
        fuzzy(query, &hays).into_iter().max_by_key(|&(score, _)| score).map(|(_, k)| fields[k].0)
    }

    /// Switcher ranking: match score, then needs-you first, then most recently used.
    pub fn switcher_results(&self, query: &str) -> Vec<usize> {
        let f = self.switcher_filter;
        let mut idx: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.switcher_keeps(i, f)).collect();
        if query.trim().is_empty() {
            // Needs you, then agent sessions, then plain shells; recent first within each.
            idx.sort_by_key(|&i| {
                let t = &self.tabs[i];
                (!t.needs_you(), t.agent.is_none(), t.urgency().min(2), std::cmp::Reverse(t.last_active))
            });
            return idx;
        }
        let hays: Vec<String> = idx.iter().map(|&i| self.haystack(i)).collect();
        let mut scored: Vec<(u32, usize)> =
            fuzzy(query, &hays).into_iter().map(|(s, k)| (s, idx[k])).collect();
        // Tabs that need you always lead (their own section), then by score.
        scored.sort_by_key(|&(s, i)| {
            (!self.tabs[i].needs_you(), std::cmp::Reverse(s), self.tabs[i].urgency().min(2), std::cmp::Reverse(self.tabs[i].last_active))
        });
        let mut out: Vec<usize> = scored.into_iter().map(|(_, i)| i).collect();
        // The helper's picks for this query lead, even where the words don't match.
        let ranked = self.meaning_hits(query);
        if !ranked.is_empty() {
            out.retain(|i| !ranked.contains(i));
            out.splice(0..0, ranked);
        }
        out
    }

    /// Tabs the helper ranked for `query` (⌘↵ in the switcher), best first, within the filter.
    pub fn meaning_hits(&self, query: &str) -> Vec<usize> {
        match &self.helper.search {
            Some((q, Some(ids))) if q == query => ids
                .iter()
                .filter_map(|id| self.tabs.iter().position(|t| &t.id == id))
                .filter(|&i| self.switcher_keeps(i, self.switcher_filter))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether tab `i` passes a switcher filter chip.
    pub fn switcher_keeps(&self, i: usize, f: SwitcherFilter) -> bool {
        let t = &self.tabs[i];
        match f {
            SwitcherFilter::All => true,
            SwitcherFilter::NeedsYou => t.needs_you(),
            SwitcherFilter::Working => t.status() == Some(&Status::Working),
            SwitcherFilter::Unread => t.unread,
            SwitcherFilter::Project => self.tabs.get(self.active).is_some_and(|a| a.group == t.group),
        }
    }

    pub fn open_overlay(&mut self, o: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_card(cx);
        self.confirm_close_group = None;
        // Rename fields open with the whole name selected, as in Finder.
        if let Overlay::Rename { text, .. } | Overlay::RenameGroup { text, .. } = &o {
            self.edit_sel = input::Sel::all(text);
        }
        self.overlay = o;
        window.focus(&self.overlay_focus);
        cx.notify();
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = Overlay::None;
        self.switcher_filter = SwitcherFilter::All;
        // Closing the theme picker any way but ↵ goes back to the theme you had.
        self.end_theme_preview(None, window, cx);
        self.key_recording = None;
        self.settings_return = None;
        self.settings_query.clear();
        self.fonts.close();
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
            Overlay::Switcher { query, .. } if ev.keystroke.key == "enter" && ev.keystroke.modifiers.platform => {
                self.helper_search(&query, cx);
                Some(Overlay::Switcher { query, selected: 0 })
            }
            Overlay::Switcher { query, .. } if ev.keystroke.key == "tab" => {
                self.switcher_filter = self.switcher_filter.step(ev.keystroke.modifiers.shift);
                Some(Overlay::Switcher { query, selected: 0 })
            }
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
            Overlay::Palette { mut query, mut selected } => match input::handle(&mut query, ev) {
                Key::Cancel => None,
                Key::Submit => {
                    let cmds = crate::palette::commands(self);
                    let pick = crate::palette::results(&cmds, &query).get(selected).copied();
                    if let Some(c) = pick.and_then(|i| cmds.into_iter().nth(i)) {
                        self.run_command(c.run, window, cx);
                        return;
                    }
                    None
                }
                key => {
                    let n = crate::palette::results(&crate::palette::commands(self), &query).len();
                    match key {
                        Key::Edited => selected = 0,
                        Key::Up => selected = selected.saturating_sub(1),
                        Key::Down => selected = (selected + 1).min(n.saturating_sub(1)),
                        _ => {}
                    }
                    Some(Overlay::Palette { query, selected })
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
            Overlay::RemindCustom { tab, mut text } => match input::handle(&mut text, ev) {
                Key::Submit => match crate::reminders::parse(&text, SystemTime::now()) {
                    Some(at) => {
                        self.set_reminder(tab, Some(at), cx);
                        None
                    }
                    None => Some(Overlay::RemindCustom { tab, text }),
                },
                Key::Cancel => None,
                _ => Some(Overlay::RemindCustom { tab, text }),
            },
            Overlay::Rename { tab, mut text } => match input::edit(&mut text, &mut self.edit_sel, ev, cx) {
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
            Overlay::RenameGroup { key, mut text } => match input::edit(&mut text, &mut self.edit_sel, ev, cx) {
                Key::Submit => {
                    let text = text.trim().to_string();
                    if groups::is_custom(&key) {
                        // A made group always keeps a name; empty leaves the current one.
                        if !text.is_empty() {
                            self.group_names.insert(key, text);
                        }
                    } else if text.is_empty() || text == groups::label(&key) {
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
            Overlay::GroupMenu { key, at } => (ev.keystroke.key != "escape").then_some(Overlay::GroupMenu { key, at }),
            Overlay::IconPicker { key, at, query, selected, cat } => self.icon_picker_key(key, at, query, selected, cat, ev, window, cx),
            o @ Overlay::TerminalMenu { .. } => (ev.keystroke.key != "escape").then_some(o),
            o @ Overlay::SidebarView { .. } => (ev.keystroke.key != "escape").then_some(o),
            o @ Overlay::Usage { .. } => (ev.keystroke.key != "escape").then_some(o),
            o @ Overlay::MoreMenu { .. } => (ev.keystroke.key != "escape").then_some(o),
            Overlay::About => (!matches!(ev.keystroke.key.as_str(), "escape" | "enter")).then_some(Overlay::About),
            Overlay::Welcome => (!matches!(ev.keystroke.key.as_str(), "escape" | "enter")).then_some(Overlay::Welcome),
            Overlay::Import { selected } => self.import_key(selected, ev, window, cx),
            Overlay::Match => self.match_key(ev, window, cx),
            // Esc inserts nothing.
            o @ Overlay::DropTheme { .. } => (ev.keystroke.key != "escape").then_some(o),
            Overlay::Update => {
                if ev.keystroke.key == "escape" {
                    self.close_update(window, cx);
                    return;
                }
                Some(Overlay::Update)
            }
            Overlay::QuitUpdate => (ev.keystroke.key != "escape").then_some(Overlay::QuitUpdate),
            Overlay::QuitRunning { expanded, selected } => self.quit_key(expanded, selected, ev, window, cx),
            Overlay::ThemePicker { query, selected, dark } => self.theme_picker_key(query, selected, dark, ev, window, cx),
            // The open font picker takes the typing for its own search; Esc clears that, then
            // closes the picker.
            Overlay::Settings { section } if self.fonts.open => {
                if let Key::Cancel = input::handle(&mut self.fonts.query, ev) {
                    if self.fonts.query.is_empty() {
                        self.fonts.close();
                    } else {
                        self.fonts.query.clear();
                    }
                }
                Some(Overlay::Settings { section })
            }
            // Typing searches; Esc clears the search, then closes.
            Overlay::Settings { section } => match input::handle(&mut self.settings_query, ev) {
                Key::Cancel if !self.settings_query.is_empty() => {
                    self.settings_query.clear();
                    Some(Overlay::Settings { section })
                }
                Key::Cancel => None,
                _ => Some(Overlay::Settings { section }),
            },
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

/// The switcher's filter chips, each with how many tabs it keeps.
fn switcher_chips(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    div().px(px(8.)).pb(px(6.)).flex().gap(px(6.)).children(SwitcherFilter::ALL.into_iter().map(|f| {
        let on = ws.switcher_filter == f;
        let n = (0..ws.tabs.len()).filter(|&i| ws.switcher_keeps(i, f)).count();
        let loud = f == SwitcherFilter::NeedsYou && n > 0;
        div()
            .id(f.label())
            .h(px(24.))
            .px(px(9.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(12.))
            .border_1()
            .border_color(if on { ui.accent } else { ui.border })
            .when(on, |d| d.bg(ui.bg_selected))
            .text_size(px(12.))
            .text_color(if on { ui.text_primary } else { ui.text_secondary })
            .cursor_pointer()
            .hover(move |d| d.bg(ui.bg_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.switcher_filter = f;
                if let Overlay::Switcher { selected, .. } = &mut this.overlay {
                    *selected = 0;
                }
                cx.notify();
            }))
            .child(f.label())
            .child(div().text_size(px(11.)).text_color(if loud { ui.status_needs } else { ui.text_quiet }).child(n.to_string()))
    }))
}

fn scrim(ws: &Workspace, close_on_click: bool, cx: &mut Context<Workspace>) -> gpui::Stateful<Div> {
    div()
        .id("overlay")
        .absolute()
        .inset_0()
        // Swallow hover and scroll so the sidebar and terminal underneath don't react.
        .occlude()
        .track_focus(&ws.overlay_focus)
        .on_key_down(cx.listener(Workspace::overlay_key))
        // GPUI turns Space/Enter on a focused element into a click; the scrim holds the
        // overlay's focus, so only a real mouse click outside the panel may close it.
        .when(close_on_click, |d| {
            d.on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                if !ev.is_keyboard() {
                    this.close_overlay(window, cx)
                }
            }))
        })
}

pub fn render(ws: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    match &ws.overlay {
        Overlay::None => None,
        Overlay::Switcher { query, selected } => {
            let results = ws.switcher_results(query);
            let selected = *selected;
            // Sections: NEEDS YOU first, then everything else.
            let meaning = ws.meaning_hits(query).len();
            let pending = ws.helper.search.as_ref().is_some_and(|(q, r)| q == query && r.is_none());
            let needs_count = if meaning > 0 { 0 } else { results.iter().take(9).filter(|&&i| ws.tabs[i].needs_you()).count() };
            let section = |label: &'static str, color: gpui::Hsla| {
                div().h(px(22.)).px(px(14.)).flex().items_center().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(label)
            };
            let mut rows: Vec<AnyElement> = Vec::new();
            for (n, &i) in results.iter().take(9).enumerate() {
                if n == 0 && meaning > 0 {
                    rows.push(section("BY MEANING", ui.accent).into_any_element());
                }
                if n == meaning && meaning > 0 {
                    rows.push(section("MATCHES", ui.text_tertiary).into_any_element());
                }
                if n == 0 && needs_count > 0 {
                    rows.push(section("NEEDS YOU", ui.status_needs).into_any_element());
                }
                let t = &ws.tabs[i];
                if query.is_empty() {
                    // NEEDS YOU, AGENTS, SHELLS: a heading wherever the kind changes.
                    let prev = n.checked_sub(1).map(|p| &ws.tabs[results[p]]);
                    let kind = |t: &crate::workspace::Tab| if t.needs_you() { 0 } else if t.agent.is_some() { 1 } else { 2 };
                    if n >= needs_count && prev.is_none_or(|p| kind(p) != kind(t)) {
                        rows.push(section(if kind(t) == 1 { "AGENTS" } else { "SHELLS" }, ui.text_tertiary).into_any_element());
                    }
                } else if n == needs_count && meaning == 0 {
                    rows.push(section("MATCHES", ui.text_tertiary).into_any_element());
                }
                let is_sel = n == selected;
                let tag = ws.matched_field(i, query);
                let age = t.agent.as_ref().map(|a| {
                    let s = a.since.elapsed().as_secs();
                    if s < 60 { format!("{s}s") } else if s < 3600 { format!("{}m", s / 60) } else { format!("{}h", s / 3600) }
                });
                let sub_color = match t.status() {
                    Some(Status::NeedsInput { .. }) => ui.status_needs,
                    Some(Status::Ended) => ui.text_disabled,
                    _ => ui.text_quiet,
                };
                rows.push(
                    div()
                        .id(SharedString::from(format!("sw-{i}")))
                        .h(px(54.))
                        .px(px(12.))
                        .rounded(px(7.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .when(is_sel, |d| d.bg(ui.bg_selected))
                        .hover(move |d| d.bg(ui.bg_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_overlay(window, cx);
                            this.activate(i, window, cx);
                        }))
                        .child(crate::app_icon::badge(t, ui, 28.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(8.))
                                        .items_center()
                                        .child(div().min_w_0().truncate().text_size(px(13.5)).text_color(ui.text_primary).child(t.title()))
                                        .child(div().flex_none().text_size(px(12.)).text_color(ui.text_tertiary).child(ws.group_label(&t.group)))
                                        .children(t.branch().map(|b| div().flex_none().text_size(px(11.5)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(b))),
                                )
                                .child(div().truncate().text_size(px(12.)).text_color(sub_color).child(t.subtitle())),
                        )
                        .children(tag.map(|f| {
                            div().flex_none().px(px(6.)).h(px(18.)).flex().items_center().rounded(px(4.)).border_1().border_color(ui.border).text_size(px(10.5)).text_color(ui.text_tertiary).child(f)
                        }))
                        .child(div().w(px(44.)).flex_none().flex().justify_end().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).children(age))
                        .into_any_element(),
                );
            }
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
                        .h(px(48.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().text_size(px(15.)).text_color(ui.text_quiet).child("⌕"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(16.))
                                .text_color(if query.is_empty() { ui.text_tertiary } else { ui.text_primary })
                                .child(if query.is_empty() { "Find a tab: title, project, branch, PR, file…".to_string() } else { format!("{query}▏") }),
                        )
                        .child(div().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(format!("{} of {}", results.len().min(9), ws.tabs.len())))
                        .child(keycap(ui, "esc")),
                )
                .child(switcher_chips(ws, cx))
                .when(pending, |d| {
                    d.child(
                        div().h(px(30.)).px(px(14.)).flex().items_center().gap(px(8.)).text_size(px(12.5)).text_color(ui.text_tertiary)
                            .child(crate::sidebar::spinner(ui.text_tertiary, ws.started))
                            .child(format!("Searching by meaning with {}…", ws.config.helper.backend().map(|b| b.label()).unwrap_or(""))),
                    )
                })
                .child(div().h(px(1.)).bg(ui.border).mb(px(4.)))
                .children(rows)
                .when(results.is_empty(), |d| {
                    d.child(div().p(px(14.)).text_size(px(13.)).text_color(ui.text_tertiary).child(if ws.switcher_filter == SwitcherFilter::All { "No matching tabs" } else { "No tabs here. ⇥ tries the next filter." }))
                })
                .child(
                    div()
                        .mt(px(4.))
                        .mx(px(-8.))
                        .mb(px(-8.))
                        .h(px(34.))
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .bg(ui.bg_chrome)
                        .rounded_b(px(12.))
                        .border_t_1()
                        .border_color(ui.border)
                        .text_size(px(11.5))
                        .text_color(ui.text_tertiary)
                        .child("↵ open")
                        .child("↑↓ move")
                        .child("⇥ filter")
                        .when(ws.config.helper.backend().is_some() && ws.config.helper.search, |d| d.child("⌘↵ by meaning"))
                        .child("esc close")
                        .child(div().flex_1())
                        .child("matches title · recap · project · branch · PR · files"),
                );
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.25)).flex().justify_center().items_start().child(panel).into_any_element())
        }
        Overlay::About => {
            let c = crate::brand::about(ws, cx);
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.35)).flex().justify_center().items_start().child(c).into_any_element())
        }
        Overlay::Import { selected } => {
            let c = crate::import_ui::sheet(ws, *selected, window, cx);
            Some(scrim(ws, false, cx).bg(gpui::black().opacity(0.35)).flex().justify_center().items_start().child(c).into_any_element())
        }
        Overlay::DropTheme { path, split, at } => {
            let c = crate::drop::theme_choice(ws, path, *split, *at, cx);
            Some(scrim(ws, true, cx).child(c).into_any_element())
        }
        Overlay::Match => {
            let c = crate::match_ui::sheet(ws, cx);
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.35)).flex().justify_center().items_start().child(c).into_any_element())
        }
        Overlay::Welcome => {
            let c = crate::brand::welcome(ws, cx);
            Some(scrim(ws, false, cx).bg(gpui::black().opacity(0.45)).flex().justify_center().items_start().child(c).into_any_element())
        }
        Overlay::Update => {
            let c = crate::update_ui::dialog(ws, cx);
            Some(scrim(ws, false, cx).bg(gpui::black().opacity(0.35)).flex().justify_center().items_start().child(c).into_any_element())
        }
        Overlay::QuitRunning { expanded, selected } => {
            let p = crate::quit::render(ws, *expanded, *selected, cx);
            Some(scrim(ws, false, cx).bg(gpui::black().opacity(0.3)).flex().justify_center().items_start().child(div().mt(px(140.)).child(p)).into_any_element())
        }
        Overlay::QuitUpdate => {
            let c = crate::update_ui::quit_prompt(ws, cx);
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.35)).flex().justify_center().items_start().child(c).into_any_element())
        }
        Overlay::MoreMenu { at } => {
            let m = more_menu(ws, *at, cx);
            Some(scrim(ws, true, cx).child(m).into_any_element())
        }
        Overlay::Usage { at } => {
            let c = crate::usage::card_for(ws).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            Some(scrim(ws, true, cx).child(div().absolute().left(at.x.max(px(8.))).top(at.y).child(c)).into_any_element())
        }
        Overlay::SidebarView { at } => {
            let m = sidebar_view_menu(ws, *at, cx);
            Some(scrim(ws, true, cx).child(m).into_any_element())
        }
        Overlay::Palette { query, selected } => {
            let panel = crate::palette::render(ws, query, *selected, cx);
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.25)).flex().justify_center().items_start().child(panel).into_any_element())
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
                            .child({
                                let projects = ws.grouped().len();
                                let count = |f: &dyn Fn(&crate::workspace::Tab) -> bool| ws.tabs.iter().filter(|t| f(t)).count();
                                let needs = count(&|t| matches!(t.status(), Some(Status::NeedsInput { .. })));
                                let errors = count(&|t| matches!(t.status(), Some(Status::Error { .. })));
                                let working = count(&|t| t.status() == Some(&Status::Working));
                                let mono = |text: String, c: gpui::Hsla| div().text_size(px(11.5)).font_family("JetBrains Mono").text_color(c).child(text);
                                div()
                                    .flex()
                                    .gap(px(12.))
                                    .child(mono(format!("{} tabs · {projects} projects", ws.tabs.len()), ui.text_tertiary))
                                    .when(needs > 0, |d| d.child(mono(format!("{needs} needs you"), ui.status_needs)))
                                    .when(errors > 0, |d| d.child(mono(format!("{errors} error"), ui.status_error)))
                                    .when(working > 0, |d| d.child(mono(format!("{working} working"), ui.text_secondary)))
                            })
                            .child(div().flex_1())
                            .child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("Type to filter · arrows move · ↵ opens · esc")),
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
                Overlay::RenameGroup { key, text } if groups::is_custom(key) => {
                    ("Name the group".to_string(), text, "Enter to save · tabs you move here stay until you move them out · Esc")
                }
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
                .child(edit_field(ws, text, cx))
                .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(hint));
            Some(scrim(ws, true, cx).flex().justify_center().items_start().child(panel).into_any_element())
        }
        Overlay::RemindCustom { tab, text } => {
            let now = SystemTime::now();
            let when = crate::reminders::parse(text, now);
            let preview = match (when, text.trim().is_empty()) {
                (_, true) => "Try 45m, 2h, 16:30, 4pm or tomorrow 9:15".to_string(),
                (Some(at), _) => format!("Reminds you {}", crate::reminders::describe(at, now)),
                (None, _) => "Can't read that time yet".to_string(),
            };
            let title = ws.tabs.get(*tab).map(|t| t.title()).unwrap_or_default();
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
                .child(div().truncate().text_size(px(12.)).text_color(ui.text_tertiary).child(format!("Remind me about {title}")))
                .child(
                    div().h(px(34.)).px(px(10.)).rounded(px(6.)).bg(ui.bg_control).flex().items_center()
                        .text_size(px(14.)).text_color(ui.text_primary).child(format!("{text}▏")),
                )
                .child(div().text_size(px(12.)).text_color(if when.is_some() { ui.text_secondary } else { ui.text_tertiary }).child(preview))
                .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("Enter to set · Esc"));
            Some(scrim(ws, true, cx).flex().justify_center().items_start().child(panel).into_any_element())
        }
        Overlay::ThemePicker { query, selected, dark } => {
            // Theme files dropped on the picker import straight away.
            let panel = div()
                .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| this.import_themes(paths.paths(), window, cx)))
                .child(crate::theme_picker::render(ws, query, *selected, *dark, window, cx));
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.2)).flex().justify_center().items_start().child(panel).into_any_element())
        }
        Overlay::Settings { section } => {
            let panel = crate::settings::render(ws, *section, window, cx);
            Some(scrim(ws, true, cx).bg(gpui::black().opacity(0.3)).flex().justify_center().items_start().child(panel).into_any_element())
        }
        Overlay::Menu { tab, at } => {
            let m = menu(ws, *tab, *at, window, cx);
            Some(scrim(ws, true, cx).child(m).into_any_element())
        }
        Overlay::IconPicker { key, at, query, selected, cat } => {
            let p = crate::group_icons::render(ws, key, *at, query, *selected, *cat, window, cx);
            Some(scrim(ws, true, cx).child(p).into_any_element())
        }
        Overlay::GroupMenu { key, at } => {
            let m = group_menu(ws, key.clone(), *at, window, cx);
            Some(scrim(ws, true, cx).child(m).into_any_element())
        }
        Overlay::TerminalMenu { tab, split, at, link, has_selection, apps } => {
            let m = terminal_menu(ws, *tab, *split, *at, link.clone(), *has_selection, apps.clone(), window, cx);
            Some(scrim(ws, true, cx).child(m).into_any_element())
        }
    }
}

/// Apps for a file link's Open With section (empty for other links).
pub fn file_apps(link: &Option<t1k_terminal::links::Link>) -> Vec<(String, std::path::PathBuf)> {
    match link {
        Some(t1k_terminal::links::Link::File { path, .. }) if path.is_file() => crate::platform::apps_for(path),
        _ => Vec::new(),
    }
}

/// A shortcut label for menus and lists: the system font (its ⌘⇧⌥ glyphs are drawn for UI),
/// each key a little apart so "⌘⇧R" doesn't run together, in a color that stays readable on a
/// highlighted row.
pub fn shortcut(ui: t1k_themes::UiColors, keys: &str) -> Div {
    let mut d = div().flex_none().flex().items_center().gap(px(1.5)).text_size(px(12.5)).text_color(ui.text_secondary);
    // Keep words ("click", "esc") whole; split symbol runs into keys.
    if keys.chars().all(|c| !c.is_ascii_alphabetic()) || keys.chars().filter(|c| c.is_ascii_alphabetic()).count() <= 1 {
        for ch in keys.chars().filter(|c| !c.is_whitespace()) {
            d = d.child(div().min_w(px(9.)).flex().justify_center().child(ch.to_string()));
        }
    } else {
        d = d.child(keys.to_string());
    }
    d
}

/// A small key label, e.g. "esc".
fn keycap(ui: t1k_themes::UiColors, label: &'static str) -> Div {
    div()
        .h(px(18.))
        .px(px(5.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(ui.border)
        .text_size(px(10.5))
        .font_family("JetBrains Mono")
        .text_color(ui.text_tertiary)
        .child(label)
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
                .child(crate::sidebar::glyph(t, ui, ws.started, t.program.is_some(), 1.))
                .child(status_text(t))
                .child(div().flex_1())
                .children(since)
                .child(div().child(ws.group_label(&t.group))),
        )
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ui.text_primary)
                .truncate()
                .child(t.title()),
        )
        .child(div().text_size(px(12.)).text_color(ui.text_secondary).line_clamp(1).child(t.subtitle()))
        .when(!facts.is_empty(), |d| {
            d.child(div().flex().gap(px(8.)).text_size(px(10.5)).font_family("JetBrains Mono").text_color(ui.text_quiet).children(facts))
        })
        .child({
            // Mini terminal: the last lines on screen, newest last; colored by state.
            let lines = t.view.read(cx).terminal.last_lines(3);
            let last_color = match t.status() {
                Some(Status::NeedsInput { .. }) => ui.status_needs,
                Some(Status::Error { .. }) => ui.status_error,
                _ => ui.text_secondary,
            };
            let n = lines.len();
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .rounded(px(5.))
                .px(px(8.))
                .py(px(5.))
                .bg(ui.bg_sidebar)
                .border_1()
                .border_color(ui.border)
                .flex()
                .flex_col()
                .justify_end()
                .font_family("JetBrains Mono")
                .text_size(px(10.5))
                .children(lines.into_iter().enumerate().map(move |(j, l)| {
                    div().truncate().text_color(if j + 1 == n { last_color } else { ui.text_quiet }).child(l)
                }))
        })
        .into_any_element()
}

fn menu(ws: &Workspace, tab: usize, at: Point<Pixels>, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let Some(t) = ws.tabs.get(tab) else { return div().into_any_element() };
    let item = |id: &str, label: String| {
        // Shortcut hint on the right for items that have one.
        let keys = match id {
            "rename" => "⌘⇧R",
            "split" => "⌘D",
            "close" => "⌘W",
            _ => "",
        };
        div()
            .id(SharedString::from(format!("menu-{id}")))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(ui.text_primary)
            .hover(move |d| d.bg(ui.bg_selected))
            .child(div().flex_1().child(label))
            .child(shortcut(ui, keys))
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
    let id = t.id.clone();
    m = m.child(item("icon", if t.icon.is_some() { "Change icon…".into() } else { "Choose icon…".into() }).on_click(cx.listener(
        move |this, ev: &gpui::ClickEvent, window, cx| this.open_icon_picker(crate::group_icons::IconFor::Tab(id.clone()), ev.position(), window, cx),
    )));
    if t.icon.is_some() {
        let id = t.id.clone();
        m = m.child(item("reset-icon", "Reset icon".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.set_icon(&crate::group_icons::IconFor::Tab(id.clone()), None, cx);
            this.close_overlay(window, cx);
        })));
    }
    if t.split.is_some() {
        m = m.child(item("close-split", "Close split".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            this.close_split(tab, window, cx);
        })));
    } else {
        m = m.child(item("split", "Open in split".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            this.activate(tab, window, cx);
            this.open_split(tab, false, window, cx);
        })));
    }
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
    chips = chips.child(
        div()
            .id("rem-custom")
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
                this.open_overlay(Overlay::RemindCustom { tab, text: String::new() }, window, cx);
            }))
            .child("Custom…"),
    );
    m = m.child(chips);
    if t.reminder_due {
        // A reminder that went off: push it back instead of clearing it.
        let mut snooze = div().flex().gap(px(4.)).px(px(8.)).pb(px(4.));
        for (label, mins) in [("Snooze 10 min", 10u64), ("Snooze 1 hour", 60)] {
            snooze = snooze.child(
                div()
                    .id(SharedString::from(format!("snooze-{mins}")))
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
                        this.set_reminder(tab, Some(SystemTime::now() + Duration::from_secs(mins * 60)), cx);
                        this.close_overlay(window, cx);
                    }))
                    .child(label),
            );
        }
        m = m.child(snooze);
    } else if let Some(at) = t.reminder_at {
        m = m.child(div().px(px(10.)).pb(px(4.)).text_size(px(11.)).text_color(ui.text_tertiary).child(format!("Set {}", crate::reminders::describe(at, SystemTime::now()))));
    }
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
    // Move to another group (pins the tab there), or back to its folder's group.
    let others: Vec<(std::path::PathBuf, String)> = ws
        .grouped()
        .into_iter()
        .map(|(g, _)| g)
        .filter(|g| *g != t.group)
        .map(|g| {
            let label = ws.group_label(&g);
            (g, label)
        })
        .collect();
    {
        m = m.child(sep());
        m = m.child(div().px(px(10.)).pt(px(2.)).pb(px(4.)).text_size(px(11.)).text_color(ui.text_tertiary).child("Move to group"));
        m = m.child(item("move-new", "New group…".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.new_group_with(tab, window, cx);
        })));
        for (n, (g, label)) in others.into_iter().enumerate() {
            m = m.child(item(&format!("move-{n}"), label).on_click(cx.listener(move |this, _, window, cx| {
                this.move_tab(tab, Some(g.clone()), cx);
                this.close_overlay(window, cx);
            })));
        }
        if t.pinned_group.is_some() {
            m = m.child(item("move-auto", "Back to its folder's group".into()).on_click(cx.listener(move |this, _, window, cx| {
                this.move_tab(tab, None, cx);
                this.close_overlay(window, cx);
            })));
        }
    }
    m = m.child(sep());
    m = m.child(item("close", "Close tab".into()).on_click(cx.listener(move |this, _, window, cx| {
        this.close_overlay(window, cx);
        this.request_close(tab, window, cx);
    })));

    // Keep the menu inside the window.
    let vh = f32::from(window.viewport_size().height);
    let top = f32::from(at.y).min(vh - 420.).max(8.);
    m.top(px(top)).into_any_element()
}

/// Right-click menu on a group header: new tab, rename, color, sort, collapse.
fn group_menu(ws: &Workspace, key: std::path::PathBuf, at: Point<Pixels>, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    use crate::session::GroupSort;
    let ui = ws.ui;
    let item = |id: &str, label: String| {
        div()
            .id(SharedString::from(format!("gmenu-{id}")))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(ui.text_primary)
            .hover(move |d| d.bg(ui.bg_selected))
            .child(label)
    };
    let sep = || div().h(px(1.)).my(px(4.)).mx(px(6.)).bg(ui.border);
    let heading = |t: &'static str| div().px(px(10.)).pt(px(2.)).pb(px(4.)).text_size(px(11.)).text_color(ui.text_tertiary).child(t);
    let mut m = div()
        .absolute()
        .left(at.x)
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

    let k = key.clone();
    m = m.child(item("new", "New tab here".into()).on_click(cx.listener(move |this, _, window, cx| {
        this.close_overlay(window, cx);
        this.new_tab_in_group(k.clone(), window, cx);
    })));
    let k = key.clone();
    let has_icon = ws.group_icons.contains_key(&key);
    m = m.child(item("icon", if has_icon { "Change icon…".into() } else { "Choose icon…".into() }).on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
        this.open_icon_picker(crate::group_icons::IconFor::Group(k.clone()), ev.position(), window, cx);
    })));
    if has_icon {
        let k = key.clone();
        m = m.child(item("reset-icon", "Reset icon".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.set_icon(&crate::group_icons::IconFor::Group(k.clone()), None, cx);
            this.close_overlay(window, cx);
        })));
    }
    let k = key.clone();
    m = m.child(item("rename", "Rename…".into()).on_click(cx.listener(move |this, _, window, cx| {
        let text = this.group_label(&k);
        this.open_overlay(Overlay::RenameGroup { key: k.clone(), text }, window, cx);
    })));
    if groups::is_custom(&key) {
        let k = key.clone();
        m = m.child(item("ungroup", "Ungroup".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.ungroup(&k, cx);
            this.close_overlay(window, cx);
        })));
    }
    if ws.group_names.contains_key(&key) && !groups::is_custom(&key) {
        let k = key.clone();
        m = m.child(item("reset", "Use folder name".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.group_names.remove(&k);
            this.save();
            this.close_overlay(window, cx);
        })));
    }
    let order: Vec<std::path::PathBuf> = ws.grouped().into_iter().map(|(g, _)| g).collect();
    let pos = order.iter().position(|g| *g == key);
    if pos.is_some_and(|p| p > 0) {
        let k = key.clone();
        m = m.child(item("up", "Move up".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.move_group(&k, -1, cx);
            this.close_overlay(window, cx);
        })));
    }
    if pos.is_some_and(|p| p + 1 < order.len()) {
        let k = key.clone();
        m = m.child(item("down", "Move down".into()).on_click(cx.listener(move |this, _, window, cx| {
            this.move_group(&k, 1, cx);
            this.close_overlay(window, cx);
        })));
    }
    let collapsed = ws.collapsed.contains(&key);
    let k = key.clone();
    m = m.child(item("collapse", if collapsed { "Expand".into() } else { "Collapse".into() }).on_click(cx.listener(move |this, _, window, cx| {
        this.toggle_group(k.clone(), cx);
        this.close_overlay(window, cx);
    })));

    let members: Vec<usize> = ws.tabs.iter().enumerate().filter(|(_, t)| t.group == key).map(|(i, _)| i).collect();
    let unread = members.iter().filter(|&&i| ws.tabs[i].unread).count();
    if unread > 0 {
        let k = key.clone();
        m = m.child(item("read", format!("Mark all as read ({unread})")).on_click(cx.listener(move |this, _, window, cx| {
            for t in this.tabs.iter_mut().filter(|t| t.group == k) {
                t.unread = false;
            }
            this.close_overlay(window, cx);
        })));
    }
    let k = key.clone();
    m = m.child(item("others", "Collapse others".into()).on_click(cx.listener(move |this, _, window, cx| {
        let others: Vec<std::path::PathBuf> = this.grouped().into_iter().map(|(g, _)| g).filter(|g| *g != k).collect();
        this.collapsed.extend(others);
        this.collapsed.remove(&k);
        this.save();
        this.close_overlay(window, cx);
    })));
    // Quiet = idle shells and finished, seen sessions; closing them loses nothing running.
    let quiet: Vec<String> = members.iter().filter(|&&i| ws.is_quiet(i)).map(|&i| ws.tabs[i].id.clone()).collect();
    if !quiet.is_empty() {
        let n = quiet.len();
        m = m.child(item("close-quiet", format!("Close {n} quiet tab{}", if n == 1 { "" } else { "s" })).on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            for id in &quiet {
                if let Some(ix) = this.tabs.iter().position(|t| &t.id == id) {
                    if this.tabs.len() > 1 {
                        this.close_tab_at(ix, window, cx);
                    }
                }
            }
        })));
    }

    // Close group: asks first, in the menu, when something in it is still running.
    let n = members.len();
    let running = ws.running_in_group(&key, cx);
    let confirming = ws.confirm_close_group.as_ref() == Some(&key);
    let label = if confirming {
        format!("Stop {running} running and close {n} tab{}", if n == 1 { "" } else { "s" })
    } else {
        format!("Close group ({n} tab{})", if n == 1 { "" } else { "s" })
    };
    let k = key.clone();
    m = m.child(
        item("close-group", label)
            .when(confirming, |d| d.text_color(ui.status_error).font_weight(FontWeight::MEDIUM))
            .on_click(cx.listener(move |this, _, window, cx| {
                if running > 0 && this.confirm_close_group.as_ref() != Some(&k) {
                    this.confirm_close_group = Some(k.clone());
                    cx.notify();
                    return;
                }
                this.close_overlay(window, cx);
                this.close_group(k.clone(), window, cx);
            })),
    );

    // Color: none plus the theme's six ANSI hues.
    m = m.child(sep()).child(heading("Color"));
    let current = ws.group_colors.get(&key).copied();
    let mut swatches = div().flex().gap(px(6.)).px(px(10.)).pb(px(6.));
    for i in 0u8..=6 {
        let color: Option<gpui::Hsla> = (i > 0).then(|| gpui::rgb(ws.theme.ansi[i as usize]).into());
        let on = current.unwrap_or(0) == i;
        let k = key.clone();
        swatches = swatches.child(
            div()
                .id(SharedString::from(format!("gcolor-{i}")))
                .size(px(20.))
                .rounded_full()
                .border_2()
                .border_color(if on { ui.text_primary } else { gpui::transparent_black() })
                .flex()
                .items_center()
                .justify_center()
                .child(match color {
                    Some(c) => div().size(px(14.)).rounded_full().bg(c),
                    None => div().size(px(14.)).rounded_full().border_1().border_color(ui.text_tertiary),
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if i == 0 {
                        this.group_colors.remove(&k);
                    } else {
                        this.group_colors.insert(k.clone(), i);
                    }
                    this.save();
                    this.close_overlay(window, cx);
                })),
        );
    }
    m = m.child(swatches);

    m = m.child(sep()).child(heading("Sort tabs by"));
    let sort = ws.group_sort.get(&key).copied().unwrap_or_default();
    for (id, label, mode) in [
        ("attention", "Needs attention", GroupSort::Attention),
        ("activity", "Recently used", GroupSort::Activity),
        ("opened", "Order opened", GroupSort::Opened),
    ] {
        let k = key.clone();
        let mark = if sort == mode { "✓ " } else { "   " };
        m = m.child(item(id, format!("{mark}{label}")).on_click(cx.listener(move |this, _, window, cx| {
            if mode == GroupSort::Attention {
                this.group_sort.remove(&k);
            } else {
                this.group_sort.insert(k.clone(), mode);
            }
            this.save();
            this.close_overlay(window, cx);
        })));
    }
    if key.is_dir() {
        let k = key.clone();
        m = m.child(sep()).child(item("reveal", "Reveal in Finder".into()).on_click(cx.listener(move |this, _, window, cx| {
            cx.reveal_path(&k);
            this.close_overlay(window, cx);
        })));
    }
    let vh = f32::from(window.viewport_size().height);
    let top = f32::from(at.y).min(vh - 400.).max(8.);
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

/// Right-click menu inside a terminal pane.
#[allow(clippy::too_many_arguments)]
fn terminal_menu(
    ws: &Workspace,
    tab: usize,
    split: bool,
    at: Point<Pixels>,
    link: Option<t1k_terminal::links::Link>,
    has_selection: bool,
    apps: Vec<(String, std::path::PathBuf)>,
    window: &Window,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let ui = ws.ui;
    let Some(t) = ws.tabs.get(tab) else { return div().into_any_element() };
    let Some(view) = (if split { t.split.as_ref().map(|s| s.view.clone()) } else { Some(t.view.clone()) }) else {
        return div().into_any_element();
    };
    let item = |id: &str, label: &str, keys: &str, enabled: bool| {
        div()
            .id(SharedString::from(format!("tmenu-{id}")))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(if enabled { ui.text_primary } else { ui.text_disabled })
            .when(enabled, |d| d.hover(move |d| d.bg(ui.bg_selected)))
            .child(div().flex_1().truncate().child(label.to_string()))
            .child(shortcut(ui, keys))
    };
    let sep = || div().h(px(1.)).my(px(4.)).mx(px(6.)).bg(ui.border);
    // Run something on the pane, close the menu and give the pane focus back.
    let act = |f: fn(&mut t1k_terminal::view::TerminalView, &mut Context<t1k_terminal::view::TerminalView>)| {
        let view = view.clone();
        cx.listener(move |this: &mut Workspace, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Workspace>| {
            this.close_overlay(window, cx);
            let h = gpui::Focusable::focus_handle(view.read(cx), cx);
            window.focus(&h);
            view.update(cx, f);
        })
    };
    let mut m = div()
        .absolute()
        .left(at.x)
        .w(px(230.))
        .p(px(5.))
        .rounded(px(8.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    if let Some(link) = link {
        if let t1k_terminal::links::Link::File { path, line, .. } = &link {
            // A file: open it (side panel for text), in the editor, with another app, or show it.
            let text = crate::viewer::is_text(path);
            let open = link.clone();
            m = m.child(item("open", if text { "Open" } else { "Open in default app" }, "⌘click", true).on_click(cx.listener(move |this, _, window, cx| {
                this.close_overlay(window, cx);
                this.open_link(tab, &open, cx);
            })));
            if text {
                let ed = link.clone();
                m = m.child(item("open-editor", "Open in editor", "", true).on_click(cx.listener(move |this, _, window, cx| {
                    this.close_overlay(window, cx);
                    this.open_link_in_editor(tab, &ed, cx);
                })));
            }
            if !apps.is_empty() {
                m = m.child(sep()).child(div().px(px(10.)).pt(px(2.)).pb(px(2.)).text_size(px(11.)).text_color(ui.text_tertiary).child("Open with"));
                for (n, (name, app)) in apps.into_iter().take(8).enumerate() {
                    let file = path.clone();
                    let label: &'static str = Box::leak(if n == 0 { format!("{name} (default)") } else { name }.into_boxed_str());
                    let id: &'static str = Box::leak(format!("with-{n}").into_boxed_str());
                    m = m.child(item(id, label, "", true).on_click(cx.listener(move |this, _, window, cx| {
                        this.close_overlay(window, cx);
                        crate::platform::open_with(&app, &file);
                    })));
                }
            }
            m = m.child(sep());
            let rev = path.clone();
            m = m.child(item("reveal", "Reveal in Finder", "", true).on_click(cx.listener(move |this, _, window, cx| {
                this.close_overlay(window, cx);
                cx.reveal_path(&rev);
            })));
            let copy_path = match line {
                Some(l) => format!("{}:{l}", path.display()),
                None => path.display().to_string(),
            };
            m = m.child(item("copy-path", "Copy path", "", true).on_click(cx.listener(move |this, _, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_path.clone()));
                this.close_overlay(window, cx);
            })));
            m = m.child(sep());
        } else {
            let label = match &link {
                t1k_terminal::links::Link::Url(_) => "Open link",
                t1k_terminal::links::Link::Pr(_) => "Open PR",
                t1k_terminal::links::Link::Ticket(_) => "Open ticket",
                t1k_terminal::links::Link::File { .. } => "Open file",
            };
            let open = link.clone();
            m = m.child(item("open", label, "⌘click", true).on_click(cx.listener(move |this, _, window, cx| {
                this.close_overlay(window, cx);
                this.open_link(tab, &open, cx);
            })));
            let text = link.label();
            m = m.child(item("copy-link", "Copy link", "", true).on_click(cx.listener(move |this, _, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                this.close_overlay(window, cx);
            })));
            m = m.child(sep());
        }
    }
    let copy = item("copy", "Copy", "⌘C", has_selection);
    m = m.child(if has_selection { copy.on_click(act(|v, cx| v.copy_selection(cx))) } else { copy });
    m = m.child(item("paste", "Paste", "⌘V", true).on_click(act(|v, cx| v.paste_clipboard(cx))));
    m = m.child(item("select-all", "Select all", "", true).on_click(act(|v, cx| v.select_all(cx))));
    m = m.child(sep());
    m = m.child(item("find", "Find…", "⌘F", true).on_click(act(|v, cx| v.open_find(cx))));
    m = m.child(item("clear", "Clear", "⌘K", true).on_click(act(|v, cx| v.clear(cx))));
    m = m.child(sep());
    if split {
        m = m.child(item("close-split", "Close split", "⌘W", true).on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            this.close_split(tab, window, cx);
        })));
    } else if t.split.is_none() {
        m = m.child(item("split-right", "Split right", "⌘D", true).on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            this.open_split(tab, false, window, cx);
        })));
        m = m.child(item("split-down", "Split down", "⌘⇧D", true).on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            this.open_split(tab, true, window, cx);
        })));
    } else {
        m = m.child(item("other-pane", "Switch pane", "⌘⌥→", true).on_click(cx.listener(move |this, _, window, cx| {
            this.close_overlay(window, cx);
            this.focus_other_pane(window, cx);
        })));
    }

    let vh = f32::from(window.viewport_size().height);
    let vw = f32::from(window.viewport_size().width);
    let top = f32::from(at.y).min(vh - 330.).max(8.);
    let left = f32::from(at.x).min(vw - 240.).max(8.);
    m.top(px(top)).left(px(left)).into_any_element()
}

/// Sidebar view options: what titles show, an optional second line, density, icons, hover card.
/// Each pick writes config.toml and applies live; the menu stays open to try options.
fn sidebar_view_menu(ws: &Workspace, at: Point<Pixels>, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let c = &ws.config;
    let heading = |t: &'static str| div().px(px(10.)).pt(px(6.)).pb(px(4.)).text_size(px(11.)).text_color(ui.text_tertiary).child(t);
    let sep = || div().h(px(1.)).my(px(4.)).mx(px(6.)).bg(ui.border);
    let check = |id: String, label: &'static str, on: bool| {
        div()
            .id(SharedString::from(id))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .text_color(ui.text_primary)
            .hover(move |d| d.bg(ui.bg_selected))
            .child(div().w(px(12.)).text_color(ui.accent).child(if on { "✓" } else { "" }))
            .child(label)
    };
    let pick = |keys: &'static [&'static str], v: &'static str| {
        cx.listener(move |_: &mut Workspace, _: &gpui::ClickEvent, _: &mut Window, _: &mut Context<Workspace>| crate::config::set(keys, v))
    };
    let flip = |keys: &'static [&'static str], on: bool| {
        cx.listener(move |_: &mut Workspace, _: &gpui::ClickEvent, _: &mut Window, _: &mut Context<Workspace>| {
            crate::config::set_value(keys, toml_edit::Value::from(!on))
        })
    };
    let mut m = div()
        .absolute()
        .left(at.x)
        .top(at.y)
        .w(px(236.))
        .p(px(5.))
        .rounded(px(8.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    m = m.child(heading("Title as"));
    for (v, label) in [("auto", "Claude title, else command"), ("command", "Command"), ("folder", "Folder"), ("branch", "Branch")] {
        m = m.child(check(format!("sv-title-{v}"), label, c.sidebar.title == v).on_click(pick(&["sidebar", "title"], v)));
    }
    m = m.child(sep()).child(heading("Second line"));
    for (v, label) in [("none", "None"), ("status", "Status / recap"), ("branch", "Branch"), ("folder", "Folder")] {
        m = m.child(check(format!("sv-second-{v}"), label, c.sidebar.second_line == v).on_click(pick(&["sidebar", "second_line"], v)));
    }
    m = m.child(sep()).child(heading("Density"));
    let mut seg = div().flex().gap(px(4.)).px(px(8.)).pb(px(4.));
    for (v, label) in [("compact", "Compact"), ("comfortable", "Comfortable"), ("spacious", "Spacious")] {
        let on = c.interface.density == v;
        seg = seg.child(
            div()
                .id(SharedString::from(format!("sv-density-{v}")))
                .flex_1()
                .h(px(24.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.5))
                .text_color(if on { ui.text_primary } else { ui.text_tertiary })
                .when(on, |d| d.bg(ui.bg_control))
                .hover(move |d| d.text_color(ui.text_primary))
                .on_click(pick(&["interface", "density"], v))
                .child(label),
        );
    }
    m = m.child(seg);
    m = m.child(sep());
    m = m.child(check("sv-icons".into(), "Program icons", c.sidebar.app_icons).on_click(flip(&["sidebar", "app_icons"], c.sidebar.app_icons)));
    m = m.child(check("sv-card".into(), "Show details on hover", c.sidebar.hover_card).on_click(flip(&["sidebar", "hover_card"], c.sidebar.hover_card)));
    m = m.child(sep());
    m = m.child(
        check("sv-more".into(), "More in Settings…", false).on_click(cx.listener(|this, _, window, cx| {
            this.open_overlay(Overlay::Settings { section: crate::settings::Section::Interface }, window, cx)
        })),
    );
    m.into_any_element()
}

/// The session strip's ⋯ menu: appearance, then actions that used to be title-bar buttons.
fn more_menu(ws: &Workspace, at: Point<Pixels>, cx: &mut Context<Workspace>) -> AnyElement {
    use crate::actions::*;
    use t1k_terminal::view as term;
    let ui = ws.ui;
    let item = |id: &'static str, label: &'static str, keys: &'static str| {
        div()
            .id(id)
            .h(px(26.))
            .px(px(10.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(ui.text_primary)
            .hover(move |d| d.bg(ui.bg_selected))
            .child(div().flex_1().child(label))
            .child(shortcut(ui, keys))
    };
    let run = |a: Box<dyn gpui::Action>| {
        let a = std::cell::RefCell::new(Some(a));
        cx.listener(move |this: &mut Workspace, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Workspace>| {
            if let Some(a) = a.borrow_mut().take() {
                this.run_command(crate::palette::Run::Action(a), window, cx);
            }
        })
    };
    let sep = || div().h(px(1.)).my(px(4.)).mx(px(6.)).bg(ui.border);
    let mode = ws.config.appearance.clone();
    let mut seg = div().flex().gap(px(4.)).px(px(8.)).py(px(4.));
    for (v, label) in [("system", "System"), ("dark", "Dark"), ("light", "Light")] {
        let on = mode == v;
        seg = seg.child(
            div()
                .id(SharedString::from(format!("more-mode-{v}")))
                .flex_1()
                .h(px(24.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(if on { ui.text_primary } else { ui.text_tertiary })
                .when(on, |d| d.bg(ui.bg_control))
                .hover(move |d| d.text_color(ui.text_primary))
                .on_click(cx.listener(move |_, _, _, _| crate::config::set(&["appearance"], v)))
                .child(label),
        );
    }
    div()
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
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(crate::updater::waiting(), |d| {
            let v = crate::updater::snapshot().version.unwrap_or_default();
            d.child(
                div()
                    .id("more-update")
                    .h(px(26.))
                    .px(px(10.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .text_color(ui.accent)
                    .hover(move |d| d.bg(ui.bg_selected))
                    .on_click(cx.listener(|this, _, window, cx| this.open_update(window, cx)))
                    .child(div().flex_1().child(format!("Update to {v}…")))
                    .child(div().text_size(px(11.5)).text_color(ui.text_tertiary).child("ready")),
            )
            .child(sep())
        })
        .child(div().px(px(10.)).pt(px(4.)).text_size(px(11.)).text_color(ui.text_tertiary).child("Appearance"))
        .child(seg)
        .child(sep())
        .child(item("more-overview", "Overview", "⌘O").on_click(run(Box::new(ToggleOverview))))
        .child(item("more-find", "Find in terminal", "⌘F").on_click(run(Box::new(term::Find))))
        .child(item("more-split", "Split right", "⌘D").on_click(run(Box::new(SplitRight))))
        .child(item("more-rename", "Rename tab", "⌘⇧R").on_click(run(Box::new(RenameTab))))
        .child(item("more-remind", "Remind me…", "").on_click(cx.listener(move |this, _, window, cx| {
            let tab = this.active;
            this.open_overlay(Overlay::Menu { tab, at }, window, cx);
        })))
        .child(sep())
        .child(item("more-palette", "Command palette", "⌘⇧P").on_click(run(Box::new(ToggleCommandPalette))))
        .child(item("more-settings", "Settings", "⌘,").on_click(run(Box::new(OpenSettings))))
        .into_any_element()
}

/// A rename field: the text with its selection and caret; click to place the caret, drag
/// to select, double-click for a word.
fn edit_field(ws: &Workspace, text: &str, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let sel = ws.edit_sel;
    let r = sel.range();
    let (lo, hi) = (r.start.min(text.len()), r.end.min(text.len()));
    let caret = || div().w(px(1.5)).h(px(17.)).flex_none().bg(ui.accent);
    let bounds = ws.edit_bounds.clone();
    let line = div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(gpui::canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {}).absolute().size_full())
        .child(div().child(text[..lo].to_string()))
        .when(lo == hi, |d| d.child(caret()))
        .child(div().bg(ui.accent.opacity(0.3)).child(text[lo..hi].to_string()))
        .child(div().child(text[hi..].to_string()));
    div()
        .id("edit-field")
        .h(px(34.))
        .px(px(10.))
        .rounded(px(6.))
        .bg(ui.bg_control)
        .border_1()
        .border_color(ui.accent.opacity(0.5))
        .flex()
        .items_center()
        .cursor(gpui::CursorStyle::IBeam)
        .text_size(px(14.))
        .text_color(ui.text_primary)
        .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &gpui::MouseDownEvent, window, cx| {
            let Some(i) = this.edit_index_at(ev.position, window) else { return };
            let text = this.edit_text().unwrap_or_default();
            this.edit_sel = match ev.click_count {
                2 => input::word_at(&text, i),
                n if n >= 3 => input::Sel::all(&text),
                _ if ev.modifiers.shift => input::Sel { anchor: this.edit_sel.anchor, cursor: i },
                _ => input::Sel { anchor: i, cursor: i },
            };
            this.edit_dragging = ev.click_count == 1;
            cx.notify();
        }))
        .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, window, cx| {
            if !this.edit_dragging || ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            if let Some(i) = this.edit_index_at(ev.position, window) {
                this.edit_sel.cursor = i;
                cx.notify();
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.edit_dragging = false))
        .child(line)
        .into_any_element()
}

impl Workspace {
    fn edit_text(&self) -> Option<String> {
        match &self.overlay {
            Overlay::Rename { text, .. } | Overlay::RenameGroup { text, .. } => Some(text.clone()),
            _ => None,
        }
    }

    /// Byte offset in the rename field nearest a window position.
    fn edit_index_at(&self, pos: Point<Pixels>, window: &mut Window) -> Option<usize> {
        let text = self.edit_text()?;
        let b = self.edit_bounds.get()?;
        let x = pos.x - b.origin.x;
        if text.is_empty() {
            return Some(0);
        }
        let font = window.text_style().font();
        let run = gpui::TextRun { len: text.len(), font, color: self.ui.text_primary, background_color: None, underline: None, strikethrough: None };
        let shaped = window.text_system().shape_line(text.clone().into(), px(14.), &[run], None);
        Some(shaped.closest_index_for_x(x).min(text.len()))
    }
}
