//! The icon picker's Image tab (canvas v3-ag A): a drop zone and "Choose image…", then a
//! cropper with a square frame rounded like the tile, a zoom slider and live previews at the
//! sizes the icon is drawn (row 22, two-line row 32, rail 18).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{div, img, prelude::*, px, AnyElement, Bounds, Context, ExternalPaths, FontWeight, KeyDownEvent, MouseButton, Pixels, Point, SharedString, Window};
use t1k_themes::UiColors;

use crate::custom_icon::{self, Icon};
use crate::group_icons::{IconFor, Mode};
use crate::overlay::Overlay;
use crate::workspace::Workspace;

const AREA_W: f32 = 278.;
const AREA_H: f32 = 200.;
const FRAME: f32 = 168.;
const MAX_ZOOM: f32 = 4.;

/// An image being cropped: the original, a PNG of it the cropper can draw, and the crop (a
/// square `side()` px wide around `cx`, `cy`, in that PNG's pixels).
pub struct Crop {
    src: PathBuf,
    preview: PathBuf,
    w: f32,
    h: f32,
    zoom: f32,
    cx: f32,
    cy: f32,
    /// Pointer and crop center when a drag began.
    drag: Option<(Point<Pixels>, f32, f32)>,
    sliding: bool,
    /// The zoom slider's track, from the last paint (pointer x to zoom).
    track: Rc<Cell<Bounds<Pixels>>>,
}

impl Drop for Crop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.preview);
    }
}

impl Crop {
    /// Read the image (blocking; the picker does this off the main thread).
    pub fn load(src: &Path) -> Option<Crop> {
        Some(Crop::new(src.to_path_buf(), custom_icon::load_for_crop(src)?))
    }

    fn new(src: PathBuf, (preview, w, h): (PathBuf, f32, f32)) -> Crop {
        Crop { src, preview, w, h, zoom: 1., cx: w / 2., cy: h / 2., drag: None, sliding: false, track: Rc::default() }
    }

    /// Centered and fit: the largest square, from the middle.
    fn side(&self) -> f32 {
        self.w.min(self.h) / self.zoom
    }

    fn clamp(&mut self) {
        let s = self.side() / 2.;
        self.cx = self.cx.clamp(s, self.w - s);
        self.cy = self.cy.clamp(s, self.h - s);
    }

    pub fn set_zoom(&mut self, z: f32) {
        self.zoom = z.clamp(1., MAX_ZOOM);
        self.clamp();
    }

    /// The crop as fractions of the image (x, y, w, h from the top left).
    fn unit(&self) -> [f64; 4] {
        let s = self.side();
        [((self.cx - s / 2.) / self.w) as f64, ((self.cy - s / 2.) / self.h) as f64, (s / self.w) as f64, (s / self.h) as f64]
    }

    /// The image placed so the crop fills a `frame` square at the origin: left, top, w, h.
    fn placed(&self, frame: f32) -> (f32, f32, f32, f32) {
        let s = self.side();
        let k = frame / s;
        (-(self.cx - s / 2.) * k, -(self.cy - s / 2.) * k, self.w * k, self.h * k)
    }

    /// The crop at `size`, rounded like the tile.
    fn preview(&self, size: f32) -> gpui::Div {
        let (l, t, w, h) = self.placed(size);
        div()
            .relative()
            .size(px(size))
            .flex_none()
            .rounded(px((size * 0.24).round()))
            .overflow_hidden()
            .child(placed_img(&self.preview, l, t, w, h))
    }
}

/// The image at its full placed size, even where it runs past its clipping parent (left
/// unsized, the wrapper shrank to the parent and the image was fitted into it).
fn placed_img(p: &Path, l: f32, t: f32, w: f32, h: f32) -> gpui::Div {
    div().absolute().left(px(l)).top(px(t)).w(px(w)).h(px(h)).flex_none().child(img(p.to_path_buf()).w(px(w)).h(px(h)).flex_none())
}

