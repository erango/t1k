//! Detect links in terminal text so they can be highlighted and opened: URLs, file paths
//! (with `:line:col`), PR references (`#123`) and ticket keys (`ENG-42`).

use std::path::PathBuf;

/// What a detected link points at. The app decides how to open each kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Link {
    Url(String),
    /// An existing file or folder, with an optional position.
    File { path: PathBuf, line: Option<u32>, col: Option<u32> },
    /// `#123`: a PR or issue in the tab's repo.
    Pr(u32),
    /// `ENG-42`: a ticket in the configured tracker.
    Ticket(String),
}

impl Link {
    /// Text shown for the link (hint mode, tooltips).
    pub fn label(&self) -> String {
        match self {
            Link::Url(u) => u.clone(),
            Link::File { path, line: Some(l), col: Some(c) } => format!("{}:{l}:{c}", path.display()),
            Link::File { path, line: Some(l), .. } => format!("{}:{l}", path.display()),
            Link::File { path, .. } => path.display().to_string(),
            Link::Pr(n) => format!("#{n}"),
            Link::Ticket(k) => k.clone(),
        }
    }
}

/// Which plain-text references to detect besides URLs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkRules {
    pub files: bool,
    pub prs: bool,
    pub tickets: bool,
}

const SCHEMES: &[&str] = &["https://", "http://", "file://"];

/// Characters that end a URL.
pub(crate) fn is_terminator(c: char) -> bool {
    c.is_whitespace()
        || c == '\0'
        || matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '|' | '\\' | '^')
        // Box-drawing and block characters (TUI borders around a URL).
        || ('\u{2500}'..='\u{259f}').contains(&c)
}

fn starts_with(chars: &[char], at: usize, pat: &str) -> bool {
    let mut i = at;
    for p in pat.chars() {
        if chars.get(i).map(|c| c.to_ascii_lowercase()) != Some(p) {
            return false;
        }
        i += 1;
    }
    true
}

/// Find URLs in a line of text. Returns half-open `[start, end)` char index ranges.
pub fn find_urls(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let Some(scheme) = SCHEMES.iter().find(|s| starts_with(chars, i, s)) else {
            i += 1;
            continue;
        };
        let body = i + scheme.chars().count();
        let mut end = body;
        while end < chars.len() && !is_terminator(chars[end]) {
            end += 1;
        }
        end = trim_trailing(chars, i, end);
        if end > body {
            out.push((i, end));
        }
        i = end.max(i + 1);
    }
    out
}

/// Real top-level domains that are also file extensions people print (`main.rs`,
/// `README.md`, `install.sh`): never a bare link.
const FILE_LIKE_TLDS: &[&str] = &["rs", "md", "sh", "py", "pl", "pm", "zip", "mov", "cc", "so"];

/// Whether `host` ends in a real public suffix (per the Public Suffix List) that isn't
/// file-like, and has a name before it (`sanity.io`, not `io`).
fn real_domain(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let tld = host.rsplit('.').next().unwrap_or("");
    if FILE_LIKE_TLDS.contains(&tld) || tld.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    psl::suffix(host.as_bytes()).is_some_and(|s| s.is_known() && s.as_bytes().len() < host.len())
}

