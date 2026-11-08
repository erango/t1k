//! "Get more fonts" (canvas v3-ac, option A): a short list of free coding fonts T1K can install
//! when asked, and the install itself.
//!
//! Each font is a pinned GitHub release asset with its SHA-256 (the upstream release, and the
//! Nerd Font build from ryanoasis/nerd-fonts for "with icons"). Installing downloads it with
//! curl, checks the hash, and copies only its font files into ~/Library/Fonts, never over a file
//! that's already there. Nothing is bundled and no password is needed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// One downloadable archive and what to take from it.
#[derive(Debug)]
pub struct Asset {
    pub url: &'static str,
    pub sha256: &'static str,
    /// Download size in bytes.
    pub bytes: u64,
    /// Font files whose path in the archive contains one of these (all when empty)…
    pub take: &'static [&'static str],
    /// …and none of these.
    pub skip: &'static [&'static str],
    /// The family it installs, for `terminal.font.family`.
    pub family: &'static str,
    /// GitHub's id for the release asset, to fetch it through the API when the release URL
    /// fails.
    pub asset_id: u64,
}

impl Asset {
    /// https://api.github.com/repos/<owner>/<repo>/releases/assets/<id>, from the release URL.
    pub fn api_url(&self) -> Option<String> {
        let rest = self.url.strip_prefix("https://github.com/")?;
        let mut parts = rest.splitn(3, '/');
        let (owner, repo) = (parts.next()?, parts.next()?);
        Some(format!("https://api.github.com/repos/{owner}/{repo}/releases/assets/{}", self.asset_id))
    }
}

#[derive(Debug)]
pub struct Font {
    pub id: &'static str,
    pub name: &'static str,
    pub license: &'static str,
    /// "ligatures" / "no ligatures", and anything else worth knowing about the shapes.
    pub ligatures: &'static str,
    pub note: &'static str,
    /// Where it comes from, as shown on hover.
    pub repo: &'static str,
    pub plain: Asset,
    /// The Nerd Font build ("with icons").
    pub icons: Asset,
    /// The sample line drawn in this font (2× PNG, for dark and light backgrounds), since a font
    /// that isn't installed can't draw its own preview.
    pub sample: (&'static [u8], &'static [u8]),
}

impl Font {
    pub fn asset(&self, icons: bool) -> &Asset {
        if icons {
            &self.icons
        } else {
            &self.plain
        }
    }

    /// "MIT · no ligatures · 0.6 MB · Warp's default".
    pub fn details(&self, icons: bool) -> String {
        let size = if icons { format!("{} with icons", mb(self.icons.bytes)) } else { mb(self.plain.bytes) };
        let mut parts = vec![self.license, self.ligatures];
        let size_ix = parts.len();
        if !self.note.is_empty() {
            parts.push(self.note);
        }
        let mut s: Vec<String> = parts.into_iter().map(String::from).collect();
        s.insert(size_ix, size);
        s.join(" · ")
    }

    /// Installed on this Mac, as either build.
    pub fn installed(&self, fonts: &[String]) -> bool {
        [self.plain.family, self.icons.family].iter().any(|f| fonts.iter().any(|n| n.eq_ignore_ascii_case(f)))
    }
}

/// "0.6 MB", "92 MB".
pub fn mb(bytes: u64) -> String {
    let m = bytes as f64 / 1_000_000.;
    if m >= 10. {
        format!("{m:.0} MB")
    } else {
        format!("{m:.1} MB")
    }
}

/// "0.3 of 0.6 MB", in the total's precision so the two line up.
pub fn progress(got: u64, total: u64) -> String {
    let t = total as f64 / 1_000_000.;
    let g = (got.min(total)) as f64 / 1_000_000.;
    if t >= 10. {
        format!("{g:.0} of {t:.0} MB")
    } else {
        format!("{g:.1} of {t:.1} MB")
    }
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// The catalog font another app names ("Hack", "JetBrainsMono-Regular", "Hack Nerd Font Mono"),
/// and whether it's the Nerd Font build.
pub fn find(name: &str) -> Option<(&'static Font, bool)> {
    let n = norm(name);
    // PostScript names carry a style: "JetBrainsMono-Regular".
    let base = norm(name.split('-').next().unwrap_or(name));
    let is = |want: &str| {
        let w = norm(want);
        n == w || base == w
    };
    for f in CATALOG.iter() {
        if is(f.icons.family) || is(&f.icons.family.replace(" Mono", "")) {
            return Some((f, true));
        }
        if is(f.name) || is(f.plain.family) {
            return Some((f, false));
        }
    }
    None
}

pub fn by_id(id: &str) -> Option<&'static Font> {
    CATALOG.iter().find(|f| f.id == id)
}

