//! Tail a Claude Code transcript (`~/.claude/projects/<proj>/<session>.jsonl`).
//!
//! The format is undocumented and changes, so parsing is tolerant: unknown entry types are
//! skipped, and only lines that mention an interesting type are parsed at all.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// On attach, read at most this much from the end (transcripts can be hundreds of MB).
const ATTACH_WINDOW: u64 = 4 << 20;
/// Per poll, read at most this much new data.
const READ_CHUNK: u64 = 2 << 20;

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// An Artifact publish call: its icon word, description, title and source file.
    ArtifactCall { tool_id: String, icon: Option<String>, description: Option<String>, title: Option<String>, file: Option<String> },
    /// The result of a tool call, naming the artifact page it published.
    ArtifactResult { tool_id: String, url: String },
    /// The session's `/color` (None: reset).
    Color(Option<String>),
    AiTitle(String),
    CustomTitle(String),
    AwaySummary(String),
    ApiError(String),
    PrLink { url: String, number: u64 },
    Branch(String),
    /// A turn ended: how long it took, and how many background agents it left running.
    TurnDone { duration_ms: u64, background_agents: u64 },
    /// A turn started by itself (a background task reported, a scheduled wake-up fired).
    SelfStarted,
    /// A link worth listing as a session output (PR, artifact, doc, ticket, deploy).
    Url(String),
    /// Tokens in the context window as of an assistant reply (input + cache), and the model.
    Usage { context_tokens: u64, model: Option<String> },
    /// You pressed Esc (or rejected a tool): Claude fires no hook for that.
    Interrupted,
    /// A file Claude edited or wrote (Edit, Write, MultiEdit, NotebookEdit).
    FileEdit(String),
    /// When the line holding the entries after it was written.
    Stamp(SystemTime),
}

impl Entry {
    /// Entries that add to the session's outputs (the rest is status, titles and usage).
    fn is_output(&self) -> bool {
        matches!(
            self,
            Entry::Url(_) | Entry::PrLink { .. } | Entry::ArtifactCall { .. } | Entry::ArtifactResult { .. } | Entry::FileEdit(_) | Entry::Stamp(_)
        )
    }
}

pub struct Tail {
    pub path: PathBuf,
    offset: Option<u64>,
    partial: Vec<u8>,
    /// Attaching skipped the transcript before this offset: its outputs are still to be read
    /// (`history_outputs`). Taken once.
    pub skipped: Option<u64>,
}

impl Tail {
    pub fn new(path: PathBuf) -> Self {
        Self { path, offset: None, partial: Vec::new(), skipped: None }
    }

    /// Read whatever was appended since the last call.
    pub fn poll(&mut self) -> Vec<Entry> {
        let Ok(mut f) = File::open(&self.path) else { return Vec::new() };
        let Ok(len) = f.metadata().map(|m| m.len()) else { return Vec::new() };
        let (start, attach) = match self.offset {
            Some(o) if o <= len => (o, false),
            // First attach, or the file was rewritten: start near the end.
            _ => {
                self.partial.clear();
                let start = len.saturating_sub(ATTACH_WINDOW);
                self.skipped = (start > 0).then_some(start);
                (start, true)
            }
        };
        let end = len.min(start + if attach { ATTACH_WINDOW } else { READ_CHUNK });
        if end <= start {
            self.offset = Some(start);
            return Vec::new();
        }
        let mut buf = vec![0; (end - start) as usize];
        if f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut buf).is_err() {
            return Vec::new();
        }
        self.offset = Some(end);
        let mut data = std::mem::take(&mut self.partial);
        data.extend_from_slice(&buf);
        let mut lines: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
        let rest = lines.pop().unwrap_or_default().to_vec();
        // Attaching mid-file: the first line is probably cut.
        if attach && start > 0 && !lines.is_empty() {
            lines.remove(0);
        }
        let out = lines.into_iter().filter_map(parse_line).flatten().collect();
        self.partial = rest;
        out
    }
}

