//! Auto-update through Sparkle 2, with T1K drawing every surface itself.
//!
//! Sparkle checks the feed, downloads, verifies the EdDSA signature and installs. We hand it
//! a user driver (the `SPUUserDriver` protocol) built at runtime with the objc crate, so its
//! callbacks land here instead of in Sparkle's stock windows. Each callback updates a shared
//! `Snapshot` and parks Sparkle's reply block; the workspace redraws when `rev()` changes and
//! answers through `install_now`, `install_on_quit`, `skip` and friends.
//!
//! Sparkle.framework is loaded at runtime from the app bundle (bundle.sh embeds it), so a
//! plain `cargo run` works without it and simply reports updates as unavailable.
//!
//! Everything here runs on the main thread: Sparkle calls its user driver and delegate there,
//! and GPUI handlers run there too. Reply blocks live in a thread local for that reason.

use std::cell::RefCell;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::config::UpdatesCfg;

/// T1K's own feed. A raw file on main, so the beta channel (items tagged
/// `<sparkle:channel>beta</sparkle:channel>`) and stable share one appcast.
pub const DEFAULT_FEED: &str = "https://raw.githubusercontent.com/erango/t1k/main/appcast.xml";

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Phase {
    /// Nothing to report (also: Sparkle isn't bundled).
    #[default]
    Idle,
    /// A check you started is running.
    Checking,
    UpToDate,
    /// Found, not downloaded yet (automatic downloads are off, or you checked by hand).
    Available,
    Downloading { got: u64, total: u64 },
    Extracting,
    /// Downloaded and verified; installs on quit or on Restart now.
    Ready,
    Installing,
    Error(String),
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub phase: Phase,
    pub version: Option<String>,
    /// Bullet points from the appcast item's description.
    pub notes: Vec<String>,
    pub notes_url: Option<String>,
    pub last_check: Option<SystemTime>,
}

#[derive(Default)]
struct Shared {
    snap: Snapshot,
    rev: u64,
    /// Set when something you asked for has an answer to show (a manual check, an error).
    want_dialog: bool,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { snap: Snapshot { phase: Phase::Idle, version: None, notes: Vec::new(), notes_url: None, last_check: None }, rev: 0, want_dialog: false });

fn change(f: impl FnOnce(&mut Shared)) {
    let mut s = SHARED.lock().unwrap();
    f(&mut s);
    s.rev += 1;
}

pub fn snapshot() -> Snapshot {
    SHARED.lock().unwrap().snap.clone()
}

/// Bumped on every change; the workspace compares it each tick.
pub fn rev() -> u64 {
    SHARED.lock().unwrap().rev
}

/// True once per answer the user should see in the dialog.
pub fn take_dialog_request() -> bool {
    std::mem::take(&mut SHARED.lock().unwrap().want_dialog)
}

/// The pill and the ⋯ menu item show while this is true.
pub fn waiting() -> bool {
    matches!(snapshot().phase, Phase::Available | Phase::Ready)
}

