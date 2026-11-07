//! Agents' plan usage limits (Claude's 5-hour and weekly windows, Antigravity's quota buckets).
//!
//! Claude Code and Antigravity CLI report them only to their status-line command, on stdin:
//! Claude as `rate_limits.{five_hour,seven_day}.{used_percentage,resets_at}` (Pro/Max plans,
//! after the first reply), Antigravity as `quota.<bucket>.{remaining_fraction,reset_in_seconds}`.
//! No hook, transcript or file carries them. So T1K wraps the user's status-line command: the
//! wrapper hands the JSON to `t1k statusline`, which forwards the limits to the app, then runs
//! the original command on the same input, so the status line looks exactly as before.
//! Outside T1K (`$T1K_BIN` unset) only the original runs.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::agents::AgentKind;

/// The `hook_event_name` T1K uses for forwarded limits (not a real hook).
pub const EVENT: &str = "T1KStatusLine";

const MARK: &str = r#""$T1K_BIN" statusline"#;
const CHAIN_OPEN: &str = r#"printf '%s' "$input" | { "#;
const CHAIN_CLOSE: &str = "\n}";

fn head(kind: AgentKind) -> String {
    let agent = if kind == AgentKind::Claude { String::new() } else { format!(" --agent {}", kind.id()) };
    format!(r#"input=$(cat); [ -n "$T1K_BIN" ] && printf '%s' "$input" | {MARK}{agent}; "#)
}

/// One usage window, e.g. Claude's 5-hour limit or an Antigravity weekly bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// Short name: "5h", "7d", or the agent's own bucket id ("gemini-weekly").
    pub name: String,
    /// 0–100.
    pub used: f32,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub windows: Vec<Window>,
    /// Unix seconds when the agent last reported them.
    pub at: i64,
}

