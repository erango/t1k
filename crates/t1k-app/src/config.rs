//! `~/.config/t1k/config.toml`: the source of truth for settings. Reloaded when it changes.
//! The menu and (later) the Settings window edit this file in place, keeping comments.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::Deserialize;

const TEMPLATE: &str = r#"# T1K settings. Changes apply as soon as you save.

# "system" follows macOS; "dark" or "light" pins one.
appearance = "system"

[theme]
# Any built-in theme, or one imported with `t1k theme import <file>`
# (or dropped onto the window). Imported themes live in ~/.config/t1k/themes/.
dark = "Mercury Dark"
light = "Mercury Light"

[window]
# How much of the desktop shows through: 1.0 is solid, 0.7 the most see-through. The
# sidebar stays a little more solid so it reads; menus and popovers stay solid.
background_opacity = 1.0
# Blur what's behind the window while it's see-through.
background_blur = true
# Let colors that programs paint behind text show through too (off keeps them solid).
background_opacity_cells = false

[terminal]
# Space around the text: this many px above and below, about 1.4× that at the sides.
padding = 14
option_as_meta = false
# Copy text to the clipboard as soon as you finish selecting it.
copy_on_select = true
# zsh: mark each prompt so Cmd-Up / Cmd-Down jump between commands. Applies to new tabs.
shell_integration = true
# Let programs turn on the kitty keyboard protocol (shift+enter, ctrl+i vs tab, key releases).
# Applies to new tabs.
kitty_keyboard = true
# Secure Keyboard Entry: while a T1K window is in front, other apps can't read what you type.
secure_input = false
# Turn it on by itself while the terminal you're typing in asks for a password (sudo, ssh),
# and off again after.
secure_input_passwords = true
# Where a new tab's first prompt appears: "bottom" (input stays where your eyes are) or "top".
prompt_gravity = "bottom"
# Keep each tab's scrollback (text and colors) when T1K quits or restarts for an update.
# Saved in ~/Library/Application Support/t1k/scrollback, readable only by you.
restore_scrollback = true
# Claude Code sessions still running when T1K quits resume in their tabs on the next launch.
resume_agents = true
# At launch, offer to bring over tabs open in other terminals (only when new ones turn up).
offer_import = true

[terminal.font]
family = "JetBrains Mono"
size = 13
line_height = 1.45
# Tried in order for glyphs the main font lacks (Powerline / Nerd Font icons, emoji).
fallbacks = ["Symbols Nerd Font Mono", "MesloLGS NF", "Meslo LG L DZ for Powerline", "Menlo", "Apple Color Emoji"]

[interface]
# Sidebar rows and program icons: "compact" (28 px rows, 16 px icons), "comfortable"
# (34 / 22) or "spacious" (40 / 28). With a second line, icons span both lines.
density = "comfortable"
# Text size of the sidebar and panels (the terminal font has its own size).
text_size = 13
# Dock icon: "face" (the android), "prompt" (>_) or "wordmark" (T1K). Each turns red
# while a tab needs you. Finder keeps the face.
app_icon = "face"
# Draws the group's color around the selected tab and the terminal it opens.
selected_outline = true
# Outputs panel: items shown per section before "+N more" (open PRs always show).
outputs_section_limit = 5

[sidebar]
width = 300
# How long the pointer rests on a tab before its detail card appears.
hover_intent_ms = 300
# A small badge for what's running in each tab (Claude, shell, vim, ssh...).
app_icons = true
# What a tab's title shows (a manual rename always wins):
# "auto" (Claude's title, else the running command, else the folder), "command", "folder", "branch".
title = "auto"
# An always-on second line: "none", "status" (what it's doing / recap), "branch", "folder".
second_line = "none"
# Show a detail card when the pointer rests on a tab.
hover_card = true
# What ⌘\\ and dragging the edge to the window's side fold the sidebar to: "rail" (a slim
# strip of tab icons that still shows what needs you) or "hidden".
collapsed = "rail"
# Rest the pointer on the rail this long to see the full sidebar over the terminal. 0: never.
peek_ms = 400
# Group colors (the line down a group's tabs and along the top of the title bar):
# "auto" (a pastel rainbow, in order down the sidebar) or "manual" (only colors you pick).
group_colors = "auto"
# How a Claude session's /color shows on its tab: "swatch" (a small bar before the title),
# "corner" (a triangle on the icon), "tint" (the row), "line" (the group line beside it), "off".
session_color = "swatch"

