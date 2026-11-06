//! Color palette used to resolve terminal colors to RGB.
//!
//! This is a minimal stand-in until the `t1k-themes` crate lands (M3).

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[derive(Clone, Debug)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection_bg: Rgb,
    pub selection_fg: Option<Rgb>,
    /// ANSI 0-15.
    pub ansi: [Rgb; 16],
}

const fn hex(v: u32) -> Rgb {
    Rgb { r: ((v >> 16) & 0xff) as u8, g: ((v >> 8) & 0xff) as u8, b: (v & 0xff) as u8 }
}

impl Default for Palette {
    fn default() -> Self {
        // Neutral dark default; replaced by imported themes later.
        Self {
            foreground: hex(0xd4d7dd),
            background: hex(0x14161a),
            cursor: hex(0xc8d3e0),
            cursor_text: hex(0x14161a),
            selection_bg: hex(0x34404f),
            selection_fg: None,
            ansi: [
                hex(0x1d2026), hex(0xe06c75), hex(0x98c379), hex(0xe5c07b),
                hex(0x61afef), hex(0xc678dd), hex(0x56b6c2), hex(0xabb2bf),
                hex(0x5c6370), hex(0xf27c86), hex(0xb5e08f), hex(0xf2d08f),
                hex(0x7dc1ff), hex(0xd99af0), hex(0x6fd0db), hex(0xe6e9ef),
            ],
        }
    }
}

impl Palette {
    /// Resolve an indexed color 0-255 (16 ANSI + 6x6x6 cube + 24 grays).
    pub fn indexed(&self, idx: u8) -> Rgb {
        match idx {
            0..=15 => self.ansi[idx as usize],
            16..=231 => {
                let i = idx - 16;
                let step = |v: u8| if v == 0 { 0 } else { v * 40 + 55 };
                Rgb { r: step(i / 36), g: step((i / 6) % 6), b: step(i % 6) }
            }
            232..=255 => {
                let v = (idx - 232) * 10 + 8;
                Rgb { r: v, g: v, b: v }
            }
        }
    }

    fn named(&self, n: NamedColor) -> Rgb {
        use NamedColor as N;
        match n {
            N::Foreground | N::BrightForeground => self.foreground,
            N::Background => self.background,
            N::Cursor => self.cursor,
            N::DimForeground => dim(self.foreground),
            N::DimBlack => dim(self.ansi[0]),
            N::DimRed => dim(self.ansi[1]),
            N::DimGreen => dim(self.ansi[2]),
            N::DimYellow => dim(self.ansi[3]),
            N::DimBlue => dim(self.ansi[4]),
            N::DimMagenta => dim(self.ansi[5]),
            N::DimCyan => dim(self.ansi[6]),
            N::DimWhite => dim(self.ansi[7]),
            other => self.ansi[(other as usize).min(15)],
        }
    }

    /// Resolve a cell color, honoring runtime overrides set by programs via OSC 4/10/11.
    pub fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(i) => overrides[i as usize].unwrap_or_else(|| self.indexed(i)),
            Color::Named(n) => overrides[n].unwrap_or_else(|| self.named(n)),
        }
    }
}

pub fn dim(c: Rgb) -> Rgb {
    Rgb { r: (c.r as u16 * 2 / 3) as u8, g: (c.g as u16 * 2 / 3) as u8, b: (c.b as u16 * 2 / 3) as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grays() {
        let p = Palette::default();
        assert_eq!(p.indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(p.indexed(231), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(p.indexed(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(p.indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(p.indexed(255), Rgb { r: 238, g: 238, b: 238 });
    }
}
