//! Triggers (canvas v3-az): rules that watch a tab and act. When a line of new output matches,
//! an agent needs you (or finishes, errs, stops watching), or a command fails, a trigger runs
//! its actions, or asks: a bar under the terminal offers them as buttons (⌃1–⌃9) with a
//! default that runs when nobody chooses in time.
//!
//! Matching sees new output only, line by line with escape sequences removed: what streams
//! past, and rows that appear on screen (programs like Claude Code draw in place), each once. Loop guards: after a trigger types into a tab it ignores that tab's
//! output for 2 s, it fires at most once per cooldown per tab, and five firings in a minute
//! switch it off for that tab.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime};

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, SharedString, Window};
use regex::Regex;
use t1k_agent::Status;

use crate::config::{ActionCfg, TriggerCfg};
use crate::split::{PaneId, MAIN};
use crate::workspace::Workspace;

const IGNORE_AFTER_TYPING: Duration = Duration::from_secs(2);
const RATE_WINDOW: Duration = Duration::from_secs(60);
const RATE_MAX: usize = 5;

/// Every action a trigger can take: (kind, what it's called, what its argument is).
pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("notify", "Notify", "Message (empty: the line)"),
    ("unread", "Mark unread", ""),
    ("needs", "Mark needs you", ""),
    ("send", "Send text", "Text; \\n presses Enter"),
    ("prompt", "Send a prompt to the agent", "Prompt"),
    ("pane", "Run in a new pane", "Command; the pane closes when it succeeds"),
    ("run", "Run a command in the background", "Shell command; $1… are the match's groups"),
    ("open", "Open a URL", "URL; $0 is the match"),
    ("copy", "Copy", "What to copy (empty: the match)"),
    ("highlight", "Highlight the match", ""),
    ("remind", "Remind me", "Minutes"),
    ("front", "Bring T1K forward", ""),
];

pub fn action_label(a: &ActionCfg) -> String {
    if !a.label.trim().is_empty() {
        return a.label.clone();
    }
    match a.kind.as_str() {
        "send" => format!("Send \u{201c}{}\u{201d}", a.arg.replace("\\n", " ⏎").trim()),
        "run" | "pane" => format!("Run {}", a.arg.split_whitespace().take(3).collect::<Vec<_>>().join(" ")),
        "remind" => format!("Remind me in {}m", a.arg.trim().parse::<u64>().unwrap_or(15)),
        k => ACTIONS.iter().find(|x| x.0 == k).map(|x| x.1.to_string()).unwrap_or_else(|| k.to_string()),
    }
}

/// What a trigger watches, in a few words ("Output matches ^FAIL").
pub fn describe_when(t: &TriggerCfg) -> String {
    match t.when.as_str() {
        "agent" => {
            let e = match t.event.as_str() {
                "done" => "Agent finished",
                "error" => "Agent error",
                "watching_ends" => "Agent stops watching",
                _ => "Agent needs you",
            };
            if t.pattern.is_empty() { e.into() } else { format!("{e}: {}", t.pattern) }
        }
        "exit" => "Command fails".into(),
        _ => format!("Output matches {}", if t.pattern.is_empty() { "anything" } else { &t.pattern }),
    }
}

pub fn describe_then(t: &TriggerCfg) -> String {
    let names: Vec<String> = t.actions.iter().map(action_label).collect();
    if t.mode == "ask" { format!("Ask: {}", names.join(" / ")) } else { names.join(", ") }
}

/// A trigger's pattern, compiled; the error says what's wrong with it.
pub fn compile(pattern: &str) -> Result<Option<Regex>, String> {
    if pattern.trim().is_empty() {
        return Ok(None);
    }
    Regex::new(pattern).map(Some).map_err(|e| e.to_string().lines().last().unwrap_or("Not a valid pattern").trim().to_string())
}

/// "aws sso login": a command's first three words.
pub fn short_cmd(c: &str) -> String {
    c.split_whitespace().take(3).collect::<Vec<_>>().join(" ")
}

/// `$0`, `$1`… in `s`, filled from the match.
pub fn fill(s: &str, caps: &[String]) -> String {
    let mut out = s.to_string();
    for (i, c) in caps.iter().enumerate().rev() {
        out = out.replace(&format!("${i}"), c);
    }
    out
}

/// An ask waiting for a choice, in a tab.
#[derive(Clone)]
pub struct Ask {
    pub trigger: usize,
    pub name: String,
    pub tab: String,
    pub pane: PaneId,
    pub line: String,
    pub caps: Vec<String>,
    pub actions: Vec<ActionCfg>,
    pub default: Option<usize>,
    pub deadline: Instant,
    pub timeout: Duration,
}

