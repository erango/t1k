//! Settings (⌘,). Every control writes `config.toml` in place; the config watcher applies it,
//! so the file and the UI never disagree.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, SharedString, Window};
use t1k_themes::UiColors;

use crate::config;
use crate::overlay::Overlay;
use crate::workspace::Workspace;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Appearance,
    Interface,
    Tabs,
    Terminal,
    Agents,
    Keybindings,
    Advanced,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Section::Appearance,
        Section::Interface,
        Section::Tabs,
        Section::Terminal,
        Section::Agents,
        Section::Keybindings,
        Section::Advanced,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Interface => "Interface",
            Section::Tabs => "Tabs & Links",
            Section::Terminal => "Terminal",
            Section::Agents => "Agents",
            Section::Keybindings => "Keybindings",
            Section::Advanced => "Advanced",
        }
    }
}

/// One setting: where it lives, what it's called (and what else people might search for),
/// its config key, and its control.
struct Entry {
    section: Section,
    group: &'static str,
    label: &'static str,
    desc: &'static str,
    key: &'static str,
    synonyms: &'static str,
    control: AnyElement,
}

impl Entry {
    fn matches(&self, q: &str) -> bool {
        let q = q.to_lowercase();
        q.split_whitespace().all(|w| {
            [self.label, self.desc, self.key, self.synonyms, self.section.label()].iter().any(|f| f.to_lowercase().contains(w))
        })
    }
}

fn set<V: Into<toml_edit::Value>>(keys: &'static [&'static str], v: V) {
    config::set_value(keys, v.into());
}

/// A setting row (design: label, description, config key; the key lights up on hover).
fn row(ui: UiColors, e: Entry, n: usize) -> impl IntoElement {
    let gid = SharedString::from(format!("setting-{n}"));
    div()
        .id(gid.clone())
        .group(gid.clone())
        .min_h(px(48.))
        .px(px(14.))
        .py(px(8.))
        .flex()
        .items_center()
        .gap(px(16.))
        .border_b_1()
        .border_color(ui.border)
        .hover(move |d| d.bg(ui.bg_hover))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(13.)).text_color(ui.text_primary).child(e.label))
                .when(!e.desc.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(ui.text_tertiary).child(e.desc)))
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family("JetBrains Mono")
                        .text_color(ui.text_disabled)
                        .group_hover(gid, move |s| s.text_color(ui.accent))
                        .child(e.key),
                ),
        )
        .child(div().flex_none().child(e.control))
}

/// A card of rows under a small group label.
fn card(ui: UiColors, label: &str, rows: Vec<AnyElement>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .when(!label.is_empty(), |d| {
            d.child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(label.to_uppercase()))
        })
        .child(div().rounded(px(9.)).bg(ui.bg_terminal).border_1().border_color(ui.border).overflow_hidden().flex().flex_col().children(rows))
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

