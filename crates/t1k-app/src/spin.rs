//! Working spinners turn in Core Animation, not in GPUI frames.
//!
//! GPUI can't redraw part of a window: turning one 12 px arc meant laying out and painting the
//! whole window 60 times a second (about 40% CPU with a few agents working). Instead each
//! spinner paints nothing itself. During paint it reports where it sits, and the last thing
//! painted in the window (`sync`) lines up one native layer per spinner above the Metal layer.
//! Each layer carries an endless rotation that the system compositor runs, so it turns at the
//! display's full rate for no app CPU. GPUI only moves layers when a frame happens anyway.
//!
//! A spinner covered by something painted after it (a menu, a dialog's backdrop) or scrolled
//! out of its clip gets no layer: GPUI draws it holding still instead, under whatever covers it.

use std::time::Instant;
use gpui::{px, App, Bounds, Hitbox, HitboxBehavior, Hsla, IntoElement, Pixels, Styled, Window};

/// One full turn.
const TURN_SECS: f64 = 0.9;
const STROKE: f32 = 1.5;

#[derive(Clone, Copy, PartialEq)]
struct Want {
    /// The spinner's center and radius, window points.
    x: f32,
    y: f32,
    r: f32,
    /// Where it may show (its bounds within the content mask).
    clip: Bounds<Pixels>,
    rgba: [f32; 4],
    /// Seconds into the turn, so it starts where the GPUI arc used to be.
    phase: f64,
}

thread_local! {
    static WANTS: std::cell::RefCell<std::collections::HashMap<u64, Vec<Want>>> = Default::default();
}

fn key(window: &Window) -> u64 {
    window.window_handle().window_id().as_u64()
}

/// A working spinner: a 0.75 arc of radius `r` turning once per 900 ms, centered in its box.
pub fn spinner(color: Hsla, started: Instant, r: f32) -> gpui::Canvas<Hitbox> {
    gpui::canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, _: &mut App| {
            let c = bounds.center();
            // Covered by something painted later (a menu, a dialog's dimmed backdrop): GPUI
            // draws it holding still, so whatever covers it draws over it as before.
            if window.topmost_hitbox_at(c) != Some(hitbox.id) {
                let turn = (started.elapsed().as_secs_f64() % TURN_SECS / TURN_SECS) as f32 * std::f32::consts::TAU;
                still_arc(window, c, r, turn, color);
                return;
            }
            let rgb = color.to_rgb();
            let want = Want {
                x: f32::from(c.x),
                y: f32::from(c.y),
                r,
                clip: window.content_mask().bounds.intersect(&bounds),
                rgba: [rgb.r, rgb.g, rgb.b, rgb.a],
                phase: started.elapsed().as_secs_f64() % TURN_SECS,
            };
            WANTS.with(|w| w.borrow_mut().entry(key(window)).or_default().push(want));
        },
    )
    .size_full()
}

