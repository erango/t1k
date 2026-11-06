//! T1K: agent-first terminal.

mod actions;
mod cli;
mod groups;
mod session;
mod sidebar;
mod workspace;

use gpui::{prelude::*, px, size, App, Application, Bounds, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions};
use t1k_terminal::view::{ClearScrollback, Copy, Paste, ScrollPageDown, ScrollPageUp};
use t1k_themes::BUILTIN;

use actions::*;
use workspace::Workspace;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if cli::run(&args) {
        return;
    }
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
            KeyBinding::new("cmd-\\", ToggleSidebar, None),
            KeyBinding::new("cmd-j", JumpToNeeds, None),
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
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    MenuItem::separator(),
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