#[allow(clippy::too_many_arguments)]
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
    // Wrapped lines must not stretch to fill the scroll area's height.
    let mut grid = div().w_full().flex().flex_wrap().content_start().items_start().gap(px(8.)).py(px(8.));
    for t in t1k_themes::all().into_iter().filter(|t| t.is_dark() == dark) {
        let on = t.name.eq_ignore_ascii_case(current);
        let name = t.name.to_string();
        grid = grid.child(
            div()
                .id(SharedString::from(format!("theme-{name}")))
                .w(px(162.))
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

/// Three icon tiles; the chosen one has an accent ring.
fn icon_picker(ui: UiColors, current: &str, cx: &mut Context<Workspace>) -> AnyElement {
    use std::sync::{Arc, LazyLock};
    static ICONS: LazyLock<Vec<(&'static str, &'static str, Arc<gpui::Image>)>> = LazyLock::new(|| {
        let img = |b: &'static [u8]| Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, b.to_vec()));
        vec![
            ("face", "Face", img(include_bytes!("../../../resources/icon-256.png"))),
            ("prompt", "Prompt", img(crate::platform::dock_icon_png("prompt", false).unwrap())),
            ("wordmark", "Wordmark", img(crate::platform::dock_icon_png("wordmark", false).unwrap())),
        ]
    });
    let mut row = div().flex().gap(px(10.));
    for (v, label, image) in ICONS.iter() {
        let on = current == *v;
        let v: &'static str = v;
        row = row.child(
            div()
                .id(SharedString::from(format!("icon-{v}")))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .on_click(cx.listener(move |_, _, _, _| set(&["interface", "app_icon"], v)))
                .child(
                    div()
                        .p(px(3.))
                        .rounded(px(12.))
                        .border_2()
                        .border_color(if on { ui.accent } else { gpui::transparent_black() })
                        .child(gpui::img(image.clone()).size(px(44.))),
                )
                .child(div().text_size(px(11.)).text_color(if on { ui.text_primary } else { ui.text_tertiary }).child(*label)),
        );
    }
    row.into_any_element()
}

fn seg(
    ui: UiColors,
    id: &'static str,
    current: &str,
    options: &[(&'static str, &'static str)],
    keys: &'static [&'static str],
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let mut seg = div().flex().gap(px(4.));
    for &(v, label) in options {
        seg = seg.child(pill(ui, format!("{id}-{v}"), label.into(), current == v).on_click(cx.listener(move |_, _, _, _| set(keys, v))));
    }
    seg.into_any_element()
}

/// Every setting, in page order.
fn entries(ws: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Vec<Entry> {
    let ui = ws.ui;
    let c = &ws.config;
    let n = &c.notify;
    let e = |section, group, label, desc, key, synonyms, control: AnyElement| Entry { section, group, label, desc, key, synonyms, control };
    // One row per agent: installed, or an Install button (backs up the agent's config first).
    let agent_rows: Vec<Entry> = t1k_agent::agents::AgentKind::ALL
        .into_iter()
        .map(|kind| {
            let found = crate::agent_setup::detected(kind);
            let done = crate::agent_setup::installed(kind);
            let control = if done {
                div().text_size(px(12.)).text_color(ui.status_done).child("✓ Connected").into_any_element()
            } else if found {
                pill(ui, format!("hooks-{}", kind.id()), "Install hooks".into(), true)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Err(e) = crate::agent_setup::install(kind) {
                            log::error!("install {} hooks: {e}", kind.label());
                        }
                        this.hooks_installed = claude_hooks_installed();
                        cx.notify();
                    }))
                    .into_any_element()
            } else {
                div().text_size(px(12.)).text_color(ui.text_disabled).child("Not found").into_any_element()
            };
            let desc: &'static str = match (kind, done) {
                (_, true) => "Live status, titles and needs-you",
                (t1k_agent::agents::AgentKind::Gemini, false) => "Retired by Google (June 2026); Antigravity CLI replaces it",
                _ => "Hooks report status to T1K; they do nothing in other terminals",
            };
            let key: &'static str = match kind {
                t1k_agent::agents::AgentKind::Claude => "~/.claude/settings.json",
                t1k_agent::agents::AgentKind::Codex => "~/.codex/hooks.json",
                t1k_agent::agents::AgentKind::Gemini => "~/.gemini/settings.json",
                t1k_agent::agents::AgentKind::Antigravity => "~/.gemini/config/hooks.json",
                t1k_agent::agents::AgentKind::Grok => "~/.grok/hooks/t1k.json",
            };
            Entry { section: Section::Agents, group: "Coding agents", label: kind.label(), desc, key, synonyms: "hooks setup install connect claude codex gemini antigravity agy grok", control }
        })
        .collect();
    let open_cfg = pill(ui, "open-cfg".into(), "Open config.toml".into(), false)
        .on_click(cx.listener(|_, _, _, cx| {
            if let Some(p) = config::path() {
                cx.open_with_system(&p);
            }
        }))
        .into_any_element();
    let themes_dir = pill(ui, "open-themes".into(), "Show themes folder".into(), false)
        .on_click(cx.listener(|_, _, _, cx| {
            if let Some(p) = config::themes_dir() {
                let _ = std::fs::create_dir_all(&p);
                cx.reveal_path(&p);
            }
        }))
        .into_any_element();
    let mut fonts = div().flex().flex_wrap().justify_end().max_w(px(420.)).gap(px(6.));
    for f in monospace_fonts(window) {
        let on = f == c.terminal.font.family;
        let name = f.clone();
        fonts = fonts.child(
            pill(ui, format!("font-{f}"), f.clone(), on)
                .font_family(SharedString::from(f.clone()))
                .on_click(cx.listener(move |_, _, _, _| set(&["terminal", "font", "family"], name.as_str()))),
        );
    }
    let text = |v: &str| div().max_w(px(260.)).truncate().text_size(px(12.)).font_family("JetBrains Mono").text_color(ui.text_secondary).child(if v.is_empty() { "auto".to_string() } else { v.to_string() }).into_any_element();
    use Section::*;
    let mut out = vec![
        e(Appearance, "Theme", "Mode", "System follows macOS; ⌘⇧L flips dark and light", "appearance", "dark light system theme",
            seg(ui, "mode", &c.appearance, &[("system", "System"), ("dark", "Dark"), ("light", "Light")], &["appearance"], cx)),
        e(Appearance, "App", "App icon", "In the Dock; it turns red while a tab needs you", "interface.app_icon", "icon dock logo face prompt wordmark",
            icon_picker(ui, &c.interface.app_icon, cx)),
        e(Interface, "Sidebar", "Density", "Row height: 28 / 32 / 36", "interface.density", "compact spacious rows",
            seg(ui, "density", &c.interface.density, &[("compact", "Compact"), ("comfortable", "Comfortable"), ("spacious", "Spacious")], &["interface", "density"], cx)),
        e(Interface, "Sidebar", "UI text size", "Sidebar and panels; the terminal has its own size", "interface.text_size", "font size scale",
            stepper(ui, "uits", c.interface.text_size as f64, 1., (10., 18.), &["interface", "text_size"], |v| format!("{v:.0} px"), cx).into_any_element()),
        e(Interface, "Sidebar", "Width", "", "sidebar.width", "",
            stepper(ui, "sw", c.sidebar.width as f64, 20., (220., 460.), &["sidebar", "width"], |v| format!("{v:.0} px"), cx).into_any_element()),
        e(Interface, "Sidebar", "Program icons", "Claude, shell, vim… on each tab, with status on the corner", "sidebar.app_icons", "badge",
            toggle(ui, "appicons", c.sidebar.app_icons, &["sidebar", "app_icons"], cx).into_any_element()),
        e(Interface, "Hover", "Show details on hover", "A card beside the row; the list never moves", "sidebar.hover_card", "card tooltip",
            toggle(ui, "hovercard", c.sidebar.hover_card, &["sidebar", "hover_card"], cx).into_any_element()),
        e(Interface, "Hover", "Hover delay", "How long the pointer rests before the card opens", "sidebar.hover_intent_ms", "delay",
            stepper(ui, "hover", c.sidebar.hover_intent_ms as f64, 50., (0., 1000.), &["sidebar", "hover_intent_ms"], |v| format!("{v:.0} ms"), cx).into_any_element()),
        e(Tabs, "Titles", "Title as", "A manual rename always wins", "sidebar.title", "name",
            seg(ui, "title", &c.sidebar.title, &[("auto", "Auto"), ("command", "Command"), ("folder", "Folder"), ("branch", "Branch")], &["sidebar", "title"], cx)),
        e(Tabs, "Titles", "Second line", "Always on for every row, so nothing moves", "sidebar.second_line", "subtitle detail",
            seg(ui, "second", &c.sidebar.second_line, &[("none", "None"), ("status", "Status"), ("branch", "Branch"), ("folder", "Folder")], &["sidebar", "second_line"], cx)),
        e(Tabs, "Links", "Editor for file links", "⌘-click a path; empty picks Zed, Cursor or VS Code", "links.editor", "open file code",
            text(&c.links.editor)),
        e(Tabs, "Links", "Ticket links", "Where keys like ENG-42 open; empty learns it from sessions", "links.tickets", "jira linear tracker",
            text(&c.links.tickets)),
        e(Terminal, "Font", "Family", "", "terminal.font.family", "font typeface monospace", fonts.into_any_element()),
        e(Terminal, "Font", "Size", "⌘+ / ⌘− / ⌘0", "terminal.font.size", "font size zoom",
            stepper(ui, "size", c.terminal.font.size as f64, 1., (8., 32.), &["terminal", "font", "size"], |v| format!("{v:.0} px"), cx).into_any_element()),
        e(Terminal, "Font", "Line height", "", "terminal.font.line_height", "spacing",
            stepper(ui, "lh", c.terminal.font.line_height as f64, 0.05, (1., 2.), &["terminal", "font", "line_height"], |v| format!("{v:.2}"), cx).into_any_element()),
        e(Terminal, "Behavior", "Padding", "Above and below; the sides get about 1.4×", "terminal.padding", "margin",
            stepper(ui, "pad", c.terminal.padding as f64, 2., (0., 40.), &["terminal", "padding"], |v| format!("{v:.0} px"), cx).into_any_element()),
        e(Terminal, "Behavior", "Option sends Meta", "Option+key sends Esc+key (for shells and Emacs)", "terminal.option_as_meta", "alt",
            toggle(ui, "meta", c.terminal.option_as_meta, &["terminal", "option_as_meta"], cx).into_any_element()),
        e(Terminal, "Behavior", "Copy on select", "", "terminal.copy_on_select", "clipboard",
            toggle(ui, "cos", c.terminal.copy_on_select, &["terminal", "copy_on_select"], cx).into_any_element()),
        e(Terminal, "Behavior", "Keep scrollback across restarts", "Each tab's output comes back after quitting or updating", "terminal.restore_scrollback", "history restore save warp",
            toggle(ui, "rsb", c.terminal.restore_scrollback, &["terminal", "restore_scrollback"], cx).into_any_element()),
        e(Terminal, "Behavior", "Resume Claude sessions", "Sessions still running at quit continue in their tabs", "terminal.resume_agents", "restart update claude resume",
            toggle(ui, "rag", c.terminal.resume_agents, &["terminal", "resume_agents"], cx).into_any_element()),
        e(Terminal, "Behavior", "Prompt position", "Where a new tab's first prompt appears", "terminal.prompt_gravity", "gravity bottom top start",
            seg(ui, "gravity", &c.terminal.prompt_gravity, &[("bottom", "Bottom"), ("top", "Top")], &["terminal", "prompt_gravity"], cx)),
        e(Terminal, "Behavior", "Shell integration", "zsh prompt marks for ⌘↑ / ⌘↓ (new tabs)", "terminal.shell_integration", "zsh prompt",
            toggle(ui, "shellint", c.terminal.shell_integration, &["terminal", "shell_integration"], cx).into_any_element()),
        e(Agents, "Notifications", "When a tab needs you", "", "notify.needs_input", "banner alert",
            toggle(ui, "n-needs", n.needs_input, &["notify", "needs_input"], cx).into_any_element()),
        e(Agents, "Notifications", "When a session errors", "", "notify.error", "banner alert",
            toggle(ui, "n-err", n.error, &["notify", "error"], cx).into_any_element()),
        e(Agents, "Notifications", "When a reminder fires", "", "notify.reminder", "banner alert",
            toggle(ui, "n-rem", n.reminder, &["notify", "reminder"], cx).into_any_element()),
        e(Agents, "Notifications", "When a session finishes", "", "notify.done", "banner alert",
            toggle(ui, "n-done", n.done, &["notify", "done"], cx).into_any_element()),
        e(Agents, "Notifications", "Dock badge", "Count of tabs that need you", "notify.dock_badge", "badge count icon",
            toggle(ui, "n-badge", n.dock_badge, &["notify", "dock_badge"], cx).into_any_element()),
        e(Interface, "Sidebar", "When collapsed", "What ⌘\\ and dragging the edge to the window's side do. The strip still shows what needs you.", "sidebar.collapsed", "rail strip hide collapse sliver narrow",
            seg(ui, "collapsed", &c.sidebar.collapsed, &[("rail", "Strip of icons"), ("hidden", "Hide")], &["sidebar", "collapsed"], cx)),
        e(Agents, "Helper", "Cheap-model helper", "Titles, recaps and ⌘↵ search for agents that don't write their own, through a CLI you already use", "helper.backend", "llm haiku gemini flash summary recap title search ai model cheap",
            {
                let mut opts: Vec<(&'static str, &'static str)> = vec![("off", "Off")];
                for b in t1k_agent::helper::Backend::ALL {
                    if b.find().is_some() || c.helper.backend == b.id() {
                        opts.push((b.id(), b.label()));
                    }
                }
                seg(ui, "helper", &c.helper.backend, &opts, &["helper", "backend"], cx)
            }),
        e(Agents, "Helper", "Name untitled sessions", "From the first prompt; Claude Code names its own", "helper.titles", "llm title name",
            toggle(ui, "h-titles", c.helper.titles, &["helper", "titles"], cx).into_any_element()),
        e(Agents, "Helper", "Recap each turn", "Sends the end of that tab's screen to the CLI's provider", "helper.recaps", "llm recap summary privacy",
            toggle(ui, "h-recaps", c.helper.recaps, &["helper", "recaps"], cx).into_any_element()),
        e(Agents, "Helper", "Search by meaning", "⌘↵ in Find tab (⌘P) ranks tabs by what they're about", "helper.search", "llm search switcher semantic",
            toggle(ui, "h-search", c.helper.search, &["helper", "search"], cx).into_any_element()),
        e(Agents, "Helper", "Last 7 days", "Time, tokens and cost of every helper call, measured", "helper.stats", "llm cost tokens usage benchmark",
            div().flex().items_center().gap(px(10.))
                .child(div().max_w(px(360.)).text_size(px(12.)).text_color(ui.text_tertiary).child(crate::helper::week().line()))
                .child(pill(ui, "h-bench".into(), "Run benchmark".into(), false).on_click(cx.listener(|this, _, window, cx| {
                    this.close_overlay(window, cx);
                    this.open_tab(None, Some("Helper benchmark".into()), window, cx);
                    let ix = this.tabs.len() - 1;
                    let bin = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "t1k".into());
                    this.tabs[ix].view.read(cx).terminal.write(format!("'{bin}' helper bench\r").into_bytes());
                })))
                .into_any_element()),
        e(Advanced, "Updates", "Check for updates automatically", "Once a day, in the background. Nothing interrupts you.", "updates.check", "update version sparkle auto",
            toggle(ui, "u-check", c.updates.check, &["updates", "check"], cx).into_any_element()),
        e(Advanced, "Updates", "Download updates automatically", "Have the update ready; you still choose when to restart", "updates.download", "update version sparkle",
            toggle(ui, "u-download", c.updates.download, &["updates", "download"], cx).into_any_element()),
        e(Advanced, "Updates", "Channel", "Beta gets new features first, with rougher edges", "updates.channel", "update version beta stable sparkle",
            seg(ui, "u-channel", &c.updates.channel, &[("stable", "Stable"), ("beta", "Beta")], &["updates", "channel"], cx)),
        e(Advanced, "Updates", concat!("Version ", env!("CARGO_PKG_VERSION")), "Check for Updates… is also in the T1K menu", "updates", "update version sparkle check",
            div().flex().items_center().gap(px(10.))
                .child(div().text_size(px(12.)).text_color(ui.text_tertiary).child(crate::update_ui::status_words(&crate::updater::snapshot())))
                .child(pill(ui, "u-now".into(), "Check now".into(), false).on_click(cx.listener(|this, _, window, cx| this.check_for_updates(window, cx))))
                .into_any_element()),
        e(Advanced, "Privacy", "Send crash reports", if crate::telemetry::available() { "Crashes and errors, never terminal contents; applies at next launch" } else { "This build has no crash reporting" }, "privacy.crash_reports", "sentry telemetry crash error report privacy",
            toggle(ui, "crash", c.privacy.crash_reports, &["privacy", "crash_reports"], cx).into_any_element()),
        e(Advanced, "Files", "Settings file", "~/.config/t1k/config.toml; edits apply live", "config.toml", "config", open_cfg),
        e(Advanced, "Files", "Imported themes", "~/.config/t1k/themes/", "themes_dir", "import", themes_dir),
    ];
    // Agents first on their page.
    let at = out.iter().position(|e| e.section == Agents).unwrap_or(out.len());
    for (i, r) in agent_rows.into_iter().enumerate() {
        out.insert(at + i, r);
    }
    out
}

