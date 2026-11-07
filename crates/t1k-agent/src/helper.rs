//! The optional cheap-model helper: one-shot calls to a coding agent's own CLI (Claude Code,
//! Codex, Antigravity, Gemini CLI or Grok) for small jobs: a title for a session that has none,
//! a recap after a turn, and ranking tabs by meaning in the switcher.
//!
//! It reuses whatever login the CLI already has; no keys are stored or extracted. Every call
//! is measured (wall time, tokens, cost when the CLI reports it) and appended to
//! `helper.jsonl`, so `t1k helper stats` and Settings can show whether it earns its keep.
//!
//! Claude Code in print mode loads the user's whole setup (CLAUDE.md, MCP servers, plugins,
//! skills) by default: a one-word reply measured 376k tokens and $0.38. The flags below cut
//! that to ~650 tokens and ~$0.0001, which is the only reason this helper is viable.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Claude,
    Codex,
    Agy,
    Gemini,
    Grok,
}

impl Backend {
    pub const ALL: [Backend; 5] = [Backend::Claude, Backend::Codex, Backend::Agy, Backend::Gemini, Backend::Grok];

    pub fn parse(s: &str) -> Option<Backend> {
        Self::ALL.into_iter().find(|b| b.id() == s)
    }

    pub fn id(self) -> &'static str {
        match self {
            Backend::Claude => "claude",
            Backend::Codex => "codex",
            Backend::Agy => "agy",
            Backend::Gemini => "gemini",
            Backend::Grok => "grok",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Claude => "Claude Code",
            Backend::Codex => "Codex",
            Backend::Agy => "Antigravity CLI",
            Backend::Gemini => "Gemini CLI",
            Backend::Grok => "Grok CLI",
        }
    }

    /// The small, fast model each CLI offers. Empty: the CLI's default.
    pub fn default_model(self) -> &'static str {
        match self {
            Backend::Claude => "haiku",
            Backend::Agy => "gemini-3.8-flash-low",
            Backend::Gemini => "gemini-2.5-flash-lite",
            Backend::Codex | Backend::Grok => "",
        }
    }

    pub fn binary(self) -> &'static str {
        self.id()
    }

    /// Where the CLI is, looking in PATH and the usual install folders (an app launched from
    /// Finder has a short PATH).
    pub fn find(self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        dirs.extend([home.join(".local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into(), home.join(".bun/bin"), home.join(".npm-global/bin")]);
        dirs.into_iter().map(|d| d.join(self.binary())).find(|p| p.is_file())
    }
}

const SYSTEM: &str = "You write short status text for a terminal app that tracks coding-agent sessions. \
Answer with only the requested text: no quotes, no preamble, no markdown.";

/// What one call cost and how long it took.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    /// Input tokens read from or written to a prompt cache (counted apart from input_tokens).
    pub cached_tokens: u64,
    pub output_tokens: u64,
    /// As reported by the CLI; None when it doesn't say.
    pub cost_usd: Option<f64>,
    /// Model time as reported by the CLI.
    pub api_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    /// Unix seconds.
    pub at: u64,
    pub backend: Backend,
    pub model: String,
    /// "title", "recap", "search" or "bench".
    pub job: String,
    pub ok: bool,
    pub wall_ms: u64,
    #[serde(flatten)]
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Reply {
    pub text: String,
    pub record: Record,
}

/// Run one prompt. Blocks (call it off the UI thread). Never runs inside a T1K tab's hook
/// context: the T1K variables are removed so the agent's own hooks stay silent.
pub fn ask(backend: Backend, model: &str, job: &str, prompt: &str, timeout: Duration) -> Reply {
    let model = if model.is_empty() { backend.default_model() } else { model };
    let started = Instant::now();
    let result = run(backend, model, prompt, timeout);
    let wall_ms = started.elapsed().as_millis() as u64;
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mk = |ok, usage, error| Record { at, backend, model: model.to_string(), job: job.to_string(), ok, wall_ms, usage, error };
    match result {
        Ok((text, usage)) => {
            let text = clean(&text);
            let ok = !text.is_empty();
            Reply { text, record: mk(ok, usage, (!ok).then(|| "empty reply".to_string())) }
        }
        Err(e) => Reply { text: String::new(), record: mk(false, Usage::default(), Some(e)) },
    }
}

fn run(backend: Backend, model: &str, prompt: &str, timeout: Duration) -> Result<(String, Usage), String> {
    let bin = backend.find().ok_or_else(|| format!("{} not found", backend.binary()))?;
    // A scratch folder as cwd keeps project instructions (CLAUDE.md, AGENTS.md) out.
    let cwd = std::env::temp_dir().join("t1k-helper");
    let _ = std::fs::create_dir_all(&cwd);
    let mut cmd = Command::new(&bin);
    cmd.current_dir(&cwd).env_remove("T1K_TAB_ID").env_remove("T1K_SOCKET").env_remove("T1K_BIN");
    let full = format!("{SYSTEM}\n\n{prompt}");
    let mut stdin_text = None;
    match backend {
        Backend::Claude => {
            cmd.args(["-p", "--output-format", "json", "--no-session-persistence", "--tools", "", "--strict-mcp-config"])
                .args(["--setting-sources", "", "--disable-slash-commands", "--exclude-dynamic-system-prompt-sections"])
                .args(["--system-prompt", SYSTEM]);
            if !model.is_empty() {
                cmd.args(["--model", model]);
            }
            stdin_text = Some(prompt.to_string());
        }
        Backend::Agy => {
            if !model.is_empty() {
                cmd.args(["--model", model]);
            }
            cmd.args(["--output-format", "json", "--disable-slash-commands"]).arg(format!("-p={full}"));
        }
        Backend::Codex => {
            cmd.args(["exec", "--json", "--skip-git-repo-check", "--sandbox", "read-only"]);
            if !model.is_empty() {
                cmd.args(["-m", model]);
            }
            cmd.arg(&full);
        }
        Backend::Gemini => {
            if !model.is_empty() {
                cmd.args(["-m", model]);
            }
            cmd.args(["--output-format", "json", "-p", &full]);
        }
        Backend::Grok => {
            if !model.is_empty() {
                cmd.args(["--model", model]);
            }
            cmd.args(["--prompt", &full]);
        }
    }
    cmd.stdin(if stdin_text.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("start {}: {e}", backend.binary()))?;
    if let (Some(text), Some(mut sin)) = (stdin_text, child.stdin.take()) {
        let _ = sin.write_all(text.as_bytes());
    }
    // Read output on threads so a chatty child can't fill the pipe and stall.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let ho = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut s);
        s
    });
    let he = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut err, &mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = ho.join().unwrap_or_default();
    let stderr = he.join().unwrap_or_default();
    if !status.success() && stdout.trim().is_empty() {
        let line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim();
        return Err(format!("{} exited {}: {}", backend.binary(), status.code().unwrap_or(-1), line.chars().take(200).collect::<String>()));
    }
    parse(backend, &stdout)
}

