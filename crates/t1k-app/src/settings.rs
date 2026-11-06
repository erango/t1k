//! Settings (⌘,). Every control writes `config.toml` in place; the config watcher applies it,
//! so the file and the UI never disagree.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, SharedString, Window};
use t1k_themes::UiColors;

use crate::config;
use crate::overlay::Overlay;
use crate::workspace::Workspace;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Terminal,
    Sidebar,
    Notifications,
    Agents,
    Advanced,
}

impl Section {
    const ALL: [Section; 6] =
        [Section::Appearance, Section::Terminal, Section::Sidebar, Section::Notifications, Section::Agents, Section::Advanced];
    fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Terminal => "Terminal",
            Section::Sidebar => "Sidebar",
            Section::Notifications => "Notifications",
            Section::Agents => "Agents",
            Section::Advanced => "Advanced",
        }
    }
}

fn set<V: Into<toml_edit::Value>>(keys: &'static [&'static str], v: V) {
    config::set_value(keys, v.into());
}

fn row(ui: UiColors, label: &str, key: &str, control: impl IntoElement) -> impl IntoElement {
    div()
        .py(px(10.))
        .flex()
        .items_center()
        .gap(px(16.))
        .border_b_1()
        .border_color(ui.border)
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(13.)).text_color(ui.text_primary).child(label.to_string()))
                .child(div().text_size(px(10.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(key.to_string())),
        )
        .child(control)
}