fn still_arc(window: &mut Window, c: gpui::Point<Pixels>, r: f32, turn: f32, color: Hsla) {
    let mut b = gpui::PathBuilder::stroke(px(STROKE));
    let steps = 24;
    for i in 0..=steps {
        let a = turn + i as f32 / steps as f32 * std::f32::consts::TAU * 0.75;
        let p = gpui::point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
        if i == 0 { b.move_to(p) } else { b.line_to(p) }
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// Put last in a window's root: after every spinner has painted, it lines up the layers.
pub fn sync() -> impl IntoElement {
    gpui::deferred(
        gpui::canvas(
            |_, _, _| {},
            |_, _, window, _: &mut App| {
                let wants = WANTS.with(|w| w.borrow_mut().remove(&key(window))).unwrap_or_default();
                mac::sync(window, key(window), &wants);
            },
        )
        .absolute()
        .size(px(0.)),
    )
    .with_priority(usize::MAX)
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{Want, STROKE, TURN_SECS};
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;

    #[link(name = "QuartzCore", kind = "framework")]
    unsafe extern "C" {
        fn CACurrentMediaTime() -> f64;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPathCreateMutable() -> *mut c_void;
        fn CGPathAddArc(path: *mut c_void, m: *const c_void, x: f64, y: f64, r: f64, start: f64, end: f64, clockwise: bool);
        fn CGPathRelease(path: *mut c_void);
    }

    /// One spinner's layers: a clip holding the turning arc.
    struct Spin {
        clip: id,
        arc: id,
        r: f32,
        rgba: [f32; 4],
    }

    /// A window's container layer (flipped, so y runs down as in GPUI) and its spinners.
    struct Host {
        view: id,
        container: id,
        spins: Vec<Spin>,
    }

    thread_local! {
        static HOSTS: RefCell<HashMap<u64, Host>> = RefCell::new(HashMap::new());
    }

    fn ns_view(window: &gpui::Window) -> Option<id> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr() as id),
            _ => None,
        }
    }

    pub(super) fn sync(window: &gpui::Window, key: u64, wants: &[Want]) {
        let scale = window.scale_factor() as f64;
        HOSTS.with(|hosts| {
            let mut hosts = hosts.borrow_mut();
            if wants.is_empty() && hosts.get(&key).is_none_or(|h| h.spins.is_empty()) {
                return;
            }
            unsafe {
                let _: () = msg_send![class!(CATransaction), begin];
                let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
                if let Some(host) = host(&mut hosts, key, window) {
                    // Spare layers go; missing ones are made. A layer keeps its rotation when it
                    // moves to another spinner, so they all stay in step.
                    while host.spins.len() > wants.len() {
                        let s = host.spins.pop().unwrap();
                        let _: () = msg_send![s.clip, removeFromSuperlayer];
                        let _: () = msg_send![s.arc, release];
                        let _: () = msg_send![s.clip, release];
                    }
                    while host.spins.len() < wants.len() {
                        let w = wants[host.spins.len()];
                        host.spins.push(make(host.container, &w, scale));
                    }
                    for (s, w) in host.spins.iter_mut().zip(wants) {
                        place(s, w, scale);
                    }
                }
                let _: () = msg_send![class!(CATransaction), commit];
            }
        });
    }

    /// The window's container, made on first use and kept sized to the view.
    unsafe fn host<'a>(hosts: &'a mut HashMap<u64, Host>, key: u64, window: &gpui::Window) -> Option<&'a mut Host> {
        let view = ns_view(window)?;
        unsafe {
            let base: id = msg_send![view, layer];
            if base == nil {
                return None;
            }
            let host = match hosts.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) if e.get().view == view => e.into_mut(),
                e => {
                    let container: id = msg_send![class!(CALayer), new];
                    let _: () = msg_send![container, setGeometryFlipped: YES];
                    let _: () = msg_send![base, addSublayer: container];
                    let host = Host { view, container, spins: Vec::new() };
                    match e {
                        std::collections::hash_map::Entry::Occupied(mut o) => {
                            o.insert(host);
                            o.into_mut()
                        }
                        std::collections::hash_map::Entry::Vacant(v) => v.insert(host),
                    }
                }
            };
            let b: NSRect = msg_send![base, bounds];
            let _: () = msg_send![host.container, setFrame: b];
            Some(host)
        }
    }

    unsafe fn make(container: id, w: &Want, scale: f64) -> Spin {
        unsafe {
            let clip: id = msg_send![class!(CALayer), new];
            let _: () = msg_send![clip, setMasksToBounds: YES];
            let arc: id = msg_send![class!(CAShapeLayer), new];
            let _: () = msg_send![arc, setFillColor: std::ptr::null::<c_void>()];
            let _: () = msg_send![arc, setLineWidth: STROKE as f64];
            let _: () = msg_send![arc, setContentsScale: scale];
            let _: () = msg_send![clip, addSublayer: arc];
            let _: () = msg_send![container, addSublayer: clip];

            let spin: id = msg_send![class!(CABasicAnimation), animationWithKeyPath: NSString::alloc(nil).init_str("transform.rotation.z")];
            let from: id = msg_send![class!(NSNumber), numberWithDouble: 0f64];
            let to: id = msg_send![class!(NSNumber), numberWithDouble: std::f64::consts::TAU];
            let _: () = msg_send![spin, setFromValue: from];
            let _: () = msg_send![spin, setToValue: to];
            let _: () = msg_send![spin, setDuration: TURN_SECS];
            let _: () = msg_send![spin, setRepeatCount: f32::INFINITY];
            let _: () = msg_send![spin, setRemovedOnCompletion: NO];
            let _: () = msg_send![spin, setBeginTime: CACurrentMediaTime() - w.phase];
            let _: () = msg_send![arc, addAnimation: spin forKey: NSString::alloc(nil).init_str("spin")];

            let mut s = Spin { clip, arc, r: -1., rgba: [-1.; 4] };
            place(&mut s, w, scale);
            s
        }
    }

    unsafe fn place(s: &mut Spin, w: &Want, scale: f64) {
        unsafe {
            if s.r != w.r {
                s.r = w.r;
                let side = 2. * (w.r + STROKE) as f64;
                let _: () = msg_send![s.arc, setBounds: NSRect::new(NSPoint::new(0., 0.), NSSize::new(side, side))];
                let path = CGPathCreateMutable();
                let c = side / 2.;
                CGPathAddArc(path, std::ptr::null(), c, c, w.r as f64, 0., std::f64::consts::TAU * 0.75, false);
                let _: () = msg_send![s.arc, setPath: path];
                CGPathRelease(path);
            }
            if s.rgba != w.rgba {
                s.rgba = w.rgba;
                let [r, g, b, a] = w.rgba.map(|v| v as f64);
                let color: id = msg_send![class!(NSColor), colorWithSRGBRed: r green: g blue: b alpha: a];
                let cg: *const c_void = msg_send![color, CGColor];
                let _: () = msg_send![s.arc, setStrokeColor: cg];
            }
            let _: () = msg_send![s.arc, setContentsScale: scale];
            let (cx, cy) = (f32::from(w.clip.origin.x), f32::from(w.clip.origin.y));
            let clip = NSRect::new(
                NSPoint::new(cx as f64, cy as f64),
                NSSize::new(f32::from(w.clip.size.width) as f64, f32::from(w.clip.size.height) as f64),
            );
            let _: () = msg_send![s.clip, setFrame: clip];
            let _: () = msg_send![s.arc, setPosition: NSPoint::new((w.x - cx) as f64, (w.y - cy) as f64)];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    pub(super) fn sync(_: &gpui::Window, _: u64, _: &[super::Want]) {}
}
