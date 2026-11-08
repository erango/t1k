//! Dropping files on a terminal (canvas v3-aa, option A), as in Warp: the paths are typed at
//! the cursor, shell-escaped, as one paste and without Enter. The pane under the pointer shows
//! an accent outline and a label while dragging; a sidebar row takes the drop for its tab.
//! Theme files ask first: import the theme, or insert the path.

use std::path::{Path, PathBuf};

use gpui::{div, prelude::*, px, AnyElement, Context, Entity, ExternalPaths, FontWeight, MouseButton, Pixels, Point, SharedString, Window};
use t1k_terminal::view::TerminalView;
use t1k_themes::UiColors;

use crate::overlay::Overlay;
use crate::workspace::Workspace;

#[derive(Clone, Debug, PartialEq)]
pub enum DropTarget {
    /// The active tab's main pane, or its split.
    Pane { split: bool },
    /// A sidebar row, by tab id.
    Row(String),
}

/// What's being dragged over the window right now.
#[derive(Clone, Debug)]
pub struct DropHover {
    pub target: DropTarget,
    pub paths: Vec<PathBuf>,
}

/// Escape for zsh/bash: backslash before anything that isn't plainly safe.
pub fn shell_escape(p: &Path) -> String {
    let s = p.to_string_lossy();
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if !(c.is_alphanumeric() || "_-./~+@%:,=".contains(c)) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// "a b c " — every path escaped, with one trailing space so the next word can follow.
pub fn insert_text(paths: &[PathBuf]) -> String {
    let mut s = paths.iter().map(|p| shell_escape(p)).collect::<Vec<_>>().join(" ");
    s.push(' ');
    s
}

fn is_image(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "tiff" | "bmp"))
}

/// A color scheme T1K can import (by extension, then by whether it parses as one).
pub fn is_theme(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("itermcolors") => true,
        Some("toml" | "yaml" | "yml" | "json" | "conf") | None => {
            std::fs::metadata(p).map(|m| m.is_file() && m.len() < 64 * 1024).unwrap_or(false) && t1k_themes::import::load(p).is_ok()
        }
        _ => false,
    }
}

/// "screenshot.png" truncated in the middle to `max` characters.
fn short_name(p: &Path, max: usize) -> String {
    let n = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let chars: Vec<char> = n.chars().collect();
    if chars.len() <= max {
        return n;
    }
    let half = (max - 1) / 2;
    format!("{}…{}", chars[..half].iter().collect::<String>(), chars[chars.len() - (max - 1 - half)..].iter().collect::<String>())
}

/// The label's copy for these paths.
fn label(paths: &[PathBuf]) -> String {
    match paths {
        [p] if is_theme(p) => "Drop to import theme or insert path".into(),
        [p] if is_image(p) => format!("Drop to insert {}", short_name(p, 28)),
        [_] => "Drop to insert path".into(),
        ps => format!("Drop to insert {} paths", ps.len()),
    }
}

impl Workspace {
    /// The pane or row under the pointer while files are dragged over it (None when it left).
    pub fn drop_hover(&mut self, target: DropTarget, inside: bool, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let was = self.drop_hover.as_ref().map(|h| h.target.clone());
        if inside {
            if was.as_ref() != Some(&target) {
                self.drop_hover = Some(DropHover { target, paths: paths.paths().to_vec() });
                cx.notify();
            }
        } else if was.as_ref() == Some(&target) {
            self.drop_hover = None;
            cx.notify();
        }
    }

    fn pane_view(&self, split: bool) -> Option<Entity<TerminalView>> {
        let t = self.tabs.get(self.active)?;
        if split {
            t.split.as_ref().map(|s| s.view.clone())
        } else {
            Some(t.view.clone())
        }
    }

    /// Files dropped on a pane: theme files ask first, everything else is typed as paths.
    pub fn drop_on_pane(&mut self, split: bool, paths: &ExternalPaths, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_hover = None;
        let paths = paths.paths().to_vec();
        if let [p] = paths.as_slice() {
            if is_theme(p) {
                self.open_overlay(Overlay::DropTheme { path: p.clone(), split, at }, window, cx);
                return;
            }
        }
        self.insert_paths(split, &paths, cx);
    }

    pub fn insert_paths(&mut self, split: bool, paths: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(v) = self.pane_view(split) {
            v.read(cx).terminal.paste(&insert_text(paths));
        }
        cx.notify();
    }

    /// Files dropped on a sidebar row: go to that tab and type the paths there.
    pub fn drop_on_row(&mut self, id: &str, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_hover = None;
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else { return };
        self.activate(ix, window, cx);
        self.insert_paths(false, paths.paths(), cx);
    }
}

