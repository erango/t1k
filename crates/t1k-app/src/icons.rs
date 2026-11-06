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





/// Two sliders (view options).
pub fn sliders(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(2., 4.5));
            p.line_to(pt(12., 4.5));
            p.move_to(pt(2., 9.5));
            p.line_to(pt(12., 9.5));
        });
        let mut f = PathBuilder::fill();
        let knob = |f: &mut PathBuilder, cx: f32, cy: f32| {
            let pts: Vec<_> = (0..12).map(|i| {
                let a = i as f32 / 12. * std::f32::consts::TAU;
                pt(cx + 1.8 * a.cos(), cy + 1.8 * a.sin())
            }).collect();
            f.add_polygon(&pts, true);
        };
        knob(&mut f, 5., 4.5);
        knob(&mut f, 9.5, 9.5);
        if let Ok(path) = f.build() {
            window.paint_path(path, color);
        }
    })
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
