//! File viewer side panel: text files open here instead of in an editor (⌘-click a path in the
//! terminal). Plain text gets line numbers and jumps to `path:line`; Markdown is rendered,
//! with a toggle to see the source. Text can be selected with the mouse (double-click for a
//! word, triple for a line, shift-click to extend) and copied with ⌘C or the Copy button.

use std::cell::RefCell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, Context, FontStyle, FontWeight, HighlightStyle, InteractiveText, MouseButton,
    ScrollStrategy, SharedString, StrikethroughStyle, StyledText, TextLayout, UnderlineStyle, UniformListScrollHandle,
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
    focus: gpui::FocusHandle,
    /// Where a selection started and where it ends now (the mouse's end).
    sel: Option<(Mark, Mark)>,
    dragging: bool,
    hits: Rc<RefCell<Hits>>,
    /// The Copy button reads "Copied" for a moment after a click.
    copied: bool,
}

/// A place in the text: which piece (a source line, or a Markdown paragraph, item, cell or
/// code line, in reading order) and a byte offset into it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Mark {
    seg: usize,
    ix: usize,
}

/// Rebuilt every frame: the text of each rendered Markdown piece, and the layouts of the
/// pieces on screen, so a mouse position can be turned into a [`Mark`].
#[derive(Default)]
struct Hits {
    texts: Vec<SharedString>,
    painted: Vec<(usize, TextLayout)>,
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
    pub fn open(path: &Path, line: Option<u32>, cx: &mut gpui::App) -> Option<Viewer> {
        let text = std::fs::read_to_string(path).ok()?;
        let markdown = is_markdown(path).then(|| markdown::parse(&text));
        let lines: Vec<SharedString> = text.lines().map(|l| SharedString::from(l.replace('\t', "    "))).collect();
        let scroll = UniformListScrollHandle::new();
        if let Some(l) = line {
            scroll.scroll_to_item(l.saturating_sub(1) as usize, ScrollStrategy::Center);
        }
        // A link to a specific line shows the source there, even for Markdown.
        let show_source = markdown.is_none() || line.is_some();
        Some(Viewer {
            path: path.to_path_buf(),
            line,
            lines: Arc::new(lines),
            markdown,
            show_source,
            scroll,
            from_outputs: false,
            focus: cx.focus_handle(),
            sel: None,
            dragging: false,
            hits: Rc::default(),
            copied: false,
        })
    }

    fn rendered_md(&self) -> bool {
        self.markdown.is_some() && !self.show_source
    }

    fn seg_text(&self, seg: usize) -> Option<SharedString> {
        if self.rendered_md() {
            self.hits.borrow().texts.get(seg).cloned()
        } else {
            self.lines.get(seg).cloned()
        }
    }

    fn seg_count(&self) -> usize {
        if self.rendered_md() { self.hits.borrow().texts.len() } else { self.lines.len() }
    }

    /// The mark under a window position: the nearest piece on screen, clamped to its ends.
    fn hit(&self, p: gpui::Point<gpui::Pixels>) -> Option<Mark> {
        let hits = self.hits.borrow();
        let mut painted: Vec<&(usize, TextLayout)> = hits.painted.iter().collect();
        painted.sort_by_key(|(s, _)| *s);
        let mut last = None;
        for (seg, l) in painted {
            let b = l.bounds();
            if p.y < b.top() {
                return Some(last.unwrap_or(Mark { seg: *seg, ix: 0 }));
            }
            if p.y < b.bottom() {
                let ix = match l.index_for_position(p) {
                    Ok(i) | Err(i) => i,
                };
                return Some(Mark { seg: *seg, ix: ix.min(l.len()) });
            }
            last = Some(Mark { seg: *seg, ix: l.len() });
        }
        last
    }

    fn ordered(&self) -> Option<(Mark, Mark)> {
        let (a, b) = self.sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        (a != b).then_some((a, b))
    }

