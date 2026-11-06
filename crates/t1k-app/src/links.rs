//! Opening links from the terminal: URLs in the browser, files in an editor at the line,
//! `#123` in the tab's GitHub repo, ticket keys in the tracker.

use std::path::{Path, PathBuf};
use std::process::Command;

use t1k_agent::outputs::Kind;
use t1k_terminal::links::Link;

use crate::platform::tool_path;
use crate::workspace::Tab;

/// Find an executable on the app's PATH.
fn which(name: &str) -> Option<PathBuf> {
    tool_path().split(':').map(|d| Path::new(d).join(name)).find(|p| p.is_file())
}

/// The editor command for a file, as program + args. `template` is the user's
/// `links.editor` (`{path}`, `{line}`, `{col}`); empty means auto-detect.
pub fn editor_command(template: &str, path: &Path, line: Option<u32>, col: Option<u32>) -> (String, Vec<String>) {
    let p = path.display().to_string();
    let l = line.unwrap_or(1).to_string();
    let c = col.unwrap_or(1).to_string();
    if !template.trim().is_empty() {
        let mut words = template.split_whitespace().map(|w| w.replace("{path}", &p).replace("{line}", &l).replace("{col}", &c));
        let prog = words.next().unwrap_or_default();
        return (prog, words.collect());
    }
    if path.is_dir() {
        return ("open".into(), vec![p]);
    }
    let at = if line.is_some() { format!("{p}:{l}:{c}") } else { p.clone() };
    for (bin, goto) in [("zed", false), ("cursor", true), ("code", true)] {
        if let Some(exe) = which(bin) {
            let mut args = Vec::new();
            if goto {
                args.push("-g".into());
            }
            args.push(at);
            return (exe.display().to_string(), args);
        }
    }
    ("open".into(), vec![p])
}

/// `owner/repo` of the GitHub `origin` remote for a folder.
pub fn github_repo(cwd: &Path) -> Option<String> {
    let out = Command::new("git")
        .env("PATH", tool_path())
        .args(["-C", &cwd.display().to_string(), "remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    parse_github_remote(String::from_utf8_lossy(&out.stdout).trim())
}

fn parse_github_remote(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let slug = rest.trim_end_matches('/').trim_end_matches(".git");
    (slug.split('/').count() == 2).then(|| slug.to_string())
}

/// Ticket URL template from config, else learned from ticket links sessions mentioned:
/// `https://acme.atlassian.net/browse/HOPR-43` → `https://acme.atlassian.net/browse/{key}`.
pub fn ticket_template(configured: &str, tabs: &[Tab]) -> Option<String> {
    if !configured.trim().is_empty() {
        return Some(configured.trim().to_string());
    }
    tabs.iter()
        .filter_map(|t| t.agent.as_ref())
        .flat_map(|a| a.outputs.iter())
        .filter(|o| o.kind == Kind::Ticket && !o.target.starts_with("https://github.com/"))
        .find_map(|o| o.target.strip_suffix(&o.label).map(|base| format!("{base}{{key}}")))
}

/// What to do for a link: a URL to open, or a command to run.
#[derive(Debug, PartialEq)]
pub enum Action {
    Url(String),
    Run(String, Vec<String>),
    /// Nothing to open it with; the message says why.
    Unavailable(String),
}

pub fn action(link: &Link, editor: &str, repo: Option<&str>, tickets: Option<&str>) -> Action {
    match link {
        Link::Url(u) => Action::Url(u.clone()),
        Link::File { path, line, col } => {
            let (prog, args) = editor_command(editor, path, *line, *col);
            Action::Run(prog, args)
        }
        // /issues/N redirects to /pull/N when it's a PR.
        Link::Pr(n) => match repo {
            Some(r) => Action::Url(format!("https://github.com/{r}/issues/{n}")),
            None => Action::Unavailable(format!("#{n}: this tab's folder has no GitHub remote")),
        },
        Link::Ticket(k) => match tickets {
            Some(t) => Action::Url(t.replace("{key}", k)),
            None => Action::Unavailable(format!("{k}: set links.tickets in config.toml")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_remotes() {
        assert_eq!(parse_github_remote("git@github.com:erango/t1k.git").as_deref(), Some("erango/t1k"));
        assert_eq!(parse_github_remote("https://github.com/erango/t1k").as_deref(), Some("erango/t1k"));
        assert_eq!(parse_github_remote("https://gitlab.com/a/b.git"), None);
    }

    #[test]
    fn editor_template() {
        let (p, a) = editor_command("code -g {path}:{line}:{col}", Path::new("/x/a.rs"), Some(4), None);
        assert_eq!((p.as_str(), a), ("code", vec!["-g".to_string(), "/x/a.rs:4:1".to_string()]));
    }

    #[test]
    fn actions() {
        assert_eq!(action(&Link::Pr(12), "", Some("erango/t1k"), None), Action::Url("https://github.com/erango/t1k/issues/12".into()));
        assert_eq!(
            action(&Link::Ticket("HOPR-43".into()), "", None, Some("https://b.atlassian.net/browse/{key}")),
            Action::Url("https://b.atlassian.net/browse/HOPR-43".into())
        );
        assert!(matches!(action(&Link::Ticket("X-1".into()), "", None, None), Action::Unavailable(_)));
    }
}