/// Cards for a list of entries, grouped by their `group`, in order.
fn cards(ui: UiColors, entries: Vec<Entry>, start: usize) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let mut current: Option<&'static str> = None;
    let mut rows: Vec<AnyElement> = Vec::new();
    for (i, e) in entries.into_iter().enumerate() {
        if current != Some(e.group) {
            if let Some(g) = current {
                out.push(card(ui, g, std::mem::take(&mut rows)).into_any_element());
            }
            current = Some(e.group);
        }
        rows.push(row(ui, e, start + i).into_any_element());
    }
    if let Some(g) = current {
        out.push(card(ui, g, rows).into_any_element());
    }
    out
}

fn page(ws: &Workspace, section: Section, query: &str, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let all = entries(ws, window, cx);
    let title = |t: String, sub: Option<String>| {
        div()
            .flex()
            .items_baseline()
            .gap(px(10.))
            .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(t))
            .children(sub.map(|s| div().text_size(px(12.)).text_color(ui.text_tertiary).child(s)))
    };
    let mut body = div().w_full().min_w_0().flex().flex_col().gap(px(16.));
    if !query.trim().is_empty() {
        // Search: matching settings from every page, as real controls, under section labels.
        let hits: Vec<Entry> = all.into_iter().filter(|e| e.matches(query)).collect();
        let sections: Vec<Section> = Section::ALL.into_iter().filter(|s| hits.iter().any(|e| e.section == *s)).collect();
        body = body.child(title(
            format!("{} result{}", hits.len(), if hits.len() == 1 { "" } else { "s" }),
            Some(format!("in {} section{} · esc clears", sections.len(), if sections.len() == 1 { "" } else { "s" })),
        ));
        let mut start = 0;
        let mut hits = hits;
        for sec in sections {
            let (mine, rest): (Vec<Entry>, Vec<Entry>) = hits.into_iter().partition(|e| e.section == sec);
            hits = rest;
            let n = mine.len();
            let rows: Vec<AnyElement> = mine.into_iter().enumerate().map(|(i, e)| row(ui, e, start + i).into_any_element()).collect();
            body = body.child(card(ui, sec.label(), rows));
            start += n;
        }
        return body.into_any_element();
    }
    body = body.child(title(section.label().to_string(), None));
    let mine: Vec<Entry> = all.into_iter().filter(|e| e.section == section).collect();
    body = body.children(cards(ui, mine, 0));
    match section {
        Section::Appearance => {
            let heading = |s: &str| div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(s.to_uppercase());
            body = body
                .child(div().flex().flex_col().gap(px(6.)).child(heading("Dark theme")).child(theme_grid(ws, true, cx)))
                .child(div().flex().flex_col().gap(px(6.)).child(heading("Light theme")).child(theme_grid(ws, false, cx)))
                .child(
                    div()
                        .id("browse-themes")
                        .h(px(28.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .w(px(220.))
                        .rounded(px(6.))
                        .bg(ui.bg_control)
                        .border_1()
                        .border_color(ui.border)
                        .text_size(px(12.5))
                        .text_color(ui.text_primary)
                        .cursor_pointer()
                        .hover(move |d| d.bg(ui.bg_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_theme_picker(window, cx);
                            this.settings_return = Some(Section::Appearance);
                        }))
                        .child("Browse themes with preview…"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(ui.text_tertiary)
                        .child("Import more: drop a Ghostty, iTerm2, Alacritty, Windows Terminal or base16 file on the window, or run `t1k theme import <file>`."),
                );
        }
        Section::Terminal => {
            let c = &ws.config;
            body = body.child(
                div()
                    .p(px(12.))
                    .rounded(px(8.))
                    .bg(ui.bg_terminal)
                    .border_1()
                    .border_color(ui.border)
                    .font_family(SharedString::from(c.terminal.font.family.clone()))
                    .text_size(px(c.terminal.font.size))
                    .line_height(gpui::relative(c.terminal.font.line_height))
                    .text_color(ui.text_primary)
                    .child("fn main() { println!(\"0O 1lI {} => != ->\"); }")
                    .child("● Bash(cargo test)  ⎿ 212/388  ✳ ⏵⏵")
                    .child(div().text_color(ui.link).child("https://github.com/erango/t1k/pull/12")),
            );
        }
        Section::Keybindings => {
            body = body.child(
                div().text_size(px(12.)).text_color(ui.text_tertiary).child(
                    "Click a shortcut, then press the new keys. ⌫ removes it, esc cancels. A shortcut taken from another action leaves that one.",
                ),
            );
            for group in ["Tabs", "Agents", "Window", "Terminal"] {
                let rows: Vec<AnyElement> =
                    crate::keymap::ALL.iter().filter(|b| b.group == group).map(|b| key_row(ws, b, cx)).collect();
                body = body.child(card(ui, group, rows));
            }
        }
        _ => {}
    }
    body.into_any_element()
}

/// One action in Settings › Keybindings: its label, its shortcuts (click to record), reset.
fn key_row(ws: &Workspace, b: &'static crate::keymap::Bind, cx: &mut Context<Workspace>) -> AnyElement {
    use crate::keymap;
    let ui = ws.ui;
    let keys = keymap::keys_for(b, &ws.config.keys);
    let changed = ws.config.keys.contains_key(b.id);
    let recording = ws.key_recording == Some(b.id);
    let note = ws.key_note.as_ref().filter(|(id, _)| *id == b.id).map(|(_, n)| n.clone());
    let chip = |text: String, on: bool| {
        div()
            .h(px(22.))
            .px(px(7.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .bg(ui.bg_sidebar)
            .border_1()
            .border_color(if on { ui.accent } else { ui.border })
            .text_size(px(12.))
            .text_color(if on { ui.accent } else { ui.text_secondary })
            .child(text)
    };
    let id = b.id;
    let shortcuts = div()
        .id(SharedString::from(format!("keys-{id}")))
        .flex()
        .gap(px(4.))
        .rounded(px(6.))
        .p(px(2.))
        .cursor_pointer()
        .hover(move |d| d.bg(ui.bg_hover))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.key_recording = if this.key_recording == Some(id) { None } else { Some(id) };
            this.key_note = None;
            cx.notify();
        }))
        .when(recording, |d| d.child(chip("Press keys…".into(), true)))
        .when(!recording && keys.is_empty(), |d| d.child(div().h(px(22.)).px(px(7.)).flex().items_center().text_size(px(12.)).text_color(ui.text_disabled).child("None")))
        .when(!recording, |d| d.children(keys.iter().map(|k| chip(keymap::display(k), false))));
    div()
        .min_h(px(36.))
        .px(px(14.))
        .py(px(4.))
        .flex()
        .items_center()
        .gap(px(10.))
        .border_b_1()
        .border_color(ui.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(13.)).text_color(ui.text_primary).child(b.label))
                .children(note.map(|n| div().text_size(px(11.5)).text_color(ui.text_tertiary).child(n))),
        )
        .when(changed && !recording, |d| {
            d.child(
                div()
                    .id(SharedString::from(format!("reset-{id}")))
                    .text_size(px(12.))
                    .text_color(ui.text_tertiary)
                    .cursor_pointer()
                    .hover(move |d| d.text_color(ui.text_primary))
                    .on_click(cx.listener(move |this, _, _, cx| this.reset_key(id, cx)))
                    .child("Reset"),
            )
        })
        .child(shortcuts)
        .into_any_element()
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

