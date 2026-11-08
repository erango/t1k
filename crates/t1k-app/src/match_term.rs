//! "Match your terminal" (canvas v3-z): read the font, size, line height and colors of the
//! terminal the user already uses, so T1K can look the same with one click.
//!
//! Sources: Warp (`~/.warp/settings.toml`, custom themes in `~/.warp/themes`), iTerm2 (its
//! preferences plist, the default profile), Ghostty, kitty and Alacritty (their config files).
//! Nothing here writes anything; `plan` turns a source into the changes T1K would make.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Warp,
    ITerm2,
    Ghostty,
    Kitty,
    Alacritty,
}

impl App {
    pub fn label(self) -> &'static str {
        match self {
            App::Warp => "Warp",
            App::ITerm2 => "iTerm2",
            App::Ghostty => "Ghostty",
            App::Kitty => "kitty",
            App::Alacritty => "Alacritty",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ThemeSrc {
    /// A T1K built-in theme with this name.
    Builtin(String),
    /// Colors read from the other terminal, in Ghostty's theme format (written to the themes
    /// folder on apply), and the name they get.
    Colors { name: String, text: String },
    /// The other terminal names a theme T1K doesn't have and can't read.
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct Found {
    pub app: App,
    /// iTerm2's profile name.
    pub profile: Option<String>,
    /// As the other app names it (may be a PostScript name, e.g. "MesloLGS-NF-Regular").
    pub font: Option<String>,
    pub size: Option<f32>,
    /// A multiple of the font size, like T1K's `line_height`.
    pub line_height: Option<f32>,
    pub theme: Option<ThemeSrc>,
    pub used: SystemTime,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn mtime(p: &Path) -> SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Every terminal with readable settings, most recently used first.
pub fn scan() -> Vec<Found> {
    let Some(h) = home() else { return Vec::new() };
    let mut out: Vec<Found> = [warp(&h), iterm(&h), ghostty(&h), kitty(&h), alacritty(&h)].into_iter().flatten().collect();
    out.sort_by(|a, b| b.used.cmp(&a.used));
    out
}

/// "Solarized Dark" and "solarized_dark" and "SolarizedDark" are the same theme.
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn builtin_named(name: &str) -> Option<String> {
    let n = norm(name);
    t1k_themes::BUILTIN.iter().find(|t| norm(&t.name) == n).map(|t| t.name.to_string())
}

/// Ghostty theme text from a palette, the one format T1K's importer and the themes folder share.
fn ghostty_text(fg: u32, bg: u32, cursor: Option<u32>, sel: Option<u32>, ansi: &[Option<u32>; 16]) -> String {
    let mut s = String::new();
    for (i, c) in ansi.iter().enumerate() {
        if let Some(c) = c {
            s += &format!("palette = {i}=#{c:06x}\n");
        }
    }
    s += &format!("background = #{bg:06x}\nforeground = #{fg:06x}\n");
    if let Some(c) = cursor {
        s += &format!("cursor-color = #{c:06x}\n");
    }
    if let Some(c) = sel {
        s += &format!("selection-background = #{c:06x}\n");
    }
    s
}

/// Only keep colors the importer accepts (it needs fg, bg and the 8 normal colors).
fn colors(name: String, text: String) -> Option<ThemeSrc> {
    t1k_themes::import::parse(text.as_bytes(), &name).ok()?;
    Some(ThemeSrc::Colors { name, text })
}

// ---- Warp ----------------------------------------------------------------------------------

fn warp(h: &Path) -> Option<Found> {
    let path = h.join(".warp/settings.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    // Warp writes TOML 1.1 (inline tables across lines), which the toml crate rejects, so read
    // the few keys we need line by line: `key = value` under `[section]`.
    let mut section = String::new();
    let mut vals = std::collections::HashMap::new();
    for line in text.lines().map(str::trim) {
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = sec.trim().to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            vals.insert(format!("{section}.{}", k.trim()), v.trim().to_string());
        }
    }
    let get = |k: &str| vals.get(k).map(|v| v.trim_matches('"').to_string());
    let num = |k: &str| get(k).and_then(|v| v.parse::<f32>().ok());
    let theme = vals.get("appearance.themes.theme").and_then(|v| toml::from_str::<toml::Value>(&format!("v = {v}")).ok()).and_then(|d| d.get("v").cloned()).and_then(|v| warp_theme(h, &v));
    Some(Found {
        app: App::Warp,
        profile: None,
        // Warp's default font (Hack) ships inside Warp; say so rather than guess.
        font: get("appearance.text.font_name").or(Some("Hack".into())),
        // Warp's window zoom scales the text too; T1K has no zoom, so fold it into the size.
        size: num("appearance.text.font_size").or(Some(13.)).map(|s| {
            let zoom = num("appearance.window.zoom_level").unwrap_or(100.) / 100.;
            (s * zoom * 2.).round() / 2.
        }),
        line_height: num("appearance.text.line_height_ratio"),
        theme,
        used: mtime(&path),
    })
}

fn warp_theme(h: &Path, v: &toml::Value) -> Option<ThemeSrc> {
    // A built-in name ("dark", "solarized_dark"), or a custom theme {custom = {name, path}}.
    if let Some(name) = v.as_str() {
        return Some(match name {
            // Warp's own Dark and Light: near-black / near-white with bright colors, which
            // T1K's Mercury pair matches.
            "dark" => ThemeSrc::Builtin("Mercury Dark".into()),
            "light" => ThemeSrc::Builtin("Mercury Light".into()),
            n => builtin_named(n).map(ThemeSrc::Builtin).unwrap_or(ThemeSrc::Unknown(n.replace('_', " "))),
        });
    }
    let custom = v.get("custom").or(Some(v))?;
    let path = custom.get("path").and_then(|p| p.as_str()).map(PathBuf::from);
    let name = custom.get("name").and_then(|p| p.as_str()).map(String::from);
    let file = path.or_else(|| name.as_ref().map(|n| h.join(".warp/themes").join(format!("{}.yaml", n.to_lowercase().replace(' ', "_")))))?;
    let label = name.unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default());
    let text = std::fs::read_to_string(&file).ok();
    text.as_deref().and_then(warp_yaml).and_then(|t| colors(format!("Warp {label}"), t)).or(Some(ThemeSrc::Unknown(label)))
}

/// Warp's theme YAML: `background`, `foreground`, and `terminal_colors: {normal, bright}: {black, red, …}`.
pub fn warp_yaml(text: &str) -> Option<String> {
    const NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    let (mut fg, mut bg) = (None, None);
    let mut ansi = [None; 16];
    let mut section = 0usize;
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.trim().split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "normal" => section = 0,
            "bright" => section = 8,
            "background" => bg = t1k_themes::import::parse_hex(v),
            "foreground" => fg = t1k_themes::import::parse_hex(v),
            _ => {
                if let Some(i) = NAMES.iter().position(|n| *n == k) {
                    ansi[section + i] = t1k_themes::import::parse_hex(v);
                }
            }
        }
    }
    Some(ghostty_text(fg?, bg?, None, None, &ansi))
}

// ---- iTerm2 ---------------------------------------------------------------------------------

fn iterm(h: &Path) -> Option<Found> {
    let path = h.join("Library/Preferences/com.googlecode.iterm2.plist");
    let v = plist::Value::from_file(&path).ok()?;
    let d = v.as_dictionary()?;
    let profiles = d.get("New Bookmarks")?.as_array()?;
    let default = d.get("Default Bookmark Guid").and_then(|g| g.as_string());
    let p = profiles
        .iter()
        .filter_map(|p| p.as_dictionary())
        .find(|p| default.is_some() && p.get("Guid").and_then(|g| g.as_string()) == default)
        .or_else(|| profiles.first().and_then(|p| p.as_dictionary()))?;
    let name = p.get("Name").and_then(|n| n.as_string()).unwrap_or("Default").to_string();
    // "MesloLGS-NF-Regular 13": the PostScript name, then the size.
    let (font, size) = match p.get("Normal Font").and_then(|f| f.as_string()).and_then(|f| f.rsplit_once(' ')) {
        Some((f, s)) => (Some(f.to_string()), s.parse::<f32>().ok()),
        None => (None, None),
    };
    // iTerm2's spacing multiplies the font's natural line height (about 1.2 × the size).
    let spacing = p.get("Vertical Spacing").and_then(|v| v.as_real().or_else(|| v.as_signed_integer().map(|i| i as f64)));
    let line_height = spacing.map(|s| ((1.2 * s as f32) * 100.).round() / 100.);
    let color = |key: &str| -> Option<u32> {
        // Profiles with separate light and dark colors keep them under "… (Dark)".
        let c = p.get(key).or_else(|| p.get(&format!("{key} (Dark)")))?.as_dictionary()?;
        let ch = |k: &str| c.get(k).and_then(|x| x.as_real().or_else(|| x.as_signed_integer().map(|i| i as f64))).map(|f| (f.clamp(0., 1.) * 255.).round() as u32);
        Some(ch("Red Component")? << 16 | ch("Green Component")? << 8 | ch("Blue Component")?)
    };
    let mut ansi = [None; 16];
    for (i, slot) in ansi.iter_mut().enumerate() {
        *slot = color(&format!("Ansi {i} Color"));
    }
    let theme = match (color("Foreground Color"), color("Background Color")) {
        (Some(fg), Some(bg)) => colors(format!("iTerm2 {name}"), ghostty_text(fg, bg, color("Cursor Color"), color("Selection Color"), &ansi)),
        _ => None,
    };
    Some(Found { app: App::ITerm2, profile: Some(name), font, size, line_height, theme, used: mtime(&path) })
}

// ---- Ghostty --------------------------------------------------------------------------------

fn ghostty(h: &Path) -> Option<Found> {
    let path = [h.join(".config/ghostty/config"), h.join("Library/Application Support/com.mitchellh.ghostty/config")].into_iter().find(|p| p.is_file())?;
    let text = std::fs::read_to_string(&path).ok()?;
    let get = |key: &str| text.lines().filter_map(|l| l.split_once('=')).filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().trim_matches('"').to_string()).next();
    let size = get("font-size").and_then(|s| s.parse().ok()).or(Some(13.));
    let line_height = get("adjust-cell-height").and_then(|v| {
        let base = 1.2;
        if let Some(p) = v.strip_suffix('%') {
            p.trim().parse::<f32>().ok().map(|p| base * (1. + p / 100.))
        } else {
            // Points added to the cell.
            v.parse::<f32>().ok().map(|px| base + px / size.unwrap_or(13.))
        }
        .map(|f| (f * 100.).round() / 100.)
    });
    let theme = get("theme").and_then(|t| {
        // "light:Foo,dark:Bar" follows the system; take the dark one.
        let name = t.split(',').find_map(|p| p.trim().strip_prefix("dark:")).unwrap_or(&t).trim().to_string();
        if let Some(b) = builtin_named(&name) {
            return Some(ThemeSrc::Builtin(b));
        }
        let files = [h.join(".config/ghostty/themes").join(&name), PathBuf::from("/Applications/Ghostty.app/Contents/Resources/ghostty/themes").join(&name)];
        files.iter().find_map(|f| std::fs::read_to_string(f).ok()).and_then(|t| colors(format!("Ghostty {name}"), t)).or(Some(ThemeSrc::Unknown(name)))
    });
    // Colors written straight into the config.
    let theme = theme.or_else(|| colors("Ghostty".into(), text.clone()));
    Some(Found { app: App::Ghostty, profile: None, font: get("font-family"), size, line_height, theme, used: mtime(&path) })
}

