//! The overlay scrollbar and the scroll-to-bottom button (canvas v3-al, option A).
//!
//! Geometry, fades and painting live here; `view.rs` owns the state machine (when they show)
//! and the input.

use std::time::{Duration, Instant};

use gpui::{fill, point, px, size, Bounds, Hsla, Pixels, Point, Window};

/// The scrollbar fades out this long after the last scroll or edge hover.
pub const IDLE: Duration = Duration::from_millis(1000);
/// The button waits this long after scrolling up, so wheel momentum that ends near the
/// bottom doesn't flash it.
pub const BUTTON_DELAY: Duration = Duration::from_millis(150);
/// The pointer this close to the pane's right edge widens the scrollbar.
pub const NEAR_EDGE: f32 = 12.;

const INSET_RIGHT: f32 = 2.;
const INSET_Y: f32 = 4.;
const NARROW: f32 = 6.;
const WIDE: f32 = 10.;
const MIN_THUMB: f32 = 24.;

/// System Settings › Accessibility › Display › Reduce motion: fades become instant.
#[cfg(target_os = "macos")]
pub fn reduce_motion() -> bool {
    use objc::runtime::{Object, BOOL, NO};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let on: BOOL = msg_send![ws, accessibilityDisplayShouldReduceMotion];
        on != NO
    }
}

#[cfg(not(target_os = "macos"))]
pub fn reduce_motion() -> bool {
    false
}

/// A value easing between 0 and 1 (ease-out), with separate in and out durations.
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    on: bool,
    from: f32,
    at: Instant,
    ms_in: f32,
    ms_out: f32,
}

impl Fade {
    pub fn new(ms_in: f32, ms_out: f32) -> Self {
        Self { on: false, from: 0., at: Instant::now(), ms_in, ms_out }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    fn target(&self) -> f32 {
        if self.on { 1. } else { 0. }
    }

    pub fn value(&self) -> f32 {
        let ms = if self.on { self.ms_in } else { self.ms_out };
        let t = if ms <= 0. { 1. } else { (self.at.elapsed().as_secs_f32() * 1000. / ms).min(1.) };
        let eased = 1. - (1. - t).powi(3);
        self.from + (self.target() - self.from) * eased
    }

    pub fn settled(&self) -> bool {
        self.value() == self.target()
    }

    /// Start moving toward `on`. Returns whether anything changed.
    pub fn set(&mut self, on: bool) -> bool {
        if on == self.on {
            return false;
        }
        self.from = self.value();
        self.on = on;
        self.at = Instant::now();
        if reduce_motion() {
            self.from = self.target();
        }
        true
    }

    /// Jump straight to `on` (debug screenshots, and hiding for the alternate screen).
    pub fn snap(&mut self, on: bool) {
        self.on = on;
        self.from = self.target();
    }
}

/// The scroll state the scrollbar draws, in lines.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub history: usize,
    pub rows: usize,
    pub offset: usize,
}

impl Metrics {
    /// Thumb (top, height) within a track `track_h` tall.
    pub fn thumb(&self, track_h: f32) -> (f32, f32) {
        let total = (self.history + self.rows).max(1) as f32;
        let h = (self.rows as f32 / total * track_h).max(MIN_THUMB).min(track_h);
        (self.top_for_offset(self.offset, track_h, h), h)
    }

    fn top_for_offset(&self, offset: usize, track_h: f32, thumb_h: f32) -> f32 {
        let travel = (track_h - thumb_h).max(0.);
        if self.history == 0 {
            return travel;
        }
        travel * (self.history - offset.min(self.history)) as f32 / self.history as f32
    }

    /// The offset that puts the thumb's top at `top`.
    pub fn offset_for(&self, top: f32, track_h: f32) -> usize {
        let (_, h) = self.thumb(track_h);
        let travel = track_h - h;
        if travel <= 0. {
            return 0;
        }
        let frac = (top / travel).clamp(0., 1.);
        ((1. - frac) * self.history as f32).round() as usize
    }

    /// Where grid line `line` (history negative) sits on the track: where the thumb's top is
    /// when that line is the top of the view, so dragging the thumb to a tick brings that
    /// line up. Lines on the screen spread over the thumb's height at the bottom.
    pub fn line_y(&self, line: i32, track_h: f32) -> f32 {
        let (_, h) = self.thumb(track_h);
        if line >= 0 {
            let at_bottom = self.top_for_offset(0, track_h, h);
            return at_bottom + h * line as f32 / self.rows.max(1) as f32;
        }
        self.top_for_offset((-line) as usize, track_h, h)
    }
}

/// The track in window coordinates for a pane, at hover width `wide` (0 narrow, 1 wide).
fn track(pane: Bounds<Pixels>, wide: f32) -> Bounds<Pixels> {
    let w = NARROW + (WIDE - NARROW) * wide;
    let right = f32::from(pane.origin.x + pane.size.width) - INSET_RIGHT;
    let top = f32::from(pane.origin.y) + INSET_Y;
    let h = (f32::from(pane.size.height) - 2. * INSET_Y).max(0.);
    Bounds::new(point(px(right - w), px(top)), size(px(w), px(h)))
}

/// Whether `pos` is within reach of the scrollbar (the strip along the pane's right edge).
pub fn near_edge(pane: Bounds<Pixels>, pos: Point<Pixels>) -> bool {
    pane.contains(&pos) && f32::from(pane.origin.x + pane.size.width - pos.x) <= NEAR_EDGE
}

pub enum Hit {
    /// On the thumb, this far below its top.
    Thumb(f32),
    /// On the track above / below the thumb.
    Above,
    Below,
}

