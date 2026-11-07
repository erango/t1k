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

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let groups = ws.grouped();

    // Groups are separated by space alone (v3: no hairlines).
    let mut list = div().id("sidebar-list").flex_1().min_h_0().overflow_y_scroll().pb(px(12.)).flex().flex_col().gap(px(14.));

    for (key, tabs) in groups {
        let tally = Tally::of(ws, &tabs);
        let has_active = tabs.contains(&ws.active);
        // v3: a group with nothing going on (all quiet, none selected) is one line until opened.
        let all_quiet = !has_active && tabs.iter().all(|&i| ws.is_quiet(i));
        let auto_folded = all_quiet && !ws.show_quiet.contains(&key);
        let folded = ws.collapsed.contains(&key) || auto_folded;
        let label = ws.group_label(&key);
        let renamed = ws.group_names.contains_key(&key);
        let folder = crate::groups::label(&key);
        let click_key = key.clone();
        let mut group = div().flex().flex_col();
        group = group.child(
            div()
                .id(SharedString::from(format!("group-{}", key.display())))
                .group(SharedString::from(format!("gh-{}", key.display())))
                .relative()
                .h(px(26.))
                .flex_none()
                .mx(px(INSET))
                .pl(px(12.))
                .pr(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .hover(move |d| d.bg(ui.bg_hover))
                .when(auto_folded, |d| d.tooltip(crate::tooltip::text(ui, QUIET_HELP)))
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
                .child(group_dot(ws.group_color(&key), ui))
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
                                .when(!quiet.is_empty(), |d| d.child(div().text_color(ui.text_tertiary).child(quiet.clone())))
                                .child(div().text_color(ui.text_disabled).child("›")),
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
                        .bg(ui.bg_hover)
                        .invisible()
                        .group_hover(gid, |s| s.visible())
                        .child(
                            small(format!("gplus-{}", key.display()))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    let cwd = k1.is_dir().then(|| k1.clone());
                                    this.open_tab(cwd, None, window, cx);
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
                }),
        );
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
                                .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
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
            .hover(move |d| d.bg(ui.bg_hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };

    div()
        .id("sidebar")
        .w(px(ws.config.sidebar.width.clamp(200., 480.)))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(ui.bg_sidebar)
        // Title-bar strip: traffic lights, then back / forward; drags the window.
        .child(
            div()
                .h(px(38.))
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
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
                        .child(crate::icons::filter_lines(ui.text_tertiary)),
                )
                .child(
                    ghost("new-tab", ui)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let cwd = this.tabs.get(this.active).and_then(|t| t.cwd.clone());
                            this.open_tab(cwd, None, window, cx);
                        }))
                        .tooltip(crate::tooltip::text(ui, "New tab  ⌘T"))
                        .child(crate::icons::plus(ui.text_secondary)),
                )
                .child(div().w(px(1.)).h(px(14.)).mx(px(4.)).bg(ui.border))
                .child(nav_button(ws, true, cx))
                .child(nav_button(ws, false, cx)),
        )
        // Find owns its row.
        .child(
            div()
                .flex_none()
                .flex()
                .gap(px(6.))
                .px(px(10.))
                .pt(px(2.))
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
        .child(list)
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
        .hover(move |d| d.bg(ui.bg_hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A low row inside a group (quiet fold, Recent).
fn quiet_row(ui: UiColors, k: f32, id: String) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .h(px(26.))
        .flex_none()
        .mt(px(ROW_GAP))
        .mx(px(INSET))
        .pl(px(43.))
        .pr(px(12.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12. * k))
        .text_color(ui.text_tertiary)
        .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_secondary))
}

/// Group marker: filled with the group's color, else an outline ring.
fn group_dot(color: Option<gpui::Hsla>, ui: UiColors) -> Div {
    let d = div().size(px(7.)).flex_none().rounded_full();
    match color {
        Some(c) => d.bg(c),
        None => d.border_1().border_color(ui.text_disabled),
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

    let mut el = div()
        .id(SharedString::from(tab.id.clone()))
        .relative()
        .flex_none()
        .h(px(height))
        .mt(px(ROW_GAP))
        .ml(px(INSET))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(if two_lines { 11. } else { 9. }))
        // v3: the program icon carries status in its corner (and spans both lines when there's
        // a second one); without icons, the glyph slot sits on the title line.
        .when(icons, |d| d.child(status_icon(tab, ui, ws.started, ws.ui.bg_sidebar, selected, icon_size)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .when(!icons, |d| d.child(glyph))
                        .child(title)
                        .children(pending)
                        .children(waiting.map(|w| {
                            div().flex_none().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(w)
                        })),
                )
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
            let left = ws.config.sidebar.width.clamp(200., 480.) - INSET + 6.;
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
        .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.close_tab_at(ix, window, cx)));

    if selected {
        // Selected: filled with the terminal background, flush to the sidebar's right edge so the
        // row reads as part of the terminal; 3px accent bar inset 7px top and bottom.
        el = el
            .bg(ui.bg_terminal)
            .rounded_l(px(6.))
            .child(div().absolute().left_0().top(px(7.)).bottom(px(7.)).w(px(3.)).rounded(px(2.)).bg(ws.group_color(&tab.group).unwrap_or(ui.accent)));
    } else {
        el = el.mr(px(INSET)).rounded(px(6.)).hover(move |d| d.bg(ui.bg_hover));
    }
    el.into_any_element()
}

