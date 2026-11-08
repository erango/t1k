//! The plan usage meter (canvas v3-t, option 5): two thin bars beside ‹ › in the sidebar's top
//! strip (or in the session strip while the sidebar is hidden), gray while low, amber from 80%,
//! red from 95%, with a popover showing every window and when it resets.
//!
//! Limits are per account, so the meter belongs to the window, not to a tab. The data comes from
//! the agents' status lines and Codex's rollout file (`t1k_agent::statusline`).

use gpui::{div, prelude::*, px, AnyElement, Context, Div, FontWeight, Hsla, MouseButton, SharedString, Window as GWindow};
use t1k_agent::agents::AgentKind;
use t1k_agent::statusline::{Usage, Window};
use t1k_themes::UiColors;

use crate::workspace::Workspace;

/// Older than this, the numbers may be out of date: no agent has reported since.
const STALE_SECS: i64 = 600;
const MONO: &str = "JetBrains Mono";

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Agents with limits, in a stable order (Claude first).
fn reported(ws: &Workspace) -> Vec<(AgentKind, Usage)> {
    AgentKind::ALL.into_iter().filter_map(|k| ws.usage.get(&k).map(|u| (k, u.clone()))).collect()
}

fn level(ui: UiColors, used: f32) -> Hsla {
    if used >= 95. {
        ui.status_error
    } else if used >= 80. {
        ui.status_needs
    } else {
        ui.status_working
    }
}

/// Short form for the label beside the bars.
fn short(name: &str) -> String {
    match name {
        "3p-weekly" => "Other".into(),
        n => match n.strip_suffix("-weekly") {
            Some(m) => capitalize(m),
            None => n.into(),
        },
    }
}

