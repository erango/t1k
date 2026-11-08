//! "Report a bug" (canvas v3-ai, option A): one sheet that fills in what a maintainer needs
//! (versions, the Mac, settings that differ from the defaults, recent warnings and errors)
//! and opens a prefilled GitHub issue. Nothing is sent from T1K; the person still reviews
//! and submits it on GitHub.
//!
//! Everything is gathered locally. Terminal output, tab titles and folder names are never
//! included; log lines and settings go through the same scrub as Sentry reports.

use std::collections::BTreeMap;
use std::io::Read;
use std::ops::Range;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, AnyElement, App, ClipboardItem, Context, FontWeight, HighlightStyle, Hsla, KeyDownEvent, MouseButton,
    ScrollHandle, SharedString, StyledText, Window,
};
use t1k_agent::agents::AgentKind;

use crate::input::{self, Key, Sel};
use crate::overlay::Overlay;
use crate::telemetry;
use crate::workspace::Workspace;

/// The Details box until it's edited; each question is followed by an empty line to answer on.
const DETAILS_PROMPT: &str = "What did you do?\n\nWhat did you expect?\n\nWhat happened instead?\n";
/// GitHub turns away much longer issue links (around 8 KB); past this the report goes by
/// clipboard instead.
pub const MAX_URL: usize = 7_500;
/// Log lines in a report (the newest; the ring buffer keeps more).
const LOG_LINES: usize = 20;
const PASTE_BODY: &str = "Paste the report from your clipboard here.";
const CARET: &str = "▏";
/// The sheet's distance from the window's top (and the least room left below it).
const TOP: f32 = 48.;
/// `<cli> --version` can hang on a broken install; a report shouldn't wait on it.
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Title,
    Details,
}

/// What the report says about this Mac and T1K. `macos`, `chip`, the shell's version and the
/// agents arrive a moment after the sheet opens (they run programs).
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Env {
    pub version: String,
    pub commit: String,
    pub channel: String,
    /// "26.1 (25B78)"
    pub macos: Option<String>,
    pub chip: Option<String>,
    pub displays: Vec<String>,
    /// "zsh 5.9"
    pub shell: String,
    pub gravity: String,
    pub integration: bool,
    /// None while looking; "Claude Code 2.0.37" per connected agent.
    pub agents: Option<Vec<String>>,
    pub theme: String,
    pub font: String,
    pub size: String,
    pub tabs: usize,
    pub groups: usize,
    pub splits: usize,
    pub crash: Option<String>,
}

/// The parts of `Env` that take a program run to learn.
struct Slow {
    macos: Option<String>,
    chip: Option<String>,
    shell: Option<String>,
    agents: Vec<String>,
}

/// Where a report started, when it wasn't the menu: its title, and the error that goes first
/// in the log list.
#[derive(Clone, Debug)]
pub struct Prefill {
    pub title: String,
    /// Module-ish source for the log line ("codex", "updater").
    pub source: String,
    pub error: String,
}

pub struct Sheet {
    pub title: String,
    title_sel: Sel,
    details: String,
    details_sel: Sel,
    /// The prompt stays tertiary until the first edit.
    edited: bool,
    pub field: Field,
    pub expanded: bool,
    with_settings: bool,
    with_logs: bool,
    pub env: Env,
    settings: Vec<String>,
    logs: Vec<String>,
    /// Which Copy was pressed, for a moment of "Copied".
    copied: Option<(&'static str, Instant)>,
    details_scroll: ScrollHandle,
}

impl Sheet {
    fn new(env: Env, settings: Vec<String>, logs: Vec<String>, title: String) -> Self {
        // The caret waits on the empty line after the first question.
        let first = DETAILS_PROMPT.find('\n').map_or(0, |i| i + 1);
        Sheet {
            title_sel: Sel { anchor: title.len(), cursor: title.len() },
            title,
            details: DETAILS_PROMPT.into(),
            details_sel: Sel { anchor: first, cursor: first },
            edited: false,
            field: Field::Title,
            expanded: false,
            with_settings: true,
            with_logs: true,
            env,
            settings,
            logs,
            copied: None,
            details_scroll: ScrollHandle::new(),
        }
    }

