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
    if any(&["chart", "graph", "dashboard", "metrics", "analytics", "stats", "kpi"]) {
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

/// Classify a URL into an output, normalizing it (drop query/fragment and PR sub-pages).
/// Unknown kinds of links return None: the panel lists things a session produced, not every URL.
pub fn classify(url: &str) -> Option<(Kind, String, String)> {
    let base = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/');
    let after = base.strip_prefix("https://")?;
    let (host, path) = after.split_once('/').unwrap_or((after, ""));
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
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
}