pub fn claude_hooks_installed() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    std::fs::read_to_string(std::path::Path::new(&home).join(".claude/settings.json")).is_ok_and(|s| s.contains("T1K_BIN"))
}

pub fn render(ws: &Workspace, section: Section, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let query = ws.settings_query.clone();
    let searching = !query.trim().is_empty();
    // Match counts per section while searching.
    let counts: Vec<(Section, usize)> = if searching {
        let all = entries(ws, window, cx);
        Section::ALL.iter().map(|s| (*s, all.iter().filter(|e| e.section == *s && e.matches(&query)).count())).collect()
    } else {
        Vec::new()
    };
    let mut nav = div().w(px(220.)).flex_none().rounded_l(px(11.)).px(px(10.)).py(px(12.)).flex().flex_col().gap(px(2.)).bg(ui.bg_sidebar).border_r_1().border_color(ui.border);
    nav = nav.child(
        div()
            .h(px(30.))
            .mb(px(10.))
            .px(px(10.))
            .rounded(px(7.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(if searching { ui.accent } else { ui.border })
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .child(crate::icons::search(ui.text_tertiary))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_color(if searching { ui.text_primary } else { ui.text_tertiary })
                    .child(if searching { format!("{query}▏") } else { "Search settings".to_string() }),
            )
            .child(div().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_disabled).child("type")),
    );
    for s in Section::ALL {
        let on = s == section && !searching;
        let count = counts.iter().find(|(x, _)| *x == s).map(|(_, n)| *n);
        let dim = searching && count == Some(0);
        nav = nav.child(
            div()
                .id(SharedString::from(format!("sec-{}", s.label())))
                .h(px(30.))
                .px(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .text_size(px(13.))
                .text_color(if on { ui.text_primary } else if dim { ui.text_disabled } else { ui.text_secondary })
                .when(on, |d| d.bg(ui.bg_selected).font_weight(FontWeight::MEDIUM))
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings_query.clear();
                    this.overlay = Overlay::Settings { section: s };
                    cx.notify();
                }))
                .child(div().flex_1().child(s.label()))
                .children(count.filter(|n| *n > 0).map(|n| div().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_quiet).child(n.to_string()))),
        );
    }
    nav = nav.child(div().flex_1()).child(
        div().px(px(10.)).text_size(px(11.)).line_height(px(16.)).text_color(ui.text_disabled).child("Edits write to ~/.config/t1k/config.toml and apply live."),
    );
    let body = page(ws, section, &query, window, cx);
    // Design: 1100×740, shrunk to fit smaller windows.
    let vp = window.viewport_size();
    let w = (f32::from(vp.width) - 64.).clamp(560., 1100.);
    let h = (f32::from(vp.height) - 96.).clamp(400., 740.);
    div()
        .w(px(w))
        .h(px(h))
        .mt(px(48.))
        .rounded(px(12.))
        .overflow_hidden()
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .relative()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(nav)
        .child(div().id("settings-body").flex_1().min_w_0().overflow_y_scroll().px(px(28.)).py(px(22.)).child(body))
        .child(
            div()
                .id("settings-close")
                .absolute()
                .top(px(14.))
                .right(px(14.))
                .size(px(26.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |d| d.bg(ui.bg_hover))
                .tooltip(crate::tooltip::text(ui, "Close (esc)"))
                .on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))
                .child(crate::icons::close(ui.text_secondary)),
        )
        .into_any_element()
}
