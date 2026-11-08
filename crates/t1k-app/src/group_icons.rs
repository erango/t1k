//! Group icons (design v3-n2): an emoji you choose per group, with suggestions worked out
//! ahead of time so the picker never waits, and a searchable palette for anything else.

use std::path::{Path, PathBuf};

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, Pixels, Point, SharedString, Window};

use crate::input::{self, Key};
use crate::overlay::Overlay;
use crate::workspace::Workspace;

pub const COLS: usize = 8;

/// What the picker sets the icon of: a group (by key) or a single tab (by id).
#[derive(Clone, Debug, PartialEq)]
pub enum IconFor {
    Group(PathBuf),
    Tab(String),
}
const VISIBLE_ROWS: usize = 6;

/// Words in a project's name, README title or tab titles, and the emoji they suggest.
const KEYWORDS: &[(&str, &str)] = &[
    ("rust", "🦀"), ("cargo", "🦀"), ("terminal", "💻"), ("term", "💻"), ("shell", "🐚"), ("cli", "⌨️"),
    ("terraform", "🏗️"), ("infra", "🏗️"), ("k8s", "☸️"), ("kube", "☸️"), ("helm", "⎈"), ("docker", "🐳"), ("deploy", "🚢"),
    ("db", "🗄️"), ("sql", "🗄️"), ("postgres", "🐘"), ("mysql", "🐬"), ("redis", "🧱"), ("data", "📊"), ("analytics", "📈"),
    ("api", "🔌"), ("server", "🖥️"), ("backend", "⚙️"), ("web", "🌐"), ("site", "🌐"), ("frontend", "🎨"), ("ui", "🎨"),
    ("design", "🎨"), ("dashboard", "📊"), ("docs", "📚"), ("doc", "📚"), ("blog", "✍️"), ("test", "🧪"), ("qa", "🧪"),
    ("release", "🚀"), ("launch", "🚀"), ("auth", "🔐"), ("security", "🛡️"), ("pay", "💳"), ("billing", "💳"),
    ("mail", "✉️"), ("email", "✉️"), ("chat", "💬"), ("slack", "💬"), ("bot", "🤖"), ("agent", "🤖"), ("ai", "🧠"),
    ("ml", "🧠"), ("llm", "🧠"), ("mobile", "📱"), ("ios", "🍎"), ("android", "🤖"), ("game", "🎮"), ("music", "🎵"),
    ("video", "🎬"), ("photo", "📷"), ("map", "🗺️"), ("hotel", "🏨"), ("travel", "✈️"), ("event", "🎟️"), ("calendar", "📅"),
    ("python", "🐍"), ("go", "🐹"), ("node", "📦"), ("js", "⚡"), ("ts", "🔷"), ("react", "⚛️"), ("swift", "🐦"),
    ("dotfiles", "🧰"), ("config", "🔧"), ("tools", "🧰"), ("bug", "🐛"), ("fix", "🩹"), ("perf", "⚡"), ("search", "🔎"),
    ("notes", "🗒️"), ("todo", "✅"), ("home", "🏠"), ("finance", "💰"), ("money", "💰"), ("shop", "🛒"), ("store", "🛒"),
    ("week", "📅"), ("sprint", "🏃"), ("review", "👀"), ("ops", "🛠️"), ("monitor", "📟"), ("log", "📜"), ("flag", "🚩"),
    ("sites", "🌐"), ("website", "🌐"), ("skills", "🎓"), ("skill", "🎓"), ("claude", "🤖"), ("find", "🔎"), ("file", "🗂️"),
    ("files", "🗂️"), ("index", "🗂️"), ("cache", "🗃️"), ("queue", "📬"), ("sync", "🔄"), ("backup", "💾"), ("image", "🖼️"),
    ("pdf", "📄"), ("translate", "🌍"), ("i18n", "🌍"), ("learn", "🎓"), ("course", "🎓"), ("health", "🩺"), ("food", "🍽️"),
    ("recipe", "🍳"), ("weather", "⛅"), ("time", "⏱️"), ("timer", "⏱️"), ("news", "📰"), ("rss", "📰"), ("podcast", "🎙️"),
    ("form", "📝"), ("survey", "📝"), ("crm", "🤝"), ("sales", "🤝"), ("support", "🛟"), ("ticket", "🎫"), ("tv", "📺"),
    ("plugin", "🧩"), ("extension", "🧩"), ("theme", "🎨"), ("icon", "🖼️"), ("font", "🔤"), ("markdown", "📝"), ("wiki", "📖"),
];

