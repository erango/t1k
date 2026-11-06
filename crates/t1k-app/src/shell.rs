//! Shell integration files, written to Application Support at startup and pointed to by
//! ZDOTDIR for zsh tabs.

use std::path::PathBuf;

const ZSHENV: &str = include_str!("../../../resources/shell-integration/zsh/.zshenv");

/// The zsh integration directory, written if missing or out of date. None for other shells
/// or when it can't be written.
pub fn zsh_dir() -> Option<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    if !shell.ends_with("/zsh") {
        return None;
    }
    let dir = crate::session::support_dir()?.join("shell-integration/zsh");
    let file = dir.join(".zshenv");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(ZSHENV) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&file, ZSHENV).ok()?;
    }
    Some(dir)
}
