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
padding = 10
option_as_meta = false

[terminal.font]
family = "JetBrains Mono"
size = 13
line_height = 1.3
# Tried in order for glyphs the main font lacks (Powerline / Nerd Font icons, emoji).
fallbacks = ["Symbols Nerd Font Mono", "MesloLGS NF", "Meslo LG L DZ for Powerline", "Menlo", "Apple Color Emoji"]

[sidebar]
width = 300
# How long the pointer rests on the sidebar before rows expand to two lines.
hover_intent_ms = 150
"#;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Config {
    pub appearance: String,
    pub theme: ThemeCfg,
    pub terminal: TerminalCfg,
    pub sidebar: SidebarCfg,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: "system".into(),
            theme: ThemeCfg::default(),
            terminal: TerminalCfg::default(),
            sidebar: SidebarCfg::default(),
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
        Self { padding: 10., option_as_meta: false, font: FontCfg::default() }
    }
}
impl Default for FontCfg {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".into(),
            size: 13.,
            line_height: 1.3,
            fallbacks: ["Symbols Nerd Font Mono", "MesloLGS NF", "Meslo LG L DZ for Powerline", "Menlo", "Apple Color Emoji"]
                .map(String::from)
                .to_vec(),
        }
    }
}
impl Default for SidebarCfg {
    fn default() -> Self {
        Self { width: 300., hover_intent_ms: 150 }
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

/// Set a key in config.toml, keeping comments and layout. `keys` is a path like ["theme", "dark"].
pub fn set(keys: &[&str], value: &str) {
    let Some(p) = path() else { return };
    let text = std::fs::read_to_string(&p).unwrap_or_else(|_| TEMPLATE.into());
    let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else { return };
    let (last, parents) = keys.split_last().unwrap();
    let mut table = doc.as_table_mut();
    for k in parents {
        let entry = table.entry(k).or_insert(toml_edit::table());
        let Some(t) = entry.as_table_mut() else { return };
        table = t;
    }
    table[*last] = toml_edit::value(value);
    let _ = std::fs::write(&p, doc.to_string());
}

#[cfg(test)]
mod tests {
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