/// Files that say what a project is built with.
const MARKERS: &[(&str, &str)] = &[
    ("Cargo.toml", "🦀"), ("go.mod", "🐹"), ("pyproject.toml", "🐍"), ("requirements.txt", "🐍"), ("package.json", "📦"),
    ("Dockerfile", "🐳"), ("main.tf", "🏗️"), ("Gemfile", "💎"), ("Package.swift", "🐦"), ("pom.xml", "☕"), ("build.gradle", "☕"),
];

/// When nothing in a project suggests an icon: a varied pool, picked by the group's name,
/// so different groups get different choices.
const POOL: &[&str] = &[
    "🦊", "🐙", "🦉", "🐝", "🦋", "🐢", "🐳", "🦜", "🐧", "🦔", "🌵", "🌲", "🍀", "🌻", "🍄", "🌊", "🔥", "⛰️", "🌙", "☀️",
    "⭐", "🪐", "🌈", "❄️", "⚡", "🍋", "🍒", "🥑", "🍩", "☕", "🎯", "🎲", "🧩", "🎈", "🪁", "🏁", "🧭", "🔭", "🧪", "🔬",
    "💎", "🔑", "🧲", "🪄", "🛰️", "⚓", "🗿", "🏔️", "🏝️", "🚲", "⛵", "🎸", "🎨", "📐", "🧱", "🪴", "🕯️", "📯", "🧿", "🔮",
];

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// Up to 5 emoji for a group, without a model: keywords in its name, its README's first
/// heading and its tabs' titles, then what the project is built with.
pub fn suggest_local(key: &Path, label: &str, titles: &[String]) -> Vec<String> {
    // What the project says about itself: its name, README (title and opening), and the
    // description in its manifest; then its tabs' titles.
    let mut own = vec![label.to_string()];
    if key.is_dir() {
        if let Ok(readme) = std::fs::read_to_string(key.join("README.md")) {
            own.extend(readme.lines().filter(|l| !l.trim().is_empty() && !l.contains("](") && !l.starts_with("```")).take(12).map(str::to_string));
        }
        for m in ["Cargo.toml", "package.json", "pyproject.toml"] {
            if let Ok(t) = std::fs::read_to_string(key.join(m)) {
                own.extend(t.lines().filter(|l| l.trim_start().trim_start_matches('"').starts_with("description")).map(str::to_string));
            }
        }
    }
    let mut out: Vec<String> = Vec::new();
    let push = |e: &str, out: &mut Vec<String>| {
        if out.len() < 5 && !out.iter().any(|x| x == e) {
            out.push(e.to_string());
        }
    };
    // Words that say more come first: keywords anywhere in what the project says about
    // itself, then emoji named like its words ("rocket", "whale"), then tab-title keywords
    // (titles are full of words like "mode" or "link" that make poor icons).
    let own_words: Vec<String> = own.iter().flat_map(|t| words(t)).collect();
    for w in &own_words {
        if let Some((_, e)) = KEYWORDS.iter().find(|(k, _)| k == w) {
            push(e, &mut out);
        }
    }
    if key.is_dir() {
        for (file, e) in MARKERS {
            if key.join(file).exists() {
                push(e, &mut out);
            }
        }
    }
    for w in own_words.iter().filter(|w| w.len() >= 4 && !STOP.contains(&w.as_str())) {
        // Only emoji whose whole name is the word: "rocket", "crab", "globe". Matching any
        // word of a longer name brought in noise like ⏭ for "next".
        if let Some(e) = palette().find(|e| e.name() == w) {
            push(e.as_str(), &mut out);
        }
    }
    for w in titles.iter().flat_map(|t| words(t)) {
        if let Some((_, e)) = KEYWORDS.iter().find(|(k, _)| *k == w) {
            push(e, &mut out);
        }
    }
    // Fill from the pool, starting at a place set by the group's name.
    let h = key.to_string_lossy().bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619)) as usize;
    let mut i = 0;
    while out.len() < 5 && i < POOL.len() {
        push(POOL[(h + i * 7) % POOL.len()], &mut out);
        i += 1;
    }
    out
}

