//! Calm sidebar: project groups with one-line tab rows. Rows never change size: resting on a
//! row shows a floating detail card beside it, and holding ⌥ swaps titles for status lines.
//! Sizes and colors follow `docs/design.md` (v2: calm sidebar).

use std::time::Instant;

use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, Context, Div, FontWeight, MouseButton, SharedString};
use t1k_agent::Status;
use t1k_themes::UiColors;

use crate::workspace::Tab;

use crate::workspace::Workspace;

const CARD_W: f32 = 380.;
const ROW_GAP: f32 = 2.;
/// What "quiet" means, shown on hover of the quiet labels.
const QUIET_HELP: &str = "Quiet tabs have nothing running and nothing new: idle shells and finished sessions you've already seen. Click to show them.";
const INSET: f32 = 8.;
/// Rows start here (design v3-y c): the group line runs at x 8 in the gutter left of them.
const ROW_LEFT: f32 = 14.;
/// Group line x (design v3-y c), where the rows' left edge used to be.
const LINE_X: f32 = 8.;

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let groups = ws.grouped();
    let dragging = ws.dragging_tab.is_some() && cx.has_active_drag();

    // Groups are separated by space alone (v3: no hairlines).
    let mut list = div()
        .id("sidebar-list")
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&ws.sidebar_scroll)
        // Dragging a tab or group near the top or bottom edge scrolls the list.
        .on_drag_move::<TabDrag>(cx.listener(|this, e: &gpui::DragMoveEvent<TabDrag>, _, cx| {
            this.drag_near_edge(f32::from(e.event.position.y), f32::from(e.bounds.top()), f32::from(e.bounds.bottom()), cx)
        }))
        .on_drag_move::<GroupDrag>(cx.listener(|this, e: &gpui::DragMoveEvent<GroupDrag>, _, cx| {
            this.drag_near_edge(f32::from(e.event.position.y), f32::from(e.bounds.top()), f32::from(e.bounds.bottom()), cx)
        }))
        .pb(px(12.))
        .flex()
        .flex_col()
        .gap(px(14.));

    // Sticky header: the group whose header has scrolled off while its tabs still show keeps a
    // copy pinned at the top, pushed up by the next group's header. Bounds are last frame's.
    let sticky = {
        let sh = &ws.sidebar_scroll;
        let (off, top) = (sh.offset().y, sh.bounds().top());
        let h = px(26.);
        groups.iter().enumerate().find_map(|(i, (key, tabs))| {
            let b = sh.bounds_for_item(i)?;
            let (y, bottom) = (b.top() + off, b.bottom() + off);
            (y < top && bottom > top + px(8.)).then(|| (key.clone(), tabs.clone(), (bottom - top - h).min(px(0.))))
        })
    };

    for (key, tabs) in groups {
        let tally = Tally::of(ws, &tabs);
        let has_active = tabs.contains(&ws.active);
        let all_quiet = !has_active && tabs.iter().all(|&i| ws.is_quiet(i));
        let folded = ws.collapsed.contains(&key) || (all_quiet && !ws.show_quiet.contains(&key));
        let _ = &tally;
        let mut group = div().relative().flex().flex_col();
        // Design v3-y c: a 2 px line in the group's color down its rows, in its own gutter at
        // x 8 (off the window edge), from just under the header to the last row. A plain guide:
        // it ignores selection, since the selected fill joining the terminal already says it.
        if let Some(c) = ws.group_color(&key).filter(|_| !folded) {
            let a = if ws.theme.is_dark() { 0.5 } else { 0.55 };
            group = group.child(div().absolute().left(px(LINE_X)).top(px(28.)).bottom_0().w(px(2.)).rounded(px(1.)).bg(c.opacity(a)));
        }
        group = group.child(group_header(ws, &key, &tabs, false, cx));
        if !folded {
            // Mixed groups fold their quiet tabs into one "+N quiet" row.
            let quiet: Vec<usize> = tabs.iter().copied().filter(|&i| ws.is_quiet(i)).collect();
            let fold = !all_quiet && !ws.show_quiet.contains(&key) && quiet.len() >= 2;
            for &ix in &tabs {
                if fold && quiet.contains(&ix) {
                    continue;
                }
                group = group.child(row(ws, ix, cx));
            }
            if fold {
                let gk = key.clone();
                group = group.child(
                    quiet_row(ui, k, format!("quiet-{}", key.display()))
                        .tooltip(crate::tooltip::text(ui, QUIET_HELP))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_quiet.insert(gk.clone());
                            cx.notify();
                        }))
                        .child(div().flex_none().child(format!("+{} quiet", quiet.len())))
                        .child(div().flex_none().text_color(ui.text_disabled).child("·"))
                        .child(
                            div().flex_1().min_w_0().truncate().text_color(ui.text_disabled).child(
                                quiet.iter().map(|&i| ws.tabs[i].title()).collect::<Vec<_>>().join(", "),
                            ),
                        ),
                );
            }
            // Recent: Claude sessions from closed tabs, folded into one quiet row.
            let recent = ws.recent_for(&key);
            if !recent.is_empty() {
                let open = ws.recent_open.contains(&key);
                let rk = key.clone();
                group = group.child(
                    quiet_row(ui, k, format!("recent-{}", key.display()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.recent_open.remove(&rk) {
                                this.recent_open.insert(rk.clone());
                            }
                            cx.notify();
                        }))
                        .child(format!("Recent {}", recent.len()))
                        .child(div().text_color(ui.text_disabled).child(if open { "⌄" } else { "›" })),
                );
                if open {
                    for r in recent {
                        let sid = r.session_id.clone();
                        group = group.child(
                            quiet_row(ui, k, format!("recent-{sid}"))
                                .h(px(28.))
                                .hover(move |d| d.bg(ui.sidebar_hover).text_color(ui.text_primary))
                                .on_click(cx.listener(move |this, _, window, cx| this.reopen_recent(&sid, window, cx)))
                                .child(div().flex_1().min_w_0().truncate().child(r.title.clone()))
                                .child(div().flex_none().text_size(px(11. * k)).child(format!("↺ {}", ago(r.ended_at)))),
                        );
                    }
                }
            }
        }
        list = list.child(group);
    }

    let field = |id: &'static str| {
        div()
            .id(id)
            .h(px(30.))
            .rounded(px(7.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .flex()
            .items_center()
            .hover(move |d| d.bg(ui.sidebar_hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };

    let width = match ws.sidebar_shown() {
        crate::rail::Shown::Full(w) => w,
        _ => ws.config.sidebar.width.clamp(crate::rail::MIN_W, crate::rail::MAX_W),
    };
    div()
        .id("sidebar")
        .relative()
        .w(px(width))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(ui.bg_sidebar)
        // The edge hairline is drawn first and under everything, so the selected row (and its
        // curves) can paint over it where it joins the terminal; a border can't be covered.
        .child(div().absolute().top_0().bottom_0().right_0().w(px(1.)).bg(ui.sidebar_edge))
        // Title-bar strip: traffic lights, then back / forward; drags the window. 44 tall like
        // the title bar, so lights, icons and the title share one center (22).
        .child(
            div()
                .h(px(44.))
                .relative()
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .children(crate::restore::progress_bar(ws))
                // Clear of the traffic lights: on a narrow sidebar the usage label shrinks first.
                .pl(px(74.))
                .pr(px(10.))
                .gap(px(2.))
                .on_mouse_down(MouseButton::Left, |ev, window, _| {
                    if ev.click_count == 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                })
                .children(crate::update_ui::pill(ws, cx))
                // v3-f B: window-level actions as plain icons beside ‹ ›.
                .child(
                    ghost("sidebar-view", ui)
                        .on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                            let p = ev.position();
                            let at = gpui::point(p.x - px(30.), p.y + px(18.));
                            this.open_overlay(crate::overlay::Overlay::SidebarView { at }, window, cx);
                        }))
                        .tooltip(crate::tooltip::text(ui, "View options"))
                        .child(crate::icons::filter_lines_at(ui.text_secondary, 16.)),
                )
                .child(
                    ghost("new-tab", ui)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let cwd = this.tabs.get(this.active).and_then(|t| t.cwd.clone());
                            let anchor = (!this.tabs.is_empty()).then_some(this.active);
                            this.open_tab(cwd, None, window, cx);
                            this.place_new_tab(this.active, anchor);
                        }))
                        .tooltip(crate::tooltip::text(ui, "New tab  ⌘T"))
                        .child(crate::icons::plus_at(ui.text_secondary, 16.)),
                )
                .child(div().w(px(1.)).h(px(16.)).mx(px(3.)).bg(ui.border))
                .children(crate::usage::meter(ws, "usage-meter", cx))
                .child(nav_button(ws, true, cx))
                .child(nav_button(ws, false, cx))
                .children(crate::restore::progress_label(ws)),
        )
        // Find owns its row.
        .child(
            div()
                .flex_none()
                .flex()
                .gap(px(6.))
                .px(px(10.))
                // Canvas v3-x A: the field's top border sits on the title bar's hairline (y 43),
                // so the two read as one line.
                .mt(px(-1.))
                .pb(px(10.))
                .child(
                    field("find")
                        .flex_1()
                        .px(px(10.))
                        .gap(px(8.))
                        .text_size(px(12.5 * k))
                        .text_color(ui.text_tertiary)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_overlay(crate::overlay::Overlay::Switcher { query: String::new(), selected: 0 }, window, cx)
                        }))
                        .child(crate::icons::search(ui.text_tertiary))
                        .child(div().flex_1().child("Find tab"))
                        .child(div().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_disabled).child("⌘P")),
                )
        )
        .children(attention(ws, cx))
        // Peeking from the rail, the edge isn't draggable (the rail's is).
        .when(!ws.peek, |d| d.child(crate::rail::edge(ws, cx)))
        .child(
            div().relative().flex_1().min_h_0().child(list).children(sticky.map(|(key, tabs, dy)| {
                div()
                    .absolute()
                    .top(dy)
                    .left_0()
                    .right_0()
                    .pb(px(4.))
                    .bg(ui.bg_sidebar)
                    .border_r_1()
                    .border_color(ui.sidebar_edge)
                    .shadow_sm()
                    .child(group_header(ws, &key, &tabs, true, cx))
            })),
        )
        .when(!dragging, |d| {
            d.children(crate::close::toast(ws, cx)).children(crate::import_ui::notice(ws, cx)).children(crate::import_ui::offer(ws, cx))
        })
        // While a tab is dragged: a place at the bottom to drop it out of its group, back into
        // its folder's project group (made if there isn't one).
        .when(dragging, |d| {
            d.child(
                div()
                    .id("pop-out")
                    .absolute()
                    .left(px(10.))
                    .right(px(10.))
                    .bottom(px(10.))
                    .h(px(44.))
                    .rounded(px(8.))
                    .bg(ui.bg_sidebar)
                    .border_1()
                    .border_dashed()
                    .border_color(ui.text_disabled)
                    .shadow_lg()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(ui.text_secondary)
                    .drag_over::<TabDrag>(move |s, _, _, _| s.bg(ui.accent.opacity(0.18)).border_color(ui.accent))
                    .on_drop(cx.listener(|this, d: &TabDrag, _, cx| this.pop_out_undoable(&d.0, cx)))
                    .child("Drop here to move it to its own project"),
            )
        })
        .into_any_element()
}

