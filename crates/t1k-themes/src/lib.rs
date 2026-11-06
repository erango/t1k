//! Themes: terminal palettes plus UI chrome colors derived from them.
//!
//! Every theme is one terminal palette. The window chrome (tab bar, sidebar, borders, text
//! tiers, status colors) is derived from it, so any theme, built-in or imported later, gives a
//! matching UI. Derivation rules live in `docs/design.md` (Appearance).

mod builtin;
pub mod import;

use std::borrow::Cow;
use std::path::Path;
use std::sync::Mutex;

use gpui::{Hsla, Rgba};
use t1k_terminal::palette::Palette;
use t1k_terminal::Rgb;

pub use builtin::BUILTIN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

/// How the app picks between the dark and light theme slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AppearanceMode {
    #[default]
    System,
    Dark,
    Light,
}

/// A theme as declared: hex colors, ANSI 0-15, optional accent override.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: Cow<'static, str>,
    pub appearance: Appearance,
    pub fg: u32,
    pub bg: u32,
    pub cursor: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
    /// UI accent (selected-tab bar, unread). Defaults to ANSI blue.
    pub accent: Option<u32>,
}

/// Chrome colors for the app UI. Names follow the tokens in `docs/design.md`.
#[derive(Clone, Copy, Debug)]
pub struct UiColors {
    pub bg_chrome: Hsla,
    pub bg_sidebar: Hsla,
    pub bg_terminal: Hsla,
    pub bg_raised: Hsla,
    pub bg_hover: Hsla,
    pub bg_selected: Hsla,
    pub bg_control: Hsla,
    pub border: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub text_quiet: Hsla,
    pub text_disabled: Hsla,
    pub accent: Hsla,
    pub link: Hsla,
    pub status_working: Hsla,
    pub status_needs: Hsla,
    pub status_done: Hsla,
    pub status_error: Hsla,
}

fn rgb(v: u32) -> Rgb {
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let x = ((a >> s) & 0xff) as f32;
        let y = ((b >> s) & 0xff) as f32;
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

fn hsla(v: u32, alpha: f32) -> Hsla {
    let mut c: Hsla =
        Rgba { r: ((v >> 16) & 0xff) as f32 / 255., g: ((v >> 8) & 0xff) as f32 / 255., b: (v & 0xff) as f32 / 255., a: 1. }
            .into();
    c.a = alpha;
    c
}

impl Theme {
    fn same_colors(&self, o: &Theme) -> bool {
        (self.fg, self.bg, self.cursor, self.selection, self.ansi, self.accent)
            == (o.fg, o.bg, o.cursor, o.selection, o.ansi, o.accent)
    }

    pub fn is_dark(&self) -> bool {
        self.appearance == Appearance::Dark
    }

    fn link_hex(&self) -> u32 {
        // Bright blue reads better on dark backgrounds; normal blue on light.
        if self.is_dark() { self.ansi[12] } else { self.ansi[4] }
    }

    pub fn palette(&self) -> Palette {
        Palette {
            foreground: rgb(self.fg),
            background: rgb(self.bg),
            cursor: rgb(self.cursor),
            cursor_text: rgb(self.bg),
            selection_bg: rgb(self.selection),
            selection_fg: None,
            link: rgb(self.link_hex()),
            ansi: self.ansi.map(rgb),
        }
    }

    pub fn ui(&self) -> UiColors {
        let (bg, fg) = (self.bg, self.fg);
        let shade = if self.is_dark() { 0x000000 } else { 0x6b7280 };
        let accent = self.accent.unwrap_or(if self.is_dark() { self.ansi[12] } else { self.ansi[4] });
        UiColors {
            bg_chrome: hsla(mix(bg, shade, if self.is_dark() { 0.18 } else { 0.07 }), 1.),
            bg_sidebar: hsla(mix(bg, shade, if self.is_dark() { 0.25 } else { 0.10 }), 1.),
            bg_terminal: hsla(bg, 1.),
            bg_raised: hsla(mix(bg, fg, 0.04), 1.),
            bg_hover: hsla(mix(bg, fg, 0.06), 1.),
            bg_selected: hsla(mix(bg, fg, 0.11), 1.),
            bg_control: hsla(mix(bg, fg, 0.16), 1.),
            border: hsla(fg, 0.10),
            text_primary: hsla(fg, 1.),
            text_secondary: hsla(mix(bg, fg, 0.72), 1.),
            text_tertiary: hsla(mix(bg, fg, 0.52), 1.),
            text_quiet: hsla(mix(bg, fg, 0.62), 1.),
            text_disabled: hsla(mix(bg, fg, 0.36), 1.),
            accent: hsla(accent, 1.),
            link: hsla(self.link_hex(), 1.),
            status_working: hsla(mix(bg, fg, 0.52), 1.),
            status_needs: hsla(self.ansi[3], 1.),
            status_done: hsla(self.ansi[2], 1.),
            status_error: hsla(self.ansi[1], 1.),
        }
    }
}

/// Themes imported into the user's themes folder (leaked: there are few, and they live forever).
static USER: Mutex<Vec<&'static Theme>> = Mutex::new(Vec::new());

/// All themes: built-ins first, then user themes (a user theme with a built-in's name wins).
pub fn all() -> Vec<&'static Theme> {
    let user = USER.lock().unwrap().clone();
    let mut out: Vec<&'static Theme> =
        BUILTIN.iter().filter(|b| !user.iter().any(|u| u.name.eq_ignore_ascii_case(&b.name))).collect();
    out.extend(user);
    out
}

pub fn find(name: &str) -> Option<&'static Theme> {
    all().into_iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

/// (Re)load every theme file in `dir`. Returns the number loaded; bad files are logged and skipped.
pub fn load_user_themes(dir: &Path) -> usize {
    let mut loaded: Vec<&'static Theme> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    paths.sort();
    for p in paths {
        match import::load(&p) {
            Ok(t) => {
                // Reuse the leaked copy if nothing changed, so reloads don't grow memory.
                let existing = USER.lock().unwrap().iter().copied().find(|e| e.name == t.name && e.same_colors(&t));
                loaded.push(existing.unwrap_or_else(|| Box::leak(Box::new(t))));
            }
            Err(e) => log::warn!("theme {}: {e}", p.display()),
        }
    }
    let n = loaded.len();
    *USER.lock().unwrap() = loaded;
    n
}

pub fn default_for(appearance: Appearance) -> &'static Theme {
    match appearance {
        Appearance::Dark => &BUILTIN[0],
        Appearance::Light => &BUILTIN[1],
    }
}

/// WCAG relative luminance of a 0xRRGGBB color.
pub fn lum(v: u32) -> f32 {
    let f = |c: u32| {
        let c = c as f32 / 255.;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f((v >> 16) & 0xff) + 0.7152 * f((v >> 8) & 0xff) + 0.0722 * f(v & 0xff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: u32, b: u32) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn names_unique_and_slots_exist() {
        let mut names: Vec<_> = BUILTIN.iter().map(|t| t.name.to_lowercase()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), BUILTIN.len());
        assert!(default_for(Appearance::Dark).is_dark());
        assert!(!default_for(Appearance::Light).is_dark());
    }

    #[test]
    fn appearance_matches_background_and_text_is_readable() {
        for t in BUILTIN {
            assert_eq!(t.is_dark(), lum(t.bg) < 0.2, "{} appearance vs bg", t.name);
            assert!(contrast(t.fg, t.bg) >= 4.5, "{} fg/bg contrast {}", t.name, contrast(t.fg, t.bg));
            assert!(contrast(t.link_hex(), t.bg) >= 3.0, "{} link contrast", t.name);
        }
    }
}