/// Up to 5 emoji for one tab, without a model: keywords and emoji names in what it's
/// working on, then its group's suggestions, then the pool (seeded by the tab, so tabs
/// in one group don't all get the same filler).
pub fn suggest_tab(texts: &[String], group: &[String], seed: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push = |e: &str, out: &mut Vec<String>| {
        if out.len() < 5 && !out.iter().any(|x| x == e) {
            out.push(e.to_string());
        }
    };
    let ws: Vec<String> = texts.iter().flat_map(|t| words(t)).collect();
    for w in &ws {
        if let Some((_, e)) = KEYWORDS.iter().find(|(k, _)| k == w) {
            push(e, &mut out);
        }
    }
    for w in ws.iter().filter(|w| w.len() >= 4 && !STOP.contains(&w.as_str())) {
        if let Some(e) = palette().find(|e| e.name() == w) {
            push(e.as_str(), &mut out);
        }
    }
    for e in group.iter().take(2) {
        push(e, &mut out);
    }
    let h = seed.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619)) as usize;
    let mut i = 0;
    while out.len() < 5 && i < POOL.len() {
        push(POOL[(h + i * 7) % POOL.len()], &mut out);
        i += 1;
    }
    out
}

pub fn tab_icon_prompt(title: &str, project: &str, detail: Option<&str>) -> String {
    format!(
        "Suggest 5 different emoji that would make a good icon for this task, so it is easy to spot in a list of tabs.\nTask: {title}\nProject: {project}\n{}Reply with the 5 emoji only, separated by spaces.",
        detail.map(|d| format!("Detail: {d}\n")).unwrap_or_default()
    )
}

/// Common words that happen to name an emoji but say nothing about a project.
const STOP: &[&str] = &["this", "that", "with", "from", "your", "into", "will", "have", "when", "what", "which", "make", "made", "mode", "link", "page", "free", "open", "back", "new", "top", "up", "down", "right", "left", "information", "input", "card"];

/// Emoji the system font draws (the newest ones show as boxes on older macOS).
fn palette() -> impl Iterator<Item = &'static emojis::Emoji> {
    emojis::iter().filter(|e| e.emoji_version() < emojis::EmojiVersion::new(15, 0))
}

/// Emoji in `reply` (the helper's answer), in order, at most 5.
pub fn parse_emoji(reply: &str) -> Vec<String> {
    let mut found: Vec<(usize, &str)> = emojis::iter()
        .filter_map(|e| reply.find(e.as_str()).map(|i| (i, e.as_str())))
        .collect();
    found.sort_by_key(|(i, e)| (*i, std::cmp::Reverse(e.len())));
    let mut out: Vec<String> = Vec::new();
    let mut end = 0;
    for (i, e) in found {
        // Skip pieces of a longer emoji already taken (a ZWJ sequence's parts).
        if i < end {
            continue;
        }
        end = i + e.len();
        out.push(e.to_string());
        if out.len() == 5 {
            break;
        }
    }
    out
}

pub fn icon_prompt(label: &str, readme_title: Option<&str>, titles: &[String]) -> String {
    format!(
        "Suggest 5 different emoji that would make a good icon for this software project, so it is easy to spot in a list.\nProject: {label}\n{}Recent work: {}\nReply with the 5 emoji only, separated by spaces.",
        readme_title.map(|t| format!("README: {t}\n")).unwrap_or_default(),
        titles.iter().take(5).cloned().collect::<Vec<_>>().join("; ")
    )
}

/// Extra words people search by that emoji names don't contain.
const ALIASES: &[(&str, &str)] = &[("db", "database file cabinet"), ("bug", "bug lady beetle"), ("k8s", "wheel helm"), ("ai", "robot brain"), ("money", "money dollar"), ("fire", "fire")];

