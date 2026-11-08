//! Group icons (design v3-n2): an emoji you choose per group, with suggestions worked out
//! ahead of time so the picker never waits, and a searchable palette for anything else. Its
//! Apps and Image tabs (canvas v3-ag A) set an installed app's icon or an image instead.

use std::path::{Path, PathBuf};

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, Pixels, Point, SharedString, Window};

use crate::custom_icon::Icon;
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

/// The picker's tabs (canvas v3-ag A): each source gets a view that suits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Emoji,
    Apps,
    Image,
}

const MODES: [(Mode, &str); 3] = [(Mode::Emoji, "Emoji"), (Mode::Apps, "Apps"), (Mode::Image, "Image")];

/// Picker state that outlives a keystroke (the overlay variant holds the query and selection).
#[derive(Default)]
pub struct PickerUi {
    pub mode: Mode,
    /// First visible line of the scrolling list.
    pub scroll: usize,
    /// Wheel movement not yet a whole line.
    scroll_acc: f32,
    pub crop: Option<crate::icon_crop::Crop>,
    /// An image is being read or saved.
    pub busy: bool,
    pub error: Option<&'static str>,
}

/// Something the picker can set: an emoji, or an installed app (index into `custom_icon::apps()`).
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Emoji(String),
    App(usize),
}

/// A run of picks laid out in a grid, under an optional label.
pub struct Section {
    pub label: Option<String>,
    pub picks: Vec<Pick>,
    pub cols: usize,
    pub cell: f32,
}

const EMOJI_CELL: f32 = 34.;
const APP_CELL: f32 = 42.;
const APP_COLS: usize = 6;
/// Height of the scrolling list (six emoji rows, as before the tabs).
const LIST_H: f32 = 204.;
const LABEL_H: f32 = 20.;

fn label_for(ws: &Workspace, key: &IconFor) -> String {
    match key {
        IconFor::Group(k) => ws.group_label(k),
        IconFor::Tab(id) => ws.tabs.iter().find(|t| &t.id == id).map(|t| t.title()).unwrap_or_default(),
    }
}

/// The names an app suggestion is matched against: the tab's title, its group's name and the
/// repo folder's name.
fn name_texts(ws: &Workspace, key: &IconFor) -> Vec<String> {
    let group = match key {
        IconFor::Group(k) => k.clone(),
        IconFor::Tab(id) => match ws.tabs.iter().find(|t| &t.id == id) {
            Some(t) => t.group.clone(),
            None => return Vec::new(),
        },
    };
    let mut v = vec![ws.group_label(&group)];
    v.extend(group.file_name().map(|n| n.to_string_lossy().to_string()));
    if let IconFor::Tab(id) = key {
        v.extend(ws.tabs.iter().find(|t| &t.id == id).map(|t| t.title()));
    }
    v
}

