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

/// Current git branch for a directory (reads HEAD directly, worktrees included). None when
/// not in a repo or HEAD is detached.
pub fn git_branch(cwd: &Path) -> Option<String> {
    for dir in cwd.ancestors() {
        let dot_git = dir.join(".git");
        let head = if dot_git.is_dir() {
            dot_git.join("HEAD")
        } else if dot_git.is_file() {
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
            let gitdir = if Path::new(gitdir).is_absolute() { PathBuf::from(gitdir) } else { dir.join(gitdir) };
            gitdir.join("HEAD")
        } else {
            continue;
        };
        let text = std::fs::read_to_string(head).ok()?;
        return text.trim().strip_prefix("ref: refs/heads/").map(str::to_string);
    }
    None
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

    #[test]
    fn reads_branch_from_head() {
        let dir = std::env::temp_dir().join(format!("t1k-branch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/feat/sidebar\n").unwrap();
        assert_eq!(git_branch(&dir.join("src")).as_deref(), Some("feat/sidebar"));
        std::fs::write(dir.join(".git/HEAD"), "0123abcd\n").unwrap();
        assert_eq!(git_branch(&dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }

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
