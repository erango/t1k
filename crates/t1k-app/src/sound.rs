//! Notification sounds (canvas v3-bc): one theme for every banner, as a pair in one timbre,
//! "alert" for a tab that needs you or an error and "soft" for done, reminders and program
//! banners. The WAVs are synthesized by scripts/gen-sounds.py; `notify.sound` picks the theme.
//! Adapted from Coo's sound themes.

pub struct Theme {
    pub id: &'static str,
    pub label: &'static str,
    pub blurb: &'static str,
    alert: &'static [u8],
    soft: &'static [u8],
}

macro_rules! theme {
    ($id:literal, $label:literal, $blurb:literal) => {
        Theme {
            id: $id,
            label: $label,
            blurb: $blurb,
            alert: include_bytes!(concat!("../../../resources/sounds/", $id, "-alert.wav")),
            soft: include_bytes!(concat!("../../../resources/sounds/", $id, "-soft.wav")),
        }
    };
}

/// The first is the default.
pub const THEMES: &[Theme] = &[
    theme!("mercury", "Mercury", "a drop of liquid metal"),
    theme!("bell", "Bell", "the terminal bell, softened"),
    theme!("tick", "Tick", "two wooden key clicks"),
    theme!("pulse", "Pulse", "two quiet sine blips"),
    theme!("chord", "Chord", "a soft open fifth"),
];
pub const DEFAULT: &str = "mercury";
/// macOS's own notification sound.
pub const SYSTEM: &str = "system";
pub const SILENT: &str = "silent";

static THEME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The theme from `notify.sound` (set when the config loads).
pub fn set_theme(id: &str) {
    if let Ok(mut t) = THEME.lock() {
        *t = id.to_string();
    }
}

pub fn theme() -> String {
    THEME.lock().map(|t| if t.is_empty() { DEFAULT.to_string() } else { t.clone() }).unwrap_or_else(|_| DEFAULT.to_string())
}

/// Every choice for Settings: (id, label, blurb).
pub fn choices() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut v: Vec<_> = THEMES.iter().map(|t| (t.id, t.label, t.blurb)).collect();
    v.push((SYSTEM, "macOS default", "the system sound"));
    v.push((SILENT, "Silent", "banners only"));
    v
}

pub fn label(id: &str) -> &'static str {
    choices().into_iter().find(|(i, _, _)| *i == id).map_or("Mercury", |(_, l, _)| l)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Needs you, errors.
    Alert,
    /// Done, reminders, program banners.
    Soft,
}

fn wav(theme: &str, kind: Kind) -> Option<&'static [u8]> {
    THEMES.iter().find(|t| t.id == theme).map(|t| match kind {
        Kind::Alert => t.alert,
        Kind::Soft => t.soft,
    })
}

/// What a banner should sound like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    Silent,
    System,
    /// A file name in ~/Library/Sounds.
    Named(String),
}

/// Same-kind banners closer together than this sound once (a burst of agents finishing).
pub const ALERT_GAP: std::time::Duration = std::time::Duration::from_secs(4);

pub fn due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= ALERT_GAP)
}

/// Whether a banner of `kind` gets its sound now: at most once per `ALERT_GAP` per kind, and
/// never while the screen is locked or asleep. Focus and Do Not Disturb are macOS's call (the
/// sound rides on the notification).
fn take(kind: Kind) -> bool {
    if imp::screens_off() {
        return false;
    }
    static LAST: std::sync::Mutex<[Option<std::time::Instant>; 2]> = std::sync::Mutex::new([None, None]);
    let now = std::time::Instant::now();
    let Ok(mut last) = LAST.lock() else { return false };
    let slot = kind as usize;
    if !due(last[slot], now) {
        return false;
    }
    last[slot] = Some(now);
    true
}

/// The sound for a banner of `kind` posted now, with theme `theme`.
pub fn for_banner(theme: &str, kind: Kind) -> Banner {
    if theme == SILENT || !take(kind) {
        return Banner::Silent;
    }
    if theme == SYSTEM {
        return Banner::System;
    }
    let theme = if wav(theme, kind).is_some() { theme } else { DEFAULT };
    match std::env::var_os("HOME").and_then(|h| sound_in(&std::path::PathBuf::from(h).join("Library/Sounds"), theme, kind)) {
        Some(name) => Banner::Named(name),
        None => Banner::System,
    }
}

/// Copy `kind` of `theme` into `dir` as "T1K <theme>-<kind>.wav" and return that name.
/// macOS 26 turns down sounds named from an app's bundle and plays its default instead, but
/// takes the user's alert sounds folder (as Coo found).
fn sound_in(dir: &std::path::Path, theme: &str, kind: Kind) -> Option<String> {
    let bytes = wav(theme, kind)?;
    let name = format!("T1K {}", file_name(theme, kind));
    let dest = dir.join(&name);
    // Rewritten when a release changes the sound.
    if std::fs::metadata(&dest).map(|m| m.len()).ok() != Some(bytes.len() as u64) {
        if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&dest, bytes)) {
            log::warn!("could not copy a notification sound: {e}");
            return None;
        }
    }
    Some(name)
}

