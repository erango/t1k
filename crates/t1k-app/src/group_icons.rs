//! Group icons (design v3-n2): an emoji you choose per group, with suggestions worked out
//! ahead of time so the picker never waits, and a searchable palette for anything else.

use std::path::{Path, PathBuf};

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, Pixels, Point, SharedString, Window};

use crate::input::{self, Key};
use crate::overlay::Overlay;
use crate::workspace::Workspace;

pub const COLS: usize = 8;
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
];

/// Files that say what a project is built with.
const MARKERS: &[(&str, &str)] = &[
    ("Cargo.toml", "🦀"), ("go.mod", "🐹"), ("pyproject.toml", "🐍"), ("requirements.txt", "🐍"), ("package.json", "📦"),
    ("Dockerfile", "🐳"), ("main.tf", "🏗️"), ("Gemfile", "💎"), ("Package.swift", "🐦"), ("pom.xml", "☕"), ("build.gradle", "☕"),
];

const FALLBACK: &[&str] = &["⭐", "🧭", "🛠️", "💡", "📁", "🌱", "🔭", "🧩"];

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// Up to 5 emoji for a group, without a model: keywords in its name, its README's first
/// heading and its tabs' titles, then what the project is built with.
pub fn suggest_local(key: &Path, label: &str, titles: &[String]) -> Vec<String> {
    let mut text = vec![label.to_string()];
    if key.is_dir() {
        if let Ok(readme) = std::fs::read_to_string(key.join("README.md")) {
            if let Some(h) = readme.lines().find(|l| l.starts_with('#')) {
                text.push(h.trim_start_matches('#').to_string());
            }
        }
    }
    text.extend(titles.iter().cloned());
    let mut out: Vec<String> = Vec::new();
    let mut push = |e: &str| {
        if out.len() < 5 && !out.iter().any(|x| x == e) {
            out.push(e.to_string());
        }
    };
    // The group's own name and README title first, then its tabs' titles (keywords only:
    // titles are full of words like "mode" or "link" that make poor icons).
    let own = text.len() - titles.len();
    for (n, w) in text.iter().enumerate().flat_map(|(n, t)| words(t).into_iter().map(move |w| (n, w))) {
        if let Some((_, e)) = KEYWORDS.iter().find(|(k, _)| *k == w) {
            push(e);
        }
        // Emoji whose own name is the word ("rocket", "whale").
        if n < own && w.len() >= 4 {
            if let Some(e) = palette().find(|e| e.name().split(' ').any(|n| n == w)) {
                push(e.as_str());
            }
        }
    }
    if key.is_dir() {
        for (file, e) in MARKERS {
            if key.join(file).exists() {
                push(e);
            }
        }
    }
    for e in FALLBACK {
        push(e);
    }
    out
}

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
pub fn items(ws: &Workspace, key: &Path, query: &str, cat: usize) -> (Vec<String>, Vec<&'static str>) {
    if !query.trim().is_empty() {
        return (Vec::new(), search(query));
    }
    (ws.icon_suggestions(key), category(cat))
}

impl Workspace {
    pub fn open_icon_picker(&mut self, key: PathBuf, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_icon_suggestions(&key, cx);
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

    #[allow(clippy::too_many_arguments)]
    pub fn icon_picker_key(&mut self, key: PathBuf, at: Point<Pixels>, mut query: String, mut selected: usize, cat: usize, ev: &KeyDownEvent, cx: &mut Context<Self>) -> Option<Overlay> {
        if ev.keystroke.modifiers.platform && ev.keystroke.key == "backspace" {
            self.set_group_icon(&key, None, cx);
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
                        self.set_group_icon(&key, pick, cx);
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
pub fn render(ws: &Workspace, key: &Path, at: Point<Pixels>, query: &str, selected: usize, cat: usize, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let (sugg, grid) = items(ws, key, query, cat);
    let current = ws.group_icons.get(key).cloned();
    let cell = |id: String, e: String, on: bool| {
        let k = key.to_path_buf();
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
                this.set_group_icon(&k, Some(pick.clone()), cx);
                this.close_overlay(window, cx);
            }))
            .child(e)
            .into_any_element()
    };
    let label = |t: String| div().px(px(4.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(t);

    let mut body = div().flex().flex_col().gap(px(6.));
    if !sugg.is_empty() {
        body = body
            .child(label(format!("Suggested for {}", ws.group_label(&key.to_path_buf()))))
            .child(div().flex().gap(px(2.)).children(sugg.iter().enumerate().map(|(i, e)| cell(format!("sg-{i}"), e.clone(), selected == i))));
    }
    // Search field: typing anywhere goes here.
    body = body.child(
        div()
            .h(px(30.))
            .px(px(10.))
            .rounded(px(7.))
            .bg(ui.bg_sidebar)
            .border_1()
            .border_color(if query.is_empty() { ui.border } else { ui.accent })
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .child(crate::icons::search(ui.text_tertiary))
            .child(if query.is_empty() {
                div().text_color(ui.text_tertiary).child("Search emoji")
            } else {
                div().text_color(ui.text_primary).child(format!("{query}▏"))
            }),
    );
    if query.trim().is_empty() {
        let k = key.to_path_buf();
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
                        let n = this.icon_suggestions(&k).len();
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
        let k = key.to_path_buf();
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
                    this.set_group_icon(&k, None, cx);
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