[tabs]
# Where ⌘T and + put a new tab in its group: "after" the current tab, "before" it, or at the
# "end" of the group. (Only for groups sorted by order opened: the default, last updated, puts
# a new tab at the top, and needs attention lifts tabs that need you.)
new_tab = "end"
# Which tab you land on after closing one: the one "below" it, or "above" it in the sidebar.
after_close = "below"
# Stay in the closed tab's group while it has tabs left (else the nearest tab in any group).
close_stays_in_group = false

[links]
# How links in the terminal open: false: ⌘-click (a plain click selects text, as in most
# terminals); true: a plain click (⌘-click still works, and dragging still selects).
click = false
# ⌘-click a file path to open it. Empty picks the first of Zed, Cursor, VS Code, else the
# default app. Otherwise a command, with {path}, {line} and {col}: "code -g {path}:{line}:{col}".
editor = ""
# Text files (code, Markdown, logs...): "panel" opens them in T1K's side panel (Markdown
# rendered), "editor" in the editor above. Other files open in their default app.
text_files = "panel"
# Where ticket keys like ENG-42 open, with {key}: "https://acme.atlassian.net/browse/{key}".
# Empty learns it from ticket links your Claude sessions mention.
tickets = ""

[notify]
# Banners when a tab needs you while you're elsewhere.
needs_input = true
error = true
reminder = true
done = false
# Number on the Dock icon: tabs that need you (input, errors, due reminders).
dock_badge = true
# Banners programs ask for (OSC 9 / 777), e.g. a long build saying it's done.
programs = true

[quick]
# Quick terminal: a panel over whatever app you're in, on a shortcut that works everywhere.
enabled = false
# The shortcut, e.g. "ctrl-alt-t" (modifiers: ctrl, alt, shift, cmd; then one key).
shortcut = "ctrl-alt-t"
# What it shows: "needs" (the tab that has waited longest for you; a quick shell when none
# does) or "shell" (always the quick shell).
shows = "needs"
# Where: "panel" (centered under the menu bar), or an edge: "top", "bottom", "left", "right".
position = "panel"
# "s", "m" or "l".
size = "m"
# Which screen: "pointer" (the one the pointer is on) or "main".
screen = "pointer"
# Hide it when you click another app.
hide_on_blur = true

[updates]
# Look for a new version once a day, in the background. Nothing interrupts you.
check = true
# Download it too, so it's ready; you still choose when to restart.
download = true
# "stable", or "beta" for new features first, with rougher edges.
channel = "stable"
# Where to look for updates. Empty uses T1K's own feed.
feed = ""

[privacy]
# Send crash reports and errors to T1K's developers (Sentry). Never terminal contents;
# paths are shortened to ~ and email addresses removed. Applies at the next launch.
crash_reports = true

[helper]
# Optional: a cheap model, run through an agent CLI you already use, writes what the agents
# don't: titles for untitled sessions, a one-line recap after each turn, and ⌘↵ in the tab
# switcher to find a tab by meaning. "off", "claude", "codex", "agy", "gemini" or "grok".
# Recaps send the end of that tab's screen to the CLI's provider. `t1k helper bench` compares
# speed and cost; `t1k helper stats` shows what it used.
backend = "off"
# Empty: the CLI's small model (haiku, gemini-3.8-flash-low...).
model = ""
titles = true
recaps = true
search = true

[keys]
# Shortcut overrides, by action (Settings › Keybindings writes these). Several shortcuts are
# separated by commas; "" turns one off. For example:
# find_tab = "cmd-p, cmd-e"
# overview = ""
"#;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Config {
    pub appearance: String,
    pub theme: ThemeCfg,
    pub terminal: TerminalCfg,
    pub interface: InterfaceCfg,
    pub sidebar: SidebarCfg,
    pub tabs: TabsCfg,
    pub links: LinksCfg,
    pub notify: NotifyCfg,
    pub updates: UpdatesCfg,
    pub quick: QuickCfg,
    pub privacy: PrivacyCfg,
    pub helper: HelperCfg,
    pub window: WindowCfg,
    /// Shortcut overrides by `keymap::Bind::id`.
    pub keys: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct HelperCfg {
    pub backend: String,
    pub model: String,
    pub titles: bool,
    pub recaps: bool,
    pub search: bool,
}

