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
    pub status: Status,
    /// When the current status began.
    pub since: Instant,
    /// Claude's title for the session (`ai-title`, or the user's `/rename`).
    pub goal: Option<String>,
    /// First prompt, as a fallback goal until Claude names the session.
    pub first_prompt: Option<String>,
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
    custom_title: bool,
    tail: Option<Tail>,
}

impl AgentState {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            status: Status::Idle,
            since: Instant::now(),
            goal: None,
            first_prompt: None,
            now: None,
            recap: None,
            branch: None,
            prs: Vec::new(),
            outputs: Vec::new(),
            context_tokens: None,
            model: None,
            custom_title: false,
            tail: None,
        }
    }

    pub fn add_output(&mut self, kind: Kind, target: String, label: String) {
        self.outputs.retain(|o| o.target != target);
        self.outputs.push(Output { kind, target, label, at: SystemTime::now() });
    }

    fn add_url(&mut self, url: &str) {
        if let Some((kind, target, label)) = classify(url) {
            self.add_output(kind, target, label);
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
        let used = self.context_tokens?;
        let big = self.model.as_deref().is_some_and(|m| m.contains("1m")) || used > 200_000;
        let window = if big { 1_000_000 } else { 200_000 };
        Some(((used * 100) / window).min(100) as u32)
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
        let changed = !entries.is_empty();
        for e in entries {
            match e {
                Entry::AiTitle(t) if !self.custom_title => self.goal = Some(t),
                Entry::AiTitle(_) => {}
                Entry::CustomTitle(t) => {
                    self.custom_title = true;
                    self.goal = Some(t);
                }
                Entry::AwaySummary(s) => self.recap = Some(s),
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
                Entry::Branch(b) => self.branch = Some(b),
                Entry::Usage { context_tokens, model } => {
                    self.context_tokens = Some(context_tokens);
                    if model.is_some() {
                        self.model = model;
                    }
                }
                Entry::TurnDone { .. } => {}
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
