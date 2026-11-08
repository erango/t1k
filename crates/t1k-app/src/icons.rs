//! Small stroked icons drawn with paths (no icon font or asset files needed).

use gpui::{canvas, point, prelude::*, px, Bounds, Hsla, PathBuilder, Pixels, Window};

fn icon(paint: impl Fn(Bounds<Pixels>, &mut Window) + 'static) -> impl IntoElement {
    canvas(|_, _, _| {}, move |b, _, window, _| paint(b, window)).size(px(14.)).flex_none()
}

/// An icon drawn in the 14-unit space, scaled to `size` px (stroke scales with it).
fn icon_at(size: f32, paint: impl Fn(&dyn Fn(f32, f32) -> gpui::Point<Pixels>, &mut PathBuilder) + 'static, color: Hsla) -> impl IntoElement {
    canvas(|_, _, _| {}, move |b, _, window, _| {
        let k = size / 14.;
        let o = b.origin;
        let pt = move |x: f32, y: f32| point(o.x + px(x * k), o.y + px(y * k));
        let mut p = PathBuilder::stroke(px(1.2 * k.sqrt()));
        paint(&pt, &mut p);
        if let Ok(path) = p.build() {
            window.paint_path(path, color);
        }
    })
    .size(px(size))
    .flex_none()
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

/// Sliders (view options: how the list shows).
pub fn sliders_at(color: Hsla, size: f32) -> impl IntoElement {
    // Three tracks, each broken by a hollow knob: "adjust how this looks", not "filter".
    icon_at(size, |pt, p| {
        for (y, x) in [(3.5f32, 9.0f32), (7., 5.), (10.5, 8.)] {
            let r = 1.5;
            p.move_to(pt(1.5, y));
            p.line_to(pt(x - r, y));
            p.move_to(pt(x + r, y));
            p.line_to(pt(12.5, y));
            let n = 16;
            p.move_to(pt(x + r, y));
            for k in 1..=n {
                let a = k as f32 / n as f32 * std::f32::consts::TAU;
                p.line_to(pt(x + r * a.cos(), y + r * a.sin()));
            }
        }
    }, color)
}

/// A small filled caret: pointing down (open) or right (folded), like a disclosure triangle.
pub fn caret(color: Hsla, open: bool) -> impl IntoElement {
    canvas(|_, _, _| {}, move |b, _, window, _| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        let mut p = PathBuilder::fill();
        if open {
            p.move_to(pt(1., 2.5));
            p.line_to(pt(7., 2.5));
            p.line_to(pt(4., 6.));
        } else {
            p.move_to(pt(2.5, 1.));
            p.line_to(pt(6., 4.));
            p.line_to(pt(2.5, 7.));
        }
        p.close();
        if let Ok(path) = p.build() {
            window.paint_path(path, color);
        }
    })
    .size(px(8.))
    .flex_none()
}

/// A window with its left panel marked: show the sidebar.
pub fn sidebar(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(2., 3.));
            p.line_to(pt(12., 3.));
            p.line_to(pt(12., 11.));
            p.line_to(pt(2., 11.));
            p.close();
            p.move_to(pt(5.5, 3.));
            p.line_to(pt(5.5, 11.));
        })
    })
}

