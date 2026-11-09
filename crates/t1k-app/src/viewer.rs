//! File viewer side panel: text files open here instead of in an editor (⌘-click a path in the
//! terminal). Plain text gets line numbers and jumps to `path:line`; Markdown is rendered,
//! with a toggle to see the source.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, Context, FontStyle, FontWeight, HighlightStyle, InteractiveText, MouseButton,
    ScrollStrategy, SharedString, StrikethroughStyle, StyledText, UnderlineStyle, UniformListScrollHandle,
};
use t1k_themes::UiColors;

use crate::markdown::{self, Block, Span};
use crate::workspace::Workspace;

/// Bigger files open in the editor instead.
const MAX_BYTES: u64 = 4 << 20;

pub struct Viewer {
    pub path: PathBuf,
    pub line: Option<u32>,
    lines: Arc<Vec<SharedString>>,
    markdown: Option<Vec<Block>>,
    pub show_source: bool,
    scroll: UniformListScrollHandle,
    /// Opened from the outputs panel: closing the viewer goes back to it.
    pub from_outputs: bool,
}

/// Whether a file should open in the viewer: a regular file, not huge, that looks like text
/// (no NUL bytes in its first 8 KB).
pub fn is_text(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return false;
    }
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut buf = [0u8; 8192];
    let n = std::io::Read::read(&mut f, &mut buf).unwrap_or(0);
    !buf[..n].contains(&0)
}

fn is_markdown(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("md" | "markdown" | "mdx"))
}

impl Viewer {
    pub fn open(path: &Path, line: Option<u32>) -> Option<Viewer> {
        let text = std::fs::read_to_string(path).ok()?;
        let markdown = is_markdown(path).then(|| markdown::parse(&text));
        let lines: Vec<SharedString> = text.lines().map(|l| SharedString::from(l.replace('\t', "    "))).collect();
        let scroll = UniformListScrollHandle::new();
        if let Some(l) = line {
            scroll.scroll_to_item(l.saturating_sub(1) as usize, ScrollStrategy::Center);
        }
        // A link to a specific line shows the source there, even for Markdown.
        let show_source = markdown.is_none() || line.is_some();
        Some(Viewer { path: path.to_path_buf(), line, lines: Arc::new(lines), markdown, show_source, scroll, from_outputs: false })
    }
}

impl Workspace {
    /// Open a file in the side panel (closing the outputs panel, which shares the space).
    pub fn view_file(&mut self, path: &Path, line: Option<u32>, cx: &mut Context<Self>) -> bool {
        if !is_text(path) {
            return false;
        }
        let Some(v) = Viewer::open(path, line) else { return false };
        self.viewer = Some(v);
        self.panel.open = false;
        cx.notify();
        true
    }

    pub fn close_viewer(&mut self, cx: &mut Context<Self>) {
        if self.viewer.take().is_some_and(|v| v.from_outputs) {
            self.panel.open = true;
        }
        cx.notify();
    }
}

fn spans_text(ui: UiColors, spans: &[Span], size: f32, base: gpui::Hsla, id: SharedString) -> AnyElement {
    let mut text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for s in spans {
        let start = text.len();
        text.push_str(&s.text);
        let range = start..text.len();
        let mut h = HighlightStyle::default();
        if s.bold {
            h.font_weight = Some(FontWeight::SEMIBOLD);
        }
        if s.italic {
            h.font_style = Some(FontStyle::Italic);
        }
        if s.strike {
            h.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(ui.text_tertiary) });
        }
        if s.code {
            h.background_color = Some(ui.bg_control);
            h.color = Some(ui.text_primary);
        }
        if let Some(l) = &s.link {
            h.color = Some(ui.link);
            h.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(ui.link.opacity(0.5)), wavy: false });
            links.push((range.clone(), l.clone()));
        }
        if h != HighlightStyle::default() {
            highlights.push((range, h));
        }
    }
    let styled = StyledText::new(text).with_highlights(highlights);
    let ranges: Vec<_> = links.iter().map(|(r, _)| r.clone()).collect();
    let urls: Vec<String> = links.into_iter().map(|(_, u)| u).collect();
    div()
        .text_size(px(size))
        .line_height(px(size * 1.55))
        .text_color(base)
        .child(InteractiveText::new(id, styled).on_click(ranges, move |ix, _, cx| {
            if let Some(u) = urls.get(ix) {
                if u.starts_with("http") {
                    cx.open_url(u);
                }
            }
        }))
        .into_any_element()
}

