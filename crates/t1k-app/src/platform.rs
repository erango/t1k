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
                if tab != nil {
                    let s: *const std::os::raw::c_char = msg_send![tab, UTF8String];
                    if !s.is_null() {
                        let s = std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned();
                        CLICKED.lock().unwrap().push(s);
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