/// Whether an archive member is one of the font files to install.
pub fn wanted(a: &Asset, member: &str) -> bool {
    let name = member.rsplit('/').next().unwrap_or(member);
    let lower = name.to_lowercase();
    if name.starts_with('.') || member.contains("__MACOSX") || !(lower.ends_with(".ttf") || lower.ends_with(".otf")) {
        return false;
    }
    (a.take.is_empty() || a.take.iter().any(|t| member.contains(t))) && !a.skip.iter().any(|s| member.contains(s))
}

// ---- checksum -----------------------------------------------------------------------------------

pub fn sha256_hex(mut r: impl Read) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn checksum_matches(path: &Path, expected: &str) -> bool {
    std::fs::File::open(path).and_then(sha256_hex).is_ok_and(|h| h.eq_ignore_ascii_case(expected.trim()))
}

// ---- install ------------------------------------------------------------------------------------

pub const NO_RESPONSE: &str = "Couldn't download: github.com didn't respond.";
pub const BAD_CHECKSUM: &str = "The download didn't match its checksum.";

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Downloading,
    Installing,
    /// The font files in the Fonts folder: the ones copied, and any that were there already
    /// (kept, not replaced).
    Done(Vec<PathBuf>),
    Failed(String),
    Cancelled,
}

/// One install, shared between the download task and the UI that draws its progress.
pub struct Job {
    pub font: &'static Font,
    pub icons: bool,
    /// Bytes downloaded so far.
    pub got: AtomicU64,
    cancel: AtomicBool,
    phase: Mutex<Phase>,
    /// The UI has handed the new files to the text system.
    pub registered: AtomicBool,
}

impl Job {
    pub fn new(font: &'static Font, icons: bool) -> Arc<Job> {
        Arc::new(Job { font, icons, got: AtomicU64::new(0), cancel: AtomicBool::new(false), phase: Mutex::new(Phase::Downloading), registered: AtomicBool::new(false) })
    }
    pub fn asset(&self) -> &'static Asset {
        self.font.asset(self.icons)
    }
    pub fn phase(&self) -> Phase {
        self.phase.lock().map(|p| p.clone()).unwrap_or(Phase::Cancelled)
    }
    fn set(&self, p: Phase) {
        if let Ok(mut g) = self.phase.lock() {
            *g = p;
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn running(&self) -> bool {
        matches!(self.phase(), Phase::Downloading | Phase::Installing)
    }
    /// A job that shows a state without running anything (`T1K_DEBUG_FONTS=states`).
    pub fn fake(font: &'static Font, icons: bool, phase: Phase, got: u64) -> Arc<Job> {
        let j = Job::new(font, icons);
        j.got.store(got, Ordering::Relaxed);
        j.set(phase);
        j.registered.store(true, Ordering::Relaxed);
        j
    }
}

/// ~/Library/Fonts.
pub fn fonts_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Fonts"))
}

/// What a failed curl means for the person waiting (its exit code).
pub fn curl_reason(code: Option<i32>) -> String {
    match code {
        // 22: an HTTP error (the asset moved or GitHub refused).
        Some(22) => "Couldn't download: GitHub didn't have the file.".into(),
        Some(23) => "Couldn't save the download.".into(),
        // DNS, connect, timeouts, resets, TLS: from here, all the same thing.
        _ => NO_RESPONSE.into(),
    }
}