/// A group's header row. `sticky` draws the copy pinned to the top of the list.
fn group_header(ws: &Workspace, key: &std::path::Path, tabs: &[usize], sticky: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let key = key.to_path_buf();
    let tally = Tally::of(ws, tabs);
    let has_active = tabs.contains(&ws.active);
    // v3: a group with nothing going on (all quiet, none selected) is one line until opened.
    let all_quiet = !has_active && tabs.iter().all(|&i| ws.is_quiet(i));
    let auto_folded = all_quiet && !ws.show_quiet.contains(&key);
    let folded = ws.collapsed.contains(&key) || auto_folded;
    let label = ws.group_label(&key);
    let renamed = ws.group_names.contains_key(&key) && !crate::groups::is_custom(&key);
    let folder = crate::groups::label(&key);
    let click_key = key.clone();
    let (drag_key, drop_key) = (key.clone(), key.clone());
    let ghost_label = label.clone();
    div()
        .id(SharedString::from(format!("group-{}{}", if sticky { "s-" } else { "" }, key.display())))
        // Drag a header onto another group's header to move all its tabs there; onto the gap
        // above a header to reorder groups (a line shows where it will land). Tabs dropped on a
        // header join that group.
        .on_drag(GroupDrag(drag_key), move |_, _, _, cx| cx.new(|_| GroupGhost { label: ghost_label.clone().into(), ui }))
        .drag_over::<GroupDrag>(move |s, _, _, _| s.bg(ui.accent.opacity(0.18)).rounded(px(6.)))
        .drag_over::<TabDrag>(move |s, _, _, _| s.bg(ui.accent.opacity(0.18)).rounded(px(6.)))
        .on_drop(cx.listener({
            let to = drop_key.clone();
            move |this, d: &GroupDrag, _, cx| {
                if d.0 != to {
                    let ids: Vec<String> = this.tabs.iter().filter(|t| t.group == d.0).map(|t| t.id.clone()).collect();
                    this.move_tabs_undoable(&ids, &to, cx);
                }
            }
        }))
        .on_drop(cx.listener({
            let to = drop_key.clone();
            move |this, d: &TabDrag, _, cx| this.move_tabs_undoable(std::slice::from_ref(&d.0), &to, cx)
        }))
        .when(!sticky, |d| {
            let to = drop_key.clone();
            d.child(
                div()
                    .id(SharedString::from(format!("reorder-{}", key.display())))
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-10.))
                    .h(px(9.))
                    .flex()
                    .items_center()
                    .drag_over::<GroupDrag>(move |s, _, _, _| s.border_b_2().border_color(ui.accent))
                    .on_drop(cx.listener(move |this, d: &GroupDrag, _, cx| this.reorder_group(&d.0, &to, cx))),
            )
        })
        .group(SharedString::from(format!("gh-{}", key.display())))
        .relative()
        .h(px(26.))
        .flex_none()
        .mx(px(INSET))
        // Label at x 24, lined up with the row icons.
        .pl(px(16.))
        .pr(px(10.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(9.))
        .hover(move |d| d.bg(ui.sidebar_hover))
        .when(auto_folded, |d| d.tooltip(crate::tooltip::text(ui, QUIET_HELP)))
        .on_hover(cx.listener({
            let key = key.clone();
            move |this, over: &bool, _, cx| this.group_hovered(&key, *over, cx)
        }))
        .on_mouse_down(MouseButton::Right, cx.listener({
            let key = key.clone();
            move |this, ev: &gpui::MouseDownEvent, window, cx| {
                this.open_overlay(crate::overlay::Overlay::GroupMenu { key: key.clone(), at: ev.position }, window, cx);
            }
        }))
        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
            if ev.click_count() == 2 {
                let text = this.group_label(&click_key);
                this.open_overlay(crate::overlay::Overlay::RenameGroup { key: click_key.clone(), text }, window, cx);
                return;
            }
            // A quiet group opens by showing its quiet tabs; otherwise click collapses.
            if all_quiet && !this.collapsed.contains(&click_key) {
                if !this.show_quiet.remove(&click_key) {
                    this.show_quiet.insert(click_key.clone());
                }
                cx.notify();
            } else {
                this.toggle_group(click_key.clone(), cx);
            }
        }))
        .child(icon_slot(ws, &key, sticky, cx))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(12. * k))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if folded { ui.text_secondary } else { ui.text_primary })
                .child(label),
        )
        .when(renamed, |d| {
            // Renamed groups keep the real folder name visible.
            d.child(div().flex_none().text_size(px(10. * k)).font_family("JetBrains Mono").text_color(ui.text_disabled).child(folder.clone()))
        })
        // A dim disclosure caret after the name (as in Claude desktop): ▾ open, ▸ folded.
        .child(crate::icons::caret(ui.text_tertiary, !folded))
        .child(div().flex_1())
        .map(|d| {
            if folded {
                // One line: loud part first in amber, then "N quiet ›".
                let (loud, quiet) = tally.summary();
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .gap(px(4.))
                        .text_size(px(11.5 * k))
                        .when(!loud.is_empty(), |d| d.child(div().text_color(ui.status_needs).child(loud.clone())))
                        .when(!quiet.is_empty(), |d| d.child(div().text_color(ui.text_tertiary).child(quiet.clone()))),
                )
            } else {
                let mut agg = div().flex_none().flex().items_center().gap(px(9.));
                for (glyph, n) in tally.loud_glyphs(ui) {
                    agg = agg.child(
                        div().flex().items_center().gap(px(4.)).child(glyph).child(
                            div().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_quiet).child(n.to_string()),
                        ),
                    );
                }
                d.child(agg)
            }
        })
        // v3-f D: on hover, + (new tab in this project) and ⋯ (group menu).
        .child({
            let gid = SharedString::from(format!("gh-{}", key.display()));
            let k1 = key.clone();
            let k2 = key.clone();
            let small = |id: String| {
                div()
                    .id(SharedString::from(id))
                    .size(px(20.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(move |d| d.bg(ui.bg_control))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            };
            // Floats over the right end (covering the counts) so it takes no space.
            div()
                .absolute()
                .right(px(4.))
                .top(px(3.))
                .pl(px(6.))
                .flex()
                .gap(px(2.))
                .rounded(px(6.))
                .bg(ui.sidebar_hover)
                .invisible()
                .group_hover(gid, |s| s.visible())
                .child(
                    small(format!("gplus-{}", key.display()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            let cwd = k1.is_dir().then(|| k1.clone());
                            let anchor = (!this.tabs.is_empty()).then_some(this.active);
                            this.open_tab(cwd, None, window, cx);
                            this.place_new_tab(this.active, anchor);
                        }))
                        .tooltip(crate::tooltip::text(ui, "New tab in this project"))
                        .child(crate::icons::plus(ui.text_secondary)),
                )
                .child(
                    small(format!("gmore-{}", key.display()))
                        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_overlay(crate::overlay::Overlay::GroupMenu { key: k2.clone(), at: ev.position() }, window, cx);
                        }))
                        .text_size(px(13.))
                        .text_color(ui.text_tertiary)
                        .child("⋯"),
                )
        })
        .into_any_element()
}