impl Usage {
    /// The window closest to its limit.
    pub fn highest(&self) -> Option<&Window> {
        self.windows.iter().max_by(|a, b| a.used.total_cmp(&b.used))
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn int(v: Option<&Value>) -> Option<i64> {
    v.and_then(|r| r.as_i64().or_else(|| r.as_f64().map(|f| f as i64)))
}

/// The limits in a status-line (or forwarded) payload; None when it has none.
pub fn parse(v: &Value) -> Option<Usage> {
    let mut windows = Vec::new();
    // Claude Code.
    if let Some(r) = v.get("rate_limits") {
        for (key, name) in [("five_hour", "5h"), ("seven_day", "7d")] {
            let Some(w) = r.get(key) else { continue };
            let Some(used) = w.get("used_percentage").and_then(Value::as_f64) else { continue };
            windows.push(Window { name: name.into(), used: (used as f32).clamp(0.0, 100.0), resets_at: int(w.get("resets_at")) });
        }
    }
    // Antigravity CLI: what's left, as a fraction, per quota bucket.
    if let Some(q) = v.get("quota").and_then(Value::as_object) {
        for (name, b) in q {
            let Some(left) = b.get("remaining_fraction").and_then(Value::as_f64) else { continue };
            let resets_at = int(b.get("reset_in_seconds")).map(|s| now() + s);
            windows.push(Window { name: name.clone(), used: ((1.0 - left) * 100.0).clamp(0.0, 100.0) as f32, resets_at });
        }
    }
    (!windows.is_empty()).then(|| Usage { windows, at: now() })
}

/// Codex has no status-line command, but its rollout file (`transcript_path` in its hooks)
/// records a `token_count` event with `rate_limits` after each reply. The newest one wins.
/// Windows are told apart by length, not position (Codex's own TUI does the same), and only
/// the account-wide `codex` limit counts (other ids are per-model quotas).
pub fn codex_rollout(tail: &str) -> Option<Usage> {
    for line in tail.lines().rev() {
        if !line.contains("\"token_count\"") || !line.contains("rate_limits") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(r) = v.pointer("/payload/rate_limits").filter(|r| r.is_object()) else { continue };
        if r.get("limit_id").and_then(Value::as_str).is_some_and(|id| id != "codex") {
            continue;
        }
        let mut windows: Vec<(i64, Window)> = ["primary", "secondary"]
            .iter()
            .filter_map(|k| {
                let w = r.get(*k)?;
                let used = w.get("used_percent")?.as_f64()? as f32;
                let mins = int(w.get("window_minutes")).unwrap_or(0);
                let name = match mins {
                    300 => "5h".to_string(),
                    10080 => "7d".to_string(),
                    m if m > 0 && m % 1440 == 0 => format!("{}d", m / 1440),
                    m if m > 0 && m % 60 == 0 => format!("{}h", m / 60),
                    m => format!("{m}m"),
                };
                Some((mins, Window { name, used: used.clamp(0.0, 100.0), resets_at: int(w.get("resets_at")) }))
            })
            .collect();
        if windows.is_empty() {
            continue;
        }
        windows.sort_by_key(|(m, _)| *m);
        return Some(Usage { windows: windows.into_iter().map(|(_, w)| w).collect(), at: now() });
    }
    None
}

/// What `t1k statusline` sends to the app: only the limits, nothing about the session.
pub fn forward_payload(kind: AgentKind, statusline_json: &Value) -> Option<Value> {
    let mut out = json!({ "hook_event_name": EVENT, "t1k_agent": kind.id() });
    for key in ["rate_limits", "quota"] {
        if let Some(v) = statusline_json.get(key) {
            out[key] = v.clone();
        }
    }
    parse(&out).map(|_| out)
}

/// The status-line command that forwards to T1K and then runs `original` (if any).
pub fn wrapped_command(kind: AgentKind, original: Option<&str>) -> String {
    match original.map(str::trim).filter(|o| !o.is_empty()) {
        Some(o) => format!("{}{CHAIN_OPEN}{o}{CHAIN_CLOSE}", head(kind)),
        None => format!("{}true", head(kind)),
    }
}

/// The user's own command inside a wrapped one (None if there was none).
pub fn original_command(wrapped: &str) -> Option<&str> {
    let rest = &wrapped[wrapped.find(MARK)?..];
    rest[rest.find(CHAIN_OPEN)? + CHAIN_OPEN.len()..].strip_suffix(CHAIN_CLOSE)
}

pub fn is_wrapped(settings: &Value) -> bool {
    settings.pointer("/statusLine/command").and_then(Value::as_str).is_some_and(|c| c.contains(MARK))
}

/// Wrap the status line in an agent's settings.json value. Returns true if it changed.
/// Keeps the user's command and its other fields (padding, refreshInterval).
pub fn wrap(kind: AgentKind, settings: &mut Value) -> bool {
    if is_wrapped(settings) {
        return false;
    }
    if !settings.is_object() {
        *settings = Value::Object(Map::new());
    }
    let obj = settings.as_object_mut().unwrap();
    let mut line = match obj.remove("statusLine") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let original = match line.get("type").and_then(Value::as_str) {
        None | Some("command") => line.get("command").and_then(Value::as_str).map(str::to_string),
        // An unknown kind of status line: leave it alone rather than lose it.
        Some(_) => {
            obj.insert("statusLine".into(), Value::Object(line));
            return false;
        }
    };
    // Antigravity replaces its own status line with a custom one unless told to stack them;
    // T1K's prints nothing, so stacking keeps the default looking as it did.
    if kind == AgentKind::Antigravity && original.is_none() {
        line.insert("stack_with_default".into(), json!(true));
    }
    line.insert("type".into(), json!("command"));
    line.insert("command".into(), json!(wrapped_command(kind, original.as_deref())));
    obj.insert("statusLine".into(), Value::Object(line));
    true
}

/// Undo `wrap`: restore the user's command, or remove the status line T1K added.
pub fn unwrap(settings: &mut Value) -> bool {
    let Some(cmd) = settings.pointer("/statusLine/command").and_then(Value::as_str).map(str::to_string) else { return false };
    if !cmd.contains(MARK) {
        return false;
    }
    match original_command(&cmd) {
        Some(o) => settings["statusLine"]["command"] = json!(o),
        None => {
            settings.as_object_mut().unwrap().remove("statusLine");
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_unwraps_the_users_command() {
        let mut s = json!({"statusLine": {"type": "command", "command": "bash ~/x/wrapper.sh", "padding": 0}});
        assert!(wrap(AgentKind::Claude, &mut s));
        assert!(!wrap(AgentKind::Claude, &mut s), "idempotent");
        let cmd = s["statusLine"]["command"].as_str().unwrap();
        assert_eq!(original_command(cmd), Some("bash ~/x/wrapper.sh"));
        assert_eq!(s["statusLine"]["padding"], 0);
        assert!(unwrap(&mut s));
        assert_eq!(s, json!({"statusLine": {"type": "command", "command": "bash ~/x/wrapper.sh", "padding": 0}}));
    }

    #[test]
    fn adds_and_removes_one_when_there_was_none() {
        let mut s = json!({"model": "opus"});
        assert!(wrap(AgentKind::Claude, &mut s));
        assert!(is_wrapped(&s));
        assert!(unwrap(&mut s));
        assert_eq!(s, json!({"model": "opus"}));
        let mut s = json!({"model": "x"});
        assert!(wrap(AgentKind::Antigravity, &mut s));
        assert_eq!(s["statusLine"]["stack_with_default"], true);
        assert!(s["statusLine"]["command"].as_str().unwrap().contains("statusline --agent antigravity;"));
    }

    #[test]
    fn parses_limits() {
        let v = json!({"rate_limits": {"five_hour": {"used_percentage": 42.5, "resets_at": 1_800_000_000}, "seven_day": {"used_percentage": 18}}});
        let u = parse(&v).unwrap();
        assert_eq!(u.windows[0], Window { name: "5h".into(), used: 42.5, resets_at: Some(1_800_000_000) });
        assert_eq!(u.windows[1], Window { name: "7d".into(), used: 18.0, resets_at: None });
        assert!(parse(&json!({"model": {}})).is_none());
        assert!(parse(&json!({"rate_limits": {}})).is_none());
        let v = json!({"quota": {"gemini-weekly": {"remaining_fraction": 0.25, "reset_time": "2026-11-14T09:07:49Z", "reset_in_seconds": 60}}});
        let u = parse(&v).unwrap();
        assert_eq!((u.windows[0].name.as_str(), u.windows[0].used), ("gemini-weekly", 75.0));
        assert!(u.windows[0].resets_at.unwrap() - now() <= 60);
        let f = forward_payload(AgentKind::Antigravity, &json!({"email": "x", "quota": v["quota"]})).unwrap();
        assert!(f.get("email").is_none() && f["t1k_agent"] == "antigravity");
    }

    #[test]
    fn reads_codex_rollout_limits() {
        let tail = concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":null,"secondary":{"used_percent":12.5,"window_minutes":10080,"resets_at":1800100000},"primary":{"used_percent":3.0,"window_minutes":300,"resets_at":1800000000}}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"gpt-x","primary":{"used_percent":99.0,"window_minutes":300}}}}"#,
            "\n",
            r#"{"type":"response_item","payload":{}}"#,
        );
        let u = codex_rollout(tail).unwrap();
        assert_eq!(u.windows.iter().map(|w| (w.name.as_str(), w.used)).collect::<Vec<_>>(), vec![("5h", 3.0), ("7d", 12.5)]);
        assert!(codex_rollout("{}").is_none());
    }

    /// The wrapper must keep the original status line working byte for byte, in and out of T1K.
    #[test]
    fn wrapper_runs_the_original_in_a_shell() {
        let cmd = wrapped_command(AgentKind::Claude, Some("cat; echo ' ok'"));
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &cmd])
            .env_remove("T1K_BIN")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(b"{\"a\":1}")?;
                c.wait_with_output()
            })
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "{\"a\":1} ok\n");
    }
}
