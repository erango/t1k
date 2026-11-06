//! T1K: agent-first terminal. M0/M1: window, terminal tabs, keybindings.

use std::collections::HashMap;

use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, Context, Entity,
    FocusHandle, Focusable, KeyBinding, Menu, MenuItem, SharedString, Subscription, Window,
    WindowBounds, WindowOptions,
};
use t1k_terminal::view::{ClearScrollback, Copy, Paste, ScrollPageDown, ScrollPageUp};
use t1k_terminal::{SpawnOptions, TerminalEvent, TerminalView};

actions!(t1k, [Quit, NewTab, CloseTab, NextTab, PrevTab, SelectTab1, SelectTab2, SelectTab3,
    SelectTab4, SelectTab5, SelectTab6, SelectTab7, SelectTab8, SelectTab9]);

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
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { tabs: Vec::new(), active: 0, focus_handle: cx.focus_handle() };
        this.open_tab(None, window, cx);
        this
    }

    fn open_tab(&mut self, cwd: Option<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let id = uuid::Uuid::new_v4().to_string();
        let opts = SpawnOptions {
            working_directory: cwd.or_else(dirs_home),
            env: HashMap::new(),
            tab_id: id.clone(),
        };
        let view = cx.new(|cx| TerminalView::new(opts, window, cx).expect("failed to spawn shell in pty"));
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
        div()
            .flex()
            .flex_row()
            .h(px(34.))
            .pl(px(78.)) // room for the traffic lights
            .items_center()
            .gap_1()
            .bg(rgb(0x0f1114))
            .border_b_1()
            .border_color(rgb(0x23272e))
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
                    .text_color(if active { rgb(0xe6e9ef) } else { rgb(0x8a919c) })
                    .when(active, |d| d.bg(rgb(0x1d2127)))
                    .hover(|d| d.bg(rgb(0x191c21)))
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
            .bg(rgb(0x14161a))
            .font_family("Menlo")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab));
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
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("cmd-k", ClearScrollback, Some("Terminal")),
            KeyBinding::new("shift-pageup", ScrollPageUp, Some("Terminal")),
            KeyBinding::new("shift-pagedown", ScrollPageDown, Some("Terminal")),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(vec![Menu {
            name: "T1K".into(),
            items: vec![MenuItem::action("New Tab", NewTab), MenuItem::action("Quit T1K", Quit)],
        }]);
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
