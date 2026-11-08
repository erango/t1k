//! A Claude session's `/color` on its tab (canvas v3-ad). The user picks how it shows
//! (`[sidebar] session_color`): a swatch before the title (D, the default), a triangle in the
//! icon's top-left corner (A), a row tint (B), or the group line beside the row (C). It never
//! means attention: status keeps its own corner and words.

use gpui::{canvas, div, point, prelude::*, px, Hsla, PathBuilder};
use t1k_themes::Theme;

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let m = |s: u32| {
        let (x, y) = (((a >> s) & 0xff) as f32, ((b >> s) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << s
    };
    m(16) | m(8) | m(0)
}

/// The theme's color for a Claude color name: its ANSI hues, with orange and pink derived.
pub fn color(name: &str, theme: &Theme) -> Option<Hsla> {
    let a = &theme.ansi;
    let dark = theme.is_dark();
    let c = match name {
        "red" => a[1],
        "green" => a[2],
        "yellow" => a[3],
        "blue" => a[4],
        "purple" | "magenta" => a[5],
        "cyan" => a[6],
        "orange" => mix(a[1], a[3], 0.5),
        "pink" => {
            if dark {
                mix(a[1], 0xffffff, 0.4)
            } else {
                mix(a[1], a[5], 0.4)
            }
        }
        _ => return None,
    };
    Some(gpui::rgb(c).into())
}

/// A: a right triangle in the icon's top-left corner, 42% of the icon.
pub fn corner(c: Hsla, icon: f32) -> impl IntoElement {
    let s = (icon * 0.42).round();
    canvas(|_, _, _| {}, move |b, _, window, _| {
        let o = b.origin;
        let mut p = PathBuilder::fill();
        p.move_to(o);
        p.line_to(point(o.x + px(s), o.y));
        p.line_to(point(o.x, o.y + px(s)));
        p.close();
        if let Ok(path) = p.build() {
            window.paint_path(path, c);
        }
    })
    .absolute()
    .top_0()
    .left_0()
    .size(px(s))
}

/// D: a 3×14 bar before the title.
pub fn swatch(c: Hsla) -> gpui::Div {
    div().w(px(3.)).h(px(14.)).flex_none().rounded(px(1.5)).bg(c)
}
