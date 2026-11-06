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

[terminal]
# Space around the text: this many px above and below, about 1.4× that at the sides.
padding = 14
option_as_meta = false
# Copy text to the clipboard as soon as you finish selecting it.
copy_on_select = false
# zsh: mark each prompt so Cmd-Up / Cmd-Down jump between commands. Applies to new tabs.
shell_integration = true

[terminal.font]
family = "JetBrains Mono"
size = 13
line_height = 1.45
# Tried in order for glyphs the main font lacks (Powerline / Nerd Font icons, emoji).
fallbacks = ["Symbols Nerd Font Mono", "MesloLGS NF", "Meslo LG L DZ for Powerline", "Menlo", "Apple Color Emoji"]

[interface]
# Row spacing in the sidebar: "compact", "comfortable" or "spacious".
density = "comfortable"
# Text size of the sidebar and panels (the terminal font has its own size).
text_size = 13

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

[links]
# ⌘-click a file path to open it. Empty picks the first of Zed, Cursor, VS Code, else the
# default app. Otherwise a command, with {path}, {line} and {col}: "code -g {path}:{line}:{col}".
editor = ""
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
"#;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Config {
    pub appearance: String,
    pub theme: ThemeCfg,
    pub terminal: TerminalCfg,
    pub interface: InterfaceCfg,
    pub sidebar: SidebarCfg,
    pub links: LinksCfg,
    pub notify: NotifyCfg,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct InterfaceCfg {
    pub density: String,
    pub text_size: f32,
}

impl Default for InterfaceCfg {
    fn default() -> Self {
        Self { density: "comfortable".into(), text_size: 13. }
    }
}

impl InterfaceCfg {
    /// Sidebar row height for the density.
    pub fn row_height(&self) -> f32 {
        match self.density.as_str() {
            "compact" => 28.,
            "spacious" => 36.,
            _ => 32.,
        }
    }

    /// Row height with an always-on second line (design DETAIL heights).
    pub fn two_line_height(&self) -> f32 {
        match self.density.as_str() {
            "compact" => 44.,
            "spacious" => 60.,
            _ => 52.,
        }
    }

    /// Scale for UI text, relative to the 13px design size.
    pub fn text_scale(&self) -> f32 {
        (self.text_size.clamp(10., 18.)) / 13.
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct LinksCfg {
    pub editor: String,
    pub tickets: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct NotifyCfg {
    pub needs_input: bool,
    pub error: bool,
    pub reminder: bool,
    pub done: bool,
    pub dock_badge: bool,
}

impl Default for NotifyCfg {
    fn default() -> Self {
        Self { needs_input: true, error: true, reminder: true, done: false, dock_badge: true }
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
    pub shell_integration: bool,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: "system".into(),
            theme: ThemeCfg::default(),
            terminal: TerminalCfg::default(),
            interface: InterfaceCfg::default(),
            sidebar: SidebarCfg::default(),
            links: LinksCfg::default(),
            notify: NotifyCfg::default(),
        }
    }
}
impl Default for ThemeCfg {
    fn default() -> Self {
        Self { dark: "Mercury Dark".into(), light: "Mercury Light".into() }
    }
}
impl Default for TerminalCfg {
    fn default() -> Self {
        Self { padding: 14., option_as_meta: false, copy_on_select: false, shell_integration: true, font: FontCfg::default() }
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
impl Default for SidebarCfg {
    fn default() -> Self {
        Self {
            width: 300.,
            hover_intent_ms: 300,
            app_icons: true,
            title: "auto".into(),
            second_line: "none".into(),
            hover_card: true,
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
