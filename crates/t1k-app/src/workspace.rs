//! Workspace: the tabs in a window, their project groups, and the active theme.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
use crate::overlay::Overlay;
use crate::session::{self, SavedTab, Session};

const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "login", "nu", "xonsh"];

pub struct Tab {
    pub id: String,
    pub view: Entity<TerminalView>,
    pub osc_title: Option<String>,
    /// Status read from the glyph a program puts before its title (Claude Code's ✳ / spinner).
    pub title_state: Option<crate::title::TitleState>,
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
    pub last_active: Instant,
    pub reminder_at: Option<SystemTime>,
    /// Reminder fired and not yet seen: loud, counts as needs-you.
    pub reminder_due: bool,
    /// Claude session id restored from the last run (for resume).
    pub last_session: Option<String>,
    /// Set when the user moved the tab to a group by hand; folder changes don't regroup it.
    pub pinned_group: Option<PathBuf>,
    /// A second terminal beside or below the main one.
    pub split: Option<crate::split::Split>,
    _sub: Subscription,
}

impl Tab {
    /// Hook-driven agent status; without hooks, what the program's title glyph says while it runs.
    pub fn status(&self) -> Option<&Status> {
        match &self.agent {
            Some(a) => Some(&a.status),
            None if self.program.is_some() => self.title_state.map(|s| s.status()),
            None => None,
        }
    }

    pub fn urgency(&self) -> u8 {
        let base = self.status().map(|s| s.urgency(self.unread)).unwrap_or(if self.unread { 2 } else { 5 });
        if self.reminder_due { base.min(2) } else { base }
    }

