//! Workspace: the tabs in a window, their project groups, and the active theme.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, App, Context, Entity, FocusHandle, Focusable, MouseButton, Subscription, Task, Window, WindowAppearance,
};
use t1k_agent::hooks::Hook;
use t1k_agent::server::Envelope;
use t1k_agent::{AgentState, Status};
use t1k_terminal::{SpawnOptions, TerminalEvent, TerminalStyle, TerminalView};
use t1k_themes::{Appearance, AppearanceMode, Theme, UiColors};

use crate::actions::*;
use crate::config::{self, Config};
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
    /// Claude session running in this tab, if any (from hooks).
    pub agent: Option<AgentState>,
    /// Finished while you were elsewhere; cleared when the tab is opened.
    pub unread: bool,
    pub created: usize,
    _sub: Subscription,
}

impl Tab {
    pub fn status(&self) -> Option<&Status> {
        self.agent.as_ref().map(|a| &a.status)
    }

    pub fn urgency(&self) -> u8 {
        self.status().map(|s| s.urgency(self.unread)).unwrap_or(if self.unread { 2 } else { 5 })
    }

    pub fn needs_you(&self) -> bool {
        self.status().is_some_and(Status::needs_you)
    }

    /// Display title. Precedence: manual rename, then the program's OSC title while a program
    /// runs, then the folder name while at the shell prompt. (Claude's ai-title slots in at M4.)
    pub fn title(&self) -> String {
        if let Some(t) = &self.manual_title {
            return t.clone();
        }
        if let Some(t) = self.agent.as_ref().and_then(|a| a.title()) {
            return t;
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
        if let Some(a) = &self.agent {
            let line = match &a.status {
                Status::Working => Some(a.now.clone().unwrap_or_else(|| "Thinking…".into())),
                Status::NeedsInput { reason } => Some(reason.clone()),
                Status::Error { message } => Some(message.clone()),
                Status::Done | Status::Idle => a.recap.clone(),
                Status::Ended => Some("Session ended".into()),
            };
            if let Some(l) = line {
                return l;
            }
        }
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
    pub config: Config,
    watcher: config::Watcher,
    socket: Option<PathBuf>,
    agent_rx: Receiver<Envelope>,
    next_created: usize,
    pub started: Instant,
    _poll: Task<()>,
    _fast: Task<()>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = session::load();
        if let Some(d) = config::themes_dir() {
            t1k_themes::load_user_themes(&d);
        }
        let mut watcher = config::Watcher::new();
        let config = match watcher.poll() {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                log::error!("{e}");
                Config::default()
            }
            None => Config::default(),
        };
        let (mut mode, mut dark, mut light) = Self::slots(&config);
        match std::env::var("T1K_APPEARANCE").as_deref() {
            Ok("dark") => mode = AppearanceMode::Dark,
            Ok("light") => mode = AppearanceMode::Light,
            _ => {}
        }
        if let Some(t) = std::env::var("T1K_THEME").ok().and_then(|n| t1k_themes::find(&n)) {
            if t.is_dark() { dark = t } else { light = t }
            mode = if t.is_dark() { AppearanceMode::Dark } else { AppearanceMode::Light };
        }

        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(800)).await;
            if this.update(cx, |ws, cx| ws.poll(cx)).is_err() {
                break;
            }
        });

        // Agent socket: hooks in this window's tabs report here.
        let (tx, agent_rx) = channel();
        let socket = session::support_dir().map(|d| t1k_agent::server::socket_path(&d));
        let socket = socket.filter(|p| match t1k_agent::server::serve(p, tx, || {}) {
            Ok(()) => true,
            Err(e) => {
                log::error!("agent socket {}: {e}", p.display());
                false
            }
        });
        // Fast loop: drain agent messages and animate spinners.
        let fast = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            if this.update_in(cx, |ws, window, cx| ws.tick(window, cx)).is_err() {
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
            config,
            watcher,
            socket,
            agent_rx,
            next_created: 0,
            started: Instant::now(),
            _poll: poll,
            _fast: fast,
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
        let mut env = HashMap::new();
        if let Some(sock) = &self.socket {
            env.insert("T1K_SOCKET".into(), sock.display().to_string());
        }
        if let Ok(exe) = std::env::current_exe() {
            env.insert("T1K_BIN".into(), exe.display().to_string());
        }
        let opts = SpawnOptions { working_directory: cwd.clone(), env, tab_id: id.clone() };
        let palette = self.theme.palette();
        let style = self.terminal_style();
        let view = cx.new(|cx| {
            let mut v = TerminalView::new(opts, window, cx).expect("failed to spawn shell in pty");
            v.palette = palette;
            v.style = style;
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
        let created = self.next_created;
        self.next_created += 1;
        self.tabs.push(Tab {
            id,
            view,
            osc_title: None,
            manual_title,
            cwd,
            program: None,
            group,
            agent: None,
            unread: false,
            created,
            _sub: sub,
        });
        self.activate(self.tabs.len() - 1, window, cx);
        self.save();
    }

    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active = ix;
        self.tabs[ix].unread = false;
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
        // Within a group: needs-you first, then unread, working, the rest (design: ordering).
        for (_, v) in &mut out {
            v.sort_by_key(|&i| (self.tabs[i].urgency(), self.tabs[i].created));
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
            if let Some(a) = t.agent.as_mut() {
                changed |= a.poll_transcript();
            }
            let term = &t.view.read(cx).terminal;
            let cwd = term.cwd();
            let program = term.foreground_name().filter(|n| !SHELLS.contains(&n.trim_start_matches('-')));
            if program != t.program {
                // Claude exited without a SessionEnd (crash, kill): the session is over.
                if program.is_none() {
                    if let Some(a) = t.agent.as_mut().filter(|a| a.status != Status::Ended) {
                        a.apply_hook(Hook::SessionEnd);
                    }
                }
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

    /// Every 100 ms: apply agent messages; keep spinners moving while anything works.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.watcher.poll() {
            Some(Ok(c)) => {
                if c != self.config {
                    self.config = c;
                    self.apply_config(window, cx);
                }
            }
            Some(Err(e)) => log::error!("config not applied: {e}"),
            None => {}
        }
        let mut changed = false;
        while let Ok(env) = self.agent_rx.try_recv() {
            changed |= self.apply_envelope(env, window);
        }
        if changed {
            self.debug_dump();
        }
        let working = self.tabs.iter().any(|t| t.status() == Some(&Status::Working));
        if changed || working {
            cx.notify();
        }
    }

    fn apply_envelope(&mut self, env: Envelope, window: &Window) -> bool {
        let Some(ix) = self.tabs.iter().position(|t| t.id == env.tab_id) else { return false };
        let Some((hook, session_id)) = Hook::parse(&env.payload) else { return false };
        let visible = ix == self.active && window.is_window_active();
        let tab = &mut self.tabs[ix];
        let sid = session_id.unwrap_or_default();
        let new_session = tab.agent.as_ref().is_none_or(|a| !sid.is_empty() && a.session_id != sid);
        if new_session {
            tab.agent = Some(AgentState::new(sid));
        }
        let finished = tab.agent.as_mut().unwrap().apply_hook(hook);
        if finished && !visible {
            tab.unread = true;
        }
        true
    }

    /// With T1K_DEBUG_DUMP=<file>, write the sidebar model as text (for tests without a screen).
    fn debug_dump(&self) {
        let Some(path) = std::env::var_os("T1K_DEBUG_DUMP") else { return };
        let mut out = format!("theme={} font={} {}\nneeds_you={}\n", self.theme.name, self.config.terminal.font.family, self.config.terminal.font.size, self.needs_queue().len());
        for (g, tabs) in self.grouped() {
            out += &format!("[{}]\n", self.group_label(&g));
            for i in tabs {
                let t = &self.tabs[i];
                out += &format!(
                    "  {} | {:?} | unread={} | {}\n",
                    t.title(),
                    t.status(),
                    t.unread,
                    t.subtitle()
                );
            }
        }
        let _ = std::fs::write(path, out);
    }

    /// Tabs that need you, longest-waiting first.
    pub fn needs_queue(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].needs_you()).collect();
        v.sort_by_key(|&i| self.tabs[i].agent.as_ref().map(|a| a.since));
        v
    }

    fn jump_to_needs(&mut self, _: &JumpToNeeds, window: &mut Window, cx: &mut Context<Self>) {
        self.jump_next_needs(window, cx);
    }

    pub fn jump_next_needs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let q = self.needs_queue();
        if let Some(&ix) = q.iter().find(|&&i| i != self.active).or(q.first()) {
            self.activate(ix, window, cx);
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
            ..Default::default()
        }
    }

    pub fn save(&self) {
        session::save(&self.session());
    }

    // ---- sidebar detail mode ----------------------------------------------------------------

    /// Hover intent: expand after the pointer rests 150 ms, collapse 400 ms after it leaves.
    pub fn sidebar_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        let delay = if hovered { self.config.sidebar.hover_intent_ms } else { 400 };
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

    /// Appearance mode and the two theme slots, as configured (unknown names fall back).
    fn slots(c: &Config) -> (AppearanceMode, &'static Theme, &'static Theme) {
        let mode = match c.appearance.as_str() {
            "dark" => AppearanceMode::Dark,
            "light" => AppearanceMode::Light,
            _ => AppearanceMode::System,
        };
        let dark = t1k_themes::find(&c.theme.dark).unwrap_or(t1k_themes::default_for(Appearance::Dark));
        let light = t1k_themes::find(&c.theme.light).unwrap_or(t1k_themes::default_for(Appearance::Light));
        (mode, dark, light)
    }

    pub fn terminal_style(&self) -> TerminalStyle {
        let t = &self.config.terminal;
        TerminalStyle {
            font_family: t.font.family.clone().into(),
            font_fallbacks: t.font.fallbacks.clone(),
            font_size: px(t.font.size.clamp(6., 72.)),
            line_height: t.font.line_height.clamp(0.8, 3.),
            padding: px(t.padding.clamp(0., 64.)),
            option_as_meta: t.option_as_meta,
        }
    }

    /// Re-read themes and push config to the theme and every terminal.
    fn apply_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = config::themes_dir() {
            t1k_themes::load_user_themes(&d);
        }
        (self.mode, self.dark, self.light) = Self::slots(&self.config);
        self.apply_theme(window, cx);
        let style = self.terminal_style();
        for tab in &self.tabs {
            tab.view.update(cx, |v, cx| {
                v.style = style.clone();
                cx.notify();
            });
        }
        crate::menus::install(cx);
        self.debug_dump();
    }

    fn write_appearance(&self) {
        let mode = match self.mode {
            AppearanceMode::System => "system",
            AppearanceMode::Dark => "dark",
            AppearanceMode::Light => "light",
        };
        config::set(&["appearance"], mode);
        config::set(&["theme", "dark"], &self.dark.name);
        config::set(&["theme", "light"], &self.light.name);
    }

    /// Import dropped theme files into the themes folder and switch to the first one.
    pub fn import_themes(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let mut first = None;
        for p in paths {
            match crate::cli::import_theme(p) {
                Ok(name) => {
                    first.get_or_insert(name);
                }
                Err(e) => log::error!("theme import {}: {e}", p.display()),
            }
        }
        if let Some(name) = first {
            self.apply_config(window, cx);
            self.select_theme(&SelectTheme { name: name.into() }, window, cx);
        }
    }

    fn toggle_appearance(&mut self, _: &ToggleAppearance, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = if self.theme.is_dark() { AppearanceMode::Light } else { AppearanceMode::Dark };
        self.apply_theme(window, cx);
        self.write_appearance();
    }

    fn follow_system(&mut self, _: &FollowSystemAppearance, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = AppearanceMode::System;
        self.apply_theme(window, cx);
        self.write_appearance();
    }

    pub fn select_theme(&mut self, a: &SelectTheme, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = t1k_themes::find(&a.name) else { return };
        if t.is_dark() { self.dark = t } else { self.light = t }
        // Show the pick now. In System mode, keep following the system if it already shows this slot.
        let showing_slot = self.resolve_theme(window).is_dark() == t.is_dark();
        if !(self.mode == AppearanceMode::System && showing_slot) {
            self.mode = if t.is_dark() { AppearanceMode::Dark } else { AppearanceMode::Light };
        }
        self.apply_theme(window, cx);
        self.write_appearance();
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
        let needs = self.needs_queue().len();
        let badge = if needs > 0 { format!("({needs}) ") } else { String::new() };
        window.set_window_title(&format!("{badge}{title} — T1K"));
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
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                this.import_themes(paths.paths(), window, cx);
            }))
            .on_action(cx.listener(Self::jump_to_needs))
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