/// How a background command a trigger ran ended (canvas v3-ba).
#[derive(Clone, Debug, PartialEq)]
pub enum RunEnd {
    Running,
    Exit(i32),
    /// Ended by a signal, or couldn't be waited on.
    Killed,
    /// Ran past the trigger's limit and was stopped.
    Stopped(u64),
}

impl RunEnd {
    pub fn failed(&self) -> bool {
        !matches!(self, RunEnd::Running | RunEnd::Exit(0))
    }
}

/// A background command a trigger started: what, how it went, its last lines.
#[derive(Clone)]
pub struct Run {
    pub command: String,
    pub end: RunEnd,
    pub took: Option<Duration>,
    pub output: Vec<String>,
}

/// What a trigger did, for the toast and the tab's log.
#[derive(Clone)]
pub struct Fired {
    pub id: u64,
    pub at: SystemTime,
    pub name: String,
    pub what: String,
    /// "automatically", "you chose", "no choice in 60s".
    pub why: String,
    /// The line it matched.
    pub line: String,
    pub run: Option<Run>,
}

/// Where a background command reports how it ended: (end, last lines, how long it took).
type RunDone = std::sync::mpsc::Receiver<(RunEnd, Vec<String>, Duration)>;

/// Lines of a background command's output kept, and how long one may be.
const RUN_LINES: usize = 20;
const RUN_LINE_MAX: usize = 300;

/// Run `command` with `sh -c` in `cwd`, keeping its last lines; stop it after `limit`
/// (SIGTERM, then SIGKILL 5 s later).
fn run_in_background(command: String, cwd: std::path::PathBuf, env: Vec<(String, String)>, limit: Duration) -> RunDone {
    use std::io::Read;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(&command).current_dir(cwd).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        cmd.env("PATH", crate::platform::tool_path());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send((RunEnd::Killed, vec![format!("couldn't start: {e}")], started.elapsed()));
                return;
            }
        };
        let lines = std::sync::Arc::new(std::sync::Mutex::new(VecDeque::<String>::new()));
        let mut readers = Vec::new();
        for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)].into_iter().flatten() {
            let lines = lines.clone();
            readers.push(std::thread::spawn(move || {
                let mut stream = stream;
                let mut split = t1k_terminal::notify_scan::LineSplitter::default();
                let mut buf = [0u8; 4096];
                let mut out = Vec::new();
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    split.feed(&buf[..n], &mut out);
                    let mut q = lines.lock().unwrap_or_else(|e| e.into_inner());
                    for l in out.drain(..) {
                        let l = if l.chars().count() > RUN_LINE_MAX { format!("{}…", l.chars().take(RUN_LINE_MAX).collect::<String>()) } else { l };
                        if q.len() >= RUN_LINES {
                            q.pop_front();
                        }
                        q.push_back(l);
                    }
                }
            }));
        }
        let mut stopped = false;
        let end = loop {
            match child.try_wait() {
                Ok(Some(status)) => break if stopped { RunEnd::Stopped(limit.as_secs() / 60) } else { status.code().map(RunEnd::Exit).unwrap_or(RunEnd::Killed) },
                Ok(None) => {}
                Err(_) => break RunEnd::Killed,
            }
            if !stopped && started.elapsed() > limit {
                stopped = true;
                unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
                let pid = child.id();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(5));
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                });
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        for r in readers {
            let _ = r.join();
        }
        let out: Vec<String> = lines.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
        let _ = tx.send((end, out, started.elapsed()));
    });
    rx
}

#[derive(Default)]
pub struct TrigState {
    compiled: Vec<Option<Regex>>,
    compiled_for: Vec<String>,
    pub asks: Vec<Ask>,
    pub log: HashMap<String, Vec<Fired>>,
    next_id: u64,
    /// A background command just started by an action, for the entry being recorded.
    started: Option<(Run, RunDone)>,
    /// Commands still running: (tab, entry id, where they report).
    running: Vec<(String, u64, RunDone)>,
    /// The entry the notice is about, so a finishing command updates it in place.
    notice_for: Option<u64>,
    /// The TRIGGERS entry open in the Outputs panel.
    pub open: Option<u64>,
    last: HashMap<(usize, String), Instant>,
    typed: HashMap<(usize, String), Instant>,
    rate: HashMap<(usize, String), VecDeque<Instant>>,
    off: HashSet<(usize, String)>,
    /// Lines already matched against, per terminal (by view): output a program redraws in
    /// place (Claude Code draws by moving the cursor) is read off the screen, and each line
    /// counts once while it's there.
    seen: HashMap<gpui::EntityId, HashMap<String, Instant>>,
    /// Last agent state seen per tab: (status kind, watching), to notice changes.
    agent_seen: HashMap<String, (u8, bool)>,
    watching_lines: bool,
}