pub fn hit(pane: Bounds<Pixels>, m: Metrics, pos: Point<Pixels>) -> Option<Hit> {
    if !near_edge(pane, pos) {
        return None;
    }
    let t = track(pane, 1.);
    let y = f32::from(pos.y - t.origin.y);
    let track_h = f32::from(t.size.height);
    if !(0. ..=track_h).contains(&y) {
        return None;
    }
    let (top, h) = m.thumb(track_h);
    Some(if y < top {
        Hit::Above
    } else if y > top + h {
        Hit::Below
    } else {
        Hit::Thumb(y - top)
    })
}

/// The offset a drag puts the view at, with the pointer at window y `y`.
pub fn drag_offset(pane: Bounds<Pixels>, m: Metrics, grab: f32, y: Pixels) -> usize {
    let t = track(pane, 1.);
    m.offset_for(f32::from(y - t.origin.y) - grab, f32::from(t.size.height))
}

/// Everything one frame of the scrollbar needs.
pub struct Paint {
    pub metrics: Metrics,
    /// Overall visibility, 0 to 1.
    pub opacity: f32,
    /// Hover width, 0 (6 pt) to 1 (10 pt with a track).
    pub wide: f32,
    pub dragging: bool,
    pub fg: Hsla,
    pub accent: Hsla,
    /// Prompt starts (grid lines), only while the hover track shows.
    pub prompts: Vec<i32>,
    /// Find matches: (first line, is the current one), while find is open.
    pub matches: Vec<(i32, bool)>,
}

pub fn paint(pane: Bounds<Pixels>, p: &Paint, window: &mut Window) {
    if p.opacity <= 0. {
        return;
    }
    let t = track(pane, p.wide);
    let (x, y0) = (t.origin.x, t.origin.y);
    let (w, track_h) = (f32::from(t.size.width), f32::from(t.size.height));
    let a = p.opacity;
    if p.wide > 0. {
        window.paint_quad(fill(t, p.fg.opacity(0.06 * p.wide * a)).corner_radii(px(w / 2.)));
    }
    // Ticks land on whole pixels; many lines share a pixel, so each pixel is painted once.
    let mut last = f32::NAN;
    for &(line, current) in &p.matches {
        let y = p.metrics.line_y(line, track_h).round().min(track_h - 2.);
        if y == last && !current {
            continue;
        }
        last = y;
        let alpha = if current { 1. } else { 0.85 };
        window.paint_quad(fill(Bounds::new(point(x, y0 + px(y)), size(px(w), px(2.))), p.accent.opacity(alpha * a)));
    }
    if p.wide > 0. {
        let color = p.fg.opacity(0.45 * p.wide * a);
        let mut last = f32::NAN;
        for &line in &p.prompts {
            let y = p.metrics.line_y(line, track_h).round().min(track_h - 1.5);
            if y == last {
                continue;
            }
            last = y;
            window.paint_quad(fill(Bounds::new(point(x + px(1.), y0 + px(y)), size(px(4.), px(1.5))), color));
        }
    }
    let (top, h) = p.metrics.thumb(track_h);
    let inset = p.wide;
    let alpha = if p.dragging { 0.7 } else { 0.35 + 0.2 * p.wide };
    let thumb = Bounds::new(point(x + px(inset), y0 + px(top)), size(px(w - 2. * inset), px(h)));
    window.paint_quad(fill(thumb, p.fg.opacity(alpha * a)).corner_radii(px((w - 2. * inset) / 2.)));
}

/// "128 new lines".
pub fn new_lines_label(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped} new line{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_follows_offset() {
        let m = Metrics { history: 900, rows: 100, offset: 0 };
        let (top, h) = m.thumb(500.);
        assert_eq!(h, 50.);
        assert_eq!(top, 450.);
        let up = Metrics { offset: 900, ..m };
        assert_eq!(up.thumb(500.).0, 0.);
        let mid = Metrics { offset: 450, ..m };
        assert_eq!(mid.thumb(500.).0, 225.);
        // Dragging the thumb's top back to where it is gives the same offset.
        assert_eq!(m.offset_for(225., 500.), 450);
        assert_eq!(m.offset_for(-10., 500.), 900);
        assert_eq!(m.offset_for(999., 500.), 0);
    }

    #[test]
    fn thumb_has_a_minimum() {
        let m = Metrics { history: 10_000, rows: 40, offset: 0 };
        assert_eq!(m.thumb(400.).1, MIN_THUMB);
        let none = Metrics { history: 0, rows: 40, offset: 0 };
        assert_eq!(none.thumb(400.), (0., 400.));
    }

    #[test]
    fn ticks_line_up_with_the_thumb() {
        let m = Metrics { history: 900, rows: 100, offset: 0 };
        // A line 450 up is at the top of the view when the thumb's top is at 225.
        assert_eq!(m.line_y(-450, 500.), 225.);
        assert_eq!(m.line_y(0, 500.), 450.);
        assert_eq!(m.line_y(50, 500.), 475.);
    }

    #[test]
    fn labels() {
        assert_eq!(new_lines_label(1), "1 new line");
        assert_eq!(new_lines_label(128), "128 new lines");
        assert_eq!(new_lines_label(12_345), "12,345 new lines");
    }

    #[test]
    fn fade_snaps() {
        let mut f = Fade::new(100., 300.);
        assert_eq!(f.value(), 0.);
        f.snap(true);
        assert!(f.on() && f.settled() && f.value() == 1.);
    }
}