/// Two right chevrons: fold a right-hand panel away.
pub fn collapse_right(color: Hsla) -> impl IntoElement {
    icon(move |b, window| {
        let o = b.origin;
        let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
        stroke(window, color, |p| {
            p.move_to(pt(3.5, 3.5));
            p.line_to(pt(7., 7.));
            p.line_to(pt(3.5, 10.5));
            p.move_to(pt(7.5, 3.5));
            p.line_to(pt(11., 7.));
            p.line_to(pt(7.5, 10.5));
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

/// A small chevron, centered in the 14 px box (history back / forward).
pub fn chevron_at(color: Hsla, left: bool, size: f32) -> impl IntoElement {
    let x = move |v: f32| if left { v } else { 14. - v };
    icon_at(size, move |pt, p| {
        p.move_to(pt(x(8.5), 3.5));
        p.line_to(pt(x(5.), 7.));
        p.line_to(pt(x(8.5), 10.5));
    }, color)
}

/// A plus (new tab).
pub fn plus(color: Hsla) -> impl IntoElement {
    plus_at(color, 14.)
}

pub fn plus_at(color: Hsla, size: f32) -> impl IntoElement {
    icon_at(size, |pt, p| {
        p.move_to(pt(7., 2.5));
        p.line_to(pt(7., 11.5));
        p.move_to(pt(2.5, 7.));
        p.line_to(pt(11.5, 7.));
    }, color)
}

/// Artifact type icons (canvas v3-ae), on the 14-unit grid like the rest, drawn at 16 px.
pub fn artifact(kind: &str, color: Hsla, size: f32) -> impl IntoElement {
    let kind = kind.to_string();
    icon_at(size, move |pt, p| {
        let rect = |p: &mut PathBuilder, x0: f32, y0: f32, x1: f32, y1: f32| {
            p.move_to(pt(x0, y0));
            p.line_to(pt(x1, y0));
            p.line_to(pt(x1, y1));
            p.line_to(pt(x0, y1));
            p.close();
        };
        let line = |p: &mut PathBuilder, x0: f32, y0: f32, x1: f32, y1: f32| {
            p.move_to(pt(x0, y0));
            p.line_to(pt(x1, y1));
        };
        // A page with a folded corner (also the base of "doc" and "link").
        let page = |p: &mut PathBuilder| {
            p.move_to(pt(3., 1.5));
            p.line_to(pt(8.5, 1.5));
            p.line_to(pt(11., 4.));
            p.line_to(pt(11., 12.5));
            p.line_to(pt(3., 12.5));
            p.close();
            p.move_to(pt(8.5, 1.5));
            p.line_to(pt(8.5, 4.));
            p.line_to(pt(11., 4.));
        };
        let circle = |p: &mut PathBuilder, cx: f32, cy: f32, r: f32| {
            p.move_to(pt(cx + r, cy));
            for k in 1..=20 {
                let a = k as f32 / 20. * std::f32::consts::TAU;
                p.line_to(pt(cx + r * a.cos(), cy + r * a.sin()));
            }
        };
        match kind.as_str() {
            "chart" => {
                line(p, 2., 2., 2., 12.);
                line(p, 2., 12., 12.5, 12.);
                line(p, 5., 12., 5., 8.);
                line(p, 8., 12., 8., 5.);
                line(p, 11., 12., 11., 3.);
            }
            "slides" => {
                rect(p, 1.5, 2.5, 12.5, 9.5);
                line(p, 7., 9.5, 7., 12.);
                line(p, 4.5, 12., 9.5, 12.);
            }
            "doc" => {
                page(p);
                line(p, 5., 7., 9., 7.);
                line(p, 5., 9.5, 9., 9.5);
            }
            "design" => {
                rect(p, 1.5, 5., 8.5, 12.);
                circle(p, 9., 5., 3.5);
            }
            "code" => {
                line(p, 5., 4., 2., 7.);
                line(p, 2., 7., 5., 10.);
                line(p, 9., 4., 12., 7.);
                line(p, 12., 7., 9., 10.);
            }
            "map" => {
                p.move_to(pt(1.5, 3.5));
                p.line_to(pt(5., 2.));
                p.line_to(pt(9., 3.5));
                p.line_to(pt(12.5, 2.));
                p.line_to(pt(12.5, 10.5));
                p.line_to(pt(9., 12.));
                p.line_to(pt(5., 10.5));
                p.line_to(pt(1.5, 12.));
                p.close();
                line(p, 5., 2., 5., 10.5);
                line(p, 9., 3.5, 9., 12.);
            }
            "calendar" => {
                rect(p, 1.5, 3., 12.5, 12.5);
                line(p, 1.5, 6., 12.5, 6.);
                line(p, 4.5, 1.5, 4.5, 4.);
                line(p, 9.5, 1.5, 9.5, 4.);
            }
            "game" => {
                p.move_to(pt(3.5, 4.));
                p.line_to(pt(10.5, 4.));
                p.line_to(pt(12.5, 10.));
                p.line_to(pt(10.5, 11.));
                p.line_to(pt(9., 9.));
                p.line_to(pt(5., 9.));
                p.line_to(pt(3.5, 11.));
                p.line_to(pt(1.5, 10.));
                p.close();
                line(p, 4.5, 5.5, 4.5, 7.5);
                line(p, 3.5, 6.5, 5.5, 6.5);
                circle(p, 9.5, 6.5, 0.6);
            }
            "link" => {
                page(p);
                line(p, 5., 10., 8.5, 6.5);
                line(p, 6., 6.5, 8.5, 6.5);
                line(p, 8.5, 6.5, 8.5, 9.);
            }
            _ => page(p),
        }
    }, color)
}
