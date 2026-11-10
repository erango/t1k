//! Context details (canvas v3-au): what fills a Claude session's context window, for people
//! who want to look inside. Hidden until asked for: clicking the strip's context meter opens a
//! popover, and its Details opens the Context tab of the right-hand panel.
//!
//! The transcript is read on a thread (`t1k_agent::context::Scan`), the whole file the first
//! time and what was appended after that, only while the popover or the tab is showing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use gpui::{canvas, div, point, prelude::*, px, AnyElement, Context, Div, FontWeight, Hsla, MouseButton, PathBuilder, SharedString};
use t1k_agent::context::{mcp_server, tokens, Report, Scan};
use t1k_agent::AgentState;
use t1k_themes::{Theme, UiColors};

use crate::workspace::Workspace;

const MONO: &str = "JetBrains Mono";

/// Which tab the right-hand panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PanelTab {
    #[default]
    Outputs,
    Context,
}

#[derive(Default)]
pub struct CtxState {
    pub(crate) scans: HashMap<PathBuf, Scan>,
    pub(crate) inflight: Option<(PathBuf, Receiver<Scan>)>,
    /// The last finished read of each transcript (the scan itself is away while it reads).
    pub(crate) reports: HashMap<PathBuf, Report>,
    /// Sections and tool groups flipped from their default open/closed state (for every tab).
    pub toggled: HashSet<String>,
}

impl CtxState {
    pub fn report(&self, path: &std::path::Path) -> Option<&Report> {
        self.reports.get(path)
    }
}

/// How full the context is: (percent, tokens used, window), as the strip's meter shows it.
pub fn fill(a: &AgentState) -> Option<(u32, u64, u64)> {
    let window = a.context_window()?;
    let p = a.context_percent()?;
    // Reported by the agent: the tokens follow from its percent, so the two always agree.
    let used = if a.context_reported.is_some() { window * p as u64 / 100 } else { a.context_tokens? };
    Some((p, used, window))
}

/// Only Claude's transcripts say what's in the context.
pub fn has_details(a: &AgentState) -> bool {
    a.kind == t1k_agent::agents::AgentKind::Claude && a.transcript().is_some()
}

impl Workspace {
    fn context_path(&self) -> Option<PathBuf> {
        if let Some(p) = &self.ctx_debug {
            return Some(p.clone());
        }
        let a = self.tabs.get(self.active)?.agent.as_ref()?;
        has_details(a).then(|| a.transcript().map(|p| p.to_path_buf())).flatten()
    }

    fn context_showing(&self) -> bool {
        matches!(self.overlay, crate::overlay::Overlay::Context { .. })
            || (self.panel.open && self.panel.tab == PanelTab::Context && self.viewer.is_none())
    }