/// A boxless 24 px icon button for the title strip.
fn ghost(id: &'static str, ui: UiColors) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .size(px(24.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .hover(move |d| d.bg(ui.sidebar_hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A low row inside a group (quiet fold, Recent).
fn quiet_row(ui: UiColors, k: f32, id: String) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .h(px(26.))
        .flex_none()
        .mt(px(ROW_GAP))
        .ml(px(ROW_LEFT))
        .mr(px(INSET))
        .pl(px(41.))
        .pr(px(12.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12. * k))
        .text_color(ui.text_tertiary)
        .hover(move |d| d.bg(ui.sidebar_hover).text_color(ui.text_secondary))
}

/// Group marker: filled with the group's color, else an outline ring.
/// The group's icon (an emoji you chose), or an empty slot the width of a tab's app icon so
/// group names line up with tab titles. Hovering the header shows a dashed placeholder;
/// clicking it opens the icon picker (design v3-n2).
fn icon_slot(ws: &Workspace, key: &std::path::Path, sticky: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let size = ws.config.interface.icon_size(false);
    let k = key.to_path_buf();
    let icon = ws.group_icons.get(key).cloned();
    // No icon: no room at rest, so names line up with icons on the left; hovering the
    // header slides a dashed placeholder in (design follow-up to v3-n2).
    let t = if icon.is_some() { 1. } else { ws.group_hover_progress(key) };
    let base = div()
        .id(SharedString::from(format!("gicon-{}{}", if sticky { "s-" } else { "" }, key.display())))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(crate::tooltip::text(ui, if icon.is_some() { "Change icon" } else { "Choose an icon" }))
        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
            cx.stop_propagation();
            this.open_icon_picker(crate::group_icons::IconFor::Group(k.clone()), ev.position(), window, cx);
        }));
    match icon {
        Some(e) => base.size(px(size)).text_size(px(size.min(18.) - 3.)).child(e),
        None => base
            .h(px(16.))
            .w(px(16. * t))
            // Cancel the header's 9 px gap while the slot is closed.
            .mr(px(-9. * (1. - t)))
            .overflow_hidden()
            .opacity(t)
            .child(
                div()
                    .size(px(16.))
                    .flex_none()
                    .rounded(px(4.))
                    .border_1()
                    .border_dashed()
                    .border_color(ui.text_disabled)
                    .flex()
                    .items_center()
                    .justify_center()
                    // A drawn plus: the text glyph sat low in the box.
                    .child(crate::icons::plus(ui.text_tertiary)),
            ),
    }
}