/// Download, check, and install a job's asset. Runs on the background executor; the download
/// is polled so a cancel kills curl within a tick, and the temp folder always goes.
pub async fn run(job: Arc<Job>, dest: PathBuf, ex: gpui::BackgroundExecutor) {
    let tmp = std::env::temp_dir().join(format!("t1k-font-{}-{}-{}", job.font.id, if job.icons { "nf" } else { "plain" }, std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let phase = download_and_install(&job, &tmp, &dest, &ex).await;
    let _ = std::fs::remove_dir_all(&tmp);
    if let Phase::Failed(r) = &phase {
        log::warn!("font install {}: {r}", job.font.id);
    }
    job.set(phase);
}

async fn download_and_install(job: &Job, tmp: &Path, dest: &Path, ex: &gpui::BackgroundExecutor) -> Phase {
    let a = job.asset();
    if std::fs::create_dir_all(tmp).is_err() {
        return Phase::Failed("Couldn't make a temporary folder.".into());
    }
    let file = tmp.join(a.url.rsplit('/').next().unwrap_or("font.zip"));
    // The release URL first, then the same asset through GitHub's API: when github.com's
    // download endpoint answers 5xx (seen while pinning these), the API still serves it.
    let mut reason = NO_RESPONSE.to_string();
    let mut ok = false;
    for (url, api) in [(a.url.to_string(), false)].into_iter().chain(a.api_url().map(|u| (u, true))) {
        let _ = std::fs::remove_file(&file);
        match fetch(job, &url, api, &file, ex).await {
            Ok(()) => {
                ok = true;
                break;
            }
            Err(Phase::Failed(r)) => reason = r,
            Err(other) => return other,
        }
    }
    if !ok {
        return Phase::Failed(reason);
    }
    job.got.store(a.bytes, Ordering::Relaxed);
    job.set(Phase::Installing);
    if !checksum_matches(&file, a.sha256) {
        return Phase::Failed(BAD_CHECKSUM.into());
    }
    if job.cancel.load(Ordering::Relaxed) {
        return Phase::Cancelled;
    }
    // bsdtar reads zip and tar.xz alike; only the font files are written out.
    let out = tmp.join("x");
    let _ = std::fs::create_dir_all(&out);
    let ok = Command::new("/usr/bin/tar")
        .arg("-xf")
        .arg(&file)
        .arg("-C")
        .arg(&out)
        // One pattern: bsdtar fails on an --include that matches nothing.
        .args(["--include", "*.[oOtT][tT][fF]"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let mut files = Vec::new();
    walk(&out, &mut files);
    let picked: Vec<PathBuf> = files.into_iter().filter(|p| p.strip_prefix(&out).ok().and_then(|r| r.to_str()).is_some_and(|r| wanted(a, r))).collect();
    if picked.is_empty() {
        return Phase::Failed(if ok { "The download had no fonts in it." } else { "Couldn't open the download." }.into());
    }
    match copy_new(&picked, dest) {
        Ok((written, kept)) => Phase::Done(written.into_iter().chain(kept).collect()),
        Err(e) => Phase::Failed(format!("Couldn't write to your Fonts folder: {}.", e.kind())),
    }
}

/// One curl download into `file`, polled for progress and cancel. Err(Cancelled) or
/// Err(Failed(reason)).
async fn fetch(job: &Job, url: &str, api: bool, file: &Path, ex: &gpui::BackgroundExecutor) -> Result<(), Phase> {
    let mut cmd = Command::new("/usr/bin/curl");
    // A stall of 20 s gives up (the API is the retry for the release URL), and --max-time
    // bounds a curl left behind if T1K quits mid-download.
    cmd.args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "15", "--speed-limit", "1", "--speed-time", "20", "--max-time", "1800"]);
    if api {
        cmd.args(["--retry", "2", "-H", "Accept: application/octet-stream"]);
    }
    let child = cmd.arg("-o").arg(file).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else { return Err(Phase::Failed(NO_RESPONSE.into())) };
    let status = loop {
        if job.cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Phase::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(_) => return Err(Phase::Failed(NO_RESPONSE.into())),
        }
        job.got.store(std::fs::metadata(file).map(|m| m.len()).unwrap_or(0), Ordering::Relaxed);
        ex.timer(Duration::from_millis(100)).await;
    };
    if status.success() {
        Ok(())
    } else {
        Err(Phase::Failed(curl_reason(status.code())))
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Copy each file into `dest` by its name, keeping any file already there (it may be a newer
/// build the person installed themselves). Returns (written, kept).
pub fn copy_new(files: &[PathBuf], dest: &Path) -> std::io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    std::fs::create_dir_all(dest)?;
    let (mut written, mut kept) = (Vec::new(), Vec::new());
    for f in files {
        let Some(name) = f.file_name() else { continue };
        let to = dest.join(name);
        // create_new: never replace, even if one appeared since we looked.
        let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(&to) {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                kept.push(to);
                continue;
            }
            Err(e) => return Err(e),
        };
        let mut src = std::fs::File::open(f)?;
        if let Err(e) = std::io::copy(&mut src, &mut out) {
            let _ = std::fs::remove_file(&to);
            return Err(e);
        }
        written.push(to);
    }
    Ok((written, kept))
}

// ---- catalog ------------------------------------------------------------------------------------

macro_rules! sample {
    ($id:literal) => {
        (
            include_bytes!(concat!("../../../resources/font-samples/", $id, "-dark.png")).as_slice(),
            include_bytes!(concat!("../../../resources/font-samples/", $id, "-light.png")).as_slice(),
        )
    };
}

/// The fonts on offer, in the order shown. Each asset was downloaded once and hashed when it
/// was pinned; sizes are the real download sizes. Nerd Fonts are v3.5.1, the "Mono" build
/// (single-width icons, so they line up in a terminal grid).
pub static CATALOG: [Font; 8] = [
    Font {
        id: "hack",
        name: "Hack",
        license: "MIT",
        ligatures: "no ligatures",
        note: "Warp's default",
        repo: "github.com/source-foundry/Hack",
        plain: Asset {
            url: "https://github.com/source-foundry/Hack/releases/download/v3.003/Hack-v3.003-ttf.zip",
            sha256: "0c2604631b1f055041c68a0e09ae4801acab6c5072ba2db6a822f53c3f8290ac",
            bytes: 615_776,
            take: &["ttf/Hack-"],
            skip: &[],
            family: "Hack",
            asset_id: 6415146,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/Hack.tar.xz",
            sha256: "cdd389472e10e2261520140ff1b382b4f8a226af5fd0b2735b975d31151d9c3c",
            bytes: 2_781_192,
            take: &["NerdFontMono-"],
            skip: &[],
            family: "Hack Nerd Font Mono",
            asset_id: 523429471,
        },
        sample: sample!("hack"),
    },
    Font {
        id: "jetbrains-mono",
        name: "JetBrains Mono",
        license: "OFL",
        ligatures: "ligatures",
        note: "",
        repo: "github.com/JetBrains/JetBrainsMono",
        plain: Asset {
            url: "https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip",
            sha256: "6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf",
            bytes: 5_622_857,
            // Not the NL (no ligatures) family, and not the variable font next to the statics.
            take: &["fonts/ttf/JetBrainsMono-"],
            skip: &[],
            family: "JetBrains Mono",
            asset_id: 91681614,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/JetBrainsMono.tar.xz",
            sha256: "04d5e8f903693f9dd13e16f867e994834e681eb3c72c0d337a770dcda09010cf",
            bytes: 7_284_244,
            take: &["JetBrainsMonoNerdFontMono-"],
            skip: &[],
            family: "JetBrainsMono Nerd Font Mono",
            asset_id: 523475208,
        },
        sample: sample!("jetbrains-mono"),
    },
    Font {
        id: "fira-code",
        name: "Fira Code",
        license: "OFL",
        ligatures: "ligatures",
        note: "",
        repo: "github.com/tonsky/FiraCode",
        plain: Asset {
            url: "https://github.com/tonsky/FiraCode/releases/download/6.2/Fira_Code_v6.2.zip",
            sha256: "0949915ba8eb24d89fd93d10a7ff623f42830d7c5ffc3ecbf960e4ecad3e3e79",
            bytes: 2_462_987,
            take: &["ttf/FiraCode-"],
            skip: &["variable"],
            family: "Fira Code",
            asset_id: 51158218,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/FiraCode.tar.xz",
            sha256: "68e3bd6164864b8b514605bc34e3a87ac401c8c48682fcce6478c70263340207",
            bytes: 3_061_404,
            take: &["NerdFontMono-"],
            skip: &[],
            family: "FiraCode Nerd Font Mono",
            asset_id: 523432568,
        },
        sample: sample!("fira-code"),
    },
    Font {
        id: "cascadia-code",
        name: "Cascadia Code",
        license: "OFL",
        ligatures: "ligatures",
        note: "",
        repo: "github.com/microsoft/cascadia-code",
        plain: Asset {
            // Microsoft ships one zip with every format; only the static TTFs are installed.
            url: "https://github.com/microsoft/cascadia-code/releases/download/v2407.24/CascadiaCode-2407.24.zip",
            sha256: "e67a68ee3386db63f48b9054bd196ea752bc6a4ebb4df35adce6733da50c8474",
            bytes: 150_454_761,
            take: &["ttf/static/CascadiaCode-"],
            skip: &[],
            family: "Cascadia Code",
            asset_id: 209450187,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/CascadiaCode.tar.xz",
            sha256: "ae598e9401e2846aa3ee364513715de0490b844a4b54f0768991f45f23aa8369",
            bytes: 3_592_604,
            take: &["NerdFontMono-"],
            skip: &[],
            family: "CaskaydiaCove Nerd Font Mono",
            asset_id: 523441239,
        },
        sample: sample!("cascadia-code"),
    },
    Font {
        id: "monaspace",
        name: "Monaspace",
        license: "OFL",
        ligatures: "ligatures, texture healing",
        note: "5 families",
        repo: "github.com/githubnext/monaspace",
        plain: Asset {
            // The static build: the variable one is smaller, but a font loaded without a
            // restart only gets its default instance, so bold would draw regular.
            url: "https://github.com/githubnext/monaspace/releases/download/v1.400/monaspace-static-v1.400.zip",
            sha256: "ab66d71be751495f679727332a3345597943bd4d7beebca03f5cde04bf994de7",
            bytes: 55_607_551,
            take: &["Static Fonts/"],
            skip: &["Wide"],
            family: "Monaspace Neon",
            asset_id: 383198921,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/Monaspace.tar.xz",
            sha256: "7cdb44b7cd3855a231641b3cc4cf38f434d615d12e1c475eb86232868c667dc0",
            bytes: 19_257_096,
            take: &["NerdFontMono-"],
            skip: &[],
            family: "MonaspiceNe Nerd Font Mono",
            asset_id: 523480809,
        },
        sample: sample!("monaspace"),
    },
    Font {
        id: "commit-mono",
        name: "Commit Mono",
        license: "OFL",
        ligatures: "no ligatures",
        note: "",
        repo: "github.com/eigilnikolajsen/commit-mono",
        plain: Asset {
            url: "https://github.com/eigilnikolajsen/commit-mono/releases/download/v1.143/CommitMono-1.143.zip",
            sha256: "f7d1f26a7c7554800a996f76f5d706bf0648b936ca2a66b5bc4d46e3a2c49ed0",
            bytes: 724_227,
            // The OTFs; the same four faces are also in ttfautohint/.
            take: &["CommitMono-1.143/CommitMono-"],
            skip: &["ttfautohint"],
            family: "CommitMono",
            asset_id: 142740944,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/CommitMono.tar.xz",
            sha256: "7ee1e16909f0fd05cee30db4299753c33a6134c418c828d6d8523d246ee3f836",
            bytes: 2_712_752,
            take: &["NerdFontMono-"],
            skip: &[],
            family: "CommitMono Nerd Font Mono",
            asset_id: 523429939,
        },
        sample: sample!("commit-mono"),
    },
    Font {
        id: "geist-mono",
        name: "Geist Mono",
        license: "OFL",
        ligatures: "no ligatures",
        note: "",
        repo: "github.com/vercel/geist-font",
        plain: Asset {
            // Vercel's zip also holds Geist (sans) and Geist Pixel; only Geist Mono is taken.
            url: "https://github.com/vercel/geist-font/releases/download/v1.9.2/geist-font-v1.9.2.zip",
            sha256: "15f066d58dcb99fb10014a49913009fdaf41e464cfb8851a44b44e593c95d3d8",
            bytes: 8_837_264,
            take: &["GeistMono/ttf/"],
            skip: &[],
            family: "Geist Mono",
            asset_id: 716741004,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/GeistMono.tar.xz",
            sha256: "d3ba4a7418cde16963fda0caa4f7ae866c509e0a864ec21e393455e1906e070c",
            bytes: 5_268_020,
            take: &["NerdFontMono-"],
            skip: &[],
            family: "GeistMono Nerd Font Mono",
            asset_id: 523447155,
        },
        sample: sample!("geist-mono"),
    },
    Font {
        id: "iosevka",
        name: "Iosevka",
        license: "OFL",
        ligatures: "ligatures",
        note: "narrow",
        repo: "github.com/be5invis/Iosevka",
        plain: Asset {
            // Iosevka has no small single-family zip; this is its main TTF package. The
            // Extended width is left out.
            url: "https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-Iosevka-34.9.1.zip",
            sha256: "7a77a3dfa0e2d713a8920a30624863e23df2867a1f55a5e3d6d23dad07c14c65",
            bytes: 176_951_351,
            take: &[],
            skip: &["Extended"],
            family: "Iosevka",
            asset_id: 672508135,
        },
        icons: Asset {
            url: "https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/Iosevka.tar.xz",
            sha256: "3b94ea1dc3955756762f977b7677bca671947dd56bc755a6f8465a8e83b5f257",
            bytes: 91_681_912,
            take: &["NerdFontMono-"],
            skip: &[],
            family: "Iosevka Nerd Font Mono",
            asset_id: 523505795,
        },
        sample: sample!("iosevka"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_pinned() {
        let mut ids = std::collections::HashSet::new();
        for f in CATALOG.iter() {
            assert!(ids.insert(f.id), "duplicate id {}", f.id);
            assert!(f.repo.starts_with("github.com/"));
            for (a, nerd) in [(&f.plain, false), (&f.icons, true)] {
                assert_eq!(a.sha256.len(), 64, "{}", a.url);
                assert!(a.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{}", a.url);
                assert!(a.bytes > 100_000, "{}", a.url);
                assert!(a.url.starts_with("https://github.com/") && a.url.contains("/releases/download/"), "{}", a.url);
                // A pinned tag, never "latest".
                assert!(!a.url.contains("/latest/"), "{}", a.url);
                assert_eq!(nerd, a.url.starts_with("https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/"), "{}", a.url);
                assert_eq!(nerd, a.family.ends_with("Nerd Font Mono"), "{}", a.family);
            }
            assert!(f.plain.url.contains(&f.repo["github.com".len()..]), "{} comes from {}", f.plain.url, f.repo);
            assert!(f.plain.asset_id > 0 && f.icons.asset_id > 0 && f.plain.asset_id != f.icons.asset_id);
            assert!(f.sample.0.starts_with(b"\x89PNG") && f.sample.1.starts_with(b"\x89PNG"));
        }
        assert_eq!(CATALOG.iter().map(|f| f.name).collect::<Vec<_>>(), ["Hack", "JetBrains Mono", "Fira Code", "Cascadia Code", "Monaspace", "Commit Mono", "Geist Mono", "Iosevka"]);
    }

    #[test]
    fn api_fallback_url() {
        assert_eq!(by_id("hack").unwrap().plain.api_url().as_deref(), Some("https://api.github.com/repos/source-foundry/Hack/releases/assets/6415146"));
        assert_eq!(by_id("hack").unwrap().icons.api_url().as_deref(), Some("https://api.github.com/repos/ryanoasis/nerd-fonts/releases/assets/523429471"));
    }

    #[test]
    fn details_line() {
        let hack = by_id("hack").unwrap();
        assert_eq!(hack.details(false), "MIT · no ligatures · 0.6 MB · Warp's default");
        assert_eq!(hack.details(true), "MIT · no ligatures · 2.8 MB with icons · Warp's default");
        assert_eq!(by_id("fira-code").unwrap().details(false), "OFL · ligatures · 2.5 MB");
        assert_eq!(mb(176_951_351), "177 MB");
        assert_eq!(progress(300_000, 615_776), "0.3 of 0.6 MB");
        assert_eq!(progress(40_000_000, 55_607_551), "40 of 56 MB");
        assert_eq!(progress(9_000_000, 615_776), "0.6 of 0.6 MB");
    }

    #[test]
    fn archive_members_to_install() {
        let f = |id: &str, icons: bool| by_id(id).unwrap().asset(icons);
        assert!(wanted(f("hack", false), "ttf/Hack-Regular.ttf"));
        assert!(!wanted(f("hack", false), "__MACOSX/ttf/._Hack-Regular.ttf"));
        assert!(wanted(f("jetbrains-mono", false), "fonts/ttf/JetBrainsMono-Bold.ttf"));
        assert!(!wanted(f("jetbrains-mono", false), "fonts/ttf/JetBrainsMonoNL-Bold.ttf"));
        assert!(!wanted(f("jetbrains-mono", false), "fonts/variable/JetBrainsMono[wght].ttf"));
        assert!(!wanted(f("jetbrains-mono", false), "fonts/webfonts/JetBrainsMono-Bold.woff2"));
        assert!(wanted(f("fira-code", false), "ttf/FiraCode-Regular.ttf"));
        assert!(!wanted(f("fira-code", false), "variable_ttf/FiraCode-VF.ttf"));
        assert!(wanted(f("cascadia-code", false), "ttf/static/CascadiaCode-Bold.ttf"));
        assert!(!wanted(f("cascadia-code", false), "ttf/CascadiaCode.ttf"));
        assert!(!wanted(f("cascadia-code", false), "ttf/static/CascadiaMonoNF-Bold.ttf"));
        assert!(wanted(f("monaspace", false), "Static Fonts/Monaspace Neon/MonaspaceNeon-Regular.otf"));
        assert!(!wanted(f("monaspace", false), "Static Fonts/Monaspace Neon/MonaspaceNeon-WideBold.otf"));
        assert!(wanted(f("commit-mono", false), "CommitMono-1.143/CommitMono-400-Regular.otf"));
        assert!(!wanted(f("commit-mono", false), "CommitMono-1.143/ttfautohint/CommitMono-400-Regular.ttf"));
        assert!(wanted(f("geist-mono", false), "geist-font/GeistMono/ttf/GeistMono-Regular.ttf"));
        assert!(!wanted(f("geist-mono", false), "geist-font/Geist/ttf/Geist-Regular.ttf"));
        assert!(wanted(f("iosevka", false), "Iosevka-Regular.ttf"));
        assert!(!wanted(f("iosevka", false), "Iosevka-ExtendedBold.ttf"));
        assert!(wanted(f("hack", true), "HackNerdFontMono-Regular.ttf"));
        assert!(!wanted(f("hack", true), "HackNerdFont-Regular.ttf"));
        assert!(!wanted(f("hack", true), "HackNerdFontPropo-Regular.ttf"));
        assert!(!wanted(f("hack", true), "LICENSE.md"));
    }

    #[test]
    fn other_apps_font_names_find_the_catalog() {
        let id = |n: &str| find(n).map(|(f, icons)| (f.id, icons));
        assert_eq!(id("Hack"), Some(("hack", false)));
        assert_eq!(id("Hack-Regular"), Some(("hack", false)));
        assert_eq!(id("JetBrainsMono-Regular"), Some(("jetbrains-mono", false)));
        assert_eq!(id("Hack Nerd Font Mono"), Some(("hack", true)));
        assert_eq!(id("Hack Nerd Font"), Some(("hack", true)));
        assert_eq!(id("FiraCodeNerdFontMono-Regular"), Some(("fira-code", true)));
        assert_eq!(id("Menlo"), None);
        assert!(by_id("hack").unwrap().installed(&["hack nerd font mono".into()]));
        assert!(!by_id("hack").unwrap().installed(&["Hackathon".into()]));
    }

    #[test]
    fn checksums() {
        // SHA-256("abc"), FIPS 180-2.
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(&b"abc"[..]).unwrap(), abc);
        let dir = std::env::temp_dir().join(format!("t1k-sum-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.bin");
        std::fs::write(&p, b"abc").unwrap();
        assert!(checksum_matches(&p, abc));
        assert!(checksum_matches(&p, &abc.to_uppercase()));
        std::fs::write(&p, b"abd").unwrap();
        assert!(!checksum_matches(&p, abc));
        assert!(!checksum_matches(&dir.join("missing"), abc));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copying_never_replaces() {
        let dir = std::env::temp_dir().join(format!("t1k-copy-{}", std::process::id()));
        let (src, dest) = (dir.join("src"), dir.join("Fonts"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("A.ttf"), b"new").unwrap();
        std::fs::write(src.join("B.ttf"), b"new").unwrap();
        std::fs::write(dest.join("B.ttf"), b"mine").unwrap();
        let (w, k) = copy_new(&[src.join("A.ttf"), src.join("B.ttf")], &dest).unwrap();
        assert_eq!(w, vec![dest.join("A.ttf")]);
        assert_eq!(k, vec![dest.join("B.ttf")]);
        assert_eq!(std::fs::read(dest.join("B.ttf")).unwrap(), b"mine");
        assert_eq!(std::fs::read(dest.join("A.ttf")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn curl_failures_read_plainly() {
        assert_eq!(curl_reason(Some(6)), NO_RESPONSE);
        assert_eq!(curl_reason(Some(28)), NO_RESPONSE);
        assert_eq!(curl_reason(None), NO_RESPONSE);
        assert!(curl_reason(Some(22)).starts_with("Couldn't download"));
    }
}