/// Floating detail card for a hovered row: full title, status, what it's doing or its recap,
/// and where it lives.
/// Hover card (v3): what this tab is doing and whether to go there, without switching.
/// Needs you quotes Claude's question from the screen; Working shows the last output lines;
/// Done shows the recap. Chips only when true. Actions: the one that fits, then split, remind.
fn card(ws: &Workspace, ix: usize, cx: &mut Context<Workspace>) -> impl IntoElement {
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
        .children(recap.map(|r| div().text_size(px(12.5 * k)).text_color(ui.text_secondary).line_clamp(3).child(format!("Recap: {r}"))))
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
        .text_size(px(17.))
        .text_color(if enabled { ui.text_secondary } else { ui.text_disabled })
        // Don't drag the window from the buttons.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(enabled, |d| {
            d.hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(back, window, cx)))
        })
        .child(if back { "‹" } else { "›" })
}

/// Title color by state (design v2): bright for selected / unread / due, normal while
/// something is happening, quieter once done and seen, quietest when idle.
fn title_color(tab: &Tab, selected: bool, alt: bool, ui: UiColors) -> gpui::Hsla {
    if alt {
        return ui.text_quiet;
    }
    if selected || tab.unread || tab.reminder_due {
        return ui.text_primary;
    }
    match tab.status() {
        Some(Status::Done) => ui.text_quiet,
        Some(Status::Idle) | Some(Status::Ended) => ui.text_tertiary,
        _ if tab.program.is_none() && tab.agent.is_none() => ui.text_tertiary,
        _ => ui.text_secondary,
    }
}

/// Program icon (18 px) with a status badge on its corner: turning ring while working,
/// amber when it needs you, red on error, mercury when unread or a reminder is due. Ended and
/// idle sessions fade the icon instead.
fn status_icon(tab: &Tab, ui: UiColors, started: Instant, sidebar_bg: gpui::Hsla, selected: bool, size: f32) -> Div {
    let ring = if selected { ui.bg_terminal } else { sidebar_bg };
    // The status dot scales with the icon (never below 9 px, so small icons still show it).
    let d = (size * 0.42).round().max(9.);
    let dot = |c: gpui::Hsla| Some(div().size(px(d)).rounded_full().bg(c).border_2().border_color(ring));
    let corner = match tab.status() {
        Some(Status::NeedsInput { .. }) => dot(ui.status_needs),
        Some(Status::Error { .. }) => dot(ui.status_error),
        _ if tab.reminder_due || tab.unread => dot(ui.accent),
        Some(Status::Working) => Some(div().size(px(d + 1.)).rounded_full().bg(ring).flex().items_center().justify_center().child(spinner_sized(ui.status_working, started, (d - 2.) / 2.6))),
        _ => None,
    };
    let quiet = matches!(tab.status(), Some(Status::Ended) | Some(Status::Idle)) && !tab.unread;
    let off = -(d / 3.).round();
    div()
        .relative()
        .size(px(size))
        .flex_none()
        .child(crate::app_icon::badge(tab, ui, size).when(quiet, |d| d.opacity(0.45)))
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
/// - otherwise, if any agents exist: a quiet "All caught up · N working" line, so the layout
///   doesn't jump when the last one is handled.
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
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let base = || div().flex_none().mx(px(10.)).mb(px(10.)).rounded(px(8.)).overflow_hidden();
    let queue = ws.needs_queue();
    if queue.is_empty() {
        let agents = ws.tabs.iter().filter(|t| t.agent.is_some()).count();
        if agents == 0 {
            return None;
        }
        let working = ws.tabs.iter().filter(|t| t.status() == Some(&Status::Working)).count();
        let tail = if working > 0 { format!(" · {working} working") } else { String::new() };
        return Some(
            base()
                .h(px(30.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(ui.bg_terminal)
                .text_size(px(12. * k))
                .text_color(ui.text_tertiary)
                .child(div().text_color(ui.status_done).child("✓"))
                .child(format!("All caught up{tail}"))
                .into_any_element(),
        );
    }
    let n = queue.len();
    let mut bar = base().bg(ui.bg_raised).border_1().border_color(ui.status_needs.opacity(0.18)).flex().flex_col();
    bar = bar.child(
        div()
            .id("needs-you")
            .h(px(30.))
            .pl(px(12.))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(9.))
            .hover(move |d| d.bg(ui.bg_hover))
            .on_click(cx.listener(|this, _, window, cx| this.jump_next_needs(window, cx)))
            .child(
                div().size(px(13.)).flex().items_center().justify_center().rounded_full().bg(ui.status_needs.opacity(0.2))
                    .child(div().size(px(7.)).rounded_full().bg(ui.status_needs)),
            )
            .child(div().text_size(px(12.5 * k)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child("Needs you"))
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
        bar = bar.child(
            div()
                .id(SharedString::from(format!("needs-{}", t.id)))
                .h(px(28.))
                .pl(px(34.))
                .pr(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .border_t_1()
                .border_color(ui.border)
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, window, cx| this.activate(i, window, cx)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12. * k)).text_color(ui.text_secondary).child(t.title()))
                .child(div().flex_none().text_size(px(11. * k)).text_color(tone.opacity(0.85)).child(needs_reason(t)))
                .child(div().w(px(28.)).flex_none().flex().justify_end().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(tone).child(age)),
        );
    }
    if n > 4 {
        bar = bar.child(div().h(px(24.)).pl(px(34.)).flex().items_center().text_size(px(11.5 * k)).text_color(ui.text_tertiary).border_t_1().border_color(ui.border).child(format!("+{} more · ⌘J walks them", n - 4)));
    }
    Some(bar.into_any_element())
}
