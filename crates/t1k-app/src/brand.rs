//! The T1K lockup (pixel face + chrome wordmark) and the About / Welcome overlays.

use std::sync::{Arc, LazyLock};

use gpui::{div, img, prelude::*, px, AnyElement, Context, FontWeight, Image, ImageFormat, MouseButton, SharedString};

use crate::overlay::Overlay;
use crate::workspace::Workspace;

pub const REPO: &str = "https://github.com/erango/t1k";

/// Rendered by scripts/make_icon.py at 2x (1440x400).
static LOCKUP: LazyLock<Arc<Image>> =
    LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../../resources/brand/lockup.png").to_vec())));

fn lockup(width: f32) -> impl IntoElement {
    img(LOCKUP.clone()).w(px(width)).h(px(width * 200. / 720.))
}

fn card(ws: &Workspace) -> gpui::Div {
    let ui = ws.ui;
    div()
        .w(px(460.))
        .mt(px(110.))
        .p(px(28.))
        .rounded(px(14.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .max_w(px(560.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(14.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn button(ws: &Workspace, id: &'static str, label: &str, primary: bool) -> gpui::Stateful<gpui::Div> {
    let ui = ws.ui;
    div()
        .id(id)
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .text_size(px(13.))
        .map(|d| if primary { d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::MEDIUM) } else { d.bg(ui.bg_control).text_color(ui.text_primary) })
        .hover(|d| d.opacity(0.9))
        .child(label.to_string())
}

/// T1K menu ▸ About T1K.
pub fn about(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    card(ws)
        .child(lockup(300.))
        .child(div().text_size(px(13.)).text_color(ui.text_secondary).child("The terminal for running many coding agents at once."))
        .child(div().text_size(px(12.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(format!("Version {}", env!("CARGO_PKG_VERSION"))))
        .child(div().text_size(px(12.)).text_color(if crate::updater::waiting() { ui.accent } else { ui.text_tertiary }).child(crate::update_ui::status_words(&crate::updater::snapshot())))
        .child(
            div()
                .pt(px(6.))
                .flex()
                .gap(px(8.))
                .child(button(ws, "about-repo", "GitHub", false).on_click(|_, _, cx| cx.open_url(REPO)))
                .child(button(ws, "about-issue", "Report a bug", false).on_click(|_, _, cx| cx.open_url(&format!("{REPO}/issues"))))
                .child(button(ws, "about-close", "Close", true).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))),
        )
        .child(
            div()
                .flex()
                .gap(px(4.))
                .text_size(px(12.))
                .text_color(ui.text_tertiary)
                .child("Built with ♥ by")
                .child(
                    div()
                        .id("about-author")
                        .text_color(ui.link)
                        .cursor_pointer()
                        .hover(|d| d.underline())
                        .on_click(|_, _, cx| cx.open_url("https://github.com/erango"))
                        .child("@erango"),
                ),
        )
        .into_any_element()
}

/// First launch: what T1K is and the three things to do first.
pub fn welcome(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let step = |n: &'static str, title: &'static str, body: &'static str| {
        div()
            .w_full()
            .flex()
            .gap(px(12.))
            .child(
                div().size(px(22.)).flex_none().rounded_full().bg(ui.bg_control).flex().items_center().justify_center()
                    .text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_secondary).child(n),
            )
            .child(
                div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child(title))
                    .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_tertiary).child(body)),
            )
    };
    let connected = t1k_agent::agents::AgentKind::ALL.iter().any(|k| crate::agent_setup::installed(*k));
    card(ws)
        .w(px(500.))
        .child(lockup(300.))
        .child(div().text_size(px(14.)).text_color(ui.text_secondary).child("Run many coding agents. See which one needs you."))
        .child(
            div()
                .w_full()
                .min_w_0()
                .pt(px(6.))
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(step("1", "Connect your agents", "T1K reads live status from Claude Code, Codex, Antigravity and Grok through their hooks. Each config is backed up first."))
                .child(step("2", "Open a tab per task", "Tabs group by project. The sidebar shows what each agent is doing; amber means it needs you, and ⌘J jumps there."))
                .child(step("3", "Find anything", "⌘P finds a tab, ⌘⇧P runs any command, ⌘O shows every agent at once.")),
        )
        .child(
            div()
                .pt(px(8.))
                .flex()
                .gap(px(8.))
                .child(
                    button(ws, "welcome-connect", if connected { "Agents connected ✓" } else { "Connect agents" }, !connected).on_click(cx.listener(|this, _, window, cx| {
                        for line in crate::agent_setup::install_all() {
                            log::info!("{line}");
                        }
                        this.hooks_installed = crate::settings::claude_hooks_installed();
                        this.open_overlay(Overlay::Settings { section: crate::settings::Section::Agents }, window, cx);
                    })),
                )
                .child(button(ws, "welcome-start", "Start", connected).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))),
        )
        .children(crate::match_ui::welcome_section(ws, cx))
        .children(crate::import_ui::welcome_card(ws, cx))
        .child(div().text_size(px(11.)).text_color(ui.text_disabled).child(SharedString::from("You can connect agents later in Settings › Agents.")))
        .into_any_element()
}