fn row(ws: &Workspace, ix: usize, cx: &mut Context<Workspace>) -> AnyElement {
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let tab = &ws.tabs[ix];
    let selected = ix == ws.active;
    let running = tab.program.is_some();

    let glyph = glyph(tab, ui, ws.started, running, k);
    let waiting = match tab.status() {
        Some(Status::NeedsInput { .. }) | Some(Status::Error { .. }) => tab.agent.as_ref().map(|a| elapsed(a.since)),
        _ => None,
    };
    let working = match tab.status() {
        Some(Status::Working) => tab.agent.as_ref().map(|a| elapsed(a.since)),
        _ => None,
    };
    let unseen = tab.unread || tab.reminder_due;
    // A pending reminder shows a quiet clock after the title.
    let pending = (tab.reminder_at.is_some() && !tab.reminder_due).then(|| {
        div().flex_none().text_size(px(11. * k)).text_color(ui.text_tertiary).child("◷")
    });

    // ⌥ swaps the title for the status line in place, so nothing moves.
    let alt = ws.alt_held;
    let title = div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(if alt { 12. } else { 13. } * k))
        .when(!alt && (selected || unseen), |d| d.font_weight(FontWeight::SEMIBOLD))
        .text_color(title_color(tab, selected, alt, ui))
        .child(if alt { tab.subtitle() } else { tab.title_as(&ws.config.sidebar.title) });
    let carded = ws.config.sidebar.hover_card
        && ws.card.as_deref() == Some(tab.id.as_str())
        && !alt
        && matches!(ws.overlay, crate::overlay::Overlay::None);
    // Optional always-on second line: every row gets it, so heights stay equal and nothing moves.
    let icons = ws.config.sidebar.app_icons;
    let second = tab.second_line(&ws.config.sidebar.second_line);
    let two_lines = ws.config.sidebar.second_line != "none";
    let height = if two_lines { ws.config.interface.two_line_height() } else { ws.config.interface.row_height() } * k.max(1.);
    let icon_size = ws.config.interface.icon_size(two_lines);
    let hover_id = tab.id.clone();
    let row_group = SharedString::from(format!("row-{}", tab.id));
    let confirming = ws.confirm_close.as_deref() == Some(tab.id.as_str());

    let (drag_id, drag_label, drop_group) = (tab.id.clone(), tab.title(), tab.group.clone());
    let (file_id, file_drop_id) = (tab.id.clone(), tab.id.clone());
    let file_hover = ws.drop_hover.as_ref().filter(|h| h.target == crate::drop::DropTarget::Row(tab.id.clone())).map(|h| h.paths.clone());
    let ws_handle = cx.entity().downgrade();
    let mut el = div()
        .id(SharedString::from(tab.id.clone()))
        // Drag a tab onto another group (its header or any of its tabs) to move it there.
        .on_drag(TabDrag(drag_id), move |d: &TabDrag, _, _, cx| {
            // Remember what's being dragged, so the sidebar can offer the pop-out place.
            let id = d.0.clone();
            let _ = ws_handle.update(cx, |ws, cx| {
                ws.dragging_tab = Some(id);
                cx.notify();
            });
            cx.new(|_| GroupGhost { label: drag_label.clone().into(), ui })
        })
        .drag_over::<TabDrag>(move |s, _, _, _| s.bg(ui.accent.opacity(0.12)))
        .on_drop(cx.listener(move |this, d: &TabDrag, _, cx| this.move_tabs_undoable(std::slice::from_ref(&d.0), &drop_group, cx)))
        // Files dropped on a row go to that tab (canvas v3-aa): accent fill and ring, "Insert path".
        .drag_over::<gpui::ExternalPaths>(move |s, _, _, _| s.bg(ui.accent.opacity(0.12)).border_1().border_color(ui.accent.opacity(0.55)).rounded(px(6.)))
        .on_drag_move::<gpui::ExternalPaths>(cx.listener(move |this, e: &gpui::DragMoveEvent<gpui::ExternalPaths>, _, cx| {
            let inside = e.bounds.contains(&e.event.position);
            let paths = e.drag(cx).clone();
            this.drop_hover(crate::drop::DropTarget::Row(file_id.clone()), inside, &paths, cx);
        }))
        .on_drop(cx.listener(move |this, paths: &gpui::ExternalPaths, window, cx| this.drop_on_row(&file_drop_id, paths, window, cx)))
        .when_some(file_hover, |d, paths| d.child(crate::drop::row_label(ui, &paths)))
        .group(row_group.clone())
        .relative()
        .flex_none()
        .h(px(height))
        .mt(px(ROW_GAP))
        .ml(px(ROW_LEFT))
        .pl(px(10.))
        .pr(px(12.))
        .flex()
        .items_center()
        .gap(px(if two_lines { 11. } else { 9. }))
        // v3: the program icon carries status in its corner (and spans both lines when there's
        // a second one); without icons, the glyph slot sits on the title line.
        // Not back yet after a quit: the icon waits at 55% (canvas v3-ab).
        .when(icons, |d| d.child(status_icon(tab, ui, ws.ui.bg_sidebar, selected, icon_size).when(ws.is_resuming(&tab.id), |d| d.opacity(0.55))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .justify_center()
                // Title and second line sit close, read as one item.
                .line_height(gpui::relative(1.25))
                .child(if confirming {
                    crate::close::confirm_row(ws, ix, k, cx)
                } else {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .when(!icons, |d| d.child(glyph))
                        .child(title)
                        // Still open in the terminal it came from (v3-v: neutral, not amber).
                        .children(tab.waiting.as_ref().map(|w| {
                            div().flex_none().text_size(px(11. * k)).text_color(ui.text_tertiary).child(format!("in {}", w.source.label()))
                        }))
                        .children(pending)
                        .children(crate::restore::row_note(ws, &tab.id, ui))
                        .children(waiting.map(|w| {
                            div().flex_none().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(w)
                        }))
                        // Design v3-q d: working tabs show a 12 px spinner and how long they've
                        // been at it, in the slot where needs-you shows its wait.
                        .children(working.map(|w| {
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(div().size(px(12.)).flex_none().child(spinner_sized(ui.status_working, ws.started, 4.75)))
                                .child(div().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(w))
                        }))
                        // Design v3-r a′: × after the right end, on hover.
                        .child(crate::close::close_button(ws, ix, row_group.clone(), cx))
                        .into_any_element()
                })
                .when(two_lines, |d| {
                    d.child(
                        div()
                            .when(!icons, |d| d.pl(px(21.)))
                            .truncate()
                            .text_size(px(12. * k))
                            .text_color(if selected { ui.text_secondary } else { ui.text_quiet })
                            .child(second.unwrap_or_default()),
                    )
                }),
        )
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.row_hovered(hover_id.clone(), *hovered, cx)))
        .when(carded, |d| {
            // Card floats over the terminal, level with the row; drawn last so nothing clips it.
            let left = ws.config.sidebar.width.clamp(200., 480.) - ROW_LEFT + 6.;
            d.child(
                div().absolute().top(px(-4.)).left(px(left)).child(
                    deferred(anchored().snap_to_window_with_margin(px(8.)).child(card(ws, ix, cx))).with_priority(1),
                ),
            )
        })
        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
            this.hide_card(cx);
            if ev.click_count() == 2 {
                this.start_rename(ix, window, cx);
            } else {
                this.activate(ix, window, cx);
            }
        }))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
            this.hide_card(cx);
            this.open_overlay(crate::overlay::Overlay::Menu { tab: ix, at: ev.position }, window, cx);
        }))
        .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.request_close(ix, window, cx)));

    if selected {
        // Selected: filled with the terminal background, flush to the sidebar's right edge so the
        // row reads as part of the terminal; no accent bar (the join says it, and a bar would
        // compete with the group line). Design v3-o C1: the row and the terminal are one surface. It reaches over the edge
        // hairline, and 8 px concave curves above and below flare it into the terminal.
        let fillet = |top: bool| {
            div()
                .absolute()
                .right_0()
                .when(top, |d| d.top(px(-R)))
                .when(!top, |d| d.bottom(px(-R)))
                .w(px(R))
                .h(px(R))
                .bg(ui.bg_terminal)
                .child(div().size(px(R)).bg(ui.bg_sidebar).when(top, |d| d.rounded_br(px(R))).when(!top, |d| d.rounded_tr(px(R))))
        };
        const R: f32 = 8.;
        el = el
            .bg(ui.bg_terminal)
            .rounded_l(px(R))
            .child(fillet(true))
            .child(fillet(false));
    } else {
        el = el.mr(px(INSET)).rounded(px(6.)).hover(move |d| d.bg(ui.sidebar_hover));
    }
    el.into_any_element()
}