/// The outputs in the part of a transcript an attach skipped (`..upto`), oldest first, for a
/// background thread: transcripts can be hundreds of MB, and only lines that can hold an
/// output are parsed.
pub fn history_outputs(path: &Path, upto: u64) -> Vec<Entry> {
    let Ok(f) = File::open(path) else { return Vec::new() };
    let mut r = BufReader::with_capacity(1 << 20, f.take(upto));
    let mut out = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match r.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if !OUTPUT_HINTS.iter().any(|k| contains(&line, k.as_bytes())) {
            continue;
        }
        if let Some(es) = parse_line(line.strip_suffix(b"\n").unwrap_or(&line)) {
            out.extend(es.into_iter().filter(Entry::is_output));
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// What a line must contain to hold an output.
const OUTPUT_HINTS: &[&str] = &["https://", "pr-link", "\"file_path\"", "\"notebook_path\""];

/// RFC 3339 UTC time as Claude writes it ("2026-11-09T20:30:31.123Z").
pub(crate) fn parse_time(s: &str) -> Option<SystemTime> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    // Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hh * 3600 + mm * 60 + ss;
    let millis = s.get(20..23).and_then(|f| f.parse::<u64>().ok()).filter(|_| b.get(19) == Some(&b'.')).unwrap_or(0);
    (secs >= 0).then(|| UNIX_EPOCH + Duration::from_secs(secs as u64) + Duration::from_millis(millis))
}

/// Files an assistant message's tool calls edit or write.
fn file_edits(msg: &Value, out: &mut Vec<Entry>) {
    let Some(items) = msg.get("content").and_then(Value::as_array) else { return };
    for it in items {
        if it.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        if !matches!(it.get("name").and_then(Value::as_str), Some("Edit" | "MultiEdit" | "Write" | "NotebookEdit")) {
            continue;
        }
        let input = &it["input"];
        if let Some(p) = input.get("file_path").or_else(|| input.get("notebook_path")).and_then(Value::as_str) {
            out.push(Entry::FileEdit(p.to_string()));
        }
    }
}

const INTERESTING: &[&str] =
    &["\"file_path\"", "\"notebook_path\"", "\"ai-title\"", "\"custom-title\"", "\"agent-color\"", "\"Artifact\"", "away_summary", "api_error", "pr-link", "turn_duration", "\"turnOrigin\"", "\"gitBranch\"", "\"usage\"", "[Request interrupted by user"];

/// Text that can mention session outputs: Claude's replies, the user's prompts, and tool
/// results from tools like artifact publishing, but not file contents or shell output.
fn output_text(v: &Value, out: &mut String) {
    let kind = v.get("type").and_then(Value::as_str);
    if !matches!(kind, Some("assistant") | Some("user")) {
        return;
    }
    // Text Claude Code injects (a skill's instructions, command output) isn't the session's
    // work, and skill docs are full of example links.
    if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return;
    }
    if let Some(r) = v.get("toolUseResult").and_then(Value::as_object) {
        // File reads/edits/writes, and shell output (greps, test logs) mention URLs in passing.
        // PRs opened from the shell still arrive as `pr-link` entries.
        if ["file", "filePath", "originalFile", "stdout"].iter().any(|k| r.contains_key(*k)) {
            return;
        }
    }
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else { return };
    fn walk(c: &Value, out: &mut String) {
        match c {
            Value::String(s) => {
                out.push_str(s);
                out.push('\n');
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            Value::Object(o) => match o.get("type").and_then(Value::as_str) {
                Some("text") => walk(o.get("text").unwrap_or(&Value::Null), out),
                Some("tool_result") => walk(o.get("content").unwrap_or(&Value::Null), out),
                // tool_use inputs (code being written), thinking, images: skip.
                _ => {}
            },
            _ => {}
        }
    }
    walk(content, out);
}

/// Artifact tool calls in an assistant message: what each publish says about its page.
fn artifact_calls(msg: &Value, out: &mut Vec<Entry>) {
    let Some(items) = msg.get("content").and_then(Value::as_array) else { return };
    for it in items {
        if it.get("type").and_then(Value::as_str) != Some("tool_use") || it.get("name").and_then(Value::as_str) != Some("Artifact") {
            continue;
        }
        let input = &it["input"];
        let s = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
        // Reads, lists and deletes say nothing about a page.
        if !matches!(s("action").as_deref(), None | Some("publish")) {
            continue;
        }
        out.push(Entry::ArtifactCall {
            tool_id: it.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            icon: s("icon"),
            description: s("description"),
            title: s("title"),
            file: s("file_path"),
        });
    }
}

/// Tool results in a user message that name an artifact URL, by the call they answer.
fn artifact_results(msg: &Value, out: &mut Vec<Entry>) {
    let Some(items) = msg.get("content").and_then(Value::as_array) else { return };
    for it in items {
        if it.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(id) = it.get("tool_use_id").and_then(Value::as_str) else { continue };
        let mut text = String::new();
        let c = &it["content"];
        match c {
            Value::String(s) => text.push_str(s),
            Value::Array(a) => a.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).for_each(|t| text.push_str(t)),
            _ => {}
        }
        // The page it published is the first artifact link (a type's own link comes later).
        if let Some(url) = crate::outputs::urls_in(&text).into_iter().find(|u| matches!(crate::outputs::classify(u), Some((crate::outputs::Kind::Artifact, _, _)))) {
            out.push(Entry::ArtifactResult { tool_id: id.to_string(), url });
        }
    }
}

