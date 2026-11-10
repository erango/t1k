//! What sits in a Claude session's context window, and what the session has used, read from
//! its transcript for the Context pane (canvas v3-au).
//!
//! Only the total in context is exact (the last reply's usage). The breakdown is estimated
//! from the size of the text Claude Code records: the request's system prompt and loaded
//! tools (`api-request-shape`), memory files (`instructions`, `nested_memory`), MCP server
//! instructions, the skills and subagent listings, and skills that were invoked. Messages
//! are what's left. Session totals add up every reply's usage, subagents included (their
//! transcripts sit in `<session>/subagents/`).
//!
//! Never read: `session_context` (it holds the user's email).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// Characters per token for the estimates.
pub const CHARS_PER_TOKEN: usize = 4;

pub fn tokens(chars: usize) -> u64 {
    chars.div_ceil(CHARS_PER_TOKEN) as u64
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelUsage {
    pub input: u64,
    pub output: u64,
    pub thinking: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub web_searches: u64,
    pub replies: u64,
}

impl ModelUsage {
    pub fn cache_write(&self) -> u64 {
        self.cache_write_5m + self.cache_write_1h
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Compaction {
    pub auto: bool,
    pub before: u64,
    pub after: Option<u64>,
    pub duration_ms: Option<u64>,
    pub at: Option<SystemTime>,
}

/// The main thread's latest request settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub max_tokens: Option<u64>,
    pub thinking: Option<String>,
    pub cache_ttl: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// System prompt, in characters.
    pub system: usize,
    /// Tools whose definitions the last request carried: (name, characters).
    pub tools: Vec<(String, usize)>,
    /// MCP server instructions by server.
    pub mcp_instructions: BTreeMap<String, usize>,
    /// Memory files loaded since the last compaction: path to characters.
    pub memory: BTreeMap<String, usize>,
    /// The skills listing: (skills, characters).
    pub skills: Option<(usize, usize)>,
    /// Skills invoked since the last compaction, whose whole text is in context.
    pub invoked_skills: BTreeMap<String, usize>,
    /// The subagent listing: type to characters.
    pub agents: BTreeMap<String, usize>,
    /// Tools known only by name (definitions load on first use).
    pub deferred: BTreeSet<String>,
    pub compactions: Vec<Compaction>,
    /// Every reply's usage by model, subagents included.
    pub models: BTreeMap<String, ModelUsage>,
    pub request: Request,
    pub started: Option<SystemTime>,
    /// Tokens in context as of the main thread's last reply (input + cache).
    pub last_context: Option<u64>,
}

impl Report {
    /// The estimated categories, in tokens: (system prompt, built-in tools, MCP tools, MCP
    /// instructions, memory files, skills, subagents).
    pub fn estimates(&self) -> Estimates {
        let (mcp, builtin): (Vec<_>, Vec<_>) = self.tools.iter().partition(|(n, _)| n.starts_with("mcp__"));
        Estimates {
            system_prompt: tokens(self.system),
            system_tools: tokens(builtin.iter().map(|(_, c)| c).sum()),
            mcp_tools: tokens(mcp.iter().map(|(_, c)| c).sum()),
            mcp_instructions: tokens(self.mcp_instructions.values().sum()),
            memory: tokens(self.memory.values().sum()),
            skills: tokens(self.skills.map_or(0, |s| s.1) + self.invoked_skills.values().sum::<usize>()),
            subagents: tokens(self.agents.values().sum()),
        }
    }

    pub fn dropped(&self) -> u64 {
        self.compactions.iter().filter_map(|c| c.after.map(|a| c.before.saturating_sub(a))).sum()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Estimates {
    pub system_prompt: u64,
    pub system_tools: u64,
    pub mcp_tools: u64,
    pub mcp_instructions: u64,
    pub memory: u64,
    pub skills: u64,
    pub subagents: u64,
}

impl Estimates {
    pub fn sum(&self) -> u64 {
        self.system_prompt + self.system_tools + self.mcp_tools + self.mcp_instructions + self.memory + self.skills + self.subagents
    }
}

/// The MCP server a tool belongs to (`mcp__server__tool`), if any.
pub fn mcp_server(tool: &str) -> Option<&str> {
    let rest = tool.strip_prefix("mcp__")?;
    rest.split_once("__").map(|(s, _)| s)
}

/// One file read in steps: where we are and the cut line carried to the next read.
#[derive(Default)]
struct Cursor {
    offset: u64,
    partial: Vec<u8>,
}

impl Cursor {
    fn read(&mut self, path: &Path, mut each: impl FnMut(&[u8])) {
        let Ok(mut f) = File::open(path) else { return };
        let Ok(len) = f.metadata().map(|m| m.len()) else { return };
        if len < self.offset {
            // Rewritten: start over.
            *self = Cursor::default();
        }
        if len == self.offset || f.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        if f.take(len - self.offset).read_to_end(&mut buf).is_err() {
            return;
        }
        self.offset += buf.len() as u64;
        let mut data = std::mem::take(&mut self.partial);
        data.extend_from_slice(&buf);
        let mut lines: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
        let rest = lines.pop().unwrap_or_default().to_vec();
        for l in lines {
            each(l);
        }
        self.partial = rest;
    }
}

/// Reads a session's transcript and its subagents' as they grow.
pub struct Scan {
    pub path: PathBuf,
    main: Cursor,
    subs: BTreeMap<PathBuf, Cursor>,
    seen: HashSet<String>,
    pub report: Report,
}

/// Lines worth parsing; everything else (most of a transcript) is skipped unparsed.
const HINTS: &[&str] = &[
    "\"usage\"",
    "\"api-request-shape\"",
    "\"api-request\"",
    "\"compact_boundary\"",
    "\"instructions\"",
    "\"nested_memory\"",
    "\"mcp_instructions_delta\"",
    "\"skill_listing\"",
    "\"invoked_skills\"",
    "\"agent_listing_delta\"",
    "\"deferred_tools_delta\"",
];

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

impl Scan {
    pub fn new(path: PathBuf) -> Self {
        Self { path, main: Cursor::default(), subs: BTreeMap::new(), seen: HashSet::new(), report: Report::default() }
    }

    /// Reads what was appended since the last call (the whole transcript the first time).
    pub fn poll(&mut self) {
        let mut main = std::mem::take(&mut self.main);
        let path = self.path.clone();
        main.read(&path, |l| self.line(l, true));
        self.main = main;
        let dir = self.path.with_extension("").join("subagents");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "jsonl") {
                    let mut c = self.subs.remove(&p).unwrap_or_default();
                    c.read(&p, |l| self.line(l, false));
                    self.subs.insert(p, c);
                }
            }
        }
    }

    fn line(&mut self, l: &[u8], main: bool) {
        if !HINTS.iter().any(|h| contains(l, h)) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(l) else { return };
        let r = &mut self.report;
        if r.started.is_none() {
            r.started = v.get("timestamp").and_then(Value::as_str).and_then(crate::transcript::parse_time);
        }
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let msg = &v["message"];
                let Some(u) = msg.get("usage") else { return };
                // A reply is written once per content block, each with the same usage.
                let id = s(msg, "id").or_else(|| s(&v, "requestId")).unwrap_or_else(|| s(&v, "uuid").unwrap_or_default());
                if !self.seen.insert(id) {
                    return;
                }
                let model = s(msg, "model").unwrap_or_else(|| "unknown".into());
                if model == "<synthetic>" {
                    return;
                }
                let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
                if main && v.get("isSidechain").and_then(Value::as_bool) != Some(true) {
                    let total = n(u, "input_tokens") + n(u, "cache_read_input_tokens") + n(u, "cache_creation_input_tokens");
                    if total > 0 {
                        r.last_context = Some(total);
                    }
                }
                let m = r.models.entry(model).or_default();
                m.replies += 1;
                m.input += n(u, "input_tokens");
                m.output += n(u, "output_tokens");
                m.cache_read += n(u, "cache_read_input_tokens");
                m.thinking += u.get("output_tokens_details").map_or(0, |d| n(d, "thinking_tokens"));
                m.web_searches += u.get("server_tool_use").map_or(0, |d| n(d, "web_search_requests"));
                match u.get("cache_creation") {
                    Some(c) => {
                        m.cache_write_5m += n(c, "ephemeral_5m_input_tokens");
                        m.cache_write_1h += n(c, "ephemeral_1h_input_tokens");
                    }
                    None => m.cache_write_5m += n(u, "cache_creation_input_tokens"),
                }
            }
            _ if !main => {}
            Some("api-request-shape") => {
                let shape = &v["shape"];
                if let Some(sys) = shape.get("system").and_then(Value::as_array) {
                    r.system = sys.iter().map(|b| b.get("text").and_then(Value::as_str).map_or(0, str::len)).sum();
                }
                if let Some(tools) = shape.get("tools").and_then(Value::as_array) {
                    r.tools = tools
                        .iter()
                        .filter_map(|t| Some((s(t, "name")?, t.to_string().len())))
                        .collect();
                }
            }
            Some("api-request") => {
                let p = &v["params"];
                r.request = Request {
                    model: s(p, "model"),
                    effort: p.get("output_config").and_then(|o| s(o, "effort")),
                    max_tokens: p.get("max_tokens").and_then(Value::as_u64),
                    thinking: p.get("thinking").and_then(|t| s(t, "display").or_else(|| s(t, "type"))),
                    cache_ttl: v.get("cacheControl").and_then(Value::as_array).and_then(|a| a.iter().find_map(|c| c.get("value").and_then(|x| s(x, "ttl")))),
                };
            }
            Some("system") if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
                let m = v.get("compactMetadata").cloned().unwrap_or(Value::Null);
                r.compactions.push(Compaction {
                    auto: s(&m, "trigger").as_deref() != Some("manual"),
                    before: m.get("preTokens").and_then(Value::as_u64).unwrap_or(0),
                    after: m.get("postTokens").and_then(Value::as_u64),
                    duration_ms: m.get("durationMs").and_then(Value::as_u64),
                    at: v.get("timestamp").and_then(Value::as_str).and_then(crate::transcript::parse_time),
                });
                // Compaction drops what was loaded along the way; Claude Code sends memory
                // files and invoked skills again after it.
                r.memory.clear();
                r.invoked_skills.clear();
            }
            Some("attachment") => attachment(r, &v["attachment"]),
            _ => {}
        }
    }
}

