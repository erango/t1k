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