impl Workspace {
    /// Open `path` in the cropper (read in the background; large files take a moment).
    pub fn start_crop(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.icon_ui.mode = Mode::Image;
        self.icon_ui.crop = None;
        self.icon_ui.error = None;
        self.icon_ui.busy = true;
        cx.notify();
        let task = cx.background_executor().spawn(async move { custom_icon::load_for_crop(&path).map(|r| (path, r)) });
        cx.spawn(async move |this, cx| {
            let crop = task.await;
            let _ = this.update(cx, |ws, cx| {
                ws.icon_ui.busy = false;
                match crop {
                    Some((src, r)) => ws.icon_ui.crop = Some(Crop::new(src, r)),
                    None => ws.icon_ui.error = Some("Couldn't open that image"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Choose image…": an open panel for one file.
    fn choose_icon_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |ws, cx| match paths.into_iter().next() {
                    // The panel can't filter by type, so say what works.
                    Some(p) if custom_icon::is_icon_image(&p) => ws.start_crop(p, cx),
                    Some(_) => {
                        ws.icon_ui.error = Some("Choose a PNG, JPG, SVG or ICNS file");
                        cx.notify();
                    }
                    None => {}
                });
            }
        })
        .detach();
    }

    /// "Use image": save the crop as the icon, then close the picker.
    fn use_crop(&mut self, key: IconFor, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.icon_ui.crop else { return };
        if self.icon_ui.busy {
            return;
        }
        let (src, unit) = (c.src.clone(), c.unit());
        self.icon_ui.busy = true;
        cx.notify();
        let task = cx.background_executor().spawn(async move { custom_icon::save_crop(&src, unit) });
        cx.spawn_in(window, async move |this, cx| {
            let sha = task.await;
            let _ = this.update_in(cx, |ws, window, cx| {
                ws.icon_ui.busy = false;
                match sha {
                    Some(sha) => {
                        ws.set_icon(&key, Some(Icon::Image(sha)), cx);
                        ws.icon_ui.crop = None;
                        ws.close_overlay(window, cx);
                    }
                    None => {
                        ws.icon_ui.error = Some("Couldn't save that image");
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Keys on the Image tab: ↵ uses the crop, Esc goes back (or closes), + and − zoom, arrows nudge.
    #[allow(clippy::too_many_arguments)]
    pub fn crop_key(&mut self, key: IconFor, at: Point<Pixels>, query: String, selected: usize, cat: usize, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        let keep = Some(Overlay::IconPicker { key: key.clone(), at, query, selected, cat });
        let Some(c) = &mut self.icon_ui.crop else {
            return (ev.keystroke.key != "escape").then_some(keep).flatten();
        };
        let nudge = c.side() / 40.;
        match ev.keystroke.key.as_str() {
            "escape" => self.icon_ui.crop = None,
            "enter" => {
                // The picker stays open while the image is saved, then closes.
                self.use_crop(key, window, cx);
                return keep;
            }
            "=" | "+" => c.set_zoom(c.zoom * 1.15),
            "-" => c.set_zoom(c.zoom / 1.15),
            "left" => c.cx -= nudge,
            "right" => c.cx += nudge,
            "up" => c.cy -= nudge,
            "down" => c.cy += nudge,
            _ => {}
        }
        if let Some(c) = &mut self.icon_ui.crop {
            c.clamp();
        }
        cx.notify();
        keep
    }

    /// An image dropped on a group header: the picker opens there, cropping it.
    pub fn drop_icon_on_group(&mut self, key: &Path, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_hover = None;
        let Some(p) = paths.paths().iter().find(|p| custom_icon::is_icon_image(p)).cloned() else {
            cx.notify();
            return;
        };
        let k = IconFor::Group(key.to_path_buf());
        self.open_icon_picker(k.clone(), window.mouse_position(), window, cx);
        self.start_crop(p, cx);
    }
}

/// The Image tab's content: the drop zone, or the cropper.
pub fn render(ws: &Workspace, key: &IconFor, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let button = |id: &'static str, text: &'static str, primary: bool| {
        div()
            .id(id)
            .h(px(26.))
            .px(px(10.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .text_size(px(12.5))
            .cursor_pointer()
            .map(|d| if primary { d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::MEDIUM) } else { d.bg(ui.bg_control).text_color(ui.text_primary) })
            .hover(|d| d.opacity(0.88))
            .child(text)
    };
    let error = ws.icon_ui.error.map(|e| div().text_size(px(11.5)).text_color(ui.status_error).child(e));
    let Some(c) = &ws.icon_ui.crop else {
        return div()
            .h(px(150.))
            .rounded(px(10.))
            .border_1()
            .border_dashed()
            .border_color(ui.text_tertiary)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .text_color(ui.text_secondary)
            .child(div().text_size(px(20.)).text_color(ui.text_tertiary).child("⤓"))
            .child(if ws.icon_ui.busy { "Opening image…" } else { "Drop an image here" })
            .child(button("icon-choose", "Choose image…", false).on_click(cx.listener(move |this, _, window, cx| this.choose_icon_image(window, cx))))
            .child(match error {
                Some(e) => e,
                None => div().text_size(px(11.)).text_color(ui.text_tertiary).child("PNG, JPG, SVG or ICNS"),
            })
            .into_any_element();
    };
    let name = c.src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (fx, fy) = ((AREA_W - FRAME) / 2., (AREA_H - FRAME) / 2.);
    let (l, t, w, h) = c.placed(FRAME);
    let mask = |x: f32, y: f32, w: f32, h: f32| div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).bg(gpui::black().opacity(0.45));
    let area = div()
        .id("crop-area")
        .relative()
        .w(px(AREA_W))
        .h(px(AREA_H))
        .rounded(px(8.))
        .overflow_hidden()
        .bg(ui.bg_sidebar)
        .cursor(gpui::CursorStyle::OpenHand)
        .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &gpui::MouseDownEvent, _, _| {
            if let Some(c) = &mut this.icon_ui.crop {
                c.drag = Some((ev.position, c.cx, c.cy));
            }
        }))
        .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
            let Some(c) = &mut this.icon_ui.crop else { return };
            let Some((start, x, y)) = c.drag else { return };
            if ev.pressed_button != Some(MouseButton::Left) {
                c.drag = None;
                return;
            }
            // Dragging moves the image under the frame.
            let k = c.side() / FRAME;
            c.cx = x - f32::from(ev.position.x - start.x) * k;
            c.cy = y - f32::from(ev.position.y - start.y) * k;
            c.clamp();
            cx.notify();
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
            if let Some(c) = &mut this.icon_ui.crop {
                c.drag = None;
            }
        }))
        .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, _, cx| {
            let Some(c) = &mut this.icon_ui.crop else { return };
            let dy = match ev.delta {
                gpui::ScrollDelta::Pixels(p) => f32::from(p.y) / 40.,
                gpui::ScrollDelta::Lines(l) => l.y,
            };
            c.set_zoom(c.zoom * (1. + dy * 0.1));
            cx.notify();
        }))
        .child(placed_img(&c.preview, fx + l, fy + t, w, h))
        .child(mask(0., 0., AREA_W, fy))
        .child(mask(0., fy + FRAME, AREA_W, AREA_H - fy - FRAME))
        .child(mask(0., fy, fx, FRAME))
        .child(mask(fx + FRAME, fy, AREA_W - fx - FRAME, FRAME))
        .child(div().absolute().left(px(fx)).top(px(fy)).size(px(FRAME)).rounded(px((FRAME * 0.24).round())).border_2().border_color(gpui::white().opacity(0.9)));
    let track = c.track.clone();
    let t = (c.zoom - 1.) / (MAX_ZOOM - 1.);
    let zoom_to = move |this: &mut Workspace, x: Pixels| {
        if let Some(c) = &mut this.icon_ui.crop {
            let b = c.track.get();
            let f = if b.size.width > px(0.) { f32::from(x - b.origin.x) / f32::from(b.size.width) } else { 0. };
            c.set_zoom(1. + f.clamp(0., 1.) * (MAX_ZOOM - 1.));
        }
    };
    let step = |id: &'static str, glyph: &'static str, f: f32| {
        div()
            .id(id)
            .w(px(14.))
            .flex()
            .justify_center()
            .cursor_pointer()
            .hover(move |d| d.text_color(ui.text_primary))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(c) = &mut this.icon_ui.crop {
                    c.set_zoom(c.zoom * f);
                }
                cx.notify();
            }))
            .child(glyph)
    };
    let slider = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.))
        .text_color(ui.text_tertiary)
        .child(step("zoom-out", "−", 1. / 1.25))
        .child(
            div()
                .id("zoom-track")
                .relative()
                .flex_1()
                .h(px(14.))
                .cursor_pointer()
                .child(gpui::canvas(move |b, _, _| track.set(b), |_, _, _, _| {}).absolute().size_full())
                .child(div().absolute().left_0().right_0().top(px(5.5)).h(px(3.)).rounded(px(2.)).bg(ui.border))
                .child(div().absolute().left(gpui::relative(t)).ml(px(-6.5 * t * 2.)).top(px(0.5)).size(px(13.)).rounded_full().bg(ui.text_primary))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    zoom_to(this, ev.position.x);
                    if let Some(c) = &mut this.icon_ui.crop {
                        c.sliding = true;
                    }
                    cx.notify();
                }))
                .on_mouse_move(cx.listener(move |this, ev: &gpui::MouseMoveEvent, _, cx| {
                    let sliding = this.icon_ui.crop.as_ref().is_some_and(|c| c.sliding);
                    if sliding && ev.pressed_button == Some(MouseButton::Left) {
                        zoom_to(this, ev.position.x);
                        cx.notify();
                    }
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
                    if let Some(c) = &mut this.icon_ui.crop {
                        c.sliding = false;
                    }
                })),
        )
        .child(step("zoom-in", "+", 1.25));
    let previews = div()
        .flex()
        .items_center()
        .gap(px(14.))
        .text_size(px(11.))
        .text_color(ui.text_tertiary)
        .child(c.preview(22.))
        .child(c.preview(32.))
        .child(c.preview(18.))
        .child("row · two-line · rail");
    let k = key.clone();
    let saving = ws.icon_ui.busy;
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(div().truncate().text_size(px(12.)).text_color(ui.text_secondary).child(SharedString::from(format!("{name} · drag to move, slide to zoom"))))
        .child(area)
        .child(slider)
        .child(previews)
        .children(error)
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(button("crop-cancel", "Cancel", false).on_click(cx.listener(|this, _, _, cx| {
                    this.icon_ui.crop = None;
                    cx.notify();
                })))
                .child(button("crop-use", if saving { "Saving…" } else { "Use image" }, true).on_click(cx.listener(move |this, _, window, cx| this.use_crop(k.clone(), window, cx)))),
        )
        .into_any_element()
}