/// Links written without a scheme, like `sanity.io/manage` or `docs.github.com`: `[start, end)`
/// and the URL to open (with `https://`). The domain must end in a known TLD, and not be part
/// of a word, an email address or a longer path.
pub fn find_bare_urls(chars: &[char]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let label = |c: char| c.is_ascii_alphanumeric() || c == '-';
    let mut i = 0;
    while i < chars.len() {
        let before = i.checked_sub(1).map(|j| chars[j]);
        if !label(chars[i]) || before.is_some_and(|c| label(c) || matches!(c, '.' | '@' | '/' | ':' | '_' | '#' | '~')) {
            i += 1;
            continue;
        }
        // The host: labels separated by dots.
        let start = i;
        let mut end = i;
        let mut last_label = start;
        let mut dots = 0;
        while end < chars.len() {
            if label(chars[end]) {
                end += 1;
            } else if chars[end] == '.' && end + 1 < chars.len() && label(chars[end + 1]) && end > start && chars[end - 1] != '.' {
                dots += 1;
                end += 1;
                last_label = end;
            } else {
                break;
            }
        }
        let _ = last_label;
        let host: String = chars[start..end].iter().collect();
        let next = chars.get(end).copied();
        let ok_host = dots > 0 && real_domain(&host) && !next.is_some_and(|c| c == '@' || c == '_');
        if !ok_host {
            i = end.max(i + 1);
            continue;
        }
        // An optional path, up to whatever ends a URL.
        if next == Some('/') {
            while end < chars.len() && !is_terminator(chars[end]) {
                end += 1;
            }
        } else if next.is_some_and(|c| !is_terminator(c) && !matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '*')) {
            // Glued to more text ("a.com=b"): not a link.
            i = end.max(i + 1);
            continue;
        }
        let end = trim_trailing(chars, start, end);
        out.push((start, end, format!("https://{}", chars[start..end].iter().collect::<String>())));
        i = end.max(i + 1);
    }
    out
}

/// Email addresses (`name@example.com`): `[start, end)` and a `mailto:` URL.
pub fn find_emails(chars: &[char]) -> Vec<(usize, usize, String)> {
    let local = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
    let host = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    let mut out = Vec::new();
    for (at, _) in chars.iter().enumerate().filter(|(_, &c)| c == '@') {
        let mut a = at;
        while a > 0 && local(chars[a - 1]) {
            a -= 1;
        }
        // No leading dot in the name ("...see.me@x.com" keeps "see.me").
        while a < at && chars[a] == '.' {
            a += 1;
        }
        let mut b = at + 1;
        while b < chars.len() && host(chars[b]) {
            b += 1;
        }
        // A sentence-ending dot isn't part of the domain.
        while b > at + 1 && matches!(chars[b - 1], '.' | '-') {
            b -= 1;
        }
        if a == at || b == at + 1 || out.last().is_some_and(|&(_, e, _)| e > a) {
            continue;
        }
        let domain: String = chars[at + 1..b].iter().collect();
        // "git@github.com:owner/repo" is a git remote, not an address.
        let remote = chars.get(b) == Some(&':') && chars.get(b + 1).is_some_and(|c| !c.is_whitespace());
        if domain.contains('.') && real_domain(&domain) && !remote {
            out.push((a, b, format!("mailto:{}", chars[a..b].iter().collect::<String>())));
        }
    }
    out
}

/// Drop trailing punctuation and unbalanced closing brackets, e.g. "(see https://x.com/a)."
fn trim_trailing(chars: &[char], start: usize, mut end: usize) -> usize {
    loop {
        let Some(&last) = chars[start..end].last() else { return end };
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '*' => true,
            ')' | ']' => {
                let open = if last == ')' { '(' } else { '[' };
                let s = &chars[start..end];
                s.iter().filter(|&&c| c == last).count() > s.iter().filter(|&&c| c == open).count()
            }
            _ => false,
        };
        if !drop {
            return end;
        }
        end -= 1;
    }
}

/// A file path candidate: `[start, end)` and the parsed path text with its position.
#[derive(Debug, Clone, PartialEq)]
pub struct PathCandidate {
    pub start: usize,
    pub end: usize,
    pub path: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
}

fn path_terminator(c: char) -> bool {
    is_terminator(c) || matches!(c, '(' | ')' | '[' | ']' | ',' | ';' | '=')
}

/// Path-looking tokens: they contain a `/`, or end in a short extension (`main.rs`), with an
/// optional `:line` or `:line:col`. Existence is checked by the caller.
pub fn find_path_candidates(chars: &[char]) -> Vec<PathCandidate> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if path_terminator(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !path_terminator(chars[i]) {
            i += 1;
        }
        let token: String = chars[start..i].iter().collect();
        if let Some(c) = parse_path_token(&token, start) {
            out.push(c);
        }
    }
    out
}