/// What the picker shows and the arrows walk: suggestions (above the list), then the list's
/// sections. Flat indexes count the suggestions first.
pub fn items(ws: &Workspace, key: &IconFor, query: &str, cat: usize) -> (Option<Section>, Vec<Section>) {
    let apps = crate::custom_icon::apps();
    let q = query.trim();
    let names = || name_texts(ws, key);
    let emoji = |v: Vec<&'static str>| v.into_iter().map(|e| Pick::Emoji(e.to_string())).collect::<Vec<_>>();
    match ws.icon_ui.mode {
        Mode::Image => (None, Vec::new()),
        Mode::Emoji if q.is_empty() => {
            // One matching app may lead the suggestions, so the common case is still one click.
            let mut sugg: Vec<Pick> = crate::custom_icon::suggest_apps(&apps, &names(), 60, 1).into_iter().map(Pick::App).collect();
            sugg.extend(ws.icon_suggestions_for(key).into_iter().map(Pick::Emoji));
            let top = (!sugg.is_empty()).then(|| Section { label: Some(format!("Suggested for {}", label_for(ws, key))), picks: sugg, cols: COLS, cell: EMOJI_CELL });
            (top, vec![Section { label: None, picks: emoji(category(cat)), cols: COLS, cell: EMOJI_CELL }])
        }
        Mode::Emoji => {
            let mut picks: Vec<Pick> = crate::custom_icon::search_apps(&apps, q).into_iter().take(1).map(Pick::App).collect();
            picks.extend(emoji(search(q)));
            (None, vec![Section { label: None, picks, cols: COLS, cell: EMOJI_CELL }])
        }
        Mode::Apps if q.is_empty() => {
            let sugg: Vec<Pick> = crate::custom_icon::suggest_apps(&apps, &names(), 1, APP_COLS).into_iter().map(Pick::App).collect();
            let top = (!sugg.is_empty()).then(|| Section { label: Some(format!("Suggested for {}", label_for(ws, key))), picks: sugg, cols: APP_COLS, cell: APP_CELL });
            (top, vec![Section { label: Some("All apps".into()), picks: (0..apps.len()).map(Pick::App).collect(), cols: APP_COLS, cell: APP_CELL }])
        }
        Mode::Apps => {
            let found: Vec<Pick> = crate::custom_icon::search_apps(&apps, q).into_iter().map(Pick::App).collect();
            let em = emoji(search(q));
            let mut body = Vec::new();
            if !found.is_empty() {
                body.push(Section { label: Some("Apps".into()), picks: found, cols: APP_COLS, cell: APP_CELL });
            }
            // A search in Apps also finds emoji.
            if !em.is_empty() {
                body.push(Section { label: Some("Emoji".into()), picks: em, cols: COLS, cell: EMOJI_CELL });
            }
            (None, body)
        }
    }
}

/// One line of the scrolling list: a label, or a row of picks (flat index of its first).
enum Line {
    Label(String),
    Row { first: usize, n: usize, cols: usize, cell: f32 },
}

impl Line {
    fn height(&self) -> f32 {
        match self {
            Line::Label(_) => LABEL_H,
            Line::Row { cell, .. } => *cell,
        }
    }
}

fn lines(body: &[Section], base: usize) -> Vec<Line> {
    let mut out = Vec::new();
    let mut at = base;
    for s in body {
        if let Some(l) = &s.label {
            out.push(Line::Label(l.clone()));
        }
        for chunk in s.picks.chunks(s.cols.max(1)) {
            out.push(Line::Row { first: at, n: chunk.len(), cols: s.cols, cell: s.cell });
            at += chunk.len();
        }
    }
    out
}

/// Rows of flat indexes, top to bottom, for ↑ ↓.
fn nav_rows(top: &Option<Section>, body: &[Section]) -> Vec<Vec<usize>> {
    let mut rows = Vec::new();
    let mut at = 0;
    for s in top.iter().chain(body.iter()) {
        for chunk in s.picks.chunks(s.cols.max(1)) {
            rows.push((at..at + chunk.len()).collect());
            at += chunk.len();
        }
    }
    rows
}

/// The scroll position after the list changes, so the line holding `selected` is in view.
fn reveal(lines: &[Line], scroll: usize, selected: usize) -> usize {
    let Some(sel) = lines.iter().position(|l| matches!(l, Line::Row { first, n, .. } if (*first..first + n).contains(&selected))) else {
        return scroll.min(lines.len().saturating_sub(1));
    };
    let mut first = scroll.min(sel);
    // A section's first row brings its label along.
    if first == sel && sel > 0 && matches!(lines[sel - 1], Line::Label(_)) {
        first = sel - 1;
    }
    while first < sel && lines[first..=sel].iter().map(Line::height).sum::<f32>() > LIST_H {
        first += 1;
    }
    first
}

/// The last useful scroll position: the end of the list fills the view.
fn max_scroll(lines: &[Line]) -> usize {
    let mut h = 0.;
    for (i, l) in lines.iter().enumerate().rev() {
        h += l.height();
        if h > LIST_H {
            return i + 1;
        }
    }
    0
}

fn pick_at(top: &Option<Section>, body: &[Section], i: usize) -> Option<Pick> {
    top.iter().chain(body.iter()).flat_map(|s| s.picks.iter()).nth(i).cloned()
}

