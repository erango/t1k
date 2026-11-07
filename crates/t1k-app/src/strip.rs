//! Session strip (v3 title bar): what the selected tab is doing, only what's true right now.
//! State chip, title, project › branch, PR + CI; context meter, Open prompt when needed,
//! Outputs, and a ⋯ menu for the rest.

use std::time::Instant;

use gpui::{div, prelude::*, px, Context, FontWeight, MouseButton};
use t1k_agent::outputs::Kind;
use t1k_agent::Status;

use crate::overlay::Overlay;
use crate::workspace::Workspace;

fn ago(since: Instant) -> String {
    let s = since.elapsed().as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let tab = ws.tabs.get(ws.active);
    let title: String = tab.map(|t| t.title()).unwrap_or_default();
    let project = tab.map(|t| ws.group_label(&t.group)).unwrap_or_default();
    let branch = tab.and_then(|t| t.branch());
    let agent = tab.and_then(|t| t.agent.as_ref());
    let mono = |s: String, c: gpui::Hsla| div().flex_none().text_size(px(11.)).font_family("JetBrains Mono").text_color(c).child(s);

    // State chip: same vocabulary as the sidebar, in words.
    let chip = agent.map(|a| {
        let (state, tone, detail): (&str, gpui::Hsla, Option<String>) = match &a.status {
            Status::Working => ("Working", ui.text_secondary, a.now.clone()),
            Status::NeedsInput { .. } => ("Needs you", ui.status_needs, Some(crate::sidebar::needs_reason(tab.unwrap()))),
            Status::Error { message } => ("Error", ui.status_error, Some(message.chars().take(60).collect())),
            Status::Done => ("Done", ui.status_done, a.recap.clone()),
            Status::Idle => ("Idle", ui.text_quiet, a.recap.clone()),
            Status::Ended => ("Ended", ui.text_tertiary, None),
        };
        let loud = matches!(a.status, Status::NeedsInput { .. } | Status::Error { .. });
        let marker = match a.status {
            Status::Working => div().size(px(10.)).child(crate::sidebar::spinner_sized(ui.status_working, ws.started, 3.6)),
            Status::Idle | Status::Ended => div().size(px(8.)).rounded_full().border_1().border_color(tone),
            // Loud states get a soft halo.
            _ if loud => div().size(px(14.)).rounded_full().bg(tone.opacity(0.22)).flex().items_center().justify_center()
                .child(div().size(px(8.)).rounded_full().bg(tone)),
            _ => div().size(px(8.)).rounded_full().bg(tone),
        }
        .flex_none();
        // A: plain dot and words, no box (a pill read as a search field); a hairline before
        // the title.
        // Only the step text gives way when space runs out; the rest is clipped, never drawn
        // over the title.
        div()
            .flex_shrink()
            .min_w(px(140.))
            .max_w(px(440.))
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(7.))
            .child(marker)
            .child(div().flex_none().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(if loud { tone } else { ui.text_secondary }).child(state))
            .children(detail.filter(|d| !d.is_empty()).map(|d| {
                div().min_w_0().truncate().text_size(px(12.5)).text_color(ui.text_tertiary).child(format!("· {d}"))
            }))
            .child(mono(ago(a.since), if loud { tone } else { ui.text_tertiary }))
            .child(div().flex_none().ml(px(5.)).w(px(1.)).h(px(16.)).bg(ui.border))
    });
    // E: loud states tint the whole strip, with a 2 px top edge in their color.
    let alarm = agent.and_then(|a| match a.status {
        Status::NeedsInput { .. } => Some(ui.status_needs),
        Status::Error { .. } => Some(ui.status_error),
        _ => None,
    });

    // Context meter: gray, amber from 70%, red from 90%.
    let ctx = agent.filter(|a| a.status != Status::Ended).and_then(|a| a.context_percent()).map(|p| {
        let c = if p >= 90 { ui.status_error } else if p >= 70 { ui.status_needs } else { ui.text_quiet };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().w(px(44.)).h(px(4.)).rounded(px(2.)).bg(ui.border).child(div().w(px(44. * p as f32 / 100.)).h(px(4.)).rounded(px(2.)).bg(c)))
            .child(mono(format!("{p}%"), if p >= 70 { c } else { ui.text_tertiary }))
    });

    let needs = matches!(agent.map(|a| &a.status), Some(Status::NeedsInput { .. }));
    let outputs = ws.tabs.get(ws.active).map(|t| t.outputs_count()).unwrap_or(0);
    let fresh = ws.outputs_new(ws.active);
    let top = agent.and_then(|a| top_output(ws, a));

    div()
        .id("titlebar")
        .h(px(44.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.))
        .pr(px(12.))
        .pl(px(match ws.sidebar_shown() {
            crate::rail::Shown::Full(_) => 18.,
            // The traffic lights straddle the rail and the strip.
            crate::rail::Shown::Rail => 84. - crate::rail::RAIL_W,
            crate::rail::Shown::Hidden => 84.,
        }))
        .relative()
        .bg(ui.bg_terminal)
        .when_some(alarm, |d, c| {
            d.bg(gpui::Hsla::blend(ui.bg_terminal, c.opacity(0.06)))
                .child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(c))
        })
        // Design v3-n2: otherwise a 2 px line in the active tab's group color, so you know
        // which project you're typing in without looking at the sidebar.
        .when(alarm.is_none(), |d| {
            let c = ws.tabs.get(ws.active).and_then(|t| ws.group_color(&t.group));
            let a = if ws.theme.is_dark() { 0.85 } else { 0.9 };
            d.children(c.map(|c| div().absolute().top_0().left_0().right_0().h(px(2.)).bg(c.opacity(a))))
        })
        .border_b_1()
        .border_color(ui.border)
        .on_mouse_down(MouseButton::Left, |ev, window, _| {
            if ev.click_count == 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
        .children(chip)
        .child(div().flex_none().max_w(px(360.)).truncate().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(title.clone()))
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.))
                .text_color(ui.text_tertiary)
                .when(!project.eq_ignore_ascii_case(&title), |d| d.child(project.clone()))
                .when_some(branch, |d, b| {
                    d.when(!project.eq_ignore_ascii_case(&title), |d| d.child(div().text_color(ui.text_disabled).child("›")))
                        .child(div().text_size(px(11.5)).font_family("JetBrains Mono").child(b))
                }),
        )
        .child(div().flex_1())
        .children(ctx)
        .when(needs, |d| {
            d.child(
                div()
                    .id("open-prompt")
                    .flex_none()
                    .h(px(26.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .bg(ui.status_needs)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ui.bg_sidebar)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| this.go_to_prompt(window, cx)))
                    .child("Open prompt"),
            )
        })
        // With the sidebar hidden, the usage meter moves here.
        .children((!matches!(ws.sidebar_shown(), crate::rail::Shown::Full(_))).then(|| crate::usage::meter(ws, "usage-strip", cx)).flatten())
        .child(outputs_button(ws, outputs, fresh, top, cx))
        .child(
            div()
                .id("more")
                .flex_none()
                .w(px(28.))
                .h(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .text_size(px(16.))
                .text_color(ui.text_quiet)
                .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                    let p = ev.position();
                    this.open_overlay(Overlay::MoreMenu { at: gpui::point(p.x - px(220.), p.y + px(16.)) }, window, cx);
                }))
                .child("⋯"),
        )
}