    /// The selected byte range within one piece, if any of it is selected.
    fn sel_in(&self, seg: usize, len: usize) -> Option<Range<usize>> {
        range_in(self.ordered(), seg, len)
    }

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.ordered()?;
        let mut out = Vec::new();
        for seg in a.seg..=b.seg {
            let t = self.seg_text(seg).unwrap_or_default();
            let start = if seg == a.seg { a.ix.min(t.len()) } else { 0 };
            let end = if seg == b.seg { b.ix.min(t.len()) } else { t.len() };
            out.push(t.get(start..end).unwrap_or_default().to_string());
        }
        Some(out.join("\n"))
    }

    /// The word around a mark (letters, digits and `_`), or the single character there.
    fn word_at(&self, m: Mark) -> (Mark, Mark) {
        let t = self.seg_text(m.seg).unwrap_or_default();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let ix = m.ix.min(t.len());
        let after = t[ix..].chars().next();
        let before = t[..ix].chars().next_back();
        let in_word = after.is_some_and(word) || before.is_some_and(word);
        if !in_word {
            let end = ix + after.map_or(0, char::len_utf8);
            return (Mark { seg: m.seg, ix }, Mark { seg: m.seg, ix: end });
        }
        let start = t[..ix].char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(ix, |(i, _)| i);
        let end = t[ix..].char_indices().find(|(_, c)| !word(*c)).map_or(t.len(), |(i, _)| ix + i);
        (Mark { seg: m.seg, ix: start }, Mark { seg: m.seg, ix: end })
    }
}

fn range_in(ordered: Option<(Mark, Mark)>, seg: usize, len: usize) -> Option<Range<usize>> {
    let (a, b) = ordered?;
    if seg < a.seg || seg > b.seg {
        return None;
    }
    let start = if seg == a.seg { a.ix.min(len) } else { 0 };
    let end = if seg == b.seg { b.ix.min(len) } else { len };
    (start < end).then_some(start..end)
}

/// Add a selection background to a piece's highlights, splitting them where they overlap.
fn with_selection(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    sel: Option<Range<usize>>,
    color: gpui::Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some(sel) = sel else { return highlights };
    let mut cuts: Vec<usize> = highlights.iter().flat_map(|(r, _)| [r.start, r.end]).chain([sel.start, sel.end]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let base = highlights.iter().find(|(r, _)| r.start <= p && q <= r.end).map(|(_, h)| *h);
        let selected = sel.start <= p && q <= sel.end;
        let h = match (base, selected) {
            (b, true) => HighlightStyle { background_color: Some(color), ..b.unwrap_or_default() },
            (Some(b), false) => b,
            (None, false) => continue,
        };
        out.push((p..q, h));
    }
    out
}

/// Wrap a piece of text so its layout is recorded once it's on screen (after the text's own
/// prepaint, so the layout has bounds).
fn track(hits: &Rc<RefCell<Hits>>, seg: usize, layout: TextLayout, el: impl IntoElement) -> gpui::Div {
    let hits = hits.clone();
    div().relative().child(el).child(
        gpui::canvas(move |_, _, _| hits.borrow_mut().painted.push((seg, layout.clone())), |_, _, _, _| {})
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
    )
}

/// What the Markdown renderer needs to make its text selectable.
struct Sel<'a> {
    v: &'a Viewer,
    color: gpui::Hsla,
}

impl Sel<'_> {
    /// The next piece's number, recording its text.
    fn next(&self, text: &str) -> usize {
        let mut h = self.v.hits.borrow_mut();
        h.texts.push(SharedString::from(text.to_string()));
        h.texts.len() - 1
    }
}

impl Workspace {
    /// Open a file in the side panel (closing the outputs panel, which shares the space).
    pub fn view_file(&mut self, path: &Path, line: Option<u32>, cx: &mut Context<Self>) -> bool {
        if !is_text(path) {
            return false;
        }
        let Some(v) = Viewer::open(path, line, cx) else { return false };
        self.viewer = Some(v);
        self.panel.open = false;
        cx.notify();
        true
    }

    fn viewer_press(&mut self, ev: &gpui::MouseDownEvent, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        window.focus(&v.focus);
        let Some(m) = v.hit(ev.position) else { return };
        v.sel = match (ev.click_count, v.sel) {
            (2, _) => Some(v.word_at(m)),
            (n, _) if n >= 3 => {
                let len = v.seg_text(m.seg).map_or(0, |t| t.len());
                Some((Mark { seg: m.seg, ix: 0 }, Mark { seg: m.seg, ix: len }))
            }
            (_, Some((anchor, _))) if ev.modifiers.shift => Some((anchor, m)),
            _ => Some((m, m)),
        };
        v.dragging = ev.click_count == 1;
        cx.notify();
    }