/// Turn an appcast description (HTML list, Markdown bullets or plain lines) into bullets.
pub fn parse_notes(desc: &str) -> Vec<String> {
    let mut text = desc.replace("<li>", "\n- ").replace("<br>", "\n").replace("<br/>", "\n").replace("</p>", "\n");
    // Drop any remaining tags.
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    text = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    text.lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•']).trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use block::{Block, RcBlock};
    use cocoa::base::{id, nil, BOOL, NO, YES};
    use cocoa::foundation::NSString;
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Protocol, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    type Choice = RcBlock<(isize,), ()>;
    type Done = RcBlock<(), ()>;

    // SPUUserUpdateChoice
    const SKIP: isize = 0;
    const INSTALL: isize = 1;
    const DISMISS: isize = 2;

    #[derive(Default)]
    pub struct Parked {
        /// Answer to "update found" / "ready to install".
        pub choice: Option<Choice>,
        /// Cancels a check or download you started.
        pub cancel: Option<Done>,
        /// Acknowledges "up to date" / an error.
        pub ack: Option<Done>,
        /// From `willInstallUpdateOnQuit`: install a silently downloaded update right now.
        pub install_now: Option<Done>,
        pub updater: Option<usize>,
        pub channel_beta: bool,
        pub feed: String,
    }

    thread_local! {
        pub static PARKED: RefCell<Parked> = RefCell::new(Parked::default());
    }

    fn string(s: id) -> Option<String> {
        if s == nil {
            return None;
        }
        unsafe {
            let p: *const std::os::raw::c_char = msg_send![s, UTF8String];
            (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    }

    fn ns(s: &str) -> id {
        unsafe { NSString::alloc(nil).init_str(s) }
    }

    unsafe fn choice(b: id) -> Choice {
        RcBlock::copy(b as *mut Block<(isize,), ()>)
    }
    unsafe fn done(b: id) -> Option<Done> {
        (b != nil).then(|| RcBlock::copy(b as *mut Block<(), ()>))
    }

    fn error_text(e: id) -> String {
        unsafe {
            if e == nil {
                return "Something went wrong.".into();
            }
            let d: id = msg_send![e, localizedDescription];
            string(d).unwrap_or_else(|| "Something went wrong.".into())
        }
    }

    fn take_item(item: id) {
        let (version, notes, url) = unsafe {
            let v: id = msg_send![item, displayVersionString];
            let d: id = msg_send![item, itemDescription];
            let u: id = msg_send![item, fullReleaseNotesURL];
            let u: id = if u == nil { msg_send![item, releaseNotesURL] } else { u };
            let u: id = if u == nil { nil } else { msg_send![u, absoluteString] };
            (string(v), string(d).map(|d| parse_notes(&d)).unwrap_or_default(), string(u))
        };
        change(|s| {
            s.snap.version = version;
            s.snap.notes = notes;
            s.snap.notes_url = url;
        });
    }

    // ---- SPUUserDriver ----

    extern "C" fn permission(_: &Object, _: Sel, _req: id, reply: id) {
        // Never shown: the Welcome card and Settings own this choice. Answer from config.
        unsafe {
            let reply: RcBlock<(id,), ()> = RcBlock::copy(reply as *mut Block<(id,), ()>);
            let r: id = msg_send![class!(SUUpdatePermissionResponse), alloc];
            let r: id = msg_send![r, initWithAutomaticUpdateChecks: YES sendSystemProfile: NO];
            reply.call((r,));
        }
    }

    extern "C" fn user_check(_: &Object, _: Sel, cancel: id) {
        PARKED.with(|p| p.borrow_mut().cancel = unsafe { done(cancel) });
        change(|s| {
            s.snap.phase = Phase::Checking;
            s.want_dialog = true;
        });
    }

    extern "C" fn found(_: &Object, _: Sel, item: id, state: id, reply: id) {
        take_item(item);
        let (stage, user): (isize, BOOL) = unsafe { (msg_send![state, stage], msg_send![state, userInitiated]) };
        PARKED.with(|p| p.borrow_mut().choice = Some(unsafe { choice(reply) }));
        change(|s| {
            s.snap.phase = if stage == 0 { Phase::Available } else { Phase::Ready };
            s.snap.last_check = Some(SystemTime::now());
            s.want_dialog |= user == YES;
        });
    }

    extern "C" fn notes_data(_: &Object, _: Sel, _data: id) {}
    extern "C" fn notes_failed(_: &Object, _: Sel, _e: id) {}

    extern "C" fn not_found(_: &Object, _: Sel, _e: id, ack: id) {
        PARKED.with(|p| p.borrow_mut().ack = unsafe { done(ack) });
        change(|s| {
            s.snap.phase = Phase::UpToDate;
            s.snap.last_check = Some(SystemTime::now());
        });
    }

    extern "C" fn error(_: &Object, _: Sel, e: id, ack: id) {
        let msg = error_text(e);
        log::warn!("update: {msg}");
        PARKED.with(|p| p.borrow_mut().ack = unsafe { done(ack) });
        change(|s| {
            // Background failures stay quiet; a check you started shows them.
            let shown = matches!(s.snap.phase, Phase::Checking | Phase::Downloading { .. } | Phase::Extracting);
            s.snap.phase = Phase::Error(msg);
            s.want_dialog |= shown;
        });
    }

    extern "C" fn download_started(_: &Object, _: Sel, cancel: id) {
        PARKED.with(|p| p.borrow_mut().cancel = unsafe { done(cancel) });
        change(|s| s.snap.phase = Phase::Downloading { got: 0, total: 0 });
    }

    extern "C" fn download_length(_: &Object, _: Sel, total: u64) {
        change(|s| s.snap.phase = Phase::Downloading { got: 0, total });
    }

    extern "C" fn download_data(_: &Object, _: Sel, n: u64) {
        change(|s| {
            if let Phase::Downloading { got, total } = s.snap.phase {
                s.snap.phase = Phase::Downloading { got: got + n, total };
            }
        });
    }

    extern "C" fn extracting(_: &Object, _: Sel) {
        change(|s| s.snap.phase = Phase::Extracting);
    }
    extern "C" fn extract_progress(_: &Object, _: Sel, _p: f64) {}

    extern "C" fn ready(_: &Object, _: Sel, reply: id) {
        PARKED.with(|p| p.borrow_mut().choice = Some(unsafe { choice(reply) }));
        change(|s| s.snap.phase = Phase::Ready);
    }

    extern "C" fn installing(_: &Object, _: Sel, _terminated: BOOL, _retry: id) {
        change(|s| s.snap.phase = Phase::Installing);
    }

    extern "C" fn installed(_: &Object, _: Sel, _relaunched: BOOL, ack: id) {
        if let Some(a) = unsafe { done(ack) } {
            unsafe { a.call(()) };
        }
    }

    extern "C" fn dismiss(_: &Object, _: Sel) {
        PARKED.with(|p| {
            let mut p = p.borrow_mut();
            p.choice = None;
            p.cancel = None;
            p.ack = None;
        });
        change(|s| {
            // A staged update (Ready) still installs on quit, and a finished answer (up to
            // date, error) stays readable in About and Settings. Anything in flight is over.
            if matches!(s.snap.phase, Phase::Checking | Phase::Available | Phase::Downloading { .. } | Phase::Extracting) {
                s.snap.phase = Phase::Idle;
            }
        });
    }

    extern "C" fn focus(_: &Object, _: Sel) {
        change(|s| s.want_dialog = true);
    }

    // ---- SPUUpdaterDelegate ----

    extern "C" fn channels(_: &Object, _: Sel, _u: id) -> id {
        unsafe {
            if PARKED.with(|p| p.borrow().channel_beta) {
                msg_send![class!(NSSet), setWithObject: ns("beta")]
            } else {
                msg_send![class!(NSSet), set]
            }
        }
    }

    extern "C" fn feed_url(_: &Object, _: Sel, _u: id) -> id {
        let f = PARKED.with(|p| p.borrow().feed.clone());
        ns(if f.is_empty() { DEFAULT_FEED } else { &f })
    }

    extern "C" fn no_prompt(_: &Object, _: Sel, _u: id) -> BOOL {
        NO
    }

    extern "C" fn will_install_on_quit(_: &Object, _: Sel, _u: id, item: id, now: id) -> BOOL {
        take_item(item);
        PARKED.with(|p| p.borrow_mut().install_now = unsafe { done(now) });
        change(|s| s.snap.phase = Phase::Ready);
        YES
    }

    extern "C" fn did_not_find(_: &Object, _: Sel, _u: id) {
        change(|s| {
            s.snap.last_check = Some(SystemTime::now());
            if matches!(s.snap.phase, Phase::Idle | Phase::Checking | Phase::Error(_)) {
                s.snap.phase = Phase::UpToDate;
            }
        });
    }

    fn driver_class() -> &'static Class {
        let mut d = ClassDecl::new("T1KUpdateDriver", class!(NSObject)).expect("T1KUpdateDriver");
        unsafe {
            if let Some(p) = Protocol::get("SPUUserDriver") {
                d.add_protocol(p);
            }
            if let Some(p) = Protocol::get("SPUUpdaterDelegate") {
                d.add_protocol(p);
            }
            d.add_method(sel!(showUpdatePermissionRequest:reply:), permission as extern "C" fn(&Object, Sel, id, id));
            d.add_method(sel!(showUserInitiatedUpdateCheckWithCancellation:), user_check as extern "C" fn(&Object, Sel, id));
            d.add_method(sel!(showUpdateFoundWithAppcastItem:state:reply:), found as extern "C" fn(&Object, Sel, id, id, id));
            d.add_method(sel!(showUpdateReleaseNotesWithDownloadData:), notes_data as extern "C" fn(&Object, Sel, id));
            d.add_method(sel!(showUpdateReleaseNotesFailedToDownloadWithError:), notes_failed as extern "C" fn(&Object, Sel, id));
            d.add_method(sel!(showUpdateNotFoundWithError:acknowledgement:), not_found as extern "C" fn(&Object, Sel, id, id));
            d.add_method(sel!(showUpdaterError:acknowledgement:), error as extern "C" fn(&Object, Sel, id, id));
            d.add_method(sel!(showDownloadInitiatedWithCancellation:), download_started as extern "C" fn(&Object, Sel, id));
            d.add_method(sel!(showDownloadDidReceiveExpectedContentLength:), download_length as extern "C" fn(&Object, Sel, u64));
            d.add_method(sel!(showDownloadDidReceiveDataOfLength:), download_data as extern "C" fn(&Object, Sel, u64));
            d.add_method(sel!(showDownloadDidStartExtractingUpdate), extracting as extern "C" fn(&Object, Sel));
            d.add_method(sel!(showExtractionReceivedProgress:), extract_progress as extern "C" fn(&Object, Sel, f64));
            d.add_method(sel!(showReadyToInstallAndRelaunch:), ready as extern "C" fn(&Object, Sel, id));
            d.add_method(sel!(showInstallingUpdateWithApplicationTerminated:retryTerminatingApplication:), installing as extern "C" fn(&Object, Sel, BOOL, id));
            d.add_method(sel!(showUpdateInstalledAndRelaunched:acknowledgement:), installed as extern "C" fn(&Object, Sel, BOOL, id));
            d.add_method(sel!(dismissUpdateInstallation), dismiss as extern "C" fn(&Object, Sel));
            d.add_method(sel!(showUpdateInFocus), focus as extern "C" fn(&Object, Sel));
            d.add_method(sel!(allowedChannelsForUpdater:), channels as extern "C" fn(&Object, Sel, id) -> id);
            d.add_method(sel!(feedURLStringForUpdater:), feed_url as extern "C" fn(&Object, Sel, id) -> id);
            d.add_method(sel!(updaterShouldPromptForPermissionToCheckForUpdates:), no_prompt as extern "C" fn(&Object, Sel, id) -> BOOL);
            d.add_method(sel!(updater:willInstallUpdateOnQuit:immediateInstallationBlock:), will_install_on_quit as extern "C" fn(&Object, Sel, id, id, id) -> BOOL);
            d.add_method(sel!(updaterDidNotFindUpdate:), did_not_find as extern "C" fn(&Object, Sel, id));
        }
        d.register()
    }

    /// Load Sparkle from the app bundle and start the updater. False when not bundled.
    pub fn start(cfg: &UpdatesCfg) -> bool {
        unsafe {
            let main: id = msg_send![class!(NSBundle), mainBundle];
            let frameworks: id = msg_send![main, privateFrameworksPath];
            let Some(dir) = string(frameworks) else { return false };
            let path = format!("{dir}/Sparkle.framework");
            if !std::path::Path::new(&path).exists() {
                log::info!("update: Sparkle not bundled; updates off");
                return false;
            }
            let bundle: id = msg_send![class!(NSBundle), bundleWithPath: ns(&path)];
            let loaded: BOOL = if bundle == nil { NO } else { msg_send![bundle, load] };
            if loaded != YES || Class::get("SPUUpdater").is_none() {
                log::error!("update: couldn't load {path}");
                return false;
            }
            PARKED.with(|p| {
                let mut p = p.borrow_mut();
                p.channel_beta = cfg.channel == "beta";
                p.feed = cfg.feed.clone();
            });
            let driver: id = msg_send![driver_class(), new];
            let updater: id = msg_send![Class::get("SPUUpdater").unwrap(), alloc];
            let updater: id = msg_send![updater, initWithHostBundle: main applicationBundle: main userDriver: driver delegate: driver];
            let mut err: id = nil;
            let ok: BOOL = msg_send![updater, startUpdater: &mut err];
            if ok != YES {
                log::error!("update: Sparkle didn't start: {}", error_text(err));
                return false;
            }
            let last: id = msg_send![updater, lastUpdateCheckDate];
            if last != nil {
                let t: f64 = msg_send![last, timeIntervalSince1970];
                change(|s| s.snap.last_check = Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64(t.max(0.))));
            }
            PARKED.with(|p| p.borrow_mut().updater = Some(updater as usize));
            apply(cfg);
            true
        }
    }

    fn updater() -> Option<id> {
        PARKED.with(|p| p.borrow().updater.map(|u| u as id))
    }

    /// Push config into Sparkle (its own defaults are not the source of truth).
    pub fn apply(cfg: &UpdatesCfg) {
        let Some(u) = updater() else { return };
        let beta = cfg.channel == "beta";
        let changed = PARKED.with(|p| {
            let mut p = p.borrow_mut();
            let changed = p.channel_beta != beta || p.feed != cfg.feed;
            p.channel_beta = beta;
            p.feed = cfg.feed.clone();
            changed
        });
        unsafe {
            let check: BOOL = if cfg.check { YES } else { NO };
            let download: BOOL = if cfg.download { YES } else { NO };
            let _: () = msg_send![u, setAutomaticallyChecksForUpdates: check];
            let _: () = msg_send![u, setAutomaticallyDownloadsUpdates: download];
            let _: () = msg_send![u, setUpdateCheckInterval: 86400.0f64];
            if changed {
                let _: () = msg_send![u, resetUpdateCycle];
            }
        }
    }

    pub fn check_now() {
        let Some(u) = updater() else { return };
        // A finished "up to date" / error must be acknowledged before a new session starts.
        if let Some(a) = PARKED.with(|p| p.borrow_mut().ack.take()) {
            unsafe { a.call(()) };
        }
        unsafe {
            let can: BOOL = msg_send![u, canCheckForUpdates];
            if can == YES {
                let _: () = msg_send![u, checkForUpdates];
            } else {
                // A session is already open (e.g. an update found in the background): show it.
                change(|s| s.want_dialog = true);
            }
        }
    }

    /// What the daily check does: silent, and downloads when that's on.
    pub fn check_background() {
        if let Some(u) = updater() {
            unsafe {
                let _: () = msg_send![u, checkForUpdatesInBackground];
            }
        }
    }

    fn answer(c: isize) -> bool {
        match PARKED.with(|p| p.borrow_mut().choice.take()) {
            Some(b) => {
                unsafe { b.call((c,)) };
                true
            }
            None => false,
        }
    }

    pub fn install_now() {
        if let Some(b) = PARKED.with(|p| p.borrow_mut().install_now.take()) {
            change(|s| s.snap.phase = Phase::Installing);
            unsafe { b.call(()) };
        } else {
            answer(INSTALL);
        }
    }

    /// Download (when only found) or keep a ready update for the next quit.
    pub fn download() {
        answer(INSTALL);
    }

    pub fn install_on_quit() {
        // A silently downloaded update is already set to install on quit; for one that came
        // through the dialog, Dismiss at "ready" keeps it staged for termination.
        if snapshot().phase == Phase::Ready {
            answer(DISMISS);
            change(|s| s.snap.phase = Phase::Ready);
        }
    }

    pub fn later() {
        answer(DISMISS);
    }

    pub fn skip() {
        if answer(SKIP) {
            change(|s| {
                s.snap.phase = Phase::Idle;
                s.snap.version = None;
            });
        }
    }

    pub fn cancel() {
        if let Some(b) = PARKED.with(|p| p.borrow_mut().cancel.take()) {
            unsafe { b.call(()) };
        }
    }

    pub fn acknowledge() {
        if let Some(a) = PARKED.with(|p| p.borrow_mut().ack.take()) {
            unsafe { a.call(()) };
        }
    }

    pub fn can_skip() -> bool {
        PARKED.with(|p| p.borrow().choice.is_some())
    }

    pub fn available() -> bool {
        updater().is_some()
    }
}