fn kind_of(s: Option<&Status>) -> u8 {
    match s {
        Some(Status::NeedsInput { .. }) => 1,
        Some(Status::Done) => 2,
        Some(Status::Error { .. }) => 3,
        Some(Status::Working) => 4,
        _ => 0,
    }
}

impl Workspace {
    fn trig_regexes(&mut self) {
        let pats: Vec<String> = self.config.triggers.iter().map(|t| t.pattern.clone()).collect();
        if pats != self.trig.compiled_for {
            self.trig.compiled = pats.iter().map(|p| compile(p).ok().flatten()).collect();
            self.trig.compiled_for = pats;
        }
    }

    /// Does trigger `t` watch this tab (its scope)?
    fn trig_in_scope(&self, t: &TriggerCfg, ix: usize, pane: PaneId) -> bool {
        let tab = &self.tabs[ix];
        match t.scope.as_str() {
            "groups" => {
                let label = self.group_label(&tab.group).to_lowercase();
                t.groups.iter().any(|g| g.trim().to_lowercase() == label)
            }
            "program" => {
                let prog = if pane == MAIN { tab.program.clone() } else { tab.panes.helper(pane).and_then(|h| h.program.clone()) };
                prog.is_some_and(|p| p.eq_ignore_ascii_case(t.program.trim()))
            }
            _ => true,
        }
    }