impl Workspace {
    /// Needs you: focus the terminal at the bottom, where Claude's question is.
    pub fn go_to_prompt(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get(self.active) {
            let view = t.view.clone();
            view.update(cx, |v, cx| {
                v.terminal.scroll_to_bottom();
                cx.notify();
            });
            let h = gpui::Focusable::focus_handle(view.read(cx), cx);
            window.focus(&h);
        }
    }
}

/// The one output worth a direct link from the strip (design v3-l): an open PR with its
/// checks, then a merged PR, then an artifact or doc, then a ticket. Commits, files and
/// plain URLs never take the slot.
pub struct TopOutput {
    /// Small glyph before the label (CI state for PRs).
    glyph: Option<(&'static str, gpui::Hsla)>,
    kind: &'static str,
    label: String,
    url: String,
    tooltip: String,
    /// PRs: GitHub's color for the PR's state, on the "PR" label.
    state: Option<gpui::Hsla>,
}

fn top_output(ws: &Workspace, a: &t1k_agent::AgentState) -> Option<TopOutput> {
    let ui = ws.ui;
    let pr = |merged_wanted: bool| {
        a.prs.iter().rev().find_map(|(n, url)| {
            let info = ws.panel.prs.get(url).cloned().flatten();
            let merged = info.as_ref().is_some_and(|i| i.state == "MERGED");
            let closed = info.as_ref().is_some_and(|i| i.state == "CLOSED");
            if closed || merged != merged_wanted {
                return None;
            }
            let gh = crate::github::pr_state(info.as_ref().map(|i| i.state.as_str()).unwrap_or("OPEN"), info.as_ref().is_some_and(|i| i.draft)).1;
            let (glyph, tip) = match info.as_ref().map(|i| i.checks) {
                _ if merged => (("✓", crate::github::merged()), "merged".to_string()),
                Some((_, bad, _)) if bad > 0 => (("✗", crate::github::closed()), format!("{bad} checks failing")),
                Some((_, _, wait)) if wait > 0 => (("●", crate::github::pending()), "checks running".into()),
                Some((ok, _, _)) if ok > 0 => (("✓", crate::github::open()), "checks passed".into()),
                _ => (("●", ui.text_disabled), "no checks yet".into()),
            };
            Some(TopOutput { glyph: Some(glyph), kind: "PR", label: format!("#{n}"), url: url.clone(), tooltip: format!("Open PR #{n} · {tip}"), state: Some(gh) })
        })
    };
    let other = |kinds: &[Kind], name: &'static str| {
        a.outputs.iter().rev().find(|o| kinds.contains(&o.kind)).map(|o| {
            let label: String = o.label.chars().take(22).collect();
            TopOutput { glyph: None, kind: name, label, url: o.target.clone(), tooltip: format!("Open {}", o.label), state: None }
        })
    };
    pr(false).or_else(|| pr(true)).or_else(|| other(&[Kind::Artifact, Kind::Doc], "")).or_else(|| other(&[Kind::Ticket], ""))
}