    /// Counted by the Needs-you queue: NeedsInput, Error, or a reminder that went off.
    pub fn needs_you(&self) -> bool {
        self.reminder_due || self.status().is_some_and(Status::needs_you)
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

    /// Status line for the detail card and the ⌥ view: what it is doing, else where it is.
    pub fn subtitle(&self) -> String {
        if let Some(a) = &self.agent {
            let line = match &a.status {
                Status::Working => Some(a.now.clone().unwrap_or_else(|| "Thinking…".into())),
                Status::NeedsInput { reason } => Some(reason.clone()),
                Status::Error { message } => Some(message.clone()),
                Status::Done | Status::Idle => a.recap.clone(),
                Status::Ended => a.recap.clone().or_else(|| Some("Session ended".into())),
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
    /// Groups whose quiet tabs are shown even in calm mode.
    pub show_quiet: HashSet<PathBuf>,
    /// Ended Claude sessions per project, newest first (persisted).
    pub recent: Vec<session::Recent>,
    /// Groups whose Recent list is expanded.
    pub recent_open: HashSet<PathBuf>,
    pub group_names: HashMap<PathBuf, String>,
    pub group_colors: HashMap<PathBuf, u8>,
    pub group_sort: HashMap<PathBuf, session::GroupSort>,
    pub sidebar_open: bool,
    /// Sidebar shows two-line rows (hover intent or ⌥ held).
    /// Tab id whose detail card is showing (hover intent on a sidebar row).
    pub card: Option<String>,
    pub alt_held: bool,
    hover_task: Option<Task<()>>,
    mode: AppearanceMode,
    dark: &'static Theme,
    light: &'static Theme,
    pub theme: &'static Theme,
    pub ui: UiColors,
    pub overlay: Overlay,
    /// Tabs visited before / after the current one (ids), for ⌘[ / ⌘].
    pub back: Vec<String>,
    pub forward: Vec<String>,
    navigating: bool,
    pub panel: crate::outputs_panel::PanelState,
    pub overlay_focus: FocusHandle,
    pub config: Config,
    watcher: config::Watcher,
    socket: Option<PathBuf>,
    agent_rx: Receiver<Envelope>,
    next_created: usize,
    pub started: Instant,
    /// Last Dock badge value we set (None = never).
    badge: Option<usize>,
    /// GitHub `owner/repo` per project group, for `#123` links (None: no GitHub remote).
    repos: HashMap<PathBuf, Option<String>>,
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
        let socket = session::support_dir().map(|d| {
            t1k_agent::server::remove_stale(&d);
            t1k_agent::server::socket_path(&d)
        });
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
            show_quiet: HashSet::new(),
            recent: Vec::new(),
            recent_open: HashSet::new(),
            group_names: saved.group_names.iter().cloned().collect(),
            group_colors: saved.group_colors.iter().cloned().collect(),
            group_sort: saved.group_sort.iter().cloned().collect(),
            sidebar_open: saved.sidebar_open.unwrap_or(true),
            card: None,
            alt_held: false,
            hover_task: None,
            mode,
            dark,
            light,
            theme: dark,
            ui: dark.ui(),
            overlay: Overlay::None,
            back: Vec::new(),
            forward: Vec::new(),
            navigating: false,
            panel: crate::outputs_panel::PanelState { open: saved.outputs_open, ..Default::default() },
            overlay_focus: cx.focus_handle(),
            config,
            watcher,
            socket,
            agent_rx,
            next_created: 0,
            started: Instant::now(),
            badge: None,
            repos: HashMap::new(),
            _poll: poll,
            _fast: fast,
        };
        cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)).detach();
        this.apply_theme(window, cx);

        this.recent = saved.recent.clone();
        let restore: Vec<SavedTab> = saved.tabs.into_iter().filter(|t| t.cwd.as_ref().is_none_or(|p| p.is_dir())).collect();
        if restore.is_empty() || std::env::var_os("T1K_FRESH").is_some() {
            this.open_tab(None, None, window, cx);
        } else {
            for t in restore {
                this.open_tab(t.cwd, t.manual_title, window, cx);
                let tab = this.tabs.last_mut().unwrap();
                tab.reminder_at = t.reminder_at.map(|s| UNIX_EPOCH + Duration::from_secs(s));
                // Re-attach the last Claude session's transcript: title, recap and outputs return.
                if let Some(sid) = &t.last_session {
                    if let Some(path) = session::find_transcript(sid) {
                        tab.agent = Some(AgentState::restored(sid.clone(), path));
                    }
                }
                tab.last_session = t.last_session;
                if let Some(g) = t.pinned_group {
                    tab.group = g.clone();
                    tab.pinned_group = Some(g);
                }
                if let Some((cwd, vertical)) = t.split {
                    let ix = this.tabs.len() - 1;
                    this.open_split_in(ix, cwd, vertical, window, cx);
                }
            }
            let active = saved.active.min(this.tabs.len() - 1);
            this.activate(active, window, cx);
            // Restoring opened every tab; that isn't history.
            this.back.clear();
            this.forward.clear();
        }
        // Debug aid for screenshots without a keyboard: T1K_DEBUG_OVERLAY=palette|switcher|
        // overview|settings|menu|groupmenu|termmenu|card opens that surface at launch.
        if let Ok(which) = std::env::var("T1K_DEBUG_OVERLAY") {
            let at = gpui::point(px(120.), px(140.));
            let o = match which.as_str() {
                "palette" => Some(Overlay::Palette { query: String::new(), selected: 0 }),
                "switcher" => Some(Overlay::Switcher { query: String::new(), selected: 0 }),
                "overview" => Some(Overlay::Overview { selected: 0 }),
                "settings" => Some(Overlay::Settings { section: crate::settings::Section::Appearance }),
                "menu" => Some(Overlay::Menu { tab: this.active, at }),
                "groupmenu" => this.tabs.get(this.active).map(|t| Overlay::GroupMenu { key: t.group.clone(), at }),
                "termmenu" => Some(Overlay::TerminalMenu { tab: this.active, split: false, at: gpui::point(px(600.), px(200.)), link: None, has_selection: false }),
                "card" => {
                    this.card = this.tabs.get(this.active).map(|t| t.id.clone());
                    None
                }
                _ => None,
            };
            if let Some(o) = o {
                this.open_overlay(o, window, cx);
            }
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
        let view = self.spawn_view(cwd.clone(), id.clone(), window, cx);
        let sub = cx.subscribe_in(&view, window, |this: &mut Self, view, ev: &TerminalEvent, window, cx| {
            let Some(ix) = this.tabs.iter().position(|t| &t.view == view) else { return };
            match ev {
                TerminalEvent::TitleChanged(t) => {
                    let (state, clean) = crate::title::split(t);
                    let background = ix != this.active;
                    let tab = &mut this.tabs[ix];
                    // Without hooks, a title going from spinner to resting means the turn ended.
                    if tab.agent.is_none()
                        && background
                        && tab.title_state == Some(crate::title::TitleState::Working)
                        && state == Some(crate::title::TitleState::Idle)
                    {
                        tab.unread = true;
                    }
                    tab.title_state = state;
                    tab.osc_title = (!clean.is_empty()).then(|| clean.to_string());
                    cx.notify();
                }
                TerminalEvent::Exited => this.close_tab_at(ix, window, cx),
                TerminalEvent::Bell => {}
                TerminalEvent::OpenLink(link) => this.open_link(ix, link, cx),
                TerminalEvent::ContextMenu { at, link, has_selection } => {
                    let o = Overlay::TerminalMenu { tab: ix, split: false, at: *at, link: link.clone(), has_selection: *has_selection };
                    this.open_overlay(o, window, cx);
                }
            }
        });
        let group = cwd.as_deref().map(groups::project_root).unwrap_or_default();
        let created = self.next_created;
        self.next_created += 1;
        self.tabs.push(Tab {
            id,
            view,
            osc_title: None,
            title_state: None,
            manual_title,
            cwd,
            program: None,
            group,
            agent: None,
            unread: false,
            created,
            last_active: Instant::now(),
            reminder_at: None,
            reminder_due: false,
            last_session: None,
            pinned_group: None,
            split: None,
            _sub: sub,
        });
        self.activate(self.tabs.len() - 1, window, cx);
        self.save();
    }

    /// A new terminal (shell in a pty) with the app's env, theme and style.
    pub fn spawn_view(&self, cwd: Option<PathBuf>, tab_id: String, window: &mut Window, cx: &mut Context<Self>) -> Entity<TerminalView> {
        let mut env = HashMap::new();
        if let Some(sock) = &self.socket {
            env.insert("T1K_SOCKET".into(), sock.display().to_string());
        }
        if let Ok(exe) = std::env::current_exe() {
            env.insert("T1K_BIN".into(), exe.display().to_string());
        }
        if self.config.terminal.shell_integration {
            if let Some(dir) = crate::shell::zsh_dir() {
                if let Ok(orig) = std::env::var("ZDOTDIR") {
                    env.insert("T1K_ORIG_ZDOTDIR".into(), orig);
                }
                env.insert("ZDOTDIR".into(), dir.display().to_string());
            }
        }
        let opts = SpawnOptions { working_directory: cwd, env, tab_id };
        let palette = self.theme.palette();
        let style = self.terminal_style();
        cx.new(|cx| {
            let mut v = TerminalView::new(opts, window, cx).expect("failed to spawn shell in pty");
            v.palette = palette;
            v.style = style;
            v
        })
    }

    fn repo_for(&mut self, group: &PathBuf) -> Option<String> {
        self.repos.entry(group.clone()).or_insert_with(|| crate::links::github_repo(group)).clone()
    }

    /// Which references each terminal turns into links: `#123` only in GitHub repos, ticket
    /// keys only once a tracker is known.
    fn update_link_rules(&mut self, cx: &mut Context<Self>) {
        let tickets = crate::links::ticket_template(&self.config.links.tickets, &self.tabs).is_some();
        for i in 0..self.tabs.len() {
            let group = self.tabs[i].group.clone();
            let prs = !group.as_os_str().is_empty() && self.repo_for(&group).is_some();
            let rules = t1k_terminal::links::LinkRules { files: true, prs, tickets };
            self.tabs[i].view.update(cx, |v, cx| {
                if v.link_rules != rules {
                    v.link_rules = rules;
                    cx.notify();
                }
            });
        }
    }

    pub fn open_link(&mut self, ix: usize, link: &t1k_terminal::links::Link, cx: &mut Context<Self>) {
        let group = self.tabs.get(ix).map(|t| t.group.clone()).unwrap_or_default();
        let repo = self.repo_for(&group);
        let tickets = crate::links::ticket_template(&self.config.links.tickets, &self.tabs);
        match crate::links::action(link, &self.config.links.editor, repo.as_deref(), tickets.as_deref()) {
            crate::links::Action::Url(u) => cx.open_url(&u),
            crate::links::Action::Run(prog, args) => {
                match std::process::Command::new(&prog).args(&args).env("PATH", crate::platform::tool_path()).spawn() {
                    // Reap it so it doesn't linger as a zombie.
                    Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
                    Err(e) => log::error!("open {}: {prog}: {e}", link.label()),
                }
            }
            crate::links::Action::Unavailable(why) => log::info!("{why}"),
        }
    }

    /// Give focus back to the active terminal unless an overlay is open.
    pub fn refocus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.overlay, Overlay::None) {
            return;
        }
        if let Some(t) = self.tabs.get(self.active) {
            let h = t.view.read(cx).focus_handle(cx);
            let split_focused = t.split.as_ref().is_some_and(|s| s.view.read(cx).focus_handle(cx).is_focused(window));
            if !h.is_focused(window) && !split_focused {
                window.focus(&h);
            }
        }
    }

    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // Visiting a tab records where you came from, like a browser; back/forward don't.
        if ix != self.active && !self.navigating {
            if let Some(cur) = self.tabs.get(self.active).map(|t| t.id.clone()) {
                if self.back.last() != Some(&cur) {
                    self.back.push(cur);
                }
                if self.back.len() > 100 {
                    self.back.remove(0);
                }
                self.forward.clear();
            }
        }
        self.active = ix;
        let t = &mut self.tabs[ix];
        t.unread = false;
        t.last_active = Instant::now();
        if t.reminder_due {
            t.reminder_due = false;
            t.reminder_at = None;
        }
        let handle = self.tabs[ix].view.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    /// ⌘[ (`back`) / ⌘]: return to the previously visited tab, skipping closed ones.
    pub fn navigate(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.tabs.get(self.active).map(|t| t.id.clone());
        loop {
            let next = if back { self.back.pop() } else { self.forward.pop() };
            let Some(id) = next else { return };
            if Some(&id) == current.as_ref() {
                continue;
            }
            let Some(ix) = self.tabs.iter().position(|t| t.id == id) else { continue };
            if let Some(cur) = current {
                if back { self.forward.push(cur) } else { self.back.push(cur) }
            }
            self.navigating = true;
            self.activate(ix, window, cx);
            self.navigating = false;
            return;
        }
    }

    /// Whether back / forward have anywhere open to go.
    pub fn can_navigate(&self, back: bool) -> bool {
        let stack = if back { &self.back } else { &self.forward };
        let current = self.tabs.get(self.active).map(|t| t.id.as_str());
        stack.iter().any(|id| Some(id.as_str()) != current && self.tabs.iter().any(|t| &t.id == id))
    }

    pub fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // Move focus to the neighbor in visual order, not in creation order.
        let order = self.visual_order();
        let pos = order.iter().position(|&i| i == ix).unwrap_or(0);
        let next = order.get(pos + 1).or_else(|| pos.checked_sub(1).and_then(|p| order.get(p))).copied();
        self.remember_session(ix);
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
        // Within a group: by the group's sort mode; attention by default (needs-you first, then
        // unread, working, the rest).
        for (g, v) in &mut out {
            match self.group_sort.get(g).copied().unwrap_or_default() {
                session::GroupSort::Attention => v.sort_by_key(|&i| (self.tabs[i].urgency(), self.tabs[i].created)),
                session::GroupSort::Activity => v.sort_by_key(|&i| std::cmp::Reverse(self.tabs[i].last_active)),
                session::GroupSort::Opened => v.sort_by_key(|&i| self.tabs[i].created),
            }
        }
        out
    }

    /// Tab indices in the order the sidebar shows them (collapsed groups included).
    pub fn visual_order(&self) -> Vec<usize> {
        self.grouped().into_iter().flat_map(|(_, v)| v).collect()
    }

    /// The group's color, if the user picked one: one of the theme's six ANSI hues.
    pub fn group_color(&self, key: &PathBuf) -> Option<gpui::Hsla> {
        let i = *self.group_colors.get(key)? as usize;
        let c = self.theme.ansi.get(i.clamp(1, 6))?;
        Some(gpui::rgb(*c).into())
    }

    /// Move a tab into a group by hand, or back to its folder's group with `None`.
    pub fn move_tab(&mut self, ix: usize, group: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(ix) else { return };
        match group {
            Some(g) => {
                t.group = g.clone();
                t.pinned_group = Some(g);
            }
            None => {
                t.pinned_group = None;
                if let Some(c) = &t.cwd {
                    t.group = groups::project_root(c);
                }
            }
        }
        self.save();
        cx.notify();
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
            if let Some(sp) = t.split.as_mut() {
                if let Some(c) = sp.view.read(cx).terminal.cwd() {
                    sp.cwd = Some(c);
                }
            }
            if cwd.is_some() && cwd != t.cwd {
                let group = groups::project_root(cwd.as_deref().unwrap());
                if t.pinned_group.is_none() && group != t.group {
                    t.group = group;
                    regrouped = true;
                }
                t.cwd = cwd;
                changed = true;
            }
        }
        self.update_link_rules(cx);
        if changed {
            cx.notify();
        }
        if regrouped || changed {
            self.save();
        }
        self.refresh_prs(cx);
        if changed {
            self.debug_dump();
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
        // A clicked banner brings its tab forward.
        for id in crate::platform::take_clicked() {
            if let Some(ix) = self.tabs.iter().position(|t| t.id == id) {
                window.activate_window();
                self.activate(ix, window, cx);
            }
        }
        let mut changed = false;
        let now = SystemTime::now();
        for t in &mut self.tabs {
            if !t.reminder_due && t.reminder_at.is_some_and(|r| r <= now) {
                t.reminder_due = true;
                changed = true;
                if self.config.notify.reminder {
                    crate::platform::notify_tab(&t.id, "T1K", "Reminder", &t.title());
                }
            }
        }
        while let Ok(env) = self.agent_rx.try_recv() {
            changed |= self.apply_envelope(env, window);
        }
        if changed {
            self.debug_dump();
        }
        let needs = self.needs_queue().len();
        if self.badge != Some(needs) {
            self.badge = Some(needs);
            crate::platform::set_dock_badge(if self.config.notify.dock_badge { needs } else { 0 });
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
            // The session this tab held before goes to Recent.
            if tab.agent.as_ref().is_some_and(|a| !a.session_id.is_empty()) {
                self.remember_session(ix);
            }
            let tab = &mut self.tabs[ix];
            tab.agent = Some(AgentState::new(sid));
        }
        let tab = &mut self.tabs[ix];
        let before = tab.agent.as_ref().map(|a| a.status.clone());
        let finished = tab.agent.as_mut().unwrap().apply_hook(hook);
        if finished && !visible {
            tab.unread = true;
        }
        // Banner on entering a loud state while you're not looking at this tab.
        let after = tab.agent.as_ref().map(|a| a.status.clone());
        if !visible && after != before {
            let n = &self.config.notify;
            let msg = match &after {
                Some(Status::NeedsInput { reason }) if n.needs_input => Some(("Needs you", reason.clone())),
                Some(Status::Error { message }) if n.error => Some(("Error", message.clone())),
                Some(Status::Done) if n.done && finished => {
                    Some(("Done", tab.agent.as_ref().and_then(|a| a.recap.clone()).unwrap_or_default()))
                }
                _ => None,
            };
            if let Some((what, body)) = msg {
                crate::platform::notify_tab(&tab.id, &format!("T1K · {what}"), &tab.title(), &body);
            }
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
                for o in t.agent.iter().flat_map(|a| a.outputs.iter()) {
                    let pr = self.panel.prs.get(&o.target).cloned().flatten().map(|p| format!(" [{} {:?}]", p.state, p.checks));
                    out += &format!("      {} {}{}\n", o.kind.label(), o.label, pr.unwrap_or_default());
                }
            }
        }
        let _ = std::fs::write(path, out);
    }

    /// Type `claude --resume <id>` into a tab sitting at its shell prompt.
    /// Put a tab's Claude session into Recent (on close, or when a new session replaces it).
    fn remember_session(&mut self, ix: usize) {
        let Some(t) = self.tabs.get(ix) else { return };
        let sid = t.agent.as_ref().map(|a| a.session_id.clone()).filter(|s| !s.is_empty()).or_else(|| t.last_session.clone());
        let (Some(sid), Some(cwd)) = (sid, t.cwd.clone()) else { return };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.recent.insert(
            0,
            session::Recent { group: t.group.clone(), cwd, session_id: sid, title: t.title(), ended_at: now },
        );
        session::prune_recent(&mut self.recent, now);
    }

    /// Recent sessions for a group that no open tab holds.
    pub fn recent_for(&self, group: &PathBuf) -> Vec<&session::Recent> {
        let open: HashSet<&str> = self.tabs.iter().filter_map(|t| t.agent.as_ref()).map(|a| a.session_id.as_str()).collect();
        self.recent.iter().filter(|r| &r.group == group && !open.contains(r.session_id.as_str())).collect()
    }

    /// Open a Recent session in a new tab: `claude --resume` in its folder.
    pub fn reopen_recent(&mut self, session_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.recent.iter().find(|r| r.session_id == session_id).cloned() else { return };
        let cwd = r.cwd.is_dir().then(|| r.cwd.clone());
        self.open_tab(cwd, None, window, cx);
        let ix = self.tabs.len() - 1;
        self.tabs[ix].last_session = Some(r.session_id.clone());
        // The shell reads this once it's up; typeahead is kept until then.
        self.tabs[ix].view.read(cx).terminal.write(format!("claude --resume {}\r", r.session_id).into_bytes());
        self.recent.retain(|x| x.session_id != r.session_id);
        self.save();
    }

    pub fn resume_session(&mut self, tab: usize, session_id: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get(tab) {
            if t.program.is_none() {
                t.view.read(cx).terminal.write(format!("claude --resume {session_id}\r").into_bytes());
            }
        }
    }

    /// Tabs that need you, longest-waiting first.
    pub fn needs_queue(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].needs_you()).collect();
        v.sort_by_key(|&i| {
            let t = &self.tabs[i];
            // Reminders wait from when they fired; agents from when their state began.
            (t.agent.as_ref().filter(|a| a.status.needs_you()).map(|a| a.since), t.reminder_at)
        });
        v
    }

    fn jump_to_needs(&mut self, _: &JumpToNeeds, window: &mut Window, cx: &mut Context<Self>) {
        self.jump_next_needs(window, cx);
    }

    pub fn jump_next_needs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.walk_needs(1, window, cx);
    }

    /// Walk the Needs-you queue (longest-waiting first) from the current tab.
    fn walk_needs(&mut self, dir: isize, window: &mut Window, cx: &mut Context<Self>) {
        let q = self.needs_queue();
        if q.is_empty() {
            return;
        }
        let n = q.len() as isize;
        let next = match q.iter().position(|&i| i == self.active) {
            Some(p) => q[((p as isize + dir).rem_euclid(n)) as usize],
            None if dir > 0 => q[0],
            None => q[(n - 1) as usize],
        };
        self.activate(next, window, cx);
    }

    /// Tabs folded into "+N quiet" in calm mode: nothing to see, not selected.
    pub fn is_quiet(&self, i: usize) -> bool {
        let t = &self.tabs[i];
        i != self.active
            && !t.unread
            && !t.reminder_due
            && t.reminder_at.is_none()
            && t.program.is_none()
            && matches!(t.status(), None | Some(Status::Done) | Some(Status::Idle) | Some(Status::Ended))
    }

    pub fn session(&self) -> Session {
        Session {
            tabs: self.visual_order().iter().map(|&i| {
                let t = &self.tabs[i];
                SavedTab {
                    cwd: t.cwd.clone(),
                    manual_title: t.manual_title.clone(),
                    reminder_at: t.reminder_at.and_then(|r| r.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()),
                    last_session: t.agent.as_ref().map(|a| a.session_id.clone()).filter(|s| !s.is_empty())
                        .or_else(|| t.last_session.clone()),
                    pinned_group: t.pinned_group.clone(),
                    split: t.split.as_ref().map(|s| (s.cwd.clone(), s.vertical)),
                }
            }).collect(),
            active: self.visual_order().iter().position(|&i| i == self.active).unwrap_or(0),
            collapsed: self.collapsed.iter().cloned().collect(),
            group_names: self.group_names.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            sidebar_open: Some(self.sidebar_open),
            outputs_open: self.panel.open,
            recent: self.recent.clone(),
            group_colors: self.group_colors.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            group_sort: self.group_sort.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            ..Default::default()
        }
    }

    pub fn save(&self) {
        session::save(&self.session());
    }

    // ---- sidebar detail card ----------------------------------------------------------------

    /// Hover intent for the sidebar detail card. The first card waits for the pointer to rest;
    /// once one is showing, moving to another row switches at once. Leaving hides it shortly
    /// after, unless another row picks it up first. Rows never change size.
    pub fn row_hovered(&mut self, id: String, hovered: bool, cx: &mut Context<Self>) {
        if hovered && self.card.is_some() {
            self.hover_task = None;
            if self.card.as_ref() != Some(&id) {
                self.card = Some(id);
                cx.notify();
            }
            return;
        }
        let delay = if hovered { self.config.sidebar.hover_intent_ms } else { 120 };
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            let _ = this.update(cx, |ws, cx| {
                let next = hovered.then(|| id.clone());
                if !hovered && ws.card.as_ref() != Some(&id) {
                    return;
                }
                if ws.card != next {
                    ws.card = next;
                    cx.notify();
                }
            });
        }));
    }

    pub fn hide_card(&mut self, cx: &mut Context<Self>) {
        self.hover_task = None;
        if self.card.take().is_some() {
            cx.notify();
        }
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
        // ⌘W in a split closes just that pane.
        let split_focused = self.tabs.get(self.active).and_then(|t| t.split.as_ref()).is_some_and(|s| s.view.read(cx).focus_handle(cx).is_focused(window));
        if split_focused {
            self.close_split(self.active, window, cx);
            return;
        }
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

    /// ⌘+ / ⌘− / ⌘0: written to config.toml, so it sticks and the reload applies it everywhere.
    fn set_font_size(&mut self, size: f32) {
        let size = size.clamp(8., 32.).round();
        if size != self.config.terminal.font.size {
            crate::config::set_value(&["terminal", "font", "size"], toml_edit::Value::from(size as i64));
        }
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
            copy_on_select: t.copy_on_select,
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

    fn active_output_count(&self) -> Option<usize> {
        let n = self.tabs.get(self.active)?.agent.as_ref()?.outputs.len();
        (n > 0).then_some(n)
    }

    pub fn toggle_outputs(&mut self, cx: &mut Context<Self>) {
        self.panel.open = !self.panel.open;
        self.refresh_prs(cx);
        self.save();
        cx.notify();
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
                    .id("outputs-toggle")
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .text_color(if self.panel.open { ui.text_primary } else { ui.text_tertiary })
                    .when(self.panel.open, |d| d.bg(ui.bg_selected))
                    .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_outputs(cx)))
                    .child("Outputs")
                    .children(self.active_output_count().map(|n| div().text_color(ui.text_tertiary).child(n.to_string()))),
            )
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
        // Cached: sidebar spinners and other workspace repaints reuse the terminal's last frame
        // unless the terminal itself changed.
        let active_view = self.tabs.get(self.active).map(|t| crate::split::render_panes(t, self.ui, window, cx));

        macro_rules! select {
            ($d:expr, $($act:ident => $ix:expr),*) => {
                $d$(.on_action(cx.listener(|this, _: &$act, window, cx| this.select_nth($ix, window, cx))))*
            };
        }
        let root = div()
            .key_context("Workspace")
            // The workspace takes focus only when there's no terminal to hold it. If it tracked
            // focus always, every click on chrome (sidebar, title bar, outputs panel) would move
            // focus off the terminal; the terminal keeps it unless an overlay takes it.
            .when(self.tabs.is_empty(), |d| d.track_focus(&self.focus_handle))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| this.refocus_terminal(window, cx)))
            .on_mouse_up(MouseButton::Right, cx.listener(|this, _, window, cx| this.refocus_terminal(window, cx)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|this, _, window, cx| this.refocus_terminal(window, cx)))
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
            .on_action(cx.listener(|this, _: &JumpToNeedsBack, window, cx| this.walk_needs(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSwitcher, window, cx| {
                if matches!(this.overlay, Overlay::Switcher { .. }) {
                    this.close_overlay(window, cx);
                } else {
                    this.open_overlay(Overlay::Switcher { query: String::new(), selected: 0 }, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleOverview, window, cx| {
                if matches!(this.overlay, Overlay::Overview { .. }) {
                    this.close_overlay(window, cx);
                } else {
                    let selected = this.visual_order().iter().position(|&i| i == this.active).unwrap_or(0);
                    this.open_overlay(Overlay::Overview { selected }, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RenameTab, window, cx| this.start_rename(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleOutputs, _, cx| this.toggle_outputs(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_overlay(Overlay::Settings { section: crate::settings::Section::Appearance }, window, cx)
            }))
            .on_action(cx.listener(Self::toggle_appearance))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| this.open_split(this.active, false, window, cx)))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| this.open_split(this.active, true, window, cx)))
            .on_action(cx.listener(|this, _: &FocusOtherPane, window, cx| this.focus_other_pane(window, cx)))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.navigate(true, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, window, cx| {
                if matches!(this.overlay, Overlay::Palette { .. }) {
                    this.close_overlay(window, cx);
                } else {
                    this.open_overlay(Overlay::Palette { query: String::new(), selected: 0 }, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.navigate(false, window, cx)))
            .on_action(cx.listener(|this, _: &IncreaseFontSize, _, _| this.set_font_size(this.config.terminal.font.size + 1.)))
            .on_action(cx.listener(|this, _: &DecreaseFontSize, _, _| this.set_font_size(this.config.terminal.font.size - 1.)))
            .on_action(cx.listener(|this, _: &ResetFontSize, _, _| this.set_font_size(crate::config::FontCfg::default().size)))
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
        let overlay = crate::overlay::render(self, window, cx);
        let panel = self.panel.open.then(|| crate::outputs_panel::render(self, cx));
        root.relative().children(sidebar).child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .child(self.render_titlebar(cx))
                .child(div().flex_1().min_h_0().children(active_view)),
        )
        .children(panel)
        .children(overlay)
    }
}
