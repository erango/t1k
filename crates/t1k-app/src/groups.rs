//! Project grouping: which group a tab belongs to, from its working directory.

use std::path::{Path, PathBuf};

/// The project a directory belongs to: the git repo root, or the directory itself.
///
/// Linked worktrees (`.git` is a file pointing into `<repo>/.git/worktrees/<name>`) group with
/// their main repository, so all worktrees of one project share a group.
pub fn project_root(cwd: &Path) -> PathBuf {
    for dir in cwd.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return dir.to_path_buf();
        }
        if dot_git.is_file() {
            return main_repo_of_worktree(&dot_git).unwrap_or_else(|| dir.to_path_buf());
        }
    }
    cwd.to_path_buf()
}

fn main_repo_of_worktree(dot_git_file: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dot_git_file).ok()?;
    let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let (repo, _) = gitdir.split_once("/.git/worktrees/")?;
    Some(PathBuf::from(repo))
}

/// Short label for a directory: its basename, or "~" for the home directory.
pub fn label(path: &Path) -> String {
    if std::env::var_os("HOME").is_some_and(|h| Path::new(&h) == path) {
        return "~".into();
    }
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Path with the home directory shortened to "~".
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("t1k-groups-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn repo_root_and_plain_dir() {
        let repo = tmp("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("crates/a")).unwrap();
        assert_eq!(project_root(&repo.join("crates/a")), repo);
        let plain = tmp("plain");
        assert_eq!(project_root(&plain), plain);
    }

    #[test]
    fn worktree_groups_with_main_repo() {
        let main = tmp("main");
        let wt = tmp("wt");
        std::fs::write(wt.join(".git"), format!("gitdir: {}/.git/worktrees/feature\n", main.display())).unwrap();
        assert_eq!(project_root(&wt), main);
    }
}
