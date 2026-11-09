//! Per-tab agent state: status machine plus the "agent card" fields (goal, now, recap, links).

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use crate::hooks::{describe_tool, Hook};
use crate::outputs::{classify, Kind, Output};
use crate::transcript::{Entry, Tail};

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Working,
    NeedsInput { reason: String },
    Done,
    Error { message: String },
    Ended,
}

impl Status {
    /// Sort key within a group: lower is more urgent.
    pub fn urgency(&self, unread: bool) -> u8 {
        match self {
            Status::NeedsInput { .. } => 0,
            Status::Error { .. } => 1,
            Status::Done if unread => 2,
            Status::Working => 3,
            Status::Done => 4,
            Status::Idle => 5,
            Status::Ended => 6,
        }
    }

    /// States where a person has to act: counted by the Needs-you queue.
    pub fn needs_you(&self) -> bool {
        matches!(self, Status::NeedsInput { .. } | Status::Error { .. })
    }
}

pub struct AgentState {
    pub session_id: String,
    /// Which coding agent this session is (Claude Code, Codex, Antigravity...).
    pub kind: crate::agents::AgentKind,
    pub status: Status,
    /// When the current status began.
    pub since: Instant,
    /// Claude's title for the session (`ai-title`, or the user's `/rename`).
    pub goal: Option<String>,
    /// First prompt, as a fallback goal until Claude names the session.
    pub first_prompt: Option<String>,
    /// The first and latest prompts in full (capped), for the helper's title and recap.
    pub first_prompt_full: Option<String>,
    pub last_prompt: Option<String>,
    /// Current step while working: "Editing src/auth.rs".
    pub now: Option<String>,
    /// Claude's latest recap (`away_summary`).
    pub recap: Option<String>,
    pub branch: Option<String>,
    pub prs: Vec<(u64, String)>,
    /// Everything the session produced, newest last (deduped by target).
    pub outputs: Vec<Output>,
    /// Tokens in context as of the latest reply, and the model that answered.
    pub context_tokens: Option<u64>,
    pub model: Option<String>,
    /// The session's `/color` (red, blue, green, yellow, purple, orange, pink, cyan).
    pub color: Option<String>,
    /// The context window as the agent's status line reports it: (percent, window size).
    pub context_reported: Option<(f64, Option<u64>)>,
    custom_title: bool,
    tail: Option<Tail>,
    /// Artifact publish calls waiting for their result, by tool call id.
    artifact_calls: std::collections::HashMap<String, ArtifactCall>,
    /// When the transcript line being applied was written (outputs keep their real time).
    stamp: Option<SystemTime>,
    /// Outputs from the part of the transcript attaching skipped, read on a thread.
    history: Option<std::sync::mpsc::Receiver<Vec<Entry>>>,
}

