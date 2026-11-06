//! T1K: agent-first terminal. M0/M1: window, terminal tabs, keybindings.

use std::collections::HashMap;

use gpui::{
    actions, div, prelude::*, px, size, Action, App, Application, Bounds, Context, Entity,
    FocusHandle, Focusable, KeyBinding, Menu, MenuItem, SharedString, Subscription, Window,
    WindowAppearance, WindowBounds, WindowOptions,
};
use t1k_themes::{Appearance, AppearanceMode, Theme, UiColors, BUILTIN};
use t1k_terminal::view::{ClearScrollback, Copy, Paste, ScrollPageDown, ScrollPageUp};
use t1k_terminal::{SpawnOptions, TerminalEvent, TerminalView};

actions!(t1k, [Quit, NewTab, CloseTab, NextTab, PrevTab, SelectTab1, SelectTab2, SelectTab3,
    SelectTab4, SelectTab5, SelectTab6, SelectTab7, SelectTab8, SelectTab9,
    ToggleAppearance, FollowSystemAppearance]);

/// Pick a theme by name. It fills the dark or light slot, matching the theme's appearance.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = t1k, no_json)]
struct SelectTheme {
    name: SharedString,
}

struct Tab {
    id: String,
    view: Entity<TerminalView>,
    title: SharedString,
    _sub: Subscription,
}

struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    focus_handle: FocusHandle,
    mode: AppearanceMode,
    dark: &'static Theme,
    light: &'static Theme,
    /// Theme currently applied (resolved from mode + system appearance).
    theme: &'static Theme,
    ui: UiColors,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut dark = t1k_themes::default_for(Appearance::Dark);
        let mut light = t1k_themes::default_for(Appearance::Light);
        let mut mode = match std::env::var("T1K_APPEARANCE").as_deref() {
            Ok("dark") => AppearanceMode::Dark,
            Ok("light") => AppearanceMode::Light,
            _ => AppearanceMode::System,
        };
        // Until config.toml lands (M3), T1K_THEME picks a starting theme.
        if let Some(t) = std::env::var("T1K_THEME").ok().and_then(|n| t1k_themes::find(&n)) {
            if t.is_dark() { dark = t } else { light = t }
            if mode == AppearanceMode::System {
                mode = if t.is_dark() { AppearanceMode::Dark } else { AppearanceMode::Light };
            }
        }
        let theme = dark;
        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            focus_handle: cx.focus_handle(),
            mode,
            dark,
            light,
            theme,
            ui: theme.ui(),
        };
        cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)).detach();
        this.apply_theme(window, cx);
        this.open_tab(None, window, cx);
        this
    }

    fn system_is_dark(window: &Window) -> bool {
        matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
    }

    fn resolve_theme(&self, window: &Window) -> &'static Theme {
        match self.mode {
            AppearanceMode::Dark => self.dark,
            AppearanceMode::Light => self.light,
            AppearanceMode::System => if Self::system_is_dark(window) { self.dark } else { self.light },
        }
    }

    /// Resolve the active theme and push it to the chrome and every terminal.
    fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme = self.resolve_theme(window);
        self.ui = self.theme.ui();
        let palette = self.theme.palette();
        for tab in &self.tabs {
            tab.view.update(cx, |v, cx| {
                v.palette = palette.clone();
                cx.notify();
            });
        }
        cx.notify();
    }

    fn toggle_appearance(&mut self, _: &ToggleAppearance, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = if self.theme.is_dark() { AppearanceMode::Light } else { AppearanceMode::Dark };
        self.apply_theme(window, cx);
    }

    fn follow_system(&mut self, _: &FollowSystemAppearance, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = AppearanceMode::System;
        self.apply_theme(window, cx);
    }

    fn select_theme(&mut self, a: &SelectTheme, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = t1k_themes::find(&a.name) else { return };
        if t.is_dark() { self.dark = t } else { self.light = t }
        // Show the pick now. In System mode, only switch if the system shows the other slot.
        let showing_slot = self.resolve_theme(window).is_dark() == t.is_dark();
        if !(self.mode == AppearanceMode::System && showing_slot) {
            self.mode = if t.is_dark() { AppearanceMode::Dark } else { AppearanceMode::Light };
        }
        self.apply_theme(window, cx);
    }

    fn open_tab(&mut self, cwd: Option<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let id = uuid::Uuid::new_v4().to_string();
        let opts = SpawnOptions {
            working_directory: cwd.or_else(dirs_home),
            env: HashMap::new(),
            tab_id: id.clone(),
        };
        let palette = self.theme.palette();
        let view = cx.new(|cx| {
            let mut v = TerminalView::new(opts, window, cx).expect("failed to spawn shell in pty");
            v.palette = palette;
            v
        });
        let sub = cx.subscribe_in(&view, window, |this: &mut Self, view, ev: &TerminalEvent, window, cx| {
            let Some(ix) = this.tabs.iter().position(|t| &t.view == view) else { return };
            match ev {
                TerminalEvent::TitleChanged(t) => {
                    this.tabs[ix].title = if t.is_empty() { "shell".into() } else { t.clone().into() };
                    cx.notify();
                }
                TerminalEvent::Exited => this.close_tab_at(ix, window, cx),
                TerminalEvent::Bell => {}
            }
        });
        self.tabs.push(Tab { id, view, title: "shell".into(), _sub: sub });
        self.activate(self.tabs.len() - 1, window, cx);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active = ix;
        let handle = self.tabs[ix].view.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        if self.tabs.is_empty() {
            window.remove_window();
            return;
        }
        let next = if self.active > ix || self.active >= self.tabs.len() {
            self.active.saturating_sub(1)
        } else {
            self.active
        };
        self.activate(next.min(self.tabs.len() - 1), window, cx);
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        // TODO(M1): inherit cwd of the active tab via OSC 7 / proc cwd.
        self.open_tab(None, window, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        self.activate((self.active + 1) % n, window, cx);
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        self.activate((self.active + n - 1) % n, window, cx);
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = self.ui;
        div()
            .flex()
            .flex_row()
            .h(px(34.))
            .pl(px(78.)) // room for the traffic lights
            .items_center()
            .gap_1()
            .bg(ui.bg_chrome)
            .border_b_1()
            .border_color(ui.border)
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let active = ix == self.active;
                div()
                    .id(SharedString::from(tab.id.clone()))
                    .px_3()
                    .py_1()
                    .max_w(px(220.))
                    .overflow_hidden()
                    .rounded_md()
                    .text_size(px(12.))
                    .text_color(if active { ui.text_primary } else { ui.text_tertiary })
                    .when(active, |d| d.bg(ui.bg_selected))
                    .hover(move |d| d.bg(ui.bg_hover))
                    .child(format!("{}  {}", ix + 1, tab.title))
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
            }))
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(Into::into)
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.tabs.get(self.active).map(|t| t.title.to_string()).unwrap_or_default();
        window.set_window_title(&format!("{title} — T1K"));
        let active_view = self.tabs.get(self.active).map(|t| t.view.clone());

        macro_rules! select {
            ($d:expr, $($act:ident => $ix:expr),*) => {
                $d$(.on_action(cx.listener(|this, _: &$act, window, cx| {
                    let ix = if $ix == 8 { this.tabs.len() - 1 } else { $ix };
                    this.activate(ix, window, cx)
                })))*
            };
        }
        let root = div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(self.ui.bg_terminal)
            .font_family("Menlo")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::toggle_appearance))
            .on_action(cx.listener(Self::follow_system))
            .on_action(cx.listener(Self::select_theme));
        let root = select!(root, SelectTab1 => 0, SelectTab2 => 1, SelectTab3 => 2, SelectTab4 => 3,
            SelectTab5 => 4, SelectTab6 => 5, SelectTab7 => 6, SelectTab8 => 7, SelectTab9 => 8);
        root.child(self.render_tab_bar(cx))
            .child(div().flex_1().min_h_0().children(active_view))
    }
}

