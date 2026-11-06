//! Calm sidebar: project groups with one-line tab rows; two-line rows on hover intent or ⌥.
//! Sizes and colors follow `docs/design.md` (v2: calm sidebar).

use std::time::Instant;

use gpui::{div, prelude::*, px, AnyElement, Context, Div, FontWeight, MouseButton, SharedString};
use t1k_agent::Status;
use t1k_themes::UiColors;

use crate::workspace::Tab;

use crate::workspace::Workspace;

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
        let renamed = ws.group_names.contains_key(&key);
        let folder = crate::groups::label(&key);
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
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                    this.toggle_group(toggle_key.clone(), cx);
                    if ev.click_count() == 2 {
                        // Second click undid the collapse; open rename instead.
                        let text = this.group_label(&toggle_key);
                        this.open_overlay(crate::overlay::Overlay::RenameGroup { key: toggle_key.clone(), text }, window, cx);
                    }
                }))
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
                .when(renamed, |d| {
                    // Renamed groups keep the real folder name visible.
                    d.child(div().flex_none().text_size(px(10.)).font_family("JetBrains Mono").text_color(ui.text_disabled).child(folder.clone()))
                })
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
        // Calm mode folds quiet tabs (nothing new, not selected) into one "+N quiet" row.
        let fold = !detail && !ws.show_quiet.contains(&key);
        let quiet: Vec<usize> = tabs.iter().copied().filter(|&i| ws.is_quiet(i)).collect();
        let fold = fold && quiet.len() >= 2;
        for ix in tabs {
            if fold && quiet.contains(&ix) {
                continue;
            }
            list = list.child(row(ws, ix, detail, cx));
        }
        if fold {
            let k = key.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("quiet-{}", key.display())))
                    .h(px(26.))
                    .flex_none()
                    .mt(px(ROW_GAP))
                    .mx(px(INSET))
                    .px(px(12.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .text_size(px(12.))
                    .text_color(ui.text_tertiary)
                    .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_secondary))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_quiet.insert(k.clone());
                        cx.notify();
                    }))
                    .child(div().w(px(12.)))
                    .child(format!("+{} quiet", quiet.len())),
            );
        } else if ws.show_quiet.contains(&key) && !detail && quiet.len() >= 2 {
            let k = key.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("unquiet-{}", key.display())))
                    .h(px(26.))
                    .flex_none()
                    .mt(px(ROW_GAP))
                    .mx(px(INSET))
                    .px(px(12.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .text_size(px(12.))
                    .text_color(ui.text_tertiary)
                    .hover(move |d| d.bg(ui.bg_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_quiet.remove(&k);
                        cx.notify();
                    }))
                    .child(div().w(px(12.)))
                    .child("Fold quiet tabs"),
            );
        }
    }

    div()
        .id("sidebar")
        .w(px(ws.config.sidebar.width.clamp(200., 480.)))
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
        .children(needs_bar(ws, cx))
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

    let glyph = glyph(tab, ui, ws.started, running);
    let waiting = match tab.status() {
        Some(Status::NeedsInput { .. }) | Some(Status::Error { .. }) => tab.agent.as_ref().map(|a| elapsed(a.since)),
        _ => None,
    };
    let unseen = tab.unread || tab.reminder_due;
    // A pending reminder shows a quiet clock after the title.
    let pending = (tab.reminder_at.is_some() && !tab.reminder_due).then(|| {
        div().flex_none().text_size(px(11.)).text_color(ui.text_tertiary).child("◷")
    });

    let title = div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(13.))
        .when(selected || unseen, |d| d.font_weight(FontWeight::SEMIBOLD))
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
        .child(
            div().flex().items_center().gap(px(9.)).child(glyph).child(title).children(pending).children(waiting.map(|w| {
                div().flex_none().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(w)
            })),
        )
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
        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
            if ev.click_count() == 2 {
                this.start_rename(ix, window, cx);
            } else {
                this.activate(ix, window, cx);
            }
        }))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
            this.open_overlay(crate::overlay::Overlay::Menu { tab: ix, at: ev.position }, window, cx);
        }))
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

/// Status glyph in a 12×12 slot. Shape differs per state so it reads without color.
/// Loud: NeedsInput (amber dot + halo), Error (red diamond), unread (accent dot).
/// Quiet: Working (gray spinner). Seen Done / Idle: nothing. Ended: dash.
fn glyph(tab: &Tab, ui: UiColors, started: Instant, running: bool) -> Div {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let slot = div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center();
    match tab.status() {
        Some(Status::NeedsInput { .. }) => slot.child(
            div().size(px(12.)).rounded_full().bg(ui.status_needs.opacity(0.22)).flex().items_center().justify_center()
                .child(div().size(px(7.)).rounded_full().bg(ui.status_needs)),
        ),
        Some(Status::Error { .. }) => slot.child(div().text_size(px(10.)).text_color(ui.status_error).child("◆")),
        _ if tab.reminder_due => slot.child(div().text_size(px(11.)).text_color(ui.accent).child("◷")),
        _ if tab.unread => slot.child(div().size(px(7.)).rounded_full().bg(ui.accent)),
        Some(Status::Working) => {
            let frame = (started.elapsed().as_millis() / 100) as usize % SPINNER.len();
            slot.child(div().text_size(px(12.)).text_color(ui.status_working).child(SPINNER[frame]))
        }
        Some(Status::Ended) => slot.child(div().w(px(7.)).h(px(1.5)).bg(ui.text_disabled)),
        Some(Status::Done) | Some(Status::Idle) => slot,
        None if running => slot.child(div().size(px(5.)).rounded_full().bg(ui.status_working)),
        None => slot,
    }
}

fn elapsed(since: Instant) -> String {
    let s = since.elapsed().as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

/// "Needs you N · ⌘J" bar: the collapsed attention queue.
fn needs_bar(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let n = ws.needs_queue().len();
    if n == 0 {
        return None;
    }
    let ui = ws.ui;
    Some(
        div()
            .id("needs-you")
            .h(px(30.))
            .flex_none()
            .mx(px(INSET))
            .mb(px(8.))
            .px(px(10.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .bg(ui.status_needs.opacity(0.12))
            .hover(move |d| d.bg(ui.status_needs.opacity(0.2)))
            .text_size(px(12.))
            .on_click(cx.listener(|this, _, window, cx| this.jump_next_needs(window, cx)))
            .child(div().size(px(7.)).rounded_full().bg(ui.status_needs))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Needs you"))
            .child(div().text_color(ui.text_secondary).child(format!("{n}")))
            .child(div().flex_1())
            .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("⌘J"))
            .into_any_element(),
    )
}