// ---- kitty ----------------------------------------------------------------------------------

fn kitty(h: &Path) -> Option<Found> {
    let path = h.join(".config/kitty/kitty.conf");
    let mut text = std::fs::read_to_string(&path).ok()?;
    // Themes usually live in an included file (kitty +kitten themes writes current-theme.conf).
    let includes: Vec<PathBuf> = text.lines().filter_map(|l| l.trim().strip_prefix("include ")).map(|f| path.parent().unwrap_or(h).join(f.trim())).collect();
    for f in includes {
        if let Ok(t) = std::fs::read_to_string(f) {
            text = t + "\n" + &text;
        }
    }
    let mut kv = std::collections::HashMap::new();
    for l in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
        if let Some((k, v)) = l.split_once(char::is_whitespace) {
            kv.insert(k.to_string(), v.trim().to_string());
        }
    }
    let size = kv.get("font_size").and_then(|s| s.parse().ok()).or(Some(11.));
    let line_height = kv.get("modify_font").and_then(|v| v.strip_prefix("cell_height")).and_then(|v| {
        let v = v.trim();
        v.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()).map(|p| ((1.2 * p / 100.) * 100.).round() / 100.)
    });
    let hex = |k: &str| kv.get(k).and_then(|v| t1k_themes::import::parse_hex(v));
    let mut ansi = [None; 16];
    for (i, slot) in ansi.iter_mut().enumerate() {
        *slot = hex(&format!("color{i}"));
    }
    let theme = match (hex("foreground"), hex("background")) {
        (Some(fg), Some(bg)) => colors("kitty".into(), ghostty_text(fg, bg, hex("cursor"), hex("selection_background"), &ansi)),
        _ => None,
    };
    let font = kv.get("font_family").filter(|f| f.as_str() != "monospace").cloned();
    Some(Found { app: App::Kitty, profile: None, font, size, line_height, theme, used: mtime(&path) })
}

