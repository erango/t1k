//! Small stroked icons drawn with paths (no icon font or asset files needed).

use gpui::{canvas, point, prelude::*, px, Bounds, Hsla, PathBuilder, Pixels, Window};

fn icon(paint: impl Fn(Bounds<Pixels>, &mut Window) + 'static) -> impl IntoElement {
    canvas(|_, _, _| {}, move |b, _, window, _| paint(b, window)).size(px(14.)).flex_none()
}

fn stroke(window: &mut Window, color: Hsla, build: impl FnOnce(&mut PathBuilder)) {
    let mut p = PathBuilder::stroke(px(1.2));
    build(&mut p);
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
}

fn rect(p: &mut PathBuilder, o: gpui::Point<Pixels>, x: f32, y: f32, w: f32, h: f32) {
    let pt = |a: f32, b: f32| point(o.x + px(a), o.y + px(b));
    p.add_polygon(&[pt(x, y), pt(x + w, y), pt(x + w, y + h), pt(x, y + h)], true);
}

/// 2×2 grid (Overview).
pub fn grid(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        stroke(window, color, |p| {
            for (x, y) in [(2., 2.), (8., 2.), (2., 8.), (8., 8.)] {
                rect(p, b.origin, x, y, 4., 4.);
            }
        })
    })
}

/// Window with a right-hand panel (Outputs); the panel fills when open.
pub fn panel_right(color: Hsla, open: bool) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        if open {
            let mut f = PathBuilder::fill();
            rect(&mut f, o, 8.5, 2.5, 3., 9.);
            if let Ok(path) = f.build() {
                window.paint_path(path, color.opacity(0.5));
            }
        }
        stroke(window, color, |p| {
            rect(p, o, 1.5, 2., 11., 10.);
            p.move_to(point(o.x + px(8.5), o.y + px(2.)));
            p.line_to(point(o.x + px(8.5), o.y + px(12.)));
        })
    })
}
