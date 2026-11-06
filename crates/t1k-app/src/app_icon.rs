//! Little program badges for sidebar rows: what's running in each tab at a glance.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gpui::{div, img, prelude::*, px, rgb, Div, Hsla};
use t1k_themes::UiColors;

use crate::workspace::Tab;

/// (glyph, color) for a program name; None means "use a monogram".
fn known(program: &str) -> Option<(&'static str, u32)> {
    let p = program.trim_start_matches('-').to_ascii_lowercase();
    Some(match p.as_str() {
        "claude" => ("✳", 0xD97757),
        "codex" => ("◎", 0x10A37F),
        "gemini" => ("✦", 0x4C8BF5),
        "agy" | "antigravity" => ("◭", 0x4C8BF5),
        "grok" => ("X", 0xE6E8EB),
        "aider" => ("◆", 0x14B014),
        "vim" | "nvim" | "vi" => ("v", 0x57A143),
        "emacs" => ("e", 0x7F5AB6),
        "nano" | "micro" | "hx" | "helix" => ("✎", 0x8C939C),
        "ssh" | "mosh" | "et" => ("⇄", 0x6CA6F5),
        "git" | "lazygit" | "tig" | "gitui" => ("⑂", 0xF05133),
        "node" | "npm" | "npx" | "pnpm" | "yarn" | "bun" | "deno" => ("⬢", 0x5FA04E),
        "python" | "python3" | "ipython" | "uv" | "pip" => ("py", 0x4B8BBE),
        "cargo" | "rustc" | "rust-analyzer" => ("⚙", 0xDEA584),
        "go" => ("go", 0x00ADD8),
        "docker" | "lazydocker" | "podman" => ("◫", 0x2496ED),
        "kubectl" | "k9s" | "helm" | "kubectx" => ("⎈", 0x326CE5),
        "htop" | "btop" | "top" | "glances" => ("▤", 0x5FCB8E),
        "less" | "more" | "man" | "bat" => ("≡", 0x8C939C),
        "make" | "just" => ("⚒", 0xC99AF0),
        "psql" | "mysql" | "sqlite3" | "redis-cli" | "mongosh" => ("⛁", 0x336791),
        "tail" | "watch" => ("↧", 0x8C939C),
        _ => return None,
    })
}

/// Mac apps that a terminal program belongs to, by bundle name.
fn app_names(program: &str) -> Vec<String> {
    let p = program.trim_start_matches('-').to_ascii_lowercase();
    let mapped: &[&str] = match p.as_str() {
        "claude" => &["Claude"],
        "codex" => &["ChatGPT", "Codex"],
        "code" | "code-insiders" => &["Visual Studio Code", "Visual Studio Code - Insiders"],
        "cursor" => &["Cursor"],
        "zed" => &["Zed", "Zed Preview"],
        "docker" | "lazydocker" | "docker-compose" => &["Docker", "OrbStack"],
        "orb" | "orbctl" => &["OrbStack"],
        "gh" => &["GitHub Desktop"],
        "psql" => &["Postgres", "TablePlus"],
        "redis-cli" => &["Redis Insight"],
        "nvim" => &["Neovide"],
        "emacs" => &["Emacs"],
        "ollama" => &["Ollama"],
        "agy" | "antigravity" => &["Antigravity"],
        "grok" => &["Grok"],
        "podman" => &["Podman Desktop"],
        _ => &[],
    };
    mapped.iter().map(|s| s.to_string()).collect()
}

fn find_app(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    [PathBuf::from("/Applications"), home.join("Applications"), PathBuf::from("/System/Applications")]
        .into_iter()
        .map(|d| d.join(format!("{name}.app")))
        .find(|p| p.is_dir())
}

/// Cached PNG of a program's Mac app icon, if it has one. Computed once per program.
fn app_icon(program: &str) -> Option<PathBuf> {
    static CACHE: Mutex<Option<HashMap<String, Option<PathBuf>>>> = Mutex::new(None);
    let key = program.trim_start_matches('-').to_ascii_lowercase();
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(&key) {
        return hit.clone();
    }
    let found = app_names(&key).iter().find_map(|n| find_app(n)).and_then(|app| {
        let dir = crate::session::support_dir()?.join("icons");
        let out = dir.join(format!("{key}.png"));
        if out.is_file() || write_icon_png(&app, &out) {
            Some(out)
        } else {
            None
        }
    });
    cache.insert(key, found.clone());
    found
}