fn parse_path_token(token: &str, start: usize) -> Option<PathCandidate> {
    if token.contains("://") {
        return None;
    }
    // Trim punctuation that ends sentences, and markdown emphasis/backticks.
    let lead = token.chars().take_while(|c| matches!(c, '*' | '_' | '`')).count();
    let body: String = token.chars().skip(lead).collect();
    let body = body.trim_end_matches(['.', ':', '!', '?', '*', '_', '`']);
    if body.is_empty() {
        return None;
    }
    // Split off :line[:col].
    let mut parts: Vec<&str> = body.split(':').collect();
    let mut nums = Vec::new();
    while parts.len() > 1 && nums.len() < 2 {
        match parts.last().and_then(|p| p.parse::<u32>().ok()) {
            Some(n) => {
                nums.insert(0, n);
                parts.pop();
            }
            None => break,
        }
    }
    let path = parts.join(":");
    let looks_like = path.contains('/') || has_extension(&path);
    if !looks_like || path.len() < 2 || path.chars().all(|c| c == '/' || c == '.') {
        return None;
    }
    // Versions and numbers ("v2.1.317", "3.14") aren't files.
    if path.chars().next().is_some_and(|c| c.is_ascii_digit()) && !path.contains('/') {
        return None;
    }
    let end = start + lead + body.chars().count();
    Some(PathCandidate { start: start + lead, end, path, line: nums.first().copied(), col: nums.get(1).copied() })
}

fn has_extension(s: &str) -> bool {
    match s.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=6).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// `#123` references, as `[start, end)` and the number. Needs a non-word character before it.
pub fn find_pr_refs(chars: &[char]) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::new();
    for i in 0..chars.len() {
        if chars[i] != '#' || (i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '&' || chars[i - 1] == '/')) {
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        let digits = j - i - 1;
        if !(1..=6).contains(&digits) || chars.get(j).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            continue;
        }
        let n: String = chars[i + 1..j].iter().collect();
        if let Ok(n) = n.parse() {
            out.push((i, j, n));
        }
    }
    out
}