impl Default for HelperCfg {
    fn default() -> Self {
        Self { backend: "off".into(), model: String::new(), titles: true, recaps: true, search: true }
    }
}

impl HelperCfg {
    pub fn backend(&self) -> Option<t1k_agent::helper::Backend> {
        t1k_agent::helper::Backend::parse(&self.backend)
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct PrivacyCfg {
    pub crash_reports: bool,
}

impl Default for PrivacyCfg {
    fn default() -> Self {
        Self { crash_reports: true }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct UpdatesCfg {
    pub check: bool,
    pub download: bool,
    pub channel: String,
    pub feed: String,
}

impl Default for UpdatesCfg {
    fn default() -> Self {
        Self { check: true, download: true, channel: "stable".into(), feed: String::new() }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct QuickCfg {
    pub enabled: bool,
    pub shortcut: String,
    pub shows: String,
    pub position: String,
    pub size: String,
    pub screen: String,
    pub hide_on_blur: bool,
}

impl Default for QuickCfg {
    fn default() -> Self {
        Self { enabled: false, shortcut: "ctrl-alt-t".into(), shows: "needs".into(), position: "panel".into(), size: "m".into(), screen: "pointer".into(), hide_on_blur: true }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct InterfaceCfg {
    pub density: String,
    pub text_size: f32,
    pub app_icon: String,
    pub selected_outline: bool,
    pub outputs_section_limit: usize,
}

impl Default for InterfaceCfg {
    fn default() -> Self {
        Self { density: "comfortable".into(), text_size: 13., app_icon: "face".into(), selected_outline: true, outputs_section_limit: 5 }
    }
}

impl Default for LinksCfg {
    fn default() -> Self {
        Self { click: false, editor: String::new(), tickets: String::new(), text_files: "panel".into() }
    }
}

impl InterfaceCfg {
    /// Sidebar row height for the density (one line).
    pub fn row_height(&self) -> f32 {
        match self.density.as_str() {
            "compact" => 28.,
            "spacious" => 40.,
            _ => 34.,
        }
    }

    /// Program icon size: follows the row, and spans both lines when there's a second line.
    pub fn icon_size(&self, two_lines: bool) -> f32 {
        match (self.density.as_str(), two_lines) {
            ("compact", false) => 16.,
            ("spacious", false) => 28.,
            (_, false) => 22.,
            ("compact", true) => 28.,
            ("spacious", true) => 36.,
            (_, true) => 32.,
        }
    }

    /// Row height with an always-on second line (design DETAIL heights).
    pub fn two_line_height(&self) -> f32 {
        match self.density.as_str() {
            "compact" => 40.,
            "spacious" => 54.,
            _ => 46.,
        }
    }

    /// Scale for UI text, relative to the 13px design size.
    pub fn text_scale(&self) -> f32 {
        (self.text_size.clamp(10., 18.)) / 13.
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct LinksCfg {
    /// A plain click opens links; false: ⌘-click (a plain click selects text).
    pub click: bool,
    pub editor: String,
    pub tickets: String,
    pub text_files: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct NotifyCfg {
    pub needs_input: bool,
    pub error: bool,
    pub reminder: bool,
    pub done: bool,
    pub dock_badge: bool,
    /// Programs' own notifications (OSC 9 / OSC 777).
    pub programs: bool,
}

impl Default for NotifyCfg {
    fn default() -> Self {
        Self { needs_input: true, error: true, reminder: true, done: false, dock_badge: true, programs: true }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ThemeCfg {
    pub dark: String,
    pub light: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct TerminalCfg {
    pub padding: f32,
    pub option_as_meta: bool,
    pub copy_on_select: bool,
    pub restore_scrollback: bool,
    pub resume_agents: bool,
    pub offer_import: bool,
    pub shell_integration: bool,
    pub kitty_keyboard: bool,
    pub secure_input: bool,
    pub secure_input_passwords: bool,
    pub prompt_gravity: String,
    pub font: FontCfg,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct FontCfg {
    pub family: String,
    pub size: f32,
    pub line_height: f32,
    pub fallbacks: Vec<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct SidebarCfg {
    pub width: f32,
    pub hover_intent_ms: u64,
    pub app_icons: bool,
    pub title: String,
    pub second_line: String,
    pub hover_card: bool,
    pub collapsed: String,
    pub peek_ms: u64,
    pub group_colors: String,
    pub session_color: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: "system".into(),
            theme: ThemeCfg::default(),
            terminal: TerminalCfg::default(),
            interface: InterfaceCfg::default(),
            sidebar: SidebarCfg::default(),
            tabs: TabsCfg::default(),
            links: LinksCfg::default(),
            notify: NotifyCfg::default(),
            updates: UpdatesCfg::default(),
            quick: QuickCfg::default(),
            privacy: PrivacyCfg::default(),
            keys: Default::default(),
            helper: HelperCfg::default(),
            window: WindowCfg::default(),
        }
    }
}
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct WindowCfg {
    pub background_opacity: f32,
    pub background_blur: bool,
    pub background_opacity_cells: bool,
}

impl Default for WindowCfg {
    fn default() -> Self {
        Self { background_opacity: 1.0, background_blur: true, background_opacity_cells: false }
    }
}

impl WindowCfg {
    /// The terminal's background alpha, kept in the range where text stays readable.
    pub fn alpha(&self) -> f32 {
        self.background_opacity.clamp(0.7, 1.0)
    }
}

impl Default for ThemeCfg {
    fn default() -> Self {
        Self { dark: "Mercury Dark".into(), light: "Mercury Light".into() }
    }
}
impl Default for TerminalCfg {
    fn default() -> Self {
        Self { padding: 14., option_as_meta: false, copy_on_select: true, restore_scrollback: true, resume_agents: true, offer_import: true, shell_integration: true, kitty_keyboard: true, secure_input: false, secure_input_passwords: true, prompt_gravity: "bottom".into(), font: FontCfg::default() }
    }
}
impl Default for FontCfg {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".into(),
            size: 13.,
            line_height: 1.45,
            fallbacks: ["Symbols Nerd Font Mono", "MesloLGS NF", "Meslo LG L DZ for Powerline", "Menlo", "Apple Color Emoji"]
                .map(String::from)
                .to_vec(),
        }
    }
}
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct TabsCfg {
    /// "after", "before" or "end".
    pub new_tab: String,
    /// "below" or "above".
    pub after_close: String,
    pub close_stays_in_group: bool,
}

impl Default for TabsCfg {
    fn default() -> Self {
        Self { new_tab: "end".into(), after_close: "below".into(), close_stays_in_group: false }
    }
}

impl Default for SidebarCfg {
    fn default() -> Self {
        Self {
            width: 300.,
            hover_intent_ms: 300,
            app_icons: true,
            title: "auto".into(),
            second_line: "none".into(),
            hover_card: true,
            collapsed: "rail".into(),
            peek_ms: 400,
            group_colors: "auto".into(),
            session_color: "swatch".into(),
        }
    }
}

pub fn dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/t1k"))
}

pub fn path() -> Option<PathBuf> {
    Some(dir()?.join("config.toml"))
}

pub fn themes_dir() -> Option<PathBuf> {
    Some(dir()?.join("themes"))
}

/// Watches the config file by mtime.
pub struct Watcher {
    mtime: Option<SystemTime>,
}

impl Watcher {
    pub fn new() -> Self {
        Self { mtime: None }
    }

    /// Returns the config if the file is new or changed since the last call.
    /// Creates the file from the template on first run. Parse errors keep the old config.
    pub fn poll(&mut self) -> Option<Result<Config, String>> {
        let p = path()?;
        if !p.exists() {
            let _ = std::fs::create_dir_all(p.parent()?);
            let _ = std::fs::write(&p, TEMPLATE);
        }
        let m = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        if m == self.mtime {
            return None;
        }
        self.mtime = m;
        let text = std::fs::read_to_string(&p).ok()?;
        Some(toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display())))
    }
}

/// Set a string key in config.toml, keeping comments and layout. `keys` is a path like ["theme", "dark"].
/// The config file as it is now (defaults if missing or invalid).
pub fn load() -> Config {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn set(keys: &[&str], value: &str) {
    set_value(keys, value.into());
}

/// Set any value in config.toml, keeping comments and layout.
pub fn set_value(keys: &[&str], value: toml_edit::Value) {
    let Some(p) = path() else { return };
    let text = std::fs::read_to_string(&p).unwrap_or_else(|_| TEMPLATE.into());
    if let Some(out) = edit(&text, keys, value) {
        let _ = std::fs::write(&p, out);
    }
}

/// A key's value as written in config.toml (None when it isn't there).
pub fn get_raw(keys: &[&str]) -> Option<toml_edit::Value> {
    get_raw_in(&std::fs::read_to_string(path()?).ok()?, keys)
}

/// A key's value as written in `text` (None when it isn't there).
pub fn get_raw_in(text: &str, keys: &[&str]) -> Option<toml_edit::Value> {
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let mut item = doc.as_item();
    for k in keys {
        item = item.get(k)?;
    }
    item.as_value().cloned()
}

/// Remove a key from config.toml (it falls back to its default).
pub fn remove(keys: &[&str]) {
    let Some(p) = path() else { return };
    let Ok(text) = std::fs::read_to_string(&p) else { return };
    if let Some(out) = remove_in(&text, keys) {
        let _ = std::fs::write(&p, out);
    }
}

/// `text` without `keys`; None when it wasn't there (or `text` isn't TOML).
pub fn remove_in(text: &str, keys: &[&str]) -> Option<String> {
    let mut doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let (last, parents) = keys.split_last()?;
    let mut table = doc.as_table_mut();
    for k in parents {
        table = table.get_mut(k).and_then(|i| i.as_table_mut())?;
    }
    table.remove(last)?;
    Some(doc.to_string())
}

/// The file as first written, with every default.
pub fn template() -> &'static str {
    TEMPLATE
}

/// Return `text` with `keys` set to `value`, creating tables as needed. None if `text` isn't TOML.
pub fn edit(text: &str, keys: &[&str], value: toml_edit::Value) -> Option<String> {
    let mut doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let (last, parents) = keys.split_last()?;
    let mut table = doc.as_table_mut();
    for k in parents {
        let entry = table.entry(k).or_insert(toml_edit::table());
        table = entry.as_table_mut()?;
    }
    // Keep the existing value's trailing comment.
    let decor = table.get(last).and_then(|i| i.as_value()).map(|v| v.decor().clone());
    let mut value = value;
    if let Some(d) = decor {
        *value.decor_mut() = d;
    }
    table[*last] = toml_edit::Item::Value(value);
    Some(doc.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_keeps_comments() {
        let out = super::edit(super::TEMPLATE, &["terminal", "font", "size"], 15.into()).unwrap();
        assert!(out.contains("size = 15"));
        assert!(out.contains("# Tried in order for glyphs"));
        let out = super::edit(&out, &["notify", "done"], true.into()).unwrap();
        let c: super::Config = toml::from_str(&out).unwrap();
        assert!(c.notify.done);
        assert_eq!(c.terminal.font.size, 15.);
        let out = super::edit("", &["theme", "dark"], "Dracula".into()).unwrap();
        assert!(out.contains("[theme]") && out.contains("dark = \"Dracula\""));
    }

    #[test]
    fn template_is_complete() {
        // The commented template written on first run must say exactly the defaults.
        let c: super::Config = toml::from_str(super::TEMPLATE).unwrap();
        assert_eq!(c, super::Config::default());
        // Partial files fill the rest from defaults.
        let p: super::Config = toml::from_str("appearance = \"dark\"\n[terminal.font]\nsize = 15\n").unwrap();
        assert_eq!(p.terminal.font.size, 15.);
        assert_eq!(p.terminal.font.family, "JetBrains Mono");
        assert_eq!(p.sidebar.width, 300.);
    }
}