    /// Called every poll: starts a read of the active tab's transcript while the details show,
    /// and takes in a finished one.
    pub fn context_tick(&mut self, cx: &mut Context<Self>) {
        if let Some((path, rx)) = &self.ctx.inflight {
            match rx.try_recv() {
                Ok(scan) => {
                    if self.ctx.reports.get(path) != Some(&scan.report) {
                        self.ctx.reports.insert(path.clone(), scan.report.clone());
                        cx.notify();
                    }
                    self.ctx.scans.insert(path.clone(), scan);
                    self.ctx.inflight = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(_) => self.ctx.inflight = None,
            }
        }
        if !self.context_showing() {
            return;
        }
        let Some(path) = self.context_path() else { return };
        let scan = self.ctx.scans.remove(&path).unwrap_or_else(|| Scan::new(path.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut scan = scan;
            scan.poll();
            let _ = tx.send(scan);
        });
        self.ctx.inflight = Some((path, rx));
    }

    pub fn show_context_tab(&mut self, cx: &mut Context<Self>) {
        self.panel.open = true;
        self.panel.tab = PanelTab::Context;
        self.viewer = None;
        self.save();
        self.context_tick(cx);
        cx.notify();
    }

    fn ctx_open(&self, key: &str, default: bool) -> bool {
        default != self.ctx.toggled.contains(key)
    }
}

// ---------- numbers ----------

fn k(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

/// The window size, "1M" or "200k".
fn win(n: u64) -> String {
    if n >= 1_000_000 && n.is_multiple_of(1_000_000) { format!("{}M", n / 1_000_000) } else if n.is_multiple_of(1000) { format!("{}k", n / 1000) } else { k(n) }
}

fn pct(n: u64, of: u64) -> String {
    if of == 0 { "—".into() } else { format!("{:.1}%", n as f64 * 100. / of as f64) }
}

fn ago(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if s < 60 { "just now".into() } else { format!("{} ago", crate::usage::span(s as i64)) }
}

fn dur(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 { format!("{s}s") } else { crate::usage::span(s as i64) }
}

/// "~/.claude/projects/…/MEMORY.md": the first two segments and the file name.
fn short_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let p = match p.strip_prefix(&home).filter(|_| !home.is_empty()) {
        Some(rest) => format!("~{rest}"),
        None => p.to_string(),
    };
    let parts: Vec<&str> = p.split('/').collect();
    if parts.len() <= 4 {
        return p;
    }
    let lead = if parts[0].is_empty() { 3 } else { 2 };
    if parts.len() <= lead + 2 {
        return p;
    }
    format!("{}/…/{}", parts[..lead].join("/"), parts[parts.len() - 1])
}

/// "claude_ai_Linear" → "Linear", "chrome-devtools" → "Chrome devtools".
fn server_name(s: &str) -> String {
    let s = s.strip_prefix("claude_ai_").or_else(|| s.strip_prefix("plugin_")).unwrap_or(s);
    let s = s.rsplit("_plugin_").next().unwrap_or(s);
    let s = s.replace(['_', '-'], " ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn server_mark(s: &str) -> &'static str {
    let l = s.to_lowercase();
    if l.contains("linear") {
        "t-linear"
    } else if l.contains("atlassian") || l.contains("jira") {
        "t-jira"
    } else if l.contains("github") {
        "t-github"
    } else if l.contains("chrome") {
        "x-chrome"
    } else {
        "x-plug"
    }
}

fn tool_glyph(name: &str) -> &'static str {
    match name {
        "Bash" | "BashOutput" | "KillShell" => "x-terminal",
        "Read" => "doc",
        "Edit" | "MultiEdit" | "NotebookEdit" => "x-pencil",
        "Write" => "x-doc-plus",
        "Grep" | "Glob" | "ToolSearch" => "x-search",
        "WebFetch" | "WebSearch" => "x-globe",
        "Agent" | "Task" => "x-agent",
        "TodoWrite" => "x-check",
        _ => "x-wrench",
    }
}

// ---------- colors (all from the theme) ----------

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let x = ((a >> s) & 0xff) as f32;
        let y = ((b >> s) & 0xff) as f32;
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Hue in degrees, saturation and lightness (0-1).
fn hsl(c: u32) -> (f32, f32, f32) {
    let h: Hsla = gpui::rgb(c).into();
    (h.h * 360., h.s, h.l)
}

fn too_close(a: u32, b: u32) -> bool {
    let ((h1, s1, l1), (h2, s2, l2)) = (hsl(a), hsl(b));
    // Grays never clash with hues; two grays are told apart by lightness alone.
    let hue_near = s1 < 0.15 || s2 < 0.15 || {
        let d = (h1 - h2).abs();
        d.min(360. - d) < 30.
    };
    let gray_pair = (s1 < 0.15) == (s2 < 0.15);
    gray_pair && hue_near && (l1 - l2).abs() < 0.08
}

/// The categories in the pane's order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Messages,
    SystemTools,
    McpTools,
    Skills,
    McpInstructions,
    SystemPrompt,
    Memory,
    Subagents,
    Free,
}

impl Cat {
    const ALL: [Cat; 9] = [Cat::Messages, Cat::SystemTools, Cat::McpTools, Cat::Skills, Cat::McpInstructions, Cat::SystemPrompt, Cat::Memory, Cat::Subagents, Cat::Free];

    fn label(self) -> &'static str {
        match self {
            Cat::Messages => "Messages",
            Cat::SystemTools => "System tools",
            Cat::McpTools => "MCP tools",
            Cat::Skills => "Skills",
            Cat::McpInstructions => "MCP instructions",
            Cat::SystemPrompt => "System prompt",
            Cat::Memory => "Memory files",
            Cat::Subagents => "Subagents",
            Cat::Free => "Free space",
        }
    }
}

/// Swatch colors from the theme's ANSI blue, cyan, magenta and green and its fg/bg (never
/// yellow or red: those mean needs-you and error). When two neighbors come out too close,
/// the later one takes its bright slot, then a step toward the background.
pub fn swatches(t: &Theme) -> [u32; 9] {
    let fg = |p: f32| mix(t.bg, t.fg, p);
    let base = [t.ansi[4], fg(0.62), t.ansi[6], t.ansi[5], mix(t.ansi[6], t.bg, 0.5), fg(0.42), t.ansi[2], fg(0.26), fg(0.07)];
    let bright = [Some(t.ansi[12]), None, Some(t.ansi[14]), Some(t.ansi[13]), None, None, Some(t.ansi[10]), None, None];
    let mut out = base;
    for i in 1..out.len() - 1 {
        if too_close(out[i - 1], out[i]) {
            // The bright slot, else a lightness step one way or the other.
            let c = out[i];
            let tries = [bright[i], Some(mix(c, t.bg, 0.35)), Some(mix(c, t.fg, 0.35)), Some(mix(c, t.bg, 0.6))];
            out[i] = tries.into_iter().flatten().find(|&b| !too_close(out[i - 1], b)).unwrap_or(c);
        }
    }
    out
}

