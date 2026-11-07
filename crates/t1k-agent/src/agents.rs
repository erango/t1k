//! Coding agents T1K understands, and how each one's hooks map onto T1K's status model.
//!
//! Claude Code is the reference (its hook schema is the internal format). Codex reads a
//! Claude-style `~/.codex/hooks.json`; Gemini CLI (retired by Google in June 2026, still used
//! by some) has its own event names in `~/.gemini/settings.json`; Antigravity CLI (`agy`, Gemini
//! CLI's successor) reads `~/.gemini/config/hooks.json`, keyed by a hook-set name. Grok CLI
//! reads Claude-style files from `~/.grok/hooks/` and sends camelCase payloads.

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AgentKind {
    #[default]
    Claude,
    Codex,
    Gemini,
    Antigravity,
    Grok,
}

impl AgentKind {
    pub const ALL: [AgentKind; 5] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Gemini, AgentKind::Antigravity, AgentKind::Grok];

    pub fn id(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
            AgentKind::Antigravity => "antigravity",
            AgentKind::Grok => "grok",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Gemini => "Gemini CLI",
            AgentKind::Antigravity => "Antigravity CLI",
            AgentKind::Grok => "Grok CLI",
        }
    }

    /// The program name its badge uses (for the program-icon lookup).
    pub fn program(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
            AgentKind::Antigravity => "agy",
            AgentKind::Grok => "grok",
        }
    }

    pub fn from_id(s: &str) -> Option<AgentKind> {
        AgentKind::ALL.into_iter().find(|k| k.id() == s)
    }

    /// The hook events T1K registers for this agent (its own names).
    pub fn events(self) -> &'static [&'static str] {
        match self {
            AgentKind::Claude => crate::hooks::EVENTS,
            AgentKind::Codex => &["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "Interrupt", "SessionEnd"],
            AgentKind::Gemini => &["SessionStart", "BeforeAgent", "BeforeTool", "AfterTool", "Notification", "AfterAgent", "SessionEnd"],
            // PreToolUse is left alone on purpose: its answer decides tool permissions.
            AgentKind::Antigravity => &["PreInvocation", "PostToolUse", "Stop"],
            AgentKind::Grok => &["SessionStart", "UserPromptSubmit", "PostToolUse", "PostToolUseFailure", "Notification", "Stop", "StopFailure", "SessionEnd"],
        }
    }

    /// What `t1k hook` must print for this agent and event (some agents read stdout as JSON).
    pub fn hook_reply(self, event: &str) -> &'static str {
        match (self, event) {
            (AgentKind::Antigravity, "Stop") => "{\"decision\":\"\"}\n",
            (AgentKind::Antigravity, _) => "{}\n",
            _ => "",
        }
    }
}

fn first_str(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).map(str::to_string)
}

