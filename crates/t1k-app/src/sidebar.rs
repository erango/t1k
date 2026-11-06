//! Calm sidebar: project groups with one-line tab rows. Rows never change size: resting on a
//! row shows a floating detail card beside it, and holding ⌥ swaps titles for status lines.
//! Sizes and colors follow `docs/design.md` (v2: calm sidebar).

use std::time::Instant;

use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, Context, Div, FontWeight, MouseButton, SharedString};
use t1k_agent::Status;
use t1k_themes::UiColors;

use crate::workspace::Tab;

use crate::workspace::Workspace;

const CARD_W: f32 = 320.;
const ROW_GAP: f32 = 2.;
const HEADER: f32 = 28.;
const INSET: f32 = 8.;

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
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
                .gap(px(7.))
                .hover(move |d| d.bg(ui.bg_hover))
                .on_mouse_down(MouseButton::Right, cx.listener({
                    let key = key.clone();
                    move |this, ev: &gpui::MouseDownEvent, window, cx| {
                        this.open_overlay(crate::overlay::Overlay::GroupMenu { key: key.clone(), at: ev.position }, window, cx);
                    }
                }))
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                    this.toggle_group(toggle_key.clone(), cx);
                    if ev.click_count() == 2 {
                        // Second click undid the collapse; open rename instead.
                        let text = this.group_label(&toggle_key);
                        this.open_overlay(crate::overlay::Overlay::RenameGroup { key: toggle_key.clone(), text }, window, cx);
                    }
                }))
                .child(chevron(!collapsed, ui.text_disabled))
                .child(folder_icon(ws.group_color(&key).unwrap_or(ui.text_tertiary), ws.group_color(&key)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13. * k))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.text_primary)
                        .child(label),
                )
                .when(renamed, |d| {
                    // Renamed groups keep the real folder name visible.
                    d.child(div().flex_none().text_size(px(10. * k)).font_family("JetBrains Mono").text_color(ui.text_disabled).child(folder.clone()))
                })
                .map(|d| {
                    let tally = Tally::of(ws, &tabs);
                    if collapsed {
                        // Collapsed: a summary, loud part in amber first.
                        let (loud, quiet) = tally.summary();
                        d.child(
                            div()
                                .flex_none()
                                .flex()
                                .text_size(px(12. * k))
                                .when(!loud.is_empty(), |d| d.child(div().text_color(ui.status_needs).child(loud.clone())))
                                .when(!quiet.is_empty(), |d| {
                                    let sep = if loud.is_empty() { "" } else { " · " };
                                    d.child(div().text_color(if has_active { ui.accent } else { ui.text_tertiary }).child(format!("{sep}{quiet}")))
                                }),
                        )
                    } else {
                        // Open: counts of loud states only, glyph + number.
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
                }),
        );
        if collapsed {
            continue;
        }
        // Quiet tabs (nothing new, not selected) fold into one "+N quiet" row.
        let fold = !ws.show_quiet.contains(&key);
        let quiet: Vec<usize> = tabs.iter().copied().filter(|&i| ws.is_quiet(i)).collect();
        let fold = fold && quiet.len() >= 2;
        for ix in tabs {
            if fold && quiet.contains(&ix) {
                continue;
            }
            list = list.child(row(ws, ix, cx));
        }
        if fold {
            let gk = key.clone();
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
                    .text_size(px(12. * k))
                    .text_color(ui.text_tertiary)
                    .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_secondary))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_quiet.insert(gk.clone());
                        cx.notify();
                    }))
                    .child(div().w(px(12.)))
                    .child(div().flex_none().child(format!("+{} quiet", quiet.len())))
                    .child(div().flex_none().text_color(ui.text_disabled).child("·"))
                    .child(
                        div().flex_1().min_w_0().truncate().text_color(ui.text_disabled).child(
                            quiet.iter().map(|&i| ws.tabs[i].title()).collect::<Vec<_>>().join(", "),
                        ),
                    ),
            );
        } else if ws.show_quiet.contains(&key) && quiet.len() >= 2 {
            let gk = key.clone();
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
                    .text_size(px(12. * k))
                    .text_color(ui.text_tertiary)
                    .hover(move |d| d.bg(ui.bg_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_quiet.remove(&gk);
                        cx.notify();
                    }))
                    .child(div().w(px(12.)))
                    .child("Fold quiet tabs"),
            );
        }
        // Recent: Claude sessions from closed tabs, folded into one quiet row.
        let recent = ws.recent_for(&key);
        if !recent.is_empty() {
            let open = ws.recent_open.contains(&key);
            let rk = key.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("recent-{}", key.display())))
                    .h(px(26.))
                    .flex_none()
                    .mt(px(ROW_GAP))
                    .mx(px(INSET))
                    .px(px(12.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .text_size(px(12. * k))
                    .text_color(ui.text_tertiary)
                    .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_secondary))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.recent_open.remove(&rk) {
                            this.recent_open.insert(rk.clone());
                        }
                        cx.notify();
                    }))
                    .child(div().w(px(12.)).text_size(px(9. * k)).child(if open { "▼" } else { "▶" }))
                    .child(format!("Recent {}", recent.len())),
            );
            if open {
                for r in recent {
                    let sid = r.session_id.clone();
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("recent-{sid}")))
                            .h(px(28.))
                            .flex_none()
                            .mt(px(ROW_GAP))
                            .mx(px(INSET))
                            .pl(px(33.))
                            .pr(px(12.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(12. * k))
                            .text_color(ui.text_tertiary)
                            .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
                            .on_click(cx.listener(move |this, _, window, cx| this.reopen_recent(&sid, window, cx)))
                            .child(div().flex_1().min_w_0().truncate().child(r.title.clone()))
                            .child(div().flex_none().text_size(px(11. * k)).child(format!("↺ {}", ago(r.ended_at)))),
                    );
                }
            }
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
        // Title-bar strip: room for the traffic lights, drags the window.
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
                .child(
                    // Wordmark (design: mercury gradient; GPUI text is flat, so its light stop).
                    div()
                        .mr(px(6.))
                        .text_size(px(11.))
                        .font_family("JetBrains Mono")
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.text_tertiary)
                        .child("T1K"),
                )
                .child(nav_button(ws, true, cx))
                .child(nav_button(ws, false, cx))
                .child(
                    div()
                        .id("sidebar-view")
                        .size(px(24.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |d| d.bg(ui.bg_hover))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                            let p = ev.position();
                            let at = gpui::point(p.x - px(200.), p.y + px(14.));
                            this.open_overlay(crate::overlay::Overlay::SidebarView { at }, window, cx);
                        }))
                        .child(crate::icons::sliders(ui.text_secondary)),
                ),
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
                .text_size(px(12. * k))
                .text_color(ui.text_tertiary)
                .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_secondary))
                .on_click(cx.listener(|this, _, window, cx| {
                    let cwd = this.tabs.get(this.active).and_then(|t| t.cwd.clone());
                    this.open_tab(cwd, None, window, cx);
                }))
                .child("+")
                .child("New tab")
                .child(div().flex_1())
                .child(div().text_size(px(11. * k)).child("⌘T")),
        )
        .into_any_element()
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
        .flex_col()
        .justify_center()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(9.))
                .child(glyph)
                .when(icons, |d| d.child(crate::app_icon::badge(tab, ui)))
                .child(title)
                .children(pending)
                .children(waiting.map(|w| {
                    div().flex_none().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(w)
                })),
        )
        .when(two_lines, |d| {
            // Indented to the title: glyph slot 12 + gap 9, plus the badge 16 + gap 9.
            d.child(
                div()
                    .pl(px(if icons { 46. } else { 21. }))
                    .truncate()
                    .text_size(px(12. * k))
                    .text_color(if selected { ui.text_secondary } else { ui.text_quiet })
                    .child(second.unwrap_or_default()),
            )
        })
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.row_hovered(hover_id.clone(), *hovered, cx)))
        .when(carded, |d| {
            // Card floats over the terminal, level with the row; drawn last so nothing clips it.
            let left = ws.config.sidebar.width.clamp(200., 480.) - INSET + 6.;
            d.child(
                div().absolute().top(px(-4.)).left(px(left)).child(
                    deferred(anchored().snap_to_window_with_margin(px(8.)).child(card(tab, ui, k))).with_priority(1),
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
fn card(tab: &Tab, ui: UiColors, k: f32) -> impl IntoElement {
    let (status, color) = match tab.status() {
        Some(Status::NeedsInput { .. }) => ("Needs you", ui.status_needs),
        Some(Status::Error { .. }) => ("Error", ui.status_error),
        Some(Status::Working) => ("Working", ui.text_secondary),
        Some(Status::Done) if tab.unread => ("Done · unread", ui.accent),
        Some(Status::Done) => ("Done", ui.text_tertiary),
        Some(Status::Idle) => ("Idle", ui.text_tertiary),
        Some(Status::Ended) => ("Session ended", ui.text_tertiary),
        None if tab.program.is_some() => ("Running", ui.text_secondary),
        None => ("Shell", ui.text_tertiary),
    };
    let since = tab.agent.as_ref().filter(|a| a.status != Status::Idle).map(|a| elapsed(a.since));
    let status_line = match since {
        Some(s) => format!("{status} · {s}"),
        None => status.to_string(),
    };
    let body = tab.subtitle();
    let place = tab.cwd.as_deref().map(crate::groups::tilde).unwrap_or_default();
    let mut meta: Vec<String> = Vec::new();
    if let Some(a) = &tab.agent {
        if let Some(b) = &a.branch {
            meta.push(format!("⎇ {b}"));
        }
        if !a.prs.is_empty() {
            meta.push(a.prs.iter().map(|(n, _)| format!("#{n}")).collect::<Vec<_>>().join(" "));
        }
    }
    if tab.reminder_due {
        meta.push("◷ reminder due".into());
    } else if tab.reminder_at.is_some() {
        meta.push("◷ reminder set".into());
    }
    div()
        .w(px(CARD_W))
        .p(px(12.))
        .rounded(px(8.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().text_size(px(13. * k)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).line_clamp(2).child(tab.title()))
        .child(div().text_size(px(11. * k)).text_color(color).child(status_line))
        .when(!body.is_empty() && body != place, |d| {
            d.child(div().text_size(px(12. * k)).text_color(ui.text_secondary).line_clamp(4).child(body))
        })
        .when(!meta.is_empty(), |d| {
            d.child(div().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_tertiary).truncate().child(meta.join("  ")))
        })
        .when(!place.is_empty(), |d| {
            d.child(div().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_disabled).truncate().child(place))
        })
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

/// Disclosure chevron: points right when closed, down when open.
fn chevron(open: bool, color: gpui::Hsla) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (x, y) = (b.origin.x + px(4.5), b.origin.y + px(4.5));
            let mut p = gpui::PathBuilder::stroke(px(1.5));
            if open {
                p.move_to(gpui::point(x - px(3.), y - px(1.5)));
                p.line_to(gpui::point(x, y + px(1.5)));
                p.line_to(gpui::point(x + px(3.), y - px(1.5)));
            } else {
                p.move_to(gpui::point(x - px(1.5), y - px(3.)));
                p.line_to(gpui::point(x + px(1.5), y));
                p.line_to(gpui::point(x - px(1.5), y + px(3.)));
            }
            if let Ok(path) = p.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(9.))
    .flex_none()
}

/// Folder outline (14px). A group color fills it at 35%.
fn folder_icon(stroke: gpui::Hsla, fill: Option<gpui::Hsla>) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let o = b.origin;
            let pt = |x: f32, y: f32| gpui::point(o.x + px(x), o.y + px(y));
            let outline = [pt(1.5, 3.), pt(5.5, 3.), pt(6.7, 4.4), pt(12.5, 4.4), pt(12.5, 11.5), pt(1.5, 11.5)];
            if let Some(f) = fill {
                let mut p = gpui::PathBuilder::fill();
                p.add_polygon(&outline, true);
                if let Ok(path) = p.build() {
                    window.paint_path(path, f.opacity(0.35));
                }
            }
            let mut p = gpui::PathBuilder::stroke(px(1.2));
            p.add_polygon(&outline, true);
            if let Ok(path) = p.build() {
                window.paint_path(path, stroke);
            }
        },
    )
    .size(px(14.))
    .flex_none()
}