// ---------- the numbers shown ----------

struct Now {
    p: u32,
    used: u64,
    window: u64,
    /// Category to tokens (Messages computed as what's left).
    cats: Vec<(Cat, u64)>,
    model: Option<String>,
}

fn now_numbers(a: Option<&AgentState>, r: &Report) -> Option<Now> {
    let (p, used, window) = match a.and_then(fill) {
        Some(f) => f,
        // The debug transcript has no agent: use the last reply's total on a 1M window.
        None => {
            let used = r.last_context?;
            let window = if used > 200_000 { 1_000_000 } else { 200_000 };
            ((used * 100 / window) as u32, used, window)
        }
    };
    let e = r.estimates();
    let messages = used.saturating_sub(e.sum());
    let cats = vec![
        (Cat::Messages, messages),
        (Cat::SystemTools, e.system_tools),
        (Cat::McpTools, e.mcp_tools),
        (Cat::Skills, e.skills),
        (Cat::McpInstructions, e.mcp_instructions),
        (Cat::SystemPrompt, e.system_prompt),
        (Cat::Memory, e.memory),
        (Cat::Subagents, e.subagents),
        (Cat::Free, window.saturating_sub(used)),
    ];
    let model = r.request.model.clone().or_else(|| a.and_then(|a| a.model.clone()));
    Some(Now { p, used, window, cats, model })
}

fn level(ui: UiColors, p: u32) -> Hsla {
    if p >= 90 { ui.status_error } else if p >= 70 { ui.status_needs } else { ui.text_primary }
}

fn stacked(ws: &Workspace, n: &Now, w: f32, h: f32) -> Div {
    let sw = swatches(ws.theme);
    let mut bar = div().w(px(w)).h(px(h)).rounded(px(h / 2.)).overflow_hidden().flex().bg(gpui::rgb(sw[8]));
    for (c, v) in &n.cats {
        if *c == Cat::Free || *v == 0 {
            continue;
        }
        let i = Cat::ALL.iter().position(|x| x == c).unwrap_or(0);
        let width = (w * *v as f32 / n.window.max(1) as f32).max(1.);
        bar = bar.child(div().h_full().w(px(width)).flex_none().bg(gpui::rgb(sw[i])));
    }
    bar
}

fn swatch(ws: &Workspace, c: Cat) -> Div {
    let i = Cat::ALL.iter().position(|x| *x == c).unwrap_or(0);
    div().size(px(9.)).flex_none().rounded(px(2.)).bg(gpui::rgb(swatches(ws.theme)[i]))
}

fn mono(s: impl Into<SharedString>, c: Hsla, size: f32) -> Div {
    div().flex_none().font_family(MONO).text_size(px(size)).text_color(c).child(s.into())
}

fn info(ui: UiColors, id: &str, text: &'static str) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("ctx-info-{id}")))
        .flex_none()
        .rounded_full()
        .child(crate::icons::artifact("x-info", ui.text_tertiary, 12.))
        .tooltip(crate::tooltip::text(ui, text))
}

const INFO_MESSAGES: &str = "The exact total minus every estimate below: your prompts, Claude's replies, tool calls and their results.";
const INFO_SUBAGENTS: &str = "Subagents work in their own context window, so their tokens aren't in the total above or its bar.\nSession totals do include them (the haiku row, for example).\nThis section is only the list of agent types, which does sit in this context.";
const INFO_SESSION: &str = "Everything this session has used since it started, including history that was compacted away and work done by subagents.\nSo it's larger than what's in context now.";
const INFO_DEFERRED: &str = "Tools Claude knows by name only.\nTheir definitions load the first time one is used, so they take no space in context now.";
const INFO_DROPPED: &str = "Added up across every compaction. None of it is in context now.";
const FOOT: &str = "Breakdown estimated from text size; total is exact.";