// ---- Alacritty ------------------------------------------------------------------------------

fn alacritty(h: &Path) -> Option<Found> {
    let path = [h.join(".config/alacritty/alacritty.toml"), h.join(".alacritty.toml")].into_iter().find(|p| p.is_file())?;
    let text = std::fs::read_to_string(&path).ok()?;
    let doc: toml::Value = toml::from_str(&text).ok()?;
    let f = doc.get("font");
    let size = f.and_then(|f| f.get("size")).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).map(|s| s as f32);
    let font = f.and_then(|f| f.get("normal")).and_then(|n| n.get("family")).and_then(|v| v.as_str()).map(String::from);
    let line_height = f.and_then(|f| f.get("offset")).and_then(|o| o.get("y")).and_then(|v| v.as_integer()).map(|y| {
        let s = size.unwrap_or(11.25);
        ((1.2 + y as f32 / s) * 100.).round() / 100.
    });
    let theme = doc.get("colors").and_then(|_| t1k_themes::import::parse(text.as_bytes(), "Alacritty").ok()).map(|_| ThemeSrc::Colors {
        name: "Alacritty".into(),
        text: text.clone(),
    });
    Some(Found { app: App::Alacritty, profile: None, font, size, line_height, theme, used: mtime(&path) })
}

// ---- Plan -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Font,
    Size,
    LineHeight,
    Theme,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Font => "Font",
            Kind::Size => "Size",
            Kind::LineHeight => "Line height",
            Kind::Theme => "Colors",
        }
    }
}

