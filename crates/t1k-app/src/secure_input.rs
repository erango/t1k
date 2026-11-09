//! Secure Keyboard Entry (canvas v3-an): while on and a T1K window is key, macOS keeps other
//! apps from reading keystrokes (EnableSecureEventInput), as Terminal and iTerm2 offer.

use std::sync::atomic::{AtomicBool, Ordering};

/// The setting, read by the menu (which has no workspace to ask).
static ON: AtomicBool = AtomicBool::new(false);
/// Whether we hold the system's secure input now. Enable/Disable are counted by macOS, so
/// every enable is paired with exactly one disable.
static ENGAGED: AtomicBool = AtomicBool::new(false);

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
    if !ws.config.terminal.secure_input {
        return None;
    }
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
            .tooltip(crate::tooltip::text(ui, "Secure Keyboard Entry is on. Other apps can't read what you type in T1K. Click to turn it off."))
            .on_click(cx.listener(|this, _, _, _| this.toggle_secure_input()))
            .child(crate::icons::lock(ui.text_secondary, 14.))
            .into_any_element(),
    )
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Apply the setting for a window that is or isn't key.
pub fn apply(setting: bool, window_key: bool) {
    ON.store(setting, Ordering::Relaxed);
    engage(setting && window_key);
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
