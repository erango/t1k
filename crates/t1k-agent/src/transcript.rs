//! Tail a Claude Code transcript (`~/.claude/projects/<proj>/<session>.jsonl`).
//!
//! The format is undocumented and changes, so parsing is tolerant: unknown entry types are
//! skipped, and only lines that mention an interesting type are parsed at all.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use serde_json::Value;

/// On attach, read at most this much from the end (transcripts can be hundreds of MB).
const ATTACH_WINDOW: u64 = 4 << 20;
/// Per poll, read at most this much new data.
const READ_CHUNK: u64 = 2 << 20;

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    AiTitle(String),
    CustomTitle(String),
    AwaySummary(String),
    ApiError(String),
    PrLink { url: String, number: u64 },
    Branch(String),
    TurnDone { duration_ms: u64 },
    /// A link worth listing as a session output (PR, artifact, doc, ticket, deploy).
    Url(String),
}

pub struct Tail {
    pub path: PathBuf,
    offset: Option<u64>,
    partial: Vec<u8>,
}

impl Tail {
    pub fn new(path: PathBuf) -> Self {
        Self { path, offset: None, partial: Vec::new() }
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
                (len.saturating_sub(ATTACH_WINDOW), true)
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

const INTERESTING: &[&str] =
    &["\"ai-title\"", "\"custom-title\"", "away_summary", "api_error", "pr-link", "turn_duration", "\"gitBranch\""];

/// Text that can mention session outputs: Claude's replies, the user's prompts, and tool
/// results from tools like artifact publishing, but not file contents or shell output.
fn output_text(v: &Value, out: &mut String) {
    let kind = v.get("type").and_then(Value::as_str);
    if !matches!(kind, Some("assistant") | Some("user")) {
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
        Some("pr-link") => {
            if let Some(url) = str_of("prUrl") {
                let number = v.get("prNumber").and_then(Value::as_u64).unwrap_or(0);
                out.push(Entry::PrLink { url, number });
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
                out.push(Entry::TurnDone { duration_ms: v.get("durationMs").and_then(Value::as_u64).unwrap_or(0) })
            }
            _ => {}
        },
        _ => {}
    }
    if let Some(b) = str_of("gitBranch").filter(|b| !b.is_empty() && b != "HEAD") {
        out.push(Entry::Branch(b));
    }
    out.extend(urls);
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_known_entries() {
        let l = br#"{"type":"ai-title","aiTitle":"Fix login","sessionId":"x"}"#;
        assert_eq!(parse_line(l), Some(vec![Entry::AiTitle("Fix login".into())]));
        let l = br#"{"type":"system","subtype":"away_summary","content":"Done; next ship it.","gitBranch":"main"}"#;
        assert_eq!(
            parse_line(l),
            Some(vec![Entry::AwaySummary("Done; next ship it.".into()), Entry::Branch("main".into())])
        );
        let l = br#"{"type":"pr-link","prNumber":23,"prUrl":"https://github.com/a/b/pull/23"}"#;
        assert_eq!(
            parse_line(l),
            Some(vec![Entry::PrLink { url: "https://github.com/a/b/pull/23".into(), number: 23 }])
        );
        assert_eq!(parse_line(br#"{"type":"assistant","message":{}}"#), None);
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
