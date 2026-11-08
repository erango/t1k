//! Where each coding agent keeps its hook config, whether it's installed, and installing
//! T1K's hooks there (with a backup).

use std::path::PathBuf;

use t1k_agent::agents::{self, AgentKind};

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The file T1K's hooks go in.
pub fn config_path(kind: AgentKind) -> Option<PathBuf> {
    let h = home()?;
    Some(match kind {
        AgentKind::Claude => h.join(".claude/settings.json"),
        AgentKind::Codex => h.join(".codex/hooks.json"),
        AgentKind::Gemini => h.join(".gemini/settings.json"),
        AgentKind::Antigravity => h.join(".gemini/config/hooks.json"),
        AgentKind::Grok => h.join(".grok/hooks/t1k.json"),
    })
}

fn on_path(bin: &str) -> bool {
    crate::platform::tool_path().split(':').any(|d| std::path::Path::new(d).join(bin).is_file())
}

/// Whether this agent seems to be installed (its binary on PATH, or its config folder).
pub fn detected(kind: AgentKind) -> bool {
    let h = home().unwrap_or_default();
    match kind {
        AgentKind::Claude => true,
        AgentKind::Codex => on_path("codex") || h.join(".codex").is_dir(),
        AgentKind::Gemini => on_path("gemini"),
        AgentKind::Antigravity => on_path("agy") || h.join(".gemini/antigravity-cli").is_dir(),
        AgentKind::Grok => on_path("grok") || h.join(".grok").is_dir(),
    }
}

fn read(kind: AgentKind) -> Option<serde_json::Value> {
    let b = std::fs::read(config_path(kind)?).ok()?;
    serde_json::from_slice(&b).ok()
}

/// Antigravity keeps its status line apart from its hooks. (Claude's is in the hooks file and
/// handled by `agents::install`.)
fn statusline_path(kind: AgentKind) -> Option<PathBuf> {
    (kind == AgentKind::Antigravity).then(|| home().map(|h| h.join(".gemini/antigravity-cli/settings.json")))?
}

fn read_json(path: &std::path::Path) -> anyhow::Result<serde_json::Value> {
    Ok(match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b)?,
        Err(_) => serde_json::json!({}),
    })
}

fn write_json(path: &std::path::Path, v: &serde_json::Value) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::copy(path, path.with_extension("json.t1k-backup"))?;
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}

pub fn installed(kind: AgentKind) -> bool {
    read(kind).is_some_and(|v| agents::installed(kind, &v))
        && statusline_path(kind).is_none_or(|p| read_json(&p).is_ok_and(|v| t1k_agent::statusline::is_wrapped(&v)))
}

/// Install T1K's hooks (and, where the agent reports usage limits there, the status-line
/// wrapper) for one agent. Ok(true) if a file changed.
pub fn install(kind: AgentKind) -> anyhow::Result<bool> {
    let path = config_path(kind).ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let mut config = read_json(&path)?;
    let mut changed = false;
    if agents::install(kind, &mut config) {
        write_json(&path, &config)?;
        changed = true;
    }
    if let Some(path) = statusline_path(kind) {
        let mut settings = read_json(&path)?;
        if t1k_agent::statusline::wrap(kind, &mut settings) {
            write_json(&path, &settings)?;
            changed = true;
        }
    }
    Ok(changed)
}

/// Whether T1K's hooks are in this agent's config (for Claude, whatever its status line).
pub fn hooked(kind: AgentKind) -> bool {
    read(kind).is_some_and(|v| agents::hooks_installed(kind, &v))
}

/// Take T1K's hooks out again (and Antigravity's status-line wrapper, which `install` added
/// with them). Claude's status line has its own switch, `set_claude_limits`. Ok(true) if a
/// file changed; each file is backed up before it's written.
pub fn remove(kind: AgentKind) -> anyhow::Result<bool> {
    let path = config_path(kind).ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let mut changed = false;
    if path.exists() {
        let mut config = read_json(&path)?;
        if agents::uninstall(kind, &mut config) {
            write_json(&path, &config)?;
            changed = true;
        }
    }
    if let Some(path) = statusline_path(kind).filter(|p| p.exists()) {
        let mut settings = read_json(&path)?;
        if t1k_agent::statusline::unwrap(&mut settings) {
            write_json(&path, &settings)?;
            changed = true;
        }
    }
    Ok(changed)
}

/// Whether Claude Code's status line goes through T1K (its plan usage limits reach the meter).
pub fn claude_limits() -> bool {
    read(AgentKind::Claude).is_some_and(|v| t1k_agent::statusline::is_wrapped(&v))
}

/// Wrap or unwrap Claude Code's status line, as `setup-claude` does. Ok(true) if it changed.
pub fn set_claude_limits(on: bool) -> anyhow::Result<bool> {
    let path = config_path(AgentKind::Claude).ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let mut settings = read_json(&path)?;
    let changed = if on { t1k_agent::statusline::wrap(AgentKind::Claude, &mut settings) } else { t1k_agent::statusline::unwrap(&mut settings) };
    if changed {
        write_json(&path, &settings)?;
    }
    Ok(changed)
}

/// Install for every detected agent; returns a line per agent for the CLI.
pub fn install_all() -> Vec<String> {
    AgentKind::ALL
        .into_iter()
        .filter(|k| detected(*k))
        .map(|k| {
            let where_ = config_path(k).map(|p| p.display().to_string()).unwrap_or_default();
            match install(k) {
                Ok(true) => format!("{}: installed in {where_} (backup next to it)", k.label()),
                Ok(false) => format!("{}: already installed in {where_}", k.label()),
                Err(e) => format!("{}: {e}", k.label()),
            }
        })
        .collect()
}

