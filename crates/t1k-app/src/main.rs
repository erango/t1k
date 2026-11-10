//! T1K: agent-first terminal.

mod actions;
mod demo;
mod import;
mod import_ui;
mod match_term;
mod drop;
mod restore;
mod session_color;
mod match_ui;
mod fonts;
mod fonts_ui;
mod agent_setup;
mod app_icon;
mod brand;
mod bug_report;
mod cli;
mod broadcast;
mod config;
mod context_ui;
mod input;
mod links;
mod markdown;
mod menus;
mod outputs_panel;
mod overlay;
mod palette;
mod platform;
mod groups;
mod helper;
mod helper_cli;
mod close;
mod github;
mod group_icons;
mod custom_icon;
mod icon_crop;
mod quit;
mod icons;
mod outline;
mod rail;
mod secure_input;
mod scroll_bar;
mod reminders;
mod usage;
mod session;
mod settings;
mod setup;
mod shell;
mod split;
mod strip;
mod sidebar;
mod title;
mod keymap;
mod telemetry;
mod theme_picker;
mod tooltip;
mod update_ui;
mod updater;
mod viewer;
mod windows;
mod workspace;

use gpui::{App, Application};

use actions::*;
use workspace::Workspace;

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
        keymap::install(cx, &if demo::on() { Default::default() } else { config::load().keys });
        // Deferred: ⌘Q from the keyboard dispatches while GPUI has the window checked out for
        // the key event, so updating it here fails and the app used to quit without asking.
        cx.on_action(|_: &Quit, cx| cx.defer(quit_or_ask));
        fn quit_or_ask(cx: &mut App) {
            let front = cx.active_window().or_else(|| cx.windows().first().copied()).and_then(|w| w.downcast::<Workspace>());
            // With an update waiting, ⌘Q asks once (it installs as T1K quits).
            if Workspace::should_confirm_quit() {
                if let Some(w) = front {
                    if w.update(cx, |ws, window, cx| ws.open_overlay(overlay::Overlay::QuitUpdate, window, cx)).is_ok() {
                        return;
                    }
                }
            }
            // Programs still running: each window asks first, then closes them gently.
            windows::quit_cancelled();
            windows::quit_next(None, cx);
        }
        cx.on_action(|_: &OpenRepo, cx| cx.open_url(brand::REPO));
        // Remove this process's hook socket on the way out.
        cx.on_app_quit(|_| async {
            if let Some(d) = session::support_dir() {
                let _ = std::fs::remove_file(t1k_agent::server::socket_path(&d));
            }
            secure_input::release();
        })
        .detach();
        // Before the menu is built, so its Secure Keyboard Entry item reads right.
        secure_input::apply(config::load().terminal.secure_input, false);
        menus::install(cx);
        platform::init_notifications();
        platform::guard_terminate();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                crate::platform::quit(cx);
            }
        })
        .detach();

        // Screenshots while you keep working: open behind other windows, without taking focus.
        let background = std::env::var_os("T1K_DEBUG_BACKGROUND").is_some();
        // One window per saved window (canvas v3-aq); the first holds the rest.
        let mut first = if demo::on() { session::Session::default() } else { session::load() };
        let rest = std::mem::take(&mut first.windows);
        for saved in std::iter::once(first).chain(rest) {
            windows::open(saved, None, !background, cx);
        }
        if cx.windows().is_empty() {
            panic!("open window");
        }
        if !background {
            cx.activate(true);
        }
    });
}