/// Long form for the popover.
fn long(name: &str) -> String {
    match name {
        "5h" => "5-hour limit".into(),
        "7d" => "Weekly limit".into(),
        "3p-weekly" => "Other models · weekly".into(),
        n => match n.strip_suffix("-weekly") {
            Some(m) => format!("{} · weekly", capitalize(m)),
            None => n.into(),
        },
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Local time pieces: (weekday 0=Sun, hour, minute, days since epoch).
fn local(t: i64) -> (i32, i32, i32, i64) {
    unsafe {
        let tt = t as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tt, &mut tm);
        (tm.tm_wday, tm.tm_hour, tm.tm_min, (t + tm.tm_gmtoff).div_euclid(86400))
    }
}

/// "3:10pm", or "Fri 9:07am" when it isn't today.
pub fn clock(t: i64) -> String {
    let (wday, h, m, day) = local(t);
    let ampm = if h < 12 { "am" } else { "pm" };
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let time = format!("{h12}:{m:02}{ampm}");
    if day == local(now()).3 {
        time
    } else {
        let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        format!("{} {time}", names[wday.clamp(0, 6) as usize])
    }
}

/// "2h 05m", "6d 14h", "12m".
pub fn span(secs: i64) -> String {
    let m = secs.max(0) / 60;
    if m >= 1440 {
        format!("{}d {}h", m / 1440, m % 1440 / 60)
    } else if m >= 60 {
        format!("{}h {:02}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

fn ago(secs: i64) -> String {
    if secs < 60 {
        "just now".into()
    } else {
        format!("{} ago", span(secs))
    }
}

fn bar(ui: UiColors, w: f32, h: f32, used: f32) -> Div {
    div()
        .w(px(w))
        .h(px(h))
        .rounded(px(h / 2.))
        .bg(ui.text_primary.opacity(0.12))
        .child(div().h_full().w(px((w * used / 100.).clamp(0., w))).rounded(px(h / 2.)).bg(level(ui, used)))
}

/// The meter button. None while no agent has reported limits (nothing is drawn, nothing moves).
pub fn meter(ws: &Workspace, id: &'static str, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let data = reported(ws);
    let (_, primary) = data.first()?;
    let newest = data.iter().map(|(_, u)| u.at).max().unwrap_or(0);
    let stale = now() - newest > STALE_SECS;
    // The label follows the window closest to its limit, whichever agent it belongs to.
    let (hk, hw): (AgentKind, Window) = data
        .iter()
        .filter_map(|(k, u)| u.highest().map(|w| (*k, w.clone())))
        .max_by(|a, b| a.1.used.total_cmp(&b.1.used))?;
    let label = (hw.used >= 80. && !stale).then(|| {
        // Another agent's "5h" needs its name; a named bucket ("Gemini") says enough.
        let name = match hw.name.as_str() {
            n @ ("5h" | "7d") if hk != data[0].0 => format!("{} {n}", hk.label().split(' ').next().unwrap_or("")),
            n => short(n),
        };
        match hw.resets_at {
            Some(t) if hw.used >= 100. => clock(t),
            _ => format!("{name} {:.0}%", hw.used),
        }
    });
    let bars = primary.windows.iter().take(2).fold(div().flex().flex_col().gap(px(3.5)), |d, w| d.child(bar(ui, 22., 4., w.used)));
    let snapshot = data.clone();
    Some(
        div()
            .id(id)
            .h(px(24.))
            .min_w_0()
            .flex_shrink()
            .px(px(6.))
            .mr(px(2.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .gap(px(6.))
            .hover(move |d| d.bg(ui.sidebar_hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(bars.when(stale, |d| d.opacity(0.45)))
            .when(stale, |d| d.child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("◷")))
            .when_some(label, |d, l| d.child(div().min_w_0().truncate().font_family(MONO).text_size(px(11.)).text_color(level(ui, hw.used)).child(l)))
            .tooltip(move |_, cx| cx.new(|_| Tip { ui, data: snapshot.clone() }).into())
            .on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                let p = ev.position();
                let at = gpui::point(p.x - px(130.), p.y + px(16.));
                this.open_overlay(crate::overlay::Overlay::Usage { at }, window, cx);
            }))
            .into_any_element(),
    )
}

/// The hover popover (the same card the click pins).
struct Tip {
    ui: UiColors,
    data: Vec<(AgentKind, Usage)>,
}

impl Render for Tip {
    fn render(&mut self, _: &mut GWindow, _: &mut Context<Self>) -> impl IntoElement {
        card(self.ui, &self.data)
    }
}

pub fn card_for(ws: &Workspace) -> Div {
    card(ws.ui, &reported(ws))
}

fn card(ui: UiColors, data: &[(AgentKind, Usage)]) -> Div {
    let t = now();
    let newest = data.iter().map(|(_, u)| u.at).max().unwrap_or(t);
    let stale = t - newest > STALE_SECS;
    let title = match data {
        [(k, _)] => format!("{} usage", k.label().split(' ').next().unwrap_or("Plan")),
        _ => "Plan usage".to_string(),
    };
    let mut c = div()
        .w(px(260.))
        .p(px(12.))
        .rounded(px(10.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_md()
        .font_family(".SystemUIFont")
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(title));
    for (k, u) in data {
        if data.len() > 1 {
            c = c.child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(k.label()));
        }
        for w in &u.windows {
            let resets = w.resets_at.map(|r| format!("Resets {} · in {}", clock(r), span(r - t)));
            c = c.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().text_size(px(12.)).text_color(ui.text_secondary).child(long(&w.name)))
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if w.used >= 80. { level(ui, w.used) } else { ui.text_primary })
                                    .child(format!("{:.0}%", w.used)),
                            ),
                    )
                    .child(bar(ui, 236., 6., w.used))
                    .when_some(resets, |d, r| d.child(div().text_size(px(11.5)).text_color(ui.text_tertiary).child(r))),
            );
        }
    }
    let foot: SharedString = if stale {
        format!("As of {} · updates when an agent tab is running", ago(t - newest)).into()
    } else {
        format!("Account-wide · updated {}", ago(t - newest)).into()
    };
    c.child(div().h(px(1.)).bg(ui.border)).child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(foot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_spans() {
        assert_eq!(short("gemini-weekly"), "Gemini");
        assert_eq!(long("7d"), "Weekly limit");
        assert_eq!(long("3p-weekly"), "Other models · weekly");
        assert_eq!(span(2 * 3600 + 5 * 60), "2h 05m");
        assert_eq!(span(6 * 86400 + 14 * 3600), "6d 14h");
        assert_eq!(span(30), "0m");
    }
}
