//! Claude Code's plan usage limits (the 5-hour and weekly windows).
//!
//! Claude Code reports them only to its status-line command, on stdin, as
//! `rate_limits.{five_hour,seven_day}.{used_percentage,resets_at}` (Pro/Max plans, after the
//! first reply of a session). No hook, transcript or file carries them. So T1K wraps the
//! user's status-line command: the wrapper hands the JSON to `t1k statusline`, which forwards
//! the limits to the app, then runs the original command on the same input, so the status
//! line looks exactly as before. Outside T1K (`$T1K_BIN` unset) only the original runs.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// The `hook_event_name` T1K uses for forwarded limits (not a real Claude hook).
pub const EVENT: &str = "T1KStatusLine";

const HEAD: &str = r#"input=$(cat); [ -n "$T1K_BIN" ] && printf '%s' "$input" | "$T1K_BIN" statusline; "#;
const CHAIN_OPEN: &str = r#"printf '%s' "$input" | { "#;
const CHAIN_CLOSE: &str = "\n}";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    /// 0–100.
    pub used: f32,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub five_hour: Option<Limit>,
    pub seven_day: Option<Limit>,
    /// Unix seconds when Claude last reported them.
    pub at: i64,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn limit(v: &Value) -> Option<Limit> {
    let used = v.get("used_percentage")?.as_f64()? as f32;
    let resets_at = v.get("resets_at").and_then(|r| r.as_i64().or_else(|| r.as_f64().map(|f| f as i64)));
    Some(Limit { used: used.clamp(0.0, 100.0), resets_at })
}

/// The limits in a status-line (or forwarded) payload; None when it has neither window.
pub fn parse(v: &Value) -> Option<Usage> {
    let r = v.get("rate_limits")?;
    let usage = Usage { five_hour: r.get("five_hour").and_then(limit), seven_day: r.get("seven_day").and_then(limit), at: now() };
    (usage.five_hour.is_some() || usage.seven_day.is_some()).then_some(usage)
}

/// What `t1k statusline` sends to the app: only the limits, nothing about the session.
pub fn forward_payload(statusline_json: &Value) -> Option<Value> {
    let r = statusline_json.get("rate_limits")?;
    Some(json!({ "hook_event_name": EVENT, "rate_limits": r }))
}

/// The status-line command that forwards to T1K and then runs `original` (if any).
pub fn wrapped_command(original: Option<&str>) -> String {
    match original.map(str::trim).filter(|o| !o.is_empty()) {
        Some(o) => format!("{HEAD}{CHAIN_OPEN}{o}{CHAIN_CLOSE}"),
        None => format!("{HEAD}true"),
    }
}

/// The user's own command inside a wrapped one (None if there was none).
pub fn original_command(wrapped: &str) -> Option<&str> {
    wrapped.strip_prefix(HEAD)?.strip_prefix(CHAIN_OPEN)?.strip_suffix(CHAIN_CLOSE)
}

pub fn is_wrapped(settings: &Value) -> bool {
    settings.pointer("/statusLine/command").and_then(Value::as_str).is_some_and(|c| c.starts_with(HEAD))
}

/// Wrap the status line in a Claude Code settings.json value. Returns true if it changed.
/// Keeps the user's command and its other fields (padding, refreshInterval).
pub fn wrap(settings: &mut Value) -> bool {
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
    line.insert("type".into(), json!("command"));
    line.insert("command".into(), json!(wrapped_command(original.as_deref())));
    obj.insert("statusLine".into(), Value::Object(line));
    true
}

/// Undo `wrap`: restore the user's command, or remove the status line T1K added.
pub fn unwrap(settings: &mut Value) -> bool {
    let Some(cmd) = settings.pointer("/statusLine/command").and_then(Value::as_str).map(str::to_string) else { return false };
    if !cmd.starts_with(HEAD) {
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
        assert!(wrap(&mut s));
        assert!(!wrap(&mut s), "idempotent");
        let cmd = s["statusLine"]["command"].as_str().unwrap();
        assert_eq!(original_command(cmd), Some("bash ~/x/wrapper.sh"));
        assert_eq!(s["statusLine"]["padding"], 0);
        assert!(unwrap(&mut s));
        assert_eq!(s, json!({"statusLine": {"type": "command", "command": "bash ~/x/wrapper.sh", "padding": 0}}));
    }

    #[test]
    fn adds_and_removes_one_when_there_was_none() {
        let mut s = json!({"model": "opus"});
        assert!(wrap(&mut s));
        assert!(is_wrapped(&s));
        assert!(unwrap(&mut s));
        assert_eq!(s, json!({"model": "opus"}));
    }

    #[test]
    fn parses_limits() {
        let v = json!({"rate_limits": {"five_hour": {"used_percentage": 42.5, "resets_at": 1_800_000_000}, "seven_day": {"used_percentage": 18}}});
        let u = parse(&v).unwrap();
        assert_eq!(u.five_hour, Some(Limit { used: 42.5, resets_at: Some(1_800_000_000) }));
        assert_eq!(u.seven_day, Some(Limit { used: 18.0, resets_at: None }));
        assert!(parse(&json!({"model": {}})).is_none());
        assert!(parse(&json!({"rate_limits": {}})).is_none());
    }

    /// The wrapper must keep the original status line working byte for byte, in and out of T1K.
    #[test]
    fn wrapper_runs_the_original_in_a_shell() {
        let cmd = wrapped_command(Some("cat; echo ' ok'"));
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