fn main() {
    env_logger::init();
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-t", NewTab, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-shift-]", NextTab, None),
            KeyBinding::new("cmd-shift-[", PrevTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PrevTab, None),
            KeyBinding::new("cmd-1", SelectTab1, None),
            KeyBinding::new("cmd-2", SelectTab2, None),
            KeyBinding::new("cmd-3", SelectTab3, None),
            KeyBinding::new("cmd-4", SelectTab4, None),
            KeyBinding::new("cmd-5", SelectTab5, None),
            KeyBinding::new("cmd-6", SelectTab6, None),
            KeyBinding::new("cmd-7", SelectTab7, None),
            KeyBinding::new("cmd-8", SelectTab8, None),
            KeyBinding::new("cmd-9", SelectTab9, None),
            KeyBinding::new("cmd-shift-l", ToggleAppearance, None),
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("cmd-k", ClearScrollback, Some("Terminal")),
            KeyBinding::new("shift-pageup", ScrollPageUp, Some("Terminal")),
            KeyBinding::new("shift-pagedown", ScrollPageDown, Some("Terminal")),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let theme_items = |dark: bool| {
            BUILTIN
                .iter()
                .filter(|t| t.is_dark() == dark)
                .map(|t| MenuItem::action(t.name, SelectTheme { name: t.name.into() }))
                .collect::<Vec<_>>()
        };
        cx.set_menus(vec![
            Menu {
                name: "T1K".into(),
                items: vec![MenuItem::action("New Tab", NewTab), MenuItem::action("Quit T1K", Quit)],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Toggle Dark / Light", ToggleAppearance),
                    MenuItem::action("Follow System Appearance", FollowSystemAppearance),
                    MenuItem::separator(),
                    MenuItem::submenu(Menu { name: "Dark Theme".into(), items: theme_items(true) }),
                    MenuItem::submenu(Menu { name: "Light Theme".into(), items: theme_items(false) }),
                ],
            },
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("T1K".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(gpui::point(px(12.), px(11.))),
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Workspace::new(window, cx)),
        )
        .expect("open window");
        cx.activate(true);
    });
}
