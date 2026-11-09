//! A thin scrollbar for scrolling divs (the outputs panel), drawn over the list's right edge.
//! Shows only while the content is taller than the view; drag the thumb or click the track.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    canvas, point, px, size, AnyElement, Bounds, DispatchPhase, Hsla, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollHandle, Styled,
};

const INSET: f32 = 3.;
const WIDTH: f32 = 6.;
/// The pointer this close to the right edge grabs the bar.
const REACH: f32 = 12.;
const MIN_THUMB: f32 = 24.;

/// Drag state kept across frames: the grab point within the thumb while dragging.
#[derive(Clone, Default)]
pub struct Drag(Rc<Cell<Option<f32>>>);

/// Thumb (top, height) for a view `view_h` tall over content `max` taller, scrolled `scrolled`.
fn thumb(view_h: f32, max: f32, scrolled: f32) -> (f32, f32) {
    let track = (view_h - 2. * INSET).max(0.);
    let h = (track * view_h / (view_h + max)).clamp(MIN_THUMB.min(track), track);
    let top = INSET + (track - h) * (scrolled / max).clamp(0., 1.);
    (top, h)
}

/// The overlay, sized to fill a `relative()` parent that holds the scrolling list.
pub fn overlay(handle: &ScrollHandle, drag: &Drag, color: Hsla) -> AnyElement {
    let (handle, drag) = (handle.clone(), drag.clone());
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let max = f32::from(handle.max_offset().height);
            let view_h = f32::from(bounds.size.height);
            if max <= 0.5 || view_h <= 0. {
                drag.0.set(None);
                return;
            }
            let scrolled = -f32::from(handle.offset().y);
            let (top, h) = thumb(view_h, max, scrolled);
            let right = f32::from(bounds.right()) - INSET;
            let alpha = if drag.0.get().is_some() { 0.7 } else { 0.4 };
            window.paint_quad(
                gpui::fill(
                    Bounds::new(point(px(right - WIDTH), bounds.top() + px(top)), size(px(WIDTH), px(h))),
                    Hsla { a: color.a * alpha, ..color },
                )
                .corner_radii(px(WIDTH / 2.)),
            );

            // Offset that puts the thumb's top at window y `y` minus the grab point.
            let to_offset = move |y: Pixels, grab: f32| {
                let travel = (view_h - 2. * INSET - h).max(1.);
                let frac = ((f32::from(y - bounds.top()) - INSET - grab) / travel).clamp(0., 1.);
                point(px(0.), px(-frac * max))
            };
            {
                let (handle, drag) = (handle.clone(), drag.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Left || !bounds.contains(&e.position) {
                        return;
                    }
                    if f32::from(bounds.right() - e.position.x) > REACH {
                        return;
                    }
                    let y = f32::from(e.position.y - bounds.top());
                    let grab = if (top..=top + h).contains(&y) { y - top } else { h / 2. };
                    drag.0.set(Some(grab));
                    handle.set_offset(to_offset(e.position.y, grab));
                    cx.stop_propagation();
                    window.refresh();
                });
            }
            {
                let (handle, drag) = (handle.clone(), drag.clone());
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    if let Some(grab) = drag.0.get() {
                        if e.pressed_button != Some(MouseButton::Left) {
                            drag.0.set(None);
                        } else {
                            handle.set_offset(to_offset(e.position.y, grab));
                        }
                        window.refresh();
                    }
                });
            }
            let drag = drag.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, window, _| {
                if drag.0.take().is_some() {
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::thumb;

    #[test]
    fn thumb_spans_track() {
        // 400 view over 1200 content: a third of the track, at the top, then at the bottom.
        let (top, h) = thumb(400., 800., 0.);
        assert_eq!(top, 3.);
        assert!((h - 394. / 3.).abs() < 0.01);
        let (top, h) = thumb(400., 800., 800.);
        assert!((top + h - 397.).abs() < 0.01);
    }

    #[test]
    fn thumb_has_min_height() {
        assert_eq!(thumb(400., 100_000., 0.).1, 24.);
    }
}
