//! Workspace: the tabs in a window, their project groups, and the active theme.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    div, prelude::*, px, App, Context, Entity, FocusHandle, Focusable, MouseButton, Subscription, Task, Window, WindowAppearance,
};
use t1k_terminal::{SpawnOptions, TerminalEvent, TerminalView};
use t1k_themes::{Appearance, AppearanceMode, Theme, UiColors};

use crate::actions::*;
use crate::groups;
use crate::session::{self, SavedTab, Session};

const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "login", "nu", "xonsh"];

pub struct Tab {
    pub id: String,
    pub view: Entity<TerminalView>,
    pub osc_title: Option<String>,
    pub manual_title: Option<String>,
    pub cwd: Option<PathBuf>,
    /// Foreground process name; None while it's the shell.
    pub program: Option<String>,
    /// Project group key (git root or folder).
    pub group: PathBuf,
    _sub: Subscription,
}

impl Tab {
    /// Display title. Precedence: manual rename, then the program's OSC title while a program
    /// runs, then the folder name while at the shell prompt. (Claude's ai-title slots in at M4.)
    pub fn title(&self) -> String {
        if let Some(t) = &self.manual_title {
            return t.clone();
        }
        if let Some(p) = &self.program {
            if let Some(t) = self.osc_title.as_ref().filter(|t| !t.trim().is_empty()) {
                return t.clone();
            }
            return p.clone();
        }
        self.cwd.as_deref().map(groups::label).unwrap_or_else(|| "shell".into())
    }

    /// Second line in detail mode: what's running and where.
    pub fn subtitle(&self) -> String {
        let place = self.cwd.as_deref().map(groups::tilde).unwrap_or_default();
        match &self.program {
            Some(p) => format!("{p} · {place}"),
            None => place,
        }
    }
}