#[cfg(target_os = "macos")]
pub use mac::{acknowledge, apply, available, can_skip, cancel, check_background, check_now, download, install_now, install_on_quit, later, skip, start};

#[cfg(not(target_os = "macos"))]
mod other {
    pub fn start(_: &super::UpdatesCfg) -> bool { false }
    pub fn apply(_: &super::UpdatesCfg) {}
    pub fn available() -> bool { false }
    pub fn can_skip() -> bool { false }
    pub fn check_now() {}
    pub fn check_background() {}
    pub fn download() {}
    pub fn install_now() {}
    pub fn install_on_quit() {}
    pub fn later() {}
    pub fn skip() {}
    pub fn cancel() {}
    pub fn acknowledge() {}
}
#[cfg(not(target_os = "macos"))]
pub use other::*;

#[cfg(test)]
mod tests {
    #[test]
    fn notes_from_html_and_markdown() {
        let html = "<h2>0.2.0</h2><ul><li>Codex shows live status</li><li>Settings &amp; search</li></ul>";
        assert_eq!(super::parse_notes(html), vec!["0.2.0", "Codex shows live status", "Settings & search"]);
        let md = "- One\n- Two\n\n# Heading\n";
        assert_eq!(super::parse_notes(md), vec!["One", "Two"]);
    }
}