    /// Every tick: read new output and exits, notice agent events, run what matched, and
    /// time out asks.
    pub fn triggers_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paused = self.config.triggers_paused || crate::demo::on();
        let on = |w: &str| self.config.triggers.iter().any(|t| t.enabled && t.when == w);
        let want_lines = !paused && on("output");
        let want_exits = !paused && on("exit");
        let want_agents = !paused && on("agent");
        // Only collect output while something watches it.
        if want_lines != self.trig.watching_lines || want_lines {
            for t in &self.tabs {
                for v in std::iter::once(&t.view).chain(t.panes.helpers.iter().map(|h| &h.view)) {
                    v.read(cx).terminal.watch_lines(want_lines);
                }
            }
            self.trig.watching_lines = want_lines;
        }
        self.trig_regexes();
        let mut hits: Vec<(usize, usize, PaneId, String, Vec<String>)> = Vec::new();
        for ix in 0..self.tabs.len() {
            let panes: Vec<(PaneId, gpui::Entity<t1k_terminal::TerminalView>)> = std::iter::once((MAIN, self.tabs[ix].view.clone()))
                .chain(self.tabs[ix].panes.helpers.iter().map(|h| (h.id, h.view.clone())))
                .collect();
            for (pane, view) in panes {
                let lines = if want_lines { self.new_lines(&view, cx) } else { Vec::new() };
                let exits = view.read(cx).terminal.take_exits();
                for (ti, t) in self.config.triggers.iter().enumerate() {
                    if !t.enabled || paused || !self.trig_in_scope(t, ix, pane) {
                        continue;
                    }
                    match t.when.as_str() {
                        "output" => {
                            for line in &lines {
                                let caps = match &self.trig.compiled[ti] {
                                    Some(re) => re.captures(line).map(|c| c.iter().map(|m| m.map(|m| m.as_str().to_string()).unwrap_or_default()).collect::<Vec<_>>()),
                                    None => (!line.trim().is_empty()).then(|| vec![line.clone()]),
                                };
                                if let Some(c) = caps {
                                    hits.push((ti, ix, pane, line.clone(), c));
                                    break;
                                }
                            }
                        }
                        "exit" if want_exits => {
                            for code in exits.iter().filter(|c| **c != 0) {
                                let s = code.to_string();
                                let ok = self.trig.compiled[ti].as_ref().is_none_or(|re| re.is_match(&s));
                                if ok {
                                    hits.push((ti, ix, pane, format!("Command failed (exit {code})"), vec![s.clone(), s]));
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Agent events: the main pane's agent changing state.
            if want_agents {
                let t = &self.tabs[ix];
                let now = (kind_of(t.agent.as_ref().map(|a| &a.status)), t.watching().is_some());
                let before = self.trig.agent_seen.insert(t.id.clone(), now);
                if let Some(before) = before.filter(|b| *b != now) {
                    let event = match (before, now) {
                        (_, (1, _)) if before.0 != 1 => Some("needs"),
                        (_, (2, false)) if before.0 != 2 => Some("done"),
                        (_, (3, _)) if before.0 != 3 => Some("error"),
                        ((_, true), (_, false)) => Some("watching_ends"),
                        _ => None,
                    };
                    if let Some(event) = event {
                        let a = t.agent.as_ref();
                        let reason = match a.map(|a| &a.status) {
                            Some(Status::NeedsInput { reason }) => reason.clone(),
                            Some(Status::Error { message }) => message.clone(),
                            _ => String::new(),
                        };
                        // What it wants: its request, then the tool call it's about.
                        let text = [reason, a.and_then(|a| a.now.clone()).unwrap_or_default()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
                        for (ti, tr) in self.config.triggers.iter().enumerate() {
                            if !tr.enabled || tr.when != "agent" || tr.event != event || !self.trig_in_scope(tr, ix, MAIN) {
                                continue;
                            }
                            let caps = match &self.trig.compiled[ti] {
                                Some(re) => re.captures(&text).map(|c| c.iter().map(|m| m.map(|m| m.as_str().to_string()).unwrap_or_default()).collect::<Vec<_>>()),
                                None => Some(vec![text.clone()]),
                            };
                            if let Some(c) = caps {
                                hits.push((ti, ix, MAIN, text.clone(), c));
                            }
                        }
                    }
                }
            }
        }
        // Several triggers on one line: the first wins; later ones still run unless they ask
        // too (one question at a time per line).
        let mut asked: HashSet<(usize, String)> = HashSet::new();
        for (ti, ix, pane, line, caps) in hits {
            let Some(t) = self.config.triggers.get(ti).cloned() else { continue };
            let tab = self.tabs[ix].id.clone();
            let key = (ti, tab.clone());
            let now = Instant::now();
            if self.trig.off.contains(&key) {
                continue;
            }
            if self.trig.typed.get(&key).is_some_and(|at| at.elapsed() < IGNORE_AFTER_TYPING) {
                continue;
            }
            // The cooldown counts per tab, or for every tab at once.
            let cool = if t.cooldown_scope == "all" { (ti, String::new()) } else { key.clone() };
            if self.trig.last.get(&cool).is_some_and(|at| at.elapsed() < Duration::from_secs(t.cooldown)) {
                continue;
            }
            let rate = self.trig.rate.entry(key.clone()).or_default();
            while rate.front().is_some_and(|at| at.elapsed() > RATE_WINDOW) {
                rate.pop_front();
            }
            rate.push_back(now);
            if rate.len() > RATE_MAX {
                self.trig.off.insert(key);
                self.trig_toast(format!("Trigger \u{201c}{}\u{201d} fired {RATE_MAX} times in a minute; it's off in this tab", t.name), cx);
                continue;
            }
            self.trig.last.insert(cool, now);
            if t.mode == "ask" {
                if !asked.insert((ix, line.clone())) {
                    continue;
                }
                let default = usize::try_from(t.default).ok().filter(|&d| d < t.actions.len());
                self.trig.asks.push(Ask {
                    trigger: ti,
                    name: t.name.clone(),
                    tab,
                    pane,
                    line,
                    caps,
                    actions: t.actions.clone(),
                    default,
                    deadline: now + Duration::from_secs(t.timeout.max(1)),
                    timeout: Duration::from_secs(t.timeout.max(1)),
                });
                self.sync_ask_flags();
                cx.notify();
            } else {
                let what: Vec<String> = t.actions.iter().map(|a| self.run_action(ti, ix, pane, a, &line, &caps, window, cx)).collect();
                self.trig_record(&t.name, ix, &line, what.join(", "), "automatically".into(), cx);
            }
        }
        self.trig_runs_done(cx);
        // Asks whose time ran out: the default, or nothing.
        let due: Vec<usize> = (0..self.trig.asks.len()).filter(|&i| self.trig.asks[i].deadline <= Instant::now()).collect();
        for i in due.into_iter().rev() {
            let ask = self.trig.asks.remove(i);
            let secs = ask.timeout.as_secs();
            self.finish_ask(ask, None, format!("no choice in {secs}s"), window, cx);
        }
    }

    /// Lines a terminal showed since the last look: what streamed past (plain output), and
    /// rows on screen not seen before (programs that draw in place). Each text once while
    /// it's on screen; a terminal seen for the first time counts what's already there as old.
    fn new_lines(&mut self, view: &gpui::Entity<t1k_terminal::TerminalView>, cx: &mut Context<Self>) -> Vec<String> {
        let term = &view.read(cx).terminal;
        let streamed = term.take_lines();
        let screen = term.last_lines(80);
        let first = !self.trig.seen.contains_key(&view.entity_id());
        let seen = self.trig.seen.entry(view.entity_id()).or_default();
        let now = Instant::now();
        let on_screen: HashSet<&str> = screen.iter().map(|l| l.trim()).collect();
        // Forget lines that left the screen a while ago, so the same text later fires again.
        seen.retain(|t, at| on_screen.contains(t.as_str()) || at.elapsed() < Duration::from_secs(30));
        let mut out = Vec::new();
        // What streamed past is always new; the screen's rows only after the first look.
        let n = streamed.len();
        for (i, l) in streamed.into_iter().chain(screen).enumerate() {
            let t = l.trim().to_string();
            if t.is_empty() {
                continue;
            }
            let fresh = !seen.contains_key(&t);
            seen.insert(t.clone(), now);
            if fresh && (i < n || !first) {
                out.push(t);
            }
        }
        out
    }

    /// Asks count as needs-you: the tab holding one says so.
    fn sync_ask_flags(&mut self) {
        let asking: HashSet<String> = self.trig.asks.iter().map(|a| a.tab.clone()).collect();
        for t in &mut self.tabs {
            t.trigger_ask = asking.contains(&t.id);
        }
    }

    /// Answer the ask at `i` with the action `choice` (None: its default, or nothing if
    /// `dismiss`).
    pub fn answer_ask(&mut self, i: usize, choice: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if i >= self.trig.asks.len() {
            return;
        }
        let ask = self.trig.asks.remove(i);
        let why = if choice.is_some() { "you chose".to_string() } else { "dismissed".to_string() };
        match choice {
            Some(c) => self.finish_ask(ask, Some(c), why, window, cx),
            None => {
                self.sync_ask_flags();
                self.trig_record(&ask.name, self.tabs.iter().position(|t| t.id == ask.tab).unwrap_or(0), &ask.line, "did nothing".into(), why, cx);
            }
        }
    }

    fn finish_ask(&mut self, ask: Ask, choice: Option<usize>, why: String, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_ask_flags();
        let Some(ix) = self.tabs.iter().position(|t| t.id == ask.tab) else { return };
        let pick = choice.or(ask.default);
        let what = match pick.and_then(|p| ask.actions.get(p)).cloned() {
            Some(a) => self.run_action(ask.trigger, ix, ask.pane, &a, &ask.line, &ask.caps, window, cx),
            None => "did nothing".into(),
        };
        self.trig_record(&ask.name, ix, &ask.line, what, why, cx);
    }

    fn trig_record(&mut self, name: &str, ix: usize, line: &str, what: String, why: String, cx: &mut Context<Self>) {
        self.trig.next_id += 1;
        let id = self.trig.next_id;
        let started = self.trig.started.take();
        let run = started.as_ref().map(|(r, _)| r.clone());
        let text = match &run {
            Some(r) => format!("Trigger \u{201c}{name}\u{201d} running {}\u{2026}", short_cmd(&r.command)),
            None => format!("Trigger \u{201c}{name}\u{201d} {what} ({why})"),
        };
        if let Some(t) = self.tabs.get(ix) {
            let tab = t.id.clone();
            if let Some((_, rx)) = started {
                self.trig.running.push((tab.clone(), id, rx));
            }
            let log = self.trig.log.entry(tab).or_default();
            log.push(Fired { id, at: SystemTime::now(), name: name.to_string(), what, why, line: line.to_string(), run });
            if log.len() > 50 {
                log.remove(0);
            }
        }
        self.trig_toast(text, cx);
        self.trig.notice_for = Some(id);
    }

    /// Background commands that finished: their entry, the notice if it's still about them,
    /// a banner and an unread tab when one failed.
    fn trig_runs_done(&mut self, cx: &mut Context<Self>) {
        let mut done = Vec::new();
        self.trig.running.retain(|(tab, id, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((tab.clone(), *id, r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (tab, id, (end, output, took)) in done {
            let Some(f) = self.trig.log.get_mut(&tab).and_then(|l| l.iter_mut().find(|f| f.id == id)) else { continue };
            let Some(run) = f.run.as_mut() else { continue };
            run.end = end.clone();
            run.took = Some(took);
            run.output = output.clone();
            let (name, prog) = (f.name.clone(), short_cmd(&run.command));
            let secs = took.as_secs_f32();
            let took_s = if secs < 60. { format!("{secs:.1}s") } else { crate::usage::span(secs as i64) };
            let summary = match &end {
                RunEnd::Exit(0) => format!("ran {prog}: exit 0 \u{b7} {took_s}"),
                RunEnd::Exit(c) => format!("failed (exit {c}) \u{b7} {took_s}"),
                RunEnd::Stopped(m) => format!("stopped after {m}m"),
                _ => format!("failed (stopped by a signal) \u{b7} {took_s}"),
            };
            f.what = summary.clone();
            if self.trig.notice_for == Some(id) && self.notice.is_some() {
                self.notice = Some((format!("Trigger \u{201c}{name}\u{201d} {summary}"), Instant::now()));
                self.notice_brief = !end.failed();
            }
            if end.failed() {
                self.trig.open = Some(id);
                if self.config.notify.error {
                    let title = self.tabs.iter().find(|t| t.id == tab).map(|t| t.title()).unwrap_or_default();
                    let last = output.last().cloned().unwrap_or_default();
                    crate::platform::notify_tab(&tab, &format!("Trigger \u{201c}{name}\u{201d} failed"), &format!("{prog} {} in {title}", match end {
                        RunEnd::Exit(c) => format!("exited {c}"),
                        RunEnd::Stopped(m) => format!("was stopped after {m}m"),
                        _ => "was stopped".into(),
                    }), &last);
                }
                // In a tab you're not looking at: it turns unread, so you find it.
                if self.tabs.get(self.active).is_none_or(|t| t.id != tab) {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                        t.unread = true;
                    }
                }
            }
            cx.notify();
        }
    }

    fn trig_toast(&mut self, text: String, cx: &mut Context<Self>) {
        self.notice = Some((text, Instant::now()));
        self.notice_undo = false;
        self.notice_brief = true;
        cx.notify();
    }

    /// Run one action in tab `ix`; says what it did ("sent \u{201c}y ⏎\u{201d}").
    #[allow(clippy::too_many_arguments)]
    fn run_action(&mut self, ti: usize, ix: usize, pane: PaneId, a: &ActionCfg, line: &str, caps: &[String], window: &mut Window, cx: &mut Context<Self>) -> String {
        let arg = fill(&a.arg, caps);
        let matched = caps.first().cloned().unwrap_or_else(|| line.to_string());
        let view = self.pane_view(ix, pane);
        let tab_id = self.tabs[ix].id.clone();
        let title = self.tabs[ix].title();
        match a.kind.as_str() {
            "notify" => {
                let body = if arg.trim().is_empty() { line.to_string() } else { arg };
                crate::platform::notify_tab(&tab_id, "T1K · Trigger", &title, &body);
                "notified you".into()
            }
            "unread" => {
                self.tabs[ix].unread = true;
                "marked the tab unread".into()
            }
            "needs" => {
                self.tabs[ix].reminder_due = true;
                "marked the tab as needing you".into()
            }
            "send" | "prompt" => {
                let Some(v) = view else { return "found no terminal".into() };
                // Its own output for the next moment is the trigger's doing, not news.
                self.trig.typed.insert((ti, tab_id), Instant::now());
                let text = arg.replace("\\n", "\n");
                let enter = a.kind == "prompt" || text.ends_with('\n');
                let body = text.trim_end_matches('\n').to_string();
                v.read(cx).terminal.paste(&body);
                if enter {
                    // Enter a moment later, so agents don't take it as part of a paste.
                    cx.spawn(async move |_, cx| {
                        cx.background_executor().timer(Duration::from_millis(120)).await;
                        let _ = v.update(cx, |v, _| v.terminal.write(&b"\r"[..]));
                    })
                    .detach();
                }
                format!("sent \u{201c}{}{}\u{201d}", body.chars().take(40).collect::<String>(), if enter { " ⏎" } else { "" })
            }
            // A pane below the tab's terminal runs it where you can see it (and answer it: a
            // login, a password), leaving whatever the tab runs alone. It closes when the
            // command succeeds; on failure it stays, so you can see why.
            "pane" => {
                if arg.trim().is_empty() {
                    return "had no command to run".into();
                }
                if self.tabs[ix].panes.count() >= crate::split::MAX_PANES {
                    return "couldn't open a pane (this tab has four)".into();
                }
                let before: Vec<PaneId> = self.tabs[ix].panes.layout.leaves();
                self.open_split(ix, true, window, cx);
                let Some(id) = self.tabs[ix].panes.layout.leaves().into_iter().find(|p| !before.contains(p)) else { return "couldn't open a pane".into() };
                let Some(v) = self.pane_view(ix, id) else { return "couldn't open a pane".into() };
                self.trig.typed.insert((ti, tab_id), Instant::now());
                // Typed once the new shell is listening.
                let line = format!("{} && exit\r", arg.trim());
                cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(Duration::from_millis(400)).await;
                    let _ = v.update(cx, |v, _| v.terminal.write(line.into_bytes()));
                })
                .detach();
                format!("opened a pane running {}", arg.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            }
            // In the background: kept to its last lines, stopped past the trigger's limit;
            // how it ended updates the entry when it does (canvas v3-ba).
            "run" => {
                if arg.trim().is_empty() {
                    return "had no command to run".into();
                }
                let cwd = self.tabs[ix].cwd.clone().unwrap_or_else(|| std::env::var_os("HOME").map(Into::into).unwrap_or_default());
                let env: Vec<(String, String)> = caps.iter().enumerate().map(|(i, c)| (format!("T1K_MATCH_{i}"), c.clone())).collect();
                let limit = Duration::from_secs(self.config.triggers.get(ti).map_or(10, |t| t.run_limit.max(1)) * 60);
                let rx = run_in_background(arg.clone(), cwd, env, limit);
                self.trig.started = Some((Run { command: arg.clone(), end: RunEnd::Running, took: None, output: Vec::new() }, rx));
                format!("running {}", short_cmd(&arg))
            }
            "open" => {
                let url = if arg.trim().is_empty() { matched } else { arg };
                cx.open_url(&url);
                "opened the link".into()
            }
            "copy" => {
                let text = if arg.trim().is_empty() { matched } else { arg };
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                "copied the match".into()
            }
            "highlight" => {
                if let Some(v) = view {
                    v.update(cx, |v, cx| v.find_text(&matched, cx));
                }
                "highlighted the match".into()
            }
            "remind" => {
                let mins = arg.trim().parse::<u64>().unwrap_or(15).max(1);
                self.set_reminder(ix, Some(SystemTime::now() + Duration::from_secs(mins * 60)), cx);
                format!("set a reminder in {mins}m")
            }
            "front" => {
                cx.activate(true);
                window.activate_window();
                self.activate(ix, window, cx);
                "brought the tab forward".into()
            }
            k => format!("skipped an unknown action ({k})"),
        }
    }

    /// ⌃1–⌃9 pick a button of the ask on the active tab, ⌃0 dismisses it.
    pub fn ask_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        if !ks.modifiers.control || ks.modifiers.platform || ks.modifiers.alt || !matches!(self.overlay, crate::overlay::Overlay::None) {
            return;
        }
        let Some(n) = ks.key.parse::<usize>().ok() else { return };
        let Some(id) = self.tabs.get(self.active).map(|t| t.id.clone()) else { return };
        let Some(i) = self.trig.asks.iter().position(|a| a.tab == id) else { return };
        if n == 0 {
            self.answer_ask(i, None, window, cx);
        } else if n <= self.trig.asks[i].actions.len() {
            self.answer_ask(i, Some(n - 1), window, cx);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    pub fn toggle_triggers_paused(&mut self, cx: &mut Context<Self>) {
        let v = !self.config.triggers_paused;
        crate::config::set_value(&["triggers_paused"], v.into());
        self.config.triggers_paused = v;
        if v {
            self.trig.asks.clear();
            self.sync_ask_flags();
        }
        cx.notify();
    }
}

// ---------- the ask bar ----------

/// Under the terminal while the active tab has an ask: what matched, the buttons (⌃1…), and
/// a bar draining toward the default.
pub fn ask_bar(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let id = ws.tabs.get(ws.active)?.id.clone();
    let mine: Vec<usize> = (0..ws.trig.asks.len()).filter(|&i| ws.trig.asks[i].tab == id).collect();
    let &i = mine.first()?;
    let a = &ws.trig.asks[i];
    let ui = ws.ui;
    let left = a.deadline.saturating_duration_since(Instant::now());
    let frac = (left.as_secs_f32() / a.timeout.as_secs_f32()).clamp(0., 1.);
    let default = a.default.and_then(|d| a.actions.get(d)).map(action_label);
    let countdown = match &default {
        Some(d) => format!("{d} in {}s", left.as_secs() + 1),
        None => format!("Nothing happens in {}s", left.as_secs() + 1),
    };
    let mut buttons = div().flex().flex_wrap().gap(px(6.));
    for (n, act) in a.actions.iter().enumerate().take(9) {
        let primary = a.default == Some(n);
        buttons = buttons.child(
            div()
                .id(SharedString::from(format!("ask-{n}")))
                .h(px(24.))
                .px(px(9.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .cursor_pointer()
                .when(primary, |d| d.bg(ui.status_needs).text_color(t1k_themes::on_hsla(ui.status_needs)).font_weight(FontWeight::MEDIUM))
                .when(!primary, |d| d.bg(ui.bg_control).text_color(ui.text_primary))
                .hover(|d| d.opacity(0.85))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.answer_ask(i, Some(n), window, cx)))
                .child(action_label(act))
                .child(div().font_family("JetBrains Mono").text_size(px(10.5)).opacity(0.7).child(format!("⌃{}", n + 1))),
        );
    }
    buttons = buttons.child(
        div()
            .id("ask-dismiss")
            .h(px(24.))
            .px(px(9.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(ui.text_secondary)
            .cursor_pointer()
            .hover(move |d| d.bg(ui.bg_hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.answer_ask(i, None, window, cx)))
            .child("Do nothing")
            .child(div().font_family("JetBrains Mono").text_size(px(10.5)).opacity(0.7).child("⌃0")),
    );
    let more = mine.len() - 1;
    Some(
        div()
            .id("ask-bar")
            .flex_none()
            .relative()
            .px(px(14.))
            .pt(px(10.))
            .pb(px(10.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .bg(ui.bg_raised)
            .border_t_1()
            .border_color(ui.status_needs.opacity(0.6))
            // The time left to the default, draining.
            .child(div().absolute().top_0().left_0().h(px(2.)).w(gpui::relative(frac)).bg(ui.status_needs))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_none().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(a.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().font_family("JetBrains Mono").text_size(px(11.5)).text_color(ui.text_secondary).child(a.line.clone()))
                    .when(more > 0, |d| d.child(div().flex_none().text_size(px(11.5)).text_color(ui.text_tertiary).child(format!("+{more} more"))))
                    .child(div().flex_none().font_family("JetBrains Mono").text_size(px(11.)).text_color(ui.status_needs).child(countdown)),
            )
            .child(buttons)
            .into_any_element(),
    )
}

/// Seconds left on the tab's ask, for its row ("42s").
pub fn ask_left(ws: &Workspace, tab: &str) -> Option<String> {
    let a = ws.trig.asks.iter().find(|a| a.tab == tab)?;
    Some(format!("{}s", a.deadline.saturating_duration_since(Instant::now()).as_secs() + 1))
}

/// The ready-made triggers Settings offers to start from (added switched off).
pub fn starters() -> Vec<TriggerCfg> {
    let act = |kind: &str, arg: &str| ActionCfg { kind: kind.into(), arg: arg.into(), label: String::new() };
    vec![
        TriggerCfg {
            name: "Approve safe commands".into(),
            enabled: false,
            when: "agent".into(),
            event: "needs".into(),
            pattern: r"Running (ls|cat|git (status|diff|log)|cargo (check|test)|npm test)\b".into(),
            mode: "ask".into(),
            actions: vec![ActionCfg { kind: "send".into(), arg: "1".into(), label: "Approve".into() }, ActionCfg { kind: "send".into(), arg: "3".into(), label: "Deny".into() }],
            default: 1,
            timeout: 60,
            ..Default::default()
        },
        TriggerCfg {
            name: "Tests fail".into(),
            enabled: false,
            pattern: r"^(FAIL|FAILED|test result: FAILED)\b(.*)".into(),
            actions: vec![act("notify", "Tests failed$2"), act("needs", "")],
            ..Default::default()
        },
        TriggerCfg {
            name: "PR link printed".into(),
            enabled: false,
            pattern: r"(https://github\.com/[\w.-]+/[\w.-]+/pull/\d+)".into(),
            mode: "ask".into(),
            actions: vec![act("open", "$1"), act("copy", "$1")],
            default: -1,
            timeout: 20,
            ..Default::default()
        },
        TriggerCfg {
            name: "Password prompt".into(),
            enabled: false,
            pattern: r"(?i)(\[sudo\] password|password:\s*$|passphrase)".into(),
            actions: vec![act("front", ""), act("notify", "A tab is asking for a password")],
            cooldown: 30,
            ..Default::default()
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_groups_and_names_actions() {
        let caps = vec!["FAIL a.rs".to_string(), "FAIL".into(), " a.rs".into()];
        assert_eq!(fill("Tests $1:$2 ($0)", &caps), "Tests FAIL: a.rs (FAIL a.rs)");
        assert_eq!(action_label(&ActionCfg { kind: "send".into(), arg: "y\\n".into(), label: String::new() }), "Send \u{201c}y ⏎\u{201d}");
        assert!(compile("(unclosed").is_err());
        assert!(compile("").unwrap().is_none());
        for s in starters() {
            assert!(compile(&s.pattern).is_ok(), "{}", s.name);
        }
    }
}
