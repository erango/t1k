//! Claude Code hook payloads and how they're installed.

use serde_json::Value;

/// Hook events T1K listens to.
pub const EVENTS: &[&str] =
    &["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Notification", "Stop", "SessionEnd"];

#[derive(Debug, Clone, PartialEq)]
pub enum Hook {
    SessionStart { session_id: String, transcript_path: Option<String>, cwd: Option<String> },
    UserPromptSubmit { prompt: String },
    PreToolUse { tool: String, input: Value },
    PostToolUse { tool: String },
    Notification { message: String },
    Stop,
    SessionEnd,
    Other(String),
}

impl Hook {
    pub fn parse(v: &Value) -> Option<(Hook, Option<String>)> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let session = s("session_id");
        let hook = match s("hook_event_name")?.as_str() {
            "SessionStart" => Hook::SessionStart {
                session_id: session.clone().unwrap_or_default(),
                transcript_path: s("transcript_path"),
                cwd: s("cwd"),
            },
            "UserPromptSubmit" => Hook::UserPromptSubmit { prompt: s("prompt").unwrap_or_default() },
            "PreToolUse" => Hook::PreToolUse {
                tool: s("tool_name").unwrap_or_default(),
                input: v.get("tool_input").cloned().unwrap_or(Value::Null),
            },
            "PostToolUse" => Hook::PostToolUse { tool: s("tool_name").unwrap_or_default() },
            "Notification" => Hook::Notification { message: s("message").unwrap_or_default() },
            "Stop" => Hook::Stop,
            "SessionEnd" => Hook::SessionEnd,
            other => Hook::Other(other.to_string()),
        };
        Some((hook, session))
    }
}

fn short_path(p: &str) -> &str {
    // Keep the last two components: "src/auth.rs".
    let mut it = p.rmatch_indices('/');
    match (it.next(), it.next()) {
        (Some(_), Some((i, _))) => &p[i + 1..],
        _ => p,
    }
}

fn clip(s: &str, n: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() <= n {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(n).collect::<String>())
    }
}

/// Plain-words description of a tool call, for the "now" line: "Editing src/auth.rs".
pub fn describe_tool(tool: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    match tool {
        "Edit" | "MultiEdit" => format!("Editing {}", short_path(s("file_path"))),
        "Write" => format!("Writing {}", short_path(s("file_path"))),
        "Read" => format!("Reading {}", short_path(s("file_path"))),
        "NotebookEdit" => format!("Editing {}", short_path(s("notebook_path"))),
        "Bash" => {
            let d = s("description");
            if d.is_empty() { format!("Running {}", clip(s("command"), 48)) } else { clip(d, 60) }
        }
        "Grep" => format!("Searching for {}", clip(s("pattern"), 40)),
        "Glob" => format!("Finding {}", clip(s("pattern"), 40)),
        "WebFetch" => format!("Fetching {}", clip(s("url"), 50)),
        "WebSearch" => format!("Searching the web: {}", clip(s("query"), 40)),
        "Agent" | "Task" => {
            let d = s("description");
            if d.is_empty() { "Running a subagent".into() } else { format!("Subagent: {}", clip(d, 50)) }
        }
        "TodoWrite" => "Updating the plan".into(),
        t if t.starts_with("mcp__") => {
            let name = t.rsplit("__").next().unwrap_or(t).replace('_', " ");
            format!("Using {name}")
        }
        t => format!("Using {t}"),
    }
}

/// Shell snippet for a settings.json hook command. Exits instantly outside T1K.
pub fn hook_command(event: &str) -> String {
    format!("[ -n \"$T1K_BIN\" ] && \"$T1K_BIN\" hook {event} || true")
}

/// Add T1K's hooks to a Claude Code settings.json value. Returns true if anything changed.
/// Idempotent: existing T1K entries (recognized by `$T1K_BIN`) are left alone.
pub fn install_into(settings: &mut Value) -> bool {
    if !settings.is_object() {
        *settings = Value::Object(Default::default());
    }
    let hooks = settings.as_object_mut().unwrap().entry("hooks").or_insert_with(|| Value::Object(Default::default()));
    let Some(hooks) = hooks.as_object_mut() else { return false };
    let mut changed = false;
    for ev in EVENTS {
        let list = hooks.entry(ev.to_string()).or_insert_with(|| Value::Array(vec![]));
        let Some(list) = list.as_array_mut() else { continue };
        let present = list.iter().any(|m| m.to_string().contains("T1K_BIN"));
        if present {
            continue;
        }
        let mut entry = serde_json::json!({
            "hooks": [{ "type": "command", "command": hook_command(ev), "timeout": 5 }]
        });
        if matches!(*ev, "PreToolUse" | "PostToolUse") {
            entry["matcher"] = Value::String("*".into());
        }
        list.push(entry);
        changed = true;
    }
    changed | crate::statusline::wrap(crate::agents::AgentKind::Claude, settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_and_describe() {
        let v = json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Edit","tool_input":{"file_path":"/a/b/src/auth.rs"}});
        let (h, sid) = Hook::parse(&v).unwrap();
        assert_eq!(sid.as_deref(), Some("s"));
        let Hook::PreToolUse { tool, input } = h else { panic!() };
        assert_eq!(describe_tool(&tool, &input), "Editing src/auth.rs");
        assert_eq!(describe_tool("Bash", &json!({"command":"cargo test --all"})), "Running cargo test --all");
        assert_eq!(describe_tool("mcp__claude_ai_Linear__get_issue", &json!({})), "Using get issue");
    }

    #[test]
    fn install_is_idempotent_and_keeps_others() {
        let mut s = json!({"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"rtk hook claude"}]}]},"x":1});
        assert!(install_into(&mut s));
        assert!(!install_into(&mut s));
        assert_eq!(s["x"], 1);
        assert_eq!(s["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(s["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }
}