pub fn parse_line(line: &[u8]) -> Option<Vec<Entry>> {
    let s = std::str::from_utf8(line).ok()?;
    let has_url = s.contains("https://");
    if !has_url && !INTERESTING.iter().any(|k| s.contains(k)) {
        return None;
    }
    let v: Value = serde_json::from_str(s).ok()?;
    let urls: Vec<Entry> = if has_url {
        let mut text = String::new();
        output_text(&v, &mut text);
        crate::outputs::urls_in(&text).into_iter().filter(|u| crate::outputs::classify(u).is_some()).map(Entry::Url).collect()
    } else {
        Vec::new()
    };
    let str_of = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let mut out = Vec::new();
    match v.get("type").and_then(Value::as_str) {
        Some("ai-title") => out.extend(str_of("aiTitle").map(Entry::AiTitle)),
        Some("custom-title") => out.extend(str_of("customTitle").map(Entry::CustomTitle)),
        // `/color red`: the session's color, chosen to tell sessions apart.
        Some("agent-color") => out.push(Entry::Color(str_of("agentColor").filter(|c| !c.is_empty() && c != "default"))),
        Some("pr-link") => {
            if let Some(url) = str_of("prUrl") {
                let number = v.get("prNumber").and_then(Value::as_u64).unwrap_or(0);
                out.push(Entry::PrLink { url, number });
            }
        }
        // The main thread's replies carry usage; subagent (sidechain) turns don't count.
        Some("assistant") if v.get("isSidechain").and_then(Value::as_bool) != Some(true) => {
            let msg = &v["message"];
            artifact_calls(msg, &mut out);
            file_edits(msg, &mut out);
            if let Some(u) = msg.get("usage") {
                let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                let context_tokens = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
                if context_tokens > 0 {
                    out.push(Entry::Usage { context_tokens, model: msg.get("model").and_then(Value::as_str).map(str::to_string) });
                }
            }
        }
        Some("system") => match v.get("subtype").and_then(Value::as_str) {
            Some("away_summary") => out.extend(str_of("content").map(Entry::AwaySummary)),
            Some("api_error") => {
                let msg = v.get("error").map(|e| match e {
                    Value::String(s) => s.clone(),
                    other => other.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| other.to_string()),
                });
                out.push(Entry::ApiError(msg.unwrap_or_else(|| "API error".into())));
            }
            Some("turn_duration") => {
                out.push(Entry::TurnDone {
                    duration_ms: v.get("durationMs").and_then(Value::as_u64).unwrap_or(0),
                    background_agents: v.get("pendingBackgroundAgentCount").and_then(Value::as_u64).unwrap_or(0),
                })
            }
            _ => {}
        },
        _ => {}
    }
    if v.get("type").and_then(Value::as_str) == Some("user") {
        artifact_results(&v["message"], &mut out);
    }
    if v.get("type").and_then(Value::as_str) == Some("user") && v.get("turnOrigin").and_then(Value::as_str).is_some_and(|o| o != "user") {
        out.push(Entry::SelfStarted);
    }
    if v.get("type").and_then(Value::as_str) == Some("user") && s.contains("[Request interrupted by user") {
        out.push(Entry::Interrupted);
    }
    if let Some(b) = str_of("gitBranch").filter(|b| !b.is_empty() && b != "HEAD") {
        out.push(Entry::Branch(b));
    }
    out.extend(urls);
    if out.iter().any(|e| e.is_output()) {
        if let Some(t) = str_of("timestamp").as_deref().and_then(parse_time) {
            out.insert(0, Entry::Stamp(t));
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn times_and_file_edits() {
        let t = parse_time("2026-11-09T20:30:31.123Z").unwrap();
        assert_eq!(t.duration_since(UNIX_EPOCH).unwrap().as_millis(), 1_794_256_231_123);
        let l = br#"{"type":"assistant","timestamp":"1970-01-02T00:00:00.000Z","message":{"content":[{"type":"tool_use","id":"t","name":"Write","input":{"file_path":"/x/a.md","content":"hi"}},{"type":"tool_use","id":"u","name":"Read","input":{"file_path":"/x/b.md"}}]}}"#;
        assert_eq!(parse_line(l), Some(vec![Entry::Stamp(UNIX_EPOCH + Duration::from_secs(86400)), Entry::FileEdit("/x/a.md".into())]));
    }

    #[test]
    fn history_reads_what_attach_skipped() {
        let dir = std::env::temp_dir().join(format!("t1k-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        let mut f = File::create(&p).unwrap();
        writeln!(f, r#"{{"type":"pr-link","prNumber":1,"prUrl":"https://github.com/a/b/pull/1","timestamp":"2026-01-01T00:00:00Z"}}"#).unwrap();
        writeln!(f, r#"{{"type":"ai-title","aiTitle":"x"}}"#).unwrap();
        let upto = f.metadata().unwrap().len();
        writeln!(f, r#"{{"type":"pr-link","prNumber":2,"prUrl":"https://github.com/a/b/pull/2"}}"#).unwrap();
        let got = history_outputs(&p, upto);
        assert!(matches!(got.as_slice(), [Entry::Stamp(_), Entry::PrLink { number: 1, .. }]), "{got:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_known_entries() {
        let l = br#"{"type":"ai-title","aiTitle":"Fix login","sessionId":"x"}"#;
        assert_eq!(parse_line(l), Some(vec![Entry::AiTitle("Fix login".into())]));
        let l = br#"{"type":"system","subtype":"away_summary","content":"Done; next ship it.","gitBranch":"main"}"#;
        assert_eq!(
            parse_line(l),
            Some(vec![Entry::AwaySummary("Done; next ship it.".into()), Entry::Branch("main".into())])
        );
        let l = br#"{"type":"agent-color","agentColor":"red","sessionId":"x"}"#;
        assert_eq!(parse_line(l), Some(vec![Entry::Color(Some("red".into()))]));
        let l = br#"{"type":"pr-link","prNumber":23,"prUrl":"https://github.com/a/b/pull/23"}"#;
        assert_eq!(
            parse_line(l),
            Some(vec![Entry::PrLink { url: "https://github.com/a/b/pull/23".into(), number: 23 }])
        );
        assert_eq!(parse_line(br#"{"type":"assistant","message":{}}"#), None);
        let l = br#"{"type":"assistant","message":{"model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_read_input_tokens":27483,"cache_creation_input_tokens":43695,"output_tokens":485}}}"#;
        assert_eq!(parse_line(l).unwrap(), vec![Entry::Usage { context_tokens: 71180, model: Some("claude-opus-5-5".into()) }]);
        let side = br#"{"type":"assistant","isSidechain":true,"message":{"usage":{"input_tokens":5}}}"#;
        assert_eq!(parse_line(side), None);
        assert_eq!(parse_line(b"not json \"ai-title\""), None);
    }

    #[test]
    fn tails_appended_lines_and_partial_writes() {
        let path = std::env::temp_dir().join(format!("t1k-tail-{}.jsonl", std::process::id()));
        let mut f = File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"ai-title","aiTitle":"A"}}"#).unwrap();
        let mut t = Tail::new(path.clone());
        assert_eq!(t.poll(), vec![Entry::AiTitle("A".into())]);
        assert!(t.poll().is_empty());
        write!(f, r#"{{"type":"ai-title","#).unwrap();
        assert!(t.poll().is_empty());
        writeln!(f, r#""aiTitle":"B"}}"#).unwrap();
        assert_eq!(t.poll(), vec![Entry::AiTitle("B".into())]);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod bench {
    /// `T1K_HIST=<transcript> cargo test --release -p t1k-agent hist_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn hist_bench() {
        let p = std::path::PathBuf::from(std::env::var("T1K_HIST").unwrap());
        let len = std::fs::metadata(&p).unwrap().len();
        let t = std::time::Instant::now();
        let e = super::history_outputs(&p, len);
        let files = e.iter().filter(|e| matches!(e, super::Entry::FileEdit(_))).count();
        let urls = e.iter().filter(|e| matches!(e, super::Entry::Url(_) | super::Entry::PrLink { .. })).count();
        println!("{} MB in {:?}: {} entries, {files} file edits, {urls} links", len >> 20, t.elapsed(), e.len());
    }
}