impl Workspace {
    pub fn open_icon_picker(&mut self, key: IconFor, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_suggestions_for(&key, cx);
        crate::custom_icon::ensure_apps(cx);
        self.sweep_icon_images();
        self.icon_ui = PickerUi::default();
        self.open_overlay(Overlay::IconPicker { key, at, query: String::new(), selected: 0, cat: 0 }, window, cx);
    }

    /// T1K_DEBUG_OVERLAY=icons|icons-search|tabicons|icons-apps|icons-image, for screenshots:
    /// everything is ready on the first frame (a window behind others doesn't repaint).
    pub fn debug_icon_picker(&mut self, which: &str, cx: &mut Context<Self>) -> Option<Overlay> {
        let t = self.tabs.get(self.active)?;
        let key = if which == "tabicons" || std::env::var_os("T1K_DEBUG_ICON_TAB").is_some() { IconFor::Tab(t.id.clone()) } else { IconFor::Group(t.group.clone()) };
        self.ensure_suggestions_for(&key, cx);
        self.icon_ui = PickerUi::default();
        self.icon_ui.mode = match which {
            "icons-apps" => Mode::Apps,
            "icons-image" => Mode::Image,
            _ => Mode::Emoji,
        };
        if self.icon_ui.mode != Mode::Image {
            crate::custom_icon::load_apps_now();
        }
        if let Some(p) = std::env::var_os("T1K_DEBUG_ICON_IMAGE").filter(|_| self.icon_ui.mode == Mode::Image) {
            self.icon_ui.crop = crate::icon_crop::Crop::load(std::path::Path::new(&p));
            if let (Some(c), Some(z)) = (&mut self.icon_ui.crop, std::env::var("T1K_DEBUG_ICON_ZOOM").ok().and_then(|z| z.parse().ok())) {
                c.set_zoom(z);
            }
        }
        let query = match (which, std::env::var("T1K_DEBUG_ICON_QUERY")) {
            (_, Ok(q)) => q,
            ("icons-search", _) => "rocket".to_string(),
            _ => String::new(),
        };
        Some(Overlay::IconPicker { key, at: gpui::point(px(60.), px(130.)), query, selected: 0, cat: 0 })
    }

