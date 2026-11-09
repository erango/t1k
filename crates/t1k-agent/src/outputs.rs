//! Session outputs: PRs, artifacts, docs, tickets, deploys and files a session produced.

use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Pr,
    Artifact,
    Doc,
    Ticket,
    Deploy,
    File,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Pr => "PRs",
            Kind::Artifact => "Artifacts",
            Kind::Doc => "Docs",
            Kind::Ticket => "Tickets",
            Kind::Deploy => "Deploys",
            Kind::File => "Files",
        }
    }

    pub const ALL: [Kind; 6] = [Kind::Pr, Kind::Artifact, Kind::Doc, Kind::Ticket, Kind::Deploy, Kind::File];
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub kind: Kind,
    /// URL, or absolute path for files.
    pub target: String,
    pub label: String,
    pub at: SystemTime,
    /// Artifacts: the icon kind ("chart", "slides", …; see `artifact_icon`) and the
    /// description, from the call that published it. None when only its link was seen.
    pub icon: Option<String>,
    pub detail: Option<String>,
}

/// Map the Artifact tool's free-form `icon` word to one of T1K's artifact icons (canvas v3-ae).
pub fn artifact_icon(word: &str) -> &'static str {
    let w = word.trim().to_ascii_lowercase();
    let any = |ws: &[&str]| ws.contains(&w.as_str());
    if any(&["table", "sheet", "spreadsheet", "csv", "data"]) {
        "table"
    } else if any(&["dashboard", "metrics", "kpi"]) {
        "dashboard"
    } else if any(&["chart", "graph", "analytics", "stats"]) {
        "chart"
    } else if any(&["slides", "deck", "presentation", "pitch"]) {
        "slides"
    } else if any(&["doc", "document", "memo", "notes", "report", "article", "writeup", "text"]) {
        "doc"
    } else if any(&["design", "canvas", "mockup", "wireframe", "ui", "palette", "layout", "icon"]) {
        "design"
    } else if any(&["code", "terminal", "script", "api", "json", "diff"]) {
        "code"
    } else if any(&["map", "location", "route", "travel"]) {
        "map"
    } else if any(&["calendar", "schedule", "timeline", "plan", "date", "agenda"]) {
        "calendar"
    } else if any(&["game", "play", "puzzle", "quiz"]) {
        "game"
    } else {
        "page"
    }
}

/// The icon for a file in the outputs panel (canvas v3-am), from its name: a few whole names
/// first (tsconfig.json is config, not data), then the last extension.
/// The tracker a ticket URL belongs to, as an icon kind for `icons::artifact`.
pub fn ticket_icon(url: &str) -> &'static str {
    let host = url.trim_start_matches("https://").split('/').next().unwrap_or("");
    match host {
        "linear.app" => "t-linear",
        "github.com" => "t-github",
        h if h.ends_with(".atlassian.net") => "t-jira",
        _ => "link",
    }
}

