//! Split panes: a tab can show a second terminal beside (⌘D) or below (⌘⇧D) its main one.
//! The main pane stays the tab's identity (title, agent, folder); the split is a helper shell,
//! for a log tail or a second command next to Claude.

use gpui::{div, prelude::*, px, AnyElement, Context, Entity, Focusable, Subscription, Window};
use t1k_terminal::view::{TerminalEvent, TerminalView};
use t1k_themes::UiColors;

use crate::workspace::{Tab, Workspace};

pub struct Split {
    pub view: Entity<TerminalView>,
    /// Folder it was opened in (saved for restore; refreshed while polling).
    pub cwd: Option<std::path::PathBuf>,
    /// Below the main pane (⌘⇧D) rather than beside it (⌘D).
    pub vertical: bool,
    _sub: Subscription,
}


fn cached(view: &Entity<TerminalView>) -> gpui::AnyView {
    gpui::AnyView::from(view.clone()).cached(gpui::StyleRefinement::default().size_full())
}

/// The active tab's terminal area: one pane, or two with a hairline between them.
pub fn render_panes(t: &Tab, ui: UiColors, _: &mut Window, _: &mut Context<Workspace>) -> AnyElement {
    let Some(split) = &t.split else {
        return cached(&t.view).into_any_element();
    };
    let pane = |v: &Entity<TerminalView>| div().flex_1().min_w_0().min_h_0().child(cached(v));
    let line = if split.vertical { div().h(px(1.)).w_full() } else { div().w(px(1.)).h_full() }.flex_none().bg(ui.border);
    div()
        .size_full()
        .flex()
        .when(split.vertical, |d| d.flex_col())
        .child(pane(&t.view))
        .child(line)
        .child(pane(&split.view))
        .into_any_element()
}

impl Workspace {
    /// Open a split in tab `ix` in its folder, or move focus to the one it has.
    pub fn open_split(&mut self, ix: usize, vertical: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(ix) else { return };
        if let Some(s) = &t.split {
            let h = s.view.read(cx).focus_handle(cx);
            window.focus(&h);
            return;
        }
        let cwd = t.cwd.clone();
        self.open_split_in(ix, cwd, vertical, window, cx);
    }

    pub fn open_split_in(&mut self, ix: usize, cwd: Option<std::path::PathBuf>, vertical: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(ix) else { return };
        let cwd = cwd.filter(|c| c.is_dir()).or_else(|| t.cwd.clone());
        // Its own id, so a Claude session in the split doesn't take over the tab's card.
        let pane_id = format!("{}-split", t.id);
        let view = self.spawn_view(cwd.clone(), pane_id, window, cx);
        let sub = cx.subscribe_in(&view, window, |this: &mut Self, view, ev: &TerminalEvent, window, cx| {
            let Some(ix) = this.tabs.iter().position(|t| t.split.as_ref().is_some_and(|s| &s.view == view)) else { return };
            match ev {
                TerminalEvent::Exited => this.close_split(ix, window, cx),
                TerminalEvent::OpenLink(link) => this.open_link(ix, link, cx),
                TerminalEvent::ContextMenu { at, link, has_selection } => {
                    let o = crate::overlay::Overlay::TerminalMenu { tab: ix, split: true, at: *at, link: link.clone(), has_selection: *has_selection };
                    this.open_overlay(o, window, cx);
                }
                TerminalEvent::TitleChanged(_) | TerminalEvent::Bell => {}
            }
        });
        let h = view.read(cx).focus_handle(cx);
        self.tabs[ix].split = Some(Split { view, cwd, vertical, _sub: sub });
        if ix == self.active {
            window.focus(&h);
        }
        self.save();
        cx.notify();
    }

    pub fn close_split(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(ix) else { return };
        if t.split.take().is_some() {
            if ix == self.active {
                let h = t.view.read(cx).focus_handle(cx);
                window.focus(&h);
            }
            self.save();
            cx.notify();
        }
    }

    /// ⌘[ / ⌘]: move focus between the main pane and the split.
    pub fn focus_other_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let Some(s) = &t.split else { return };
        let main = t.view.read(cx).focus_handle(cx);
        let other = s.view.read(cx).focus_handle(cx);
        window.focus(if main.is_focused(window) { &other } else { &main });
    }
}
