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






/// Magnifier (find).
pub fn search(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        stroke(window, color, |p| {
            let c = (6., 6.);
            let pts: Vec<_> = (0..=24).map(|i| {
                let a = i as f32 / 24. * std::f32::consts::TAU;
                point(o.x + px(c.0 + 4. * a.cos()), o.y + px(c.1 + 4. * a.sin()))
            }).collect();
            p.add_polygon(&pts, true);
            p.move_to(point(o.x + px(9.), o.y + px(9.)));
            p.line_to(point(o.x + px(12.5), o.y + px(12.5)));
        })
    })
}

/// Three shortening lines (view options: how the list shows).
pub fn filter_lines(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(2.5, 4.));
            p.line_to(pt(11.5, 4.));
            p.move_to(pt(4.5, 7.));
            p.line_to(pt(9.5, 7.));
            p.move_to(pt(6.3, 10.));
            p.line_to(pt(7.7, 10.));
        })
    })
}

/// A tray (outputs).
pub fn tray(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(3.5, 3.));
            p.line_to(pt(10.5, 3.));
            p.line_to(pt(12.2, 8.));
            p.line_to(pt(12.2, 11.5));
            p.line_to(pt(1.8, 11.5));
            p.line_to(pt(1.8, 8.));
            p.line_to(pt(3.5, 3.));
            p.move_to(pt(1.8, 8.));
            p.line_to(pt(4.8, 8.));
            p.line_to(pt(5.6, 9.6));
            p.line_to(pt(8.4, 9.6));
            p.line_to(pt(9.2, 8.));
            p.line_to(pt(12.2, 8.));
        })
    })
}

/// An ×, for closing a dialog.
pub fn close(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(3.5, 3.5));
            p.line_to(pt(10.5, 10.5));
            p.move_to(pt(10.5, 3.5));
            p.line_to(pt(3.5, 10.5));
        })
    })
}

/// A left chevron, for going back.
pub fn back(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(8.5, 3.));
            p.line_to(pt(4.5, 7.));
            p.line_to(pt(8.5, 11.));
        })
    })
}

/// A plus (new tab).
pub fn plus(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(7., 2.5));
            p.line_to(pt(7., 11.5));
            p.move_to(pt(2.5, 7.));
            p.line_to(pt(11.5, 7.));
        })
    })
}
