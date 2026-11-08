//! Custom icons beyond emoji (canvas v3-ag, option A): an installed app's icon or an image of
//! your own, for a tab or a group. Both are kept as 256 px PNGs in the support folder's
//! `icons/`, so the icon stays when the app is uninstalled or the original image moves.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gpui::{div, img, prelude::*, px, AnyElement, Div};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use t1k_themes::UiColors;

/// A tab's or group's chosen icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    Emoji(String),
    /// An app, by bundle id (or its path when it has none).
    App(String),
    /// An image, by the SHA-256 of its stored PNG.
    Image(String),
}

/// session.json form: `{"kind": "emoji"|"app"|"image", "value": ...}`. Sessions written before
/// app and image icons hold a plain string, which is an emoji.
#[derive(Serialize, Deserialize)]
struct Tagged {
    kind: String,
    value: String,
}

impl Serialize for Icon {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (kind, value) = match self {
            Icon::Emoji(v) => ("emoji", v),
            Icon::App(v) => ("app", v),
            Icon::Image(v) => ("image", v),
        };
        Tagged { kind: kind.into(), value: value.clone() }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Plain(String),
            Tagged(Tagged),
        }
        Ok(match Stored::deserialize(d)? {
            Stored::Plain(e) => Icon::Emoji(e),
            Stored::Tagged(t) => match t.kind.as_str() {
                "app" => Icon::App(t.value),
                "image" => Icon::Image(t.value),
                // An unknown kind from a newer T1K: keep the session, show it as text.
                _ => Icon::Emoji(t.value),
            },
        })
    }
}

fn icons_dir() -> Option<PathBuf> {
    Some(crate::session::support_dir()?.join("icons"))
}

