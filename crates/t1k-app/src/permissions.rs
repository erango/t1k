//! What T1K is allowed to do on this Mac (canvas v3-bc): notifications, which T1K needs to
//! tell you an agent needs you, and Full Disk Access, which is optional. Read when T1K starts
//! and whenever its window gets focus (you may be coming back from System Settings).

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Notif {
    /// Not read yet, or a dev build without a bundle (osascript banners, not checked).
    Unknown,
    /// Never answered (the prompt was dismissed): "Ask again" can show it.
    NotSetUp,
    Off,
    /// Allowed, but the alert style is None, so nothing appears.
    NoBanners,
    On,
}

impl Notif {
    /// Needs a person: shows amber, the sidebar card and the dot.
    pub fn needs_fixing(self) -> bool {
        matches!(self, Notif::NotSetUp | Notif::Off | Notif::NoBanners)
    }
}

static NOTIF: AtomicU8 = AtomicU8::new(0);
static CHANGED: AtomicBool = AtomicBool::new(false);

fn encode(n: Notif) -> u8 {
    match n {
        Notif::Unknown => 0,
        Notif::NotSetUp => 1,
        Notif::Off => 2,
        Notif::NoBanners => 3,
        Notif::On => 4,
    }
}

pub fn notifications() -> Notif {
    match NOTIF.load(Ordering::SeqCst) {
        1 => Notif::NotSetUp,
        2 => Notif::Off,
        3 => Notif::NoBanners,
        4 => Notif::On,
        _ => Notif::Unknown,
    }
}

fn store(n: Notif) {
    if NOTIF.swap(encode(n), Ordering::SeqCst) != encode(n) {
        CHANGED.store(true, Ordering::SeqCst);
    }
}

/// Whether a read since the last call changed something (the workspace redraws).
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::SeqCst)
}

/// From UNNotificationSettings: authorizationStatus (0 not determined, 1 denied, 2 authorized,
/// 3 provisional, 4 ephemeral) and alertStyle (0 none, 1 banner, 2 alert).
pub fn classify(status: i64, alert_style: i64) -> Notif {
    match status {
        0 => Notif::NotSetUp,
        1 => Notif::Off,
        _ if alert_style == 0 => Notif::NoBanners,
        _ => Notif::On,
    }
}

/// Re-read both permissions (the notification one arrives a moment later, off the main thread).
pub fn refresh() {
    full_disk_now();
    #[cfg(target_os = "macos")]
    if crate::platform::BUNDLED.load(Ordering::SeqCst) {
        use block::ConcreteBlock;
        use cocoa::base::id;
        use objc::{class, msg_send, sel, sel_impl};
        unsafe {
            let handler = ConcreteBlock::new(|settings: id| {
                let status: i64 = msg_send![settings, authorizationStatus];
                let style: i64 = msg_send![settings, alertStyle];
                let n = classify(status, style);
                if n == Notif::On {
                    crate::platform::UN_READY.store(true, Ordering::SeqCst);
                }
                store(n);
            })
            .copy();
            let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
            let _: () = msg_send![center, getNotificationSettingsWithCompletionHandler: &*handler];
        }
    }
}

/// Show macOS's prompt again (only works while it was never answered).
pub fn ask_again() {
    #[cfg(target_os = "macos")]
    unsafe {
        use block::ConcreteBlock;
        use cocoa::base::{id, BOOL, NO};
        use objc::{class, msg_send, sel, sel_impl};
        let handler = ConcreteBlock::new(|granted: BOOL, _err: id| {
            if granted != NO {
                crate::platform::UN_READY.store(true, Ordering::SeqCst);
            }
            refresh();
        })
        .copy();
        let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let _: () = msg_send![center, requestAuthorizationWithOptions: (1usize | 2 | 4) completionHandler: &*handler];
    }
}

pub const NOTIFICATIONS_URL: &str = "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=com.erango.t1k";
pub const FULL_DISK_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FullDisk {
    NotGranted,
    /// Granted after T1K started: it takes effect after a restart.
    GrantedSinceLaunch,
    On,
}

static FULL_DISK: AtomicU8 = AtomicU8::new(0);
static AT_LAUNCH: OnceLock<bool> = OnceLock::new();

/// Reading the privacy database needs Full Disk Access; nothing else about it is read.
fn probe() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    std::fs::File::open(std::path::Path::new(&home).join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok()
}

fn full_disk_now() {
    let now = probe();
    let launch = *AT_LAUNCH.get_or_init(|| now);
    let v = match (launch, now) {
        (_, false) => 0,
        (false, true) => 1,
        (true, true) => 2,
    };
    if FULL_DISK.swap(v, Ordering::SeqCst) != v {
        CHANGED.store(true, Ordering::SeqCst);
    }
}

pub fn full_disk() -> FullDisk {
    match FULL_DISK.load(Ordering::SeqCst) {
        1 => FullDisk::GrantedSinceLaunch,
        2 => FullDisk::On,
        _ => FullDisk::NotGranted,
    }
}

/// Quit (the usual way, so the session is saved) and open T1K again once this process is
/// gone. If quitting is cancelled, the waiter gives up after a minute.
pub fn restart(window: &mut gpui::Window, cx: &mut gpui::App) {
    let Ok(exe) = std::env::current_exe() else { return };
    // .../T1K.app/Contents/MacOS/t1k → .../T1K.app
    let Some(app) = exe.ancestors().nth(3).filter(|p| p.extension().is_some_and(|e| e == "app")).map(|p| p.to_path_buf()) else { return };
    let pid = std::process::id();
    let script = "for i in $(seq 600); do kill -0 \"$1\" 2>/dev/null || { open \"$2\"; exit 0; }; sleep 0.1; done";
    let _ = std::process::Command::new("/bin/sh").args(["-c", script, "t1k-restart", &pid.to_string()]).arg(&app).spawn();
    window.dispatch_action(Box::new(crate::actions::Quit), cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_settings_map_to_states() {
        assert_eq!(classify(0, 0), Notif::NotSetUp);
        assert_eq!(classify(1, 1), Notif::Off);
        assert_eq!(classify(2, 0), Notif::NoBanners);
        assert_eq!(classify(2, 1), Notif::On);
        assert_eq!(classify(3, 2), Notif::On);
        assert!(Notif::NoBanners.needs_fixing() && !Notif::Unknown.needs_fixing() && !Notif::On.needs_fixing());
    }
}
