//! Markdown to a flat list of blocks the file viewer can draw: headings, paragraphs, list
//! items (with depth), quotes, code blocks, rules and tables. Inline styling is kept as spans.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Span>),
    Paragraph(Vec<Span>),
    /// depth (0 = top level), marker ("•", "1.", "☐", "☑"), content
    Item(usize, String, Vec<Span>),
    Quote(Vec<Span>),
    Code(Option<String>, String),
    Rule,
    Table(Vec<Vec<Vec<Span>>>),
}

#[derive(Default)]
struct Style {
    bold: usize,
    italic: usize,
    strike: usize,
    link: Option<String>,
}

pub fn parse(src: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut blocks = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Style::default();
    // Lists: one entry per open list, holding the next number for ordered lists.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut item_marker: Option<String> = None;
    let mut quote = 0usize;
    let mut code: Option<(Option<String>, String)> = None;
    let mut table: Option<Vec<Vec<Vec<Span>>>> = None;
    let mut heading: Option<u8> = None;

    let push = |spans: &mut Vec<Span>, style: &Style, text: &str, code: bool| {
        if text.is_empty() {
            return;
        }
        spans.push(Span {
            text: text.to_string(),
            bold: style.bold > 0,
            italic: style.italic > 0,
            strike: style.strike > 0,
            code,
            link: style.link.clone(),
        });
    };

    // Close the current run of inline text into a block of the right kind.
    let flush = |blocks: &mut Vec<Block>, spans: &mut Vec<Span>, heading: Option<u8>, lists: &Vec<Option<u64>>, marker: &mut Option<String>, quote: usize| {
        if spans.is_empty() && marker.is_none() {
            return;
        }
        let s = std::mem::take(spans);
        if let Some(h) = heading {
            blocks.push(Block::Heading(h, s));
        } else if let Some(m) = marker.take() {
            blocks.push(Block::Item(lists.len().saturating_sub(1), m, s));
        } else if quote > 0 {
            blocks.push(Block::Quote(s));
        } else {
            blocks.push(Block::Paragraph(s));
        }
    };

    for ev in Parser::new_ext(src, opts) {
        if let Some((_, buf)) = code.as_mut() {
            match ev {
                Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, text) = code.take().unwrap();
                    blocks.push(Block::Code(lang, text.trim_end_matches('\n').to_string()));
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                heading = None;
            }
            Event::End(TagEnd::Paragraph) => {
                if table.is_none() {
                    flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                }
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                item_marker = Some(match lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}.");
                        *n += 1;
                        m
                    }
                    _ => "•".into(),
                });
            }
            Event::End(TagEnd::Item) => flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote),
            Event::TaskListMarker(done) => item_marker = Some(if done { "☑".into() } else { "☐".into() }),
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                let lang = match kind {
                    CodeBlockKind::Fenced(l) if !l.is_empty() => Some(l.to_string()),
                    _ => None,
                };
                code = Some((lang, String::new()));
            }
            Event::Start(Tag::Table(_)) => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                table = Some(Vec::new());
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let Some(t) = table.as_mut() {
                    t.push(Vec::new());
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(std::mem::take(&mut spans));
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = table.take() {
                    blocks.push(Block::Table(t));
                }
            }
            Event::Start(Tag::Strong) => style.bold += 1,
            Event::End(TagEnd::Strong) => style.bold = style.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => style.italic += 1,
            Event::End(TagEnd::Emphasis) => style.italic = style.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => style.strike += 1,
            Event::End(TagEnd::Strikethrough) => style.strike = style.strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => style.link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => style.link = None,
            Event::Text(t) => push(&mut spans, &style, &t, false),
            Event::Code(t) => push(&mut spans, &style, &t, true),
            Event::SoftBreak => push(&mut spans, &style, " ", false),
            Event::HardBreak => push(&mut spans, &style, "\n", false),
            // Raw HTML isn't rendered; dropping it beats showing tags as text.
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::Rule => {
                flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
                blocks.push(Block::Rule);
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut spans, heading, &lists, &mut item_marker, quote);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_markdown() {
        let md = "# Title\n\nSome **bold** and `code` with a [link](https://x.io).\n\n- one\n- two\n  1. nested\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let b = parse(md);
        assert!(matches!(&b[0], Block::Heading(1, s) if s[0].text == "Title"));
        let Block::Paragraph(p) = &b[1] else { panic!("{:?}", b[1]) };
        assert!(p.iter().any(|s| s.bold && s.text == "bold"));
        assert!(p.iter().any(|s| s.code && s.text == "code"));
        assert!(p.iter().any(|s| s.link.as_deref() == Some("https://x.io")));
        assert!(b.iter().any(|x| matches!(x, Block::Item(0, m, _) if m == "•")));
        assert!(b.iter().any(|x| matches!(x, Block::Item(1, m, _) if m == "1.")));
        assert!(b.iter().any(|x| matches!(x, Block::Quote(_))));
        assert!(b.iter().any(|x| matches!(x, Block::Code(Some(l), t) if l == "rust" && t == "fn main() {}")));
        assert!(b.iter().any(|x| matches!(x, Block::Rule)));
        assert!(b.iter().any(|x| matches!(x, Block::Table(rows) if rows.len() == 2 && rows[1][1][0].text == "2")));
    }
}
