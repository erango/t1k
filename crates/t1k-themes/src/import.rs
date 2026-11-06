//! Import color schemes from other terminals. The format is detected from the file contents.
//!
//! Supported: Ghostty theme files, iTerm2 `.itermcolors`, Alacritty TOML, Windows Terminal JSON
//! (a scheme or a `schemes` list), base16 YAML, and T1K's own TOML.

use std::borrow::Cow;
use std::path::Path;

use crate::{lum, Appearance, Theme};

pub type Result<T> = std::result::Result<T, String>;

/// Parse "#rrggbb", "rrggbb", "0xrrggbb" (also "#rgb").
pub fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'');
    let h = s.strip_prefix('#').or_else(|| s.strip_prefix("0x")).unwrap_or(s);
    match h.len() {
        6 => u32::from_str_radix(h, 16).ok(),
        3 => {
            let v = u32::from_str_radix(h, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some((r * 17) << 16 | (g * 17) << 8 | b * 17)
        }
        // #rrggbbaa: drop alpha.
        8 => u32::from_str_radix(&h[..6], 16).ok(),
        _ => None,
    }
}

/// Fill in what a format may omit, and decide dark vs light from the background.
fn finish(name: String, fg: u32, bg: u32, cursor: Option<u32>, selection: Option<u32>, ansi: [u32; 16]) -> Theme {
    let appearance = if lum(bg) < 0.2 { Appearance::Dark } else { Appearance::Light };
    let selection = selection.unwrap_or_else(|| {
        // 25% of the way from bg to fg.
        let m = |s: u32| {
            let (a, b) = (((bg >> s) & 0xff) as f32, ((fg >> s) & 0xff) as f32);
            ((a + (b - a) * 0.25).round() as u32) << s
        };
        m(16) | m(8) | m(0)
    });
    Theme { name: Cow::Owned(name), appearance, fg, bg, cursor: cursor.unwrap_or(fg), selection, ansi, accent: None }
}

fn name_from_path(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_else(|| "Imported".into())
}

pub fn load(path: &Path) -> Result<Theme> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    parse(&bytes, &name_from_path(path))
}

/// Detect the format and parse. `fallback_name` is used when the file has no name.
pub fn parse(bytes: &[u8], fallback_name: &str) -> Result<Theme> {
    let text = String::from_utf8_lossy(bytes);
    let t = text.trim_start();
    if t.starts_with("<?xml") || t.starts_with("<!DOCTYPE plist") || t.starts_with("<plist") {
        return iterm(bytes, fallback_name);
    }
    if t.starts_with('{') {
        return windows_terminal(&text, fallback_name);
    }
    if text.lines().any(|l| l.trim_start().starts_with("base00")) {
        return base16(&text, fallback_name);
    }
    if text.lines().any(|l| l.trim_start().starts_with("palette")) {
        return ghostty(&text, fallback_name);
    }
    toml_theme(&text, fallback_name)
}

fn require(v: Option<u32>, what: &str) -> Result<u32> {
    v.ok_or_else(|| format!("missing {what}"))
}

fn ghostty(text: &str, name: &str) -> Result<Theme> {
    let mut ansi = [None; 16];
    let (mut fg, mut bg, mut cursor, mut sel) = (None, None, None, None);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "palette" => {
                if let Some((i, c)) = v.split_once('=') {
                    if let (Ok(i), Some(c)) = (i.trim().parse::<usize>(), parse_hex(c)) {
                        if i < 16 {
                            ansi[i] = Some(c);
                        }
                    }
                }
            }
            "foreground" => fg = parse_hex(v),
            "background" => bg = parse_hex(v),
            "cursor-color" => cursor = parse_hex(v),
            "selection-background" => sel = parse_hex(v),
            _ => {}
        }
    }
    let ansi = collect_ansi(ansi)?;
    Ok(finish(name.into(), require(fg, "foreground")?, require(bg, "background")?, cursor, sel, ansi))
}

fn collect_ansi(a: [Option<u32>; 16]) -> Result<[u32; 16]> {
    // Bright colors default to their normal counterparts.
    let mut out = [0; 16];
    for i in 0..16 {
        out[i] = match a[i].or(if i >= 8 { a[i - 8] } else { None }) {
            Some(c) => c,
            None => return Err(format!("missing ANSI color {i}")),
        };
    }
    Ok(out)
}

