//! Bring tabs over from other terminals.
//!
//! Every terminal looks the same from below: a terminal app process, a tty per tab, a shell on
//! it, maybe a program in the foreground. Scanning processes finds each tab's folder and
//! what runs there in any terminal, with no permission needed. Claude Code sessions are exact:
//! Claude writes `~/.claude/sessions/<pid>.json` with the session id and whether it's busy.
//! Terminals that can say more (tab titles and order) are asked separately, only when the
//! user agrees (iTerm2 and Terminal.app go through AppleScript, which macOS asks about).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;
use t1k_terminal::process;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
pub enum Source {
    ITerm2,
    Terminal,
    Ghostty,
    Warp,
    WezTerm,
    Kitty,
    Alacritty,
    Tmux,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::ITerm2 => "iTerm2",
            Source::Terminal => "Terminal",
            Source::Ghostty => "Ghostty",
            Source::Warp => "Warp",
            Source::WezTerm => "WezTerm",
            Source::Kitty => "kitty",
            Source::Alacritty => "Alacritty",
            Source::Tmux => "tmux",
        }
    }

    /// Which terminal a process belongs to, from its executable path and name.
    fn of(path: &str, name: &str) -> Option<Source> {
        let p = path.to_ascii_lowercase();
        let n = name.to_ascii_lowercase();
        Some(if p.contains("/iterm.app/") || n.starts_with("itermserver") {
            Source::ITerm2
        } else if p.contains("/terminal.app/") {
            Source::Terminal
        } else if p.contains("/ghostty.app/") {
            Source::Ghostty
        // Warp updates itself in place: a running copy's path can be its moved-aside binary
        // under Application Support/dev.warp.Warp-Stable.
        } else if p.contains("/warp.app/") || p.contains("/warppreview.app/") || p.contains("/dev.warp.") {
            Source::Warp
        } else if p.contains("/wezterm.app/") || n.starts_with("wezterm") {
            Source::WezTerm
        } else if p.contains("/kitty.app/") || n == "kitty" {
            Source::Kitty
        } else if p.contains("/alacritty.app/") || n == "alacritty" {
            Source::Alacritty
        } else if n == "tmux" {
            Source::Tmux
        } else {
            return None;
        })
    }
}

/// A Claude Code session running in a tab.
#[derive(Clone, Debug, Serialize)]
pub struct Claude {
    pub pid: i32,
    pub session_id: String,
    /// Working on a turn right now (never interrupted for a move).
    pub busy: bool,
    pub name: Option<String>,
}

/// One tab (or tmux pane) found in another terminal.
#[derive(Clone, Debug, Serialize)]
pub struct Found {
    pub source: Source,
    pub tty: String,
    pub cwd: Option<PathBuf>,
    /// The foreground program when it isn't the shell ("npm run dev", "nvim notes.md").
    pub running: Option<String>,
    pub claude: Option<Claude>,
    /// From the app, when it can say (and the user let us ask).
    pub title: Option<String>,
    /// (window, tab) order from the app, when known.
    pub order: Option<(u32, u32)>,
}

const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "nu", "xonsh", "elvish", "tcsh", "ksh", "login"];

fn is_shell(name: &str) -> bool {
    SHELLS.contains(&name.trim_start_matches('-'))
}