fn cat_row(ws: &Workspace, c: Cat, v: u64, of: u64, size: f32) -> Div {
    let ui = ws.ui;
    let est = !matches!(c, Cat::Free);
    let mut label = div().flex_1().min_w_0().flex().items_center().gap(px(5.)).child(div().truncate().text_size(px(size)).text_color(ui.text_secondary).child(c.label()));
    if c == Cat::Messages {
        label = label.child(info(ui, "messages", INFO_MESSAGES));
    }
    div()
        .h(px(22.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(swatch(ws, c))
        .child(label)
        .child(mono(if est { format!("≈{}", k(v)) } else { k(v) }, ui.text_primary, size - 0.5))
        .child(div().w(px(46.)).flex().justify_end().child(mono(pct(v, of), ui.text_tertiary, size - 0.5)))
}

// ---------- popover ----------

pub fn popover(ws: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let ui = ws.ui;
    let agent = ws.tabs.get(ws.active).and_then(|t| t.agent.as_ref());
    let report = ws.context_path().and_then(|p| ws.ctx.report(&p).cloned());
    let mut c = div()
        .w(px(320.))
        .p(px(14.))
        .rounded(px(10.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_md()
        .font_family(".SystemUIFont")
        .flex()
        .flex_col()
        .gap(px(8.));
    let empty = Report::default();
    let r = report.as_ref().unwrap_or(&empty);
    let Some(n) = now_numbers(agent, r) else {
        return c.child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("Context appears after the first reply."));
    };
    c = c
        .child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child("CONTEXT"))
                .children(n.model.clone().map(|m| mono(m, ui.text_tertiary, 11.))),
        )
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .child(div().font_family(MONO).text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(level(ui, n.p)).child(format!("{}%", n.p)))
                .child(mono(format!("{} of {}", k(n.used), win(n.window)), ui.text_tertiary, 12.)),
        )
        .child(stacked(ws, &n, 292., 10.));
    if n.p >= 70 {
        c = c.child(div().text_size(px(12.)).text_color(level(ui, n.p)).child("Near the limit. Claude Code compacts automatically."));
    }
    if report.is_none() {
        c = c.child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("Reading the transcript…"));
    } else {
        let mut top: Vec<(Cat, u64)> = n.cats.iter().copied().filter(|(c, _)| *c != Cat::Free).collect();
        top.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        let mut rows = div().flex().flex_col();
        for (cat, v) in top.into_iter().take(4) {
            rows = rows.child(cat_row(ws, cat, v, n.window, 12.5));
        }
        let free = n.cats.last().map(|c| c.1).unwrap_or(0);
        rows = rows.child(cat_row(ws, Cat::Free, free, n.window, 12.5));
        c = c.child(rows);
        let comp = match r.compactions.last() {
            None => "No compactions yet".to_string(),
            Some(l) => {
                let count = r.compactions.len();
                let s = if count == 1 { "" } else { "s" };
                let when = l.at.map(ago).map(|a| format!(" · last {a}")).unwrap_or_default();
                let sizes = l.after.map(|a| format!(" ({} → {})", k(l.before), k(a))).unwrap_or_default();
                format!("{count} compaction{s}{when}{sizes}")
            }
        };
        c = c.child(div().text_size(px(12.)).text_color(ui.text_secondary).child(comp));
        c = c.child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("Breakdown estimated; total is exact."));
    }
    c.child(
        div()
            .id("ctx-details")
            .text_size(px(12.5))
            .text_color(ui.accent)
            .cursor_pointer()
            .hover(|d| d.opacity(0.8))
            .child("Details ›")
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlay(window, cx);
                this.show_context_tab(cx);
            })),
    )
}

// ---------- pane ----------

#[allow(clippy::too_many_arguments)]
fn section(ws: &Workspace, key: &'static str, title: &'static str, default_open: bool, info_text: Option<&'static str>, right: String, body: impl FnOnce() -> Div, cx: &mut Context<Workspace>) -> Div {
    let ui = ws.ui;
    let open = ws.ctx_open(key, default_open);
    let header = div()
        .id(SharedString::from(format!("ctx-sec-{key}")))
        .h(px(28.))
        .px(px(8.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .hover(move |d| d.bg(ui.bg_hover))
        .child(crate::icons::caret(ui.text_tertiary, open))
        .child(div().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(title))
        .children(info_text.map(|t| info(ui, key, t)))
        .child(div().flex_1())
        .child(mono(right, ui.text_disabled, 11.))
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.ctx.toggled.remove(key) {
                this.ctx.toggled.insert(key.to_string());
            }
            this.save();
            cx.notify();
        }));
    let mut s = div().flex().flex_col().child(header);
    if open {
        s = s.child(body().px(px(10.)).pb(px(10.)));
    }
    s
}

fn kv(ui: UiColors, key: impl Into<SharedString>, val: impl Into<SharedString>) -> Div {
    div()
        .h(px(21.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(ui.text_secondary).child(key.into()))
        .child(mono(val, ui.text_primary, 11.5))
}

fn icon_row(ui: UiColors, glyph: &str, label: impl Into<SharedString>, val: String, indent: f32) -> Div {
    div()
        .h(px(21.))
        .pl(px(indent))
        .flex()
        .items_center()
        .gap(px(7.))
        .child(crate::icons::artifact(glyph, ui.text_tertiary, 14.))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(ui.text_secondary).child(label.into()))
        .child(mono(val, ui.text_tertiary, 11.5))
}