fn short_hash(s: &str) -> String {
    let h = Sha256::digest(s.as_bytes());
    h.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn is_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A stored image's PNG.
pub fn image_png(sha: &str) -> Option<PathBuf> {
    is_sha(sha).then(|| icons_dir().map(|d| d.join(format!("{sha}.png")))).flatten()
}

/// The 256 px PNG kept for an app icon, made from the app if it isn't there yet.
pub fn app_png(value: &str) -> Option<PathBuf> {
    let out = icons_dir()?.join(format!("app-{}.png", short_hash(value)));
    if out.is_file() {
        return Some(out);
    }
    // Don't retry an app that's gone on every frame.
    static MISSING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut missing = MISSING.lock().ok()?;
    let missing = missing.get_or_insert_with(HashSet::new);
    if missing.contains(value) {
        return None;
    }
    if resolve_app(value).is_some_and(|app| render_app_icon(&app, &out, 256)) {
        return Some(out);
    }
    missing.insert(value.to_string());
    None
}

/// Where an app is now: its path, or what Launch Services says for a bundle id.
fn resolve_app(value: &str) -> Option<PathBuf> {
    if value.starts_with('/') {
        return Path::new(value).is_dir().then(|| PathBuf::from(value));
    }
    if let Some(a) = apps().iter().find(|a| a.bundle_id.as_deref() == Some(value)) {
        return Some(a.path.clone());
    }
    platform_app_for_bundle(value)
}

/// An app icon (drawn 1.22× and clipped to the tile, which removes macOS's built-in margin) or
/// an image (filling the tile), as the tile's content. None when it can't be drawn.
pub fn content(icon: &Icon, size: f32) -> Option<AnyElement> {
    match icon {
        Icon::Emoji(_) => None,
        Icon::App(v) => Some(overscaled(app_png(v)?, size).into_any_element()),
        Icon::Image(sha) => {
            let png = image_png(sha).filter(|p| p.is_file())?;
            let png = crate::app_icon::sized_icon(&png, pixels(size)).unwrap_or(png);
            Some(img(png).size(px(size)).into_any_element())
        }
    }
}

/// `size` points in the window's pixels.
pub fn pixels(size: f32) -> u32 {
    let scale = crate::app_icon::SCALE.load(Ordering::Relaxed).max(1);
    (size * scale as f32).round() as u32
}

/// A Mac app icon drawn 1.22× its tile and centered, for a parent that clips to the tile.
pub fn overscaled(png: PathBuf, size: f32) -> Div {
    let over = (size * 1.22).round();
    let png = crate::app_icon::sized_icon(&png, pixels(over)).unwrap_or(png);
    let shift = -((over - size) / 2.);
    div().absolute().left(px(shift)).top(px(shift)).child(img(png).size(px(over)))
}

/// A group's icon: an emoji as text, an app or image in a tile of `size`.
pub fn mark(icon: &Icon, ui: UiColors, size: f32, emoji_size: f32) -> Div {
    match icon {
        Icon::Emoji(e) => div().text_size(px(emoji_size)).child(e.clone()),
        other => {
            let tile = div().relative().size(px(size)).flex_none().rounded(px((size * 0.24).round())).overflow_hidden().opacity(ui.icon_alpha);
            match content(other, size) {
                Some(c) => tile.child(c),
                None => tile.bg(ui.bg_control),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Installed apps

#[derive(Clone, Debug, PartialEq)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: PathBuf,
}

impl AppInfo {
    /// What a chosen app icon stores.
    pub fn value(&self) -> String {
        self.bundle_id.clone().unwrap_or_else(|| self.path.display().to_string())
    }

    /// Its icon for the picker grid, at `px` pixels (made in the background; None until then).
    pub fn grid_png(&self, px: u32) -> Option<PathBuf> {
        Some(icons_dir()?.join("apps").join(format!("{}-{px}.png", short_hash(&self.value()))))
    }
}

static APPS: Mutex<Option<Arc<Vec<AppInfo>>>> = Mutex::new(None);
static SCANNING: AtomicBool = AtomicBool::new(false);

/// The installed apps, alphabetical (empty until the first scan is done).
pub fn apps() -> Arc<Vec<AppInfo>> {
    APPS.lock().ok().and_then(|a| a.clone()).unwrap_or_default()
}

pub fn apps_loaded() -> bool {
    APPS.lock().is_ok_and(|a| a.is_some())
}

/// /Applications, ~/Applications and /System/Applications, one folder deep (Utilities).
pub fn scan_apps() -> Vec<AppInfo> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut out: Vec<AppInfo> = Vec::new();
    let mut seen = HashSet::new();
    for root in [PathBuf::from("/Applications"), home.join("Applications"), PathBuf::from("/System/Applications")] {
        let mut dirs = vec![root];
        let mut depth = 0;
        while !dirs.is_empty() && depth < 2 {
            let mut next = Vec::new();
            for d in dirs {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "app") {
                        let info = read_app(&p);
                        // The same app in two places (or twice by name): keep the first.
                        if seen.insert(info.bundle_id.clone().unwrap_or_else(|| info.name.to_lowercase())) {
                            out.push(info);
                        }
                    } else if p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                        next.push(p);
                    }
                }
            }
            dirs = next;
            depth += 1;
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn read_app(p: &Path) -> AppInfo {
    let file_name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let plist = plist::Value::from_file(p.join("Contents/Info.plist")).ok();
    let dict = plist.as_ref().and_then(|v| v.as_dictionary());
    let get = |k: &str| dict.and_then(|d| d.get(k)).and_then(|v| v.as_string()).map(str::to_string).filter(|s| !s.is_empty());
    AppInfo { name: get("CFBundleDisplayName").filter(|n| !n.contains('$')).unwrap_or(file_name), bundle_id: get("CFBundleIdentifier"), path: p.to_path_buf() }
}

/// Scan the apps once per run, off the main thread, then make their grid icons in batches.
pub fn ensure_apps(cx: &mut gpui::Context<crate::workspace::Workspace>) {
    if apps_loaded() || SCANNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let scan = cx.background_executor().spawn(async { scan_apps() });
    cx.spawn(async move |this, cx| {
        let list = Arc::new(scan.await);
        if let Ok(mut a) = APPS.lock() {
            *a = Some(list.clone());
        }
        let _ = this.update(cx, |_, cx| cx.notify());
        let px = pixels(GRID_ICON);
        for chunk in list.chunks(16) {
            let chunk = chunk.to_vec();
            let made = cx
                .background_executor()
                .spawn(async move {
                    let mut made = false;
                    for a in chunk {
                        if let Some(out) = a.grid_png(px).filter(|p| !p.is_file()) {
                            made |= render_app_icon(&a.path, &out, px);
                        }
                    }
                    made
                })
                .await;
            if made {
                let _ = this.update(cx, |_, cx| cx.notify());
            }
        }
    })
    .detach();
}

/// Scan and make grid icons right now (debug overlays: the first frame must be complete).
pub fn load_apps_now() {
    let list = Arc::new(scan_apps());
    let px = pixels(GRID_ICON);
    for a in list.iter() {
        if let Some(out) = a.grid_png(px).filter(|p| !p.is_file()) {
            render_app_icon(&a.path, &out, px);
        }
    }
    if let Ok(mut a) = APPS.lock() {
        *a = Some(list);
    }
}

/// Icon size in the picker's app grid.
pub const GRID_ICON: f32 = 28.;

/// Words too common in folder and tab names to say which app they mean.
const APP_STOP: &[&str] = &[
    "app", "apps", "the", "and", "for", "with", "new", "old", "dev", "test", "tests", "main", "src", "lib", "tmp", "fix", "pro",
    "plugin", "plugins", "mac", "macos", "desktop", "repo", "project", "work", "my",
];

/// Bundle id parts that every app shares.
const BUNDLE_STOP: &[&str] = &["com", "org", "net", "io", "app", "apple", "desktop", "mac", "macos", "osx", "inc", "www", "ltd", "dev", "co", "electron"];

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// How well `w` names this app: the whole name, a word of it, part of its bundle id, or (for
/// longer words) the start of its name.
fn word_score(a: &AppInfo, w: &str) -> u32 {
    let name = a.name.to_lowercase();
    let joined: String = name.chars().filter(|c| c.is_alphanumeric()).collect();
    if joined == w {
        100
    } else if words(&name).iter().any(|n| n == w) {
        80
    } else if a.bundle_id.as_ref().is_some_and(|b| b.to_lowercase().split(['.', '-']).filter(|p| !BUNDLE_STOP.contains(p)).any(|p| p == w)) {
        60
    } else if w.len() >= 4 && joined.starts_with(w) {
        40
    } else {
        0
    }
}

/// Apps that the words in `texts` (a tab's title, its group or repo name) point to, best
/// first; only those scoring at least `min`.
pub fn suggest_apps(apps: &[AppInfo], texts: &[String], min: u32, n: usize) -> Vec<usize> {
    let ws: Vec<String> = texts.iter().flat_map(|t| words(t)).filter(|w| w.len() >= 3 && !APP_STOP.contains(&w.as_str())).collect();
    let mut scored: Vec<(u32, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let s = ws.iter().map(|w| word_score(a, w)).max().unwrap_or(0);
            (s >= min && s > 0).then_some((s, i))
        })
        .collect();
    // Stable: equal scores keep alphabetical order.
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    scored.into_iter().take(n).map(|(_, i)| i).collect()
}

/// Apps whose name matches a search: names starting with it, then a word starting with it,
/// then containing it.
pub fn search_apps(apps: &[AppInfo], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u8, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let name = a.name.to_lowercase();
            let rank = if name.starts_with(&q) {
                0
            } else if words(&name).iter().any(|w| w.starts_with(&q)) {
                1
            } else if name.contains(&q) || a.bundle_id.as_ref().is_some_and(|b| b.to_lowercase().split('.').skip(1).any(|p| p.starts_with(&q))) {
                2
            } else {
                return None;
            };
            Some((rank, i))
        })
        .collect();
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, i)| i).collect()
}