/// Scan this user's processes for tabs in other terminals.
pub fn scan() -> Vec<Found> {
    let me = unsafe { libc::getuid() };
    let own = std::process::id() as i32;
    let procs: HashMap<i32, process::Info> = process::all_pids().into_iter().filter_map(process::info).filter(|i| i.uid == me).map(|i| (i.pid, i)).collect();
    let names: HashMap<i32, String> = procs.keys().map(|&p| (p, process::name(p).unwrap_or_default())).collect();

    // Terminal app processes, and T1K's own (never imported).
    let mut source_of: HashMap<i32, Option<Source>> = HashMap::new();
    for &pid in procs.keys() {
        let path = process::path(pid).unwrap_or_default();
        let name = &names[&pid];
        let t1k = pid == own || path.contains("/T1K.app/") || name == "t1k";
        source_of.insert(pid, if t1k { None } else { Source::of(&path, name) });
    }
    let is_t1k = |pid: i32| pid == own || process::path(pid).is_some_and(|p| p.contains("/T1K.app/")) || names.get(&pid).is_some_and(|n| n == "t1k");

    // Walk up from a process to the terminal that owns it.
    let owner = |mut pid: i32| -> Option<Source> {
        for _ in 0..32 {
            if is_t1k(pid) {
                return None;
            }
            if let Some(Some(s)) = source_of.get(&pid) {
                return Some(*s);
            }
            pid = procs.get(&pid)?.ppid;
            if pid <= 1 {
                return None;
            }
        }
        None
    };

    // Processes by tty.
    let mut by_tty: HashMap<u32, Vec<&process::Info>> = HashMap::new();
    for i in procs.values() {
        if i.tdev != 0 && i.tdev != u32::MAX {
            by_tty.entry(i.tdev).or_default().push(i);
        }
    }

    let mut out = Vec::new();
    for (dev, list) in by_tty {
        // The tab's shell: a shell whose parent isn't on this tty (the top one).
        let on_tty = |p: i32| list.iter().any(|i| i.pid == p);
        let mut tops: Vec<&&process::Info> = list.iter().filter(|i| !on_tty(i.ppid)).collect();
        tops.sort_by_key(|i| i.pid);
        let Some(top) = tops.iter().find(|i| is_shell(&names[&i.pid])).or(tops.first()) else { continue };
        let Some(source) = owner(top.pid) else { continue };
        // `login` sits above the shell in Terminal.app and iTerm2: use the shell under it.
        let shell = if names[&top.pid] == "login" { list.iter().filter(|i| i.ppid == top.pid).min_by_key(|i| i.pid).copied().unwrap_or(top) } else { top };
        let fg = list.iter().find(|i| i.pid == top.tpgid).copied();
        let fg_name = fg.map(|f| names[&f.pid].clone()).unwrap_or_default();
        // A tmux client tab: its panes are found on their own.
        if fg_name == "tmux" && source != Source::Tmux {
            continue;
        }
        let claude = list.iter().filter(|i| names[&i.pid] == "claude").find_map(|i| claude_session(i.pid));
        let running = fg.filter(|f| !is_shell(&names[&f.pid]) && names[&f.pid] != "claude").and_then(|f| process::command(f.pid));
        let cwd = claude.as_ref().and_then(|c| claude_cwd(c.pid)).or_else(|| process::cwd(shell.pid));
        out.push(Found { source, tty: process::tty_name(dev).unwrap_or_default(), cwd, running, claude, title: None, order: None });
    }
    out.sort_by(|a, b| (a.source, &a.tty).cmp(&(b.source, &b.tty)));
    out
}

fn sessions_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".claude/sessions"))
}