/// Floating detail card for a hovered row: full title, status, what it's doing or its recap,
/// and where it lives.
/// Hover card (v3): what this tab is doing and whether to go there, without switching.
/// Needs you quotes Claude's question from the screen; Working shows the last output lines;
/// Done shows the recap. Chips only when true. Actions: the one that fits, then split, remind.
pub(crate) fn card(ws: &Workspace, ix: usize, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let k = ws.config.interface.text_scale();
    let tab = &ws.tabs[ix];
    let agent = tab.agent.as_ref();
    let (status, color) = match tab.status() {
        Some(Status::NeedsInput { .. }) => ("Needs you", ui.status_needs),
        Some(Status::Error { .. }) => ("Error", ui.status_error),
        Some(Status::Working) => ("Working", ui.text_secondary),
        Some(Status::Done) if tab.unread => ("Done · unread", ui.accent),
        Some(Status::Done) => ("Done", ui.status_done),
        Some(Status::Idle) => ("Idle", ui.text_tertiary),
        Some(Status::Ended) => ("Session ended", ui.text_tertiary),
        None if tab.program.is_some() => ("Running", ui.text_secondary),
        None => ("Shell", ui.text_tertiary),
    };
    let since = agent.filter(|a| a.status != Status::Idle).map(|a| elapsed(a.since));
    let why = match tab.status() {
        Some(Status::NeedsInput { .. }) => Some(needs_reason(tab)),
        Some(Status::Working) => agent.and_then(|a| a.now.clone()),
        Some(Status::Error { message }) => Some(message.chars().take(60).collect()),
        _ => None,
    };
    let needs = matches!(tab.status(), Some(Status::NeedsInput { .. }));
    let working = matches!(tab.status(), Some(Status::Working) | Some(Status::Error { .. }));
    // Screen excerpt: Claude's question while it waits, the last lines while it works.
    let screen = (needs || working).then(|| tab.view.read(cx).terminal.last_lines(if needs { 6 } else { 3 }));
    let recap = agent.and_then(|a| a.recap.clone()).filter(|_| !needs);
    let mut chips: Vec<(String, gpui::Hsla)> = Vec::new();
    if let Some(b) = tab.branch() {
        chips.push((format!("⎇ {b}"), ui.text_secondary));
    }
    if let Some(a) = agent {
        if let Some((n, url)) = a.prs.last() {
            let ci = ws.panel.prs.get(url).cloned().flatten().map(|i| {
                let (ok, bad, wait) = i.checks;
                if bad > 0 { format!(" ✗ {bad}") } else if wait > 0 { format!(" ● {wait}") } else if ok > 0 { format!(" ✓ {ok}") } else { String::new() }
            });
            chips.push((format!("#{n}{}", ci.unwrap_or_default()), ui.link));
        }
        if let Some(p) = a.context_percent().filter(|_| a.status != Status::Ended) {
            chips.push((format!("ctx {p}%"), if p >= 90 { ui.status_error } else if p >= 70 { ui.status_needs } else { ui.text_secondary }));
        }
        let files = a.files_edited();
        if files > 0 {
            chips.push((format!("{files} file{} edited", if files == 1 { "" } else { "s" }), ui.text_secondary));
        }
    }
    if tab.reminder_at.is_some() && !tab.reminder_due {
        chips.push(("◷ reminder set".into(), ui.text_tertiary));
    }
    let button = |id: &'static str, label: &'static str, primary: Option<gpui::Hsla>| {
        div()
            .id(id)
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(12. * k))
            .map(|d| match primary {
                Some(c) => d.bg(c).text_color(ui.bg_sidebar).font_weight(FontWeight::MEDIUM),
                None => d.bg(ui.bg_control).text_color(ui.text_primary),
            })
            .hover(|d| d.opacity(0.9))
            .child(label)
    };
    let id = tab.id.clone();
    div()
        .id("tab-card")
        .w(px(CARD_W))
        .p(px(14.))
        .rounded(px(10.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(10.))
        // Moving onto the card keeps it open.
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.row_hovered(id.clone(), *hovered, cx)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().text_size(px(14. * k)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).line_clamp(2).child(tab.title()))
                .child(div().flex_none().text_size(px(11.5 * k)).text_color(ui.text_tertiary).child(ws.group_label(&tab.group))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .text_size(px(12. * k))
                .child(div().size(px(8.)).rounded_full().bg(color))
                .child(div().flex_none().text_color(color).child(match &since {
                    Some(s) => format!("{status} · {s}"),
                    None => status.to_string(),
                }))
                .children(why.filter(|w| !w.is_empty()).map(|w| div().min_w_0().truncate().text_color(ui.text_tertiary).child(format!("· {w}")))),
        )
        .children(screen.filter(|l| !l.is_empty()).map(|lines| {
            let n = lines.len();
            div()
                .px(px(10.))
                .py(px(8.))
                .rounded(px(6.))
                .bg(ui.bg_sidebar)
                .border_1()
                .border_color(ui.border)
                .flex()
                .flex_col()
                .font_family("JetBrains Mono")
                .text_size(px(11.5))
                .children(lines.into_iter().enumerate().map(move |(j, l)| {
                    div().truncate().text_color(if needs && j + 1 == n { ui.status_needs } else { ui.text_secondary }).child(l)
                }))
        }))
        .children(recap.map(|r| div().text_size(px(12.5 * k)).text_color(ui.text_secondary).child(format!("Recap: {r}"))))
        .when(!chips.is_empty(), |d| {
            d.child(div().flex().flex_wrap().gap(px(6.)).children(chips.into_iter().map(|(t, c)| {
                div().px(px(6.)).py(px(2.)).rounded(px(4.)).bg(ui.bg_control).text_size(px(11.)).font_family("JetBrains Mono").text_color(c).child(t)
            })))
        })
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(if needs {
                    button("card-go", "Go to prompt ↵", Some(ui.status_needs)).on_click(cx.listener(move |this, _, window, cx| {
                        this.hide_card(cx);
                        this.activate(ix, window, cx);
                        this.go_to_prompt(window, cx);
                    }))
                } else {
                    button("card-open", "Open", None).on_click(cx.listener(move |this, _, window, cx| {
                        this.hide_card(cx);
                        this.activate(ix, window, cx);
                    }))
                })
                .child(button("card-split", "Open in split", None).on_click(cx.listener(move |this, _, window, cx| {
                    this.hide_card(cx);
                    this.activate(ix, window, cx);
                    this.open_split(ix, false, window, cx);
                })))
                .child(button("card-remind", "Remind me…", None).on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                    this.hide_card(cx);
                    this.open_overlay(crate::overlay::Overlay::Menu { tab: ix, at: ev.position() }, window, cx);
                }))),
        )
}

