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

/// The Dock icon while a tab needs you: the face with its eyes open in red.
#[cfg(target_os = "macos")]
const ALERT_ICON: &[u8] = include_bytes!("../../../resources/icon-alert.png");

/// Swap the Dock icon to the alert face (true) or back to the app icon (false).
/// Must run on the main thread.
#[cfg(target_os = "macos")]
pub fn set_dock_alert(on: bool) {
    use cocoa::base::{id, nil};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        if !on {
            // nil restores the bundle's icon.
            let _: () = msg_send![app, setApplicationIconImage: nil];
            return;
        }
        let data: id = msg_send![class!(NSData), dataWithBytes: ALERT_ICON.as_ptr() length: ALERT_ICON.len()];
        let image: id = msg_send![class!(NSImage), alloc];
        let image: id = msg_send![image, initWithData: data];
        if image != nil {
            let _: () = msg_send![app, setApplicationIconImage: image];
            let _: () = msg_send![image, release];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_alert(_: bool) {}

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