fn est(chars: usize) -> String {
    format!("≈{}", k(tokens(chars)))
}

/// Compaction bar: kept part solid, dropped part striped, both scaled to the window.
fn compaction_bar(ui: UiColors, w: f32, before: u64, after: u64, window: u64) -> impl IntoElement {
    let kept = (after as f32 / window.max(1) as f32).min(1.) * w;
    let total = (before as f32 / window.max(1) as f32).min(1.) * w;
    let solid = gpui::Hsla::blend(ui.bg_terminal, ui.text_primary.opacity(0.45));
    let stripe = ui.text_primary.opacity(0.22);
    canvas(|_, _, _| {}, move |b, _, window, _| {
        let o = b.origin;
        let h = 8.;
        window.paint_quad(gpui::fill(gpui::Bounds::new(o, gpui::size(px(w), px(h))), ui.text_primary.opacity(0.06)));
        window.paint_quad(gpui::fill(gpui::Bounds::new(o, gpui::size(px(kept), px(h))), solid));
        // Diagonal stripes over the dropped part, clipped to it.
        let (x0, x1) = (kept + 1., total);
        let mut p = PathBuilder::stroke(px(1.));
        let mut x = x0 - h;
        while x < x1 {
            let (mut ax, mut ay, mut bx, mut by) = (x, h, x + h, 0.);
            if ax < x0 {
                ay -= x0 - ax;
                ax = x0;
            }
            if bx > x1 {
                by += bx - x1;
                bx = x1;
            }
            if bx > ax {
                p.move_to(point(o.x + px(ax), o.y + px(ay)));
                p.line_to(point(o.x + px(bx), o.y + px(by)));
            }
            x += 4.;
        }
        if let Ok(path) = p.build() {
            window.paint_path(path, stripe);
        }
    })
    .w(px(w))
    .h(px(8.))
}