/// Render an app's Finder icon (NSWorkspace) to a 64 px PNG.
#[cfg(target_os = "macos")]
fn write_icon_png(app: &Path, out: &Path) -> bool {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSSize, NSString};
    use objc::{class, msg_send, sel, sel_impl};
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    unsafe {
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let path = NSString::alloc(nil).init_str(&app.display().to_string());
        let image: id = msg_send![ws, iconForFile: path];
        if image == nil {
            return false;
        }
        let _: () = msg_send![image, setSize: NSSize::new(64., 64.)];
        let tiff: id = msg_send![image, TIFFRepresentation];
        if tiff == nil {
            return false;
        }
        let rep: id = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        if rep == nil {
            return false;
        }
        let props: id = msg_send![class!(NSDictionary), dictionary];
        // NSBitmapImageFileTypePNG = 4
        let png: id = msg_send![rep, representationUsingType: 4usize properties: props];
        if png == nil {
            return false;
        }
        let dest = NSString::alloc(nil).init_str(&out.display().to_string());
        let ok: cocoa::base::BOOL = msg_send![png, writeToFile: dest atomically: cocoa::base::YES];
        ok != cocoa::base::NO
    }
}

#[cfg(not(target_os = "macos"))]
fn write_icon_png(_: &Path, _: &Path) -> bool {
    false
}

/// A 16 px rounded badge for the tab's program: Claude's spark when a session is attached,
/// a prompt mark at the shell, a known tool's mark, else the program's first letter.
pub fn badge(tab: &Tab, ui: UiColors, size: f32) -> Div {
    // Small icons fill their tile edge to edge with a tighter corner, so they read at 16-22 px.
    let radius = (size * 0.24).round();
    let base = div().size(px(size)).flex_none().rounded(px(radius)).overflow_hidden().flex().items_center().justify_center();
    // A live agent session names the program (its process may show as node or python).
    let live_agent = tab.agent.as_ref().filter(|a| a.status != t1k_agent::Status::Ended);
    let program = match live_agent {
        Some(a) => Some(a.kind.program()),
        None => tab.program.as_deref(),
    };
    // Debug aid for screenshots: T1K_DEBUG_PROGRAM=<name> shows that program's badge everywhere.
    let debug = std::env::var("T1K_DEBUG_PROGRAM").ok();
    let program = debug.as_deref().or(program);
    let Some(program) = program else {
        // Shell prompt.
        return base
            .bg(ui.bg_control)
            .text_size(px((size * 0.5).round()))
            .font_family("JetBrains Mono")
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(ui.text_secondary)
            .child(">_");
    };
    // The real Mac app's icon when one matches (Claude, Docker, Cursor...). macOS icons carry
    // ~10% transparent margin; drawing them 1.22× and clipping to the tile removes it.
    if let Some(png) = app_icon(program) {
        let over = (size * 1.22).round();
        let shift = -((over - size) / 2.);
        return base.relative().child(div().absolute().left(px(shift)).top(px(shift)).child(img(png).size(px(over))));
    }
    let (glyph, color): (String, Hsla) = match known(program) {
        Some((g, c)) => (g.to_string(), rgb(c).into()),
        None => {
            let first = program.trim_start_matches('-').chars().next().unwrap_or('?').to_ascii_uppercase();
            (first.to_string(), ui.text_secondary)
        }
    };
    let two = glyph.chars().count() > 1;
    base.bg(color.opacity(0.22))
        .text_size(px((size * if two { 0.48 } else { 0.66 }).round()))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(color)
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::known;

    #[test]
    fn maps_common_programs() {
        assert_eq!(known("claude").unwrap().0, "✳");
        assert_eq!(known("-zsh"), None);
        assert_eq!(known("nvim").unwrap().0, "v");
        assert!(known("someprog").is_none());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod icon_tests {
    #[test]
    fn writes_png_for_an_installed_app() {
        // Finder is always there.
        let out = std::env::temp_dir().join(format!("t1k-icon-test-{}.png", std::process::id()));
        assert!(super::write_icon_png(std::path::Path::new("/System/Library/CoreServices/Finder.app"), &out));
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        let _ = std::fs::remove_file(out);
    }
}