/// Turn an agent's hook payload into T1K's internal (Claude-schema) form: `hook_event_name`,
/// `session_id`, `prompt`, `tool_name`, `tool_input`, `message`, plus `t1k_agent`. None means
/// the event doesn't affect status.
pub fn normalize(kind: AgentKind, event: &str, payload: Value) -> Option<Value> {
    let mut v = match payload {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let src = Value::Object(v.clone());
    let tool = first_str(&src, &["tool_name", "toolName", "tool"]);
    let name: &str = match (kind, event) {
        (AgentKind::Claude, e) => e,
        // Codex speaks Claude's schema; asking the user is its "needs you".
        (AgentKind::Codex, "Interrupt") => "Stop",
        (AgentKind::Codex, "PreToolUse") if tool.as_deref() == Some("request_user_input") => {
            v.insert("message".into(), json!("Codex is asking you a question"));
            "Notification"
        }
        (AgentKind::Codex, e) => e,
        (AgentKind::Gemini, "BeforeAgent") => "UserPromptSubmit",
        (AgentKind::Gemini, "AfterAgent") => "Stop",
        (AgentKind::Gemini, "BeforeTool") => "PreToolUse",
        (AgentKind::Gemini, "AfterTool") => "PostToolUse",
        (AgentKind::Gemini, "Notification") => {
            if first_str(&src, &["message"]).is_none() {
                let what = tool.clone().map(|t| format!("Gemini needs your permission to use {t}")).unwrap_or_else(|| "Gemini is waiting for you".into());
                v.insert("message".into(), json!(what));
            }
            "Notification"
        }
        (AgentKind::Gemini, e @ ("SessionStart" | "SessionEnd")) => e,
        (AgentKind::Antigravity, "PreInvocation") => "UserPromptSubmit",
        (AgentKind::Antigravity, "PostToolUse") => "PostToolUse",
        (AgentKind::Antigravity, "Stop") => "Stop",
        (AgentKind::Grok, "PostToolUseFailure") => "PostToolUse",
        (AgentKind::Grok, "StopFailure") => "Stop",
        (AgentKind::Grok, "Notification") => {
            let what = match first_str(&src, &["notificationType", "notification_type"]).as_deref() {
                Some("elicitation_dialog") => "Grok is asking you a question",
                _ => "Grok needs your permission",
            };
            v.insert("message".into(), json!(first_str(&src, &["message"]).unwrap_or_else(|| what.into())));
            "Notification"
        }
        (AgentKind::Grok, e) => e,
        _ => return None,
    };
    v.insert("hook_event_name".into(), json!(name));
    if !v.contains_key("session_id") {
        if let Some(s) = first_str(&src, &["sessionId", "conversation_id", "conversationId", "thread_id", "thread-id"]) {
            v.insert("session_id".into(), json!(s));
        }
    }
    if !v.contains_key("prompt") {
        if let Some(p) = first_str(&src, &["userPrompt", "user_prompt", "input", "query"]) {
            v.insert("prompt".into(), json!(p));
        }
    }
    if let Some(t) = tool {
        v.insert("tool_name".into(), json!(t));
    }
    if !v.contains_key("tool_input") {
        if let Some(i) = src.get("toolInput").or_else(|| src.get("args")).cloned() {
            v.insert("tool_input".into(), i);
        }
    }
    // Only Claude's transcripts are read (other agents log in other formats).
    if kind != AgentKind::Claude {
        let path = v.remove("transcript_path");
        // Codex's rollout file holds its plan usage limits (see statusline::codex_rollout).
        if let (AgentKind::Codex, Some(p)) = (kind, path) {
            v.insert("codex_rollout".into(), p);
        }
    }
    v.insert("t1k_agent".into(), json!(kind.id()));
    Some(Value::Object(v))
}

/// The shell command an agent's hook config runs. Outside T1K it does nothing (and still
/// prints what the agent expects on stdout).
pub fn hook_command(kind: AgentKind, event: &str) -> String {
    let reply = kind.hook_reply(event).trim_end();
    let call = format!("\"$T1K_BIN\" hook --agent {} {event}", kind.id());
    if reply.is_empty() {
        format!("[ -n \"$T1K_BIN\" ] && {call} || true")
    } else {
        format!("if [ -n \"$T1K_BIN\" ]; then {call}; else cat >/dev/null; printf '%s\\n' '{reply}'; fi")
    }
}

fn is_t1k(v: &Value) -> bool {
    v.to_string().contains("T1K_BIN")
}

/// Claude / Codex / Gemini settings: `hooks.<Event>` is a list of `{matcher?, hooks: [..]}`.
fn install_matcher_style(kind: AgentKind, settings: &mut Value, tool_matcher: &str) -> bool {
    if !settings.is_object() {
        *settings = Value::Object(Map::new());
    }
    let hooks = settings.as_object_mut().unwrap().entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let Some(hooks) = hooks.as_object_mut() else { return false };
    let mut changed = false;
    for ev in kind.events() {
        let list = hooks.entry(ev.to_string()).or_insert_with(|| Value::Array(vec![]));
        let Some(list) = list.as_array_mut() else { continue };
        if list.iter().any(is_t1k) {
            continue;
        }
        let command = if kind == AgentKind::Claude { crate::hooks::hook_command(ev) } else { hook_command(kind, ev) };
        let mut entry = json!({ "hooks": [{ "type": "command", "command": command, "timeout": 5 }] });
        if matches!(*ev, "PreToolUse" | "PostToolUse" | "BeforeTool" | "AfterTool" | "PostToolUseFailure") {
            entry["matcher"] = json!(tool_matcher);
        }
        list.push(entry);
        changed = true;
    }
    changed
}

/// Antigravity `hooks.json`: `{ "<set name>": { "<Event>": [...] } }`. Tool events take
/// `{matcher, hooks: [..]}`, the others a bare list of commands.
fn install_antigravity(config: &mut Value) -> bool {
    if !config.is_object() {
        *config = Value::Object(Map::new());
    }
    let obj = config.as_object_mut().unwrap();
    if obj.get("t1k-status").is_some_and(is_t1k) {
        return false;
    }
    let mut set = Map::new();
    for ev in AgentKind::Antigravity.events() {
        let cmd = json!({ "type": "command", "command": hook_command(AgentKind::Antigravity, ev), "timeout": 5 });
        let entry = if *ev == "PostToolUse" { json!([{ "matcher": "*", "hooks": [cmd] }]) } else { json!([cmd]) };
        set.insert(ev.to_string(), entry);
    }
    obj.insert("t1k-status".into(), Value::Object(set));
    true
}

/// Add T1K's hooks to an agent's config value. Returns true if anything changed. Idempotent.
pub fn install(kind: AgentKind, config: &mut Value) -> bool {
    match kind {
        // Claude also gets the status-line wrapper, the only source of its plan usage limits.
        AgentKind::Claude => install_matcher_style(kind, config, "*") | crate::statusline::wrap(kind, config),
        AgentKind::Codex => install_matcher_style(kind, config, "*"),
        // Gemini matchers are regular expressions.
        AgentKind::Gemini => install_matcher_style(kind, config, ".*"),
        AgentKind::Antigravity => install_antigravity(config),
        // Grok reads every file in ~/.grok/hooks/; T1K owns its own file there.
        AgentKind::Grok => {
            let changed = install_matcher_style(kind, config, "*");
            // Only the blocking notification types mean "needs you".
            if let Some(list) = config["hooks"]["Notification"].as_array_mut() {
                for e in list.iter_mut().filter(|e| is_t1k(e)) {
                    e["matcher"] = json!("^(permission_prompt|elicitation_dialog)$");
                }
            }
            changed
        }
    }
}

/// Whether a config already has T1K's hooks.
pub fn installed(kind: AgentKind, config: &Value) -> bool {
    match kind {
        AgentKind::Antigravity => config.get("t1k-status").is_some_and(is_t1k),
        AgentKind::Claude => config.get("hooks").is_some_and(is_t1k) && crate::statusline::is_wrapped(config),
        _ => config.get("hooks").is_some_and(is_t1k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_gemini_and_antigravity_events() {
        let v = normalize(AgentKind::Gemini, "BeforeTool", json!({"tool_name":"write_file","tool_input":{"file_path":"a.rs"},"session_id":"g1"})).unwrap();
        assert_eq!(v["hook_event_name"], "PreToolUse");
        assert_eq!(v["t1k_agent"], "gemini");
        let v = normalize(AgentKind::Gemini, "AfterAgent", json!({})).unwrap();
        assert_eq!(v["hook_event_name"], "Stop");
        let v = normalize(AgentKind::Antigravity, "PreInvocation", json!({"conversationId":"c9","userPrompt":"fix it"})).unwrap();
        assert_eq!((v["hook_event_name"].as_str(), v["session_id"].as_str(), v["prompt"].as_str()), (Some("UserPromptSubmit"), Some("c9"), Some("fix it")));
        assert!(normalize(AgentKind::Antigravity, "PostInvocation", json!({})).is_none());
    }

    #[test]
    fn codex_question_needs_you() {
        let v = normalize(AgentKind::Codex, "PreToolUse", json!({"hook_event_name":"PreToolUse","tool_name":"request_user_input","transcript_path":"/x"})).unwrap();
        assert_eq!(v["hook_event_name"], "Notification");
        assert!(v.get("transcript_path").is_none());
        let v = normalize(AgentKind::Codex, "Interrupt", json!({})).unwrap();
        assert_eq!(v["hook_event_name"], "Stop");
    }

    #[test]
    fn installs_idempotently() {
        let mut codex = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "other"}]}]}});
        assert!(install(AgentKind::Codex, &mut codex));
        assert!(!install(AgentKind::Codex, &mut codex));
        assert_eq!(codex["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(installed(AgentKind::Codex, &codex));

        let mut agy = json!({"orca-status": {"Stop": []}});
        assert!(install(AgentKind::Antigravity, &mut agy));
        assert!(!install(AgentKind::Antigravity, &mut agy));
        assert!(agy.get("orca-status").is_some());
        assert_eq!(agy["t1k-status"]["PostToolUse"][0]["matcher"], "*");
        assert!(agy["t1k-status"]["Stop"][0]["command"].as_str().unwrap().contains("decision"));

        let mut grok = json!({});
        install(AgentKind::Grok, &mut grok);
        assert_eq!(grok["hooks"]["Notification"][0]["matcher"], "^(permission_prompt|elicitation_dialog)$");
        let v = normalize(AgentKind::Grok, "Notification", json!({"hookEventName":"notification","notificationType":"elicitation_dialog","sessionId":"k"})).unwrap();
        assert_eq!((v["hook_event_name"].as_str(), v["session_id"].as_str()), (Some("Notification"), Some("k")));

        let mut gem = json!({});
        install(AgentKind::Gemini, &mut gem);
        assert_eq!(gem["hooks"]["BeforeTool"][0]["matcher"], ".*");
    }
}

#[cfg(test)]
mod end_to_end {
    use super::*;
    use crate::hooks::Hook;

    #[test]
    fn normalized_payloads_parse_as_hooks() {
        let cases = [
            (AgentKind::Codex, "UserPromptSubmit", json!({"hook_event_name":"UserPromptSubmit","prompt":"fix login","session_id":"c1"})),
            (AgentKind::Gemini, "BeforeTool", json!({"tool_name":"replace","tool_input":{"file_path":"/x/src/a.rs"}})),
            (AgentKind::Antigravity, "Stop", json!({})),
            (AgentKind::Grok, "Notification", json!({"notificationType":"permission_prompt"})),
        ];
        let got: Vec<String> = cases
            .into_iter()
            .map(|(k, e, p)| format!("{:?}", Hook::parse(&normalize(k, e, p).unwrap()).unwrap().0))
            .collect();
        assert!(got[0].starts_with("UserPromptSubmit"), "{got:?}");
        assert!(got[1].starts_with("PreToolUse"), "{got:?}");
        assert_eq!(got[2], "Stop");
        assert!(got[3].contains("Grok needs your permission"), "{got:?}");
    }
}