pub fn file_icon(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    const LOCKS: &[&str] = &["cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "gemfile.lock", "poetry.lock", "go.sum", "composer.lock"];
    const CONFIGS: &[&str] = &["dockerfile", "makefile", "justfile", ".gitignore", ".editorconfig", ".dockerignore", ".prettierrc", ".eslintrc", "tsconfig.json"];
    if LOCKS.contains(&name.as_str()) || name.ends_with(".lock") {
        return "f-lock";
    }
    if CONFIGS.contains(&name.as_str()) || name.ends_with(".config.js") || name.ends_with(".config.ts") {
        return "f-config";
    }
    let Some((stem, ext)) = name.rsplit_once('.') else { return "f-generic" };
    if stem.is_empty() {
        return "f-generic";
    }
    match ext {
        "rs" | "go" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "java" | "kt" | "swift" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "rb" | "php"
        | "scala" | "lua" | "dart" | "ex" | "exs" | "erl" | "hs" | "ml" | "zig" | "sql" | "vue" | "svelte" => "f-code",
        "json" | "jsonc" | "json5" | "yaml" | "yml" | "toml" | "xml" | "plist" | "ini" | "env" | "proto" | "graphql" => "f-data",
        "html" | "htm" | "css" | "scss" | "sass" | "less" => "f-web",
        "md" | "mdx" | "markdown" => "f-markdown",
        "txt" | "rst" | "org" | "log" | "adoc" => "f-text",
        "csv" | "tsv" | "xlsx" | "xls" | "numbers" | "parquet" => "f-table",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "heic" | "ico" | "icns" | "bmp" | "tiff" => "f-image",
        "pdf" => "f-pdf",
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "dmg" => "f-archive",
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" => "f-shell",
        _ => "f-generic",
    }
}

/// A doc link's icon and where it lives ("Google Docs", or the domain), for the outputs panel.
pub fn doc_kind(url: &str) -> (&'static str, String) {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").trim_start_matches("www.").to_ascii_lowercase();
    let path = &rest[rest.find('/').unwrap_or(rest.len())..];
    let (icon, place) = if host == "docs.google.com" {
        if path.starts_with("/presentation/") {
            ("slides", "Google Slides")
        } else if path.starts_with("/spreadsheets/") {
            ("table", "Google Sheets")
        } else {
            ("doc", "Google Docs")
        }
    } else if host == "claude.ai" {
        ("doc", "Claude Docs")
    } else if host.ends_with("notion.so") || host.ends_with("notion.site") {
        ("doc", "Notion")
    } else if host.ends_with(".atlassian.net") && path.starts_with("/wiki/") {
        ("doc", "Confluence")
    } else if host == "linear.app" {
        ("doc", "Linear")
    } else {
        return ("doc", host);
    };
    (icon, place.to_string())
}

/// Find URLs in a raw line (JSON-escaped text is fine: quotes and backslashes end a URL).
pub fn urls_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("https://") {
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '<' | '>' | '`' | '|' | '{' | '}' | ']'))
            .unwrap_or(tail.len());
        let mut url = &tail[..end];
        while let Some(c) = url.chars().last() {
            let unbalanced = c == ')' && url.matches(')').count() > url.matches('(').count();
            if matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '*') || unbalanced {
                url = &url[..url.len() - c.len_utf8()];
            } else {
                break;
            }
        }
        if url.len() > "https://".len() + 3 {
            out.push(url.to_string());
        }
        rest = &tail[end.max(1)..];
    }
    out
}

/// Example repos in docs and instructions ("github.com/owner/repo/pull/13"), not real ones.
fn placeholder(owner: &str, repo: &str) -> bool {
    let o = owner.to_ascii_lowercase();
    let r = repo.to_ascii_lowercase();
    let templated = |s: &str| s.contains(['<', '>', '{', '}', '$']) || s.starts_with(':');
    const OWNERS: &[&str] = &["owner", "org", "organization", "orgname", "user", "username", "your-org", "your-username", "yourname", "you", "me", "example", "foo", "acme"];
    const REPOS: &[&str] = &["repo", "repository", "repo-name", "reponame", "your-repo", "project", "example", "foo", "bar", "my-repo", "myrepo"];
    templated(&o) || templated(&r) || (OWNERS.contains(&o.as_str()) && REPOS.contains(&r.as_str()))
}