    /// The exact text attached under "What happened": what the expanded block shows.
    pub fn attached(&self) -> String {
        attached(&self.env, self.with_settings.then_some(&self.settings[..]), self.with_logs.then_some(&self.logs[..]))
    }

    pub fn body(&self) -> String {
        body(&self.details, &self.attached())
    }

    fn url(&self) -> String {
        issue_url(self.title.trim(), &self.body())
    }

    /// "T1K 0.9.2 · macOS 26.1 · 2 agents"
    fn summary(&self) -> String {
        let mut s = format!("T1K {}", self.env.version);
        if let Some(m) = &self.env.macos {
            s.push_str(&format!(" · macOS {}", m.split(' ').next().unwrap_or(m)));
        }
        if let Some(a) = &self.env.agents {
            s.push_str(&format!(" · {}", plural(a.len(), "agent", "agents")));
        }
        s
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---- the report text -------------------------------------------------------------------------

fn env_lines(e: &Env) -> Vec<String> {
    let mut v = vec![
        format!("- T1K {} ({}, {})", e.version, e.commit, e.channel),
        format!("- macOS {}{}", e.macos.as_deref().unwrap_or("…"), e.chip.as_ref().map(|c| format!(", {c}")).unwrap_or_default()),
        format!("- Displays: {}", if e.displays.is_empty() { "unknown".to_string() } else { e.displays.join(", ") }),
        format!("- Shell: {} · prompt at {} · shell integration {}", e.shell, e.gravity, if e.integration { "on" } else { "off" }),
        format!(
            "- Agents: {}",
            match &e.agents {
                None => "…".to_string(),
                Some(a) if a.is_empty() => "none connected".to_string(),
                Some(a) => a.join(", "),
            }
        ),
        format!("- Look: {} · {} {}", e.theme, e.font, e.size),
        format!("- Workspace: {}, {}, {}", plural(e.tabs, "tab", "tabs"), plural(e.groups, "group", "groups"), plural(e.splits, "split", "splits")),
    ];
    if let Some(id) = &e.crash {
        v.push(format!("- Crash report: {id} (Sentry)"));
    }
    v
}

/// Everything after "What happened". A `<details>` section is left out when it's off (None)
/// or empty.
pub fn attached(env: &Env, settings: Option<&[String]>, logs: Option<&[String]>) -> String {
    let mut s = String::from("### Environment\n");
    for l in env_lines(env) {
        s.push_str(&l);
        s.push('\n');
    }
    let mut section = |title: &str, lang: &str, lines: &[String]| {
        s.push_str(&format!("\n<details><summary>{title} ({})</summary>\n\n```{lang}\n", lines.len()));
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        s.push_str("```\n</details>\n");
    };
    if let Some(lines) = settings.filter(|l| !l.is_empty()) {
        section("Settings I changed", "toml", lines);
    }
    if let Some(lines) = logs.filter(|l| !l.is_empty()) {
        section("Recent warnings and errors", "", lines);
    }
    s
}

pub fn body(description: &str, attached: &str) -> String {
    format!("### What happened\n{}\n\n{attached}", description.trim_end())
}

/// Percent-encode everything but RFC 3986's unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn issue_url(title: &str, body: &str) -> String {
    format!("{}/issues/new?title={}&body={}&labels=bug", crate::brand::REPO, encode(title), encode(body))
}

/// Whether the report has to go by clipboard (the link would be too long for GitHub).
pub fn too_long(title: &str, body: &str) -> bool {
    issue_url(title, body).len() > MAX_URL
}

fn flatten(prefix: &str, t: &toml::Table, out: &mut BTreeMap<String, toml::Value>) {
    for (k, v) in t {
        let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
        match v {
            toml::Value::Table(t) => flatten(&key, t, out),
            v => {
                out.insert(key, v.clone());
            }
        }
    }
}

fn same(a: &toml::Value, b: &toml::Value) -> bool {
    use toml::Value::{Float, Integer};
    match (a, b) {
        // `size = 14` and `size = 14.0` are the same setting.
        (Integer(i), Float(f)) | (Float(f), Integer(i)) => (*i as f64 - f).abs() < 1e-9,
        _ => a == b,
    }
}

/// `key = value` for each key in `user` that differs from `defaults` (or that `defaults`
/// doesn't have, like `[keys]` overrides), scrubbed. A file that doesn't parse says so.
pub fn changed_settings(user: &str, defaults: &str) -> Vec<String> {
    let Ok(user) = user.parse::<toml::Table>() else {
        return vec!["# config.toml doesn't parse".into()];
    };
    let mut have = BTreeMap::new();
    flatten("", &user, &mut have);
    let mut base = BTreeMap::new();
    if let Ok(d) = defaults.parse::<toml::Table>() {
        flatten("", &d, &mut base);
    }
    have.into_iter()
        .filter(|(k, v)| base.get(k).is_none_or(|d| !same(d, v)))
        .map(|(k, v)| telemetry::scrub(&format!("{k} = {v}")).into_owned())
        .collect()
}

// ---- gathering -------------------------------------------------------------------------------

fn which(bin: &str) -> Option<PathBuf> {
    crate::platform::tool_path().split(':').map(|d| std::path::Path::new(d).join(bin)).find(|p| p.is_file())
}

/// A program's trimmed stdout, or None if it fails or outlives `timeout`.
fn run(program: &std::path::Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("PATH", crate::platform::tool_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out.trim().to_string()).filter(|s| !s.is_empty())
}

/// The first version-looking token: "2.0.37 (Claude Code)" → "2.0.37", "codex-cli 0.58.0" →
/// "0.58.0", "GNU bash, version 3.2.57(1)-release" → "3.2.57".
pub fn version_in(s: &str) -> Option<String> {
    s.split_whitespace().find_map(|w| {
        let w = w.strip_prefix('v').unwrap_or(w);
        let v: String = w.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let v = v.trim_end_matches('.');
        (v.chars().next().is_some_and(|c| c.is_ascii_digit())).then(|| v.to_string())
    })
}

fn gather_slow(shell: Option<PathBuf>) -> Slow {
    let sw = |arg: &str| run(std::path::Path::new("/usr/bin/sw_vers"), &[arg], VERSION_TIMEOUT);
    // The agents' CLIs start slowly (node); ask them all at once.
    let agents = std::thread::scope(|s| {
        let handles: Vec<_> = AgentKind::ALL
            .into_iter()
            .filter(|k| crate::agent_setup::installed(*k))
            .map(|k| {
                s.spawn(move || {
                    let v = which(k.program()).and_then(|p| run(&p, &["--version"], VERSION_TIMEOUT)).and_then(|o| version_in(&o));
                    match v {
                        Some(v) => format!("{} {v}", k.label()),
                        None => k.label().to_string(),
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    Slow {
        macos: sw("-productVersion").map(|v| match sw("-buildVersion") {
            Some(b) => format!("{v} ({b})"),
            None => v,
        }),
        chip: run(std::path::Path::new("/usr/sbin/sysctl"), &["-n", "machdep.cpu.brand_string"], VERSION_TIMEOUT),
        shell: shell.and_then(|p| run(&p, &["--version"], VERSION_TIMEOUT)).and_then(|o| version_in(&o)),
        agents,
    }
}

fn shell_path() -> Option<PathBuf> {
    std::env::var_os("SHELL").map(PathBuf::from).filter(|p| p.is_absolute())
}

impl Workspace {
    /// The quick parts of the environment, read on the spot.
    fn bug_env(&self, window: &Window, cx: &App) -> Env {
        let c = &self.config;
        let here = window.display(cx).map(|d| d.id());
        let displays = cx
            .displays()
            .iter()
            .map(|d| {
                let s = d.bounds().size;
                let mut t = format!("{}×{} pt", f32::from(s.width).round(), f32::from(s.height).round());
                // GPUI knows the scale only for the display this window is on.
                if Some(d.id()) == here {
                    t.push_str(&format!(" @{}x", window.scale_factor()));
                }
                t
            })
            .collect();
        Env {
            version: env!("CARGO_PKG_VERSION").into(),
            commit: env!("T1K_COMMIT").into(),
            channel: c.updates.channel.clone(),
            macos: None,
            chip: None,
            displays,
            shell: shell_path().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "unknown".into()),
            gravity: c.terminal.prompt_gravity.clone(),
            integration: c.terminal.shell_integration,
            agents: None,
            theme: self.theme.name.to_string(),
            font: c.terminal.font.family.clone(),
            size: format!("{}", c.terminal.font.size),
            tabs: self.tabs.len(),
            groups: self.grouped().len(),
            splits: self.tabs.iter().filter(|t| t.split.is_some()).count(),
            crash: telemetry::last_event_id(),
        }
    }

    /// Open the sheet: from the menu or palette (`prefill` None), or from an error ("Report
    /// this…"), which names it in the title and puts it first in the log list.
    pub fn open_report_bug(&mut self, prefill: Option<Prefill>, window: &mut Window, cx: &mut Context<Self>) {
        let env = self.bug_env(window, cx);
        let settings = crate::config::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| changed_settings(&t, crate::config::template()))
            .unwrap_or_default();
        let mut logs = telemetry::recent();
        let mut title = String::new();
        if let Some(p) = prefill {
            title = telemetry::scrub(&p.title).into_owned();
            let keep = logs.len().saturating_sub(LOG_LINES - 1);
            logs.drain(..keep);
            logs.insert(0, telemetry::log_line(&telemetry::clock(), "ERROR", &p.source, &p.error));
        } else {
            let keep = logs.len().saturating_sub(LOG_LINES);
            logs.drain(..keep);
        }
        self.open_overlay(Overlay::ReportBug(Box::new(Sheet::new(env, settings, logs, title))), window, cx);

        let shell = shell_path();
        let task = cx.background_executor().spawn(async move { gather_slow(shell) });
        cx.spawn(async move |this, cx| {
            let slow = task.await;
            let _ = this.update(cx, |ws, cx| {
                if let Overlay::ReportBug(s) = &mut ws.overlay {
                    s.env.macos = slow.macos;
                    s.env.chip = slow.chip;
                    if let Some(v) = slow.shell {
                        s.env.shell = format!("{} {v}", s.env.shell);
                    }
                    s.env.agents = Some(slow.agents);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn report_sheet(&mut self) -> Option<&mut Sheet> {
        match &mut self.overlay {
            Overlay::ReportBug(s) => Some(s),
            _ => None,
        }
    }

    pub fn report_bug_key(&mut self, mut s: Box<Sheet>, ev: &KeyDownEvent, cx: &mut Context<Self>) -> Option<Overlay> {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "escape" => return None,
            "enter" if ks.modifiers.platform => {
                self.submit_report(&s, cx);
                return None;
            }
            "tab" => {
                s.field = if s.field == Field::Title { Field::Details } else { Field::Title };
                return Some(Overlay::ReportBug(s));
            }
            _ => {}
        }
        match s.field {
            Field::Title => {
                if let Key::Submit = input::edit(&mut s.title, &mut s.title_sel, ev, cx) {
                    s.field = Field::Details;
                }
            }
            Field::Details => {
                let sheet = &mut *s;
                if edit_lines(&mut sheet.details, &mut sheet.details_sel, ev, cx) {
                    sheet.edited = true;
                    // Typing at the end keeps the end in view once the box scrolls.
                    if !sheet.details[sheet.details_sel.cursor..].contains('\n') {
                        sheet.details_scroll.scroll_to_bottom();
                    }
                }
            }
        }
        Some(Overlay::ReportBug(s))
    }

    /// Open the prefilled issue; a report too long for a link goes by clipboard.
    fn submit_report(&mut self, s: &Sheet, cx: &mut Context<Self>) {
        let title = s.title.trim();
        let body = s.body();
        if too_long(title, &body) {
            cx.write_to_clipboard(ClipboardItem::new_string(body));
            cx.open_url(&issue_url(title, PASTE_BODY));
            self.notice_brief = false;
            self.notice_undo = false;
            // Notices last 5 s; this one 4 (canvas v3-ai).
            let at = Instant::now().checked_sub(Duration::from_secs(1)).unwrap_or_else(Instant::now);
            self.notice = Some(("Copied. Paste it into the issue on GitHub.".into(), at));
        } else {
            cx.open_url(&issue_url(title, &body));
        }
    }

    fn report_submit_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Overlay::ReportBug(s) = std::mem::replace(&mut self.overlay, Overlay::None) {
            self.submit_report(&s, cx);
        }
        self.close_overlay(window, cx);
    }

    fn report_copy(&mut self, which: &'static str, cx: &mut Context<Self>) {
        let Some(s) = self.report_sheet() else { return };
        let text = if which == "copy-report" {
            format!("# {}\n\n{}", s.title.trim(), s.body())
        } else {
            s.attached()
        };
        s.copied = Some((which, Instant::now()));
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1600)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }
}

/// T1K_DEBUG_OVERLAY=bug|bug-open|bug-long: the sheet with a sample report; `bug-open` with
/// the attached details expanded, `bug-long` too long for a link.
pub fn debug_open(ws: &mut Workspace, which: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    log::warn!("sample warning for the bug report, from {}", std::env::var("HOME").unwrap_or_default());
    ws.open_report_bug(None, window, cx);
    if let Some(s) = ws.report_sheet() {
        s.title = "Sidebar rows jump when a Codex tab finishes".into();
        s.title_sel = Sel { anchor: s.title.len(), cursor: s.title.len() };
        s.details = "What did you do?\nOpened 3 Codex tabs and waited.\n\nWhat did you expect?\nRows to stay put.\n\nWhat happened instead?\nThe list jumped by one row each time a tab finished.".into();
        if which == "bug-long" {
            s.details.push_str(&"\n\nThe rows moved again after the next turn.".repeat(200));
        }
        s.details_sel = Sel { anchor: s.details.len(), cursor: s.details.len() };
        s.edited = true;
        s.expanded = which == "bug-open";
    }
}

// ---- multi-line editing ----------------------------------------------------------------------

fn line_start(text: &str, i: usize) -> usize {
    text[..i].rfind('\n').map_or(0, |j| j + 1)
}

fn line_end(text: &str, i: usize) -> usize {
    text[i..].find('\n').map_or(text.len(), |j| i + j)
}

/// The byte `col` characters into the line `from..to` (its end if shorter).
fn column(text: &str, from: usize, to: usize, col: usize) -> usize {
    text[from..to].char_indices().nth(col).map_or(to, |(j, _)| from + j)
}

/// ↑ / ↓ by logical line, keeping the column.
pub fn vertical(text: &str, i: usize, up: bool) -> usize {
    let start = line_start(text, i);
    let col = text[start..i].chars().count();
    if up {
        if start == 0 {
            return 0;
        }
        let prev = line_start(text, start - 1);
        column(text, prev, start - 1, col)
    } else {
        let end = line_end(text, i);
        if end == text.len() {
            return text.len();
        }
        column(text, end + 1, line_end(text, end + 1), col)
    }
}

fn insert(text: &mut String, sel: &mut Sel, s: &str) {
    let r = sel.range();
    text.replace_range(r.clone(), s);
    let at = r.start + s.len();
    *sel = Sel { anchor: at, cursor: at };
}

/// The Details box: the single-line field's keys (`input::edit`), plus ↵ for a new line, ⌘V
/// keeping line breaks, ⌘⌫ and ⌘← / ⌘→ by line, and ↑ / ↓ between lines. True if the text
/// changed.
pub fn edit_lines(text: &mut String, sel: &mut Sel, ev: &KeyDownEvent, cx: &mut App) -> bool {
    let ks = &ev.keystroke;
    let m = &ks.modifiers;
    sel.anchor = sel.anchor.min(text.len());
    sel.cursor = sel.cursor.min(text.len());
    let at = sel.cursor;
    let go = |sel: &mut Sel, to: usize| {
        sel.cursor = to;
        if !m.shift {
            sel.anchor = to;
        }
        false
    };
    match ks.key.as_str() {
        "enter" => {
            insert(text, sel, "\n");
            return true;
        }
        "v" if m.platform => {
            let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) else { return false };
            insert(text, sel, &t.replace("\r\n", "\n").replace('\r', "\n"));
            return true;
        }
        "backspace" if m.platform && sel.range().is_empty() => {
            // To the start of the line; at the start, join it with the line above.
            let start = line_start(text, at);
            sel.anchor = if start == at { at.saturating_sub(1) } else { start };
            insert(text, sel, "");
            return true;
        }
        "left" if m.platform => return go(sel, line_start(text, at)),
        "right" if m.platform => return go(sel, line_end(text, at)),
        "up" | "down" => return go(sel, vertical(text, at, ks.key == "up")),
        _ => {}
    }
    matches!(input::edit(text, sel, ev, cx), Key::Edited)
}

// ---- rendering -------------------------------------------------------------------------------

/// Text with its selection highlighted, or a caret where the cursor is (`focused` only).
fn with_caret(text: &str, sel: Sel, focused: bool, accent: Hsla) -> StyledText {
    let r = sel.range();
    let r = r.start.min(text.len())..r.end.min(text.len());
    if !focused {
        return StyledText::new(text.to_string());
    }
    if r.is_empty() {
        let mut d = text.to_string();
        d.insert_str(r.start, CARET);
        let hl: Vec<(Range<usize>, HighlightStyle)> = vec![(r.start..r.start + CARET.len(), HighlightStyle { color: Some(accent), ..Default::default() })];
        StyledText::new(d).with_highlights(hl)
    } else {
        StyledText::new(text.to_string()).with_highlights(vec![(r, HighlightStyle { background_color: Some(accent.opacity(0.3)), ..Default::default() })])
    }
}

fn label(ws: &Workspace, text: &'static str) -> gpui::Div {
    div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(ws.ui.text_secondary).child(text)
}

fn field_box(ws: &Workspace, id: &'static str, focused: bool) -> gpui::Stateful<gpui::Div> {
    let ui = ws.ui;
    div()
        .id(id)
        .px(px(10.))
        .rounded(px(7.))
        .bg(ui.bg_terminal)
        .border_1()
        .border_color(if focused { ui.accent.opacity(0.5) } else { ui.border })
        .text_size(px(13.))
        .line_height(px(19.))
        .text_color(ui.text_primary)
        .cursor(gpui::CursorStyle::IBeam)
}

fn button(ws: &Workspace, id: &'static str, label: impl Into<SharedString>, primary: bool) -> gpui::Stateful<gpui::Div> {
    let ui = ws.ui;
    div()
        .id(id)
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(7.))
        .text_size(px(13.))
        .whitespace_nowrap()
        .map(|d| if primary { d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::MEDIUM) } else { d.bg(ui.bg_control).text_color(ui.text_primary) })
        .hover(|d| d.opacity(0.9))
        .child(label.into())
}

fn checkbox_row(ws: &Workspace, id: &'static str, text: &'static str, n: usize, on: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.5))
        .text_color(ui.text_primary)
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(s) = this.report_sheet() {
                if id == "bug-settings" {
                    s.with_settings = !s.with_settings;
                } else {
                    s.with_logs = !s.with_logs;
                }
                cx.notify();
            }
        }))
        .child(crate::import_ui::checkbox(ui, Some(on)))
        .child(text)
        .child(div().text_color(ui.text_tertiary).child(format!("({n})")))
        .into_any_element()
}

pub fn sheet(ws: &Workspace, s: &Sheet, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let copied = |which: &str| s.copied.is_some_and(|(w, at)| w == which && at.elapsed() < Duration::from_millis(1500));
    let long = s.url().len() > MAX_URL;

    let title = field_box(ws, "bug-title", s.field == Field::Title)
        .h(px(32.))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            if let Some(s) = this.report_sheet() {
                s.field = Field::Title;
                cx.notify();
            }
        }))
        .child(with_caret(&s.title, s.title_sel, s.field == Field::Title, ui.accent));

    let details = field_box(ws, "bug-details", s.field == Field::Details)
        .py(px(8.))
        .min_h(px(150.))
        .max_h(px(260.))
        .overflow_y_scroll()
        .track_scroll(&s.details_scroll)
        .when(!s.edited, |d| d.text_color(ui.text_tertiary))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            if let Some(s) = this.report_sheet() {
                s.field = Field::Details;
                cx.notify();
            }
        }))
        .child(with_caret(&s.details, s.details_sel, s.field == Field::Details, ui.accent));

    let attached = s.attached();
    let header = div()
        .h(px(34.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .id("bug-expand")
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(s) = this.report_sheet() {
                        s.expanded = !s.expanded;
                        cx.notify();
                    }
                }))
                .child(div().w(px(10.)).text_size(px(10.)).text_color(ui.text_tertiary).child(if s.expanded { "▾" } else { "▸" }))
                .child(div().flex_none().text_size(px(13.)).text_color(ui.text_primary).child("Attached details"))
                .child(div().min_w_0().truncate().text_size(px(12.)).text_color(ui.text_tertiary).child(s.summary())),
        )
        .child(
            div()
                .id("bug-copy-attached")
                .flex_none()
                .text_size(px(12.))
                .text_color(ui.accent)
                .cursor_pointer()
                .hover(|d| d.underline())
                .on_click(cx.listener(|this, _, _, cx| this.report_copy("copy-attached", cx)))
                .child(if copied("copy-attached") { "Copied" } else { "Copy" }),
        );
    let preview = s.expanded.then(|| {
        div()
            .id("bug-preview")
            .mx(px(12.))
            .mb(px(10.))
            .px(px(10.))
            .py(px(8.))
            .max_h(px(190.))
            .overflow_y_scroll()
            .rounded(px(6.))
            .bg(ui.bg_terminal)
            .border_1()
            .border_color(ui.border)
            .font_family("JetBrains Mono")
            .text_size(px(11.))
            .line_height(px(16.))
            .text_color(ui.text_tertiary)
            .children(attached.lines().map(|l| {
                let head = l.starts_with("###") || l.starts_with("<details>");
                div().min_h(px(16.)).when(head, |d| d.text_color(ui.text_secondary)).child(l.to_string())
            }))
    });
    let block = div()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(ui.border)
        .child(header)
        .children(preview)
        .child(
            div()
                .px(px(12.))
                .pb(px(10.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(checkbox_row(ws, "bug-settings", "Settings I changed", s.settings.len(), s.with_settings, cx))
                .child(checkbox_row(ws, "bug-logs", "Recent warnings and errors", s.logs.len(), s.with_logs, cx)),
        );

    let primary = button(ws, "bug-open", if long { "Copy and open GitHub" } else { "Open on GitHub" }, true)
        .child(div().font_family("JetBrains Mono").text_size(px(11.)).opacity(0.7).child("↵"))
        .on_click(cx.listener(|this, _, window, cx| this.report_submit_clicked(window, cx)));

    let head = div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Report a bug"))
        .child(div().text_size(px(12.5)).text_color(ui.text_tertiary).child("Opens a new issue on github.com/erango/t1k with this filled in."));
    // In a short window the middle scrolls; the title and the buttons stay put.
    let middle = div()
        .id("bug-middle")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(label(ws, "What went wrong?"))
                .child(title)
                .child(label(ws, "Details"))
                .child(details)
                .child(block),
        );
    let foot = div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(12.))
        .when(long, |d| {
            d.child(
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(7.))
                    .bg(ui.accent.opacity(0.08))
                    .border_1()
                    .border_color(ui.accent.opacity(0.25))
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(ui.text_secondary)
                    .child("This report is too long for a GitHub link. T1K will copy it, and you paste it into the issue."),
            )
        })
        .child(
            div()
                .text_size(px(11.5))
                .line_height(px(16.))
                .text_color(ui.text_tertiary)
                .child("Nothing is sent until you submit on GitHub. Terminal output, tab titles and folder names are never included."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    button(ws, "bug-copy-report", if copied("copy-report") { "Copied" } else { "Copy report" }, false)
                        .on_click(cx.listener(|this, _, _, cx| this.report_copy("copy-report", cx))),
                )
                .child(div().flex_1())
                .child(button(ws, "bug-cancel", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
                .child(primary),
        );
    div()
        .w(px(560.))
        .mt(px(TOP))
        .max_h(window.viewport_size().height - px(TOP * 2.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(20.))
        .rounded(px(14.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(head)
        .child(middle)
        .child(foot)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            version: "0.9.2".into(),
            commit: "a41c2e9".into(),
            channel: "stable".into(),
            macos: Some("26.1 (25B78)".into()),
            chip: Some("Apple M3 Pro".into()),
            displays: vec!["1512×982 pt @2x".into()],
            shell: "zsh 5.9".into(),
            gravity: "bottom".into(),
            integration: true,
            agents: Some(vec!["Claude Code 2.0.37".into()]),
            theme: "Mercury Dark".into(),
            font: "JetBrains Mono".into(),
            size: "13".into(),
            tabs: 14,
            groups: 4,
            splits: 1,
            crash: None,
        }
    }

    #[test]
    fn body_follows_the_template() {
        let settings = vec!["notify.done = true".to_string()];
        let logs = vec!["14:02:40 ERROR hooks: Codex hook timed out after 2s".to_string()];
        let b = body("It broke.\n\n", &attached(&env(), Some(&settings), Some(&logs)));
        assert!(b.starts_with("### What happened\nIt broke.\n\n### Environment\n- T1K 0.9.2 (a41c2e9, stable)\n- macOS 26.1 (25B78), Apple M3 Pro\n"));
        assert!(b.contains("- Shell: zsh 5.9 · prompt at bottom · shell integration on\n"));
        assert!(b.contains("- Workspace: 14 tabs, 4 groups, 1 split\n"));
        assert!(!b.contains("Crash report"));
        assert!(b.contains("<details><summary>Settings I changed (1)</summary>\n\n```toml\nnotify.done = true\n```\n</details>\n"));
        assert!(b.contains("<details><summary>Recent warnings and errors (1)</summary>\n\n```\n14:02:40 ERROR"));
    }

    #[test]
    fn sections_drop_out_when_off_or_empty() {
        let mut e = env();
        e.agents = Some(Vec::new());
        e.crash = Some("3f2c9a1e".into());
        let a = attached(&e, None, Some(&[]));
        assert!(!a.contains("<details>"));
        assert!(a.contains("- Agents: none connected\n"));
        assert!(a.contains("- Crash report: 3f2c9a1e (Sentry)\n"));
    }

    #[test]
    fn long_reports_switch_to_the_clipboard() {
        let short = body("x", &attached(&env(), None, None));
        assert!(!too_long("Rows jump", &short));
        let logs: Vec<String> = (0..20).map(|i| format!("14:02:{i:02} WARN  pty: {}", "resize ignored ".repeat(20))).collect();
        let long = body("x", &attached(&env(), None, Some(&logs)));
        assert!(too_long("Rows jump", &long));
        // The fallback link always fits.
        assert!(!too_long("Rows jump", PASTE_BODY));
        let url = issue_url("a b&c", "x\ny");
        assert_eq!(url, format!("{}/issues/new?title=a%20b%26c&body=x%0Ay&labels=bug", crate::brand::REPO));
    }

    #[test]
    fn settings_diff_lists_only_changes() {
        let defaults = "appearance = \"system\"\n[terminal.font]\nsize = 13\nfamily = \"JetBrains Mono\"\n[notify]\ndone = false\n";
        let user = "appearance = \"system\"\n[terminal.font]\nsize = 14.0\nfamily = \"JetBrains Mono\"\n[notify]\ndone = true\n[keys]\nfind_tab = \"cmd-e\"\n";
        assert_eq!(changed_settings(user, defaults), vec!["keys.find_tab = \"cmd-e\"", "notify.done = true", "terminal.font.size = 14.0"]);
        assert!(changed_settings("[terminal.font]\nsize = 13.0\n", defaults).is_empty());
        assert!(changed_settings(crate::config::template(), crate::config::template()).is_empty());
        assert_eq!(changed_settings("not toml [", defaults), vec!["# config.toml doesn't parse"]);
    }

    #[test]
    fn settings_values_are_scrubbed() {
        std::env::set_var("HOME", "/Users/dana");
        let out = changed_settings("[links]\neditor = \"/Users/dana/bin/ed {path}\"\n", "[links]\neditor = \"\"\n");
        assert_eq!(out, vec!["links.editor = \"~/bin/ed {path}\""]);
    }

    #[test]
    fn versions() {
        assert_eq!(version_in("2.0.37 (Claude Code)").as_deref(), Some("2.0.37"));
        assert_eq!(version_in("codex-cli 0.58.0").as_deref(), Some("0.58.0"));
        assert_eq!(version_in("zsh 5.9 (arm64-apple-darwin24.0)").as_deref(), Some("5.9"));
        assert_eq!(version_in("GNU bash, version 3.2.57(1)-release").as_deref(), Some("3.2.57"));
        assert_eq!(version_in("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(version_in("no version here"), None);
    }

    #[test]
    fn lines_move_by_column() {
        let t = "abc\nde\nfghij";
        assert_eq!(vertical(t, 2, false), 6); // col 2 → end of "de"
        assert_eq!(vertical(t, 6, false), 9);
        assert_eq!(vertical(t, 9, true), 6);
        assert_eq!(vertical(t, 1, true), 0);
        assert_eq!(vertical(t, 9, false), t.len());
    }
}
