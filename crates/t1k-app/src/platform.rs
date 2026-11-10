//! Small macOS integrations: Dock badge, notifications, PATH for helper tools.

/// Show `n` on the Dock icon (hidden at 0). Must run on the main thread.
#[cfg(target_os = "macos")]
pub fn set_dock_badge(n: usize) {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let tile: id = msg_send![app, dockTile];
        let label: id = if n == 0 { nil } else { NSString::alloc(nil).init_str(&n.to_string()) };
        let _: () = msg_send![tile, setBadgeLabel: label];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_badge(_: usize) {}

/// Dock icon artwork for `interface.app_icon`, plain and needs-you. None = the bundle's icon.
pub fn dock_icon_png(style: &str, alert: bool) -> Option<&'static [u8]> {
    match (style, alert) {
        ("prompt", false) => Some(include_bytes!("../../../resources/icons/prompt.png")),
        ("prompt", true) => Some(include_bytes!("../../../resources/icons/prompt-alert.png")),
        ("wordmark", false) => Some(include_bytes!("../../../resources/icons/wordmark.png")),
        ("wordmark", true) => Some(include_bytes!("../../../resources/icons/wordmark-alert.png")),
        // The face opens its eyes in red.
        (_, true) => Some(include_bytes!("../../../resources/icon-alert.png")),
        (_, false) => None,
    }
}

/// Set the Dock icon to `style` (face, prompt, wordmark), in its needs-you variant when
/// `alert`. Must run on the main thread.
#[cfg(target_os = "macos")]
pub fn set_dock_icon(style: &str, alert: bool) {
    use cocoa::base::{id, nil};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let Some(png) = dock_icon_png(style, alert) else {
            // nil restores the bundle's icon.
            let _: () = msg_send![app, setApplicationIconImage: nil];
            return;
        };
        let data: id = msg_send![class!(NSData), dataWithBytes: png.as_ptr() length: png.len()];
        let image: id = msg_send![class!(NSImage), alloc];
        let image: id = msg_send![image, initWithData: data];
        if image != nil {
            let _: () = msg_send![app, setApplicationIconImage: image];
            let _: () = msg_send![image, release];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_icon(_: &str, _: bool) {}

/// Post a notification banner. Uses `osascript` until T1K ships a signed bundle with its own
/// notification permission; clicking it opens Script Editor's context, not the tab (yet).
pub fn notify(title: &str, subtitle: &str, body: &str) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
        esc(body),
        esc(title),
        esc(subtitle)
    );
    let _ = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// PATH for helper tools (`gh`). Apps launched from Finder get a minimal PATH, so add the
/// usual Homebrew and user locations.
pub fn tool_path() -> String {
    let mut parts: Vec<String> = std::env::var("PATH").unwrap_or_default().split(':').map(String::from).collect();
    let home = std::env::var("HOME").unwrap_or_default();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", &format!("{home}/.local/bin"), &format!("{home}/bin")] {
        if !parts.iter().any(|p| p == extra) {
            parts.push(extra.to_string());
        }
    }
    parts.join(":")
}

// ---- notifications with click-to-focus ------------------------------------------------------

/// Tab ids from notifications the user clicked, drained by the workspace each tick.
static CLICKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static UN_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// (tab id, minutes) from a reminder banner's Snooze buttons, drained each tick.
static SNOOZED: std::sync::Mutex<Vec<(String, u64)>> = std::sync::Mutex::new(Vec::new());

pub fn take_snoozed() -> Vec<(String, u64)> {
    std::mem::take(&mut *SNOOZED.lock().unwrap())
}

/// Tab ids whose notification was clicked since the last call.
pub fn take_clicked() -> Vec<String> {
    std::mem::take(&mut *CLICKED.lock().unwrap())
}

