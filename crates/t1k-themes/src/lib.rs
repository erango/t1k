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
    /// Hover fill on the sidebar (its own shade, since the sidebar isn't the terminal's).
    pub sidebar_hover: Hsla,
    /// Hairline between the sidebar and the terminal (broken where the selected tab joins).
    pub sidebar_edge: Hsla,
    pub bg_selected: Hsla,
    pub bg_control: Hsla,
    pub border: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub text_quiet: Hsla,
    pub text_disabled: Hsla,
    pub accent: Hsla,
    /// Text on an accent fill (primary buttons, on pills): dark on a light accent, white on a
    /// dark one, so it always reads.
    pub on_accent: Hsla,
    pub link: Hsla,
    pub status_working: Hsla,
    pub status_needs: Hsla,
    pub status_done: Hsla,
    pub status_error: Hsla,
    /// Opacity for program icons (lowered while the window is inactive).
    pub icon_alpha: f32,
}

fn blend(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (x, y) = (Rgba::from(a), Rgba::from(b));
    let l = |p: f32, q: f32| p + (q - p) * t;
    Rgba { r: l(x.r, y.r), g: l(x.g, y.g), b: l(x.b, y.b), a: l(x.a, y.a) }.into()
}

impl UiColors {
    /// The window's inactive look (canvas v3-u, option d), `k` of the way there (0 = active,
    /// 1 = inactive, in between while fading). Chrome text and accents soften toward the
    /// sidebar, borders and fills flatten. Amber and red stay as they are, so a tab that needs
    /// you stands out more, not less. The terminal is drawn from its own palette and is untouched.
    pub fn inactive(&self, k: f32) -> UiColors {
        if k <= 0. {
            return *self;
        }
        let k = k.min(1.);
        let surface = self.bg_sidebar;
        let text = |c: Hsla| blend(c, surface, 0.30 * k);
        let line = |c: Hsla| Hsla { a: c.a * (1. - 0.40 * k), ..c };
        let fill = |c: Hsla| blend(c, surface, 0.50 * k);
        let accent = |c: Hsla| blend(Hsla { s: c.s * (1. - 0.35 * k), ..c }, surface, 0.25 * k);
        UiColors {
            bg_raised: fill(self.bg_raised),
            bg_hover: fill(self.bg_hover),
            sidebar_hover: fill(self.sidebar_hover),
            bg_control: fill(self.bg_control),
            sidebar_edge: line(self.sidebar_edge),
            border: line(self.border),
            text_primary: text(self.text_primary),
            text_secondary: text(self.text_secondary),
            text_tertiary: text(self.text_tertiary),
            text_quiet: text(self.text_quiet),
            text_disabled: text(self.text_disabled),
            accent: accent(self.accent),
            link: accent(self.link),
            status_working: text(self.status_working),
            status_done: accent(self.status_done),
            icon_alpha: 1. - 0.25 * k,
            ..*self
        }
    }
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

    /// Near-black on white or white on black (contrast 18:1 or more): the chrome then keeps
    /// secondary text, borders and selection strong instead of fading them into the background.
    pub fn is_high_contrast(&self) -> bool {
        let (a, b) = (lum(self.fg), lum(self.bg));
        (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 18.
    }

    pub fn ui(&self) -> UiColors {
        let (bg, fg) = (self.bg, self.fg);
        let hc = self.is_high_contrast();
        // Text tiers and fills: (normal, high contrast).
        let t = |n: f32, h: f32| if hc { h } else { n };
        let sidebar = mix(bg, fg, if self.is_dark() { t(0.05, 0.1) } else { t(0.12, 0.1) });
        let shade = if self.is_dark() { 0x000000 } else { 0x6b7280 };
        let accent = self.accent.unwrap_or(if self.is_dark() { self.ansi[12] } else { self.ansi[4] });
        UiColors {
            bg_chrome: hsla(mix(bg, shade, if self.is_dark() { 0.18 } else { 0.07 }), 1.),
            // Design v3-o S3: shaded toward the theme's own text color (lighter than the
            // terminal in dark, darker in light), so warm themes stay warm.
            bg_sidebar: hsla(sidebar, 1.),
            sidebar_hover: hsla(mix(sidebar, fg, t(0.06, 0.12)), 1.),
            sidebar_edge: hsla(fg, t(0.08, 0.45)),
            bg_terminal: hsla(bg, 1.),
            // Cards, popovers and offers sit on the sidebar (itself 5% toward the text in dark
            // themes), so they need a clear step above it to read as a separate surface.
            bg_raised: hsla(mix(bg, fg, if self.is_dark() { t(0.11, 0.16) } else { 0.0 }), 1.),
            bg_hover: hsla(mix(bg, fg, t(0.06, 0.1)), 1.),
            bg_selected: hsla(mix(bg, fg, t(0.11, 0.2)), 1.),
            bg_control: hsla(mix(bg, fg, t(0.16, 0.22)), 1.),
            border: hsla(fg, t(0.10, 0.45)),
            text_primary: hsla(fg, 1.),
            text_secondary: hsla(mix(bg, fg, t(0.72, 0.9)), 1.),
            text_tertiary: hsla(mix(bg, fg, t(0.52, 0.78)), 1.),
            text_quiet: hsla(mix(bg, fg, t(0.62, 0.84)), 1.),
            text_disabled: hsla(mix(bg, fg, t(0.36, 0.62)), 1.),
            accent: hsla(accent, 1.),
            on_accent: hsla(on(accent), 1.),
            link: hsla(self.link_hex(), 1.),
            status_working: hsla(mix(bg, fg, t(0.52, 0.78)), 1.),
            status_needs: hsla(self.ansi[3], 1.),
            status_done: hsla(self.ansi[2], 1.),
            status_error: hsla(self.ansi[1], 1.),
            icon_alpha: 1.,
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
/// Text color for a fill: near-black on light fills, white on dark ones (whichever contrasts
/// more).
pub fn on(fill: u32) -> u32 {
    let l = lum(fill);
    let (dark, light) = ((l + 0.05) / (lum(0x111317) + 0.05), 1.05 / (l + 0.05));
    if dark >= light { 0x111317 } else { 0xFFFFFF }
}

/// Text color for a fill given as Hsla.
pub fn on_hsla(fill: Hsla) -> Hsla {
    let c = fill.to_rgb();
    let v = ((c.r * 255.).round() as u32) << 16 | ((c.g * 255.).round() as u32) << 8 | (c.b * 255.).round() as u32;
    hsla(on(v), 1.)
}

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
    fn high_contrast_themes_meet_aaa() {
        let ratio = |a: u32, b: u32| {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        for name in ["High Contrast Dark", "High Contrast Light"] {
            let t = find(name).unwrap();
            assert!(t.is_high_contrast(), "{name}");
            // ANSI black is a background color on dark themes; everything else is text.
            let skip = if t.is_dark() { 1 } else { 0 };
            for (i, c) in t.ansi.iter().enumerate().skip(skip) {
                assert!(ratio(*c, t.bg) >= 7., "{name} ansi {i}: {:.1}", ratio(*c, t.bg));
            }
        }
        assert!(!find("Mercury Dark").unwrap().is_high_contrast());
        assert!(!find("Mercury Light").unwrap().is_high_contrast());
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