/// Status glyph in a 12×12 slot. Shape differs per state so it reads without color.
/// Loud: NeedsInput (amber dot + halo), Error (red diamond), unread (accent dot).
/// Quiet: Working (gray spinner). Seen Done / Idle: nothing. Ended: dash.
pub fn glyph(tab: &Tab, ui: UiColors, started: Instant, running: bool, k: f32) -> Div {
    let slot = div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center();
    match tab.status() {
        Some(Status::NeedsInput { .. }) => slot.child(
            div().size(px(12.)).rounded_full().bg(ui.status_needs.opacity(0.22)).flex().items_center().justify_center()
                .child(div().size(px(7.)).rounded_full().bg(ui.status_needs)),
        ),
        Some(Status::Error { .. }) => slot.child(div().text_size(px(10. * k)).text_color(ui.status_error).child("◆")),
        _ if tab.reminder_due => slot.child(div().text_size(px(11. * k)).text_color(ui.accent).child("◷")),
        _ if tab.unread => slot.child(div().size(px(7.)).rounded_full().bg(ui.accent)),
        Some(Status::Working) => slot.child(spinner(ui.status_working, started)),
        Some(Status::Ended) => slot.child(div().w(px(7.)).h(px(1.5)).bg(ui.text_disabled)),
        Some(Status::Done) | Some(Status::Idle) => slot,
        None if running => slot.child(div().size(px(5.)).rounded_full().bg(ui.status_working)),
        None => slot,
    }
}