/// Set up UserNotifications when running as T1K.app (it needs a bundle identifier; a bare
/// binary keeps using osascript). Asks for permission once. Call on the main thread.
#[cfg(target_os = "macos")]
pub fn init_notifications() {
    use block::ConcreteBlock;
    use cocoa::base::{id, nil, BOOL};
    use objc::declare::ClassDecl;
    use objc::runtime::{Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    #[link(name = "UserNotifications", kind = "framework")]
    extern "C" {}

    unsafe {
        let bundle: id = msg_send![class!(NSBundle), mainBundle];
        let ident: id = msg_send![bundle, bundleIdentifier];
        if ident == nil {
            return;
        }

        extern "C" fn did_receive(_: &Object, _: Sel, _center: id, response: id, done: id) {
            unsafe {
                let notification: id = msg_send![response, notification];
                let request: id = msg_send![notification, request];
                let content: id = msg_send![request, content];
                let info: id = msg_send![content, userInfo];
                use cocoa::foundation::NSString;
                let key = NSString::alloc(nil).init_str("tab");
                let tab: id = msg_send![info, objectForKey: key];
                let action: id = msg_send![response, actionIdentifier];
                let action = if action == nil {
                    String::new()
                } else {
                    let a: *const std::os::raw::c_char = msg_send![action, UTF8String];
                    if a.is_null() { String::new() } else { std::ffi::CStr::from_ptr(a).to_string_lossy().into_owned() }
                };
                if tab != nil {
                    let s: *const std::os::raw::c_char = msg_send![tab, UTF8String];
                    if !s.is_null() {
                        let s = std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned();
                        match action.strip_prefix("snooze-").and_then(|m| m.parse::<u64>().ok()) {
                            Some(mins) => SNOOZED.lock().unwrap().push((s, mins)),
                            None => CLICKED.lock().unwrap().push(s),
                        }
                    }
                }
                let block = &*(done as *const block::Block<(), ()>);
                block.call(());
            }
        }
        // Show banners even while T1K is frontmost (the tab may be one you aren't looking at).
        extern "C" fn will_present(_: &Object, _: Sel, _center: id, _n: id, done: id) {
            unsafe {
                let block = &*(done as *const block::Block<(usize,), ()>);
                // UNNotificationPresentationOptionList | Banner | Sound
                block.call(((1 << 3) | (1 << 4) | (1 << 1),));
            }
        }

        let class = match ClassDecl::new("T1KNotificationDelegate", class!(NSObject)) {
            Some(mut decl) => {
                decl.add_method(
                    sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
                    did_receive as extern "C" fn(&Object, Sel, id, id, id),
                );
                decl.add_method(
                    sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
                    will_present as extern "C" fn(&Object, Sel, id, id, id),
                );
                decl.register()
            }
            None => class!(T1KNotificationDelegate),
        };
        let delegate: id = msg_send![class, new];
        let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let _: () = msg_send![center, setDelegate: delegate];
        // Reminder banners carry Snooze buttons.
        {
            use cocoa::foundation::NSString;
            let s = |t: &str| NSString::alloc(nil).init_str(t);
            let a10: id = msg_send![class!(UNNotificationAction), actionWithIdentifier: s("snooze-10") title: s("Snooze 10 min") options: 0usize];
            let a60: id = msg_send![class!(UNNotificationAction), actionWithIdentifier: s("snooze-60") title: s("Snooze 1 hour") options: 0usize];
            let actions: id = msg_send![class!(NSArray), arrayWithObjects: [a10, a60].as_ptr() count: 2usize];
            let none: id = msg_send![class!(NSArray), array];
            let cat: id = msg_send![class!(UNNotificationCategory), categoryWithIdentifier: s("t1k-reminder") actions: actions intentIdentifiers: none options: 0usize];
            let set: id = msg_send![class!(NSSet), setWithObject: cat];
            let _: () = msg_send![center, setNotificationCategories: set];
        }
        let handler = ConcreteBlock::new(|granted: BOOL, _err: id| {
            if granted != cocoa::base::NO {
                UN_READY.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        })
        .copy();
        // UNAuthorizationOptionBadge | Sound | Alert
        let _: () = msg_send![center, requestAuthorizationWithOptions: (1usize | 2 | 4) completionHandler: &*handler];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn init_notifications() {}

/// Post a banner that focuses `tab` when clicked. Falls back to osascript (no click target)
/// outside the app bundle or without permission.
pub fn notify_tab(tab: &str, title: &str, subtitle: &str, body: &str) {
    notify_tab_in(tab, title, subtitle, body, None)
}

/// A reminder banner: like `notify_tab`, with Snooze buttons.
pub fn notify_reminder(tab: &str, title: &str, body: &str) {
    notify_tab_in(tab, "T1K", title, body, Some("t1k-reminder"))
}

fn notify_tab_in(tab: &str, title: &str, subtitle: &str, body: &str, category: Option<&str>) {
    #[cfg(target_os = "macos")]
    if UN_READY.load(std::sync::atomic::Ordering::SeqCst) {
        use cocoa::base::{id, nil};
        use cocoa::foundation::NSString;
        use objc::{class, msg_send, sel, sel_impl};
        unsafe {
            let s = |t: &str| NSString::alloc(nil).init_str(t);
            let content: id = msg_send![class!(UNMutableNotificationContent), new];
            let _: () = msg_send![content, setTitle: s(title)];
            let _: () = msg_send![content, setSubtitle: s(subtitle)];
            let _: () = msg_send![content, setBody: s(body)];
            let info: id = msg_send![class!(NSDictionary), dictionaryWithObject: s(tab) forKey: s("tab")];
            let _: () = msg_send![content, setUserInfo: info];
            if let Some(c) = category {
                let _: () = msg_send![content, setCategoryIdentifier: s(c)];
            }
            let sound: id = msg_send![class!(UNNotificationSound), defaultSound];
            let _: () = msg_send![content, setSound: sound];
            let ident = s(&format!("t1k-{tab}-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
            let request: id = msg_send![class!(UNNotificationRequest), requestWithIdentifier: ident content: content trigger: nil];
            let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
            let _: () = msg_send![center, addNotificationRequest: request withCompletionHandler: nil];
            let _: () = msg_send![content, release];
        }
        return;
    }
    notify(title, subtitle, body);
}

/// Apps that can open a file, default first, as (display name, app path): what Finder lists
/// under "Open With".
#[cfg(target_os = "macos")]
pub fn apps_for(path: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};
    let to_string = |s: id| -> Option<String> {
        if s == nil {
            return None;
        }
        let p: *const std::os::raw::c_char = unsafe { msg_send![s, UTF8String] };
        (!p.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned())
    };
    let mut out: Vec<(String, std::path::PathBuf)> = Vec::new();
    unsafe {
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let fm: id = msg_send![class!(NSFileManager), defaultManager];
        let p = NSString::alloc(nil).init_str(&path.display().to_string());
        let url: id = msg_send![class!(NSURL), fileURLWithPath: p];
        let default: id = msg_send![ws, URLForApplicationToOpenURL: url];
        let all: id = msg_send![ws, URLsForApplicationsToOpenURL: url];
        let mut urls: Vec<id> = Vec::new();
        if default != nil {
            urls.push(default);
        }
        if all != nil {
            let n: usize = msg_send![all, count];
            for i in 0..n {
                urls.push(msg_send![all, objectAtIndex: i]);
            }
        }
        for u in urls {
            let Some(app) = to_string(msg_send![u, path]) else { continue };
            if out.iter().any(|(_, p)| p.to_str() == Some(app.as_str())) {
                continue;
            }
            let app_ns = NSString::alloc(nil).init_str(&app);
            let name = to_string(msg_send![fm, displayNameAtPath: app_ns]).unwrap_or_else(|| app.clone());
            out.push((name.trim_end_matches(".app").to_string(), std::path::PathBuf::from(app)));
        }
    }
    out
}

#[cfg(not(target_os = "macos"))]
pub fn apps_for(_: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    Vec::new()
}

/// Open a file with a specific app (`open -a`).
pub fn open_with(app: &std::path::Path, file: &std::path::Path) {
    let _ = std::process::Command::new("/usr/bin/open").arg("-a").arg(app).arg(file).spawn().map(|mut c| std::thread::spawn(move || c.wait()));
}

/// System Settings › Accessibility › Display › Reduce motion.
pub fn reduce_motion() -> bool {
    use cocoa::base::{id, BOOL, NO};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let on: BOOL = msg_send![ws, accessibilityDisplayShouldReduceMotion];
        on != NO
    }
}

// ---- quitting from outside T1K -----------------------------------------------------------------

/// T1K's own quit paths set this just before quitting, after they've asked about running tabs.
pub static ALLOW_QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// macOS asked T1K to quit (Dock › Quit, logout, another app): the workspace turns it into ⌘Q.
pub static QUIT_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Quit for real (the question was asked, or there was nothing to ask about).
pub fn quit(cx: &mut gpui::App) {
    // Windows closing from here on aren't being closed by the person: they stay saved.
    crate::windows::set_quitting();
    ALLOW_QUIT.store(true, std::sync::atomic::Ordering::SeqCst);
    cx.quit();
}

/// GPUI lets macOS terminate the app outright, so Dock › Quit skipped the "still running"
/// question that ⌘Q asks. Answer applicationShouldTerminate: on GPUI's app delegate: cancel,
/// and let the workspace ask (it quits through `quit`, which lets the next request through).
#[cfg(target_os = "macos")]
pub fn guard_terminate() {
    use cocoa::base::id;
    use objc::runtime::{class_addMethod, object_getClass, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    extern "C" fn should_terminate(_: &Object, _: Sel, _: id) -> u64 {
        use std::sync::atomic::Ordering;
        // An update installing (Sparkle quits the app to swap it) always goes through.
        let installing = crate::updater::snapshot().phase == crate::updater::Phase::Installing;
        if ALLOW_QUIT.load(Ordering::SeqCst) || installing {
            1 // NSTerminateNow
        } else {
            QUIT_REQUESTED.store(true, Ordering::SeqCst);
            0 // NSTerminateCancel
        }
    }
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let delegate: id = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        let cls = object_getClass(delegate as *const Object) as *mut objc::runtime::Class;
        let imp: extern "C" fn(&Object, Sel, id) -> u64 = should_terminate;
        class_addMethod(cls, sel!(applicationShouldTerminate:), std::mem::transmute::<extern "C" fn(&Object, Sel, id) -> u64, objc::runtime::Imp>(imp), c"Q@:@".as_ptr());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn guard_terminate() {}

// ---- global hotkey (quick terminal, canvas v3-ay) -------------------------------------------
// A Carbon hot key: it works in every app and needs no Accessibility permission.

static HOTKEY_FIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
mod hotkey {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicPtr, AtomicBool, Ordering};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventHotKeyID {
        signature: u32,
        id: u32,
    }
    #[repr(C)]
    struct EventTypeSpec {
        class: u32,
        kind: u32,
    }
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn RegisterEventHotKey(code: u32, mods: u32, id: EventHotKeyID, target: *mut c_void, options: u32, out: *mut *mut c_void) -> i32;
        fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
        fn GetApplicationEventTarget() -> *mut c_void;
        fn InstallEventHandler(
            target: *mut c_void,
            handler: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
            count: u32,
            types: *const EventTypeSpec,
            user: *mut c_void,
            out: *mut *mut c_void,
        ) -> i32;
    }
    static CURRENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static HANDLER: AtomicBool = AtomicBool::new(false);

    extern "C" fn pressed(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> i32 {
        super::HOTKEY_FIRED.store(true, Ordering::SeqCst);
        0
    }

    pub fn unregister() {
        let old = CURRENT.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !old.is_null() {
            unsafe { UnregisterEventHotKey(old) };
        }
    }

    /// False when macOS refuses (another app registered the same hot key).
    pub fn register(code: u32, mods: u32) -> bool {
        unregister();
        unsafe {
            if !HANDLER.swap(true, Ordering::SeqCst) {
                // 'keyb', kEventHotKeyPressed.
                let spec = EventTypeSpec { class: u32::from_be_bytes(*b"keyb"), kind: 5 };
                InstallEventHandler(GetApplicationEventTarget(), pressed, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut());
            }
            let mut out: *mut c_void = std::ptr::null_mut();
            let id = EventHotKeyID { signature: u32::from_be_bytes(*b"t1kq"), id: 1 };
            let status = RegisterEventHotKey(code, mods, id, GetApplicationEventTarget(), 0, &mut out);
            if status != 0 || out.is_null() {
                return false;
            }
            CURRENT.store(out, Ordering::SeqCst);
            true
        }
    }
}

/// A shortcut like "ctrl-alt-t" as a macOS key code and Carbon modifiers. None if it has no
/// modifier (a bare key would be taken from every app) or names a key we can't place.
pub fn hotkey_parse(shortcut: &str) -> Option<(u32, u32)> {
    let mut mods = 0u32;
    let mut key = None;
    for part in shortcut.split('-').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" => mods |= 0x100,
            "shift" => mods |= 0x200,
            "alt" | "option" | "opt" => mods |= 0x800,
            "ctrl" | "control" => mods |= 0x1000,
            k => key = Some(k.to_string()),
        }
    }
    let code = match key?.as_str() {
        "a" => 0, "s" => 1, "d" => 2, "f" => 3, "h" => 4, "g" => 5, "z" => 6, "x" => 7, "c" => 8, "v" => 9, "b" => 11, "q" => 12,
        "w" => 13, "e" => 14, "r" => 15, "y" => 16, "t" => 17, "1" => 18, "2" => 19, "3" => 20, "4" => 21, "6" => 22, "5" => 23,
        "=" => 24, "9" => 25, "7" => 26, "minus" => 27, "8" => 28, "0" => 29, "]" => 30, "o" => 31, "u" => 32, "[" => 33, "i" => 34,
        "p" => 35, "enter" => 36, "l" => 37, "j" => 38, "'" => 39, "k" => 40, ";" => 41, "\\" => 42, "," => 43, "/" => 44, "n" => 45,
        "m" => 46, "." => 47, "tab" => 48, "space" => 49, "`" => 50, "escape" => 53,
        "f1" => 122, "f2" => 120, "f3" => 99, "f4" => 118, "f5" => 96, "f6" => 97, "f7" => 98, "f8" => 100, "f9" => 101, "f10" => 109,
        "f11" => 103, "f12" => 111,
        _ => return None,
    };
    (mods != 0).then_some((code, mods))
}

/// Register the global hot key (replacing any earlier one). False if macOS refuses it.
pub fn hotkey_register(shortcut: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        hotkey_parse(shortcut).is_some_and(|(code, mods)| hotkey::register(code, mods))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = shortcut;
        false
    }
}

pub fn hotkey_unregister() {
    #[cfg(target_os = "macos")]
    hotkey::unregister();
}

/// True once per press of the global hot key.
pub fn take_hotkey() -> bool {
    HOTKEY_FIRED.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// Where the pointer is, in GPUI's global coordinates (top-left origin), and the menu bar's
/// height.
#[cfg(target_os = "macos")]
pub fn pointer_and_menu_bar() -> (gpui::Point<gpui::Pixels>, f32) {
    use cocoa::base::id;
    use cocoa::foundation::{NSPoint, NSRect};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let p: NSPoint = msg_send![class!(NSEvent), mouseLocation];
        let screens: id = msg_send![class!(NSScreen), screens];
        let primary: id = msg_send![screens, firstObject];
        let frame: NSRect = if primary.is_null() { NSRect::new(NSPoint::new(0., 0.), cocoa::foundation::NSSize::new(0., 0.)) } else { msg_send![primary, frame] };
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let menu: id = msg_send![app, mainMenu];
        let bar: f64 = if menu.is_null() { 24. } else { msg_send![menu, menuBarHeight] };
        (gpui::point(gpui::px(p.x as f32), gpui::px((frame.size.height - p.y) as f32)), bar as f32)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn pointer_and_menu_bar() -> (gpui::Point<gpui::Pixels>, f32) {
    (gpui::point(gpui::px(0.), gpui::px(0.)), 0.)
}
