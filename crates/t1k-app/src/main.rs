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
mod keymap;
mod telemetry;
mod tooltip;
mod update_ui;
mod updater;
mod viewer;
mod workspace;

use gpui::{prelude::*, px, size, App, Application, Bounds, WindowBounds, WindowOptions};

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
    // Crash reports: only builds with a DSN, and only while privacy.crash_reports is on.
    let _sentry = telemetry::init(config::load().privacy.crash_reports);
    // Debug aid: T1K_DEBUG_CRASH=error sends a test error report, =panic crashes on purpose.
    match std::env::var("T1K_DEBUG_CRASH").as_deref() {
        Ok("error") => log::error!("test error report from {}", std::env::var("HOME").unwrap_or_default()),
        Ok("panic") => panic!("test panic in `private text`"),
        _ => {}
    }
    Application::new().run(|cx: &mut App| {
        keymap::install(cx, &config::load().keys);
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
        cx.on_action(|_: &ReportBug, cx| cx.open_url(&format!("{}/issues", brand::REPO)));
        cx.on_action(|_: &OpenRepo, cx| cx.open_url(brand::REPO));
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