    pub fn set_group_icon(&mut self, key: &Path, icon: Option<Icon>, cx: &mut Context<Self>) {
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

    pub fn set_icon(&mut self, key: &IconFor, icon: Option<Icon>, cx: &mut Context<Self>) {
        let old = self.icon_of(key);
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
        // The replaced image's file goes once nothing shows it (an undoable closed tab counts).
        if let Some(Icon::Image(sha)) = old {
            if !self.icon_images_in_use().contains(&sha) {
                crate::custom_icon::remove_image(&sha);
            }
        }
    }

    fn icon_images_in_use(&self) -> std::collections::HashSet<String> {
        let tabs = self.tabs.iter().filter_map(|t| t.icon.as_ref());
        let closed = self.closed.iter().filter_map(|c| c.icon.as_ref());
        tabs.chain(closed)
            .chain(self.group_icons.values())
            .filter_map(|i| match i {
                Icon::Image(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// Once a run: delete stored images left behind (a crash, tabs closed for good).
    fn sweep_icon_images(&self) {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            crate::custom_icon::sweep_images(&self.icon_images_in_use());
        }
    }

    fn apply_pick(&mut self, key: &IconFor, pick: Pick, cx: &mut Context<Self>) {
        let icon = match pick {
            Pick::Emoji(e) => Icon::Emoji(e),
            Pick::App(i) => {
                let Some(app) = crate::custom_icon::apps().get(i).cloned() else { return };
                let value = app.value();
                // Keep a copy of the icon now, so it survives the app being uninstalled.
                let _ = crate::custom_icon::app_png(&value);
                Icon::App(value)
            }
        };
        self.set_icon(key, Some(icon), cx);
    }

    pub fn icon_of(&self, key: &IconFor) -> Option<Icon> {
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

    fn set_picker_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.icon_ui.mode != mode {
            self.icon_ui.mode = mode;
            self.icon_ui.scroll = 0;
            self.icon_ui.error = None;
            if let Overlay::IconPicker { selected, .. } = &mut self.overlay {
                *selected = 0;
            }
        }
        cx.notify();
    }

    fn scroll_picker(&mut self, dy: f32, cx: &mut Context<Self>) {
        let Overlay::IconPicker { key, query, cat, .. } = &self.overlay else { return };
        let (top, body) = items(self, key, query, *cat);
        let base = top.as_ref().map_or(0, |s| s.picks.len());
        let ls = lines(&body, base);
        self.icon_ui.scroll_acc -= dy / EMOJI_CELL;
        let n = self.icon_ui.scroll_acc.trunc() as i32;
        if n != 0 {
            self.icon_ui.scroll_acc -= n as f32;
            self.icon_ui.scroll = (self.icon_ui.scroll as i32 + n).clamp(0, max_scroll(&ls) as i32) as usize;
            cx.notify();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn icon_picker_key(&mut self, key: IconFor, at: Point<Pixels>, mut query: String, mut selected: usize, cat: usize, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        let m = &ev.keystroke.modifiers;
        // Tab / ⇧Tab step through Emoji · Apps · Image.
        if ev.keystroke.key == "tab" {
            let i = MODES.iter().position(|(md, _)| *md == self.icon_ui.mode).unwrap_or(0);
            let next = MODES[(i + if m.shift { 2 } else { 1 }) % 3].0;
            self.set_picker_mode(next, cx);
            return Some(Overlay::IconPicker { key, at, query, selected: 0, cat });
        }
        if self.icon_ui.mode == Mode::Image {
            return self.crop_key(key, at, query, selected, cat, ev, window, cx);
        }
        // The search field edits like any text field: ⌘A selects it all (typing or ⌫ replaces
        // it), ⌘V pastes, ⌘⌫ clears it. Removing the icon is the button's job.
        if m.platform && ev.keystroke.key == "a" {
            self.icon_query_all = !query.is_empty();
            cx.notify();
            return Some(Overlay::IconPicker { key, at, query, selected, cat });
        }
        if m.platform && ev.keystroke.key == "v" {
            if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                if std::mem::take(&mut self.icon_query_all) {
                    query.clear();
                }
                query.push_str(t.lines().next().unwrap_or_default());
            }
            self.icon_ui.scroll = 0;
            return Some(Overlay::IconPicker { key, at, query, selected: 0, cat });
        }
        let editing = !matches!(ev.keystroke.key.as_str(), "left" | "right" | "up" | "down" | "enter" | "escape") && !m.platform || ev.keystroke.key == "backspace";
        if editing && std::mem::take(&mut self.icon_query_all) {
            // Replace the selection: a backspace just clears it, a character starts over.
            query.clear();
            if ev.keystroke.key == "backspace" {
                self.icon_ui.scroll = 0;
                return Some(Overlay::IconPicker { key, at, query, selected: 0, cat });
            }
        }
        self.icon_query_all = false;
        let (top, body) = items(self, &key, &query, cat);
        let rows = nav_rows(&top, &body);
        let n: usize = rows.iter().map(Vec::len).sum();
        let pos = |i: usize| rows.iter().enumerate().find_map(|(r, row)| row.iter().position(|x| *x == i).map(|c| (r, c)));
        // Arrows: ← → by one, ↑ ↓ by a row (to the same column, or the row's last).
        let step = |r: usize, c: usize| rows.get(r).map(|row| row[c.min(row.len() - 1)]);
        match ev.keystroke.key.as_str() {
            "left" => selected = selected.saturating_sub(1),
            "right" => selected = (selected + 1).min(n.saturating_sub(1)),
            "up" => {
                if let Some((r, c)) = pos(selected).filter(|(r, _)| *r > 0) {
                    selected = step(r - 1, c).unwrap_or(selected);
                }
            }
            "down" => {
                if let Some((r, c)) = pos(selected) {
                    selected = step(r + 1, c).unwrap_or(selected);
                }
            }
            _ => match input::handle(&mut query, ev) {
                Key::Cancel if !query.is_empty() => {
                    query.clear();
                    selected = 0;
                    self.icon_ui.scroll = 0;
                }
                Key::Cancel => return None,
                Key::Submit => {
                    if let Some(p) = pick_at(&top, &body, selected) {
                        self.apply_pick(&key, p, cx);
                    }
                    return None;
                }
                Key::Edited => {
                    selected = 0;
                    self.icon_ui.scroll = 0;
                }
                _ => {}
            },
        }
        // Keep the selection in view (the query may have changed the list).
        let (top, body) = items(self, &key, &query, cat);
        let base = top.as_ref().map_or(0, |s| s.picks.len());
        self.icon_ui.scroll = reveal(&lines(&body, base), self.icon_ui.scroll, selected);
        Some(Overlay::IconPicker { key, at, query, selected, cat })
    }
}

/// Space between cells: app columns spread across the picker's width, as in the canvas.
fn gap(cols: usize) -> f32 {
    if cols == APP_COLS {
        (278. - APP_COLS as f32 * (APP_CELL - 2.)) / (APP_COLS - 1) as f32
    } else {
        2.
    }
}

/// One pick in a grid cell; `on` is the keyboard selection.
fn cell(ws: &Workspace, key: &IconFor, id: String, pick: &Pick, size: f32, on: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let k = key.clone();
    let p = pick.clone();
    let d = div()
        .id(SharedString::from(id))
        .size(px(size - 2.))
        .flex_none()
        .rounded(px(7.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(on, |d| d.bg(ui.bg_selected).border_1().border_color(ui.accent))
        .when(!on, |d| d.hover(move |d| d.bg(ui.bg_hover)))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.apply_pick(&k, p.clone(), cx);
            this.close_overlay(window, cx);
        }));
    match pick {
        Pick::Emoji(e) => d.text_size(px(19.)).child(e.clone()).into_any_element(),
        Pick::App(i) => {
            let apps = crate::custom_icon::apps();
            let Some(app) = apps.get(*i) else { return d.into_any_element() };
            let icon = if size >= APP_CELL { crate::custom_icon::GRID_ICON } else { 24. };
            let png = app.grid_png(crate::custom_icon::pixels(crate::custom_icon::GRID_ICON)).filter(|p| p.is_file());
            // Until its icon is made (in the background), a quiet tile with its first letter.
            let art = match png {
                Some(p) => gpui::img(p).size(px(icon)).into_any_element(),
                None => div()
                    .size(px(icon))
                    .rounded(px((icon * 0.24).round()))
                    .bg(ui.bg_control)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(ui.text_tertiary)
                    .child(app.name.chars().next().unwrap_or('?').to_string())
                    .into_any_element(),
            };
            d.tooltip(crate::tooltip::text(ui, app.name.clone())).child(art).into_any_element()
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(ws: &Workspace, key: &IconFor, at: Point<Pixels>, query: &str, selected: usize, cat: usize, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let mode = ws.icon_ui.mode;
    let (top, body_sections) = items(ws, key, query, cat);
    let current = ws.icon_of(key);
    let label = |t: String| div().px(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(t);

    // Emoji · Apps · Image, full width.
    let seg = div().h(px(24.)).p(px(2.)).rounded(px(7.)).bg(ui.bg_sidebar).border_1().border_color(ui.border).flex().text_size(px(12.)).children(MODES.iter().map(|(m, name)| {
        let (m, on) = (*m, *m == mode);
        div()
            .id(SharedString::from(format!("icon-mode-{name}")))
            .flex_1()
            .rounded(px(5.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(if on { ui.text_primary } else { ui.text_tertiary })
            .when(on, |d| d.bg(ui.bg_selected))
            .when(!on, |d| d.hover(move |d| d.text_color(ui.text_secondary)))
            .on_click(cx.listener(move |this, _, _, cx| this.set_picker_mode(m, cx)))
            .child(*name)
    }));
    let mut body = div().flex().flex_col().gap(px(8.)).child(seg);

    if mode == Mode::Image {
        body = body.child(crate::icon_crop::render(ws, key, cx));
    } else {
        // Search field: typing anywhere in the picker goes here, so it always looks focused
        // (accent border, caret) and takes clicks without closing the picker.
        let caret = div().w(px(1.5)).h(px(15.)).flex_none().bg(ui.accent);
        let placeholder = if mode == Mode::Apps { "Search apps" } else { "Search emoji" };
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
                    div().flex().items_center().child(caret).child(div().ml(px(1.)).text_color(ui.text_tertiary).child(placeholder))
                } else {
                    let all = ws.icon_query_all;
                    div()
                        .flex()
                        .items_center()
                        .text_color(ui.text_primary)
                        .child(div().when(all, |d| d.bg(ui.accent.opacity(0.3)).rounded(px(2.))).child(query.to_string()))
                        .when(!all, |d| d.child(caret))
                }),
        );
        if let Some(s) = &top {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(s.label.clone().map(label))
                    .child(div().flex().gap(px(gap(s.cols))).children(s.picks.iter().enumerate().map(|(i, p)| cell(ws, key, format!("sg-{i}"), p, s.cell, selected == i, cx)))),
            );
        }
        if mode == Mode::Emoji && query.trim().is_empty() {
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
                            let (top, _) = items(this, &k, &q, i);
                            let n = top.map_or(0, |s| s.picks.len());
                            this.icon_ui.scroll = 0;
                            this.overlay = Overlay::IconPicker { key: k.clone(), at, query: q, selected: n, cat: i };
                            cx.notify();
                        }
                    }))
                    .child(category_icon(i))
            })));
        }
        // The list: from the scroll position, as many lines as fit.
        let base = top.as_ref().map_or(0, |s| s.picks.len());
        let ls = lines(&body_sections, base);
        let all: Vec<Pick> = body_sections.iter().flat_map(|s| s.picks.iter().cloned()).collect();
        let mut list = div().flex().flex_col();
        let mut used = 0.;
        for l in ls.iter().skip(ws.icon_ui.scroll.min(ls.len().saturating_sub(1))) {
            if used + l.height() > LIST_H + 0.5 {
                break;
            }
            used += l.height();
            list = list.child(match l {
                Line::Label(t) => div().h(px(LABEL_H)).flex().items_end().pb(px(4.)).child(label(t.clone())),
                Line::Row { first, n, cols, cell: size } => div().h(px(*size)).flex().gap(px(gap(*cols))).children((*first..first + n).map(|i| {
                    cell(ws, key, format!("g-{i}"), &all[i - base], *size, selected == i, cx)
                })),
            });
        }
        let empty = all.is_empty().then(|| {
            let text = match mode {
                Mode::Apps if !crate::custom_icon::apps_loaded() => "Finding apps…",
                Mode::Apps => "No apps match",
                _ => "No emoji match",
            };
            div().p(px(8.)).text_size(px(12.5)).text_color(ui.text_tertiary).child(text)
        });
        body = body.child(
            div()
                .id("icon-list")
                .h(px(LIST_H))
                .overflow_hidden()
                .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, _, cx| {
                    let dy = match ev.delta {
                        gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
                        gpui::ScrollDelta::Lines(l) => l.y * EMOJI_CELL,
                    };
                    this.scroll_picker(dy, cx);
                }))
                .children(empty)
                .child(list),
        );
    }
    if current.is_some() && ws.icon_ui.crop.is_none() {
        let k = key.clone();
        body = body.child(
            div().pt(px(6.)).border_t_1().border_color(ui.border).child(
                div()
                    .id("icon-remove")
                    .h(px(28.))
                    .px(px(6.))
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
                    .child(div().flex_1().child("Remove icon")),
            ),
        );
    }
    let vh = f32::from(window.viewport_size().height);
    let top_y = f32::from(at.y).min(vh - 470.).max(8.);
    div()
        .id("icon-picker")
        .absolute()
        .left(at.x)
        .top(px(top_y))
        .w(px(300.))
        .p(px(10.))
        .rounded(px(12.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // An image dropped anywhere on the picker opens the cropper.
        .drag_over::<gpui::ExternalPaths>(move |s, paths: &gpui::ExternalPaths, _, _| {
            if paths.paths().iter().any(|p| crate::custom_icon::is_icon_image(p)) { s.border_color(ui.accent) } else { s }
        })
        .on_drop(cx.listener(move |this, paths: &gpui::ExternalPaths, _, cx| {
            if let Some(p) = paths.paths().iter().find(|p| crate::custom_icon::is_icon_image(p)) {
                this.start_crop(p.clone(), cx);
            }
        }))
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
