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
    /// When the tab's state last changed (a turn started or ended, it asked for you, a program
    /// started or quit). "Last updated" sorts by this; saved, so the order survives a restart.
    pub updated: SystemTime,
    /// The state `updated` last saw, to notice the next change.
    pub seen_state: Option<std::mem::Discriminant<Status>>,
    pub reminder_at: Option<SystemTime>,
    /// Reminder fired and not yet seen: loud, counts as needs-you.
    pub reminder_due: bool,
    /// Claude session id restored from the last run (for resume).
    pub last_session: Option<String>,
    /// Set when the user moved the tab to a group by hand; folder changes don't regroup it.
    pub pinned_group: Option<PathBuf>,
    /// An emoji, app icon or image chosen to stand in for the program icon.
    pub icon: Option<crate::custom_icon::Icon>,
    /// Brought over from another terminal: where from ("Warp ttys004"), so it isn't again.
    pub origin: Option<String>,
    /// A Claude session still open in the terminal this tab came from.
    pub waiting: Option<crate::import_ui::Waiting>,
    /// What the tab ran in the other terminal (not restarted; "Run again" types it).
    pub was_running: Option<String>,
    /// Helper panes beside or below the main one, and how they're laid out.
    pub panes: crate::split::Panes,
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
    /// Fixed group order; new groups join at the end.
    pub group_order: Vec<PathBuf>,
    pub group_icons: HashMap<PathBuf, crate::custom_icon::Icon>,
    pub icon_suggestions: HashMap<PathBuf, Vec<String>>,
    /// Tab icon suggestions, worked out when the picker first opens for a tab (not saved).
    pub tab_icon_suggestions: HashMap<String, Vec<String>>,
    /// The import sheet while it's open.
    pub import: Option<crate::import_ui::Sheet>,
    /// Tabs in other terminals not yet in T1K (launch scan): welcome card and palette hint.
    pub import_found: Option<Vec<crate::import::Found>>,
    /// Show the launch offer (new tabs turned up in other terminals).
    pub import_offer: bool,
    /// Tabs already offered ("Warp ttys004"), so the launch offer is only for new ones.
    pub import_seen: Vec<String>,
    /// Last import: (tabs, unix time).
    pub import_last: Option<(usize, i64)>,
    /// A short message at the bottom of the sidebar, and when it appeared.
    pub notice: Option<(String, Instant)>,
    /// The sidebar row under the pointer (tab id), for its × button.
    pub hover_row: Option<String>,
    /// The tab being dragged in the sidebar (cleared on drop; ignored once no drag is active).
    pub dragging_tab: Option<String>,
    /// When the last tab needing you was handled: "All caught up" shows for a few seconds,
    /// then fades out (it's noise when nothing is going on).
    pub caught_up_at: Option<Instant>,
    had_needs: bool,
    caught_up_task: Option<Task<()>>,
    /// The notice offers Undo (it's about a move).
    pub notice_undo: bool,
    /// Tab moves between groups, newest last: (tab id, group, pinned group) before the move.
    pub move_undo: Vec<Vec<(String, PathBuf, Option<PathBuf>)>>,
    /// "Match your terminal": the scan and the sheet's choices.
    pub matcher: Option<crate::match_ui::MatchState>,
    /// config.toml before the last match, for Undo.
    pub match_undo: Option<crate::match_ui::MatchUndo>,
    /// The welcome card's match section was used or dismissed.
    pub match_hidden: bool,
    /// "Set up T1K": progress (saved), snapshots for Undo, tried keys.
    pub setup: crate::setup::SetupState,
    /// The font picker and "Get more fonts" installs.
    pub fonts: crate::fonts_ui::FontPicker,
    /// Files being dragged over a pane or a sidebar row.
    pub drop_hover: Option<crate::drop::DropHover>,
    /// ⌘A in the emoji picker's search: the whole query is selected.
    pub icon_query_all: bool,
    /// ⌘J found nothing but offered an unread tab: (its id, when), for a second ⌘J.
    unread_offer: Option<(String, Instant)>,
    /// The notice is a passing remark (gone in 2 s, not 5).
    pub notice_brief: bool,
    /// The icon picker's tab, scroll and cropper.
    pub icon_ui: crate::group_icons::PickerUi,
    /// The window isn't key (its chrome dims), and the pointer is over the sidebar (it doesn't).
    window_inactive: bool,
    sidebar_hover: bool,
    /// Tabs coming back after a quit (canvas v3-ab).
    pub restore: Option<crate::restore::Restore>,
    /// Automatic group hue (ANSI 4-6), by name, bumped so neighbors differ.
    auto_hues: HashMap<PathBuf, u8>,
    /// The order the hues were last worked out for; they change only when it does.
    hues_for: Vec<PathBuf>,
    /// When a listed group lost its last tab; it keeps its slot for a while in case it returns.
    group_emptied: HashMap<PathBuf, Instant>,
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
    /// The theme's colors with the window active; `ui` is these, dimmed while it's inactive.
    ui_base: UiColors,
    /// Fade toward the inactive look: (from, to, started, seconds).
    dim: (f32, f32, Instant, f32),
    dim_anim: Option<Task<()>>,
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
    /// The selected sidebar row's bounds, set while it paints (for the selected-tab outline).
    /// The window background last asked of macOS, so it's set only when it changes.
    window_look: Option<gpui::WindowBackgroundAppearance>,
    pub selected_row: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// Where the active tab's panes were drawn (⌘⌥ arrows).
    pub pane_bounds: crate::split::PaneBounds,
    pub outputs_scroll: gpui::ScrollHandle,
    pub outputs_bar: crate::scroll_bar::Drag,
    /// While dragging a tab or group near the list's top or bottom edge: px per frame
    /// (negative scrolls up), and the task scrolling it.
    drag_scroll: f32,
    drag_scroll_task: Option<Task<()>>,
    /// The switcher's filter chip (back to All when it closes).
    pub switcher_filter: crate::overlay::SwitcherFilter,
    /// While the theme picker previews: what to go back to on esc.
    theme_before: Option<(AppearanceMode, &'static Theme, &'static Theme)>,
    /// The Settings page a drill-down (the theme picker) was opened from; Esc and ‹ return there.
    pub settings_return: Option<crate::settings::Section>,
    /// Outputs count per tab id when the user last saw them (panel open on that tab).
    pub outputs_seen: HashMap<String, usize>,
    /// Set once Quit anyway was pressed: programs are being closed.
    pub quitting: Option<Instant>,
    /// Rename fields: selection, where the text sits (for clicks) and whether a drag is on.
    pub edit_sel: crate::input::Sel,
    pub edit_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub edit_dragging: bool,
    /// Claude sessions running when the quit began (they exit, but resume next launch).
    pub resume_after_quit: HashSet<String>,
    /// Programs asked to stop from the quit dialog, and when.
    pub stopping: HashMap<String, Instant>,
    /// Keeps the quit dialog's list live while it's open.
    pub quit_ticker: Option<gpui::Task<()>>,
    /// Closed tabs, newest last, for Undo and ⌘⇧T.
    pub closed: Vec<crate::close::ClosedTab>,
    pub close_toast: Option<(String, Instant)>,
    pub toast_task: Option<gpui::Task<()>>,
    /// A tab whose × asked "Stop <program>?" in its row.
    pub confirm_close: Option<String>,
    /// A helper pane asked to close while something runs in it: (tab id, pane), confirming
    /// in its header.
    pub confirm_pane: Option<(String, crate::split::PaneId)>,
    /// Hook ids of helper panes that became their tab's main pane: their processes still
    /// report as "<tab>-p<n>", so they map to the tab.
    pub hook_alias: HashMap<String, String>,
    /// The group menu's Close group is asking to stop running programs first.
    pub confirm_close_group: Option<PathBuf>,
    pub close_batch: u64,
    /// (closing tab, tab to land on): chosen when a gentle close starts, because the session
    /// ending on the way re-sorts the closing tab to the bottom of its group.
    pub land_after_close: Option<(String, String)>,
    /// Polls in a row a Needs-you tab showed no question on screen.
    needs_gone: HashMap<String, u8>,
    /// Since when a Working Claude tab has shown no "esc to interrupt" on screen.
    working_gone: HashMap<String, Instant>,
    /// The group header under the pointer and since when (its icon placeholder slides in).
    pub hovered_group: Option<(PathBuf, Instant)>,
    hover_anim: Option<gpui::Task<()>>,
    /// Scrollback files were written for every tab (at quit); the session points at them.
    scrollback_saved: bool,
    /// Scrollback to replay into the next terminal `open_tab` spawns (session restore).
    pub restore_next: Option<Vec<u8>>,
    /// The last shortcut change, shown under its row ("Was on Find tab").
    pub key_note: Option<(&'static str, String)>,
    _key_interceptor: gpui::Subscription,
    pub back: Vec<String>,
    pub forward: Vec<String>,
    navigating: bool,
    pub panel: crate::outputs_panel::PanelState,
    /// Context details: transcript reads and which sections are open (canvas v3-au).
    pub ctx: crate::context_ui::CtxState,
    /// `T1K_DEBUG_CONTEXT`: a transcript shown as the active tab's context.
    pub ctx_debug: Option<PathBuf>,
    pub overlay_focus: FocusHandle,
    pub config: Config,
    watcher: config::Watcher,
    socket: Option<PathBuf>,
    agent_rx: Receiver<Envelope>,
    /// Each agent's plan usage limits (Claude's 5-hour and weekly, Antigravity's buckets), from
    /// the status-line wrapper. Per account, so one per agent, not per tab.
    pub usage: std::collections::HashMap<t1k_agent::agents::AgentKind, t1k_agent::statusline::Usage>,
    usage_minute: i64,
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

/// How long "All caught up" stays before fading out (the fade takes `CAUGHT_UP_FADE`).
pub const CAUGHT_UP_SHOW: Duration = Duration::from_millis(3500);
pub const CAUGHT_UP_FADE: f32 = 0.25;

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Demo mode reads nothing of the user's: no session, config or other terminals.
        let demo = crate::demo::on();
        let saved = if demo { Session::default() } else { session::load() };
        if let Some(d) = config::themes_dir() {
            t1k_themes::load_user_themes(&d);
        }
        let mut watcher = config::Watcher::new();
        let config = match if demo { None } else { watcher.poll() } {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                log::error!("{e}");
                Config::default()
            }
            None => Config::default(),
        };
        let mut config = config;
        if demo {
            // Mercury Dark unless T1K_APPEARANCE / T1K_THEME say otherwise.
            config.appearance = "dark".into();
        }
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
        let socket = session::support_dir().filter(|_| !demo).map(|d| {
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
            group_order: saved.group_order.clone(),
            group_icons: saved.group_icons.iter().cloned().collect(),
            icon_suggestions: saved.icon_suggestions.iter().cloned().collect(),
            tab_icon_suggestions: HashMap::new(),
            import: None,
            import_found: None,
            import_offer: false,
            import_seen: saved.import_seen.clone(),
            import_last: saved.import_last,
            notice: None,
            notice_undo: false,
            hover_row: None,
            dragging_tab: None,
            caught_up_at: None,
            had_needs: false,
            caught_up_task: None,
            move_undo: Vec::new(),
            matcher: None,
            match_undo: None,
            match_hidden: saved.match_offered,
            setup: crate::setup::SetupState::new(saved.setup_step),
            fonts: Default::default(),
            drop_hover: None,
            icon_query_all: false,
            unread_offer: None,
            notice_brief: false,
            icon_ui: Default::default(),
            window_inactive: false,
            sidebar_hover: false,
            restore: None,
            auto_hues: HashMap::new(),
            hues_for: Vec::new(),
            group_emptied: HashMap::new(),
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
            ui_base: dark.ui(),
            dim: (0., 0., Instant::now(), 0.),
            dim_anim: None,
            overlay: Overlay::None,
            hooks_installed: crate::settings::claude_hooks_installed(),
            settings_query: String::new(),
            key_recording: None,
            helper: Default::default(),
            turns_done: Vec::new(),
            sidebar_scroll: gpui::ScrollHandle::new(),
            selected_row: Default::default(),
            pane_bounds: Default::default(),
            window_look: None,
            outputs_scroll: gpui::ScrollHandle::new(),
            outputs_bar: Default::default(),
            drag_scroll: 0.,
            drag_scroll_task: None,
            switcher_filter: Default::default(),
            theme_before: None,
            settings_return: None,
            outputs_seen: HashMap::new(),
            quitting: None,
            edit_sel: Default::default(),
            edit_bounds: Default::default(),
            edit_dragging: false,
            resume_after_quit: HashSet::new(),
            stopping: HashMap::new(),
            quit_ticker: None,
            closed: Vec::new(),
            close_toast: None,
            toast_task: None,
            confirm_close: None,
            confirm_pane: None,
            hook_alias: HashMap::new(),
            land_after_close: None,
            confirm_close_group: None,
            close_batch: 0,
            needs_gone: HashMap::new(),
            working_gone: HashMap::new(),
            hovered_group: None,
            hover_anim: None,
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
            ctx: crate::context_ui::CtxState {
                toggled: std::env::var("T1K_DEBUG_CONTEXT_OPEN").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_default(),
                ..Default::default()
            },
            ctx_debug: std::env::var("T1K_DEBUG_CONTEXT").ok().map(|v| PathBuf::from(v.split(':').next().unwrap_or_default())),
            overlay_focus: cx.focus_handle(),
            config,
            watcher,
            socket,
            agent_rx,
            usage_minute: 0,
            usage: saved.usage.iter().filter_map(|(k, u)| Some((t1k_agent::agents::AgentKind::from_id(k)?, u.clone()))).collect(),
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
        // Tabs open in other terminals: the welcome card, the palette hint, the launch offer.
        if !demo {
            this.scan_for_import(cx);
            this.scan_for_match(cx);
        }
        cx.observe_window_activation(window, |this, window, cx| this.set_inactive(!window.is_window_active(), cx)).detach();
        if !window.is_window_active() && std::env::var_os("T1K_DEBUG_BACKGROUND").is_some() {
            this.set_inactive(true, cx);
        }
        if !demo {
            crate::updater::start(&this.config.updates);
        }
        crate::secure_input::apply(this.config.terminal.secure_input, window.is_window_active());
        // Remember where the window is; saved with the session (debounced through `poll`).
        cx.observe_window_bounds(window, |this, window, _| this.window_frame = Some(frame_of(window))).detach();
        // The window's close button quits T1K (one window), so it asks the same question.
        let handle = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            handle
                .update(cx, |ws, cx| {
                    if ws.quitting.is_some() || ws.running_programs(cx).is_empty() {
                        return true;
                    }
                    ws.open_overlay(crate::overlay::Overlay::QuitRunning { expanded: false, selected: 0 }, window, cx);
                    false
                })
                .unwrap_or(true)
        });
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
        // A tab whose folder is gone still comes back (in the home folder), marked, rather than
        // silently disappearing.
        let restore: Vec<SavedTab> = saved.tabs;
        let mut resuming = std::collections::HashSet::new();
        let mut failed = Vec::new();
        if demo {
            crate::demo::populate(&mut this, window, cx);
        } else if restore.is_empty() || std::env::var_os("T1K_FRESH").is_some() {
            this.open_tab(None, None, window, cx);
        } else {
            let dir = session::support_dir().map(|d| d.join("scrollback"));
            for t in restore {
                if this.config.terminal.restore_scrollback {
                    this.restore_next = t.scrollback.as_ref().zip(dir.as_ref()).and_then(|(f, d)| std::fs::read(d.join(f)).ok()).map(restored_banner);
                }
                let missing = t.cwd.as_ref().is_some_and(|p| !p.is_dir());
                this.open_tab(t.cwd.filter(|p| p.is_dir()), t.manual_title, window, cx);
                let tab = this.tabs.last_mut().unwrap();
                if missing {
                    failed.push((tab.id.clone(), "Folder missing"));
                }
                tab.reminder_at = t.reminder_at.map(|s| UNIX_EPOCH + Duration::from_secs(s));
                tab.unread = t.unread;
                if let Some(u) = t.updated {
                    tab.updated = UNIX_EPOCH + Duration::from_secs(u);
                }
                // Re-attach the last Claude session's transcript: title, recap and outputs return.
                if let Some(sid) = &t.last_session {
                    if let Some(path) = session::find_transcript(sid) {
                        // Read it now, so titles, recaps and outputs are there in the first frame
                        // rather than a poll later (relaunching shouldn't look like loss).
                        let mut a = AgentState::restored(sid.clone(), path);
                        a.poll_transcript();
                        tab.agent = Some(a);
                    }
                }
                // Claude was mid-session when T1K quit: pick it back up.
                if t.agent_live && this.config.terminal.resume_agents && !missing {
                    if let Some(sid) = &t.last_session {
                        tab.view.read(cx).terminal.write(format!("claude --resume {sid}\r").into_bytes());
                        resuming.insert(tab.id.clone());
                    }
                }
                tab.last_session = t.last_session;
                tab.icon = t.icon;
                tab.origin = t.origin;
                if let Some(g) = t.pinned_group {
                    tab.group = g.clone();
                    tab.pinned_group = Some(g);
                }
                let ix = this.tabs.len() - 1;
                if let Some(p) = t.panes {
                    this.restore_panes(ix, p, window, cx);
                } else if let Some((cwd, down)) = t.split {
                    // Sessions from before panes: one helper beside or below.
                    let layout = crate::split::Layout::Split { down, ratio: 0.5, a: Box::new(crate::split::Layout::Leaf(0)), b: Box::new(crate::split::Layout::Leaf(1)) };
                    this.restore_panes(ix, crate::split::SavedPanes { layout, cwds: vec![(1, cwd)] }, window, cx);
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
            this.start_restore(std::mem::take(&mut resuming), std::mem::take(&mut failed), cx);
        }
        if first_launch && !demo {
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
        // Debug aid: T1K_DEBUG_USAGE=low|high5|high7|limit|stale|multi fakes plan usage limits.
        if let Ok(which) = std::env::var("T1K_DEBUG_USAGE") {
            use t1k_agent::{agents::AgentKind, statusline::{Usage, Window}};
            let t = crate::usage::now();
            let w = |name: &str, used: f32, mins: i64| Window { name: name.into(), used, resets_at: Some(t + mins * 60) };
            let (five, seven) = match which.as_str() {
                "high5" => (86., 41.),
                "high7" => (35., 91.),
                "limit" => (100., 72.),
                _ => (42., 18.),
            };
            let at = if which == "stale" { t - 25 * 60 } else { t - 60 };
            this.usage.insert(AgentKind::Claude, Usage { windows: vec![w("5h", five, 125), w("7d", seven, 3 * 1440 + 300)], at });
            if which == "multi" {
                this.usage.insert(AgentKind::Codex, Usage { windows: vec![w("5h", 12., 200), w("7d", 64., 5000)], at });
                this.usage.insert(AgentKind::Antigravity, Usage { windows: vec![w("gemini-weekly", 83., 9000), w("3p-weekly", 0., 10000)], at });
            }
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
        // Debug aid: T1K_DEBUG_PRS=1|open|done puts PRs in every state on the active tab, with
        // made-up gh answers, and opens the outputs panel (gh can't be reached reliably).
        if let Ok(which) = std::env::var("T1K_DEBUG_PRS") {
            if let Some(t) = this.tabs.get_mut(this.active) {
                this.panel.fake = true;
                this.panel.open = true;
                let a = t.agent.get_or_insert_with(|| AgentState::new("debug".into()));
                let now = std::time::SystemTime::now();
                let mins = |m: u64| now - Duration::from_secs(m * 60);
                let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
                // (number, state, draft, title, checks, decision, approvers, changes, ended, minutes ago)
                type Fake<'a> = (u64, &'a str, bool, &'a str, (u32, u32, u32), &'a str, Vec<String>, Vec<String>, Option<u64>, u64);
                let mut open: Vec<Fake> = vec![
                    (2504, "OPEN", false, "Score coding outcomes per turn", (6, 0, 0), "APPROVED", s(&["ana"]), vec![], None, 4),
                    (2511, "OPEN", false, "Fix stale hint labels after a resize in split panes", (3, 0, 2), "CHANGES_REQUESTED", vec![], s(&["ana"]), None, 25),
                    (2498, "OPEN", false, "Retry gh lookups that time out", (5, 1, 0), "REVIEW_REQUIRED", vec![], vec![], None, 90),
                    (2490, "OPEN", false, "Two approvals, no required review", (2, 0, 0), "", s(&["ana", "bo"]), vec![], None, 150),
                    (2487, "OPEN", true, "Draft: sidebar density setting", (0, 0, 0), "", vec![], vec![], None, 200),
                ];
                // The newest open PR is the title bar's: make it the one with the most to say.
                open.rotate_left(2);
                let done: Vec<Fake> = vec![
                    (2470, "MERGED", false, "Dedupe date utils", (8, 0, 0), "APPROVED", s(&["bo"]), vec![], Some(180), 300),
                    (2462, "CLOSED", false, "Try a second socket per tab", (1, 0, 0), "", vec![], vec![], Some(2 * 1440), 3000),
                    (2455, "MERGED", false, "Merge time unknown", (0, 0, 0), "", vec![], vec![], None, 4000),
                ];
                let fakes = match which.as_str() {
                    "open" => open,
                    "done" => done,
                    _ => open.into_iter().chain(done).collect(),
                };
                for (n, state, draft, title, checks, decision, approvers, changes, ended, ago) in fakes {
                    let url = format!("https://github.com/erango/t1k/pull/{n}");
                    a.prs.push((n, url.clone()));
                    a.outputs.push(t1k_agent::outputs::Output { kind: t1k_agent::outputs::Kind::Pr, target: url.clone(), label: format!("t1k#{n}"), at: mins(ago), icon: None, detail: None });
                    let info = crate::outputs_panel::PrInfo {
                        state: state.into(),
                        draft,
                        title: title.into(),
                        checks,
                        decision: decision.into(),
                        approvers,
                        changes,
                        ended: ended.map(mins),
                        fetched: Instant::now(),
                    };
                    this.panel.prs.insert(url, Some(info));
                }
                // One gh hasn't answered for yet: today's look.
                if which != "done" {
                    let url = "https://github.com/erango/t1k/pull/2520".to_string();
                    a.prs.insert(0, (2520, url.clone()));
                    a.outputs.push(t1k_agent::outputs::Output { kind: t1k_agent::outputs::Kind::Pr, target: url, label: "t1k#2520".into(), at: mins(1), icon: None, detail: None });
                }
            }
        }
        // Debug aid: T1K_DEBUG_PANES=right,down,zoom,menu splits the active tab at launch (screenshots).
        if let Ok(spec) = std::env::var("T1K_DEBUG_PANES") {
            for step in spec.split(',') {
                match step {
                    "right" => this.open_split(this.active, false, window, cx),
                    "down" => this.open_split(this.active, true, window, cx),
                    "zoom" => this.toggle_zoom(window, cx),
                    "menu" => this.overlay = Overlay::PaneMenu { tab: this.active, pane: 1, at: gpui::point(px(700.), px(130.)) },
                    "confirm" => this.confirm_pane = this.tabs.get(this.active).map(|t| (t.id.clone(), 1)),
                    _ => {}
                }
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
        // T1K_DEBUG_DROP=pane|row|group|theme:<path>: files "dragged" over the pane or the active row,
        // or the theme-file choice, for screenshots (a real Finder drag can't be scripted).
        if let Some((kind, path)) = std::env::var("T1K_DEBUG_DROP").ok().as_deref().and_then(|v| v.split_once(':')) {
            let paths = vec![std::path::PathBuf::from(path)];
            match kind {
                "pane" => self_drop_hover(&mut this, crate::drop::DropTarget::Pane { pane: 0 }, paths),
                "row" => {
                    let id = this.tabs.get(this.active).map(|t| t.id.clone()).unwrap_or_default();
                    self_drop_hover(&mut this, crate::drop::DropTarget::Row(id), paths)
                }
                "group" => {
                    let g = this.tabs.get(this.active).map(|t| t.group.clone()).unwrap_or_default();
                    self_drop_hover(&mut this, crate::drop::DropTarget::Group(g), paths)
                }
                "theme" => this.overlay = Overlay::DropTheme { path: paths[0].clone(), pane: 0, at: gpui::point(px(700.), px(400.)) },
                _ => {}
            }
        }
        // T1K_DEBUG_CONTEXT=<transcript>:pane opens the panel on the Context tab.
        if std::env::var("T1K_DEBUG_CONTEXT").is_ok_and(|v| v.ends_with(":pane")) {
            this.panel.open = true;
            this.panel.tab = crate::context_ui::PanelTab::Context;
        }
        // Debug aid for screenshots without a keyboard: T1K_DEBUG_OVERLAY=palette|switcher|
        // overview|settings|menu|groupmenu|termmenu|card opens that surface at launch.
        if let Ok(which) = std::env::var("T1K_DEBUG_OVERLAY") {
            let at = gpui::point(px(120.), px(140.));
            let o = match which.as_str() {
                "palette" => Some(Overlay::Palette { query: std::env::var("T1K_DEBUG_QUERY").unwrap_or_default(), selected: 0 }),
                "about" => Some(Overlay::About),
                "welcome" => Some(Overlay::Welcome),
                "more" => Some(Overlay::MoreMenu { at: gpui::point(px(480.), px(44.)) }),
                "sidebarview" => Some(Overlay::SidebarView { at: gpui::point(px(90.), px(44.)) }),
                "usage" => Some(Overlay::Usage { at: gpui::point(px(60.), px(40.)) }),
                "context" => Some(Overlay::Context { at: gpui::point(px(760.), px(40.)) }),
                "switcher" => Some(Overlay::Switcher { query: std::env::var("T1K_DEBUG_QUERY").unwrap_or_default(), selected: 0 }),
                "overview" => Some(Overlay::Overview { selected: 0 }),
                "settings" => Some(Overlay::Settings { section: crate::settings::Section::Appearance }),
                "settings-interface" => Some(Overlay::Settings { section: crate::settings::Section::Interface }),
                "settings-terminal" => Some(Overlay::Settings { section: crate::settings::Section::Terminal }),
                "settings-fonts" => {
                    this.fonts.open = true;
                    Some(Overlay::Settings { section: crate::settings::Section::Terminal })
                }
                "settings-advanced" => Some(Overlay::Settings { section: crate::settings::Section::Advanced }),
                "settings-tabs" => Some(Overlay::Settings { section: crate::settings::Section::Tabs }),
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
                "quit-running" => Some(Overlay::QuitRunning { expanded: false, selected: 0 }),
                "quit-list" => Some(Overlay::QuitRunning { expanded: true, selected: 0 }),
                "rename" => {
                    let text = "Fix the login redirect".to_string();
                    this.edit_sel = crate::input::Sel { anchor: 8, cursor: 13 };
                    Some(Overlay::Rename { tab: this.active, text })
                }
                "themes" => {
                    // The theme in use is the one selected (and previewed), as when opened by hand.
                    let dark = this.theme.is_dark();
                    let current = this.theme_slot(dark).name.clone();
                    let selected = crate::theme_picker::results("", dark).iter().position(|t| t.name == current).unwrap_or(0);
                    Some(Overlay::ThemePicker { query: String::new(), selected, dark })
                }
                "remind" => Some(Overlay::RemindCustom { tab: this.active, text: "tomorrow 9:15".into() }),
                "icons" | "icons-search" | "tabicons" | "icons-apps" | "icons-image" => this.debug_icon_picker(&which, cx),
                "settings-keys" => Some(Overlay::Settings { section: crate::settings::Section::Keybindings }),
                "settings-search" => {
                    this.settings_query = "font".into();
                    Some(Overlay::Settings { section: crate::settings::Section::Appearance })
                }
                "menu" => Some(Overlay::Menu { tab: this.active, at }),
                "groupmenu" => this.tabs.get(this.active).map(|t| Overlay::GroupMenu { key: t.group.clone(), at }),
                "termmenu" => Some(Overlay::TerminalMenu { tab: this.active, pane: 0, at: gpui::point(px(600.), px(200.)), link: None, has_selection: false, apps: Vec::new() }),
                "card" => {
                    this.card = this.tabs.get(this.active).map(|t| t.id.clone());
                    None
                }
                "import" => {
                    this.open_import(window, cx);
                    None
                }
                "match" => {
                    this.open_match(window, cx);
                    None
                }
                "bug" | "bug-open" | "bug-long" => {
                    crate::bug_report::debug_open(&mut this, &which, window, cx);
                    None
                }
                s if s == "setup" || s.starts_with("setup-") => crate::setup::debug_overlay(&mut this, s, cx),
                "waiting" => {
                    // A Claude session "still open in iTerm2" (this process stands in for it).
                    let ix = this.active;
                    this.tabs[ix].waiting = Some(crate::import_ui::Waiting { pid: std::process::id() as i32, session_id: "3f2c9a1e".into(), source: crate::import::Source::ITerm2, tty: "ttys009".into(), busy: false });
                    None
                }
                "card-strip" => {
                    this.card = this.tabs.get(this.active).map(|t| format!("strip:{}", t.id));
                    None
                }
                _ => None,
            };
            if let Some(o) = o {
                this.open_overlay(o, window, cx);
            }
        }
        this.debug_fonts(window, cx);
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
        self.push_tab(id, view, cwd, manual_title, window, cx);
    }

    /// The main pane's events, wherever its tab is.
    fn subscribe_main(&mut self, view: &Entity<TerminalView>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_in(view, window, |this: &mut Self, view, ev: &TerminalEvent, window, cx| {
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
                TerminalEvent::Notify(n) => this.program_notice(ix, n, true, window, cx),
                TerminalEvent::OpenLink(link) => this.open_link(ix, link, cx),
                TerminalEvent::ContextMenu { at, link, has_selection } => {
                    let o = Overlay::TerminalMenu { tab: ix, pane: 0, at: *at, link: link.clone(), has_selection: *has_selection, apps: crate::overlay::file_apps(link) };
                    this.open_overlay(o, window, cx);
                }
            }
        })
    }

    /// A pane becomes its tab's main one (the main pane closed with others open).
    pub fn promote_view(&mut self, ix: usize, view: Entity<TerminalView>, window: &mut Window, cx: &mut Context<Self>) {
        let sub = self.subscribe_main(&view, window, cx);
        let t = &mut self.tabs[ix];
        t.view = view;
        t._sub = sub;
        t.agent = None;
        t.program = None;
        t.title_state = None;
    }

    /// A running terminal (a pane moved out of its tab) becomes a tab of its own after the active one.
    pub fn adopt_view(&mut self, id: String, view: Entity<TerminalView>, cwd: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.active;
        self.push_tab(id, view, cwd, None, window, cx);
        self.place_new_tab(self.active, Some(anchor));
    }

    fn push_tab(&mut self, id: String, view: Entity<TerminalView>, cwd: Option<PathBuf>, manual_title: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let sub = self.subscribe_main(&view, window, cx);
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
            updated: SystemTime::now(),
            seen_state: None,
            reminder_at: None,
            reminder_due: false,
            last_session: None,
            pinned_group: None,
            icon: None,
            origin: None,
            waiting: None,
            was_running: None,
            panes: Default::default(),
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
        // Demo terminals run no shell, so they need no zsh integration files.
        if self.config.terminal.shell_integration && !crate::demo::on() {
            if let Some(dir) = crate::shell::zsh_dir() {
                // An empty ZDOTDIR means "unset" to us; restoring it as "" would make zsh look
                // for /.zshrc.
                if let Ok(orig) = std::env::var("ZDOTDIR").map(|z| z.trim().to_string()).and_then(|z| if z.is_empty() { Err(std::env::VarError::NotPresent) } else { Ok(z) }) {
                    env.insert("T1K_ORIG_ZDOTDIR".into(), orig);
                }
                env.insert("ZDOTDIR".into(), dir.display().to_string());
            }
        }
        let command = crate::demo::on().then(crate::demo::command);
        let opts = SpawnOptions { working_directory: cwd, env, tab_id, start_at_bottom: self.config.terminal.prompt_gravity != "top", restore, command, kitty_keyboard: self.config.terminal.kitty_keyboard };
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
            let helper_focused = t.panes.helpers.iter().any(|p| p.view.read(cx).focus_handle(cx).is_focused(window));
            if !h.is_focused(window) && !helper_focused {
                window.focus(&h);
            }
        }
    }

    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // A tab that couldn't come back after a quit has had its look once you move on.
        if ix != self.active {
            if let Some(id) = self.tabs.get(self.active).map(|t| t.id.clone()) {
                if self.restore_failure(&id).is_some() {
                    self.restore_handled(&id);
                }
            }
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
        self.reveal_in_sidebar(ix);
        cx.notify();
    }

    /// Arriving at a tab whose group is scrolled out of the sidebar (⌘P, back/forward, ⌘J):
    /// bring the group into view. A group that's partly visible is left alone, so clicking a row
    /// never jumps the list.
    fn reveal_in_sidebar(&self, ix: usize) {
        let Some(gi) = self.grouped().iter().position(|(g, _)| *g == self.tabs[ix].group) else { return };
        let sh = &self.sidebar_scroll;
        let (Some(b), view) = (sh.bounds_for_item(gi), sh.bounds()) else { return };
        let off = sh.offset().y;
        let (top, bottom) = (b.top() + off, b.bottom() + off);
        if bottom <= view.top() || top >= view.bottom() {
            sh.scroll_to_item(gi);
        }
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
        // Closing a tab you're not on keeps you where you are.
        let landing = self.land_after_close.take().filter(|(c, _)| *c == self.tabs[ix].id).and_then(|(_, t)| self.tabs.iter().position(|x| x.id == t));
        let next = if ix == self.active { landing.or_else(|| self.after_close_target(ix)) } else { Some(self.active) };
        self.remember_session(ix);
        let id = self.tabs[ix].id.clone();
        self.restore_handled(&id);
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

    /// Where to land after closing tab `ix` ([tabs] after_close, close_stays_in_group): the
    /// neighbor in sidebar order, in the chosen direction first; with "stay in the group", the
    /// nearest tab left in its group, if any.
    pub(crate) fn after_close_target(&self, ix: usize) -> Option<usize> {
        let order = self.visual_order();
        let pos = order.iter().position(|&i| i == ix)?;
        let above_first = self.config.tabs.after_close == "above";
        // Every other tab, nearest first, the preferred direction winning ties.
        let mut near: Vec<(usize, usize)> = order
            .iter()
            .enumerate()
            .filter(|&(p, _)| p != pos)
            .map(|(p, &i)| {
                let dist = p.abs_diff(pos) * 2;
                let away = (p < pos) != above_first; // the other direction ranks second at equal distance
                (dist + away as usize, i)
            })
            .collect();
        near.sort();
        let group = &self.tabs[ix].group;
        if self.config.tabs.close_stays_in_group {
            if let Some(&(_, i)) = near.iter().find(|(_, i)| &self.tabs[*i].group == group) {
                return Some(i);
            }
        }
        near.first().map(|&(_, i)| i)
    }

    /// Put the tab just opened at `new` where [tabs] new_tab says, next to `anchor` (the tab you
    /// were on) when both are in the same group. Groups order tabs by `created`.
    pub fn place_new_tab(&mut self, new: usize, anchor: Option<usize>) {
        let Some(a) = anchor.filter(|&a| a != new && a < self.tabs.len() && new < self.tabs.len()) else { return };
        if self.tabs[a].group != self.tabs[new].group {
            return;
        }
        let c = self.tabs[a].created;
        let (shift_from, at) = match self.config.tabs.new_tab.as_str() {
            "after" => (c + 1, c + 1),
            "before" => (c, c),
            _ => return,
        };
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if i != new && t.created >= shift_from {
                t.created += 1;
            }
        }
        self.tabs[new].created = at;
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
        // Groups keep their place: the saved order, then any new ones in the order they appeared.
        out.sort_by_key(|(g, _)| self.group_order.iter().position(|o| o == g).unwrap_or(usize::MAX));
        // Within a group: by the group's sort mode; attention by default (needs-you first, then
        // unread, working, the rest).
        // While tabs come back after a quit, keep the order they had (they were saved in it):
        // resuming sessions pass through working and idle on the way, and sorting by attention
        // then shuffled the rows until they settled.
        let settling = self.restore_settling();
        for (g, v) in &mut out {
            match self.group_sort.get(g).copied().unwrap_or_default() {
                session::GroupSort::Attention if settling => v.sort_by_key(|&i| self.tabs[i].created),
                session::GroupSort::Attention => v.sort_by_key(|&i| (self.tabs[i].urgency(), self.tabs[i].created)),
                session::GroupSort::Updated => v.sort_by_key(|&i| (std::cmp::Reverse(self.tabs[i].updated), self.tabs[i].created)),
                session::GroupSort::Activity => v.sort_by_key(|&i| std::cmp::Reverse(self.tabs[i].last_active)),
                session::GroupSort::Opened => v.sort_by_key(|&i| self.tabs[i].created),
            }
        }
        out
    }

    /// Stamps `updated` on tabs whose state changed since the last look. While restored tabs
    /// settle (resuming passes through working and idle) it only records the state, so the
    /// saved times keep the order they had.
    fn stamp_updates(&mut self) -> bool {
        let settling = self.restore_settling();
        let mut changed = false;
        for t in &mut self.tabs {
            let now = t.status().map(std::mem::discriminant);
            if now == t.seen_state {
                continue;
            }
            if t.seen_state.is_some() && !settling {
                t.updated = SystemTime::now();
                changed = true;
            }
            t.seen_state = now;
        }
        changed
    }

    /// Tab indices in the order the sidebar shows them (collapsed groups included).
    pub fn visual_order(&self) -> Vec<usize> {
        self.grouped().into_iter().flat_map(|(_, v)| v).collect()
    }

    /// The group's color, if the user picked one: one of the theme's six ANSI hues.
    pub fn group_color(&self, key: &PathBuf) -> Option<gpui::Hsla> {
        let picked = self.group_colors.get(key).copied();
        let auto = (self.config.sidebar.group_colors != "manual").then(|| self.auto_hues.get(key).copied()).flatten();
        let i = picked.or(auto)? as usize;
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

    /// Take a tab out of its group, back into its folder's project group (made if there isn't
    /// one yet), with Undo.
    pub fn pop_out_undoable(&mut self, id: &str, cx: &mut Context<Self>) {
        self.dragging_tab = None;
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else { return };
        let t = &self.tabs[ix];
        let before = vec![(t.id.clone(), t.group.clone(), t.pinned_group.clone())];
        self.move_tab(ix, None, cx);
        let t = &self.tabs[ix];
        if before[0].1 == t.group && before[0].2.is_none() {
            return;
        }
        let (title, to) = (t.title(), t.group.clone());
        self.move_undo.push(before);
        self.notice_brief = false;
        self.notice = Some((format!("Moved {title} to {}", self.group_label(&to)), Instant::now()));
        self.notice_undo = true;
        self.match_undo = None;
        cx.notify();
    }

    /// Move tabs (by id) into group `to`, remembering where they were for the notice's Undo.
    pub fn move_tabs_undoable(&mut self, ids: &[String], to: &PathBuf, cx: &mut Context<Self>) {
        self.dragging_tab = None;
        let mut before = Vec::new();
        for id in ids {
            let Some(ix) = self.tabs.iter().position(|t| &t.id == id) else { continue };
            let t = &self.tabs[ix];
            if &t.group == to {
                continue;
            }
            before.push((t.id.clone(), t.group.clone(), t.pinned_group.clone()));
            self.move_tab(ix, Some(to.clone()), cx);
        }
        if before.is_empty() {
            return;
        }
        let n = before.len();
        let name = self.group_label(to);
        let what = if n == 1 { self.tabs.iter().find(|t| t.id == before[0].0).map(|t| t.title()).unwrap_or_default() } else { format!("{n} tabs") };
        self.move_undo.push(before);
        if self.move_undo.len() > 20 {
            self.move_undo.remove(0);
        }
        self.notice_brief = false;
        self.notice = Some((format!("Moved {what} to {name}"), Instant::now()));
        self.notice_undo = true;
        self.match_undo = None;
        cx.notify();
    }

    /// Dragging near the sidebar list's top or bottom edge scrolls it, faster the closer to
    /// the edge (and at full speed past it), for as long as the drag stays there.
    pub fn drag_near_edge(&mut self, y: f32, top: f32, bottom: f32, cx: &mut Context<Self>) {
        const ZONE: f32 = 40.;
        const MAX: f32 = 14.;
        let speed = if y < top + ZONE {
            -MAX * ((top + ZONE - y) / ZONE).min(1.)
        } else if y > bottom - ZONE {
            MAX * ((y - (bottom - ZONE)) / ZONE).min(1.)
        } else {
            0.
        };
        self.drag_scroll = speed;
        if speed == 0. || self.drag_scroll_task.is_some() {
            return;
        }
        self.drag_scroll_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(16)).await;
            let go = this.update(cx, |ws, cx| {
                if ws.drag_scroll == 0. || !cx.has_active_drag() {
                    ws.drag_scroll = 0.;
                    ws.drag_scroll_task = None;
                    return false;
                }
                let h = &ws.sidebar_scroll;
                let max = f32::from(h.max_offset().height);
                let mut o = h.offset();
                // Offsets run negative as the list scrolls down.
                o.y = px((f32::from(o.y) - ws.drag_scroll).clamp(-max, 0.));
                h.set_offset(o);
                cx.notify();
                true
            });
            if !go.unwrap_or(false) {
                break;
            }
        }));
    }

    /// Put the last moved tabs back where they were (Undo on the notice, or the palette).
    pub fn undo_move(&mut self, cx: &mut Context<Self>) {
        // Undo takes back whichever came last: a match or a move.
        if self.match_undo.is_some() {
            self.undo_match(cx);
            return;
        }
        let Some(before) = self.move_undo.pop() else { return };
        for (id, group, pinned) in before {
            if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                t.group = group;
                t.pinned_group = pinned;
            }
        }
        self.notice_brief = false;
        self.notice = Some(("Moved back".into(), Instant::now()));
        self.notice_undo = false;
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

    /// New groups join the end of the order; a group with no tabs keeps its slot 10 minutes.
    fn update_group_order(&mut self) {
        let live: HashSet<&PathBuf> = self.tabs.iter().map(|t| &t.group).collect();
        let mut changed = false;
        for t in &self.tabs {
            if !self.group_order.contains(&t.group) {
                self.group_order.push(t.group.clone());
                changed = true;
            }
        }
        let now = Instant::now();
        for g in &self.group_order {
            if live.contains(g) {
                self.group_emptied.remove(g);
            } else {
                self.group_emptied.entry(g.clone()).or_insert(now);
            }
        }
        let before = self.group_order.len();
        let emptied = &self.group_emptied;
        self.group_order.retain(|g| live.contains(g) || emptied.get(g).is_none_or(|t| now.duration_since(*t) < Duration::from_secs(600)));
        if changed || self.group_order.len() != before {
            let order = &self.group_order;
            self.group_emptied.retain(|g, _| order.contains(g));
            self.save();
        }
    }

    /// Automatic hues: blue, magenta or cyan by a hash of the group's key (never the status
    /// colors), stepping to the next hue when the group above has the same one. Worked out
    /// again only when the order changes, so colors don't shuffle while you work.
    fn update_hues(&mut self) {
        let order: Vec<PathBuf> = self.grouped().into_iter().map(|(g, _)| g).collect();
        if order == self.hues_for {
            return;
        }
        const HUES: [u8; 3] = [4, 5, 6];
        let mut prev: Option<u8> = None;
        self.auto_hues.clear();
        for g in &order {
            let hue = match self.group_colors.get(g) {
                Some(&c) => c,
                None => {
                    let h = g.to_string_lossy().bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
                    let mut i = (h % 3) as usize;
                    if prev == Some(HUES[i]) {
                        i = (i + 1) % 3;
                    }
                    HUES[i]
                }
            };
            self.auto_hues.insert(g.clone(), hue);
            prev = Some(hue);
        }
        self.hues_for = order;
    }

    /// The icon suggestions for a group (worked out on first sight; see `ensure_icon_suggestions`).
    pub fn icon_suggestions(&self, key: &std::path::Path) -> Vec<String> {
        self.icon_suggestions.get(key).cloned().unwrap_or_default()
    }

    /// Work out a group's icon suggestions if it has none yet: keywords now, and the helper
    /// (when it's on) replaces them in the background.
    pub fn ensure_icon_suggestions(&mut self, key: &std::path::Path, cx: &mut Context<Self>) {
        if self.icon_suggestions.contains_key(key) {
            return;
        }
        let label = self.group_label(&key.to_path_buf());
        let titles: Vec<String> = self.tabs.iter().filter(|t| t.group == key).map(|t| t.title()).take(5).collect();
        self.icon_suggestions.insert(key.to_path_buf(), crate::group_icons::suggest_local(key, &label, &titles));
        self.helper_icon(key, &label, &titles, cx);
    }

    /// Drop group `moved` just above group `before` (dragging a header).
    pub fn reorder_group(&mut self, moved: &PathBuf, before: &PathBuf, cx: &mut Context<Self>) {
        if moved == before {
            return;
        }
        self.update_group_order();
        self.group_order.retain(|g| g != moved);
        let at = self.group_order.iter().position(|g| g == before).unwrap_or(self.group_order.len());
        self.group_order.insert(at, moved.clone());
        self.save();
        cx.notify();
    }

    /// Move a group one place up (`-1`) or down (`1`) in the sidebar.
    pub fn move_group(&mut self, key: &PathBuf, delta: isize, cx: &mut Context<Self>) {
        self.update_group_order();
        // Step over groups that aren't on screen (kept slots of closed groups).
        let shown: Vec<PathBuf> = self.grouped().into_iter().map(|(g, _)| g).collect();
        let Some(pos) = shown.iter().position(|g| g == key) else { return };
        let target = pos as isize + delta;
        if target < 0 || target as usize >= shown.len() {
            return;
        }
        let other = &shown[target as usize];
        let (a, b) = (self.group_order.iter().position(|g| g == key), self.group_order.iter().position(|g| g == other));
        if let (Some(a), Some(b)) = (a, b) {
            self.group_order.swap(a, b);
            self.save();
            cx.notify();
        }
    }

    /// Claude fires no hook when you answer or dismiss its question some other way (Esc,
    /// rewind): a tab marked Needs you whose question is no longer on screen for two polls
    /// in a row goes back to Idle.
    fn clear_stale_needs(&mut self, cx: &mut Context<Self>) -> bool {
        const ASKING: &[&str] = &["Do you want", "Would you like", "❯ 1.", "1. Yes", "Esc to cancel", "Enter to confirm", "Enter to select"];
        let mut changed = false;
        for t in &mut self.tabs {
            let Some(a) = t.agent.as_mut().filter(|a| matches!(a.status, Status::NeedsInput { .. })) else {
                self.needs_gone.remove(&t.id);
                continue;
            };
            let lines = t.view.read(cx).terminal.last_lines(20);
            if lines.iter().any(|l| ASKING.iter().any(|m| l.contains(m))) {
                self.needs_gone.remove(&t.id);
                continue;
            }
            let n = self.needs_gone.entry(t.id.clone()).or_insert(0);
            *n += 1;
            if *n >= 2 {
                self.needs_gone.remove(&t.id);
                a.now = None;
                a.status = Status::Idle;
                a.since = std::time::Instant::now();
                changed = true;
            }
        }
        changed
    }

    /// A prompt can start a turn that ends with no reply and no Stop hook; the tab then stays
    /// Working forever. While Claude works it shows "esc to interrupt" under its spinner, so a
    /// Working Claude tab without it on screen for 15 s goes back to Idle.
    fn clear_stale_working(&mut self, cx: &mut Context<Self>) -> bool {
        const GRACE: Duration = Duration::from_secs(15);
        let mut changed = false;
        for t in &mut self.tabs {
            let claude = t.agent.as_ref().is_some_and(|a| a.kind == t1k_agent::agents::AgentKind::Claude);
            let Some(a) = t.agent.as_mut().filter(|a| claude && a.status == Status::Working) else {
                self.working_gone.remove(&t.id);
                continue;
            };
            let lines = t.view.read(cx).terminal.last_lines(25);
            if lines.iter().any(|l| l.to_lowercase().contains("to interrupt")) {
                self.working_gone.remove(&t.id);
                continue;
            }
            let since = *self.working_gone.entry(t.id.clone()).or_insert_with(Instant::now);
            if since.elapsed() >= GRACE && a.since.elapsed() >= GRACE {
                self.working_gone.remove(&t.id);
                a.now = None;
                a.status = Status::Idle;
                a.since = Instant::now();
                changed = true;
            }
        }
        changed
    }

    pub fn group_hovered(&mut self, key: &std::path::Path, over: bool, cx: &mut Context<Self>) {
        if over {
            self.hovered_group = Some((key.to_path_buf(), Instant::now()));
            // Repaint for the slide-in (150 ms), then stop.
            self.hover_anim = Some(cx.spawn(async move |this, cx| {
                for _ in 0..12 {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        } else if self.hovered_group.as_ref().is_some_and(|(k, _)| k == key) {
            self.hovered_group = None;
        }
        cx.notify();
    }

    /// 0 → 1 over 150 ms (ease-out) while the pointer is on group `key`'s header.
    pub fn group_hover_progress(&self, key: &std::path::Path) -> f32 {
        match &self.hovered_group {
            Some((k, at)) if k == key => {
                let t = (at.elapsed().as_secs_f32() / 0.15).min(1.);
                1. - (1. - t) * (1. - t)
            }
            _ => 0.,
        }
    }

    /// Write every tab's scrollback to `scrollback/<tab id>.ansi` (at quit). Owner-only, since
    /// terminal output can hold secrets.
    fn save_scrollback(&mut self, cx: &mut Context<Self>) {
        self.scrollback_saved = false;
        if !self.config.terminal.restore_scrollback || crate::demo::on() {
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
        self.context_tick(cx);
        // Demo tabs keep their made-up state: no processes, folders or gh to look at.
        if crate::demo::on() {
            return;
        }
        // The window can become key before the activation observer exists; catch up here.
        crate::secure_input::apply(self.config.terminal.secure_input, !self.window_inactive);
        // macOS asked to quit (Dock › Quit, logout): ask about running tabs, as ⌘Q does.
        if crate::platform::QUIT_REQUESTED.swap(false, std::sync::atomic::Ordering::SeqCst) {
            cx.dispatch_action(&crate::actions::Quit);
        }
        let mut changed = self.clear_stale_needs(cx);
        changed |= self.clear_stale_working(cx);
        let mut regrouped = false;
        for t in &mut self.tabs {
            if let Some(a) = t.agent.as_mut() {
                changed |= a.poll_transcript();
            }
            let term = &t.view.read(cx).terminal;
            let cwd = term.cwd();
            let program = term.foreground_name().filter(|n| !SHELLS.contains(&n.trim_start_matches('-')));
            if program.is_some() && t.was_running.is_some() {
                // Something runs again: the "was running" note has done its job.
                t.was_running = None;
                changed = true;
            }
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
            for p in t.panes.helpers.iter_mut() {
                let term = &p.view.read(cx).terminal;
                if let Some(c) = term.cwd() {
                    p.cwd = Some(c);
                }
                let program = term.foreground_name().filter(|n| !crate::close::SHELLS.contains(&n.trim_start_matches('-')));
                let command = program.as_ref().and_then(|_| term.foreground_command()).map(|(c, _)| c);
                if program != p.program || command != p.command {
                    p.program = program;
                    p.command = command;
                    changed = true;
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
        changed |= self.poll_waiting(cx);
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
        let needs = !self.needs_queue().is_empty();
        if self.had_needs && !needs {
            self.caught_up_at = Some(Instant::now());
            // Repaint through the fade-out, then once more when it's gone.
            self.caught_up_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CAUGHT_UP_SHOW).await;
                for _ in 0..20 {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                }
                let _ = this.update(cx, |ws, cx| {
                    ws.caught_up_at = None;
                    cx.notify();
                });
            }));
        } else if needs {
            self.caught_up_at = None;
            self.caught_up_task = None;
        }
        self.had_needs = needs;
        self.update_group_order();
        self.update_hues();
        // Icon suggestions are ready before anyone opens the picker.
        let missing: Vec<PathBuf> = self.group_order.iter().filter(|g| !self.icon_suggestions.contains_key(*g)).cloned().collect();
        for g in missing {
            self.ensure_icon_suggestions(&g, cx);
        }
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
        match if crate::demo::on() { None } else { self.watcher.poll() } {
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
        changed |= self.stamp_updates();
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
        // The usage meter's countdowns and staleness move with the clock.
        let minute = crate::usage::now() / 60;
        let tick = !self.usage.is_empty() && std::mem::replace(&mut self.usage_minute, minute) != minute;
        if changed || working || tick {
            cx.notify();
        }
    }

    fn apply_envelope(&mut self, mut env: Envelope, window: &Window) -> bool {
        if let Some(tab) = self.hook_alias.get(&env.tab_id) {
            env.tab_id = tab.clone();
        }
        // Plan usage limits are per account, not per tab: keep the latest from any tab.
        if env.payload.get("hook_event_name").and_then(|v| v.as_str()) == Some(t1k_agent::statusline::EVENT) {
            // How full this tab's context is, as the agent itself reports it.
            let mut ctx_changed = false;
            if let Some(ctx) = t1k_agent::statusline::context(&env.payload) {
                if let Some(a) = self.tabs.iter_mut().find(|t| t.id == env.tab_id).and_then(|t| t.agent.as_mut()) {
                    ctx_changed = a.context_reported != Some(ctx);
                    a.context_reported = Some(ctx);
                }
            }
            if let Some(cost) = t1k_agent::statusline::cost(&env.payload) {
                if let Some(a) = self.tabs.iter_mut().find(|t| t.id == env.tab_id).and_then(|t| t.agent.as_mut()) {
                    ctx_changed |= a.cost != Some(cost);
                    a.cost = Some(cost);
                }
            }
            let Some(usage) = t1k_agent::statusline::parse(&env.payload) else { return ctx_changed };
            let kind = env.payload.get("t1k_agent").and_then(|v| v.as_str()).and_then(t1k_agent::agents::AgentKind::from_id).unwrap_or_default();
            let changed = self.usage.get(&kind).is_none_or(|u| u.windows != usage.windows);
            self.usage.insert(kind, usage);
            return changed || ctx_changed;
        }
        // Codex writes its limits into its rollout file after each reply.
        let mut usage_changed = false;
        if let Some(path) = env.payload.get("codex_rollout").and_then(|v| v.as_str()) {
            let event = env.payload.get("hook_event_name").and_then(|v| v.as_str()).unwrap_or_default();
            if matches!(event, "Stop" | "SessionStart" | "UserPromptSubmit") {
                if let Some(usage) = read_tail(std::path::Path::new(path), 256 << 10).as_deref().and_then(t1k_agent::statusline::codex_rollout) {
                    let kind = t1k_agent::agents::AgentKind::Codex;
                    usage_changed = self.usage.get(&kind).is_none_or(|u| u.windows != usage.windows);
                    self.usage.insert(kind, usage);
                }
            }
        }
        let Some(ix) = self.tabs.iter().position(|t| t.id == env.tab_id) else { return usage_changed };
        // Its agent is talking again: it's back.
        self.restore_heard(&env.tab_id);
        let Some((hook, session_id)) = Hook::parse(&env.payload) else { return usage_changed };
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

    /// A program asked for a notification (OSC 9 / 777): a banner and an unread tab, unless
    /// you're looking at it. An agent in the main pane is skipped: its hooks already notify.
    pub fn program_notice(&mut self, ix: usize, n: &t1k_terminal::notify_scan::Notice, main_pane: bool, window: &Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(ix) else { return };
        if !self.config.notify.programs || (main_pane && tab.agent.is_some()) {
            return;
        }
        if ix == self.active && window.is_window_active() {
            return;
        }
        // No text: never log what programs print.
        log::info!("program notification (osc), banner posted");
        tab.unread = true;
        let title = if n.title.is_empty() { "T1K".to_string() } else { format!("T1K · {}", n.title) };
        crate::platform::notify_tab(&tab.id, &title, &tab.title(), &n.body);
        cx.notify();
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
            self.nothing_needs_you(window, cx);
            return;
        }
        self.unread_offer = None;
        let n = q.len() as isize;
        let next = match q.iter().position(|&i| i == self.active) {
            Some(p) => q[((p as isize + dir).rem_euclid(n)) as usize],
            None if dir > 0 => q[0],
            None => q[(n - 1) as usize],
        };
        self.activate(next, window, cx);
    }

    /// ⌘J with nothing waiting: say so (a key that does nothing feels broken). With unread tabs,
    /// offer the newest, and a second ⌘J within a few seconds opens it.
    fn nothing_needs_you(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, at)) = self.unread_offer.take() {
            if at.elapsed() < Duration::from_secs(5) {
                if let Some(ix) = self.tabs.iter().position(|t| t.id == id) {
                    self.notice = None;
                    self.activate(ix, window, cx);
                    return;
                }
            }
        }
        let unread: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].unread && i != self.active).collect();
        let working = self.tabs.iter().filter(|t| t.status() == Some(&Status::Working)).count();
        let key = crate::keymap::display_for(&JumpToNeeds, &self.config.keys);
        let text = if let Some(&newest) = unread.iter().max_by_key(|&&i| self.tabs[i].last_active) {
            self.unread_offer = Some((self.tabs[newest].id.clone(), Instant::now()));
            format!("Nothing needs you · {} unread, {key} again to open the newest", unread.len())
        } else if working > 0 {
            format!("Nothing needs you · {working} working")
        } else {
            "Nothing needs you right now".to_string()
        };
        self.notice_brief = false;
        self.notice = Some((text, Instant::now()));
        self.notice_undo = false;
        self.notice_brief = self.unread_offer.is_none();
        cx.notify();
    }

    /// Tabs folded into "+N quiet" in calm mode: nothing to see, not selected.
    pub fn is_quiet(&self, i: usize) -> bool {
        let t = &self.tabs[i];
        i != self.active
            && !self.restore_holds(&t.id)
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
                    icon: t.icon.clone(),
                    origin: t.origin.clone(),
                    split: None,
                    panes: t.panes.saved(),
                    scrollback: self.scrollback_saved.then(|| format!("{}.ansi", t.id)),
                    agent_live: self.resume_after_quit.contains(&t.id)
                        || t.agent.as_ref().is_some_and(|a| {
                            a.kind == t1k_agent::agents::AgentKind::Claude && !a.session_id.is_empty() && !matches!(a.status, Status::Ended)
                        }) && t.program.is_some(),
                    unread: t.unread,
                    updated: t.updated.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()),
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
            group_order: self.group_order.clone(),
            group_icons: self.group_icons.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            icon_suggestions: self.icon_suggestions.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            usage: self.usage.iter().map(|(k, u)| (k.id().to_string(), u.clone())).collect(),
            // What was already offered, so neither launch offer comes back for the same thing.
            import_seen: self.import_seen.clone(),
            import_last: self.import_last,
            match_offered: self.match_hidden,
            setup_step: self.setup.progress,
            ..Default::default()
        }
    }

    pub fn save(&self) {
        if crate::demo::on() {
            return;
        }
        session::save(&self.session());
    }

    // ---- sidebar detail card ----------------------------------------------------------------

    /// Hover intent for the sidebar detail card. The first card waits for the pointer to rest;
    /// once one is showing, moving to another row switches at once. Leaving hides it shortly
    /// after, unless another row picks it up first. Rows never change size.
    pub fn row_hovered(&mut self, id: String, hovered: bool, cx: &mut Context<Self>) {
        // The row under the pointer, right away (its × shows); the card waits for intent below.
        if hovered {
            if self.hover_row.as_ref() != Some(&id) {
                self.hover_row = Some(id.clone());
                cx.notify();
            }
        } else if self.hover_row.as_ref() == Some(&id) {
            self.hover_row = None;
            cx.notify();
        }
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
        self.ui_base = self.theme.ui();
        self.ui = self.ui_base.inactive(self.dim_level());
        let palette = self.theme.palette();
        for tab in &self.tabs {
            tab.view.update(cx, |v, cx| {
                v.palette = palette.clone();
                cx.notify();
            });
        }
        cx.notify();
    }

    /// Canvas v3-u: 150 ms ease-out into the inactive look, 80 ms back; instant with Reduce motion.
    /// Flip Secure Keyboard Entry. Written to config.toml; the watcher applies it.
    pub fn toggle_secure_input(&self) {
        crate::config::set_value(&["terminal", "secure_input"], (!self.config.terminal.secure_input).into());
    }

    fn set_inactive(&mut self, inactive: bool, cx: &mut Context<Self>) {
        self.window_inactive = inactive;
        crate::secure_input::apply(self.config.terminal.secure_input, !inactive);
        self.apply_dim(cx);
    }

    /// The pointer entered or left the sidebar (it undims a background window's chrome).
    pub fn sidebar_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.sidebar_hover != hovered {
            self.sidebar_hover = hovered;
            self.apply_dim(cx);
        }
    }

    fn apply_dim(&mut self, cx: &mut Context<Self>) {
        let inactive = self.window_inactive && !self.sidebar_hover;
        let to = if inactive { 1. } else { 0. };
        if self.dim.1 == to {
            return;
        }
        let secs = if crate::platform::reduce_motion() { 0. } else if inactive { 0.15 } else { 0.08 };
        self.dim = (self.dim_level(), to, Instant::now(), secs);
        self.ui = self.ui_base.inactive(self.dim_level());
        cx.notify();
        if secs > 0. {
            self.dim_anim = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let done = this.update(cx, |ws, cx| {
                    ws.ui = ws.ui_base.inactive(ws.dim_level());
                    cx.notify();
                    ws.dim.2.elapsed().as_secs_f32() >= ws.dim.3
                });
                if done.unwrap_or(true) {
                    break;
                }
            }));
        }
    }

    fn dim_level(&self) -> f32 {
        let (from, to, at, secs) = self.dim;
        let t = if secs <= 0. { 1. } else { (at.elapsed().as_secs_f32() / secs).min(1.) };
        let e = 1. - (1. - t) * (1. - t);
        from + (to - from) * e
    }

    // ---- actions ----------------------------------------------------------------------------

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let custom = self.tabs.get(self.active).map(|t| t.group.clone()).filter(|g| groups::is_custom(g));
        let cwd = self.tabs.get(self.active).and_then(|t| t.view.read(cx).terminal.cwd());
        let anchor = (!self.tabs.is_empty()).then_some(self.active);
        self.open_tab(cwd, None, window, cx);
        if let Some(g) = custom {
            self.move_tab(self.active, Some(g), cx);
        }
        self.place_new_tab(self.active, anchor);
    }

    /// ⌘Z: while a move's notice shows (with its Undo), take the move back; otherwise undo
    /// typing in the focused pane (Ctrl-_, which Claude Code, zsh and bash undo with).
    fn undo_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notice_undo && self.notice.is_some() {
            return self.undo_move(cx);
        }
        let Some(t) = self.tabs.get(self.active) else { return };
        let view = self.focused_view(window, cx).unwrap_or_else(|| t.view.clone());
        view.read(cx).terminal.write(&b"\x1f"[..]);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        // ⌘W in a tab with panes closes just the focused pane.
        if self.tabs.get(self.active).is_some_and(|t| t.panes.multi()) {
            let id = self.focused_pane(window, cx);
            self.close_pane(self.active, id, window, cx);
            return;
        }
        self.close_with_undo(self.active, window, cx);
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

    /// Whether the window is see-through (canvas v3-ar).
    pub fn translucent(&self) -> bool {
        self.config.window.alpha() < 1.
    }

    /// The terminal-side background: terminal, title strip, outputs panel.
    pub fn term_bg(&self) -> gpui::Hsla {
        self.ui.bg_terminal.opacity(self.config.window.alpha())
    }

    /// The sidebar's background: a little more solid than the terminal, so its text reads.
    pub fn side_bg(&self) -> gpui::Hsla {
        self.ui.bg_sidebar.opacity((self.config.window.alpha() + 0.05).min(1.))
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
            bg_alpha: self.config.window.alpha(),
            cell_alpha: self.config.window.background_opacity_cells,
        }
    }

    /// Re-read themes and push config to the theme and every terminal.
    pub fn apply_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = config::themes_dir() {
            t1k_themes::load_user_themes(&d);
        }
        (self.mode, self.dark, self.light) = Self::slots(&self.config);
        crate::updater::apply(&self.config.updates);
        crate::secure_input::apply(self.config.terminal.secure_input, window.is_window_active());
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
        crate::app_icon::SCALE.store(window.scale_factor().round().max(1.) as u32, std::sync::atomic::Ordering::Relaxed);
        // A drag that left the window (or was cancelled) leaves no outline behind.
        if self.drop_hover.is_some() && !cx.has_active_drag() && std::env::var_os("T1K_DEBUG_DROP").is_none() {
            self.drop_hover = None;
        }
        let look = match (self.translucent(), self.config.window.background_blur) {
            (false, _) => gpui::WindowBackgroundAppearance::Opaque,
            (true, true) => gpui::WindowBackgroundAppearance::Blurred,
            (true, false) => gpui::WindowBackgroundAppearance::Transparent,
        };
        if self.window_look != Some(look) {
            window.set_background_appearance(look);
            self.window_look = Some(look);
        }
        let active_view = crate::import_ui::waiting_pane(self, self.active, cx)
            .or_else(|| self.tabs.get(self.active).map(|t| crate::split::render_panes(self, t, window, cx)));

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
            // See-through, each surface paints its own background, so none doubles up.
            .when(!self.translucent() || self.tabs.is_empty(), |d| d.bg(self.term_bg()))
            .font_family(".SystemUIFont")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(|this, a: &JumpToNeeds, window, cx| {
                if !this.setup_intercept(crate::setup::Try::Jump, window, cx) {
                    this.jump_to_needs(a, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &JumpToNeedsBack, window, cx| this.walk_needs(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSwitcher, window, cx| {
                if this.setup_intercept(crate::setup::Try::Find, window, cx) {
                } else if matches!(this.overlay, Overlay::Switcher { .. }) {
                    this.close_overlay(window, cx);
                } else {
                    this.open_overlay(Overlay::Switcher { query: String::new(), selected: 0 }, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleOverview, window, cx| {
                if this.setup_intercept(crate::setup::Try::Overview, window, cx) {
                } else if matches!(this.overlay, Overlay::Overview { .. }) {
                    this.close_overlay(window, cx);
                } else {
                    let selected = this.visual_order().iter().position(|&i| i == this.active).unwrap_or(0);
                    this.open_overlay(Overlay::Overview { selected }, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RenameTab, window, cx| this.start_rename(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSecureInput, _, _| this.toggle_secure_input()))
            .on_action(cx.listener(|this, _: &ToggleOutputs, _, cx| this.toggle_outputs(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_overlay(Overlay::Settings { section: crate::settings::Section::Appearance }, window, cx)
            }))
            .on_action(cx.listener(Self::toggle_appearance))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                if !this.setup_intercept(crate::setup::Try::Split, window, cx) {
                    this.open_split(this.active, false, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &RunSetup, window, cx| this.open_setup(0, window, cx)))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| this.open_split(this.active, true, window, cx)))
            .on_action(cx.listener(|this, _: &FocusPaneLeft, window, cx| this.focus_dir(crate::split::Dir::Left, window, cx)))
            .on_action(cx.listener(|this, _: &FocusPaneRight, window, cx| this.focus_dir(crate::split::Dir::Right, window, cx)))
            .on_action(cx.listener(|this, _: &FocusNextPane, window, cx| this.focus_next_pane(window, cx)))
            .on_action(cx.listener(|this, _: &FocusPaneUp, window, cx| this.focus_dir(crate::split::Dir::Up, window, cx)))
            .on_action(cx.listener(|this, _: &FocusPaneDown, window, cx| this.focus_dir(crate::split::Dir::Down, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomPane, window, cx| this.toggle_zoom(window, cx)))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.navigate(true, window, cx)))
            .on_action(cx.listener(|this, _: &ShowAbout, window, cx| this.open_overlay(Overlay::About, window, cx)))
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| this.check_for_updates(window, cx)))
            .on_action(cx.listener(|this, _: &ReportBug, window, cx| this.open_report_bug(None, window, cx)))
            .on_action(cx.listener(|this, _: &ShowUpdate, window, cx| this.open_update(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, window, cx| {
                if this.setup_intercept(crate::setup::Try::Palette, window, cx) {
                } else if matches!(this.overlay, Overlay::Palette { .. }) {
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
            .on_action(cx.listener(|this, _: &ReopenClosedTab, window, cx| this.reopen_closed(window, cx)))
            .on_action(cx.listener(|this, _: &UndoMove, _, cx| this.undo_move(cx)))
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo_key(window, cx)))
            .on_action(cx.listener(|this, _: &MoveGroupUp, _, cx| {
                if let Some(g) = this.tabs.get(this.active).map(|t| t.group.clone()) {
                    this.move_group(&g, -1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MoveGroupDown, _, cx| {
                if let Some(g) = this.tabs.get(this.active).map(|t| t.group.clone()) {
                    this.move_group(&g, 1, cx);
                }
            }))
            .on_modifiers_changed(cx.listener(|this, ev: &gpui::ModifiersChangedEvent, _, cx| {
                if this.alt_held != ev.modifiers.alt {
                    this.alt_held = ev.modifiers.alt;
                    this.setup_alt(ev.modifiers.alt, cx);
                    cx.notify();
                }
            }));
        let root = select!(root, SelectTab1 => 0, SelectTab2 => 1, SelectTab3 => 2, SelectTab4 => 3,
            SelectTab5 => 4, SelectTab6 => 5, SelectTab7 => 6, SelectTab8 => 7, SelectTab9 => 8);

        use crate::rail::Shown;
        let shown = self.sidebar_shown();
        self.selected_row.set(None);
        let outline = crate::outline::render(self, shown, self.window_inactive, window);
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
        .children(outline)
        .children(peek)
        // With the sidebar folded away, its notices ("Nothing needs you", Undo) show as a small
        // toast at the bottom of the window instead.
        .when(!matches!(shown, Shown::Full(_)) && self.notice.is_some(), |d| {
            d.children(crate::import_ui::notice(self, cx).map(|n| {
                div().absolute().left_0().right_0().bottom(px(16.)).flex().justify_center().child(div().relative().w(px(400.)).h(px(54.)).child(n))
            }))
        })
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
    bytes.extend_from_slice(format!("\x1b[0m\x1b[2m{}\x1b[0m\r\n", t1k_terminal::terminal::RESTORED_BANNER).as_bytes());
    bytes
}

/// The last `max` bytes of a file, from the first whole line.
fn read_tail(path: &std::path::Path, max: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(max))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Some(if len > max { text.split_once('\n').map(|(_, r)| r.to_string()).unwrap_or_default() } else { text })
}

fn self_drop_hover(ws: &mut Workspace, target: crate::drop::DropTarget, paths: Vec<PathBuf>) {
    ws.drop_hover = Some(crate::drop::DropHover { target, paths });
}