fn pill(ui: UiColors, id: String, label: String, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id))
        .px(px(10.))
        .h(px(26.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(if on { ui.text_primary } else { ui.text_secondary })
        .bg(if on { ui.accent.opacity(0.28) } else { ui.bg_control })
        .hover(move |d| d.bg(ui.accent.opacity(0.18)))
        .child(label)
}

fn toggle(ui: UiColors, id: &'static str, on: bool, keys: &'static [&'static str], cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id(id)
        .w(px(36.))
        .h(px(20.))
        .rounded_full()
        .p(px(2.))
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { ui.accent } else { ui.bg_control })
        .on_click(cx.listener(move |_, _, _, _| set(keys, !on)))
        .child(div().size(px(16.)).rounded_full().bg(gpui::white()))
}

fn stepper(
    ui: UiColors,
    id: &'static str,
    value: f64,
    step: f64,
    (min, max): (f64, f64),
    keys: &'static [&'static str],
    fmt: fn(f64) -> String,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let btn = |suffix: &str, label: &str, next: f64| {
        let next = (next.clamp(min, max) * 100.).round() / 100.;
        div()
            .id(SharedString::from(format!("{id}-{suffix}")))
            .size(px(26.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .bg(ui.bg_control)
            .text_color(ui.text_primary)
            .hover(move |d| d.bg(ui.accent.opacity(0.18)))
            .on_click(cx.listener(move |_, _, _, _| {
                // Keep integers as integers in the file.
                if next.fract() == 0. && step.fract() == 0. {
                    set(keys, next as i64);
                } else {
                    set(keys, next);
                }
            }))
            .child(label.to_string())
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(btn("dec", "−", value - step))
        .child(div().w(px(56.)).text_center().text_size(px(13.)).text_color(ui.text_primary).child(fmt(value)))
        .child(btn("inc", "+", value + step))
}

fn swatch(t: &t1k_themes::Theme) -> impl IntoElement {
    let c = |v: u32| -> Hsla { gpui::rgb(v).into() };
    div()
        .w(px(64.))
        .h(px(28.))
        .rounded(px(5.))
        .bg(c(t.bg))
        .border_1()
        .border_color(c(t.fg).opacity(0.2))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(3.))
        .children([1usize, 2, 3, 4, 5].map(|i| div().size(px(7.)).rounded_full().bg(c(t.ansi[i]))))
}

fn theme_grid(ws: &Workspace, dark: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let current = if dark { &ws.config.theme.dark } else { &ws.config.theme.light };
    let mut grid = div().flex().flex_wrap().gap(px(8.)).py(px(8.));
    for t in t1k_themes::all().into_iter().filter(|t| t.is_dark() == dark) {
        let on = t.name.eq_ignore_ascii_case(current);
        let name = t.name.to_string();
        grid = grid.child(
            div()
                .id(SharedString::from(format!("theme-{name}")))
                .w(px(150.))
                .p(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(if on { ui.accent } else { ui.border })
                .when(on, |d| d.bg(ui.accent.opacity(0.12)))
                .hover(move |d| d.bg(ui.bg_hover))
                .flex()
                .items_center()
                .gap(px(8.))
                .on_click(cx.listener(move |_, _, _, _| {
                    let slot: &'static [&'static str] = if dark { &["theme", "dark"] } else { &["theme", "light"] };
                    set(slot, name.as_str());
                }))
                .child(swatch(t))
                .child(div().flex_1().text_size(px(11.)).text_color(ui.text_primary).line_clamp(2).child(t.name.to_string())),
        );
    }
    grid
}

fn section_body(ws: &Workspace, section: Section, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let c = &ws.config;
    let heading = |s: &str| div().pt(px(18.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(s.to_uppercase());
    let mut body = div().flex().flex_col();
    match section {
        Section::Appearance => {
            let mode = c.appearance.clone();
            let mut seg = div().flex().gap(px(4.));
            for (v, label) in [("system", "System"), ("dark", "Dark"), ("light", "Light")] {
                seg = seg.child(pill(ui, format!("mode-{v}"), label.into(), mode == v).on_click(cx.listener(move |_, _, _, _| {
                    set(&["appearance"], v);
                })));
            }
            body = body
                .child(row(ui, "Appearance", "appearance", seg))
                .child(heading("Dark theme"))
                .child(theme_grid(ws, true, cx))
                .child(heading("Light theme"))
                .child(theme_grid(ws, false, cx))
                .child(
                    div()
                        .pt(px(8.))
                        .text_size(px(12.))
                        .text_color(ui.text_tertiary)
                        .child("Import more: drop a Ghostty, iTerm2, Alacritty, Windows Terminal or base16 file on the window, or run `t1k theme import <file>`."),
                );
        }
        Section::Terminal => {
            let fonts = monospace_fonts(window);
            let mut chips = div().flex().flex_wrap().gap(px(6.)).py(px(8.));
            for f in fonts {
                let on = f == c.terminal.font.family;
                let name = f.clone();
                chips = chips.child(pill(ui, format!("font-{f}"), f.clone(), on).font_family(SharedString::from(f.clone())).on_click(
                    cx.listener(move |_, _, _, _| set(&["terminal", "font", "family"], name.as_str())),
                ));
            }
            body = body
                .child(heading("Font"))
                .child(chips)
                .child(row(ui, "Size", "terminal.font.size", stepper(ui, "size", c.terminal.font.size as f64, 1., (8., 32.), &["terminal", "font", "size"], |v| format!("{v:.0} px"), cx)))
                .child(row(ui, "Line height", "terminal.font.line_height", stepper(ui, "lh", c.terminal.font.line_height as f64, 0.05, (1., 2.), &["terminal", "font", "line_height"], |v| format!("{v:.2}"), cx)))
                .child(row(ui, "Padding", "terminal.padding", stepper(ui, "pad", c.terminal.padding as f64, 2., (0., 40.), &["terminal", "padding"], |v| format!("{v:.0} px"), cx)))
                .child(row(ui, "Option key sends Meta (Esc+)", "terminal.option_as_meta", toggle(ui, "meta", c.terminal.option_as_meta, &["terminal", "option_as_meta"], cx)))
                .child(row(ui, "Copy on select", "terminal.copy_on_select", toggle(ui, "cos", c.terminal.copy_on_select, &["terminal", "copy_on_select"], cx)))
                .child(
                    div()
                        .mt(px(14.))
                        .p(px(12.))
                        .rounded(px(8.))
                        .bg(ui.bg_terminal)
                        .font_family(SharedString::from(c.terminal.font.family.clone()))
                        .text_size(px(c.terminal.font.size))
                        .line_height(gpui::relative(c.terminal.font.line_height))
                        .text_color(ui.text_primary)
                        .child("fn main() { println!(\"0O 1lI {} => != ->\"); }")
                        .child(div().text_color(ui.link).child("https://github.com/erango/t1k/pull/12")),
                );
        }
        Section::Sidebar => {
            let density = c.interface.density.clone();
            let mut seg = div().flex().gap(px(4.));
            for (v, label) in [("compact", "Compact"), ("comfortable", "Comfortable"), ("spacious", "Spacious")] {
                seg = seg.child(pill(ui, format!("density-{v}"), label.into(), density == v).on_click(cx.listener(move |_, _, _, _| {
                    set(&["interface", "density"], v);
                })));
            }
            body = body
                .child(row(ui, "Density", "interface.density", seg))
                .child(row(ui, "Text size", "interface.text_size", stepper(ui, "uits", c.interface.text_size as f64, 1., (10., 18.), &["interface", "text_size"], |v| format!("{v:.0} px"), cx)))
                .child(row(ui, "Width", "sidebar.width", stepper(ui, "sw", c.sidebar.width as f64, 20., (220., 460.), &["sidebar", "width"], |v| format!("{v:.0} px"), cx)))
                .child(row(ui, "Show details after resting", "sidebar.hover_intent_ms", stepper(ui, "hover", c.sidebar.hover_intent_ms as f64, 50., (0., 1000.), &["sidebar", "hover_intent_ms"], |v| format!("{v:.0} ms"), cx)))
                .child(div().pt(px(10.)).text_size(px(12.)).text_color(ui.text_tertiary).child("Hold ⌥ to see every tab's status line. ⌘\\ hides the sidebar."));
        }
        Section::Notifications => {
            let n = &c.notify;
            body = body
                .child(row(ui, "When a tab needs input", "notify.needs_input", toggle(ui, "n-needs", n.needs_input, &["notify", "needs_input"], cx)))
                .child(row(ui, "When a session errors", "notify.error", toggle(ui, "n-err", n.error, &["notify", "error"], cx)))
                .child(row(ui, "When a reminder fires", "notify.reminder", toggle(ui, "n-rem", n.reminder, &["notify", "reminder"], cx)))
                .child(row(ui, "When a session finishes", "notify.done", toggle(ui, "n-done", n.done, &["notify", "done"], cx)))
                .child(row(ui, "Needs-you count on the Dock icon", "notify.dock_badge", toggle(ui, "n-badge", n.dock_badge, &["notify", "dock_badge"], cx)));
        }
        Section::Agents => {
            let installed = claude_hooks_installed();
            let status = if installed { "Installed in ~/.claude/settings.json" } else { "Not installed" };
            let action = pill(ui, "hooks".into(), if installed { "Reinstall".into() } else { "Install hooks".into() }, !installed)
                .on_click(cx.listener(|_, _, _, cx| {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).arg("setup-claude").status();
                    }
                    cx.notify();
                }));
            body = body
                .child(row(ui, "Claude Code hooks", status, action))
                .child(
                    div()
                        .pt(px(10.))
                        .text_size(px(12.))
                        .text_color(ui.text_tertiary)
                        .child("Hooks let Claude report status to T1K. They back up settings.json first and do nothing in other terminals."),
                );
        }
        Section::Advanced => {
            let open_cfg = pill(ui, "open-cfg".into(), "Open config.toml".into(), false).on_click(cx.listener(|_, _, _, cx| {
                if let Some(p) = config::path() {
                    cx.open_with_system(&p);
                }
            }));
            let themes = pill(ui, "open-themes".into(), "Show themes folder".into(), false).on_click(cx.listener(|_, _, _, cx| {
                if let Some(p) = config::themes_dir() {
                    let _ = std::fs::create_dir_all(&p);
                    cx.reveal_path(&p);
                }
            }));
            body = body
                .child(row(ui, "Settings file", "~/.config/t1k/config.toml", open_cfg))
                .child(row(ui, "Imported themes", "~/.config/t1k/themes/", themes));
        }
    }
    body.into_any_element()
}

/// Installed font families that look monospaced (by name), with the configured one first.
fn monospace_fonts(window: &Window) -> Vec<String> {
    const HINTS: [&str; 12] =
        ["mono", "menlo", "monaco", "courier", "code", "consolas", "iosevka", "hack", "fira", "meslo", "jetbrains", "sf mono"];
    let mut names: Vec<String> = window
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|n| !n.starts_with('.'))
        .filter(|n| {
            let l = n.to_lowercase();
            HINTS.iter().any(|h| l.contains(h)) && !l.contains("symbols") && !l.contains("nerd font propo")
        })
        .collect();
    names.sort();
    names.dedup();
    names.truncate(40);
    names
}

fn claude_hooks_installed() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    std::fs::read_to_string(std::path::Path::new(&home).join(".claude/settings.json")).is_ok_and(|s| s.contains("T1K_BIN"))
}

pub fn render(ws: &Workspace, section: Section, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let mut nav = div().w(px(190.)).flex_none().p(px(10.)).flex().flex_col().gap(px(2.)).bg(ui.bg_sidebar);
    nav = nav.child(div().px(px(10.)).pt(px(6.)).pb(px(12.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Settings"));
    for s in Section::ALL {
        let on = s == section;
        nav = nav.child(
            div()
                .id(SharedString::from(format!("sec-{}", s.label())))
                .h(px(30.))
                .px(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .text_size(px(13.))
                .text_color(if on { ui.text_primary } else { ui.text_secondary })
                .when(on, |d| d.bg(ui.bg_selected))
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.overlay = Overlay::Settings { section: s };
                    cx.notify();
                }))
                .child(s.label()),
        );
    }
    let body = section_body(ws, section, window, cx);
    div()
        .w(px(860.))
        .h(px(600.))
        .mt(px(60.))
        .rounded(px(12.))
        .overflow_hidden()
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(nav)
        .child(div().id("settings-body").flex_1().overflow_y_scroll().px(px(24.)).py(px(16.)).child(body))
        .into_any_element()
}