/// Pull the reply text and usage out of a CLI's output.
pub fn parse(backend: Backend, out: &str) -> Result<(String, Usage), String> {
    use serde_json::Value;
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    match backend {
        Backend::Claude => {
            let v: Value = serde_json::from_str(out.trim()).map_err(|e| format!("claude output: {e}"))?;
            if v["is_error"].as_bool() == Some(true) {
                return Err(v["result"].as_str().unwrap_or("error").to_string());
            }
            let u = &v["usage"];
            Ok((
                v["result"].as_str().unwrap_or_default().to_string(),
                Usage {
                    input_tokens: n(&u["input_tokens"]),
                    cached_tokens: n(&u["cache_creation_input_tokens"]) + n(&u["cache_read_input_tokens"]),
                    output_tokens: n(&u["output_tokens"]),
                    cost_usd: v["total_cost_usd"].as_f64(),
                    api_ms: v["duration_api_ms"].as_u64(),
                },
            ))
        }
        Backend::Agy => {
            let v: Value = serde_json::from_str(out.trim()).map_err(|e| format!("agy output: {e}"))?;
            if v["status"].as_str().is_some_and(|s| s != "SUCCESS") {
                return Err(format!("agy: {}", v["status"]));
            }
            let u = &v["usage"];
            Ok((
                v["response"].as_str().unwrap_or_default().to_string(),
                Usage {
                    input_tokens: n(&u["input_tokens"]),
                    cached_tokens: n(&u["cache_read_tokens"]),
                    output_tokens: n(&u["output_tokens"]) + n(&u["thinking_tokens"]),
                    cost_usd: None,
                    api_ms: v["duration_seconds"].as_f64().map(|s| (s * 1000.) as u64),
                },
            ))
        }
        Backend::Codex => {
            // JSON lines: the last agent message is the reply; turn.completed has usage.
            let mut text = String::new();
            let mut usage = Usage::default();
            for line in out.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                match v["type"].as_str() {
                    Some("item.completed") if v["item"]["type"] == "agent_message" => {
                        text = v["item"]["text"].as_str().unwrap_or_default().to_string();
                    }
                    Some("turn.completed") => {
                        let u = &v["usage"];
                        usage.cached_tokens += n(&u["cached_input_tokens"]);
                        usage.input_tokens += n(&u["input_tokens"]).saturating_sub(n(&u["cached_input_tokens"]));
                        usage.output_tokens += n(&u["output_tokens"]);
                    }
                    Some("turn.failed") | Some("error") => {
                        return Err(format!("codex: {}", v["error"]["message"].as_str().or(v["message"].as_str()).unwrap_or("failed")));
                    }
                    _ => {}
                }
            }
            Ok((text, usage))
        }
        Backend::Gemini => {
            let v: Value = serde_json::from_str(out.trim()).map_err(|e| format!("gemini output: {e}"))?;
            let mut usage = Usage::default();
            if let Some(models) = v["stats"]["models"].as_object() {
                for m in models.values() {
                    let t = &m["tokens"];
                    usage.cached_tokens += n(&t["cached"]);
                    usage.input_tokens += n(&t["prompt"]).saturating_sub(n(&t["cached"]));
                    usage.output_tokens += n(&t["candidates"]) + n(&t["thoughts"]);
                    usage.api_ms = Some(usage.api_ms.unwrap_or(0) + n(&m["api"]["totalLatencyMs"]));
                }
            }
            Ok((v["response"].as_str().unwrap_or_default().to_string(), usage))
        }
        // Grok CLI prints the reply as text and doesn't report usage.
        Backend::Grok => Ok((out.to_string(), Usage::default())),
    }
}