fn render_markdown(ui: UiColors, blocks: &[Block]) -> AnyElement {
    let mut col = div().flex().flex_col().px(px(22.)).py(px(18.));
    for (i, b) in blocks.iter().enumerate() {
        let id = SharedString::from(format!("md-{i}"));
        // List items sit close together; everything else gets paragraph spacing.
        let tight = matches!(b, Block::Item(..)) && i > 0 && matches!(blocks[i - 1], Block::Item(..));
        let gap = if i == 0 { 0. } else if tight { 3. } else { 10. };
        col = col.child(div().mt(px(gap)).child(match b {
            Block::Heading(level, spans) => {
                let size = match level {
                    1 => 22.,
                    2 => 18.,
                    3 => 15.5,
                    _ => 14.,
                };
                div()
                    .pt(px(if *level <= 2 { 8. } else { 4. }))
                    .when(*level <= 2, |d| d.pb(px(6.)).border_b_1().border_color(ui.border))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(spans_text(ui, spans, size, ui.text_primary, id))
                    .into_any_element()
            }
            Block::Paragraph(spans) => spans_text(ui, spans, 13.5, ui.text_secondary, id),
            Block::Item(depth, marker, spans) => div()
                .pl(px(*depth as f32 * 18.))
                .flex()
                .gap(px(8.))
                .child(div().flex_none().min_w(px(14.)).text_size(px(13.5)).text_color(ui.text_tertiary).child(marker.clone()))
                .child(div().flex_1().min_w_0().child(spans_text(ui, spans, 13.5, ui.text_secondary, id)))
                .into_any_element(),
            Block::Quote(spans) => div()
                .pl(px(12.))
                .border_l_2()
                .border_color(ui.border)
                .child(spans_text(ui, spans, 13.5, ui.text_tertiary, id))
                .into_any_element(),
            Block::Code(lang, text) => div()
                .rounded(px(7.))
                .bg(ui.bg_sidebar)
                .border_1()
                .border_color(ui.border)
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(lang.clone().map(|l| div().text_size(px(10.5)).text_color(ui.text_tertiary).child(l)))
                .child(div().font_family("JetBrains Mono").text_size(px(12.)).line_height(px(18.)).text_color(ui.text_primary).whitespace_nowrap().children(text.lines().map(|l| div().child(SharedString::from(l.to_string()))).collect::<Vec<_>>()))
                .into_any_element(),
            Block::Rule => div().h(px(1.)).my(px(6.)).bg(ui.border).into_any_element(),
            Block::Table(rows) => {
                let mut t = div().flex().flex_col().rounded(px(6.)).border_1().border_color(ui.border).overflow_hidden();
                for (r, row) in rows.iter().enumerate() {
                    let mut line = div().flex().when(r == 0, |d| d.bg(ui.bg_hover)).when(r > 0, |d| d.border_t_1().border_color(ui.border));
                    for (c, cell) in row.iter().enumerate() {
                        line = line.child(
                            div().flex_1().min_w_0().px(px(10.)).py(px(6.)).when(c > 0, |d| d.border_l_1().border_color(ui.border))
                                .when(r == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                                .child(spans_text(ui, cell, 12.5, ui.text_secondary, SharedString::from(format!("md-{i}-{r}-{c}")))),
                        );
                    }
                    t = t.child(line);
                }
                t.into_any_element()
            }
        }));
    }
    col.into_any_element()
}

pub fn render(ws: &Workspace, v: &Viewer, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let name = v.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let place = crate::groups::tilde(v.path.parent().unwrap_or(Path::new("/")));
    let button = |id: &'static str, label: &'static str| {
        div()
            .id(id)
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(12.))
            .text_color(ui.text_secondary)
            .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(label)
    };
    let path = v.path.clone();
    let reveal = v.path.clone();
    let system = v.path.clone();
    let header = div()
        .h(px(44.))
        .flex_none()
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(6.))
        .border_b_1()
        .border_color(ui.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(name))
                .child(div().truncate().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(place)),
        )
        .when(v.markdown.is_some(), |d| {
            d.child(
                button("viewer-source", if v.show_source { "Preview" } else { "Source" }).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(v) = this.viewer.as_mut() {
                        v.show_source = !v.show_source;
                    }
                    cx.notify();
                })),
            )
        })
        .child(button("viewer-editor", "Open in editor").on_click(cx.listener(move |this, _, _, cx| {
            let line = this.viewer.as_ref().and_then(|v| v.line);
            let link = t1k_terminal::links::Link::File { path: path.clone(), line, col: None };
            let tab = this.active;
            this.open_link_in_editor(tab, &link, cx);
        })))
        // The file's own app (Preview, Xcode, a Markdown app...), as Finder would open it.
        .child(button("viewer-system", "Default app").on_click(move |_, _, cx| cx.open_with_system(&system)))
        .child(button("viewer-reveal", "Reveal").on_click(move |_, _, cx| cx.reveal_path(&reveal)))
        .child(button("viewer-close", "✕").on_click(cx.listener(|this, _, _, cx| this.close_viewer(cx))));

    let body: AnyElement = match (&v.markdown, v.show_source) {
        (Some(blocks), false) => div().id("viewer-md").flex_1().min_h_0().overflow_y_scroll().child(render_markdown(ui, blocks)).into_any_element(),
        _ => {
            let lines = v.lines.clone();
            let target = v.line.map(|l| l.saturating_sub(1) as usize);
            let digits = lines.len().to_string().len().max(3) as f32;
            uniform_list("viewer-lines", lines.len(), move |range, _, _| {
                range
                    .map(|i| {
                        let hit = target == Some(i);
                        div()
                            .flex()
                            .h(px(19.))
                            .when(hit, |d| d.bg(ui.accent.opacity(0.14)))
                            .child(
                                div().w(px(digits * 7.5 + 16.)).flex_none().pr(px(10.)).flex().justify_end()
                                    .text_color(if hit { ui.text_secondary } else { ui.text_disabled }).child((i + 1).to_string()),
                            )
                            .child(div().flex_1().whitespace_nowrap().text_color(ui.text_primary).child(lines[i].clone()))
                    })
                    .collect()
            })
            .track_scroll(v.scroll.clone())
            .flex_1()
            .min_h_0()
            .py(px(8.))
            .font_family("JetBrains Mono")
            .text_size(px(12.))
            .into_any_element()
        }
    };

    div()
        .id("viewer")
        .w(px(560.))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(ui.bg_raised)
        .border_l_1()
        .border_color(ui.border)
        .child(header)
        .child(body)
        .into_any_element()
}