pub fn pane(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let agent = ws.tabs.get(ws.active).and_then(|t| t.agent.as_ref());
    let report = ws.context_path().and_then(|p| ws.ctx.report(&p).cloned());
    let mut list = div()
        .id("ctx-list")
        .size_full()
        .overflow_y_scroll()
        .px(px(6.))
        .pb(px(16.))
        .flex()
        .flex_col()
        .gap(px(2.));
    let Some(r) = report else {
        let msg = if ws.context_path().is_some() { "Reading the transcript…" } else { "Context appears after the first reply." };
        return list.child(div().p(px(14.)).text_size(px(12.5)).text_color(ui.text_tertiary).child(msg)).into_any_element();
    };
    let Some(n) = now_numbers(agent, &r) else {
        return list.child(div().p(px(14.)).text_size(px(12.5)).text_color(ui.text_tertiary).child("Context appears after the first reply.")).into_any_element();
    };
    let w = 348.;

    // NOW
    let (mcp_def, sys_def): (Vec<&String>, Vec<&String>) = r.deferred.iter().partition(|n| n.starts_with("mcp__"));
    let now_body = || {
        let mut b = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .child(div().font_family(MONO).text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(level(ui, n.p)).child(format!("{}%", n.p)))
                    .child(mono(format!("{} of {}", k(n.used), win(n.window)), ui.text_tertiary, 12.)),
            )
            .child(stacked(ws, &n, w, 10.));
        let mut rows = div().flex().flex_col();
        for (c, v) in &n.cats {
            rows = rows.child(cat_row(ws, *c, *v, n.window, 12.5));
        }
        let dash = |label: &'static str, count: usize| {
            div()
                .h(px(22.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().size(px(9.)).flex_none().rounded(px(2.)).border_1().border_color(ui.text_disabled))
                .child(div().flex_1().text_size(px(12.5)).text_color(ui.text_tertiary).child(label))
                .child(mono(format!("{count} names"), ui.text_tertiary, 12.))
                .child(div().w(px(46.)).flex().justify_end().child(mono("—", ui.text_disabled, 12.)))
        };
        rows = rows.child(div().h(px(6.))).child(dash("Deferred MCP tools", mcp_def.len())).child(dash("Deferred system tools", sys_def.len()));
        b = b.child(rows).child(div().pt(px(4.)).text_size(px(11.)).text_color(ui.text_tertiary).child(FOOT));
        b
    };
    list = list.child(section(ws, "now", "NOW", true, None, "exact total".into(), now_body, cx));

    // COMPACTIONS
    let comp_right = if r.compactions.is_empty() { "none".to_string() } else { format!("{} · {} dropped", r.compactions.len(), k(r.dropped())) };
    let comps = r.compactions.clone();
    let started = r.started;
    let window = n.window;
    let comp_body = move || {
        let mut b = div().flex().flex_col().gap(px(10.));
        if comps.is_empty() {
            return b.child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("No compactions yet."));
        }
        // Timeline: session start to now, a ring per compaction.
        if let Some(start) = started {
            let span = SystemTime::now().duration_since(start).map(|d| d.as_secs_f32()).unwrap_or(1.).max(1.);
            let mut line = div().relative().w(px(w)).h(px(26.)).child(div().absolute().left_0().right_0().top(px(6.)).h(px(1.)).bg(ui.text_primary.opacity(0.18)));
            for (i, c) in comps.iter().enumerate() {
                let Some(at) = c.at else { continue };
                let f = at.duration_since(start).map(|d| d.as_secs_f32()).unwrap_or(0.) / span;
                let x = (f * w).clamp(4., w - 4.);
                line = line
                    .child(div().absolute().left(px(x - 4.)).top(px(2.5)).size(px(8.)).rounded_full().border_1().border_color(ui.text_tertiary).bg(ui.bg_chrome))
                    .child(div().absolute().left(px(x - 8.)).top(px(12.)).font_family(MONO).text_size(px(9.5)).text_color(ui.text_tertiary).child(format!("#{}", i + 1)));
            }
            b = b.child(line).child(
                div()
                    .flex()
                    .child(mono(format!("started {}", ago(start)), ui.text_disabled, 10.5))
                    .child(div().flex_1())
                    .child(mono("now", ui.text_disabled, 10.5)),
            );
        }
        for (i, c) in comps.iter().enumerate() {
            let after = c.after.unwrap_or(0);
            let mut under = Vec::new();
            if let Some(at) = c.at {
                under.push(ago(at));
            }
            if let Some(d) = c.duration_ms {
                under.push(format!("took {}", dur(d)));
            }
            b = b.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .child(div().flex_1().text_size(px(12.)).text_color(ui.text_secondary).child(format!("#{} · {}", i + 1, if c.auto { "auto" } else { "manual" })))
                            .child(mono(c.after.map(|a| format!("{} → {}", k(c.before), k(a))).unwrap_or_else(|| k(c.before)), ui.text_primary, 11.5)),
                    )
                    .child(compaction_bar(ui, w, c.before, after, window))
                    .child(
                        div()
                            .flex()
                            .child(div().flex_1().text_size(px(11.)).text_color(ui.text_tertiary).child(under.join(" · ")))
                            .children(c.after.map(|a| mono(format!("−{}", k(c.before.saturating_sub(a))), ui.text_tertiary, 11.))),
                    ),
            );
        }
        b
    };
    let comp_default = !r.compactions.is_empty();
    list = list.child(section(ws, "compactions", "COMPACTIONS", comp_default, (!r.compactions.is_empty()).then_some(INFO_DROPPED), comp_right, comp_body, cx));

    // SESSION
    let cost = agent.and_then(|a| a.cost);
    let models = r.models.clone();
    let session_right = cost.map(|c| format!("${:.2}", c.usd)).unwrap_or_default();
    let session_body = move || {
        let mut b = div().flex().flex_col();
        if let Some(c) = cost {
            b = b.child(kv(ui, "Cost", format!("${:.2}", c.usd)));
        }
        let mut ms: Vec<(&String, &t1k_agent::context::ModelUsage)> = models.iter().collect();
        ms.sort_by_key(|(_, m)| std::cmp::Reverse(m.replies));
        let mut searches = 0;
        for (i, (name, m)) in ms.iter().enumerate() {
            searches += m.web_searches;
            if i == 0 {
                b = b
                    .child(div().pt(px(6.)).pb(px(2.)).child(mono(name.to_string(), ui.text_tertiary, 11.)))
                    .child(kv(ui, "Input", k(m.input)))
                    .child(kv(ui, "Output", k(m.output)))
                    .when(m.thinking > 0, |d| d.child(kv(ui, "Thinking", k(m.thinking))))
                    .child(kv(ui, "Cache read", k(m.cache_read)))
                    .child(kv(ui, "Cache write · 5m", k(m.cache_write_5m)))
                    .child(kv(ui, "Cache write · 1h", k(m.cache_write_1h)));
            } else {
                b = b.child(kv(ui, name.to_string(), format!("{} in · {} out", k(m.input + m.cache_read + m.cache_write()), k(m.output))));
            }
        }
        b = b.child(div().h(px(6.)));
        if searches > 0 {
            b = b.child(kv(ui, "Web searches", searches.to_string()));
        }
        if let Some(c) = cost {
            b = b
                .child(kv(ui, "Wall · API", format!("{} · {}", dur(c.duration_ms), dur(c.api_ms))))
                .child(kv(ui, "Lines", format!("+{} −{}", c.lines_added, c.lines_removed)));
        }
        b
    };
    list = list.child(section(ws, "session", "SESSION", true, Some(INFO_SESSION), session_right, session_body, cx));

    // REQUEST
    let req = r.request.clone();
    let req_body = move || {
        let mut b = div().flex().flex_col();
        let rows = [
            ("Model", req.model.clone().or(n.model.clone())),
            ("Effort", req.effort.clone()),
            ("Max output", req.max_tokens.map(win)),
            ("Thinking display", req.thinking.clone()),
            ("Cache TTL", req.cache_ttl.clone()),
        ];
        for (key, v) in rows {
            if let Some(v) = v {
                b = b.child(kv(ui, key, v));
            }
        }
        b
    };
    list = list.child(section(ws, "request", "REQUEST", true, None, String::new(), req_body, cx));

    // LOADED TOOLS: built-in first, then each MCP server, biggest first.
    let mut groups: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for (name, chars) in &r.tools {
        let g = mcp_server(name).map(str::to_string).unwrap_or_default();
        groups.entry(g).or_default().push((name.clone(), *chars));
    }
    let tools_total: usize = r.tools.iter().map(|t| t.1).sum();
    let mut ordered: Vec<(String, Vec<(String, usize)>)> = groups.into_iter().collect();
    ordered.sort_by_key(|(g, v)| (!g.is_empty(), std::cmp::Reverse(v.iter().map(|t| t.1).sum::<usize>())));
    let mut tools_body = div().flex().flex_col();
    for (g, mut v) in ordered {
        v.sort_by_key(|t| std::cmp::Reverse(t.1));
        let sum: usize = v.iter().map(|t| t.1).sum();
        let key = format!("tools:{g}");
        let builtin = g.is_empty();
        let open = ws.ctx_open(&key, builtin);
        let label = if builtin { format!("Built-in · {} tools", v.len()) } else { format!("{} · {} tools", server_name(&g), v.len()) };
        let k2 = key.clone();
        tools_body = tools_body.child(
            div()
                .id(SharedString::from(key.clone()))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.ctx.toggled.remove(&k2) {
                        this.ctx.toggled.insert(k2.clone());
                    }
                    cx.notify();
                }))
                .child(icon_row(ui, if builtin { "x-wrench" } else { server_mark(&g) }, label, est(sum), 0.).text_color(ui.text_primary)),
        );
        if open {
            let shown = if builtin { 9 } else { 2 };
            let more_open = ws.ctx_open(&format!("{key}:more"), false);
            let take = if more_open { v.len() } else { shown };
            for (name, chars) in v.iter().take(take) {
                let short = name.rsplit("__").next().unwrap_or(name).to_string();
                let glyph = if builtin { tool_glyph(name) } else { "x-wrench" };
                tools_body = tools_body.child(icon_row(ui, glyph, short, est(*chars), 14.));
            }
            if v.len() > take {
                let rest: usize = v.iter().skip(take).map(|t| t.1).sum();
                let mk = format!("{key}:more");
                tools_body = tools_body.child(
                    div()
                        .id(SharedString::from(mk.clone()))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ctx.toggled.insert(mk.clone());
                            cx.notify();
                        }))
                        .child(
                            div()
                                .h(px(21.))
                                .pl(px(35.))
                                .flex()
                                .items_center()
                                .child(div().flex_1().text_size(px(12.)).text_color(ui.text_tertiary).hover(move |d| d.text_color(ui.text_primary)).child(format!("+{} more", v.len() - take)))
                                .child(mono(est(rest), ui.text_disabled, 11.5)),
                        ),
                );
            }
        }
    }
    list = list.child(section(ws, "tools", "LOADED TOOLS", false, None, est(tools_total), move || tools_body, cx));

    // MEMORY FILES
    let mem = r.memory.clone();
    let mem_total: usize = mem.values().sum();
    let mem_body = move || {
        let mut b = div().flex().flex_col();
        for (path, chars) in &mem {
            b = b.child(
                div()
                    .id(SharedString::from(format!("mem-{path}")))
                    .tooltip(crate::tooltip::text(ui, path.clone()))
                    .child(icon_row(ui, "doc", short_path(path), est(*chars), 0.)),
            );
        }
        if mem.is_empty() {
            b = b.child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("None loaded."));
        }
        b
    };
    list = list.child(section(ws, "memory", "MEMORY FILES", false, None, est(mem_total), mem_body, cx));

    // MCP INSTRUCTIONS
    let instr = r.mcp_instructions.clone();
    let instr_total: usize = instr.values().sum();
    let instr_right = format!("{} servers · {}", instr.len(), est(instr_total));
    let instr_body = move || {
        let mut v: Vec<(&String, &usize)> = instr.iter().collect();
        v.sort_by_key(|x| std::cmp::Reverse(*x.1));
        v.into_iter().fold(div().flex().flex_col(), |b, (s, c)| b.child(icon_row(ui, server_mark(s), server_name(s), est(*c), 0.)))
    };
    list = list.child(section(ws, "instructions", "MCP INSTRUCTIONS", false, None, instr_right, instr_body, cx));

    // SKILLS
    let (skill_count, listing) = r.skills.unwrap_or_default();
    let invoked = r.invoked_skills.clone();
    let skills_total = listing + invoked.values().sum::<usize>();
    let skills_body = move || {
        let mut b = div().flex().flex_col().child(icon_row(ui, "x-check", format!("Listing · {skill_count} skills"), est(listing), 0.));
        for (name, c) in &invoked {
            b = b.child(icon_row(ui, "doc", format!("{name} (in use)"), est(*c), 0.));
        }
        b
    };
    list = list.child(section(ws, "skills", "SKILLS", false, None, format!("{skill_count} · {}", est(skills_total)), skills_body, cx));

    // SUBAGENTS
    let agents = r.agents.clone();
    let agents_total: usize = agents.values().sum();
    let agents_body = move || agents.iter().fold(div().flex().flex_col(), |b, (t, c)| b.child(icon_row(ui, "x-agent", t.clone(), est(*c), 0.)));
    list = list.child(section(ws, "subagents", "SUBAGENTS", false, Some(INFO_SUBAGENTS), format!("{} types · {}", r.agents.len(), est(agents_total)), agents_body, cx));

    // DEFERRED TOOLS: names only, by server.
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for name in &r.deferred {
        *by.entry(mcp_server(name).map(str::to_string).unwrap_or_default()).or_default() += 1;
    }
    let servers = by.keys().filter(|k| !k.is_empty()).count();
    let def_right = format!("{} names · {servers} servers", r.deferred.len());
    let def_body = move || {
        let mut v: Vec<(String, usize)> = by.into_iter().collect();
        v.sort_by_key(|(g, n)| (!g.is_empty(), std::cmp::Reverse(*n)));
        v.into_iter().fold(div().flex().flex_col(), |b, (g, n)| {
            let (glyph, label) = if g.is_empty() { ("x-wrench", "Built-in".to_string()) } else { (server_mark(&g), server_name(&g)) };
            b.child(icon_row(ui, glyph, label, format!("{n}"), 0.))
        })
    };
    list = list.child(section(ws, "deferred", "DEFERRED TOOLS", false, Some(INFO_DEFERRED), def_right, def_body, cx));

    list.into_any_element()
}

