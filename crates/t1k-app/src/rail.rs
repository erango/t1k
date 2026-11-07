//! Sidebar resizing and the collapsed states (design v3-g..j).
//!
//! The sidebar's right edge drags to resize (200-480, sticky at 300). Dragging to within
//! `COLLAPSE_AT` of the window's left edge previews the collapsed state the user picked in
//! Settings (`sidebar.collapsed`: a 48 px rail of tab glyphs, or hidden); moving back out
//! springs to the minimum width and keeps resizing. Nothing is committed until release.

use std::time::{Duration, Instant};

use gpui::{div, prelude::*, px, AnyElement, Context, CursorStyle, MouseButton, MouseUpEvent, Pixels, Render, SharedString, Window};
use t1k_agent::Status;

use crate::workspace::Workspace;

pub const RAIL_W: f32 = 48.;
pub const MIN_W: f32 = 200.;
pub const MAX_W: f32 = 480.;
pub const DEFAULT_W: f32 = 300.;
/// Pointer this close to the window's left edge collapses the sidebar (on release).
pub const COLLAPSE_AT: f32 = 90.;
const SNAP: f32 = 8.;

/// The value carried by a sidebar-edge drag.
#[derive(Clone, Copy)]
pub struct EdgeDrag;

/// GPUI draws something under the pointer while dragging; we draw nothing.
pub struct NoGhost;
impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragPreview {
    Width(f32),
    Collapsed,
}

/// Where the edge goes for a pointer at `x` (window coordinates).
pub fn preview_for(x: f32) -> DragPreview {
    if x < COLLAPSE_AT {
        return DragPreview::Collapsed;
    }
    let w = x.clamp(MIN_W, MAX_W);
    DragPreview::Width(if (w - DEFAULT_W).abs() <= SNAP { DEFAULT_W } else { w.round() })
}

/// What the sidebar looks like right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shown {
    Full(f32),
    Rail,
    Hidden,
}

impl Workspace {
    pub fn collapse_to_rail(&self) -> bool {
        self.config.sidebar.collapsed != "hidden"
    }

    pub fn sidebar_shown(&self) -> Shown {
        let collapsed = if self.collapse_to_rail() { Shown::Rail } else { Shown::Hidden };
        match self.sidebar_drag {
            Some(DragPreview::Width(w)) => Shown::Full(w),
            Some(DragPreview::Collapsed) => collapsed,
            None if self.sidebar_open => Shown::Full(self.config.sidebar.width.clamp(MIN_W, MAX_W)),
            None => collapsed,
        }
    }

    pub fn sidebar_drag_move(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let p = preview_for(f32::from(x));
        if self.sidebar_drag != Some(p) {
            self.sidebar_drag = Some(p);
            self.peek = false;
            cx.notify();
        }
    }

    /// Release: commit what the drag previewed.
    pub fn sidebar_drag_end(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.sidebar_drag.take() else { return };
        match p {
            DragPreview::Width(w) => {
                self.sidebar_open = true;
                if (w - self.config.sidebar.width).abs() >= 1. {
                    self.config.sidebar.width = w;
                    crate::config::set_value(&["sidebar", "width"], toml_edit::Value::from(w as i64));
                }
            }
            DragPreview::Collapsed => self.sidebar_open = false,
        }
        self.save();
        cx.notify();
    }

    /// Rest the pointer on the rail to see the full sidebar over the terminal.
    pub fn rail_hover(&mut self, over: bool, cx: &mut Context<Self>) {
        self.peek_gen += 1;
        let generation = self.peek_gen;
        let ms = if over { self.config.sidebar.peek_ms } else { 200 };
        if over && (ms == 0 || self.peek) {
            return;
        }
        self.peek_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            let _ = this.update(cx, |ws, cx| {
                if ws.peek_gen == generation && ws.peek != over && ws.sidebar_drag.is_none() {
                    ws.peek = over;
                    cx.notify();
                }
            });
        }));
    }
}

/// The 8 px grab strip on the sidebar's (or rail's) right edge, below the title strip.
pub fn edge(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let dragging = ws.sidebar_drag.is_some();
    div()
        .id("sidebar-edge")
        .absolute()
        .top(px(38.))
        .bottom_0()
        .right(px(-4.))
        .w(px(8.))
        .cursor(CursorStyle::ResizeLeftRight)
        .group("sidebar-edge")
        .on_drag(EdgeDrag, |_, _, _, cx| cx.new(|_| NoGhost))
        .on_click(cx.listener(|this, ev: &gpui::ClickEvent, _, cx| {
            // Double-click: default width, or open the rail at its saved width.
            if ev.click_count() == 2 {
                if this.sidebar_open {
                    this.config.sidebar.width = DEFAULT_W;
                    crate::config::set_value(&["sidebar", "width"], toml_edit::Value::from(DEFAULT_W as i64));
                } else {
                    this.sidebar_open = true;
                }
                this.save();
                cx.notify();
            }
        }))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(if dragging { 3. } else { 3.5 }))
                .w(px(if dragging { 2. } else { 1. }))
                .when(dragging, |d| d.bg(ui.accent))
                .when(!dragging, |d| d.invisible().group_hover("sidebar-edge", |s| s.visible().bg(ui.border))),
        )
}

