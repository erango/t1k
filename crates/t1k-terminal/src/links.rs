//! Detect URLs in terminal text so they can be highlighted and opened.

const SCHEMES: &[&str] = &["https://", "http://", "file://"];

/// Characters that end a URL.
fn is_terminator(c: char) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
