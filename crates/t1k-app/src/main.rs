//! T1K: agent-first terminal.

mod actions;
mod agent_setup;
mod app_icon;
mod brand;
mod cli;
mod config;
mod input;
mod links;
mod markdown;
mod menus;
mod outputs_panel;
mod overlay;
mod palette;
mod platform;
mod groups;
mod icons;
mod session;
mod settings;
mod shell;
mod split;
mod strip;
mod sidebar;
mod title;
mod tooltip;
mod update_ui;
mod updater;
mod viewer;
mod workspace;

use gpui::{prelude::*, px, size, App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions};
use t1k_terminal::view::{ClearScrollback, Copy, Find, FindNext, FindPrevious, NextPrompt, Paste, PreviousPrompt, ScrollPageDown, ScrollPageUp, ShowLinkHints};

use actions::*;
use workspace::Workspace;

/// The window frame from last time, if it still lands on a connected display (a monitor may
/// have been unplugged since).
fn restored_bounds(cx: &App) -> Option<WindowBounds> {
    use session::WindowState;
    let w = session::load().window?;
    if w.w < 200. || w.h < 150. {
        return None;
    }
    let b = Bounds::new(gpui::point(px(w.x), px(w.y)), size(px(w.w), px(w.h)));
    // At least a 120×60 corner of it must be on some display.
    let visible = cx.displays().iter().any(|d| {
        let s = d.bounds();
        let ix = f32::from(b.right().min(s.right())) - f32::from(b.left().max(s.left()));
        let iy = f32::from(b.bottom().min(s.bottom())) - f32::from(b.top().max(s.top()));
        ix >= 120. && iy >= 60.
    });
    if !visible {
        return None;
    }
    Some(match w.state {
        WindowState::Windowed => WindowBounds::Windowed(b),
        WindowState::Maximized => WindowBounds::Maximized(b),
        WindowState::Fullscreen => WindowBounds::Fullscreen(b),
    })
}

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
            KeyBinding::new("cmd-shift-j", JumpToNeedsBack, None),
            KeyBinding::new("cmd-p", ToggleSwitcher, None),
            KeyBinding::new("cmd-o", ToggleOverview, None),
            KeyBinding::new("cmd-shift-r", RenameTab, None),
            KeyBinding::new("cmd-shift-o", ToggleOutputs, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-shift-p", ToggleCommandPalette, None),
            KeyBinding::new("cmd-d", SplitRight, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-[", GoBack, None),
            KeyBinding::new("cmd-]", GoForward, None),
            KeyBinding::new("cmd-alt-left", FocusOtherPane, None),
            KeyBinding::new("cmd-alt-right", FocusOtherPane, None),
            KeyBinding::new("cmd-alt-up", FocusOtherPane, None),
            KeyBinding::new("cmd-alt-down", FocusOtherPane, None),
            KeyBinding::new("cmd-=", IncreaseFontSize, None),
            KeyBinding::new("cmd-+", IncreaseFontSize, None),
            KeyBinding::new("cmd--", DecreaseFontSize, None),
            KeyBinding::new("cmd-0", ResetFontSize, None),
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("cmd-k", ClearScrollback, Some("Terminal")),
            KeyBinding::new("cmd-shift-u", ShowLinkHints, Some("Terminal")),
            KeyBinding::new("cmd-f", Find, Some("Terminal")),
            KeyBinding::new("cmd-up", PreviousPrompt, Some("Terminal")),
            KeyBinding::new("cmd-down", NextPrompt, Some("Terminal")),
            KeyBinding::new("cmd-g", FindNext, Some("Terminal")),
            KeyBinding::new("cmd-shift-g", FindPrevious, Some("Terminal")),
            KeyBinding::new("shift-pageup", ScrollPageUp, Some("Terminal")),
            KeyBinding::new("shift-pagedown", ScrollPageDown, Some("Terminal")),
        ]);
        cx.on_action(|_: &Quit, cx| {
            // With an update waiting, ⌘Q asks once (it installs as T1K quits).
            if Workspace::should_confirm_quit() {
                if let Some(w) = cx.windows().first().and_then(|w| w.downcast::<Workspace>()) {
                    if w.update(cx, |ws, window, cx| ws.open_overlay(overlay::Overlay::QuitUpdate, window, cx)).is_ok() {
                        return;
                    }
                }
            }
            cx.quit()
        });
        // Remove this process's hook socket on the way out.
        cx.on_app_quit(|_| async {
            if let Some(d) = session::support_dir() {
                let _ = std::fs::remove_file(t1k_agent::server::socket_path(&d));
            }
        })
        .detach();
        menus::install(cx);
        platform::init_notifications();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let window_bounds = restored_bounds(cx).unwrap_or_else(|| WindowBounds::Windowed(Bounds::centered(None, size(px(1100.), px(720.)), cx)));
        cx.open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
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