    fn viewer_drag(&mut self, p: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| v.dragging) else { return };
        if let (Some(m), Some((anchor, head))) = (v.hit(p), v.sel) {
            if m != head {
                v.sel = Some((anchor, m));
                cx.notify();
            }
        }
    }

    fn viewer_release(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = self.viewer.as_mut().filter(|v| v.dragging) {
            v.dragging = false;
            cx.notify();
        }
    }

    /// The selection, or the whole file when nothing is selected.
    fn viewer_copy(&mut self, whole: bool, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        let text = match v.selected_text() {
            Some(t) => t,
            None if whole => std::fs::read_to_string(&v.path).unwrap_or_else(|_| v.lines.join("\n")),
            None => return,
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        v.copied = true;
        let path = v.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1400)).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(v) = this.viewer.as_mut().filter(|v| v.path == path) {
                    v.copied = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn viewer_select_all(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        let n = v.seg_count();
        if n > 0 {
            let len = v.seg_text(n - 1).map_or(0, |t| t.len());
            v.sel = Some((Mark { seg: 0, ix: 0 }, Mark { seg: n - 1, ix: len }));
            cx.notify();
        }
    }

    pub fn debug_viewer_selection(&mut self, spec: &str) {
        let mark = |s: &str| s.split_once(':').and_then(|(a, b)| Some(Mark { seg: a.parse().ok()?, ix: b.parse().ok()? }));
        if let (Some(v), Some((a, b))) = (self.viewer.as_mut(), spec.split_once('-')) {
            v.sel = mark(a).zip(mark(b));
        }
    }

    pub fn close_viewer(&mut self, cx: &mut Context<Self>) {
        if self.viewer.take().is_some_and(|v| v.from_outputs) {
            self.panel.open = true;
        }
        cx.notify();
    }
}

fn spans_text(ui: UiColors, sel: &Sel, spans: &[Span], size: f32, base: gpui::Hsla, id: SharedString) -> AnyElement {
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
    let seg = sel.next(&text);
    let highlights = with_selection(highlights, sel.v.sel_in(seg, text.len()), sel.color);
    let styled = StyledText::new(text).with_highlights(highlights);
    let layout = styled.layout().clone();
    let ranges: Vec<_> = links.iter().map(|(r, _)| r.clone()).collect();
    let urls: Vec<String> = links.into_iter().map(|(_, u)| u).collect();
    div()
        .text_size(px(size))
        .line_height(px(size * 1.55))
        .text_color(base)
        .child(track(
            &sel.v.hits,
            seg,
            layout,
            InteractiveText::new(id, styled).on_click(ranges, move |ix, _, cx| {
                if let Some(u) = urls.get(ix) {
                    if u.starts_with("http") {
                        cx.open_url(u);
                    }
                }
            }),
        ))
        .into_any_element()
}

