//! Non-GUI subcommands: `t1k hook [--agent X] <Event>`, `t1k setup-claude`, `t1k setup-agents`.

use std::io::Read;
use std::path::PathBuf;

use t1k_agent::server::{send, Envelope};

/// Handle a subcommand if present. Returns true if the process should exit.
pub fn run(args: &[String]) -> bool {
    match args.get(1).map(String::as_str) {
        Some("hook") => {
            hook(&args[2..]);
            true
        }
        Some("statusline") => {
            statusline(&args[2..]);
            true
        }
        // List tabs open in other terminals (read-only). `--titles` also asks the apps that
        // can say for tab names (macOS may ask to allow it).
        Some("import") if args.iter().any(|a| a == "--list") => {
            let mut found = crate::import::scan();
            if args.iter().any(|a| a == "--titles") {
                let mut sources: Vec<_> = found.iter().map(|f| f.source).collect();
                sources.dedup();
                crate::import::apply_titles(&mut found, &crate::import::titles(&sources));
            }
            for f in &found {
                let home = std::env::var("HOME").unwrap_or_default();
                let cwd = f.cwd.as_ref().map(|c| c.display().to_string().replacen(&home, "~", 1)).unwrap_or_default();
                let claude = f.claude.as_ref().map(|c| format!("  claude {} ({})", &c.session_id[..8.min(c.session_id.len())], if c.busy { "busy" } else { "idle" })).unwrap_or_default();
                let running = f.running.as_ref().map(|r| format!("  running: {r}")).unwrap_or_default();
                let title = f.title.as_ref().map(|t| format!("  \"{t}\"")).unwrap_or_default();
                println!("{:<9} {:<8} {cwd}{title}{claude}{running}", f.source.label(), f.tty);
            }
            true
        }
        Some("theme") => {
            theme_cmd(&args[2..]);
            true
        }
        Some("setup-claude") => {
            if let Err(e) = setup_claude() {
                eprintln!("t1k: {e}");
                std::process::exit(1);
            }
            true
        }
        // Hooks for every agent found: Claude Code, Codex, Gemini CLI, Antigravity CLI, Grok.
        Some("setup-agents") => {
            for line in crate::agent_setup::install_all() {
                println!("{line}");
            }
            println!("The hooks do nothing outside T1K.");
            true
        }
        Some("helper") => {
            crate::helper_cli::run(&args[2..]);
            true
        }
        _ => false,
    }
}

/// Forward an agent's hook payload (stdin) to the T1K window that owns this tab:
/// `t1k hook [--agent claude|codex|gemini|antigravity|grok] <Event>`. Never fails loudly: a hook
/// must not disturb the agent, and it no-ops outside T1K (still printing what the agent expects).
fn hook(args: &[String]) {
    use t1k_agent::agents::{normalize, AgentKind};
    let mut kind = AgentKind::Claude;
    let mut event = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--agent" {
            kind = it.next().and_then(|k| AgentKind::from_id(k)).unwrap_or(AgentKind::Claude);
        } else if event.is_none() {
            event = Some(a.clone());
        }
    }
    let event = event.unwrap_or_default();
    print!("{}", kind.hook_reply(&event));
    let (Some(tab_id), Some(socket)) = (std::env::var("T1K_TAB_ID").ok(), std::env::var_os("T1K_SOCKET")) else {
        return;
    };
    let mut buf = Vec::new();
    if std::io::stdin().take(8 << 20).read_to_end(&mut buf).is_err() {
        return;
    }
    let raw = serde_json::from_slice::<serde_json::Value>(&buf).unwrap_or(serde_json::json!({}));
    // Claude's own hooks carry the event in the payload; other agents' come from argv.
    let payload = if kind == AgentKind::Claude { Some(raw) } else { normalize(kind, &event, raw) };
    let Some(mut payload) = payload else { return };
    // Tool output can be huge and isn't needed for status.
    if let Some(o) = payload.as_object_mut() {
        o.remove("tool_response");
    }
    let _ = send(&PathBuf::from(socket), &Envelope { tab_id, payload });
}

/// Forward an agent's plan usage limits from its status-line input (stdin) to the T1K window that
/// owns this tab. Prints nothing: the wrapper runs the user's own status line after this.
fn statusline(args: &[String]) {
    use t1k_agent::agents::AgentKind;
    let kind = match args {
        [flag, k, ..] if flag == "--agent" => AgentKind::from_id(k).unwrap_or(AgentKind::Claude),
        _ => AgentKind::Claude,
    };
    let (Some(tab_id), Some(socket)) = (std::env::var("T1K_TAB_ID").ok(), std::env::var_os("T1K_SOCKET")) else {
        return;
    };
    let mut buf = Vec::new();
    if std::io::stdin().take(4 << 20).read_to_end(&mut buf).is_err() {
        return;
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) else { return };
    if let Some(payload) = t1k_agent::statusline::forward_payload(kind, &v) {
        let _ = send(&PathBuf::from(socket), &Envelope { tab_id, payload });
    }
}

/// Add T1K's hooks to ~/.claude/settings.json (backing it up first).
fn setup_claude() -> anyhow::Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let path = PathBuf::from(home).join(".claude/settings.json");
    let mut settings: serde_json::Value = match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b)?,
        Err(_) => serde_json::json!({}),
    };
    if !t1k_agent::hooks::install_into(&mut settings) {
        println!("T1K hooks already installed in {}", path.display());
        return Ok(());
    }
    if path.exists() {
        let backup = path.with_extension("json.t1k-backup");
        std::fs::copy(&path, &backup)?;
        println!("Backed up {} to {}", path.display(), backup.display());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_vec_pretty(&settings)?)?;
    println!("Installed T1K hooks and the status-line wrapper in {}. They do nothing outside T1K; your status line looks the same.", path.display());
    Ok(())
}

fn theme_cmd(args: &[String]) {
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("import"), Some(_)) => {
            for f in &args[1..] {
                match import_theme(std::path::Path::new(f)) {
                    Ok(name) => println!("Imported \"{name}\". Pick it from View › Dark/Light Theme, or set it in config.toml."),
                    Err(e) => {
                        eprintln!("t1k: {f}: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        (Some("list"), _) => {
            if let Some(d) = crate::config::themes_dir() {
                t1k_themes::load_user_themes(&d);
            }
            for t in t1k_themes::all() {
                println!("{:<24} {}", t.name, if t.is_dark() { "dark" } else { "light" });
            }
        }
        _ => {
            eprintln!("usage: t1k theme import <file>...   (Ghostty, iTerm2 .itermcolors, Alacritty, Windows Terminal, base16)");
            eprintln!("       t1k theme list");
            std::process::exit(2);
        }
    }
}

/// Validate a theme file and copy it into the themes folder. Returns the theme's name.
pub fn import_theme(path: &std::path::Path) -> Result<String, String> {
    let theme = t1k_themes::import::load(path)?;
    let dir = crate::config::themes_dir().ok_or("HOME not set")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = path.file_name().ok_or("bad path")?;
    std::fs::copy(path, dir.join(file)).map_err(|e| e.to_string())?;
    Ok(theme.name.to_string())
}