/// One row: what T1K has now, what it would change to, and why not when it can't.
#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    pub now: String,
    /// None when the other terminal's value can't be used.
    pub to: Option<String>,
    pub reason: Option<String>,
}

impl Item {
    /// Applies (has a value that differs from T1K's).
    pub fn applies(&self) -> bool {
        self.to.is_some() && self.reason.is_none()
    }
}

/// What T1K has now, for comparing.
pub struct Current {
    pub font: String,
    pub size: f32,
    pub line_height: f32,
    pub theme: String,
}

fn num(f: f32) -> String {
    let s = format!("{f:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Map a font name as another app writes it ("MesloLGLDZForPowerline-Regular") to an installed
/// family ("Meslo LG L DZ for Powerline").
pub fn installed_family(name: &str, fonts: &[String]) -> Option<String> {
    let n = norm(name);
    if let Some(f) = fonts.iter().find(|f| norm(f) == n) {
        return Some(f.clone());
    }
    // Drop a style suffix: "-Regular", "-Book", "Regular".
    let base = name.split('-').next().unwrap_or(name);
    let b = norm(base.trim_end_matches("Regular"));
    fonts.iter().find(|f| norm(f) == b).cloned()
}

pub fn plan(f: &Found, cur: &Current, fonts: &[String]) -> Vec<Item> {
    let app = f.app.label();
    let unread = |now: String| format!("Couldn't read this from {app}'s settings. Keeping {now}.");
    let mut out = Vec::new();
    // Font.
    let now = cur.font.clone();
    out.push(match &f.font {
        None => Item { kind: Kind::Font, to: None, reason: Some(unread(now.clone())), now },
        Some(name) => match installed_family(name, fonts) {
            Some(fam) if fam == cur.font => Item { kind: Kind::Font, to: Some(fam), reason: Some("Already the same.".into()), now },
            Some(fam) => Item { kind: Kind::Font, to: Some(fam), reason: None, now },
            None => Item { kind: Kind::Font, to: Some(name.clone()), reason: Some(format!("{name} isn't installed on this Mac. Keeping {now}.")), now },
        },
    });
    let same = |a: f32, b: f32| (a - b).abs() < 0.01;
    // Size.
    let now = num(cur.size);
    out.push(match f.size {
        None => Item { kind: Kind::Size, to: None, reason: Some(unread(now.clone())), now },
        Some(s) => Item { kind: Kind::Size, to: Some(num(s)), reason: same(s, cur.size).then(|| "Already the same.".into()), now },
    });
    // Line height.
    let now = num(cur.line_height);
    out.push(match f.line_height {
        None => Item { kind: Kind::LineHeight, to: None, reason: Some(unread(now.clone())), now },
        Some(l) => Item { kind: Kind::LineHeight, to: Some(num(l)), reason: same(l, cur.line_height).then(|| "Already the same.".into()), now },
    });
    // Colors.
    let now = cur.theme.clone();
    out.push(match &f.theme {
        None => Item { kind: Kind::Theme, to: None, reason: Some(unread(now.clone())), now },
        Some(ThemeSrc::Builtin(n)) => Item { kind: Kind::Theme, to: Some(n.clone()), reason: (n == &cur.theme).then(|| "Already the same.".into()), now },
        Some(ThemeSrc::Colors { name, .. }) => Item { kind: Kind::Theme, to: Some(name.clone()), reason: (name == &cur.theme).then(|| "Already the same.".into()), now },
        Some(ThemeSrc::Unknown(n)) => Item { kind: Kind::Theme, to: Some(n.clone()), reason: Some(format!("No matching T1K theme. Keeping {now}.")), now },
    });
    out
}

/// The theme file to write for colors read from another app (None for a built-in).
pub fn theme_file(f: &Found) -> Option<(String, String)> {
    match &f.theme {
        Some(ThemeSrc::Colors { name, text }) => Some((name.clone(), text.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_map_to_installed_families() {
        let fonts = vec!["Meslo LG L DZ for Powerline".to_string(), "JetBrains Mono".to_string(), "MesloLGS NF".to_string()];
        assert_eq!(installed_family("MesloLGLDZForPowerline-Regular", &fonts).as_deref(), Some("Meslo LG L DZ for Powerline"));
        assert_eq!(installed_family("MesloLGS-NF-Regular", &fonts), None);
        assert_eq!(installed_family("MesloLGSNF-Regular", &fonts).as_deref(), Some("MesloLGS NF"));
        assert_eq!(installed_family("JetBrainsMono-Regular", &fonts).as_deref(), Some("JetBrains Mono"));
        assert_eq!(installed_family("Hack", &fonts), None);
    }

    #[test]
    fn warp_yaml_becomes_a_theme() {
        let y = "accent: '#c1c0c0'\nbackground: '#2d2a2e'\nforeground: '#fcfcfa'\nterminal_colors:\n  bright:\n    black: '#727072'\n    red: '#ff6188'\n  normal:\n    black: '#2d2a2e'\n    red: '#ff6188'\n    green: '#a9dc76'\n    yellow: '#ffd866'\n    blue: '#6ab0f3'\n    magenta: '#ab9df2'\n    cyan: '#78dce8'\n    white: '#fcfcfa'\n";
        let text = warp_yaml(y).unwrap();
        let t = t1k_themes::import::parse(text.as_bytes(), "Warp Monokai").unwrap();
        assert_eq!(t.bg, 0x2d2a2e);
        assert_eq!(t.ansi[9], 0xff6188);
        assert_eq!(t.ansi[1], 0xff6188);
    }

    #[test]
    fn plan_explains_what_cannot_apply() {
        let f = Found { app: App::Warp, profile: None, font: Some("Hack".into()), size: Some(11.), line_height: Some(1.5), theme: Some(ThemeSrc::Unknown("cyber wave".into())), used: SystemTime::UNIX_EPOCH };
        let cur = Current { font: "JetBrains Mono".into(), size: 13., line_height: 1.45, theme: "Mercury Dark".into() };
        let p = plan(&f, &cur, &["JetBrains Mono".to_string()]);
        assert!(!p[0].applies() && p[0].reason.as_deref().unwrap().contains("isn't installed"));
        assert!(p[1].applies() && p[1].to.as_deref() == Some("11"));
        assert!(p[2].applies() && p[2].to.as_deref() == Some("1.5"));
        assert!(!p[3].applies());
    }
}