fn render_markdown(ui: UiColors, sel: &Sel, blocks: &[Block]) -> AnyElement {
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
                    .child(spans_text(ui, sel, spans, size, ui.text_primary, id))
                    .into_any_element()
            }
            Block::Paragraph(spans) => spans_text(ui, sel, spans, 13.5, ui.text_secondary, id),
            Block::Item(depth, marker, spans) => div()
                .pl(px(*depth as f32 * 18.))
                .flex()
                .gap(px(8.))
                .child(div().flex_none().min_w(px(14.)).text_size(px(13.5)).text_color(ui.text_tertiary).child(marker.clone()))
                .child(div().flex_1().min_w_0().child(spans_text(ui, sel, spans, 13.5, ui.text_secondary, id)))
                .into_any_element(),
            Block::Quote(spans) => div()
                .pl(px(12.))
                .border_l_2()
                .border_color(ui.border)
                .child(spans_text(ui, sel, spans, 13.5, ui.text_tertiary, id))
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
                .child(div().font_family("JetBrains Mono").text_size(px(12.)).line_height(px(18.)).text_color(ui.text_primary).whitespace_nowrap().children(text.lines().map(|l| {
                    let seg = sel.next(l);
                    let styled = StyledText::new(l.to_string()).with_highlights(with_selection(Vec::new(), sel.v.sel_in(seg, l.len()), sel.color));
                    let layout = styled.layout().clone();
                    track(&sel.v.hits, seg, layout, styled)
                }).collect::<Vec<_>>()))
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
                                .child(spans_text(ui, sel, cell, 12.5, ui.text_secondary, SharedString::from(format!("md-{i}-{r}-{c}")))),
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
                        v.sel = None;
                    }
                    cx.notify();
                })),
            )
        })
        .child(button("viewer-copy", if v.copied { "Copied" } else { "Copy" }).on_click(cx.listener(|this, _, _, cx| this.viewer_copy(true, cx))))
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

    {
        let mut h = v.hits.borrow_mut();
        h.texts.clear();
        h.painted.clear();
    }
    let color = ui.accent.opacity(0.28);
    let body: AnyElement = match (&v.markdown, v.show_source) {
        (Some(blocks), false) => {
            let sel = Sel { v, color };
            div().id("viewer-md").flex_1().min_h_0().overflow_y_scroll().child(render_markdown(ui, &sel, blocks)).into_any_element()
        }
        _ => {
            let lines = v.lines.clone();
            let hits = v.hits.clone();
            let ordered = v.ordered();
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
                            .child({
                                let len = lines[i].len();
                                let range = range_in(ordered, i, len);
                                let styled = StyledText::new(lines[i].clone()).with_highlights(with_selection(Vec::new(), range, color));
                                let layout = styled.layout().clone();
                                track(&hits, i, layout, div().whitespace_nowrap().child(styled)).flex_1().text_color(ui.text_primary)
                            })
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

    // While dragging a selection, follow the mouse anywhere in the window, and end on release
    // even outside the panel.
    let drag_follow = v.dragging.then(|| {
        let ws = cx.entity().downgrade();
        gpui::canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let w = ws.clone();
                window.on_mouse_event(move |ev: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let _ = w.update(cx, |this, cx| this.viewer_drag(ev.position, cx));
                    }
                });
                let w = ws.clone();
                window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let _ = w.update(cx, |this, cx| this.viewer_release(cx));
                    }
                });
            },
        )
        .absolute()
        .size_0()
    });
    let body = div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .cursor(gpui::CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &gpui::MouseDownEvent, window, cx| this.viewer_press(ev, window, cx)))
        // Keep focus here (the workspace hands it back to the terminal on mouse-up).
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(body)
        .children(drag_follow);

    div()
        .id("viewer")
        .track_focus(&v.focus)
        .key_context("Viewer")
        .on_action(cx.listener(|this, _: &crate::actions::ViewerCopy, _, cx| this.viewer_copy(false, cx)))
        .on_action(cx.listener(|this, _: &crate::actions::ViewerSelectAll, _, cx| this.viewer_select_all(cx)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn m(seg: usize, ix: usize) -> Mark {
        Mark { seg, ix }
    }

    #[test]
    fn selection_covers_the_pieces_between_its_ends() {
        let sel = Some((m(1, 3), m(3, 2)));
        assert_eq!(range_in(sel, 0, 10), None);
        assert_eq!(range_in(sel, 1, 10), Some(3..10));
        assert_eq!(range_in(sel, 2, 5), Some(0..5));
        assert_eq!(range_in(sel, 3, 10), Some(0..2));
        assert_eq!(range_in(sel, 4, 10), None);
    }

    #[test]
    fn selection_splits_overlapping_highlights() {
        let bold = HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
        let c = gpui::Hsla::default();
        let out = with_selection(vec![(2..6, bold)], Some(4..8), c);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![2..4, 4..6, 6..8]);
        assert_eq!(out[0].1.background_color, None);
        assert_eq!(out[1].1.font_weight, Some(FontWeight::SEMIBOLD));
        assert_eq!(out[1].1.background_color, Some(c));
        assert_eq!(out[2].1.font_weight, None);
        assert_eq!(with_selection(vec![(2..6, bold)], None, c).len(), 1);
    }
}
