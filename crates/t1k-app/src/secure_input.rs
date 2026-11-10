//! Secure Keyboard Entry (canvas v3-an): while on and a T1K window is key, macOS keeps other
//! apps from reading keystrokes (EnableSecureEventInput), as Terminal and iTerm2 offer.

use std::sync::atomic::{AtomicBool, Ordering};

/// The setting, read by the menu (which has no workspace to ask).
static ON: AtomicBool = AtomicBool::new(false);
/// Whether we hold the system's secure input now. Enable/Disable are counted by macOS, so
/// every enable is paired with exactly one disable.
static ENGAGED: AtomicBool = AtomicBool::new(false);
/// On only because the focused terminal is at a password prompt (the setting itself is off).
static AUTO: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn EnableSecureEventInput() -> i32;
    fn DisableSecureEventInput() -> i32;
}

/// The lock shown while it's on (sidebar top strip, or the title strip with the sidebar
/// hidden). Clicking it turns it off.
pub fn indicator(ws: &crate::workspace::Workspace, id: &'static str, cx: &mut gpui::Context<crate::workspace::Workspace>) -> Option<gpui::AnyElement> {
    use gpui::{div, prelude::*, px, MouseButton};
    let auto = AUTO.load(Ordering::Relaxed);
    if !ws.config.terminal.secure_input && !auto {
        return None;
    }
    let tip = if ws.config.terminal.secure_input {
        "Secure Keyboard Entry is on. Other apps can't read what you type in T1K. Click to turn it off."
    } else {
        "Secure Keyboard Entry is on for this password prompt. It turns off when the prompt is answered."
    };
    let ui = ws.ui;
    Some(
        div()
            .id(id)
            .size(px(22.))
            .flex_none()
            .rounded(px(5.))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |d| d.bg(ui.bg_selected))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .tooltip(crate::tooltip::text(ui, tip))
            .on_click(cx.listener(move |this, _, _, _| {
                if !auto {
                    this.toggle_secure_input()
                }
            }))
            .child(crate::icons::lock(ui.text_secondary, 14.))
            .into_any_element(),
    )
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Apply the setting for a window that is or isn't key.
pub fn apply(setting: bool, window_key: bool) {
    apply_with(setting, false, window_key);
}

/// As `apply`, also holding it while `password` (the focused terminal asks for one).
/// Returns whether the lock's look changed.
pub fn apply_with(setting: bool, password: bool, window_key: bool) -> bool {
    ON.store(setting, Ordering::Relaxed);
    let auto = !setting && password && window_key;
    engage((setting || password) && window_key);
    AUTO.swap(auto, Ordering::Relaxed) != auto
}

/// Let go at quit.
pub fn release() {
    engage(false);
}

fn engage(want: bool) {
    if ENGAGED.swap(want, Ordering::SeqCst) == want {
        return;
    }
    #[cfg(target_os = "macos")]
    let status = unsafe { if want { EnableSecureEventInput() } else { DisableSecureEventInput() } };
    #[cfg(not(target_os = "macos"))]
    let status = 0;
    log::info!("secure keyboard entry {} ({status})", if want { "on" } else { "off" });
}
