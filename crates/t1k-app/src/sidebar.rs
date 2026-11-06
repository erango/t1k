//! Calm sidebar: project groups with one-line tab rows; two-line rows on hover intent or ⌥.
//! Sizes and colors follow `docs/design.md` (v2: calm sidebar).

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, SharedString};

use crate::workspace::Workspace;

const WIDTH: f32 = 300.;
const ROW_CALM: f32 = 32.;
const ROW_DETAIL: f32 = 52.;
const ROW_GAP: f32 = 2.;
const HEADER: f32 = 28.;
const INSET: f32 = 6.;

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let detail = ws.show_detail();
    let groups = ws.grouped();

    let mut list = div().id("sidebar-list").flex_1().min_h_0().overflow_y_scroll().pb(px(12.)).flex().flex_col();

    for (gi, (key, tabs)) in groups.into_iter().enumerate() {
        let collapsed = ws.collapsed.contains(&key);
        if gi > 0 {
            // Group gap: 8 + hairline + 12.
            list = list.child(div().mt(px(8.)).mb(px(12.)).mx(px(12.)).h(px(1.)).bg(ui.border));
        }
        let label = ws.group_label(&key);
        let count = tabs.len();
        let has_active = tabs.contains(&ws.active);
        let toggle_key = key.clone();
        list = list.child(
            div()
                .id(SharedString::from(format!("group-{}", key.display())))
                .h(px(HEADER))
                .flex_none()
                .mx(px(INSET))
                .px(px(6.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(toggle_key.clone(), cx)))
                .child(
                    div()
                        .w(px(12.))
                        .text_size(px(9.))
                        .text_color(ui.text_tertiary)
                        .child(if collapsed { "▶" } else { "▼" }),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.text_secondary)
                        .child(label),
                )
                .when(collapsed, |d| {
                    // Collapsed summary: tab count, accent if it hides the active tab.
                    d.child(
                        div()
                            .text_size(px(11.))
                            .text_color(if has_active { ui.accent } else { ui.text_tertiary })
                            .child(format!("{count}")),
                    )
                }),
        );
        if collapsed {
            continue;
        }
        for ix in tabs {
            list = list.child(row(ws, ix, detail, cx));
        }
    }

    div()
        .id("sidebar")
        .w(px(WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(ui.bg_sidebar)
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.sidebar_hovered(*hovered, cx)))
        // Title-bar strip: room for the traffic lights, drags the window.
        .child(
            div()
                .h(px(38.))
                .flex_none()
                .on_mouse_down(MouseButton::Left, |ev, window, _| {
                    if ev.click_count == 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                }),
        )
        .child(list)
        .child(
            div()
                .id("new-tab")
                .h(px(32.))
                .flex_none()
                .mx(px(INSET))
                .mb(px(8.))
                .px(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(ui.text_tertiary)
                .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_secondary))
                .on_click(cx.listener(|this, _, window, cx| {
                    let cwd = this.tabs.get(this.active).and_then(|t| t.cwd.clone());
                    this.open_tab(cwd, None, window, cx);
                }))
                .child("+")
                .child("New tab")
                .child(div().flex_1())
                .child(div().text_size(px(11.)).child("⌘T")),
        )
        .into_any_element()
}

fn row(ws: &Workspace, ix: usize, detail: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let tab = &ws.tabs[ix];
    let selected = ix == ws.active;
    let running = tab.program.is_some();

    // Glyph slot (12×12). Agent states arrive in M4; for now a quiet dot marks a running program.
    let glyph = div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().when(running, |d| {
        d.child(div().size(px(6.)).rounded_full().bg(ui.status_working))
    });

    let title = div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(13.))
        .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
        .text_color(if selected { ui.text_primary } else { ui.text_secondary })
        .child(tab.title());

    let mut el = div()
        .id(SharedString::from(tab.id.clone()))
        .relative()
        .flex_none()
        .h(px(if detail { ROW_DETAIL } else { ROW_CALM }))
        .mt(px(ROW_GAP))
        .ml(px(INSET))
        .px(px(12.))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(2.))
        .child(div().flex().items_center().gap(px(9.)).child(glyph).child(title))
        .when(detail, |d| {
            d.child(
                div()
                    .pl(px(21.))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(ui.text_quiet)
                    .child(tab.subtitle()),
            )
        })
        .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
        .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.close_tab_at(ix, window, cx)));

    if selected {
        // Selected: filled with the terminal background, flush to the sidebar's right edge so the
        // row reads as part of the terminal; 3px accent bar inset 7px top and bottom.
        el = el
            .bg(ui.bg_terminal)
            .rounded_l(px(6.))
            .child(div().absolute().left_0().top(px(7.)).bottom(px(7.)).w(px(3.)).rounded(px(2.)).bg(ui.accent));
    } else {
        el = el.mr(px(INSET)).rounded(px(6.)).hover(move |d| d.bg(ui.bg_hover));
    }
    el.into_any_element()
}