/// Wrap a pane so it takes file drops and shows the outline and label while hovered.
pub fn pane(ws: &Workspace, split: bool, content: impl IntoElement, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let hover = ws.drop_hover.as_ref().filter(|h| h.target == DropTarget::Pane { split });
    div()
        .id(if split { "pane-split" } else { "pane-main" })
        .relative()
        .size_full()
        .child(content)
        .on_drag_move::<ExternalPaths>(cx.listener(move |this, e: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
            let inside = e.bounds.contains(&e.event.position);
            let paths = e.drag(cx).clone();
            this.drop_hover(DropTarget::Pane { split }, inside, &paths, cx);
        }))
        .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
            let at = window.mouse_position();
            this.drop_on_pane(split, paths, at, window, cx);
        }))
        // A Claude session resuming after a quit, or a tab that couldn't come back (v3-ab).
        .when(!split, |d| d.children(ws.tabs.get(ws.active).and_then(|t| crate::restore::pane_overlay(ws, &t.id))))
        .when_some(hover, |d, h| d.child(outline(ui)).child(drop_label(ui, &h.paths)))
        .into_any_element()
}

fn outline(ui: UiColors) -> gpui::Div {
    div()
        .absolute()
        .top(px(2.))
        .left(px(2.))
        .right(px(2.))
        .bottom(px(2.))
        .rounded(px(6.))
        .border_1()
        .border_color(ui.accent.opacity(0.55))
        .bg(ui.accent.opacity(0.04))
}

fn drop_label(ui: UiColors, paths: &[PathBuf]) -> gpui::Div {
    let image = match paths {
        [p] if is_image(p) => Some(p.clone()),
        _ => None,
    };
    let pill = div()
        .h(px(30.))
        .rounded(px(15.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.accent.opacity(0.35))
        .shadow_md()
        .flex()
        .items_center()
        .gap(px(8.))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .text_color(ui.text_primary)
        .map(|d| match image {
            Some(p) => d.pl(px(5.)).pr(px(12.)).child(gpui::img(p).size(px(22.)).rounded(px(4.)).flex_none()),
            None => d.pl(px(10.)).pr(px(12.)).child(div().text_size(px(13.)).text_color(ui.text_secondary).child("⎘")),
        })
        .child(label(paths));
    div().absolute().left_0().right_0().bottom(px(16.)).flex().justify_center().child(pill)
}

/// "Insert path" on a sidebar row while files are dragged over it.
pub fn row_label(ui: UiColors, paths: &[PathBuf]) -> gpui::Div {
    let text = if paths.len() == 1 { "Insert path".to_string() } else { format!("Insert {} paths", paths.len()) };
    div()
        .absolute()
        .right(px(10.))
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .child(div().px(px(6.)).rounded(px(4.)).bg(ui.bg_sidebar).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(ui.accent).child(text))
}

/// The choice for a dropped theme file, at the drop point. Esc or a click away inserts nothing.
pub fn theme_choice(ws: &Workspace, path: &Path, split: bool, at: Point<Pixels>, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let name: SharedString = short_name(path, 32).into();
    let (p1, p2) = (path.to_path_buf(), path.to_path_buf());
    let button = |id: &'static str, text: &'static str, primary: bool| {
        div()
            .id(id)
            .h(px(26.))
            .px(px(10.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .text_size(px(12.))
            .cursor_pointer()
            .map(|d| if primary { d.bg(ui.accent).text_color(gpui::white()).font_weight(FontWeight::MEDIUM) } else { d.bg(ui.bg_control).text_color(ui.text_primary) })
            .hover(|d| d.opacity(0.88))
            .child(text)
    };
    div()
        .absolute()
        .left(at.x - px(125.))
        .top(at.y + px(8.))
        .w(px(250.))
        .p(px(12.))
        .rounded(px(10.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .font_family(".SystemUIFont")
        .flex()
        .flex_col()
        .gap(px(6.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).truncate().child(name))
        .child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("This looks like a theme."))
        .child(
            div()
                .pt(px(4.))
                .flex()
                .gap(px(6.))
                .child(div().flex_1())
                .child(button("drop-insert", "Insert path", false).on_click(cx.listener(move |this, _, window, cx| {
                    this.close_overlay(window, cx);
                    this.insert_paths(split, std::slice::from_ref(&p1), cx);
                })))
                .child(button("drop-import", "Import theme", true).on_click(cx.listener(move |this, _, window, cx| {
                    this.close_overlay(window, cx);
                    this.import_themes(std::slice::from_ref(&p2), window, cx);
                }))),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_escaped_for_the_shell() {
        let t = insert_text(&[PathBuf::from("/Users/me/Desktop/Screen Shot 1.png"), PathBuf::from("/tmp/a'b(c)")]);
        assert_eq!(t, "/Users/me/Desktop/Screen\\ Shot\\ 1.png /tmp/a\\'b\\(c\\) ");
        assert_eq!(short_name(Path::new("/x/a-very-long-screenshot-name-from-today.png"), 28).chars().count(), 28);
        assert_eq!(label(&[PathBuf::from("/a"), PathBuf::from("/b")]), "Drop to insert 2 paths");
    }
}