/// Trim quotes, markdown bullets and trailing periods models like to add.
fn clean(s: &str) -> String {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').trim();
    let s = s.strip_prefix("- ").unwrap_or(s);
    s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

// ---- Jobs ----

/// A 3-6 word title from the user's first prompt.
pub fn title_prompt(first_prompt: &str) -> String {
    format!(
        "Name this coding session in 3 to 6 words, like a ticket title (e.g. \"Fix login redirect loop\"). \
         The user's first request:\n\n{}",
        excerpt(first_prompt, 1500)
    )
}

/// One line on what the agent just did and what's next, from the last prompt and the screen.
pub fn recap_prompt(last_prompt: Option<&str>, screen: &str) -> String {
    format!(
        "A coding agent just finished a turn. In one line under 110 characters, say what it did and \
         what's next or what it's waiting for (e.g. \"Fixed the flaky auth test; next, push the branch\").\n\n\
         The user asked: {}\n\nEnd of the terminal:\n{}",
        last_prompt.map(|p| excerpt(p, 600)).unwrap_or_else(|| "(unknown)".into()),
        excerpt_tail(screen, 3000)
    )
}

/// Rank tab cards by how well they match a description. Cards are "id: text" lines.
pub fn search_prompt(query: &str, cards: &[(usize, String)]) -> String {
    let list = cards.iter().map(|(i, c)| format!("{i}: {}", excerpt(c, 200))).collect::<Vec<_>>().join("\n");
    format!(
        "Which of these terminal tabs best matches \"{query}\"? Reply with up to 5 ids, best first, \
         comma separated, or \"none\".\n\n{list}"
    )
}

/// Tab ids from a search reply, in order, keeping only ids that were offered.
pub fn parse_ids(reply: &str, offered: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    for part in reply.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(i) = part.parse::<usize>() {
            if offered.contains(&i) && !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out
}

fn excerpt(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

fn excerpt_tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    "…".to_string() + &s.chars().skip(n - max).collect::<String>()
}

// ---- Measurements ----

pub fn log_path(support_dir: &Path) -> PathBuf {
    support_dir.join("helper.jsonl")
}

pub fn append(support_dir: &Path, r: &Record) {
    let Ok(line) = serde_json::to_string(r) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path(support_dir)) {
        let _ = writeln!(f, "{line}");
    }
}

pub fn read_log(support_dir: &Path, since_unix: u64) -> Vec<Record> {
    std::fs::read_to_string(log_path(support_dir))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Record>(l).ok())
        .filter(|r| r.at >= since_unix)
        .collect()
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stats {
    pub calls: usize,
    pub failed: usize,
    pub median_ms: u64,
    pub p90_ms: u64,
    pub avg_input: u64,
    pub avg_cached: u64,
    pub avg_output: u64,
    /// Sum of reported costs, and how many calls reported one.
    pub cost_usd: f64,
    pub priced_calls: usize,
}

impl Stats {
    pub fn of(records: &[&Record]) -> Stats {
        let ok: Vec<&&Record> = records.iter().filter(|r| r.ok).collect();
        let mut ms: Vec<u64> = ok.iter().map(|r| r.wall_ms).collect();
        ms.sort_unstable();
        let pct = |p: f64| if ms.is_empty() { 0 } else { ms[((ms.len() - 1) as f64 * p).round() as usize] };
        let avg = |f: &dyn Fn(&Record) -> u64| if ok.is_empty() { 0 } else { ok.iter().map(|r| f(r)).sum::<u64>() / ok.len() as u64 };
        Stats {
            calls: records.len(),
            failed: records.len() - ok.len(),
            median_ms: pct(0.5),
            p90_ms: pct(0.9),
            avg_input: avg(&|r| r.usage.input_tokens),
            avg_cached: avg(&|r| r.usage.cached_tokens),
            avg_output: avg(&|r| r.usage.output_tokens),
            cost_usd: records.iter().filter_map(|r| r.usage.cost_usd).sum(),
            priced_calls: records.iter().filter(|r| r.usage.cost_usd.is_some()).count(),
        }
    }

    /// "12 calls · median 3.1 s · 690 in + 12 out tokens · $0.0016 ($0.00013 a call)"
    pub fn line(&self) -> String {
        if self.calls == 0 {
            return "No calls yet".into();
        }
        let mut s = format!("{} call{}", self.calls, if self.calls == 1 { "" } else { "s" });
        if self.failed > 0 {
            s += &format!(" ({} failed)", self.failed);
        }
        if self.calls > self.failed {
            s += &format!(" · median {:.1} s, p90 {:.1} s", self.median_ms as f64 / 1000., self.p90_ms as f64 / 1000.);
            s += &format!(" · {} in + {} out tokens a call", self.avg_input + self.avg_cached, self.avg_output);
        }
        if self.priced_calls > 0 {
            s += &format!(" · ${:.4} (${:.5} a call)", self.cost_usd, self.cost_usd / self.priced_calls as f64);
        } else {
            s += " · cost not reported (subscription)";
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_and_agy() {
        let c = r#"{"result":"ok","is_error":false,"total_cost_usd":0.0001336,"duration_api_ms":1122,
            "usage":{"input_tokens":2,"cache_creation_input_tokens":657,"cache_read_input_tokens":0,"output_tokens":4}}"#;
        let (t, u) = parse(Backend::Claude, c).unwrap();
        assert_eq!(t, "ok");
        assert_eq!((u.input_tokens, u.cached_tokens, u.output_tokens), (2, 657, 4));
        assert_eq!(u.cost_usd, Some(0.0001336));
        let a = r#"{"status":"SUCCESS","response":"ok\n","duration_seconds":3.01,"usage":{"input_tokens":8706,"output_tokens":1,"thinking_tokens":0,"cache_read_tokens":0}}"#;
        let (t, u) = parse(Backend::Agy, a).unwrap();
        assert_eq!(clean(&t), "ok");
        assert_eq!((u.input_tokens, u.cost_usd, u.api_ms), (8706, None, Some(3010)));
    }

    #[test]
    fn parses_codex_lines() {
        let out = "{\"type\":\"thread.started\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Fix login loop\"}}\n\
                   {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1200,\"cached_input_tokens\":1000,\"output_tokens\":6}}";
        let (t, u) = parse(Backend::Codex, out).unwrap();
        assert_eq!(t, "Fix login loop");
        assert_eq!((u.input_tokens, u.cached_tokens, u.output_tokens), (200, 1000, 6));
    }

    #[test]
    fn ids_and_stats() {
        assert_eq!(parse_ids("3, 7, 99, 3", &[1, 3, 7]), [3, 7]);
        let r = |ms, cost| Record {
            at: 0,
            backend: Backend::Claude,
            model: "haiku".into(),
            job: "title".into(),
            ok: true,
            wall_ms: ms,
            usage: Usage { input_tokens: 10, cached_tokens: 650, output_tokens: 8, cost_usd: cost, api_ms: None },
            error: None,
        };
        let rs = [r(3000, Some(0.0002)), r(2000, Some(0.0001)), r(9000, None)];
        let s = Stats::of(&rs.iter().collect::<Vec<_>>());
        assert_eq!((s.calls, s.median_ms, s.p90_ms, s.priced_calls), (3, 3000, 9000, 2));
        assert!(s.line().contains("median 3.0 s"));
        assert_eq!(clean("\"Fix login redirect loop.\"\n"), "Fix login redirect loop.");
    }
}