/// While dragging: the width (or what releasing will do) near the top of the edge.
pub fn readout(ws: &Workspace) -> Option<AnyElement> {
    let ui = ws.ui;
    let (x, text) = match (ws.sidebar_drag?, ws.sidebar_shown()) {
        (DragPreview::Width(w), _) => (
            w,
            if w == DEFAULT_W {
                "300 · default".to_string()
            } else if w <= MIN_W {
                "200 · min".to_string()
            } else if w >= MAX_W {
                "480 · max".to_string()
            } else {
                format!("{} px", w as i64)
            },
        ),
        (DragPreview::Collapsed, Shown::Rail) => (RAIL_W, "Release to collapse to the strip".into()),
        (DragPreview::Collapsed, _) => (0., "Release to hide the sidebar".into()),
    };
    Some(
        div()
            .absolute()
            .top(px(46.))
            .left(px(x + 10.))
            .px(px(8.))
            .h(px(22.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .shadow_sm()
            .text_size(px(11.5))
            .font_family("JetBrains Mono")
            .text_color(ui.text_secondary)
            .child(text)
            .into_any_element(),
    )
}

/// The collapsed sidebar: one glyph per tab, grouped, with the needs-you count on top.
pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let needs = ws.needs_queue().len();
    let started: Instant = ws.started;
    let mut col = div()
        .id("rail-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.))
        .pb(px(12.));
    for (key, tabs) in ws.grouped() {
        let mut g = div().w_full().flex().flex_col().items_center();
        g = g.child(
            div()
                .h(px(14.))
                .flex()
                .items_center()
                .child(div().size(px(7.)).rounded_full().bg(ws.group_color(&key).unwrap_or(ui.text_disabled))),
        );
        for i in tabs {
            let t = &ws.tabs[i];
            let selected = i == ws.active;
            let quiet = matches!(t.status(), Some(Status::Done) | Some(Status::Idle) | Some(Status::Ended) | None)
                && !t.unread
                && !t.reminder_due;
            let dot = match t.status() {
                Some(Status::NeedsInput { .. }) => Some(ui.status_needs),
                Some(Status::Error { .. }) => Some(ui.status_error),
                _ if t.reminder_due || t.unread => Some(ui.accent),
                _ => None,
            };
            let working = t.status() == Some(&Status::Working);
            g = g.child(
                div()
                    .id(SharedString::from(format!("rail-{}", t.id)))
                    .relative()
                    .w_full()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(ui.bg_terminal))
                    .when(!selected, |d| d.hover(move |d| d.bg(ui.bg_hover)))
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(i, window, cx)))
                    .when(selected, |d| d.child(div().absolute().left_0().top(px(5.)).bottom(px(5.)).w(px(3.)).rounded_r(px(2.)).bg(ui.accent)))
                    .child(
                        div()
                            .relative()
                            .when(quiet && !selected, |d| d.opacity(0.45))
                            .child(crate::app_icon::badge(t, ui, 18.))
                            .children(dot.map(|c| {
                                div().absolute().right(px(-3.)).bottom(px(-3.)).size(px(9.)).rounded_full().bg(c).border_2().border_color(ui.bg_sidebar)
                            }))
                            .when(working && dot.is_none(), |d| {
                                d.child(div().absolute().right(px(-4.)).bottom(px(-4.)).child(crate::sidebar::spinner_sized(ui.status_working, started, 4.)))
                            }),
                    ),
            );
        }
        col = col.child(g);
    }
    div()
        .id("rail")
        .relative()
        .w(px(RAIL_W))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(ui.bg_sidebar)
        .on_hover(cx.listener(|this, over: &bool, _, cx| this.rail_hover(*over, cx)))
        // Title strip height: the traffic lights sit over it.
        .child(
            div().h(px(44.)).flex_none().on_mouse_down(MouseButton::Left, |ev, window, _| {
                if ev.click_count == 2 {
                    window.zoom_window();
                } else {
                    window.start_window_move();
                }
            }),
        )
        .children((needs > 0).then(|| {
            div()
                .id("rail-needs")
                .mx_auto()
                .mb(px(10.))
                .w(px(32.))
                .h(px(20.))
                .flex_none()
                .rounded(px(10.))
                .bg(ui.status_needs)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(ui.bg_sidebar)
                .cursor_pointer()
                .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::JumpToNeeds), cx))
                .child(needs.to_string())
        }))
        .child(col)
        .child(edge(ws, cx))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_preview() {
        assert_eq!(preview_for(10.), DragPreview::Collapsed);
        assert_eq!(preview_for(89.), DragPreview::Collapsed);
        // Out of the collapse zone but under the minimum: springs to the minimum.
        assert_eq!(preview_for(120.), DragPreview::Width(MIN_W));
        assert_eq!(preview_for(305.), DragPreview::Width(DEFAULT_W));
        assert_eq!(preview_for(350.4), DragPreview::Width(350.));
        assert_eq!(preview_for(900.), DragPreview::Width(MAX_W));
    }
}
