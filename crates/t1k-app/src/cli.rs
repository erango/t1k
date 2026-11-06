//! Non-GUI subcommands: `t1k hook <Event>` and `t1k setup-claude`.

use std::io::Read;
use std::path::PathBuf;

use t1k_agent::server::{send, Envelope};

/// Handle a subcommand if present. Returns true if the process should exit.
pub fn run(args: &[String]) -> bool {
    match args.get(1).map(String::as_str) {
        Some("hook") => {
            hook();
            true
        }
        Some("setup-claude") => {
            if let Err(e) = setup_claude() {
                eprintln!("t1k: {e}");
                std::process::exit(1);
            }
            true
        }
        _ => false,
    }
}

/// Forward a Claude Code hook payload (stdin) to the T1K window that owns this tab.
/// Never fails loudly: a hook must not disturb Claude, and it no-ops outside T1K.
fn hook() {
    let (Some(tab_id), Some(socket)) = (std::env::var("T1K_TAB_ID").ok(), std::env::var_os("T1K_SOCKET")) else {
        return;
    };
    let mut buf = Vec::new();
    if std::io::stdin().take(8 << 20).read_to_end(&mut buf).is_err() {
        return;
    }
    let Ok(mut payload) = serde_json::from_slice::<serde_json::Value>(&buf) else { return };
    // Tool output can be huge and isn't needed for status.
    if let Some(o) = payload.as_object_mut() {
        o.remove("tool_response");
    }
    let _ = send(&PathBuf::from(socket), &Envelope { tab_id, payload });
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
    println!("Installed T1K hooks in {}. They do nothing outside T1K.", path.display());
    Ok(())
}