pub fn search(query: &str) -> Vec<&'static str> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let extra: Vec<&str> = ALIASES.iter().filter(|(a, _)| a.starts_with(&q)).flat_map(|(_, w)| w.split(' ')).collect();
    let mut hits: Vec<(u8, &'static str)> = Vec::new();
    for (k, e) in KEYWORDS {
        if k.starts_with(&q) {
            hits.push((0, e));
        }
    }
    for e in palette() {
        let name = e.name();
        let rank = if name.split(' ').any(|w| w.starts_with(&q)) || extra.iter().any(|x| name.split(' ').any(|w| w == *x)) {
            1
        } else if e.shortcodes().any(|s| s.contains(&q)) || name.contains(&q) {
            2
        } else {
            continue;
        };
        hits.push((rank, e.as_str()));
    }
    hits.sort_by_key(|(r, _)| *r);
    let mut out: Vec<&'static str> = Vec::new();
    for (_, e) in hits {
        if !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

pub fn category(i: usize) -> Vec<&'static str> {
    emojis::Group::iter()
        .nth(i)
        .map(|g| g.emojis().filter(|e| e.emoji_version() < emojis::EmojiVersion::new(15, 0)).map(|e| e.as_str()).collect())
        .unwrap_or_default()
}

fn category_icon(i: usize) -> &'static str {
    ["😀", "👋", "🐻", "🍔", "✈️", "⚽", "💡", "🔣", "🏁"].get(i).copied().unwrap_or("·")
}

/// What the picker's arrows walk: suggestions then the grid (or search results).
pub fn items(ws: &Workspace, key: &IconFor, query: &str, cat: usize) -> (Vec<String>, Vec<&'static str>) {
    if !query.trim().is_empty() {
        return (Vec::new(), search(query));
    }
    (ws.icon_suggestions_for(key), category(cat))
}

impl Workspace {
    pub fn open_icon_picker(&mut self, key: IconFor, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_suggestions_for(&key, cx);
        self.open_overlay(Overlay::IconPicker { key, at, query: String::new(), selected: 0, cat: 0 }, window, cx);
    }

    pub fn set_group_icon(&mut self, key: &Path, icon: Option<String>, cx: &mut Context<Self>) {
        match icon {
            Some(e) => {
                self.group_icons.insert(key.to_path_buf(), e);
            }
            None => {
                self.group_icons.remove(key);
            }
        }
        self.save();
        cx.notify();
    }