/// Classify a URL into an output, normalizing it (drop query/fragment and PR sub-pages).
/// Unknown kinds of links return None: the panel lists things a session produced, not every URL.
pub fn classify(url: &str) -> Option<(Kind, String, String)> {
    let base = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/');
    let after = base.strip_prefix("https://")?;
    let (host, path) = after.split_once('/').unwrap_or((after, ""));
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if host == "github.com" && seg.len() >= 2 && placeholder(seg[0], seg[1]) {
        return None;
    }
    match host {
        "github.com" if seg.len() >= 4 && seg[2] == "pull" && seg[3].chars().all(|c| c.is_ascii_digit()) => {
            let canon = format!("https://github.com/{}/{}/pull/{}", seg[0], seg[1], seg[3]);
            Some((Kind::Pr, canon, format!("{}#{}", seg[1], seg[3])))
        }
        "github.com" if seg.len() >= 4 && seg[2] == "issues" && seg[3].chars().all(|c| c.is_ascii_digit()) => {
            let canon = format!("https://github.com/{}/{}/issues/{}", seg[0], seg[1], seg[3]);
            Some((Kind::Ticket, canon, format!("{}#{}", seg[1], seg[3])))
        }
        "linear.app" if seg.len() >= 3 && seg[1] == "issue" => Some((Kind::Ticket, base.into(), seg[2].to_string())),
        h if h.ends_with(".atlassian.net") && seg.first() == Some(&"browse") && seg.len() >= 2 => {
            Some((Kind::Ticket, base.into(), seg[1].to_string()))
        }
        h if h.ends_with(".atlassian.net") && seg.first() == Some(&"wiki") => {
            Some((Kind::Doc, base.into(), "Confluence page".into()))
        }
        "claude.ai" => {
            if !matches!(seg.as_slice(), ["artifact", _, ..] | ["code", "artifact", _, ..]) {
                return None;
            }
            // The id says nothing to a person; a publish call gives it a title later (v3-ae).
            Some((Kind::Artifact, base.into(), "Artifact".to_string()))
        }
        "docs.google.com" => {
            let what = match seg.first() {
                Some(&"document") => "Google Doc",
                Some(&"spreadsheets") => "Google Sheet",
                Some(&"presentation") => "Google Slides",
                _ => return None,
            };
            Some((Kind::Doc, base.into(), what.into()))
        }
        h if h == "www.notion.so" || h == "notion.so" || h.ends_with(".notion.site") => {
            let title = seg.last().map(|s| s.rsplit_once('-').map(|(t, _)| t).unwrap_or(s).replace('-', " "));
            Some((Kind::Doc, base.into(), title.filter(|t| !t.is_empty()).unwrap_or_else(|| "Notion page".into())))
        }
        h if h.ends_with(".vercel.app") || h.ends_with(".netlify.app") || h.ends_with(".pages.dev") => {
            Some((Kind::Deploy, base.into(), h.to_string()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_json_escaped_text() {
        let line = r#"{"text":"Opened https://github.com/erango/t1k/pull/12/files?x=1, see (https://claude.ai/artifact/Fm4Hyv7MYFsH7ACopnt1Ka).\nDone"}"#;
        let u = urls_in(line);
        assert_eq!(u, ["https://github.com/erango/t1k/pull/12/files?x=1", "https://claude.ai/artifact/Fm4Hyv7MYFsH7ACopnt1Ka"]);
        assert_eq!(classify(&u[0]).unwrap(), (Kind::Pr, "https://github.com/erango/t1k/pull/12".into(), "t1k#12".into()));
        assert_eq!(classify(&u[1]).unwrap().0, Kind::Artifact);
        assert!(classify("https://github.com/owner/repo/pull/13").is_none());
        assert!(classify("https://github.com/<org>/<repo>/pull/1").is_none());
        assert!(classify("https://github.com/bizzabo/fred-agent/pull/13").is_some());
    }

    #[test]
    fn classifies_known_hosts_only() {
        assert_eq!(classify("https://acme.atlassian.net/browse/ENG-43").unwrap().2, "ENG-43");
        assert_eq!(classify("https://linear.app/acme/issue/ENG-7/fix-it").unwrap().2, "ENG-7");
        assert_eq!(classify("https://docs.google.com/document/d/abc/edit").unwrap().0, Kind::Doc);
        assert_eq!(classify("https://my-app-git-x.vercel.app/").unwrap().0, Kind::Deploy);
        assert_eq!(classify("https://claude.ai/code/artifact/123e4567-e89b").unwrap().0, Kind::Artifact);
        assert!(classify("https://docs.rs/gpui").is_none());
        assert!(classify("https://github.com/erango/t1k").is_none());
    }

    #[test]
    fn file_icons() {
        assert_eq!(file_icon("/a/src/main.rs"), "f-code");
        assert_eq!(file_icon("/a/tsconfig.json"), "f-config");
        assert_eq!(file_icon("/a/package.json"), "f-data");
        assert_eq!(file_icon("/a/Cargo.lock"), "f-lock");
        assert_eq!(file_icon("/a/README.MD"), "f-markdown");
        assert_eq!(file_icon("/a/out.tar.gz"), "f-archive");
        assert_eq!(file_icon("/a/LICENSE"), "f-generic");
        assert_eq!(file_icon("/a/.env"), "f-generic");
        assert_eq!(file_icon("/a/Dockerfile"), "f-config");
    }

    #[test]
    fn doc_kinds() {
        assert_eq!(doc_kind("https://docs.google.com/presentation/d/x/edit"), ("slides", "Google Slides".into()));
        assert_eq!(doc_kind("https://docs.google.com/spreadsheets/d/x"), ("table", "Google Sheets".into()));
        assert_eq!(doc_kind("https://acme.atlassian.net/wiki/spaces/X"), ("doc", "Confluence".into()));
        assert_eq!(doc_kind("https://www.dropbox.com/s/x"), ("doc", "dropbox.com".into()));
    }
}