fn iterm(bytes: &[u8], name: &str) -> Result<Theme> {
    let v: plist::Value = plist::from_bytes(bytes).map_err(|e| e.to_string())?;
    let dict = v.as_dictionary().ok_or("not a plist dictionary")?;
    let color = |key: &str| -> Option<u32> {
        let d = dict.get(key)?.as_dictionary()?;
        let c = |k: &str| d.get(k).and_then(|x| x.as_real()).map(|f| (f.clamp(0., 1.) * 255.).round() as u32);
        Some(c("Red Component")? << 16 | c("Green Component")? << 8 | c("Blue Component")?)
    };
    let mut ansi = [None; 16];
    for (i, slot) in ansi.iter_mut().enumerate() {
        *slot = color(&format!("Ansi {i} Color"));
    }
    Ok(finish(
        name.into(),
        require(color("Foreground Color"), "Foreground Color")?,
        require(color("Background Color"), "Background Color")?,
        color("Cursor Color"),
        color("Selection Color"),
        collect_ansi(ansi)?,
    ))
}

fn windows_terminal(text: &str, name: &str) -> Result<Theme> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    // Either a scheme, or a settings file with a list of schemes (take the first).
    let scheme = v.get("schemes").and_then(|s| s.get(0)).unwrap_or(&v);
    let c = |k: &str| scheme.get(k).and_then(|x| x.as_str()).and_then(parse_hex);
    const KEYS: [&str; 8] = ["black", "red", "green", "yellow", "blue", "purple", "cyan", "white"];
    let mut ansi = [None; 16];
    for (i, k) in KEYS.iter().enumerate() {
        ansi[i] = c(k);
        let bright = format!("bright{}{}", k[..1].to_uppercase(), &k[1..]);
        ansi[i + 8] = c(&bright);
    }
    let name = scheme.get("name").and_then(|n| n.as_str()).unwrap_or(name).to_string();
    Ok(finish(
        name,
        require(c("foreground"), "foreground")?,
        require(c("background"), "background")?,
        c("cursorColor"),
        c("selectionBackground"),
        collect_ansi(ansi)?,
    ))
}

fn base16(text: &str, name: &str) -> Result<Theme> {
    let mut b = [None; 16];
    let mut scheme_name = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let k = k.trim();
        let v = v.split('#').next().filter(|s| !s.trim().is_empty()).unwrap_or(v);
        if k == "scheme" || k == "name" {
            scheme_name = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(i) = k.strip_prefix("base0").and_then(|h| u32::from_str_radix(h, 16).ok()) {
            b[i as usize] = parse_hex(v.trim().trim_matches('"'));
        }
    }
    let g = |i: usize| require(b[i], &format!("base0{i:X}"));
    let ansi = [
        g(0)?, g(8)?, g(0xB)?, g(0xA)?, g(0xD)?, g(0xE)?, g(0xC)?, g(5)?,
        g(3)?, g(8)?, g(0xB)?, g(0xA)?, g(0xD)?, g(0xE)?, g(0xC)?, g(7)?,
    ];
    Ok(finish(scheme_name.unwrap_or_else(|| name.into()), g(5)?, g(0)?, Some(g(5)?), Some(g(2)?), ansi))
}