/// ‹ / › in the sidebar's title strip: back and forward through visited tabs (⌘[ / ⌘]).
fn nav_button(ws: &Workspace, back: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let enabled = ws.can_navigate(back);
    div()
        .id(if back { "nav-back" } else { "nav-forward" })
        .size(px(24.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        // Don't drag the window from the buttons.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(enabled, |d| {
            d.hover(move |d| d.bg(ui.sidebar_hover))
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(back, window, cx)))
        })
        .child(crate::icons::chevron_at(if enabled { ui.text_secondary } else { ui.text_disabled }, back, 16.))
}

/// Title color: bright for selected / unread / due, one readable level for everything else.
/// State lives in the icon (badge, fade), not the title: with many settled tabs, graded title
/// fades turned the whole list gray and hard to scan.
fn title_color(tab: &Tab, selected: bool, alt: bool, ui: UiColors) -> gpui::Hsla {
    if alt {
        return ui.text_quiet;
    }
    if selected || tab.unread || tab.reminder_due {
        return ui.text_primary;
    }
    ui.text_secondary
}

/// Program icon (18 px) with a status badge on its corner: turning ring while working,
/// amber when it needs you, red on error, mercury when unread or a reminder is due. Ended and
/// idle sessions fade the icon instead.
fn status_icon(tab: &Tab, ui: UiColors, sidebar_bg: gpui::Hsla, selected: bool, size: f32) -> Div {
    let ring = if selected { ui.bg_terminal } else { sidebar_bg };
    // The status dot scales with the icon (never below 9 px, so small icons still show it).
    let d = (size * 0.42).round().max(9.);
    let dot = |c: gpui::Hsla| Some(div().size(px(d)).rounded_full().bg(c).border_2().border_color(ring));
    let corner = match tab.status() {
        Some(Status::NeedsInput { .. }) => dot(ui.status_needs),
        Some(Status::Error { .. }) => dot(ui.status_error),
        _ if tab.reminder_due || tab.unread => dot(ui.accent),
        // Working shows at the row's right end instead (design v3-q d): a corner spinner
        // this small couldn't be seen.
        _ => None,
    };
    // Idle, ended, and plain shells with nothing running fade the icon; the title stays readable.
    let quiet = !tab.unread
        && (matches!(tab.status(), Some(Status::Ended) | Some(Status::Idle)) || (tab.program.is_none() && tab.agent.is_none()));
    let off = -(d / 3.).round();
    div()
        .relative()
        .size(px(size))
        .flex_none()
        .child(crate::app_icon::badge(tab, ui, size).when(quiet || tab.waiting.is_some(), |d| d.opacity(if tab.waiting.is_some() { 0.5 } else { 0.45 })))
        .children(corner.map(|c| div().absolute().right(px(off)).bottom(px(off)).child(c)))
}

/// Per-group state counts for the header.
struct Tally {
    needs: usize,
    error: usize,
    unread: usize,
    due: usize,
    working: usize,
    quiet: usize,
}

impl Tally {
    fn of(ws: &Workspace, tabs: &[usize]) -> Self {
        let mut t = Tally { needs: 0, error: 0, unread: 0, due: 0, working: 0, quiet: 0 };
        for &i in tabs {
            let tab = &ws.tabs[i];
            match tab.status() {
                Some(Status::NeedsInput { .. }) => t.needs += 1,
                Some(Status::Error { .. }) => t.error += 1,
                _ if tab.reminder_due => t.due += 1,
                _ if tab.unread => t.unread += 1,
                Some(Status::Working) => t.working += 1,
                _ => t.quiet += 1,
            }
        }
        t
    }

    /// ("1 needs you", "3 working · 2 quiet") for a collapsed header.
    fn summary(&self) -> (String, String) {
        let mut loud = Vec::new();
        if self.needs > 0 {
            loud.push(format!("{} needs you", self.needs));
        }
        if self.error > 0 {
            loud.push(format!("{} error", self.error));
        }
        let mut quiet = Vec::new();
        if self.unread + self.due > 0 {
            quiet.push(format!("{} unread", self.unread + self.due));
        }
        if self.working > 0 {
            quiet.push(format!("{} working", self.working));
        }
        if self.quiet > 0 {
            quiet.push(format!("{} quiet", self.quiet));
        }
        (loud.join(" · "), quiet.join(" · "))
    }

    /// Glyph + count for each loud state present (open header, right side).
    fn loud_glyphs(&self, ui: UiColors) -> Vec<(Div, usize)> {
        let dot = |c: gpui::Hsla, size: f32| div().size(px(size)).rounded_full().bg(c);
        let mut v = Vec::new();
        if self.needs > 0 {
            v.push((dot(ui.status_needs, 7.), self.needs));
        }
        if self.error > 0 {
            v.push((div().text_size(px(9.)).text_color(ui.status_error).child("◆"), self.error));
        }
        if self.unread + self.due > 0 {
            v.push((dot(ui.accent, 6.), self.unread + self.due));
        }
        v
    }
}



/// Working: a gray ¾ ring turning once every 900 ms (design: quiet, motion says busy).
pub fn spinner(color: gpui::Hsla, started: Instant) -> impl IntoElement {
    spinner_sized(color, started, 4.25).size(px(10.))
}