/// H2: [tray + count | top output]. The left half opens the panel, the right half the output.
fn outputs_button(ws: &Workspace, n: usize, fresh: bool, top: Option<TopOutput>, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let open = ws.panel.open;
    // Each half rounds its own outer corners: overflow clipping is rectangular, so a fill
    // would otherwise show square corners inside the rounded border.
    let split = top.is_some();
    let left = div()
        .id("outputs-toggle")
        .h_full()
        .px(px(7.))
        .map(|d| if split { d.rounded_l(px(5.)) } else { d.rounded(px(5.)) })
        .flex()
        .items_center()
        .gap(px(5.))
        .when(open, |d| d.bg(ui.bg_selected))
        .hover(move |d| d.bg(ui.bg_selected))
        .tooltip(crate::tooltip::text(ui, if n == 0 { "Outputs · none yet".to_string() } else { format!("Outputs · {n}") }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _, _, cx| this.toggle_outputs(cx)))
        .child(crate::icons::tray(if n == 0 { ui.text_disabled } else if open { ui.text_primary } else { ui.text_secondary }))
        .when(n > 0, |d| {
            d.child(
                div()
                    .text_color(if fresh { ui.accent } else { ui.text_secondary })
                    .font_weight(if fresh { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                    .child(n.to_string()),
            )
        });
    let framed = n > 0;
    div()
        .flex_none()
        .h(px(26.))
        .rounded(px(6.))
        .overflow_hidden()
        .flex()
        .items_center()
        .text_size(px(12.))
        .line_height(px(14.))
        .when(framed, |d| d.border_1().border_color(ui.border))
        .child(left)
        .children(top.map(|t| {
            let url = t.url.clone();
            div()
                .flex()
                .items_center()
                .h_full()
                .child(div().w(px(1.)).h(px(14.)).bg(ui.border))
                .child(
                    div()
                        .id("top-output")
                        .h_full()
                        .rounded_r(px(5.))
                        .pl(px(7.))
                        .pr(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_color(if fresh { ui.accent } else { ui.text_secondary })
                        .hover(move |d| d.bg(ui.bg_selected).text_color(ui.text_primary))
                        .tooltip(crate::tooltip::text(ui, t.tooltip))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .children(t.glyph.map(|(g, c)| div().text_size(px(10.)).text_color(c).child(g)))
                        .when(!t.kind.is_empty(), |d| d.child(div().when_some(t.state, |d, c| d.text_color(c).font_weight(FontWeight::SEMIBOLD)).child(t.kind)))
                        .child(div().font_family(if t.kind == "PR" { "JetBrains Mono" } else { ".SystemUIFont" }).when(t.kind == "PR", |d| d.text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD)).child(t.label)),
                )
        }))
}