/// Alacritty `[colors]` TOML, or T1K's own format:
/// `name`, `foreground`, `background`, `cursor`, `selection`, `accent`, `ansi = [16 hex strings]`.
fn toml_theme(text: &str, name: &str) -> Result<Theme> {
    let v: toml::Value = toml::from_str(text).map_err(|e| format!("not a recognized theme format: {e}"))?;
    let hex = |x: Option<&toml::Value>| x.and_then(|x| x.as_str()).and_then(parse_hex);
    if let Some(colors) = v.get("colors") {
        let section = |s: &str, k: &str| hex(colors.get(s).and_then(|t| t.get(k)));
        const KEYS: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
        let mut ansi = [None; 16];
        for (i, k) in KEYS.iter().enumerate() {
            ansi[i] = section("normal", k);
            ansi[i + 8] = section("bright", k);
        }
        return Ok(finish(
            name.into(),
            require(section("primary", "foreground"), "colors.primary.foreground")?,
            require(section("primary", "background"), "colors.primary.background")?,
            section("cursor", "cursor"),
            section("selection", "background"),
            collect_ansi(ansi)?,
        ));
    }
    let list = v.get("ansi").and_then(|a| a.as_array()).ok_or("missing `ansi` (or `[colors]`)")?;
    let mut ansi = [None; 16];
    for (i, c) in list.iter().take(16).enumerate() {
        ansi[i] = hex(Some(c));
    }
    let mut t = finish(
        v.get("name").and_then(|n| n.as_str()).unwrap_or(name).into(),
        require(hex(v.get("foreground")), "foreground")?,
        require(hex(v.get("background")), "background")?,
        hex(v.get("cursor")),
        hex(v.get("selection")),
        collect_ansi(ansi)?,
    );
    t.accent = hex(v.get("accent"));
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSI: [&str; 16] = [
        "000000", "aa0000", "00aa00", "aaaa00", "0000aa", "aa00aa", "00aaaa", "aaaaaa",
        "555555", "ff5555", "55ff55", "ffff55", "5555ff", "ff55ff", "55ffff", "ffffff",
    ];

    fn check(t: &Theme) {
        assert_eq!(t.ansi[1], 0xaa0000);
        assert_eq!(t.ansi[12], 0x5555ff);
        assert_eq!(t.bg, 0x101010);
        assert_eq!(t.fg, 0xd0d0d0);
        assert!(t.is_dark());
    }

    #[test]
    fn ghostty_format() {
        let mut s = String::from("background = 101010\nforeground = #d0d0d0\ncursor-color = d0d0d0\n");
        for (i, c) in ANSI.iter().enumerate() {
            s += &format!("palette = {i}=#{c}\n");
        }
        check(&parse(s.as_bytes(), "x").unwrap());
    }

    #[test]
    fn alacritty_format() {
        let n = |o: usize| {
            ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"]
                .iter()
                .enumerate()
                .map(|(i, k)| format!("{k} = \"0x{}\"\n", ANSI[i + o]))
                .collect::<String>()
        };
        let s = format!(
            "[colors.primary]\nbackground = '#101010'\nforeground = '#d0d0d0'\n[colors.normal]\n{}[colors.bright]\n{}",
            n(0),
            n(8)
        );
        check(&parse(s.as_bytes(), "x").unwrap());
    }

    #[test]
    fn windows_terminal_format() {
        let keys = ["black", "red", "green", "yellow", "blue", "purple", "cyan", "white"];
        let mut fields = vec![
            "\"name\": \"WT Test\"".to_string(),
            "\"background\": \"#101010\"".into(),
            "\"foreground\": \"#D0D0D0\"".into(),
        ];
        for (i, k) in keys.iter().enumerate() {
            fields.push(format!("\"{k}\": \"#{}\"", ANSI[i]));
            fields.push(format!("\"bright{}{}\": \"#{}\"", k[..1].to_uppercase(), &k[1..], ANSI[i + 8]));
        }
        let t = parse(format!("{{{}}}", fields.join(",")).as_bytes(), "x").unwrap();
        check(&t);
        assert_eq!(t.name, "WT Test");
    }

    #[test]
    fn iterm_format() {
        let comp = |hex: &str| {
            let v = u32::from_str_radix(hex, 16).unwrap();
            let f = |s: u32| ((v >> s) & 0xff) as f64 / 255.;
            format!(
                "<dict><key>Blue Component</key><real>{}</real><key>Green Component</key><real>{}</real><key>Red Component</key><real>{}</real></dict>",
                f(0), f(8), f(16)
            )
        };
        let mut body = String::new();
        for (i, c) in ANSI.iter().enumerate() {
            body += &format!("<key>Ansi {i} Color</key>{}", comp(c));
        }
        body += &format!("<key>Background Color</key>{}", comp("101010"));
        body += &format!("<key>Foreground Color</key>{}", comp("d0d0d0"));
        let xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>{body}</dict></plist>");
        check(&parse(xml.as_bytes(), "x").unwrap());
    }

    #[test]
    fn base16_format() {
        let s = "scheme: \"B16 Test\"\nbase00: \"101010\"\nbase01: \"202020\"\nbase02: \"303030\"\nbase03: \"555555\"\n\
                 base04: \"909090\"\nbase05: \"d0d0d0\"\nbase06: \"e0e0e0\"\nbase07: \"ffffff\"\nbase08: \"aa0000\"\n\
                 base09: \"cc6600\"\nbase0A: \"aaaa00\"\nbase0B: \"00aa00\"\nbase0C: \"00aaaa\"\nbase0D: \"5555ff\"\n\
                 base0E: \"aa00aa\"\nbase0F: \"884400\"\n";
        let t = parse(s.as_bytes(), "x").unwrap();
        assert_eq!(t.name, "B16 Test");
        assert_eq!((t.bg, t.fg, t.ansi[1], t.ansi[12]), (0x101010, 0xd0d0d0, 0xaa0000, 0x5555ff));
    }

    #[test]
    fn native_format_and_light_detection() {
        let ansi = ANSI.iter().map(|c| format!("\"#{c}\"")).collect::<Vec<_>>().join(", ");
        let s = format!("name = \"Paper\"\nbackground = \"#fafafa\"\nforeground = \"#222222\"\naccent = \"#3366cc\"\nansi = [{ansi}]\n");
        let t = parse(s.as_bytes(), "x").unwrap();
        assert_eq!(t.name, "Paper");
        assert!(!t.is_dark());
        assert_eq!(t.accent, Some(0x3366cc));
    }

    #[test]
    fn errors_are_explained() {
        assert!(parse(b"background = 101010\n", "x").unwrap_err().contains("missing"));
    }
}