/// The working ring at radius `r` (it fills its parent).
pub fn spinner_sized(color: gpui::Hsla, started: Instant, r: f32) -> gpui::Canvas<()> {
    let turn = (started.elapsed().as_millis() % 900) as f32 / 900. * std::f32::consts::TAU;
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let c = bounds.center();
            let mut b = gpui::PathBuilder::stroke(px(1.5));
            let steps = 24;
            for i in 0..=steps {
                let a = turn + i as f32 / steps as f32 * std::f32::consts::TAU * 0.75;
                let p = gpui::point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                if i == 0 { b.move_to(p) } else { b.line_to(p) }
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size_full()
}

/// "5m", "3h", "2d" since a unix time.
fn ago(unix: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(unix);
    let s = now.saturating_sub(unix);
    match s {
        0..=3599 => format!("{}m", (s / 60).max(1)),
        3600..=86399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
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

/// Why a tab is in the Needs-you list, in a few words.
pub fn needs_reason(tab: &Tab) -> String {
    match tab.status() {
        Some(Status::NeedsInput { reason }) => {
            if let Some(tool) = reason.split("permission to use ").nth(1) {
                format!("Approve {}", tool.trim().trim_end_matches('.'))
            } else if reason.contains("waiting for your input") {
                "Waiting for you".into()
            } else {
                reason.chars().take(32).collect()
            }
        }
        Some(Status::Error { .. }) => "Error".into(),
        _ if tab.reminder_due => "Reminder".into(),
        _ => String::new(),
    }
}

/// Top-of-sidebar attention area (v3):
/// - hooks not installed: a one-line "Connect Claude Code" banner;
/// - something needs you: "Needs you N · ⌘J" with one row per tab (what it wants, how long);
/// - otherwise, if any agents exist: a quiet "All caught up" row in the same place, so the
///   layout doesn't jump when the last one is handled.
fn attention(ws: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    out.extend(connect_banner(ws, cx));
    out.extend(needs_area(ws, cx));
    out
}

fn connect_banner(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let base = || div().flex_none().mx(px(10.)).mb(px(10.)).rounded(px(8.)).overflow_hidden();
    if !ws.hooks_installed {
        return Some(
            base()
                .id("connect")
                .h(px(30.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(ui.accent.opacity(0.08))
                .border_1()
                .border_color(ui.accent.opacity(0.25))
                .text_size(px(12. * k))
                .text_color(ui.text_secondary)
                .hover(move |d| d.bg(ui.accent.opacity(0.14)))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_overlay(crate::overlay::Overlay::Settings { section: crate::settings::Section::Agents }, window, cx)
                }))
                .child(div().flex_1().truncate().child("Connect Claude Code for live status"))
                .child(div().text_color(ui.accent).child("Set up"))
                .into_any_element(),
        );
    }
    None
}

fn needs_area(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    // Design v3-m option F: a row styled like a group header, not a box, so nothing under
    // Find reads as a second field. When tabs need you they list under it like a group's
    // tabs; the header line itself never moves.
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let queue = ws.needs_queue();
    let agents = ws.tabs.iter().filter(|t| t.agent.is_some()).count();
    if queue.is_empty() && agents == 0 {
        return None;
    }
    // "All caught up" only right after the last one is handled: shown a few seconds, then it
    // fades and folds away (when nothing is going on, it's noise).
    let fade = ws.caught_up_at.map(|at| {
        let past = at.elapsed().as_secs_f32() - crate::workspace::CAUGHT_UP_SHOW.as_secs_f32();
        (past / crate::workspace::CAUGHT_UP_FADE).clamp(0., 1.)
    });
    if queue.is_empty() && fade.is_none_or(|f| f >= 1.) {
        return None;
    }
    let keep = 1. - fade.unwrap_or(0.);
    let header = || {
        div()
            .id("needs-you")
            .h(px(26.))
            .flex_none()
            .mx(px(INSET))
            .px(px(10.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(7.))
            .hover(move |d| d.bg(ui.sidebar_hover))
    };
    let mut area = div().flex_none().mb(px(8.)).flex().flex_col().gap(px(2.));
    if queue.is_empty() {
        // Fading: the row folds its height away as it goes, so the list rises smoothly.
        let e = keep * keep * (3. - 2. * keep);
        area = div().flex_none().mb(px(8. * e)).h(px(26. * e)).overflow_hidden().opacity(e).flex().flex_col();
        let working = ws.tabs.iter().filter(|t| t.status() == Some(&Status::Working)).count();
        area = area.child(
            header()
                .tooltip(crate::tooltip::text(ui, "Nothing needs you. ⌘J jumps to the next tab that does."))
                .child(div().w(px(11.)).text_size(px(11. * k)).text_color(ui.status_done.opacity(0.75)).child("✓"))
                .child(div().flex_1().text_size(px(12. * k)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_secondary).child("All caught up"))
                .when(working > 0, |d| d.child(div().text_size(px(11.5 * k)).text_color(ui.text_tertiary).child(format!("{working} working")))),
        );
        return Some(area.into_any_element());
    }
    let n = queue.len();
    area = area.child(
        header()
            .on_click(cx.listener(|this, _, window, cx| this.jump_next_needs(window, cx)))
            .child(
                div().w(px(11.)).flex().justify_center().child(
                    div().size(px(13.)).flex().items_center().justify_center().rounded_full().bg(ui.status_needs.opacity(0.2))
                        .child(div().size(px(7.)).rounded_full().bg(ui.status_needs)),
                ),
            )
            .child(div().text_size(px(12. * k)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Needs you"))
            .child(div().text_size(px(11. * k)).font_family("JetBrains Mono").font_weight(FontWeight::SEMIBOLD).text_color(ui.status_needs).child(format!("{n}")))
            .child(div().flex_1())
            .child(div().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child("⌘J")),
    );
    for &i in queue.iter().take(4) {
        let t = &ws.tabs[i];
        let age = match t.status() {
            Some(Status::NeedsInput { .. }) | Some(Status::Error { .. }) => t.agent.as_ref().map(|a| elapsed(a.since)).unwrap_or_default(),
            _ => "due".into(),
        };
        let error = matches!(t.status(), Some(Status::Error { .. }));
        let tone = if error { ui.status_error } else { ui.status_needs };
        area = area.child(
            div()
                .id(SharedString::from(format!("needs-{}", t.id)))
                .h(px(28.))
                .flex_none()
                .mx(px(INSET))
                .pl(px(27.))
                .pr(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .hover(move |d| d.bg(ui.sidebar_hover))
                .on_click(cx.listener(move |this, _, window, cx| this.activate(i, window, cx)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.5 * k)).text_color(ui.text_primary).child(t.title()))
                .child(div().flex_none().text_size(px(11. * k)).text_color(tone).child(needs_reason(t)))
                .child(div().w(px(28.)).flex_none().flex().justify_end().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(tone).child(age)),
        );
    }
    if n > 4 {
        area = area.child(
            div().h(px(24.)).mx(px(INSET)).pl(px(27.)).flex().items_center().text_size(px(11.5 * k)).text_color(ui.text_tertiary).child(format!("+{} more · ⌘J walks them", n - 4)),
        );
    }
    Some(area.into_any_element())
}

/// A group header being dragged to a new place.
#[derive(Clone)]
pub struct GroupDrag(pub std::path::PathBuf);

/// A tab being dragged to another group (by tab id).
#[derive(Clone)]
pub struct TabDrag(pub String);

/// What follows the pointer while dragging a group: its name, lifted.
pub struct GroupGhost {
    label: SharedString,
    ui: UiColors,
}

impl gpui::Render for GroupGhost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        let ui = self.ui;
        div()
            .h(px(26.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .shadow_lg()
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ui.text_primary)
            .child(self.label.clone())
    }
}