/// Ticket keys like `ENG-42` or `HOPR-43`: 2-10 uppercase letters/digits starting with a letter,
/// a dash, then digits, on word boundaries.
pub fn find_tickets(chars: &[char]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let boundary = i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '-' | '_' | '/' | '.'));
        if !boundary || !chars[i].is_ascii_uppercase() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && (chars[j].is_ascii_uppercase() || chars[j].is_ascii_digit()) {
            j += 1;
        }
        let key_len = j - i;
        if !(2..=10).contains(&key_len) || chars.get(j) != Some(&'-') {
            i = j.max(i + 1);
            continue;
        }
        let mut k = j + 1;
        while k < chars.len() && chars[k].is_ascii_digit() {
            k += 1;
        }
        let digits = k - j - 1;
        let after_ok = chars.get(k).is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_')));
        // UTF-8, SHA-256, ISO-8601 and friends aren't tickets.
        let key: String = chars[i..j].iter().collect();
        let common = matches!(key.as_str(), "UTF" | "SHA" | "ISO" | "RFC" | "MD" | "CVE" | "HTTP" | "TLS" | "AES" | "RSA" | "GPT" | "ES" | "UTC" | "GMT");
        if (1..=6).contains(&digits) && after_ok && !common {
            out.push((i, k, chars[i..k].iter().collect()));
        }
        i = k;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn path_candidates() {
        let c = find_path_candidates(&chars("error at src/auth.rs:42:7, see `Cargo.toml` and ./run.sh."));
        let got: Vec<_> = c.iter().map(|p| (p.path.as_str(), p.line, p.col)).collect();
        assert_eq!(got, [("src/auth.rs", Some(42), Some(7)), ("Cargo.toml", None, None), ("./run.sh", None, None)]);
        let c = find_path_candidates(&chars("Claude Code v2.1.317 took 3.5s; https://x.io/a.rs"));
        assert!(c.is_empty(), "{c:?}");
        // Range covers the line suffix so the whole thing underlines.
        let line = chars("  ~/git-repos/t1k/README.md:3 ");
        let p = &find_path_candidates(&line)[0];
        assert_eq!(line[p.start..p.end].iter().collect::<String>(), "~/git-repos/t1k/README.md:3");
        let line = chars("A copy is at /private/tmp/claude-501/-Users-x-git-repos/4f0d86cf/scratchpad/slack-nov9.md");
        let c = find_path_candidates(&line);
        assert_eq!(c.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["/private/tmp/claude-501/-Users-x-git-repos/4f0d86cf/scratchpad/slack-nov9.md"]);
    }

    #[test]
    fn pr_refs_and_tickets() {
        let c = chars("Fixes #123 (see #7); not a#1 or #12ab or color #fff, HOPR-43 and ENG-7.");
        assert_eq!(find_pr_refs(&c).iter().map(|r| r.2).collect::<Vec<_>>(), [123, 7]);
        assert_eq!(find_tickets(&c).iter().map(|r| r.2.as_str()).collect::<Vec<_>>(), ["HOPR-43", "ENG-7"]);
        assert!(find_tickets(&chars("UTF-8 and SHA-256, x-API-12, abc-12")).is_empty());
    }

    fn urls(s: &str) -> Vec<String> {
        let chars: Vec<char> = s.chars().collect();
        find_urls(&chars).into_iter().map(|(a, b)| chars[a..b].iter().collect()).collect()
    }

    #[test]
    fn plain() {
        assert_eq!(urls("see https://github.com/erango/t1k/pull/3 now"), ["https://github.com/erango/t1k/pull/3"]);
    }

    #[test]
    fn trailing_punctuation_and_parens() {
        assert_eq!(urls("(https://a.com/x)."), ["https://a.com/x"]);
        assert_eq!(urls("https://en.wikipedia.org/wiki/Foo_(bar)"), ["https://en.wikipedia.org/wiki/Foo_(bar)"]);
        assert_eq!(urls("**https://a.com/b**"), ["https://a.com/b"]);
    }

    #[test]
    fn multiple_and_quotes() {
        assert_eq!(urls("\"http://a.io\" and <file:///tmp/x>"), ["http://a.io", "file:///tmp/x"]);
    }

    #[test]
    fn box_border_and_bare_scheme() {
        assert_eq!(urls("│ https://x.dev/a │"), ["https://x.dev/a"]);
        assert!(urls("https:// nothing").is_empty());
    }

    #[test]
    fn bare_domains() {
        let b = |s: &str| find_bare_urls(&chars(s)).into_iter().map(|(_, _, u)| u).collect::<Vec<_>>();
        assert_eq!(b("open sanity.io/manage, then"), ["https://sanity.io/manage"]);
        assert_eq!(b("see docs.github.com."), ["https://docs.github.com"]);
        assert_eq!(b("(vercel.com/dashboard)"), ["https://vercel.com/dashboard"]);
        assert!(b("edit src/main.rs and README.md, run install.sh").is_empty());
        assert!(b("mail me at a@b.com").is_empty());
        assert!(b("v2.1.317 and 3.14").is_empty());
        assert!(b("crates/x/foo.io/bar").is_empty());
        assert_eq!(b("deploy to my-app.vercel.app"), ["https://my-app.vercel.app"]);
        assert!(b("not.arealtld and foo.notatld").is_empty());
    }

    #[test]
    fn emails() {
        let e = |s: &str| find_emails(&chars(s)).into_iter().map(|(_, _, u)| u).collect::<Vec<_>>();
        assert_eq!(e("(yaron.frost@gmail.com)"), ["mailto:yaron.frost@gmail.com"]);
        assert_eq!(e("write a.b+tag@mail.example.co.uk."), ["mailto:a.b+tag@mail.example.co.uk"]);
        assert!(e("git@github.com:erango/t1k.git and @user and foo@bar").is_empty());
    }
}