fn attachment(r: &mut Report, a: &Value) {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let strs = |k: &str| -> Vec<String> { a.get(k).and_then(Value::as_array).map(|x| x.iter().filter_map(|i| i.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    match a.get("type").and_then(Value::as_str) {
        Some("instructions") => {
            for f in a.get("files").and_then(Value::as_array).into_iter().flatten() {
                if let Some(p) = s(f, "path") {
                    r.memory.insert(p, f.get("content").and_then(Value::as_str).map_or(0, str::len));
                }
            }
        }
        Some("nested_memory") => {
            let c = &a["content"];
            if let Some(p) = s(a, "path").or_else(|| s(c, "path")) {
                r.memory.insert(p, c.get("content").and_then(Value::as_str).map_or(0, str::len));
            }
        }
        Some("mcp_instructions_delta") => {
            for n in strs("removedNames") {
                r.mcp_instructions.remove(&n);
            }
            let blocks = strs("addedBlocks");
            for (i, n) in strs("addedNames").into_iter().enumerate() {
                r.mcp_instructions.insert(n, blocks.get(i).map_or(0, String::len));
            }
        }
        Some("skill_listing") => {
            let chars = a.get("content").and_then(Value::as_str).map_or(0, str::len);
            let count = a.get("skillCount").and_then(Value::as_u64).map(|c| c as usize).unwrap_or_else(|| strs("names").len());
            if a.get("isInitial").and_then(Value::as_bool) == Some(false) {
                let (c0, n0) = r.skills.unwrap_or_default();
                r.skills = Some((c0.max(count), n0 + chars));
            } else {
                r.skills = Some((count, chars));
            }
        }
        Some("invoked_skills") => {
            for sk in a.get("skills").and_then(Value::as_array).into_iter().flatten() {
                if let Some(n) = s(sk, "name") {
                    r.invoked_skills.insert(n, sk.get("content").and_then(Value::as_str).map_or(0, str::len));
                }
            }
        }
        Some("agent_listing_delta") => {
            if a.get("isInitial").and_then(Value::as_bool) == Some(true) {
                r.agents.clear();
            }
            for n in strs("removedTypes") {
                r.agents.remove(&n);
            }
            let lines = strs("addedLines");
            for (i, n) in strs("addedTypes").into_iter().enumerate() {
                r.agents.insert(n, lines.get(i).map_or(0, String::len));
            }
        }
        Some("deferred_tools_delta") => {
            for n in strs("removedNames") {
                r.deferred.remove(&n);
            }
            r.deferred.extend(strs("addedNames"));
            r.deferred.extend(strs("readdedNames"));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_breakdown_compactions_and_totals() {
        let dir = std::env::temp_dir().join(format!("t1k-ctx-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("s/subagents")).unwrap();
        let p = dir.join("s.jsonl");
        let mut f = File::create(&p).unwrap();
        let lines = [
            r#"{"type":"attachment","timestamp":"2026-11-10T10:00:00Z","attachment":{"type":"instructions","files":[{"path":"/a/CLAUDE.md","content":"12345678"}]}}"#,
            r#"{"type":"attachment","attachment":{"type":"session_context","context":{"userEmail":"x@y.z"}}}"#,
            r#"{"type":"attachment","attachment":{"type":"skill_listing","content":"abcd","skillCount":2,"isInitial":true}}"#,
            r#"{"type":"attachment","attachment":{"type":"deferred_tools_delta","addedNames":["mcp__linear__get","WebSearch"],"removedNames":[]}}"#,
            r#"{"type":"attachment","attachment":{"type":"mcp_instructions_delta","addedNames":["linear"],"addedBlocks":["12345678"],"removedNames":[]}}"#,
            r#"{"type":"attachment","attachment":{"type":"agent_listing_delta","addedTypes":["Explore"],"addedLines":["- Explore: x"],"isInitial":true}}"#,
            r#"{"type":"api-request-shape","shape":{"system":[{"type":"text","text":"12345678"}],"tools":[{"name":"Bash"},{"name":"mcp__linear__save"}]}}"#,
            r#"{"type":"api-request","params":{"model":"claude-opus-5-5","max_tokens":128000,"thinking":{"type":"adaptive","display":"summarized"},"output_config":{"effort":"high"}},"cacheControl":[{"value":{"type":"ephemeral","ttl":"1h"}}]}"#,
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":10,"cache_read_input_tokens":100,"cache_creation":{"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":4},"output_tokens_details":{"thinking_tokens":5}}}}"#,
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":10}}}"#,
            r#"{"type":"system","subtype":"compact_boundary","timestamp":"2026-11-10T11:00:00Z","compactMetadata":{"trigger":"auto","preTokens":200460,"postTokens":33142,"durationMs":41000}}"#,
        ];
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        let mut sub = File::create(dir.join("s/subagents/agent-1.jsonl")).unwrap();
        writeln!(sub, r#"{{"type":"assistant","isSidechain":true,"message":{{"id":"m2","model":"claude-haiku-5-5","usage":{{"input_tokens":7,"output_tokens":1}}}}}}"#).unwrap();
        let mut scan = Scan::new(p.clone());
        scan.poll();
        let r = &scan.report;
        assert!(r.memory.is_empty(), "compaction clears memory files");
        assert_eq!(r.skills, Some((2, 4)));
        assert_eq!(r.deferred.len(), 2);
        assert_eq!(r.mcp_instructions.get("linear"), Some(&8));
        assert_eq!(r.system, 8);
        assert_eq!(r.tools.len(), 2);
        assert_eq!(r.request.effort.as_deref(), Some("high"));
        assert_eq!(r.request.cache_ttl.as_deref(), Some("1h"));
        assert_eq!(r.request.thinking.as_deref(), Some("summarized"));
        let opus = &r.models["claude-opus-5-5"];
        assert_eq!((opus.replies, opus.output, opus.cache_write(), opus.thinking), (1, 10, 7, 5));
        assert_eq!(r.models["claude-haiku-5-5"].input, 7);
        assert_eq!(r.dropped(), 200460 - 33142);
        assert!(r.started.is_some());
        // Appended lines arrive on the next poll.
        writeln!(f, r#"{{"type":"attachment","attachment":{{"type":"nested_memory","path":"/a/b/CLAUDE.md","content":{{"content":"1234"}}}}}}"#).unwrap();
        scan.poll();
        assert_eq!(scan.report.memory.get("/a/b/CLAUDE.md"), Some(&4));
        assert_eq!(mcp_server("mcp__claude_ai_Linear__get_issue"), Some("claude_ai_Linear"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod bench {
    /// `T1K_CTX=<transcript> cargo test --release -p t1k-agent ctx_bench -- --ignored --nocapture`
    /// Prints sizes and counts only, never content.
    #[test]
    #[ignore]
    fn ctx_bench() {
        let p = std::path::PathBuf::from(std::env::var("T1K_CTX").unwrap());
        let t = std::time::Instant::now();
        let mut s = super::Scan::new(p);
        s.poll();
        let r = &s.report;
        let e = r.estimates();
        println!(
            "{:?}: est {:?} sum {} | tools {} deferred {} memory {} mcp-instr {} agents {} skills {:?} compactions {} dropped {} models {:?} request {:?}",
            t.elapsed(), e, e.sum(), r.tools.len(), r.deferred.len(), r.memory.len(), r.mcp_instructions.len(), r.agents.len(), r.skills,
            r.compactions.len(), r.dropped(), r.models.iter().map(|(k, m)| (k.clone(), m.replies, m.output)).collect::<Vec<_>>(), r.request
        );
    }
}