/// The Outputs | Context switch in the panel's header (only for tabs with context details).
pub fn switch(ws: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let ui = ws.ui;
    let k = ws.config.interface.text_scale();
    let tab = ws.panel.tab;
    let seg = |id: &'static str, label: &'static str, which: PanelTab| {
        let on = tab == which;
        div()
            .id(id)
            .px(px(8.))
            .h(px(22.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(12.5 * k))
            .font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
            .text_color(if on { ui.text_primary } else { ui.text_tertiary })
            .when(on, |d| d.bg(ui.bg_control))
            .hover(move |d| d.text_color(ui.text_primary))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.panel.tab = which;
                if which == PanelTab::Context {
                    this.context_tick(cx);
                }
                this.save();
                cx.notify();
            }))
            .child(label)
    };
    div().flex().items_center().gap(px(2.)).child(seg("panel-outputs", "Outputs", PanelTab::Outputs)).child(seg("panel-context", "Context", PanelTab::Context))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbors_stay_apart_in_every_builtin_theme() {
        for t in t1k_themes::all() {
            let s = swatches(t);
            for i in 1..s.len() - 1 {
                assert!(!too_close(s[i - 1], s[i]), "{}: {:?} and {:?} look alike", t.name, Cat::ALL[i - 1], Cat::ALL[i]);
            }
        }
    }

    #[test]
    fn paths_and_names() {
        assert_eq!(short_path("/a/b/c/d/e/MEMORY.md"), "/a/b/…/MEMORY.md");
        assert_eq!(short_path("/a/b.md"), "/a/b.md");
        assert_eq!(server_name("claude_ai_Linear"), "Linear");
        assert_eq!(server_name("chrome-devtools"), "Chrome devtools");
        assert_eq!(win(1_000_000), "1M");
        assert_eq!(win(200_000), "200k");
        assert_eq!(k(144_200), "144.2k");
    }
}