pub struct Workspace {
    pub tabs: Vec<Tab>,
    /// Index into `tabs`.
    pub active: usize,
    focus_handle: FocusHandle,
    pub collapsed: HashSet<PathBuf>,
    pub group_names: HashMap<PathBuf, String>,
    pub sidebar_open: bool,
    /// Sidebar shows two-line rows (hover intent or ⌥ held).
    pub detail: bool,
    pub alt_held: bool,
    hover_task: Option<Task<()>>,
    mode: AppearanceMode,
    dark: &'static Theme,
    light: &'static Theme,
    pub theme: &'static Theme,
    pub ui: UiColors,
    _poll: Task<()>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = session::load();
        let mut dark = saved.theme_dark.as_deref().and_then(t1k_themes::find).filter(|t| t.is_dark());
        let mut light = saved.theme_light.as_deref().and_then(t1k_themes::find).filter(|t| !t.is_dark());
        let mut mode = match saved.appearance.as_deref() {
            Some("dark") => AppearanceMode::Dark,
            Some("light") => AppearanceMode::Light,
            _ => AppearanceMode::System,
        };
        match std::env::var("T1K_APPEARANCE").as_deref() {
            Ok("dark") => mode = AppearanceMode::Dark,
            Ok("light") => mode = AppearanceMode::Light,
            _ => {}
        }
        if let Some(t) = std::env::var("T1K_THEME").ok().and_then(|n| t1k_themes::find(&n)) {
            if t.is_dark() { dark = Some(t) } else { light = Some(t) }
            mode = if t.is_dark() { AppearanceMode::Dark } else { AppearanceMode::Light };
        }
        let dark = dark.unwrap_or(t1k_themes::default_for(Appearance::Dark));
        let light = light.unwrap_or(t1k_themes::default_for(Appearance::Light));

        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(800)).await;
            if this.update(cx, |ws, cx| ws.poll(cx)).is_err() {
                break;
            }
        });

        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            focus_handle: cx.focus_handle(),
            collapsed: saved.collapsed.iter().cloned().collect(),
            group_names: saved.group_names.iter().cloned().collect(),
            sidebar_open: saved.sidebar_open.unwrap_or(true),
            detail: false,
            alt_held: false,
            hover_task: None,
            mode,
            dark,
            light,
            theme: dark,
            ui: dark.ui(),
            _poll: poll,
        };
        cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)).detach();
        this.apply_theme(window, cx);

        let restore: Vec<SavedTab> = saved.tabs.into_iter().filter(|t| t.cwd.as_ref().is_none_or(|p| p.is_dir())).collect();
        if restore.is_empty() || std::env::var_os("T1K_FRESH").is_some() {
            this.open_tab(None, None, window, cx);
        } else {
            for t in restore {
                this.open_tab(t.cwd, t.manual_title, window, cx);
            }
            let active = saved.active.min(this.tabs.len() - 1);
            this.activate(active, window, cx);
        }
        this
    }

    // ---- tabs -------------------------------------------------------------------------------

    pub fn open_tab(
        &mut self,
        cwd: Option<PathBuf>,
        manual_title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        let cwd = cwd.or_else(|| std::env::var_os("HOME").map(Into::into));
        let opts = SpawnOptions { working_directory: cwd.clone(), env: HashMap::new(), tab_id: id.clone() };
        let palette = self.theme.palette();
        let view = cx.new(|cx| {
            let mut v = TerminalView::new(opts, window, cx).expect("failed to spawn shell in pty");
            v.palette = palette;
            v
        });
        let sub = cx.subscribe_in(&view, window, |this: &mut Self, view, ev: &TerminalEvent, window, cx| {
            let Some(ix) = this.tabs.iter().position(|t| &t.view == view) else { return };
            match ev {
                TerminalEvent::TitleChanged(t) => {
                    this.tabs[ix].osc_title = (!t.is_empty()).then(|| t.clone());
                    cx.notify();
                }
                TerminalEvent::Exited => this.close_tab_at(ix, window, cx),
                TerminalEvent::Bell => {}
            }
        });
        let group = cwd.as_deref().map(groups::project_root).unwrap_or_default();
        self.tabs.push(Tab { id, view, osc_title: None, manual_title, cwd, program: None, group, _sub: sub });
        self.activate(self.tabs.len() - 1, window, cx);
        self.save();
    }

    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active = ix;
        let handle = self.tabs[ix].view.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    pub fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // Move focus to the neighbor in visual order, not in creation order.
        let order = self.visual_order();
        let pos = order.iter().position(|&i| i == ix).unwrap_or(0);
        let next = order.get(pos + 1).or_else(|| pos.checked_sub(1).and_then(|p| order.get(p))).copied();
        self.tabs.remove(ix);
        if self.tabs.is_empty() {
            self.save();
            window.remove_window();
            return;
        }
        let next = next.map(|n| if n > ix { n - 1 } else { n }).unwrap_or(0);
        self.activate(next.min(self.tabs.len() - 1), window, cx);
        self.save();
    }

    /// Groups in display order (first-opened first), each with its tab indices.
    pub fn grouped(&self) -> Vec<(PathBuf, Vec<usize>)> {
        let mut out: Vec<(PathBuf, Vec<usize>)> = Vec::new();
        for (i, t) in self.tabs.iter().enumerate() {
            match out.iter_mut().find(|(g, _)| *g == t.group) {
                Some((_, v)) => v.push(i),
                None => out.push((t.group.clone(), vec![i])),
            }
        }
        out
    }

    /// Tab indices in the order the sidebar shows them (collapsed groups included).
    pub fn visual_order(&self) -> Vec<usize> {
        self.grouped().into_iter().flat_map(|(_, v)| v).collect()
    }

    pub fn group_label(&self, key: &PathBuf) -> String {
        self.group_names.get(key).cloned().unwrap_or_else(|| groups::label(key))
    }

    /// Refresh each tab's cwd and foreground program; regroup tabs that changed project.
    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut regrouped = false;
        for t in &mut self.tabs {
            let term = &t.view.read(cx).terminal;
            let cwd = term.cwd();
            let program = term.foreground_name().filter(|n| !SHELLS.contains(&n.trim_start_matches('-')));
            if program != t.program {
                t.program = program;
                changed = true;
            }
            if cwd.is_some() && cwd != t.cwd {
                let group = groups::project_root(cwd.as_deref().unwrap());
                if group != t.group {
                    t.group = group;
                    regrouped = true;
                }
                t.cwd = cwd;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
        if regrouped || changed {
            self.save();
        }
    }

    pub fn session(&self) -> Session {
        Session {
            tabs: self.visual_order().iter().map(|&i| SavedTab {
                cwd: self.tabs[i].cwd.clone(),
                manual_title: self.tabs[i].manual_title.clone(),
            }).collect(),
            active: self.visual_order().iter().position(|&i| i == self.active).unwrap_or(0),
            collapsed: self.collapsed.iter().cloned().collect(),
            group_names: self.group_names.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            sidebar_open: Some(self.sidebar_open),
            appearance: Some(match self.mode {
                AppearanceMode::System => "system",
                AppearanceMode::Dark => "dark",
                AppearanceMode::Light => "light",
            }.into()),
            theme_dark: Some(self.dark.name.into()),
            theme_light: Some(self.light.name.into()),
        }
    }

    pub fn save(&self) {
        session::save(&self.session());
    }

    // ---- sidebar detail mode ----------------------------------------------------------------

    /// Hover intent: expand after the pointer rests 150 ms, collapse 400 ms after it leaves.
    pub fn sidebar_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        let delay = if hovered { 150 } else { 400 };
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            let _ = this.update(cx, |ws, cx| {
                if ws.detail != hovered {
                    ws.detail = hovered;
                    cx.notify();
                }
            });
        }));
    }

    pub fn show_detail(&self) -> bool {
        self.detail || self.alt_held
    }

    // ---- theme ------------------------------------------------------------------------------

    fn resolve_theme(&self, window: &Window) -> &'static Theme {
        let system_dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
        match self.mode {
            AppearanceMode::Dark => self.dark,
            AppearanceMode::Light => self.light,
            AppearanceMode::System => if system_dark { self.dark } else { self.light },
        }
    }

    fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme = self.resolve_theme(window);
        self.ui = self.theme.ui();
        let palette = self.theme.palette();
        for tab in &self.tabs {
            tab.view.update(cx, |v, cx| {
                v.palette = palette.clone();
                cx.notify();
            });
        }
        cx.notify();
    }

    // ---- actions ----------------------------------------------------------------------------

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.tabs.get(self.active).and_then(|t| t.view.read(cx).terminal.cwd());
        self.open_tab(cwd, None, window, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.visual_order();
        let n = order.len() as isize;
        let pos = order.iter().position(|&i| i == self.active).unwrap_or(0) as isize;
        let next = order[((pos + delta).rem_euclid(n)) as usize];
        self.activate(next, window, cx);
    }

    fn select_nth(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.visual_order();
        // ⌘9 is always the last tab, as in browsers.
        let ix = if n == 8 { order.last() } else { order.get(n) };
        if let Some(&ix) = ix {
            self.activate(ix, window, cx);
        }
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.save();
        cx.notify();
    }

    pub fn toggle_group(&mut self, key: PathBuf, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        self.save();
        cx.notify();
    }

    fn toggle_appearance(&mut self, _: &ToggleAppearance, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = if self.theme.is_dark() { AppearanceMode::Light } else { AppearanceMode::Dark };
        self.apply_theme(window, cx);
        self.save();
    }

    fn follow_system(&mut self, _: &FollowSystemAppearance, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = AppearanceMode::System;
        self.apply_theme(window, cx);
        self.save();
    }

    fn select_theme(&mut self, a: &SelectTheme, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = t1k_themes::find(&a.name) else { return };
        if t.is_dark() { self.dark = t } else { self.light = t }
        // Show the pick now. In System mode, keep following the system if it already shows this slot.
        let showing_slot = self.resolve_theme(window).is_dark() == t.is_dark();
        if !(self.mode == AppearanceMode::System && showing_slot) {
            self.mode = if t.is_dark() { AppearanceMode::Dark } else { AppearanceMode::Light };
        }
        self.apply_theme(window, cx);
        self.save();
    }

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = self.ui;
        let tab = self.tabs.get(self.active);
        let title = tab.map(|t| t.title()).unwrap_or_default();
        let place = tab.and_then(|t| t.cwd.as_deref().map(groups::tilde)).unwrap_or_default();
        let moon = if self.theme.is_dark() { "☾" } else { "☀" };
        div()
            .id("titlebar")
            .h(px(38.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .pr(px(10.))
            .pl(px(if self.sidebar_open { 16. } else { 84. }))
            .bg(ui.bg_terminal)
            .on_mouse_down(MouseButton::Left, |ev, window, _| {
                if ev.click_count == 2 {
                    window.zoom_window();
                } else {
                    window.start_window_move();
                }
            })
            .child(div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).text_color(ui.text_primary).truncate().child(title))
            .child(div().text_size(px(12.)).text_color(ui.text_tertiary).truncate().child(place))
            .child(div().flex_1())
            .child(
                div()
                    .id("appearance")
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .text_size(px(14.))
                    .text_color(ui.text_tertiary)
                    .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_appearance(&ToggleAppearance, window, cx)))
                    .child(moon),
            )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.tabs.get(self.active).map(|t| t.title()).unwrap_or_default();
        window.set_window_title(&format!("{title} — T1K"));
        let active_view = self.tabs.get(self.active).map(|t| t.view.clone());

        macro_rules! select {
            ($d:expr, $($act:ident => $ix:expr),*) => {
                $d$(.on_action(cx.listener(|this, _: &$act, window, cx| this.select_nth($ix, window, cx))))*
            };
        }
        let root = div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_row()
            .bg(self.ui.bg_terminal)
            .font_family(".SystemUIFont")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_appearance))
            .on_action(cx.listener(Self::follow_system))
            .on_action(cx.listener(Self::select_theme))
            .on_modifiers_changed(cx.listener(|this, ev: &gpui::ModifiersChangedEvent, _, cx| {
                if this.alt_held != ev.modifiers.alt {
                    this.alt_held = ev.modifiers.alt;
                    cx.notify();
                }
            }));
        let root = select!(root, SelectTab1 => 0, SelectTab2 => 1, SelectTab3 => 2, SelectTab4 => 3,
            SelectTab5 => 4, SelectTab6 => 5, SelectTab7 => 6, SelectTab8 => 7, SelectTab9 => 8);

        let sidebar = self.sidebar_open.then(|| crate::sidebar::render(self, cx));
        root.children(sidebar).child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .child(self.render_titlebar(cx))
                .child(div().flex_1().min_h_0().children(active_view)),
        )
    }
}
