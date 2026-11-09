//! The selected-tab outline (canvas v3-as, off by default): one stroke in the group's color
//! around the selected row, up and down the sidebar's edge, and around the
//! terminal to the window's right edge. The row is the neck, the terminal the mouth.

use gpui::{canvas, point, px, AnyElement, Bounds, Hsla, IntoElement, PathBuilder, Pixels, Point, Styled, Window};

use crate::rail::Shown;
use crate::workspace::Workspace;

const STROKE: f32 = 1.5;
/// The selected row's concave fillets in the sidebar.
const R: f32 = 8.;
/// macOS window corner radius; square in full screen.
const WINDOW_R: f32 = 16.;

pub fn render(ws: &Workspace, shown: Shown, inactive: bool, window: &Window) -> Option<AnyElement> {
    if !ws.config.interface.selected_outline || ws.tabs.is_empty() {
        return None;
    }
    // Hidden sidebar: one terminal, nothing to point at. The rail has no row join to flow into,
    // so it gets the terminal edge alone.
    let (s, neck) = match shown {
        Shown::Full(w) => (w, true),
        Shown::Rail => (crate::rail::RAIL_W, false),
        Shown::Hidden => return None,
    };
    let tab = ws.tabs.get(ws.active)?;
    let base = ws.group_color(&tab.group).unwrap_or(ws.ui.accent);
    let mut a = if ws.theme.is_dark() { 0.8 } else { 0.9 };
    if inactive {
        a /= 2.;
    }
    let color = Hsla { a: base.a * a, ..base };
    let row = ws.selected_row.clone();
    let viewport = ws.sidebar_scroll.clone();
    let rw = if window.is_fullscreen() { 0. } else { WINDOW_R };
    Some(
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                // The row reports itself during prepaint, so this frame's position is known.
                let row = row.get().filter(|r| neck && half_visible(*r, viewport.bounds()));
                let rows = row.map(|r| {
                    (f32::from(r.left() - bounds.left()), f32::from(r.top() - bounds.top()), f32::from(r.bottom() - bounds.top()))
                });
                for p in paths(bounds, s, rows, rw) {
                    if let Ok(path) = p.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element(),
    )
}

/// At least half the row is inside the sidebar's scrolling list.
fn half_visible(row: Bounds<Pixels>, view: Bounds<Pixels>) -> bool {
    if view.size.height <= px(0.) {
        return true;
    }
    let top = row.top().max(view.top());
    let bottom = row.bottom().min(view.bottom());
    f32::from(bottom - top) * 2. >= f32::from(row.size.height)
}

/// The strokes, in window points relative to `b`: around the row and out to the window's right
/// edge when `row` is `(left, top, bottom)`, else the terminal's left edge alone.
fn paths(b: Bounds<Pixels>, s: f32, row: Option<(f32, f32, f32)>, rw: f32) -> Vec<PathBuilder> {
    let c = STROKE / 2.;
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let at = |x: f32, y: f32| -> Point<Pixels> { b.origin + point(px(x), px(y)) };
    let arc = |p: &mut PathBuilder, r: f32, sweep: bool, x: f32, y: f32| {
        p.arc_to(point(px(r), px(r)), px(0.), false, sweep, at(x, y));
    };
    // The window's top-right and bottom-right corners, from the top edge's end.
    let top_right = |p: &mut PathBuilder| {
        p.line_to(at(w - rw, c));
        if rw > 0. {
            arc(p, rw - c, true, w - c, rw);
        }
    };
    let bottom_right = |p: &mut PathBuilder| {
        p.line_to(at(w - rw, h - c));
        if rw > 0. {
            arc(p, rw - c, false, w - c, h - rw);
        }
    };
    let new = || PathBuilder::stroke(px(STROKE));
    match row {
        Some((x0, y0, y1)) => {
            // The row's left end, following its own rounding, so the outline closes around it.
            let neck_x = x0 + R;
            // Near the window's edge the corners shrink so the line still fits.
            let rt = R.min(((y0 - c) / 2.).max(0.));
            let rb = R.min(((h - y1 - c) / 2.).max(0.));
            let mut left = new();
            left.move_to(at(neck_x, y0 + c));
            arc(&mut left, R - c, false, x0 + c, y0 + R);
            left.line_to(at(x0 + c, y1 - R));
            arc(&mut left, R - c, false, neck_x, y1 - c);
            let mut top = new();
            top.move_to(at(neck_x, y0 + c));
            top.line_to(at(s - rt, y0 + c));
            arc(&mut top, rt + c, false, s + c, y0 - rt);
            top.line_to(at(s + c, c + rt));
            arc(&mut top, rt, true, s + c + rt, c);
            top_right(&mut top);
            let mut bottom = new();
            bottom.move_to(at(neck_x, y1 - c));
            bottom.line_to(at(s - rb, y1 - c));
            arc(&mut bottom, rb + c, true, s + c, y1 + rb);
            bottom.line_to(at(s + c, h - c - rb));
            arc(&mut bottom, rb, false, s + c + rb, h - c);
            bottom_right(&mut bottom);
            vec![left, top, bottom]
        }
        None => {
            let mut p = new();
            p.move_to(at(w - c, rw.max(c)));
            if rw > 0. {
                arc(&mut p, rw - c, false, w - rw, c);
            }
            p.line_to(at(s + c + R, c));
            arc(&mut p, R, false, s + c, c + R);
            p.line_to(at(s + c, h - c - R));
            arc(&mut p, R, false, s + c + R, h - c);
            bottom_right(&mut p);
            vec![p]
        }
    }
}