/// Working: a gray ¾ ring turning once every 900 ms (design: quiet, motion says busy).
pub fn spinner(color: gpui::Hsla, started: Instant) -> impl IntoElement {
    let turn = (started.elapsed().as_millis() % 900) as f32 / 900. * std::f32::consts::TAU;
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let c = bounds.center();
            let r = 4.25_f32;
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
    .size(px(10.))
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

/// "Needs you N · ⌘J" bar: the collapsed attention queue.
fn needs_bar(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let k = ws.config.interface.text_scale();
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
            .mb(px(6.))
            .pl(px(12.))
            .pr(px(10.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .gap(px(9.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .hover(move |d| d.bg(ui.bg_hover))
            .on_click(cx.listener(|this, _, window, cx| this.jump_next_needs(window, cx)))
            .child(
                div().size(px(13.)).flex().items_center().justify_center().rounded_full().bg(ui.status_needs.opacity(0.2))
                    .child(div().size(px(7.)).rounded_full().bg(ui.status_needs)),
            )
            .child(div().text_size(px(12.5 * k)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child("Needs you"))
            .child(div().text_size(px(11. * k)).font_family("JetBrains Mono").font_weight(FontWeight::SEMIBOLD).text_color(ui.status_needs).child(format!("{n}")))
            .child(div().flex_1())
            .child(div().text_size(px(11. * k)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child("⌘J"))
            .into_any_element(),
    )
}