    pub fn set_icon(&mut self, key: &IconFor, icon: Option<String>, cx: &mut Context<Self>) {
        match key {
            IconFor::Group(k) => self.set_group_icon(k, icon, cx),
            IconFor::Tab(id) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| &t.id == id) {
                    t.icon = icon;
                }
                self.save();
                cx.notify();
            }
        }
    }

    pub fn icon_of(&self, key: &IconFor) -> Option<String> {
        match key {
            IconFor::Group(k) => self.group_icons.get(k).cloned(),
            IconFor::Tab(id) => self.tabs.iter().find(|t| &t.id == id).and_then(|t| t.icon.clone()),
        }
    }

    pub fn icon_suggestions_for(&self, key: &IconFor) -> Vec<String> {
        match key {
            IconFor::Group(k) => self.icon_suggestions(k),
            IconFor::Tab(id) => self.tab_icon_suggestions.get(id).cloned().unwrap_or_default(),
        }
    }

    /// Suggestions are worked out when the picker first opens for it: right away from the
    /// words on hand, then replaced by the helper's picks if it's on.
    pub fn ensure_suggestions_for(&mut self, key: &IconFor, cx: &mut Context<Self>) {
        let id = match key {
            IconFor::Group(k) => return self.ensure_icon_suggestions(k, cx),
            IconFor::Tab(id) => id,
        };
        if self.tab_icon_suggestions.contains_key(id) {
            return;
        }
        let Some(t) = self.tabs.iter().find(|t| &t.id == id) else { return };
        let group = t.group.clone();
        let title = t.title();
        let detail = t.agent.as_ref().and_then(|a| a.recap.clone().or_else(|| a.first_prompt.clone()));
        let texts: Vec<String> = std::iter::once(title.clone()).chain(detail.clone()).collect();
        self.ensure_icon_suggestions(&group, cx);
        let local = suggest_tab(&texts, &self.icon_suggestions(&group), id);
        self.tab_icon_suggestions.insert(id.clone(), local);
        let project = self.group_label(&group);
        self.helper_tab_icon(id, &title, &project, detail.as_deref(), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn icon_picker_key(&mut self, key: IconFor, at: Point<Pixels>, mut query: String, mut selected: usize, cat: usize, ev: &KeyDownEvent, cx: &mut Context<Self>) -> Option<Overlay> {
        if ev.keystroke.modifiers.platform && ev.keystroke.key == "backspace" {
            self.set_icon(&key, None, cx);
            return None;
        }
        let (sugg, grid) = items(self, &key, &query, cat);
        let n = sugg.len() + grid.len();
        // Arrows: ← → by one, ↑ ↓ by a row; the suggestions row counts as its own row.
        let row_of = |i: usize| if i < sugg.len() { 0 } else { 1 + (i - sugg.len()) / COLS };
        match ev.keystroke.key.as_str() {
            "left" => selected = selected.saturating_sub(1),
            "right" => selected = (selected + 1).min(n.saturating_sub(1)),
            "up" => {
                selected = match row_of(selected) {
                    0 => selected,
                    1 if !sugg.is_empty() => (selected - sugg.len()).min(sugg.len() - 1),
                    _ => selected.saturating_sub(COLS),
                }
            }
            "down" => {
                selected = if selected < sugg.len() { sugg.len() + selected } else { selected + COLS }.min(n.saturating_sub(1));
            }
            _ => match input::handle(&mut query, ev) {
                Key::Cancel if !query.is_empty() => {
                    query.clear();
                    selected = 0;
                }
                Key::Cancel => return None,
                Key::Submit => {
                    let pick = if selected < sugg.len() { sugg.get(selected).cloned() } else { grid.get(selected - sugg.len()).map(|s| s.to_string()) };
                    if pick.is_some() {
                        self.set_icon(&key, pick, cx);
                    }
                    return None;
                }
                Key::Edited => selected = 0,
                _ => {}
            },
        }
        Some(Overlay::IconPicker { key, at, query, selected, cat })
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(ws: &Workspace, key: &IconFor, at: Point<Pixels>, query: &str, selected: usize, cat: usize, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let (sugg, grid) = items(ws, key, query, cat);
    let current = ws.icon_of(key);
    let cell = |id: String, e: String, on: bool| {
        let k = key.clone();
        let pick = e.clone();
        div()
            .id(SharedString::from(id))
            .size(px(32.))
            .flex_none()
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(19.))
            .cursor_pointer()
            .when(on, |d| d.bg(ui.bg_selected).border_1().border_color(ui.accent))
            .when(!on, |d| d.hover(move |d| d.bg(ui.bg_hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.set_icon(&k, Some(pick.clone()), cx);
                this.close_overlay(window, cx);
            }))
            .child(e)
            .into_any_element()
    };
    let label = |t: String| div().px(px(4.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(t);

    let mut body = div().flex().flex_col().gap(px(6.));
    if !sugg.is_empty() {
        body = body
            .child(label(format!("Suggested for {}", match key {
                IconFor::Group(k) => ws.group_label(k),
                IconFor::Tab(id) => ws.tabs.iter().find(|t| &t.id == id).map(|t| t.title()).unwrap_or_default(),
            })))
            .child(div().flex().gap(px(2.)).children(sugg.iter().enumerate().map(|(i, e)| cell(format!("sg-{i}"), e.clone(), selected == i))));
    }
    // Search field: typing anywhere in the picker goes here, so it always looks focused (accent
    // border, caret) and takes clicks without closing the picker.
    let caret = div().w(px(1.5)).h(px(15.)).flex_none().bg(ui.accent);
    body = body.child(
        div()
            .id("emoji-search")
            .h(px(30.))
            .px(px(10.))
            .rounded(px(7.))
            .bg(ui.bg_sidebar)
            .border_1()
            .border_color(ui.accent.opacity(0.6))
            .cursor(gpui::CursorStyle::IBeam)
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .child(crate::icons::search(ui.text_tertiary))
            .child(if query.is_empty() {
                div().flex().items_center().child(caret).child(div().ml(px(1.)).text_color(ui.text_tertiary).child("Search emoji"))
            } else {
                div().flex().items_center().text_color(ui.text_primary).child(query.to_string()).child(caret)
            }),
    );
    if query.trim().is_empty() {
        let k = key.clone();
        body = body.child(div().flex().justify_between().children((0..9).map(|i| {
            let k = k.clone();
            let on = i == cat;
            div()
                .id(SharedString::from(format!("cat-{i}")))
                .size(px(28.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(15.))
                .when(on, |d| d.bg(ui.bg_selected))
                .when(!on, |d| d.opacity(0.6).hover(move |d| d.bg(ui.bg_hover).opacity(1.)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Overlay::IconPicker { query, at, .. } = &this.overlay {
                        let (q, at) = (query.clone(), *at);
                        let n = this.icon_suggestions_for(&k).len();
                        this.overlay = Overlay::IconPicker { key: k.clone(), at, query: q, selected: n, cat: i };
                        cx.notify();
                    }
                }))
                .child(category_icon(i))
        })));
    }
    // A window of grid rows that follows the selection.
    let gsel = selected.checked_sub(sugg.len());
    let rows = grid.len().div_ceil(COLS);
    let first = gsel.map(|g| g / COLS).unwrap_or(0).saturating_sub(VISIBLE_ROWS / 2).min(rows.saturating_sub(VISIBLE_ROWS));
    let mut g = div().flex().flex_col().gap(px(2.));
    for r in first..(first + VISIBLE_ROWS).min(rows) {
        g = g.child(div().flex().gap(px(2.)).children((0..COLS).filter_map(|c| {
            let i = r * COLS + c;
            grid.get(i).map(|e| cell(format!("g-{i}"), e.to_string(), gsel == Some(i)))
        })));
    }
    if grid.is_empty() && !query.trim().is_empty() {
        g = g.child(div().p(px(8.)).text_size(px(12.5)).text_color(ui.text_tertiary).child("No emoji match"));
    }
    body = body.child(div().h(px(VISIBLE_ROWS as f32 * 34.)).child(g));
    if current.is_some() {
        let k = key.clone();
        body = body.child(
            div()
                .id("icon-remove")
                .h(px(28.))
                .px(px(8.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .text_size(px(12.5))
                .text_color(ui.text_secondary)
                .hover(move |d| d.bg(ui.bg_hover))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_icon(&k, None, cx);
                    this.close_overlay(window, cx);
                }))
                .child(div().flex_1().child("Remove icon"))
                .child(div().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child("⌘⌫")),
        );
    }
    let vh = f32::from(window.viewport_size().height);
    let top = f32::from(at.y).min(vh - 420.).max(8.);
    div()
        .absolute()
        .left(at.x)
        .top(px(top))
        .w(px(300.))
        .p(px(8.))
        .rounded(px(10.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_suggestions() {
        let s = suggest_tab(&["Fix the docker deploy".into()], &["🦀".into()], "tab-1");
        assert_eq!(s.len(), 5);
        assert_eq!(&s[..3], &["🩹".to_string(), "🐳".to_string(), "🚢".to_string()]);
        assert!(s.contains(&"🦀".to_string()));
    }

    #[test]
    fn local_suggestions() {
        let s = suggest_local(Path::new("/nonexistent/t1k"), "t1k", &["Fix the terminal renderer in Rust".into()]);
        assert_eq!(s.len(), 5);
        assert!(s.contains(&"🦀".to_string()));
        assert!(s.contains(&"💻".to_string()));
    }

    #[test]
    fn parses_helper_reply() {
        assert_eq!(parse_emoji("🦀 💻 🚀 ⚡ 🧪 🐳"), vec!["🦀", "💻", "🚀", "⚡", "🧪"]);
        assert_eq!(parse_emoji("Sure: 🏗️, 🗄️"), vec!["🏗️", "🗄️"]);
    }

    #[test]
    fn searches_names_and_aliases() {
        assert!(search("rocket").contains(&"🚀"));
        assert!(search("db").contains(&"🗄️"));
        assert!(search("").is_empty());
    }
}
