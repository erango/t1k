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

/// Circle with its left half filled (follow the system appearance).
pub fn half_circle(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let c = point(b.origin.x + px(7.), b.origin.y + px(7.));
        let r = 5.;
        let at = |a: f32| point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
        // Left half, filled.
        let mut f = PathBuilder::fill();
        let steps = 16;
        let half: Vec<_> = (0..=steps).map(|i| at(std::f32::consts::FRAC_PI_2 + i as f32 / steps as f32 * std::f32::consts::PI)).collect();
        f.add_polygon(&half, true);
        if let Ok(path) = f.build() {
            window.paint_path(path, color);
        }
        stroke(window, color, |p| {
            let ring: Vec<_> = (0..=32).map(|i| at(i as f32 / 32. * std::f32::consts::TAU)).collect();
            p.add_polygon(&ring, true);
        })
    })
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
