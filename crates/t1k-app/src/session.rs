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
    pub appearance: Option<String>,
    #[serde(default)]
    pub theme_dark: Option<String>,
    #[serde(default)]
    pub theme_light: Option<String>,
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