// ---------------------------------------------------------------------------------------------
// Images

/// Files the Image tab takes (NSImage reads them all).
pub fn is_icon_image(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "svg" | "icns" | "gif" | "webp" | "heic" | "tif" | "tiff" | "bmp")
    )
}

/// The image drawn into a PNG of at most 1024 px a side for the cropper (it can't show SVG
/// or ICNS itself): its path and pixel size.
pub fn load_for_crop(src: &Path) -> Option<(PathBuf, f32, f32)> {
    let (w, h) = platform_image_size(src)?;
    if w <= 0. || h <= 0. {
        return None;
    }
    let k = 1024. / w.max(h);
    let (pw, ph) = ((w * k).round().max(1.) as u32, (h * k).round().max(1.) as u32);
    // A fresh name each time: GPUI caches images by path.
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let out = std::env::temp_dir().join(format!("t1k-crop-{}-{}.png", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    render_png(Source::File(src), None, pw, ph, &out).then_some((out, pw as f32, ph as f32))
}

/// Crop `src` to `unit` (x, y, w, h as fractions, from the top left), save it as a 256 px PNG
/// in `icons/` named by its hash, and return the hash.
pub fn save_crop(src: &Path, unit: [f64; 4]) -> Option<String> {
    let dir = icons_dir()?;
    let _ = std::fs::create_dir_all(&dir);
    let tmp = dir.join(format!(".crop-{}.png", std::process::id()));
    if !render_png(Source::File(src), Some(unit), 256, 256, &tmp) {
        return None;
    }
    let bytes = std::fs::read(&tmp).ok()?;
    let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    std::fs::rename(&tmp, dir.join(format!("{sha}.png"))).ok()?;
    Some(sha)
}

/// Delete a stored image and its sized copies (best effort; the caller checked nothing uses it).
pub fn remove_image(sha: &str) {
    let (Some(dir), true) = (icons_dir(), is_sha(sha)) else { return };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n == format!("{sha}.png") || (n.starts_with(&format!("{sha}-")) && n.ends_with(".png")) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Delete stored images nothing uses (at launch, for files left by a crash or by tabs closed
/// for good).
pub fn sweep_images(used: &HashSet<String>) {
    let Some(dir) = icons_dir() else { return };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(stem) = n.strip_suffix(".png") else { continue };
        let sha = stem.split('-').next().unwrap_or_default();
        if is_sha(sha) && !used.contains(sha) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing with AppKit

enum Source<'a> {
    File(&'a Path),
    /// The Finder icon of this app (or any file).
    Icon(&'a Path),
}

/// Draw an app's icon into a `px` square PNG.
pub fn render_app_icon(app: &Path, out: &Path, px: u32) -> bool {
    render_png(Source::Icon(app), None, px, px, out)
}

/// Draw `src` (or the `from` part of it: x, y, w, h as fractions from the top left) into a
/// `w`×`h` px PNG at `out`. NSImage picks the best representation for the size and resamples
/// with high-quality interpolation; safe off the main thread (the context is per thread).
#[cfg(target_os = "macos")]
fn render_png(src: Source, from: Option<[f64; 4]>, w: u32, h: u32, out: &Path) -> bool {
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
    use objc::{class, msg_send, sel, sel_impl};
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let ok = (|| {
            let image: id = match src {
                Source::File(p) => {
                    let s = NSString::alloc(nil).init_str(&p.display().to_string());
                    let i: id = msg_send![class!(NSImage), alloc];
                    let i: id = msg_send![i, initWithContentsOfFile: s];
                    if i == nil {
                        return false;
                    }
                    msg_send![i, autorelease]
                }
                Source::Icon(p) => {
                    let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
                    let s = NSString::alloc(nil).init_str(&p.display().to_string());
                    msg_send![ws, iconForFile: s]
                }
            };
            if image == nil {
                return false;
            }
            let size: NSSize = msg_send![image, size];
            if size.width <= 0. || size.height <= 0. {
                return false;
            }
            let space = NSString::alloc(nil).init_str("NSCalibratedRGBColorSpace");
            let rep: id = msg_send![class!(NSBitmapImageRep), alloc];
            let rep: id = msg_send![rep, initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
                pixelsWide: w as isize
                pixelsHigh: h as isize
                bitsPerSample: 8isize
                samplesPerPixel: 4isize
                hasAlpha: YES
                isPlanar: NO
                colorSpaceName: space
                bytesPerRow: 0isize
                bitsPerPixel: 0isize];
            if rep == nil {
                return false;
            }
            let rep: id = msg_send![rep, autorelease];
            let ctx: id = msg_send![class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: rep];
            if ctx == nil {
                return false;
            }
            let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
            let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: ctx];
            // NSImageInterpolationHigh
            let _: () = msg_send![ctx, setImageInterpolation: 3usize];
            // AppKit's origin is the bottom left.
            let from = match from {
                Some([x, y, uw, uh]) => NSRect::new(NSPoint::new(x * size.width, (1. - y - uh) * size.height), NSSize::new(uw * size.width, uh * size.height)),
                None => NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.)),
            };
            let to = NSRect::new(NSPoint::new(0., 0.), NSSize::new(w as f64, h as f64));
            // NSCompositingOperationSourceOver
            let _: () = msg_send![image, drawInRect: to fromRect: from operation: 2usize fraction: 1.0f64];
            let _: () = msg_send![ctx, flushGraphics];
            let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
            let props: id = msg_send![class!(NSDictionary), dictionary];
            // NSBitmapImageFileTypePNG = 4
            let png: id = msg_send![rep, representationUsingType: 4usize properties: props];
            if png == nil {
                return false;
            }
            let dest = NSString::alloc(nil).init_str(&out.display().to_string());
            let ok: cocoa::base::BOOL = msg_send![png, writeToFile: dest atomically: YES];
            ok != NO
        })();
        pool.drain();
        ok
    }
}

#[cfg(not(target_os = "macos"))]
fn render_png(_: Source, _: Option<[f64; 4]>, _: u32, _: u32, _: &Path) -> bool {
    false
}

/// An image's size in points as NSImage reads it (only its shape matters).
#[cfg(target_os = "macos")]
fn platform_image_size(p: &Path) -> Option<(f32, f32)> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSAutoreleasePool, NSSize, NSString};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let s = NSString::alloc(nil).init_str(&p.display().to_string());
        let i: id = msg_send![class!(NSImage), alloc];
        let i: id = msg_send![i, initWithContentsOfFile: s];
        let size = if i == nil {
            None
        } else {
            let size: NSSize = msg_send![i, size];
            let _: () = msg_send![i, release];
            Some((size.width as f32, size.height as f32))
        };
        pool.drain();
        size
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_image_size(_: &Path) -> Option<(f32, f32)> {
    None
}

#[cfg(target_os = "macos")]
fn platform_app_for_bundle(bundle: &str) -> Option<PathBuf> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSAutoreleasePool, NSString};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let b = NSString::alloc(nil).init_str(bundle);
        let url: id = msg_send![ws, URLForApplicationWithBundleIdentifier: b];
        let path = if url == nil {
            None
        } else {
            let p: id = msg_send![url, path];
            let c: *const std::os::raw::c_char = msg_send![p, UTF8String];
            (!c.is_null()).then(|| PathBuf::from(std::ffi::CStr::from_ptr(c).to_string_lossy().to_string()))
        };
        pool.drain();
        path
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_app_for_bundle(_: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, bundle: &str) -> AppInfo {
        AppInfo { name: name.into(), bundle_id: Some(bundle.into()), path: PathBuf::from(format!("/Applications/{name}.app")) }
    }

    fn sample() -> Vec<AppInfo> {
        vec![
            app("Docker", "com.docker.docker"),
            app("Figma", "com.figma.Desktop"),
            app("Linear", "com.linear"),
            app("Notes", "com.apple.Notes"),
            app("Postgres", "com.postgresapp.Postgres2"),
            app("Visual Studio Code", "com.microsoft.VSCode"),
        ]
    }

    #[test]
    fn suggests_apps_by_name_and_bundle_id() {
        let a = sample();
        assert_eq!(suggest_apps(&a, &["figma-plugin".into()], 60, 6), vec![1]);
        // Bundle id parts count ("vscode"), common words don't ("plugin", "desktop").
        assert_eq!(suggest_apps(&a, &["vscode settings".into()], 60, 6), vec![5]);
        assert!(suggest_apps(&a, &["desktop plugin app".into()], 1, 6).is_empty());
        // Best match first: the whole name beats the start of one.
        assert_eq!(suggest_apps(&a, &["post".into(), "linear".into()], 1, 6), vec![2, 4]);
        // The start of a name is only a weak match: not enough for the Emoji tab's one app.
        assert!(suggest_apps(&a, &["post".into()], 60, 1).is_empty());
        assert_eq!(suggest_apps(&a, &["Release notes".into(), "docker".into()], 60, 6).len(), 2);
    }

    #[test]
    fn searches_apps() {
        let a = sample();
        assert_eq!(search_apps(&a, "fig"), vec![1]);
        assert_eq!(search_apps(&a, "code"), vec![5]);
        assert!(search_apps(&a, " ").is_empty());
    }

    #[test]
    fn icon_reads_plain_strings_and_tagged_values() {
        let old: Icon = serde_json::from_str("\"🦀\"").unwrap();
        assert_eq!(old, Icon::Emoji("🦀".into()));
        let app: Icon = serde_json::from_str(r#"{"kind":"app","value":"com.figma.Desktop"}"#).unwrap();
        assert_eq!(app, Icon::App("com.figma.Desktop".into()));
        for i in [Icon::Emoji("🐳".into()), Icon::App("/Applications/X.app".into()), Icon::Image("ab".repeat(32))] {
            let s = serde_json::to_string(&i).unwrap();
            assert!(s.starts_with("{\"kind\":"));
            assert_eq!(serde_json::from_str::<Icon>(&s).unwrap(), i);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn renders_and_crops_images() {
        let dir = std::env::temp_dir().join(format!("t1k-icon-crop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("a.svg");
        std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="red"/><rect x="20" width="20" height="20" fill="blue"/></svg>"#).unwrap();
        let (preview, w, h) = load_for_crop(&svg).expect("svg loads");
        assert_eq!((w, h), (1024., 512.));
        assert!(preview.is_file());
        let out = dir.join("crop.png");
        assert!(render_png(Source::File(&svg), Some([0.5, 0., 0.5, 1.]), 256, 256, &out));
        assert_eq!(&std::fs::read(&out).unwrap()[1..4], b"PNG");
        assert!(render_app_icon(Path::new("/System/Library/CoreServices/Finder.app"), &dir.join("f.png"), 64));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_file(preview);
    }
}
