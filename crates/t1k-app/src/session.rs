//! Save and restore the open tabs (and appearance choices until config.toml lands).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct SavedTab {
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub manual_title: Option<String>,
    /// Reminder time, unix seconds.
    #[serde(default)]
    pub reminder_at: Option<u64>,
    /// Last Claude session in this tab, for "Resume Claude session" after a restart.
    #[serde(default)]
    pub last_session: Option<String>,
    /// Group the user moved this tab to (it stays there whatever its folder).
    #[serde(default)]
    pub pinned_group: Option<PathBuf>,
    /// Split pane: its folder and whether it sits below the main pane.
    #[serde(default)]
    pub split: Option<(Option<PathBuf>, bool)>,
    /// Name of this tab's saved scrollback in `scrollback/`, written at quit.
    #[serde(default)]
    pub scrollback: Option<String>,
    /// A Claude session was still running in this tab when T1K quit.
    #[serde(default)]
    pub agent_live: bool,
}

/// A Claude session that no tab holds any more (tab closed, or replaced by a newer session).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Recent {
    pub group: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
    pub title: String,
    /// Unix seconds.
    pub ended_at: u64,
}

/// Newest 6 per group, nothing older than 14 days, one entry per session.
pub fn prune_recent(v: &mut Vec<Recent>, now: u64) {
    v.sort_by(|a, b| b.ended_at.cmp(&a.ended_at));
    let mut seen = std::collections::HashSet::new();
    let mut per_group: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    v.retain(|r| {
        let n = per_group.entry(r.group.clone()).or_default();
        let keep = now.saturating_sub(r.ended_at) < 14 * 86400 && seen.insert(r.session_id.clone()) && *n < 6;
        if keep {
            *n += 1;
        }
        keep
    });
}

/// Window frame from the last run, in screen points.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct SavedWindow {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// "windowed", "maximized" or "fullscreen".
    #[serde(default)]
    pub state: WindowState,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WindowState {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Session {
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub collapsed: Vec<PathBuf>,
    #[serde(default)]
    pub group_names: Vec<(PathBuf, String)>,
    #[serde(default)]
    pub sidebar_open: Option<bool>,
    #[serde(default)]
    pub outputs_open: bool,
    #[serde(default)]
    pub appearance: Option<String>,
    #[serde(default)]
    pub theme_dark: Option<String>,
    #[serde(default)]
    pub theme_light: Option<String>,
    #[serde(default)]
    pub recent: Vec<Recent>,
    #[serde(default)]
    pub window: Option<SavedWindow>,
    /// The first-launch welcome has been shown.
    #[serde(default)]
    pub welcomed: bool,
    /// Group color: index 1-6 into the theme's ANSI colors.
    #[serde(default)]
    pub group_colors: Vec<(PathBuf, u8)>,
    #[serde(default)]
    pub group_sort: Vec<(PathBuf, GroupSort)>,
    /// Sidebar order of groups (fixed; changed only by hand). Groups not listed go last.
    #[serde(default)]
    pub group_order: Vec<PathBuf>,
}

/// How tabs are ordered within a group.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GroupSort {
    /// Needs-you first, then unread, working, the rest.
    #[default]
    Attention,
    /// Most recently used first.
    Activity,
    /// In the order they were opened.
    Opened,
}

pub fn support_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/t1k"))
}

fn path() -> Option<PathBuf> {
    Some(support_dir()?.join("session.json"))
}

pub fn load() -> Session {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(s: &Session) {
    let Some(p) = path() else { return };
    let Ok(json) = serde_json::to_vec_pretty(s) else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write then rename, so a crash mid-write never leaves a truncated file.
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

/// Find a Claude session's transcript: ~/.claude/projects/<any>/<session_id>.jsonl.
pub fn find_transcript(session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains('/') {
        return None;
    }
    let projects = PathBuf::from(std::env::var_os("HOME")?).join(".claude/projects");
    let name = format!("{session_id}.jsonl");
    std::fs::read_dir(projects).ok()?.flatten().map(|d| d.path().join(&name)).find(|p| p.is_file())
}

#[cfg(test)]
mod recent_tests {
    use super::*;

    fn r(g: &str, id: &str, at: u64) -> Recent {
        Recent { group: g.into(), cwd: g.into(), session_id: id.into(), title: id.into(), ended_at: at }
    }

    #[test]
    fn prune_keeps_newest_per_group() {
        let now = 100 * 86400;
        let mut v: Vec<Recent> = (0..8).map(|i| r("/a", &format!("s{i}"), now - i * 60)).collect();
        v.push(r("/b", "old", now - 15 * 86400));
        v.push(r("/b", "s0", now - 30)); // duplicate id
        prune_recent(&mut v, now);
        assert_eq!(v.iter().filter(|x| x.group == PathBuf::from("/a")).count(), 6);
        assert!(!v.iter().any(|x| x.session_id == "old"));
        assert_eq!(v.iter().filter(|x| x.session_id == "s0").count(), 1);
    }
}
