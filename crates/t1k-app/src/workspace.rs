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
    /// Git branch of the tab's folder (refreshed while polling).
    pub branch: Option<String>,
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
    /// Outputs shown in the panel: everything but single files, plus one entry for edited files.
    pub fn outputs_count(&self) -> usize {
        self.agent
            .as_ref()
            .map(|a| a.outputs.iter().filter(|o| o.kind != t1k_agent::outputs::Kind::File).count() + usize::from(a.files_edited() > 0))
            .unwrap_or(0)
    }

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

    /// Title for the sidebar's "Title as" setting. A manual rename always wins.
    pub fn title_as(&self, mode: &str) -> String {
        if let Some(t) = &self.manual_title {
            return t.clone();
        }
        let folder = || self.cwd.as_deref().map(groups::label).unwrap_or_else(|| "shell".into());
        match mode {
            "command" => match &self.program {
                Some(p) => self.osc_title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| p.clone()),
                None => folder(),
            },
            "folder" => folder(),
            "branch" => self.branch().unwrap_or_else(folder),
            _ => self.title(),
        }
    }

    /// The agent's branch, else the folder's.
    pub fn branch(&self) -> Option<String> {
        self.agent.as_ref().and_then(|a| a.branch.clone()).or_else(|| self.branch.clone())
    }

    /// The always-on second line for the sidebar's "Second line" setting.
    pub fn second_line(&self, mode: &str) -> Option<String> {
        match mode {
            "status" => Some(self.subtitle()),
            "branch" => self.branch().map(|b| format!("⎇ {b}")),
            "folder" => self.cwd.as_deref().map(groups::tilde),
            _ => None,
        }
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
    /// Live sidebar-edge drag: what releasing would do.
    pub sidebar_drag: Option<crate::rail::DragPreview>,
    /// The full sidebar shown over the terminal from the rail.
    pub peek: bool,
    pub peek_gen: u64,
    pub peek_task: Option<Task<()>>,
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
    /// Whether T1K's Claude Code hooks are in ~/.claude/settings.json (refreshed while polling).
    pub hooks_installed: bool,
    /// The window's frame, kept current as it moves and resizes; saved with the session.
    window_frame: Option<session::SavedWindow>,
    saved_frame: Option<session::SavedWindow>,
    /// The file open in the side panel, if any.
    pub viewer: Option<crate::viewer::Viewer>,
    /// Text typed into Settings' search.
    pub settings_query: String,
    /// Settings › Keybindings: the action waiting for a new shortcut.
    pub key_recording: Option<&'static str>,
    /// The cheap-model helper's queue and switcher search.
    pub helper: crate::helper::HelperState,
    /// Tabs whose agent finished a turn since the last tick (for the helper).
    turns_done: Vec<String>,
    /// The sidebar list's scroll position (for the sticky group header).
    pub sidebar_scroll: gpui::ScrollHandle,
    /// The switcher's filter chip (back to All when it closes).
    pub switcher_filter: crate::overlay::SwitcherFilter,
    /// While the theme picker previews: what to go back to on esc.
    theme_before: Option<(AppearanceMode, &'static Theme, &'static Theme)>,
    /// The Settings page a drill-down (the theme picker) was opened from; Esc and ‹ return there.
    pub settings_return: Option<crate::settings::Section>,
    /// Outputs count per tab id when the user last saw them (panel open on that tab).
    pub outputs_seen: HashMap<String, usize>,
    /// Scrollback files were written for every tab (at quit); the session points at them.
    scrollback_saved: bool,
    /// Scrollback to replay into the next terminal `open_tab` spawns (session restore).
    restore_next: Option<Vec<u8>>,
    /// The last shortcut change, shown under its row ("Was on Find tab").
    pub key_note: Option<(&'static str, String)>,
    _key_interceptor: gpui::Subscription,
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
    /// Dock icon last set: (style, needs-you).
    dock_icon: Option<(String, bool)>,
    /// Last `updater::rev()` drawn.
    update_rev: u64,
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
        // Every 33 ms the spinner advances (only while something works and the sidebar shows);
        // every third pass also does the 100 ms housekeeping in `tick`.
        let fast = cx.spawn_in(window, async move |this, cx| {
            let mut n: u32 = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                n = n.wrapping_add(1);
                let alive = this.update_in(cx, |ws, window, cx| {
                    if n.is_multiple_of(3) {
                        ws.tick(window, cx);
                    } else if ws.sidebar_open && ws.tabs.iter().any(|t| t.status() == Some(&Status::Working)) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
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
            sidebar_drag: None,
            peek: false,
            peek_gen: 0,
            peek_task: None,
            card: None,
            alt_held: false,
            hover_task: None,
            mode,
            dark,
            light,
            theme: dark,
            ui: dark.ui(),
            overlay: Overlay::None,
            hooks_installed: crate::settings::claude_hooks_installed(),
            settings_query: String::new(),
            key_recording: None,
            helper: Default::default(),
            turns_done: Vec::new(),
            sidebar_scroll: gpui::ScrollHandle::new(),
            switcher_filter: Default::default(),
            theme_before: None,
            settings_return: None,
            outputs_seen: HashMap::new(),
            scrollback_saved: false,
            restore_next: None,
            key_note: None,
            _key_interceptor: {
                // Recording a shortcut must see keys before any binding fires (⌘P would
                // otherwise open the switcher instead of being recorded).
                let weak = cx.entity().downgrade();
                cx.intercept_keystrokes(move |ev, _window, cx| {
                    let Some(ws) = weak.upgrade() else { return };
                    if ws.read(cx).key_recording.is_none() {
                        return;
                    }
                    let ks = ev.keystroke.clone();
                    ws.update(cx, |ws, cx| ws.record_key(&ks, cx));
                    cx.stop_propagation();
                })
            },
            viewer: None,
            window_frame: None,
            saved_frame: None,
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
            dock_icon: None,
            update_rev: 0,
            repos: HashMap::new(),
            _poll: poll,
            _fast: fast,
        };
        cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)).detach();
        crate::updater::start(&this.config.updates);
        // Remember where the window is; saved with the session (debounced through `poll`).
        cx.observe_window_bounds(window, |this, window, _| this.window_frame = Some(frame_of(window))).detach();
        cx.on_app_quit(|this, cx| {
            this.save_scrollback(cx);
            this.save();
            async {}
        })
        .detach();
        this.apply_theme(window, cx);

        this.recent = saved.recent.clone();
        this.window_frame = Some(frame_of(window));
        // First launch ever: no saved session and never welcomed.
        let first_launch = !saved.welcomed && saved.tabs.is_empty();
        let restore: Vec<SavedTab> = saved.tabs.into_iter().filter(|t| t.cwd.as_ref().is_none_or(|p| p.is_dir())).collect();
        if restore.is_empty() || std::env::var_os("T1K_FRESH").is_some() {
            this.open_tab(None, None, window, cx);
        } else {
            let dir = session::support_dir().map(|d| d.join("scrollback"));
            for t in restore {
                if this.config.terminal.restore_scrollback {
                    this.restore_next = t.scrollback.as_ref().zip(dir.as_ref()).and_then(|(f, d)| std::fs::read(d.join(f)).ok()).map(restored_banner);
                }
                this.open_tab(t.cwd, t.manual_title, window, cx);
                let tab = this.tabs.last_mut().unwrap();
                tab.reminder_at = t.reminder_at.map(|s| UNIX_EPOCH + Duration::from_secs(s));
                // Re-attach the last Claude session's transcript: title, recap and outputs return.
                if let Some(sid) = &t.last_session {
                    if let Some(path) = session::find_transcript(sid) {
                        tab.agent = Some(AgentState::restored(sid.clone(), path));
                    }
                }
                // Claude was mid-session when T1K quit: pick it back up.
                if t.agent_live && this.config.terminal.resume_agents {
                    if let Some(sid) = &t.last_session {
                        tab.view.read(cx).terminal.write(format!("claude --resume {sid}\r").into_bytes());
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
            // Read once: the next quit writes fresh files.
            if let Some(d) = &dir {
                let _ = std::fs::remove_dir_all(d);
            }
            let active = saved.active.min(this.tabs.len() - 1);
            this.activate(active, window, cx);
            // Restoring opened every tab; that isn't history.
            this.back.clear();
            this.forward.clear();
        }
        if first_launch {
            this.open_overlay(Overlay::Welcome, window, cx);
        }
        // Debug aid: T1K_DEBUG_SIDEBAR=rail|hidden|peek|drag:<x> starts collapsed / peeking /
        // mid-drag with the pointer at x.
        match std::env::var("T1K_DEBUG_SIDEBAR").as_deref() {
            Ok("rail") => this.sidebar_open = false,
            Ok("hidden") => {
                this.sidebar_open = false;
                this.config.sidebar.collapsed = "hidden".into();
            }
            Ok("peek") => {
                this.sidebar_open = false;
                this.peek = true;
            }
            Ok(d) if d.starts_with("drag:") => {
                this.sidebar_drag = d[5..].parse::<f32>().ok().map(crate::rail::preview_for);
            }
            _ => {}
        }
        // Debug aid: T1K_DEBUG_AGENT=working|needs|error|done fakes a Claude session on the
        // active tab, for screenshots of the strip, sidebar and card.
        if let Ok(state) = std::env::var("T1K_DEBUG_AGENT") {
            if let Some(t) = this.tabs.get_mut(this.active) {
                use t1k_agent::hooks::Hook;
                let mut a = AgentState::new("debug".into());
                a.apply_hook(Hook::UserPromptSubmit { prompt: "Link hint mode for terminal".into() });
                a.apply_hook(Hook::PreToolUse { tool: "Edit".into(), input: serde_json::json!({"file_path": "/x/src/links.rs"}) });
                a.goal = Some("Link hint mode for terminal".into());
                a.branch = Some("feat/hints".into());
                a.context_tokens = Some(142_000);
                a.recap = Some("Labels render in reading order; next, dim non-link text.".into());
                a.prs.push((418, "https://github.com/erango/t1k/pull/418".into()));
                match state.as_str() {
                    "needs" => { a.apply_hook(Hook::Notification { message: "Claude needs your permission to use Bash".into() }); }
                    "error" => { a.apply_hook(Hook::Stop); }
                    "done" => { a.apply_hook(Hook::Stop); }
                    // A Codex session with no title or recap that just finished: the helper
                    // fills both (needs [helper] backend set).
                    "helper" => {
                        a.kind = t1k_agent::agents::AgentKind::Codex;
                        a.goal = None;
                        a.recap = None;
                        a.apply_hook(Hook::Stop);
                        this.turns_done.push(t.id.clone());
                    }
                    _ => {}
                }
                if state == "error" {
                    a.status = Status::Error { message: "API error 529 · overloaded".into() };
                }
                t.agent = Some(a);
            }
        }
        // Debug aid: T1K_DEBUG_VIEW=<path>[:line] opens a file in the side panel.
        if let Ok(spec) = std::env::var("T1K_DEBUG_VIEW") {
            let (p, line) = match spec.rsplit_once(':') {
                Some((p, l)) if l.parse::<u32>().is_ok() => (p.to_string(), l.parse().ok()),
                _ => (spec.clone(), None),
            };
            this.view_file(std::path::Path::new(&p), line, cx);
        }
        // Debug aid for screenshots without a keyboard: T1K_DEBUG_OVERLAY=palette|switcher|
        // overview|settings|menu|groupmenu|termmenu|card opens that surface at launch.
        if let Ok(which) = std::env::var("T1K_DEBUG_OVERLAY") {
            let at = gpui::point(px(120.), px(140.));
            let o = match which.as_str() {
                "palette" => Some(Overlay::Palette { query: String::new(), selected: 0 }),
                "about" => Some(Overlay::About),
                "welcome" => Some(Overlay::Welcome),
                "more" => Some(Overlay::MoreMenu { at: gpui::point(px(480.), px(44.)) }),
                "sidebarview" => Some(Overlay::SidebarView { at: gpui::point(px(90.), px(44.)) }),
                "switcher" => Some(Overlay::Switcher { query: String::new(), selected: 0 }),
                "overview" => Some(Overlay::Overview { selected: 0 }),
                "settings" => Some(Overlay::Settings { section: crate::settings::Section::Appearance }),
                "settings-interface" => Some(Overlay::Settings { section: crate::settings::Section::Interface }),
                "settings-advanced" => Some(Overlay::Settings { section: crate::settings::Section::Advanced }),
                "update" => {
                    crate::updater::check_now();
                    Some(Overlay::Update)
                }
                // The daily background check, now (download + Ready, no dialog).
                "update-background" => {
                    crate::updater::check_background();
                    None
                }
                "quit" => Some(Overlay::QuitUpdate),
                "themes" => {
                    let dark = this.theme.is_dark();
                    Some(Overlay::ThemePicker { query: String::new(), selected: 0, dark })
                }
                "remind" => Some(Overlay::RemindCustom { tab: this.active, text: "tomorrow 9:15".into() }),
                "settings-keys" => Some(Overlay::Settings { section: crate::settings::Section::Keybindings }),
                "settings-search" => {
                    this.settings_query = "font".into();
                    Some(Overlay::Settings { section: crate::settings::Section::Appearance })
                }
                "menu" => Some(Overlay::Menu { tab: this.active, at }),
                "groupmenu" => this.tabs.get(this.active).map(|t| Overlay::GroupMenu { key: t.group.clone(), at }),
                "termmenu" => Some(Overlay::TerminalMenu { tab: this.active, split: false, at: gpui::point(px(600.), px(200.)), link: None, has_selection: false, apps: Vec::new() }),
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
                    let o = Overlay::TerminalMenu { tab: ix, split: false, at: *at, link: link.clone(), has_selection: *has_selection, apps: crate::overlay::file_apps(link) };
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
            branch: None,
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
    pub fn spawn_view(&mut self, cwd: Option<PathBuf>, tab_id: String, window: &mut Window, cx: &mut Context<Self>) -> Entity<TerminalView> {
        let restore = self.restore_next.take();
        let mut env = HashMap::new();
        if let Some(sock) = &self.socket {
            env.insert("T1K_SOCKET".into(), sock.display().to_string());
        }
        if let Ok(exe) = std::env::current_exe() {
            env.insert("T1K_BIN".into(), exe.display().to_string());
        }
        if self.config.terminal.shell_integration {
            if let Some(dir) = crate::shell::zsh_dir() {
                // An empty ZDOTDIR means "unset" to us; restoring it as "" would make zsh look
                // for /.zshrc.
                if let Ok(orig) = std::env::var("ZDOTDIR").map(|z| z.trim().to_string()).and_then(|z| if z.is_empty() { Err(std::env::VarError::NotPresent) } else { Ok(z) }) {
                    env.insert("T1K_ORIG_ZDOTDIR".into(), orig);
                }
                env.insert("ZDOTDIR".into(), dir.display().to_string());
            }
        }
        let opts = SpawnOptions { working_directory: cwd, env, tab_id, start_at_bottom: self.config.terminal.prompt_gravity != "top", restore };
        let palette = self.theme.palette();
        let style = self.terminal_style();
        cx.new(|cx| {
            let mut v = TerminalView::new(opts, window, cx).expect("failed to spawn shell in pty");
            v.palette = palette;
            v.style = style;
            v
        })
    }

    fn repo_for(&mut self, group: &std::path::Path) -> Option<String> {
        self.repos.entry(group.to_path_buf()).or_insert_with(|| crate::links::github_repo(group)).clone()
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

    /// Open a link: text files go to the viewer panel (links.text_files = "panel"), everything
    /// else to the browser, editor or default app.
    pub fn open_link(&mut self, ix: usize, link: &t1k_terminal::links::Link, cx: &mut Context<Self>) {
        if let t1k_terminal::links::Link::File { path, line, .. } = link {
            if self.config.links.text_files != "editor" && self.view_file(path, *line, cx) {
                return;
            }
            if !crate::viewer::is_text(path) && !path.is_dir() {
                // Images, PDFs and other documents: the default app.
                cx.open_with_system(path);
                return;
            }
        }
        self.open_link_in_editor(ix, link, cx);
    }

    /// Open a link the classic way: browser, configured editor, PR or tracker.
    pub fn open_link_in_editor(&mut self, ix: usize, link: &t1k_terminal::links::Link, cx: &mut Context<Self>) {
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
                // No known folder yet: home, never a group of your own it was just taken out of.
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
                t.group = groups::project_root(t.cwd.as_deref().unwrap_or(&home));
            }
        }
        self.save();
        cx.notify();
    }

    /// Start a group of your own with tab `ix` in it, then ask for its name.
    pub fn new_group_with(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let key = groups::new_custom();
        self.group_names.insert(key.clone(), "New group".into());
        self.move_tab(ix, Some(key.clone()), cx);
        self.open_overlay(crate::overlay::Overlay::RenameGroup { key, text: String::new() }, window, cx);
    }

    /// A new tab in group `key`: in the folder for a folder group; for a group of your own,
    /// in the folder of the tab you were on, and kept in the group.
    pub fn new_tab_in_group(&mut self, key: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !groups::is_custom(&key) {
            let cwd = key.is_dir().then(|| key.clone());
            self.open_tab(cwd, None, window, cx);
            return;
        }
        let from = self.tabs.get(self.active).filter(|t| t.group == key).or_else(|| self.tabs.iter().find(|t| t.group == key));
        let cwd = from.and_then(|t| t.view.read(cx).terminal.cwd());
        self.open_tab(cwd, None, window, cx);
        self.move_tab(self.active, Some(key), cx);
    }

    /// Send every tab in a group of your own back to its folder's group; the group goes away.
    pub fn ungroup(&mut self, key: &PathBuf, cx: &mut Context<Self>) {
        let ixs: Vec<usize> = self.tabs.iter().enumerate().filter(|(_, t)| &t.group == key).map(|(i, _)| i).collect();
        for i in ixs {
            self.move_tab(i, None, cx);
        }
        self.group_names.remove(key);
        self.group_colors.remove(key);
        self.save();
    }

    /// Write every tab's scrollback to `scrollback/<tab id>.ansi` (at quit). Owner-only, since
    /// terminal output can hold secrets.
    fn save_scrollback(&mut self, cx: &mut Context<Self>) {
        self.scrollback_saved = false;
        if !self.config.terminal.restore_scrollback {
            return;
        }
        let Some(dir) = session::support_dir().map(|d| d.join("scrollback")) else { return };
        let _ = std::fs::remove_dir_all(&dir);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        for t in &self.tabs {
            let bytes = t.view.read(cx).terminal.snapshot(5_000);
            let path = dir.join(format!("{}.ansi", t.id));
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path) {
                let _ = f.write_all(&bytes);
            }
        }
        self.scrollback_saved = true;
    }

    /// Whether tab `ix` has outputs the user hasn't seen in the panel yet.
    pub fn outputs_new(&self, ix: usize) -> bool {
        self.tabs.get(ix).is_some_and(|t| self.outputs_seen.get(&t.id).is_some_and(|s| t.outputs_count() > *s))
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
            let branch = t.cwd.as_deref().and_then(groups::git_branch);
            if branch != t.branch {
                t.branch = branch;
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
        let hooks = crate::settings::claude_hooks_installed();
        if hooks != self.hooks_installed {
            self.hooks_installed = hooks;
            changed = true;
        }
        self.update_link_rules(cx);
        if changed {
            cx.notify();
        }
        if self.window_frame != self.saved_frame {
            self.saved_frame = self.window_frame;
            changed = true;
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
        // Outputs already there count as seen; new ones stay "new" until the panel shows them.
        for (i, t) in self.tabs.iter().enumerate() {
            let n = t.outputs_count();
            let seen = self.outputs_seen.entry(t.id.clone()).or_insert(n);
            if (self.panel.open && i == self.active) || n < *seen {
                *seen = n;
            }
        }
        let rev = crate::updater::rev();
        if rev != self.update_rev {
            self.update_rev = rev;
            // A check you started has an answer: show it, unless you're in the middle of
            // something else (the pill and ⋯ menu still carry it).
            if crate::updater::take_dialog_request() && matches!(self.overlay, Overlay::None | Overlay::Update) {
                self.open_overlay(Overlay::Update, window, cx);
            }
            // Debug aid: T1K_DEBUG_UPDATE=install presses Restart now once an update is ready.
            if std::env::var("T1K_DEBUG_UPDATE").as_deref() == Ok("install") && crate::updater::snapshot().phase == crate::updater::Phase::Ready {
                self.save();
                crate::updater::install_now();
            }
            cx.notify();
        }
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
                    crate::platform::notify_reminder(&t.id, "Reminder", &t.title());
                }
            }
        }
        for (id, mins) in crate::platform::take_snoozed() {
            if let Some(ix) = self.tabs.iter().position(|t| t.id == id) {
                self.set_reminder(ix, Some(now + Duration::from_secs(mins * 60)), cx);
                changed = true;
            }
        }
        while let Ok(env) = self.agent_rx.try_recv() {
            changed |= self.apply_envelope(env, window);
        }
        for id in std::mem::take(&mut self.turns_done) {
            if let Some(ix) = self.tabs.iter().position(|t| t.id == id) {
                self.helper_after_turn(ix, cx);
            }
        }
        if changed {
            self.debug_dump();
        }
        let needs = self.needs_queue().len();
        if self.badge != Some(needs) {
            self.badge = Some(needs);
            crate::platform::set_dock_badge(if self.config.notify.dock_badge { needs } else { 0 });
        }
        // The chosen icon, in red while anything needs you.
        let icon = (self.config.interface.app_icon.clone(), self.config.notify.dock_badge && needs > 0);
        if self.dock_icon.as_ref() != Some(&icon) {
            crate::platform::set_dock_icon(&icon.0, icon.1);
            self.dock_icon = Some(icon);
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
            let mut a = AgentState::new(sid);
            a.kind = env.payload.get("t1k_agent").and_then(|v| v.as_str()).and_then(t1k_agent::agents::AgentKind::from_id).unwrap_or_default();
            tab.agent = Some(a);
        }
        let tab = &mut self.tabs[ix];
        let before = tab.agent.as_ref().map(|a| a.status.clone());
        let finished = tab.agent.as_mut().unwrap().apply_hook(hook);
        if finished {
            self.turns_done.push(env.tab_id.clone());
        }
        let tab = &mut self.tabs[ix];
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
                    scrollback: self.scrollback_saved.then(|| format!("{}.ansi", t.id)),
                    agent_live: t.agent.as_ref().is_some_and(|a| {
                        a.kind == t1k_agent::agents::AgentKind::Claude && !a.session_id.is_empty() && !matches!(a.status, Status::Ended)
                    }) && t.program.is_some(),
                }
            }).collect(),
            active: self.visual_order().iter().position(|&i| i == self.active).unwrap_or(0),
            collapsed: self.collapsed.iter().cloned().collect(),
            group_names: self.group_names.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            sidebar_open: Some(self.sidebar_open),
            outputs_open: self.panel.open,
            recent: self.recent.clone(),
            window: self.window_frame,
            welcomed: true,
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
        let custom = self.tabs.get(self.active).map(|t| t.group.clone()).filter(|g| groups::is_custom(g));
        let cwd = self.tabs.get(self.active).and_then(|t| t.view.read(cx).terminal.cwd());
        self.open_tab(cwd, None, window, cx);
        if let Some(g) = custom {
            self.move_tab(self.active, Some(g), cx);
        }
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
        // ⌘\ while peeking pins the sidebar open.
        self.sidebar_open = !self.sidebar_open || self.peek;
        self.peek = false;
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
            // One number in config; the sides get proportionally more (design: 14 × 20).
            padding: px(t.padding.clamp(0., 64.)),
            padding_x: px((t.padding.clamp(0., 64.) * 10. / 7.).round()),
            option_as_meta: t.option_as_meta,
            copy_on_select: t.copy_on_select,
        }
    }

    /// Re-read themes and push config to the theme and every terminal.
    pub fn apply_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = config::themes_dir() {
            t1k_themes::load_user_themes(&d);
        }
        (self.mode, self.dark, self.light) = Self::slots(&self.config);
        crate::updater::apply(&self.config.updates);
        self.apply_theme(window, cx);
        let style = self.terminal_style();
        for tab in &self.tabs {
            tab.view.update(cx, |v, cx| {
                v.style = style.clone();
                cx.notify();
            });
        }
        crate::keymap::install(cx, &self.config.keys);
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

    /// The appearance mode the user chose (not the one a theme preview forces).
    pub fn chosen_mode(&self) -> AppearanceMode {
        self.theme_before.map(|b| b.0).unwrap_or(self.mode)
    }

    /// Set the appearance mode from the theme picker. During a preview it takes effect when
    /// the preview ends, so the theme under the cursor stays on screen.
    pub fn set_mode(&mut self, mode: AppearanceMode, window: &mut Window, cx: &mut Context<Self>) {
        match &mut self.theme_before {
            Some(b) => b.0 = mode,
            None => {
                self.mode = mode;
                self.apply_theme(window, cx);
            }
        }
        let v = match mode {
            AppearanceMode::System => "system",
            AppearanceMode::Dark => "dark",
            AppearanceMode::Light => "light",
        };
        config::set(&["appearance"], v);
        cx.notify();
    }

    /// The theme in the dark or light slot.
    pub fn theme_slot(&self, dark: bool) -> &'static Theme {
        if dark { self.dark } else { self.light }
    }

    /// Show `name` across the window without saving it (theme picker).
    pub fn preview_theme(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = t1k_themes::find(name) else { return };
        if self.theme_before.is_none() {
            self.theme_before = Some((self.mode, self.dark, self.light));
        }
        if t.is_dark() { self.dark = t } else { self.light = t }
        self.mode = if t.is_dark() { AppearanceMode::Dark } else { AppearanceMode::Light };
        self.apply_theme(window, cx);
    }

    /// The slot's theme before any preview (what "in use" means in the picker).
    pub fn saved_slot(&self, dark: bool) -> &'static Theme {
        match self.theme_before {
            Some((_, d, l)) => if dark { d } else { l },
            None => self.theme_slot(dark),
        }
    }

    /// Leave the preview: keep `pick` (saved like any theme choice) or go back to before.
    pub fn end_theme_preview(&mut self, pick: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((mode, dark, light)) = self.theme_before.take() else {
            if let Some(name) = pick {
                self.select_theme(&SelectTheme { name: name.to_string().into() }, window, cx);
            }
            return;
        };
        (self.mode, self.dark, self.light) = (mode, dark, light);
        match pick {
            Some(name) => self.select_theme(&SelectTheme { name: name.to_string().into() }, window, cx),
            None => self.apply_theme(window, cx),
        }
    }

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        crate::strip::render(self, cx)
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
            .on_action(cx.listener(|this, _: &ShowAbout, window, cx| this.open_overlay(Overlay::About, window, cx)))
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| this.check_for_updates(window, cx)))
            .on_action(cx.listener(|this, _: &ShowUpdate, window, cx| this.open_update(window, cx)))
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
            .on_action(cx.listener(|this, _: &ShowThemePicker, window, cx| this.open_theme_picker(window, cx)))
            .on_modifiers_changed(cx.listener(|this, ev: &gpui::ModifiersChangedEvent, _, cx| {
                if this.alt_held != ev.modifiers.alt {
                    this.alt_held = ev.modifiers.alt;
                    cx.notify();
                }
            }));
        let root = select!(root, SelectTab1 => 0, SelectTab2 => 1, SelectTab3 => 2, SelectTab4 => 3,
            SelectTab5 => 4, SelectTab6 => 5, SelectTab7 => 6, SelectTab8 => 7, SelectTab9 => 8);

        use crate::rail::Shown;
        let shown = self.sidebar_shown();
        let sidebar = match shown {
            Shown::Full(_) => Some(crate::sidebar::render(self, cx)),
            Shown::Rail => Some(crate::rail::render(self, cx)),
            Shown::Hidden => None,
        };
        // From the rail: the full sidebar over the terminal, not resizing it.
        let peek = (self.peek && shown == Shown::Rail).then(|| {
            div()
                .id("peek")
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(crate::rail::RAIL_W))
                .flex()
                .shadow_lg()
                .occlude()
                .on_hover(cx.listener(|this, over: &bool, _, cx| this.rail_hover(*over, cx)))
                .child(crate::sidebar::render(self, cx))
        });
        let readout = crate::rail::readout(self);
        let overlay = crate::overlay::render(self, window, cx);
        // The file viewer and the outputs panel share the right side; the viewer wins.
        let panel = match &self.viewer {
            Some(v) => Some(crate::viewer::render(self, v, cx)),
            None => self.panel.open.then(|| crate::outputs_panel::render(self, cx)),
        };
        let root = root
            .on_drag_move::<crate::rail::EdgeDrag>(cx.listener(|this, ev: &gpui::DragMoveEvent<crate::rail::EdgeDrag>, _, cx| {
                this.sidebar_drag_move(ev.event.position.x, cx)
            }))
            .capture_any_mouse_up(cx.listener(Self::sidebar_drag_end));
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
        .children(peek)
        .children(readout)
        .children(overlay)
    }
}

/// The window's frame and state, for saving.
fn frame_of(window: &Window) -> session::SavedWindow {
    use gpui::WindowBounds as B;
    let (b, state) = match window.window_bounds() {
        B::Windowed(b) => (b, session::WindowState::Windowed),
        B::Maximized(b) => (b, session::WindowState::Maximized),
        B::Fullscreen(b) => (b, session::WindowState::Fullscreen),
    };
    session::SavedWindow {
        x: f32::from(b.origin.x),
        y: f32::from(b.origin.y),
        w: f32::from(b.size.width),
        h: f32::from(b.size.height),
        state,
    }
}

/// Saved scrollback plus a quiet divider, so it's clear where the last run ended.
fn restored_banner(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.is_empty() {
        return bytes;
    }
    bytes.extend_from_slice(b"\x1b[0m\x1b[2m\xe2\x94\x80\xe2\x94\x80 restored from the last session \xe2\x94\x80\xe2\x94\x80\x1b[0m\r\n");
    bytes
}