fn file_name(theme: &str, kind: Kind) -> String {
    let kind = match kind {
        Kind::Alert => "alert",
        Kind::Soft => "soft",
    };
    format!("{theme}-{kind}.wav")
}

/// About where macOS's alert volume sits, so previews sound like banners.
const PREVIEW_VOLUME: f32 = 0.75;

/// Play `kind` of `theme` now: Settings previews, and banners from a build without a bundle.
pub fn play(theme: &str, kind: Kind) {
    if let Some(bytes) = wav(theme, kind) {
        imp::play(bytes, PREVIEW_VOLUME);
    } else if theme == SYSTEM {
        imp::play_file("/System/Library/Sounds/Tink.aiff", PREVIEW_VOLUME);
    }
}

/// How long a preview of the pair runs (alert, then soft), for the playing wave in Settings.
pub fn pair_length(theme: &str) -> std::time::Duration {
    let secs = |b: &[u8]| (b.len().saturating_sub(44)) as f32 / (44100. * 2.);
    let s = match (wav(theme, Kind::Alert), wav(theme, Kind::Soft)) {
        (Some(a), Some(b)) => secs(a).min(0.9) + 0.25 + secs(b),
        _ if theme == SYSTEM => 0.6,
        _ => 0.,
    };
    std::time::Duration::from_secs_f32(s)
}

#[cfg(target_os = "macos")]
mod imp {
    use cocoa::base::{id, nil, BOOL, YES};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetOnlineDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayIsAsleep(display: u32) -> u32;
        fn CGSessionCopyCurrentDictionary() -> id;
    }

    fn screen_locked() -> bool {
        unsafe {
            let dict = CGSessionCopyCurrentDictionary();
            if dict == nil {
                return false;
            }
            let key = NSString::alloc(nil).init_str("CGSSessionScreenIsLocked");
            let v: id = msg_send![dict, objectForKey: key];
            let locked = v != nil && {
                let b: BOOL = msg_send![v, boolValue];
                b == YES
            };
            let _: () = msg_send![key, release];
            let _: () = msg_send![dict, release];
            locked
        }
    }

    /// Nobody's looking: the screen is locked, or no display is awake.
    pub fn screens_off() -> bool {
        if screen_locked() {
            return true;
        }
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        let err = unsafe { CGGetOnlineDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
        if err != 0 {
            return false;
        }
        ids[..count as usize].iter().all(|&d| unsafe { CGDisplayIsAsleep(d) } != 0)
    }

    thread_local! {
        // NSSound stops when released: keep the last one alive.
        static CURRENT: std::cell::Cell<id> = const { std::cell::Cell::new(nil) };
    }

    fn start(sound: id, volume: f32) {
        if sound == nil {
            return;
        }
        unsafe {
            let _: () = msg_send![sound, setVolume: volume];
            let _: BOOL = msg_send![sound, play];
        }
        let old = CURRENT.with(|c| c.replace(sound));
        if old != nil {
            unsafe {
                let _: () = msg_send![old, release];
            }
        }
    }

    pub fn play(bytes: &[u8], volume: f32) {
        unsafe {
            let data: id = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() length: bytes.len()];
            let sound: id = msg_send![class!(NSSound), alloc];
            let sound: id = msg_send![sound, initWithData: data];
            start(sound, volume);
        }
    }

    pub fn play_file(path: &str, volume: f32) {
        unsafe {
            let path = NSString::alloc(nil).init_str(path);
            let sound: id = msg_send![class!(NSSound), alloc];
            let sound: id = msg_send![sound, initWithContentsOfFile: path byReference: YES];
            let _: () = msg_send![path, release];
            start(sound, volume);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn play(_: &[u8], _: f32) {}
    pub fn play_file(_: &str, _: f32) {}
    pub fn screens_off() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_banners_sounds_once() {
        let t0 = std::time::Instant::now();
        assert!(due(None, t0));
        assert!(!due(Some(t0), t0 + std::time::Duration::from_millis(1500)));
        assert!(due(Some(t0), t0 + ALERT_GAP));
    }

    #[test]
    fn every_theme_has_both_small_mono_wavs() {
        assert_eq!(THEMES[0].id, DEFAULT);
        for t in THEMES {
            for (k, w) in [("alert", t.alert), ("soft", t.soft)] {
                assert_eq!(&w[0..4], b"RIFF", "{} {k}", t.id);
                assert_eq!(u16::from_le_bytes([w[22], w[23]]), 1, "{} {k} mono", t.id);
                assert!(w.len() < 300_000, "{} {k} too big", t.id);
            }
        }
        assert!(wav(SILENT, Kind::Alert).is_none());
    }

    #[test]
    fn sounds_are_copied_into_the_alert_sounds_folder() {
        let dir = std::env::temp_dir().join(format!("t1k-sounds-{}", std::process::id()));
        for t in THEMES {
            for kind in [Kind::Alert, Kind::Soft] {
                let name = sound_in(&dir, t.id, kind).unwrap();
                assert_eq!(name, format!("T1K {}", file_name(t.id, kind)));
                assert_eq!(std::fs::read(dir.join(&name)).unwrap(), wav(t.id, kind).unwrap());
            }
        }
        assert_eq!(sound_in(&dir, SILENT, Kind::Alert), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
