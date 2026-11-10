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
    // Title-bar icons draw at 16 px (14 read too small there).
    icon_at(16., |pt, p| {
            p.move_to(pt(2., 3.));
            p.line_to(pt(12., 3.));
            p.line_to(pt(12., 11.));
            p.line_to(pt(2., 11.));
            p.close();
            p.move_to(pt(5.5, 3.));
            p.line_to(pt(5.5, 11.));
        }, color)
}

/// Two right chevrons: fold a right-hand panel away.
pub fn collapse_right(color: Hsla) -> impl IntoElement {
    // Title-bar icons draw at 16 px (14 read too small there).
    icon_at(16., |pt, p| {
            p.move_to(pt(3.5, 3.5));
            p.line_to(pt(7., 7.));
            p.line_to(pt(3.5, 10.5));
            p.move_to(pt(7.5, 3.5));
            p.line_to(pt(11., 7.));
            p.line_to(pt(7.5, 10.5));
        }, color)
}

/// A tray (outputs).
pub fn tray(color: Hsla) -> impl IntoElement {
    // Title-bar icons draw at 16 px (14 read too small there).
    icon_at(16., |pt, p| {
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
        }, color)
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

/// A padlock (Secure Keyboard Entry, canvas v3-an).
pub fn lock(color: Hsla, size: f32) -> impl IntoElement {
    icon_at(size, |pt, p| {
        // Body: 3.25,6.25 7.5 × 5.5 (square corners read the same at this size).
        p.move_to(pt(3.25, 6.25));
        p.line_to(pt(10.75, 6.25));
        p.line_to(pt(10.75, 11.75));
        p.line_to(pt(3.25, 11.75));
        p.close();
        // Shackle: up from 5, round over to 9, back down.
        p.move_to(pt(5., 6.25));
        p.line_to(pt(5., 4.75));
        for k in 1..=12 {
            let a = std::f32::consts::PI * (1. - k as f32 / 12.);
            p.line_to(pt(7. + 2. * a.cos(), 4.75 - 2. * a.sin()));
        }
        p.line_to(pt(9., 6.25));
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
            // A merged PR: two branches joining (GitHub's merge mark, simplified).
            "merge" => {
                circle(p, 4., 2.75, 1.5);
                circle(p, 4., 11.25, 1.5);
                circle(p, 10.5, 8., 1.5);
                line(p, 4., 4.25, 4., 9.75);
                p.move_to(pt(4., 4.25));
                p.line_to(pt(4., 5.5));
                p.line_to(pt(7.5, 8.));
                p.line_to(pt(9., 8.));
            }
            // Copy a link: two chain links on a diagonal.
            "chain" => {
                let capsule = |p: &mut PathBuilder, cx: f32, cy: f32| {
                    // A 4 x 8 capsule turned 45°, centered at (cx, cy).
                    let (c, s) = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
                    let rot = |x: f32, y: f32| (cx + x * c - y * s, cy + x * s + y * c);
                    let mut pts = Vec::new();
                    for k in 0..=12 {
                        let a = std::f32::consts::PI + k as f32 / 12. * std::f32::consts::PI;
                        pts.push(rot(2.2 * a.cos(), -2. + 2.2 * a.sin()));
                    }
                    for k in 0..=12 {
                        let a = k as f32 / 12. * std::f32::consts::PI;
                        pts.push(rot(2.2 * a.cos(), 2. + 2.2 * a.sin()));
                    }
                    p.move_to(pt(pts[0].0, pts[0].1));
                    for &(x, y) in &pts[1..] {
                        p.line_to(pt(x, y));
                    }
                    p.close();
                };
                capsule(p, 5.2, 8.8);
                capsule(p, 8.8, 5.2);
            }
            // Copy a path: a folder.
            "folder" => {
                p.move_to(pt(1.5, 3.));
                p.line_to(pt(5.5, 3.));
                p.line_to(pt(7., 4.5));
                p.line_to(pt(12.5, 4.5));
                p.line_to(pt(12.5, 11.5));
                p.line_to(pt(1.5, 11.5));
                p.close();
            }
            // Ticket trackers (monochrome marks, like the file icons).
            "t-linear" => {
                circle(p, 7., 7., 5.5);
                line(p, 2.6, 8.8, 5.2, 11.4);
                line(p, 1.9, 6.1, 7.9, 12.1);
                line(p, 2.5, 3.7, 10.3, 11.5);
            }
            "t-jira" => {
                // Two nested diamonds, the Jira mark's chevrons simplified.
                p.move_to(pt(7., 1.5));
                p.line_to(pt(12.5, 7.));
                p.line_to(pt(7., 12.5));
                p.line_to(pt(1.5, 7.));
                p.close();
                p.move_to(pt(7., 4.5));
                p.line_to(pt(9.5, 7.));
                p.line_to(pt(7., 9.5));
                p.line_to(pt(4.5, 7.));
                p.close();
            }
            // A fox head: GitLab's mark as one outline.
            "t-gitlab" => {
                p.move_to(pt(7., 12.5));
                p.line_to(pt(1.5, 8.));
                p.line_to(pt(3., 2.5));
                p.line_to(pt(4.75, 7.));
                p.line_to(pt(9.25, 7.));
                p.line_to(pt(11., 2.5));
                p.line_to(pt(12.5, 8.));
                p.close();
            }
            // Shortcut: a tilted flag.
            "t-shortcut" => {
                p.move_to(pt(2., 9.));
                p.line_to(pt(8., 2.));
                p.line_to(pt(12., 5.5));
                p.line_to(pt(6., 12.5));
                p.close();
                line(p, 5., 5.5, 9., 9.);
            }
            // Asana: three dots.
            "t-asana" => {
                circle(p, 7., 4., 2.2);
                circle(p, 3.6, 9.8, 2.2);
                circle(p, 10.4, 9.8, 2.2);
            }
            // ClickUp: a chevron over an arc.
            "t-clickup" => {
                p.move_to(pt(3., 6.5));
                p.line_to(pt(7., 2.5));
                p.line_to(pt(11., 6.5));
                p.move_to(pt(2.5, 9.5));
                p.line_to(pt(4.75, 11.5));
                p.line_to(pt(7., 12.25));
                p.line_to(pt(9.25, 11.5));
                p.line_to(pt(11.5, 9.5));
            }
            // Trello: a board with two lists.
            "t-trello" => {
                rect(p, 1.5, 1.5, 12.5, 12.5);
                rect(p, 3.5, 3.5, 6., 10.5);
                rect(p, 8., 3.5, 10.5, 7.5);
            }
            // Azure Boards: a card with a tick.
            "t-azure" => {
                rect(p, 1.5, 2.5, 12.5, 11.5);
                p.move_to(pt(4., 7.25));
                p.line_to(pt(6.25, 9.5));
                p.line_to(pt(10., 5.));
            }
            // YouTrack: a ring cut by a slash.
            "t-youtrack" => {
                circle(p, 7., 7., 5.5);
                line(p, 4., 10., 10., 4.);
                line(p, 4., 4., 7., 7.);
            }
            // Sentry: a peak with an arc inside.
            "t-sentry" => {
                p.move_to(pt(1.5, 11.5));
                p.line_to(pt(7., 2.));
                p.line_to(pt(12.5, 11.5));
                p.line_to(pt(9.5, 11.5));
                p.move_to(pt(3.5, 11.5));
                p.line_to(pt(5.5, 11.5));
                p.line_to(pt(5.75, 9.5));
                p.line_to(pt(7., 7.5));
            }
            "t-github" => {
                // GitHub's open-issue mark: a ring with a dot.
                circle(p, 7., 7., 5.5);
                circle(p, 7., 7., 1.2);
            }
            "closed" => {
                line(p, 3.5, 3.5, 10.5, 10.5);
                line(p, 10.5, 3.5, 3.5, 10.5);
            }
            "chart" => {
                line(p, 2., 2., 2., 12.);
                line(p, 2., 12., 12.5, 12.);
                line(p, 5., 12., 5., 8.);
                line(p, 8., 12., 8., 5.);
                line(p, 11., 12., 11., 3.);
            }
            // A frame with a play mark: a Motion artifact (animation, video).
            "motion" => {
                rect(p, 1.5, 2.5, 12.5, 11.5);
                p.move_to(pt(5.75, 4.75));
                p.line_to(pt(9.5, 7.));
                p.line_to(pt(5.75, 9.25));
                p.close();
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
            // v3-am: tables and dashboards, then files (the page plus one mark inside it).
            "table" => {
                rect(p, 1.5, 2.5, 12.5, 11.5);
                line(p, 1.5, 5.5, 12.5, 5.5);
                line(p, 1.5, 8.5, 12.5, 8.5);
                line(p, 5.5, 2.5, 5.5, 11.5);
            }
            "dashboard" => {
                rect(p, 1.5, 1.5, 6., 7.5);
                rect(p, 8., 1.5, 12.5, 5.);
                rect(p, 8., 6.5, 12.5, 12.5);
                rect(p, 1.5, 9., 6., 12.5);
            }
            // Context pane (canvas v3-au): tool glyphs, server marks, the info icon.
            "x-terminal" => {
                rect(p, 1.5, 2.5, 12.5, 11.5);
                p.move_to(pt(3.75, 5.));
                p.line_to(pt(5.75, 7.));
                p.line_to(pt(3.75, 9.));
                line(p, 7., 9., 10., 9.);
            }
            "x-pencil" => {
                p.move_to(pt(2., 12.));
                p.line_to(pt(2.5, 9.5));
                p.line_to(pt(9.5, 2.5));
                p.line_to(pt(11.5, 4.5));
                p.line_to(pt(4.5, 11.5));
                p.close();
                line(p, 8., 4., 10., 6.);
            }
            "x-doc-plus" => {
                page(p);
                line(p, 7., 6.5, 7., 10.5);
                line(p, 5., 8.5, 9., 8.5);
            }
            "x-search" => {
                circle(p, 6., 6., 4.);
                line(p, 9., 9., 12.5, 12.5);
            }
            "x-globe" => {
                circle(p, 7., 7., 5.5);
                line(p, 1.5, 7., 12.5, 7.);
                p.move_to(pt(7., 1.5));
                p.line_to(pt(5., 4.));
                p.line_to(pt(4.5, 7.));
                p.line_to(pt(5., 10.));
                p.line_to(pt(7., 12.5));
                p.line_to(pt(9., 10.));
                p.line_to(pt(9.5, 7.));
                p.line_to(pt(9., 4.));
                p.close();
            }
            "x-agent" => {
                circle(p, 5., 4.5, 2.);
                p.move_to(pt(1.5, 12.));
                p.line_to(pt(2., 9.5));
                p.line_to(pt(3.5, 8.));
                p.line_to(pt(6.5, 8.));
                p.line_to(pt(8., 9.5));
                p.line_to(pt(8.5, 12.));
                circle(p, 11.5, 4., 1.);
                circle(p, 11.5, 9., 1.);
                line(p, 7., 5., 10.5, 4.);
                line(p, 8.5, 7., 10.5, 8.5);
            }
            "x-check" => {
                p.move_to(pt(1.5, 3.5));
                p.line_to(pt(2.75, 4.75));
                p.line_to(pt(4.75, 2.25));
                p.move_to(pt(1.5, 9.));
                p.line_to(pt(2.75, 10.25));
                p.line_to(pt(4.75, 7.75));
                line(p, 6.5, 3.5, 12.5, 3.5);
                line(p, 6.5, 9., 12.5, 9.);
            }
            "x-wrench" => {
                p.move_to(pt(2., 12.));
                p.line_to(pt(7., 7.));
                p.move_to(pt(7., 7.));
                p.line_to(pt(6.5, 5.));
                p.line_to(pt(7.5, 2.75));
                p.line_to(pt(9.75, 1.75));
                p.line_to(pt(9., 4.25));
                p.line_to(pt(9.75, 5.));
                p.line_to(pt(12.25, 4.25));
                p.line_to(pt(11.25, 6.5));
                p.line_to(pt(9., 7.5));
                p.line_to(pt(7., 7.));
            }
            "x-plug" => {
                line(p, 5., 1.5, 5., 4.5);
                line(p, 9., 1.5, 9., 4.5);
                p.move_to(pt(3., 4.5));
                p.line_to(pt(11., 4.5));
                p.line_to(pt(11., 7.));
                p.line_to(pt(9., 9.5));
                p.line_to(pt(5., 9.5));
                p.line_to(pt(3., 7.));
                p.close();
                line(p, 7., 9.5, 7., 12.5);
            }
            "x-chrome" => {
                circle(p, 7., 7., 5.5);
                circle(p, 7., 7., 2.25);
                line(p, 7., 4.75, 11.75, 4.25);
                line(p, 5.05, 8.1, 2.6, 4.);
                line(p, 8.95, 8.1, 6.25, 12.45);
            }
            // Broadcast input (canvas v3-aw): a dot with a wave each side; "-off" struck through.
            b if b.starts_with("x-broadcast") => {
                circle(p, 7., 7., 1.2);
                let arc = |p: &mut PathBuilder, r: f32, side: f32| {
                    for k in 0..=8 {
                        let a = (-0.9 + 1.8 * k as f32 / 8.) + if side < 0. { std::f32::consts::PI } else { 0. };
                        let q = pt(7. + r * a.cos(), 7. + r * a.sin());
                        if k == 0 { p.move_to(q) } else { p.line_to(q) }
                    }
                };
                arc(p, 3.4, 1.);
                arc(p, 3.4, -1.);
                arc(p, 5.8, 1.);
                arc(p, 5.8, -1.);
                if b.ends_with("-off") {
                    line(p, 2., 12., 12., 2.);
                }
            }
            "x-info" => {
                circle(p, 7., 7., 5.5);
                line(p, 7., 6.25, 7., 10.);
                line(p, 7., 4., 7., 4.6);
            }
            f if f.starts_with("f-") => {
                page(p);
                let poly = |p: &mut PathBuilder, pts: &[(f32, f32)]| {
                    p.move_to(pt(pts[0].0, pts[0].1));
                    for &(x, y) in &pts[1..] {
                        p.line_to(pt(x, y));
                    }
                };
                match f {
                    "f-code" => {
                        poly(p, &[(6., 6.75), (4.75, 8.5), (6., 10.25)]);
                        poly(p, &[(8., 6.75), (9.25, 8.5), (8., 10.25)]);
                    }
                    "f-data" => {
                        poly(p, &[(6.25, 6.5), (5.5, 6.75), (5.5, 8.), (4.75, 8.5), (5.5, 9.), (5.5, 10.25), (6.25, 10.5)]);
                        poly(p, &[(7.75, 6.5), (8.5, 6.75), (8.5, 8.), (9.25, 8.5), (8.5, 9.), (8.5, 10.25), (7.75, 10.5)]);
                    }
                    "f-web" => {
                        poly(p, &[(5.75, 7.), (4.5, 8.75), (5.75, 10.5)]);
                        poly(p, &[(8.25, 7.), (9.5, 8.75), (8.25, 10.5)]);
                        line(p, 7.5, 6.75, 6.5, 10.75);
                    }
                    "f-markdown" => {
                        line(p, 6., 6.5, 5.5, 10.75);
                        line(p, 8.25, 6.5, 7.75, 10.75);
                        line(p, 4.75, 7.9, 9.25, 7.9);
                        line(p, 4.5, 9.4, 9., 9.4);
                    }
                    "f-text" => {
                        line(p, 5., 7., 9., 7.);
                        line(p, 5., 8.75, 9., 8.75);
                        line(p, 5., 10.5, 7.5, 10.5);
                    }
                    "f-table" => {
                        rect(p, 4.5, 6.5, 9.5, 11.);
                        line(p, 4.5, 8.5, 9.5, 8.5);
                        line(p, 7., 6.5, 7., 11.);
                    }
                    "f-image" => {
                        circle(p, 6., 7.25, 0.75);
                        poly(p, &[(4.5, 11.), (6.25, 9.), (7.25, 10.), (8.25, 8.5), (9.5, 11.)]);
                        p.close();
                    }
                    // A solid band: two strokes 0.8 apart cover y 8 to 10.
                    "f-pdf" => {
                        line(p, 4.5, 8.6, 9.5, 8.6);
                        line(p, 4.5, 9.4, 9.5, 9.4);
                    }
                    "f-archive" => {
                        line(p, 6.25, 2.75, 7., 2.75);
                        line(p, 7., 4., 7.75, 4.);
                        line(p, 6.25, 5.25, 7., 5.25);
                        line(p, 7., 6.5, 7.75, 6.5);
                        rect(p, 6.25, 7.75, 7.75, 10.25);
                    }
                    "f-shell" => {
                        poly(p, &[(5., 7.25), (6.5, 8.5), (5., 9.75)]);
                        line(p, 7.25, 10., 9.25, 10.);
                    }
                    "f-config" => {
                        line(p, 4.75, 7.5, 9.25, 7.5);
                        line(p, 4.75, 10., 9.25, 10.);
                        line(p, 6., 6.75, 6., 8.25);
                        line(p, 8., 9.25, 8., 10.75);
                    }
                    "f-lock" => {
                        rect(p, 5.25, 8.5, 8.75, 11.);
                        poly(p, &[(6., 8.5), (6., 7.4), (6.3, 6.85), (7., 6.6), (7.7, 6.85), (8., 7.4), (8., 8.5)]);
                    }
                    _ => {}
                }
            }
            _ => page(p),
        }
    }, color)
}

/// The mark of an agent CLI that has no Mac app icon to borrow (Codex, Gemini, Antigravity,
/// Grok), drawn bare at `size` px so it reads as large as Claude's app icon. None for others.
pub fn agent_mark(program: &str, size: f32) -> Option<gpui::AnyElement> {
    use gpui::rgb;
    let p = program.trim_start_matches('-').to_ascii_lowercase();
    let kind = match p.as_str() {
        "codex" => 0,
        "gemini" => 1,
        "agy" | "antigravity" => 2,
        "grok" => 3,
        _ => return None,
    };
    let color: Hsla = match kind {
        0 => rgb(0x10A37F).into(),
        1 | 2 => rgb(0x4C8BF5).into(),
        _ => rgb(0x9AA0A8).into(),
    };
    Some(
        canvas(|_, _, _| {}, move |b, _, window, _| {
            // Drawn in a 24-unit space, like the landing page's SVGs.
            let k = size / 24.;
            let o = b.origin;
            let pt = move |x: f32, y: f32| point(o.x + px(x * k), o.y + px(y * k));
            let mut paint = |mut p: PathBuilder, build: &dyn Fn(&mut PathBuilder)| {
                build(&mut p);
                if let Ok(path) = p.build() {
                    window.paint_path(path, color);
                }
            };
            let circle = |p: &mut PathBuilder, r: f32| {
                let pts: Vec<_> = (0..=40).map(|i| {
                    let a = i as f32 / 40. * std::f32::consts::TAU;
                    pt(12. + r * a.cos(), 12. + r * a.sin())
                }).collect();
                p.add_polygon(&pts, true);
            };
            match kind {
                // Codex: a ring around a dot.
                0 => {
                    paint(PathBuilder::stroke(px(2.4 * k)), &|p| circle(p, 9.5));
                    paint(PathBuilder::fill(), &|p| circle(p, 4.2));
                }
                // Gemini: a four-pointed star with curved sides.
                1 => paint(PathBuilder::fill(), &|p| {
                    p.move_to(pt(12., 1.5));
                    p.curve_to(pt(22.5, 12.), pt(13.4, 10.6));
                    p.curve_to(pt(12., 22.5), pt(13.4, 13.4));
                    p.curve_to(pt(1.5, 12.), pt(10.6, 13.4));
                    p.curve_to(pt(12., 1.5), pt(10.6, 10.6));
                    p.close();
                }),
                // Antigravity: a triangle, its left half filled.
                2 => {
                    paint(PathBuilder::stroke(px(2.2 * k)), &|p| p.add_polygon(&[pt(12., 2.5), pt(22., 20.5), pt(2., 20.5)], true));
                    paint(PathBuilder::fill(), &|p| p.add_polygon(&[pt(12., 2.5), pt(12., 20.5), pt(2., 20.5)], true));
                }
                // Grok: an X.
                _ => paint(PathBuilder::stroke(px(2.6 * k)), &|p| {
                    p.move_to(pt(4., 3.5));
                    p.line_to(pt(20., 20.5));
                    p.move_to(pt(20., 3.5));
                    p.line_to(pt(4., 20.5));
                }),
            }
        })
        .size(px(size))
        .flex_none()
        .into_any_element(),
    )
}