/// "Set as icon" on a group header while an image is dragged over it.
pub fn header_label(ui: UiColors) -> gpui::Div {
    div()
        .absolute()
        .right(px(10.))
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .child(div().px(px(6.)).rounded(px(4.)).bg(ui.bg_sidebar).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(ui.accent).child("Set as icon"))
}

/// Make a group header take image drops: it highlights, says "Set as icon", and opens the
/// cropper. Other files are ignored (a header has no terminal to type paths into).
pub fn header_drop(el: gpui::Stateful<gpui::Div>, ws: &Workspace, key: &Path, cx: &mut Context<Workspace>) -> gpui::Stateful<gpui::Div> {
    let ui = ws.ui;
    let hovered = ws.drop_hover.as_ref().is_some_and(|h| h.target == crate::drop::DropTarget::Group(key.to_path_buf()) && h.paths.iter().any(|p| custom_icon::is_icon_image(p)));
    let (k1, k2) = (key.to_path_buf(), key.to_path_buf());
    el.drag_over::<ExternalPaths>(move |s, paths: &ExternalPaths, _, _| {
        if paths.paths().iter().any(|p| custom_icon::is_icon_image(p)) {
            s.bg(ui.accent.opacity(0.12)).border_1().border_color(ui.accent.opacity(0.55))
        } else {
            s
        }
    })
    .on_drag_move::<ExternalPaths>(cx.listener(move |this, e: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
        let inside = e.bounds.contains(&e.event.position);
        let paths = e.drag(cx).clone();
        this.drop_hover(crate::drop::DropTarget::Group(k1.clone()), inside, &paths, cx);
    }))
    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| this.drop_icon_on_group(&k2, paths, window, cx)))
    .when(hovered, |d| d.bg(ui.accent.opacity(0.12)).child(header_label(ui)))
}