fn claude_file(pid: i32) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(sessions_dir()?.join(format!("{pid}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

fn claude_session(pid: i32) -> Option<Claude> {
    let v = claude_file(pid)?;
    if v.get("kind").and_then(|k| k.as_str()).is_some_and(|k| k != "interactive") {
        return None;
    }
    Some(Claude {
        pid,
        session_id: v.get("sessionId")?.as_str()?.to_string(),
        busy: v.get("status").and_then(|s| s.as_str()).is_some_and(|s| s != "idle"),
        name: v.get("name").and_then(|s| s.as_str()).map(str::to_string),
    })
}

fn claude_cwd(pid: i32) -> Option<PathBuf> {
    claude_file(pid)?.get("cwd")?.as_str().map(PathBuf::from)
}

/// Whether a Claude process is working on a turn (from its session file).
pub fn claude_busy(pid: i32) -> Option<bool> {
    claude_session(pid).map(|c| c.busy)
}

/// Bring the source terminal to the front.
pub fn show_app(s: Source) {
    let app = match s {
        Source::ITerm2 => "iTerm",
        Source::Terminal => "Terminal",
        Source::Ghostty => "Ghostty",
        Source::Warp => "Warp",
        Source::WezTerm => "WezTerm",
        Source::Kitty => "kitty",
        Source::Alacritty => "Alacritty",
        Source::Tmux => return,
    };
    let _ = std::process::Command::new("open").args(["-a", app]).spawn();
}

/// Type /exit into the idle Claude prompt on `tty` (iTerm2 and Terminal only).
pub fn send_exit(s: Source, tty: &str) {
    let dev = format!("/dev/{tty}");
    let script = match s {
        Source::ITerm2 => format!(
            r#"tell application "iTerm2"
  repeat with w in windows
    repeat with t in tabs of w
      repeat with s in sessions of t
        if tty of s is "{dev}" then tell s to write text "/exit"
      end repeat
    end repeat
  end repeat
end tell"#
        ),
        Source::Terminal => format!(
            r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{dev}" then do script "/exit" in t
    end repeat
  end repeat
end tell"#
        ),
        _ => return,
    };
    let _ = std::process::Command::new("osascript").arg("-e").arg(script).output();
}

/// Whether a process is still running.
pub fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

// ---- titles and order (asked only with the user's go-ahead) ---------------------------------

/// Tab titles and (window, tab) order by tty, from the apps that can say.
pub fn titles(sources: &[Source]) -> HashMap<String, (String, (u32, u32))> {
    let mut out = HashMap::new();
    for s in sources {
        let got = match s {
            Source::ITerm2 => applescript(ITERM),
            Source::Terminal => applescript(TERMINAL),
            Source::WezTerm => wezterm(),
            Source::Tmux => tmux(),
            _ => Vec::new(),
        };
        for (tty, title, order) in got {
            out.insert(tty.trim_start_matches("/dev/").to_string(), (title, order));
        }
    }
    out
}

/// Fill in titles and order on what `scan` found.
pub fn apply_titles(found: &mut [Found], titles: &HashMap<String, (String, (u32, u32))>) {
    for f in found {
        if let Some((t, o)) = titles.get(&f.tty) {
            // Titles that only name the shell say nothing; T1K's own title is better.
            let plain = t.trim().is_empty() || is_shell(t.split_whitespace().next().unwrap_or(""));
            f.title = (!plain).then(|| t.clone());
            f.order = Some(*o);
        }
    }
}

const ITERM: &str = r#"set out to ""
tell application "iTerm2"
  set wi to 0
  repeat with w in windows
    set wi to wi + 1
    set ti to 0
    repeat with t in tabs of w
      set ti to ti + 1
      repeat with s in sessions of t
        set out to out & (tty of s) & tab & (name of s) & tab & wi & tab & ti & linefeed
      end repeat
    end repeat
  end repeat
end tell
return out"#;

const TERMINAL: &str = r#"set out to ""
tell application "Terminal"
  set wi to 0
  repeat with w in windows
    set wi to wi + 1
    set ti to 0
    repeat with t in tabs of w
      set ti to ti + 1
      set out to out & (tty of t) & tab & (custom title of t) & tab & wi & tab & ti & linefeed
    end repeat
  end repeat
end tell
return out"#;

fn applescript(script: &str) -> Vec<(String, String, (u32, u32))> {
    let Ok(o) = std::process::Command::new("osascript").arg("-e").arg(script).output() else { return Vec::new() };
    parse_rows(&String::from_utf8_lossy(&o.stdout))
}

fn parse_rows(text: &str) -> Vec<(String, String, (u32, u32))> {
    text.lines()
        .filter_map(|l| {
            let mut c = l.split('\t');
            let (tty, title, w, t) = (c.next()?, c.next()?, c.next()?, c.next()?);
            Some((tty.to_string(), title.to_string(), (w.trim().parse().ok()?, t.trim().parse().ok()?)))
        })
        .collect()
}

fn wezterm() -> Vec<(String, String, (u32, u32))> {
    let Ok(o) = std::process::Command::new("wezterm").args(["cli", "list", "--format", "json"]).output() else { return Vec::new() };
    let Ok(v) = serde_json::from_slice::<Vec<serde_json::Value>>(&o.stdout) else { return Vec::new() };
    v.iter()
        .filter_map(|p| {
            let tty = p.get("tty_name")?.as_str()?.to_string();
            let title = p.get("title").and_then(|t| t.as_str()).unwrap_or_default().to_string();
            Some((tty, title, (p.get("window_id")?.as_u64()? as u32, p.get("tab_id")?.as_u64()? as u32)))
        })
        .collect()
}

fn tmux() -> Vec<(String, String, (u32, u32))> {
    let Ok(o) = std::process::Command::new("tmux").args(["list-panes", "-a", "-F", "#{pane_tty}\t#{session_name}:#{window_name}\t#{window_index}\t#{pane_index}"]).output() else {
        return Vec::new();
    };
    parse_rows(&String::from_utf8_lossy(&o.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources() {
        assert_eq!(Source::of("/Applications/iTerm.app/Contents/MacOS/iTerm2", "iTerm2"), Some(Source::ITerm2));
        assert_eq!(Source::of("/Users/x/Library/Application Support/iTerm2/iTermServer-3.5", "iTermServer-3.5"), Some(Source::ITerm2));
        assert_eq!(Source::of("/Applications/Ghostty.app/Contents/MacOS/ghostty", "ghostty"), Some(Source::Ghostty));
        assert_eq!(Source::of("/Applications/Warp.app/Contents/MacOS/stable", "stable"), Some(Source::Warp));
        assert_eq!(Source::of("/opt/homebrew/bin/tmux", "tmux"), Some(Source::Tmux));
        assert_eq!(Source::of("/bin/zsh", "zsh"), None);
    }

    #[test]
    fn rows_and_titles() {
        let rows = parse_rows("/dev/ttys003\tzsh\t1\t2\n/dev/ttys004\tdeploy logs\t1\t3\nbad\n");
        assert_eq!(rows.len(), 2);
        let titles: HashMap<_, _> = rows.into_iter().map(|(t, n, o)| (t.trim_start_matches("/dev/").to_string(), (n, o))).collect();
        let f = |tty: &str| Found { source: Source::ITerm2, tty: tty.into(), cwd: None, running: None, claude: None, title: None, order: None };
        let mut found = vec![f("ttys003"), f("ttys004")];
        apply_titles(&mut found, &titles);
        assert_eq!(found[0].title, None);
        assert_eq!(found[0].order, Some((1, 2)));
        assert_eq!(found[1].title.as_deref(), Some("deploy logs"));
    }
}