impl AgentState {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            kind: crate::agents::AgentKind::Claude,
            status: Status::Idle,
            since: Instant::now(),
            goal: None,
            first_prompt: None,
            first_prompt_full: None,
            last_prompt: None,
            now: None,
            recap: None,
            branch: None,
            prs: Vec::new(),
            outputs: Vec::new(),
            context_tokens: None,
            model: None,
            color: None,
            context_reported: None,
            custom_title: false,
            tail: None,
            artifact_calls: Default::default(),
            stamp: None,
            history: None,
        }
    }

    pub fn add_output(&mut self, kind: Kind, target: String, label: String) {
        self.add_output_at(kind, target, label, SystemTime::now());
    }

    fn add_output_at(&mut self, kind: Kind, target: String, label: String, at: SystemTime) {
        // Mentioned again: it moves to newest, keeping what a publish call said about it.
        let old = self.outputs.iter().position(|o| o.target == target).map(|i| self.outputs.remove(i));
        let (label, icon, detail) = match old {
            Some(o) if o.icon.is_some() => (o.label, o.icon, o.detail),
            _ => (label, None, None),
        };
        self.outputs.push(Output { kind, target, label, at, icon, detail });
    }

    /// An Artifact publish call's result named its page: give that output a title, an icon
    /// and the description instead of "Artifact HLA68hSS" (canvas v3-ae).
    fn artifact_published(&mut self, url: &str, call: ArtifactCall) {
        let Some((Kind::Artifact, target, _)) = classify(url) else { return };
        if !self.outputs.iter().any(|o| o.target == target) {
            let at = self.stamp.unwrap_or_else(SystemTime::now);
            self.add_output_at(Kind::Artifact, target.clone(), "Artifact".into(), at);
        }
        let Some(o) = self.outputs.iter_mut().find(|o| o.target == target) else { return };
        // Updates to the same page often say less than the first publish; keep what's known.
        let title = call.title.or_else(|| call.file.as_deref().and_then(html_title));
        if let Some(t) = title.or_else(|| (o.icon.is_none()).then(|| call.description.as_deref().map(first_words)).flatten()) {
            o.label = t;
        }
        if let Some(i) = call.icon.as_deref() {
            o.icon = Some(crate::outputs::artifact_icon(i).to_string());
        } else if o.icon.is_none() {
            o.icon = Some("page".into());
        }
        if call.description.is_some() {
            o.detail = call.description;
        }
    }

    fn add_url(&mut self, url: &str) {
        if let Some((kind, target, label)) = classify(url) {
            let at = self.stamp.unwrap_or_else(SystemTime::now);
            self.add_output_at(kind, target, label, at);
        }
    }

    /// A session from a previous run: ended, with its transcript attached so the title, recap
    /// and outputs come back.
    pub fn restored(session_id: String, transcript: PathBuf) -> Self {
        let mut a = Self::new(session_id);
        a.status = Status::Ended;
        a.tail = Some(Tail::new(transcript));
        a
    }

    fn set_status(&mut self, s: Status) {
        if self.status != s {
            self.status = s;
            self.since = Instant::now();
        }
    }

    /// Share of the context window in use, 0-100. The window is 200k tokens unless the model
    /// says 1M or usage has already passed 200k (then the session must be on a 1M window).
    pub fn context_percent(&self) -> Option<u32> {
        if let Some((p, _)) = self.context_reported {
            return Some(p.round().clamp(0., 100.) as u32);
        }
        let used = self.context_tokens?;
        Some(((used * 100) / self.context_window()?).min(100) as u32)
    }

    /// The model's context window in tokens (1M for "[1m]" models or past 200k, else 200k).
    pub fn context_window(&self) -> Option<u64> {
        if let Some((_, Some(size))) = self.context_reported {
            return Some(size);
        }
        let used = self.context_tokens?;
        let big = self.model.as_deref().is_some_and(|m| m.contains("1m")) || used > 200_000;
        Some(if big { 1_000_000 } else { 200_000 })
    }

    /// Number of distinct files the session edited.
    pub fn files_edited(&self) -> usize {
        self.outputs.iter().filter(|o| o.kind == Kind::File).count()
    }

    /// Best title: Claude's or the user's session title, else the first prompt.
    pub fn title(&self) -> Option<String> {
        self.goal.clone().or_else(|| self.first_prompt.clone())
    }

    /// Apply a hook. Returns true when the turn just finished (Stop), so the UI can mark unread.
    pub fn apply_hook(&mut self, hook: Hook) -> bool {
        match hook {
            Hook::SessionStart { transcript_path, .. } => {
                if let Some(p) = transcript_path {
                    if self.tail.as_ref().map(|t| &t.path) != Some(&PathBuf::from(&p)) {
                        self.tail = Some(Tail::new(p.into()));
                    }
                }
                if self.status == Status::Ended {
                    self.set_status(Status::Idle);
                }
            }
            Hook::UserPromptSubmit { prompt } => {
                let full: String = prompt.trim().chars().take(2000).collect();
                if !full.is_empty() {
                    self.first_prompt_full.get_or_insert_with(|| full.clone());
                    self.last_prompt = Some(full);
                }
                if self.first_prompt.is_none() {
                    let p = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                    if !p.is_empty() {
                        self.first_prompt = Some(p.chars().take(80).collect());
                    }
                }
                self.now = None;
                self.set_status(Status::Working);
            }
            Hook::PreToolUse { tool, input } => {
                if matches!(tool.as_str(), "Edit" | "MultiEdit" | "Write" | "NotebookEdit") {
                    let path = input.get("file_path").or_else(|| input.get("notebook_path")).and_then(|v| v.as_str());
                    if let Some(p) = path {
                        let name = p.rsplit('/').next().unwrap_or(p).to_string();
                        self.add_output(Kind::File, p.to_string(), name);
                    }
                }
                self.now = Some(describe_tool(&tool, &input));
                self.set_status(Status::Working);
            }
            Hook::PostToolUse { .. } => self.set_status(Status::Working),
            Hook::Notification { message } => {
                // "Claude is waiting for your input" fires after a turn already ended; keep Done.
                let idle_ping = message.contains("waiting for your input");
                if !(idle_ping && matches!(self.status, Status::Done | Status::Idle)) {
                    self.set_status(Status::NeedsInput { reason: message });
                }
            }
            Hook::Stop => {
                self.now = None;
                let was_working = !matches!(self.status, Status::Done | Status::Idle);
                self.set_status(Status::Done);
                return was_working;
            }
            Hook::SessionEnd => {
                self.now = None;
                self.set_status(Status::Ended);
            }
            Hook::Other(_) => {}
        }
        false
    }

    /// Read new transcript entries. Returns true if anything visible changed.
    pub fn poll_transcript(&mut self) -> bool {
        let Some(tail) = self.tail.as_mut() else { return false };
        let entries = tail.poll();
        // Attaching read only the transcript's end: read the rest's outputs on a thread.
        if let Some(upto) = tail.skipped.take() {
            let path = tail.path.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.history = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(crate::transcript::history_outputs(&path, upto));
            });
        }
        let mut changed = !entries.is_empty();
        for e in entries {
            self.apply_entry(e);
        }
        self.stamp = None;
        if let Some(older) = self.history.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.history = None;
            self.merge_history(older);
            changed = true;
        }
        changed
    }

    /// Outputs from earlier in the transcript go before the ones already listed; a later
    /// mention of the same output keeps its place and details.
    fn merge_history(&mut self, older: Vec<Entry>) {
        let newer = std::mem::take(&mut self.outputs);
        let newer_prs = std::mem::take(&mut self.prs);
        let calls = std::mem::take(&mut self.artifact_calls);
        for e in older {
            self.apply_entry(e);
        }
        self.stamp = None;
        self.artifact_calls = calls;
        for o in newer {
            self.outputs.retain(|x| x.target != o.target);
            self.outputs.push(o);
        }
        for p in newer_prs {
            self.prs.retain(|x| x.1 != p.1);
            self.prs.push(p);
        }
    }

    fn apply_entry(&mut self, e: Entry) {
        match e {
            Entry::Stamp(t) => self.stamp = Some(t),
            Entry::FileEdit(p) => {
                let name = p.rsplit('/').next().unwrap_or(&p).to_string();
                let at = self.stamp.unwrap_or_else(SystemTime::now);
                self.add_output_at(Kind::File, p, name, at);
            }
            Entry::AiTitle(t) if !self.custom_title => self.goal = Some(t),
            Entry::AiTitle(_) => {}
            Entry::CustomTitle(t) => {
                self.custom_title = true;
                self.goal = Some(t);
            }
            Entry::AwaySummary(s) => self.recap = Some(s),
            Entry::Color(c) => self.color = c,
            Entry::ApiError(m) => {
                if self.status == Status::Working {
                    self.set_status(Status::Error { message: m });
                }
            }
            Entry::PrLink { url, number } => {
                self.add_url(&url);
                if !self.prs.iter().any(|(_, u)| *u == url) {
                    self.prs.push((number, url));
                }
            }
            Entry::Url(u) => {
                if let Some((Kind::Pr, canon, _)) = classify(&u) {
                    if let Some(n) = canon.rsplit('/').next().and_then(|n| n.parse().ok()) {
                        if !self.prs.iter().any(|(_, x)| *x == canon) {
                            self.prs.push((n, canon));
                        }
                    }
                }
                self.add_url(&u);
            }
            Entry::ArtifactCall { tool_id, icon, description, title, file } => {
                self.artifact_calls.insert(tool_id, ArtifactCall { icon, description, title, file });
            }
            Entry::ArtifactResult { tool_id, url } => {
                if let Some(call) = self.artifact_calls.remove(&tool_id) {
                    self.artifact_published(&url, call);
                }
            }
            Entry::Branch(b) => self.branch = Some(b),
            Entry::Usage { context_tokens, model } => {
                self.context_tokens = Some(context_tokens);
                if model.is_some() {
                    self.model = model;
                }
            }
            Entry::TurnDone { .. } => {}
            // Esc on a permission prompt or mid-turn: Claude is back at its prompt.
            Entry::Interrupted => {
                if matches!(self.status, Status::NeedsInput { .. } | Status::Working | Status::Error { .. }) {
                    self.now = None;
                    self.set_status(Status::Idle);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn artifacts_get_a_title_and_an_icon() {
        let dir = std::env::temp_dir().join(format!("t1k-art-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let html = dir.join("q3.html");
        std::fs::write(&html, "<html><head><title>Q3 retention</title></head></html>").unwrap();
        let t = dir.join("t.jsonl");
        let lines = [
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Artifact","input":{"file_path": html, "icon":"deck","description":"Board deck for Q3"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Published the Artifact at https://claude.ai/artifact/AAAA1111 (version 1)."}]}}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Old one: https://claude.ai/artifact/LINK99"}]}}),
        ];
        std::fs::write(&t, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
        let mut a = AgentState::restored("s".into(), t);
        a.poll_transcript();
        let pub_ = a.outputs.iter().find(|o| o.target.ends_with("AAAA1111")).unwrap();
        assert_eq!(pub_.label, "Q3 retention");
        assert_eq!(pub_.icon.as_deref(), Some("slides"));
        assert_eq!(pub_.detail.as_deref(), Some("Board deck for Q3"));
        let link = a.outputs.iter().find(|o| o.target.ends_with("LINK99")).unwrap();
        assert_eq!((link.label.as_str(), link.icon.as_deref()), ("Artifact", None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn status_flow() {
        let mut a = AgentState::new("s".into());
        a.apply_hook(Hook::UserPromptSubmit { prompt: "fix the login bug\nmore".into() });
        assert_eq!(a.status, Status::Working);
        assert_eq!(a.title().as_deref(), Some("fix the login bug"));
        a.apply_hook(Hook::PreToolUse { tool: "Read".into(), input: json!({"file_path":"/x/src/a.rs"}) });
        assert_eq!(a.now.as_deref(), Some("Reading src/a.rs"));
        a.apply_hook(Hook::Notification { message: "Claude needs your permission to use Bash".into() });
        assert!(a.status.needs_you());
        a.apply_hook(Hook::PostToolUse { tool: "Bash".into() });
        assert_eq!(a.status, Status::Working);
        assert!(a.apply_hook(Hook::Stop));
        assert_eq!(a.status, Status::Done);
        // The idle reminder after a finished turn doesn't turn it into NeedsInput.
        a.apply_hook(Hook::Notification { message: "Claude is waiting for your input".into() });
        assert_eq!(a.status, Status::Done);
    }

    #[test]
    fn interrupt_clears_needs_you() {
        let line = br#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;
        let entries = crate::transcript::parse_line(line).unwrap();
        assert!(entries.iter().any(|e| matches!(e, crate::transcript::Entry::Interrupted)));
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ArtifactCall {
    icon: Option<String>,
    description: Option<String>,
    title: Option<String>,
    file: Option<String>,
}

/// The `<title>` of a published HTML file, if it's still there.
fn html_title(path: &str) -> Option<String> {
    if !path.ends_with(".html") {
        return None;
    }
    use std::io::Read;
    let mut buf = vec![0; 16 * 1024];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let lower = head.to_ascii_lowercase();
    let start = lower.find("<title>")? + 7;
    let end = start + lower[start..].find("</title>")?;
    let t = head[start..end].trim();
    (!t.is_empty()).then(|| t.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">"))
}

/// The first few words of a description, as a stand-in title.
fn first_words(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().take(6).collect();
    let t = words.join(" ");
    t.trim_end_matches(['.', ',', ':', ';']).to_string()
}
