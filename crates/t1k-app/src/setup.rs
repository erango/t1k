//! "Set up T1K" (canvas v3-ah, option A): a sheet that walks through the settings people miss,
//! one step at a time, with a step list on the left and a live preview beside the choices.
//! Every choice writes config.toml at once (the watcher applies it), so the window behind the
//! sheet changes as you pick; nothing waits for Done. "All set" lists what each step changed,
//! with Undo. Agent switches change the agents' own settings files, and only when clicked.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, KeyDownEvent, MouseButton, SharedString, Window};
use t1k_agent::agents::AgentKind;
use t1k_themes::UiColors;
use toml_edit::Value;

use crate::import_ui::button;
use crate::input::{self, Key};
use crate::overlay::Overlay;
use crate::settings::{seg, stepper, switch, toggle};
use crate::workspace::Workspace;

pub const STEPS: usize = 7;
/// The "All set" page, after the last step.
pub const DONE: usize = STEPS;
const AGENTS: usize = 0;
const LOOK: usize = 1;
const SIDEBAR: usize = 2;
const TABS: usize = 3;
const ATTENTION: usize = 4;
const HELPER: usize = 5;
const KEYS: usize = 6;

const NAMES: [&str; STEPS] = ["Agents", "Look", "Sidebar", "Tabs", "Attention", "Helper", "Keys"];
const TITLES: [(&str, &str); STEPS] = [
    ("Connect your agents", "T1K shows what each agent is doing. Pick the ones you use."),
    ("Make it look like home", "Pick up your old terminal's look, or choose here."),
    ("Your sidebar", "How much each row shows. The preview updates as you choose."),
    ("Tabs and restarts", "Where tabs go, and what comes back when you relaunch."),
    ("What should get your attention", "Only what needs a person. Everything else stays quiet."),
    ("A helper for titles and recaps", "Optional. Off unless you pick an agent to run it."),
    ("Keys worth knowing", "Try each one. Setup comes back after."),
];

type CfgKey = &'static [&'static str];

/// A config key a step sets: its path, its name in "All set", and its choices (value, label).
struct Setting {
    key: CfgKey,
    name: &'static str,
    options: &'static [(&'static str, &'static str)],
}

const fn s(key: CfgKey, name: &'static str, options: &'static [(&'static str, &'static str)]) -> Setting {
    Setting { key, name, options }
}

const APPEARANCE: &[(&str, &str)] = &[("system", "System"), ("dark", "Dark"), ("light", "Light")];
const DENSITY: &[(&str, &str)] = &[("compact", "Compact"), ("comfortable", "Comfortable"), ("spacious", "Spacious")];
const SECOND_LINE: &[(&str, &str)] = &[("none", "None"), ("status", "Status"), ("branch", "Branch"), ("folder", "Folder")];
const TITLE: &[(&str, &str)] = &[("auto", "Auto"), ("command", "Command"), ("folder", "Folder"), ("branch", "Branch")];
const GROUP_COLORS: &[(&str, &str)] = &[("auto", "Automatic"), ("manual", "Only when I pick")];
const SESSION_COLOR: &[(&str, &str)] = &[("swatch", "Swatch"), ("corner", "Corner"), ("tint", "Tint"), ("line", "Line"), ("off", "Off")];
const NEW_TAB: &[(&str, &str)] = &[("after", "After current"), ("before", "Before"), ("end", "End of group")];
const AFTER_CLOSE: &[(&str, &str)] = &[("below", "Tab below"), ("above", "Tab above")];

/// What each step writes to config.toml, so its Undo can put it back.
const SETTINGS: [&[Setting]; STEPS] = [
    &[],
    &[
        s(&["appearance"], "Appearance", APPEARANCE),
        s(&["theme", "dark"], "Dark theme", &[]),
        s(&["theme", "light"], "Light theme", &[]),
        s(&["terminal", "font", "family"], "Font", &[]),
        s(&["terminal", "font", "size"], "Size", &[]),
        s(&["terminal", "font", "line_height"], "Line height", &[]),
    ],
    &[
        s(&["interface", "density"], "Density", DENSITY),
        s(&["sidebar", "second_line"], "Second line", SECOND_LINE),
        s(&["sidebar", "title"], "Titles", TITLE),
        s(&["sidebar", "group_colors"], "Group colors", GROUP_COLORS),
        s(&["sidebar", "session_color"], "Claude /color", SESSION_COLOR),
    ],
    &[
        s(&["tabs", "new_tab"], "New tabs", NEW_TAB),
        s(&["tabs", "after_close"], "After closing", AFTER_CLOSE),
        s(&["tabs", "close_stays_in_group"], "Stay in the group", &[]),
        s(&["terminal", "restore_scrollback"], "Keep output", &[]),
        s(&["terminal", "resume_agents"], "Resume Claude", &[]),
    ],
    &[
        s(&["notify", "needs_input"], "Needs you", &[]),
        s(&["notify", "error"], "Errors", &[]),
        s(&["notify", "reminder"], "Reminders", &[]),
        s(&["notify", "done"], "Finished", &[]),
        s(&["notify", "dock_badge"], "Dock badge", &[]),
    ],
    &[
        s(&["helper", "backend"], "Helper", &[]),
        s(&["helper", "titles"], "Titles", &[]),
        s(&["helper", "recaps"], "Recaps", &[]),
        s(&["helper", "search"], "Search", &[]),
    ],
    &[],
];

/// config.toml values as they were when a step was first shown (None: the key wasn't there).
pub type Snapshot = Vec<(CfgKey, Option<Value>)>;

/// The keys worth trying on the last step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Try {
    Find,
    Jump,
    Overview,
    Palette,
    Split,
    Alt,
}

impl Try {
    const ALL: [Try; 6] = [Try::Find, Try::Jump, Try::Overview, Try::Palette, Try::Split, Try::Alt];
    fn label(self) -> &'static str {
        match self {
            Try::Find => "Find any tab",
            Try::Jump => "Jump to what needs you",
            Try::Overview => "Overview of every tab",
            Try::Palette => "Every command",
            Try::Split => "Split the pane",
            Try::Alt => "Hold for detail in the sidebar",
        }
    }
    /// Its `keymap` id (⌥ isn't a binding).
    fn bind(self) -> Option<&'static str> {
        match self {
            Try::Find => Some("find_tab"),
            Try::Jump => Some("jump_to_needs"),
            Try::Overview => Some("overview"),
            Try::Palette => Some("command_palette"),
            Try::Split => Some("split_right"),
            Try::Alt => None,
        }
    }
}

/// Something setup's Agents step can switch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Agent(AgentKind),
    /// Claude Code's status line through T1K (its usage limits).
    Limits,
}

pub struct SetupState {
    /// The last step finished (saved in session.json): 0 once started, `STEPS` when done or
    /// skipped. None: never opened.
    pub progress: Option<u8>,
    /// Per step, config.toml when this run first showed it.
    before: Vec<Option<Snapshot>>,
    /// Agent switches flipped in this run, and how each was before the first flip.
    agents_before: Vec<(Target, bool)>,
    /// (agent, found on this Mac, hooks in), read when the step opens and after each switch.
    agents: Vec<(AgentKind, bool, bool)>,
    limits: bool,
    /// Steps passed with Next in this run (their ✓).
    done: [bool; STEPS],
    undone: [bool; STEPS],
    /// Keys tried, each with what its row says now.
    tried: Vec<(Try, Option<String>)>,
    /// A surface opened from setup: the step to come back to when it closes, and the key it
    /// was trying.
    returning: Option<(usize, Option<Try>)>,
    /// ⌥ went down on the Keys step; letting go ticks it.
    alt_down: bool,
    /// "Try it" on the ⌥ row was clicked: it says to hold the key.
    alt_hint: bool,
}

impl SetupState {
    pub fn new(progress: Option<u8>) -> Self {
        Self {
            progress,
            before: vec![None; STEPS],
            agents_before: Vec::new(),
            agents: Vec::new(),
            limits: false,
            done: [false; STEPS],
            undone: [false; STEPS],
            tried: Vec::new(),
            returning: None,
            alt_down: false,
            alt_hint: false,
        }
    }

    /// Where "Continue" picks up: the step after the last one finished, if setup was left
    /// halfway.
    pub fn resume_at(&self) -> Option<usize> {
        self.progress.map(usize::from).filter(|&p| p < STEPS)
    }

    fn tick(&mut self, t: Try, note: Option<String>) {
        self.tried.retain(|(x, _)| *x != t);
        self.tried.push((t, note));
    }

    fn flipped(&mut self, t: Target, was: bool) {
        if !self.agents_before.iter().any(|(x, _)| *x == t) {
            self.agents_before.push((t, was));
        }
    }

    fn refresh_agents(&mut self) {
        self.agents = AgentKind::ALL.iter().map(|&k| (k, crate::agent_setup::detected(k), crate::agent_setup::hooked(k))).collect();
        self.limits = crate::agent_setup::claude_limits();
    }

    fn is_on(&self, t: Target) -> bool {
        match t {
            Target::Agent(k) => self.agents.iter().any(|(x, _, on)| *x == k && *on),
            Target::Limits => self.limits,
        }
    }
}

/// A value without its comments and spacing, to compare.
fn norm(v: &Option<Value>) -> Option<String> {
    v.as_ref().map(|v| {
        let mut v = v.clone();
        v.decor_mut().clear();
        v.to_string()
    })
}

/// `keys` as written in `text`.
pub fn snapshot_in(text: &str, keys: &[CfgKey]) -> Snapshot {
    keys.iter().map(|k| (*k, crate::config::get_raw_in(text, k))).collect()
}

/// The keys whose value in `text` isn't what the snapshot has.
pub fn changed_in<'a>(before: &'a Snapshot, text: &str) -> Vec<&'a (CfgKey, Option<Value>)> {
    before.iter().filter(|(k, v)| norm(v) != norm(&crate::config::get_raw_in(text, k))).collect()
}

/// `text` with the snapshot's values put back (keys it didn't have are removed).
pub fn restore_in(text: &str, before: &Snapshot) -> String {
    let mut out = text.to_string();
    for (k, v) in before {
        let next = match v {
            Some(v) => crate::config::edit(&out, k, v.clone()),
            None => crate::config::remove_in(&out, k),
        };
        if let Some(n) = next {
            out = n;
        }
    }
    out
}

fn config_text() -> String {
    crate::config::path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default()
}

/// A value as "All set" says it: the control's label for a choice, on / off for a switch.
fn plain(v: &Value, options: &[(&str, &str)]) -> String {
    match v {
        Value::String(s) => {
            let s = s.value();
            options.iter().find(|(id, _)| id == s).map(|(_, l)| l.to_string()).unwrap_or_else(|| s.clone())
        }
        Value::Boolean(b) => if *b.value() { "on" } else { "off" }.to_string(),
        other => norm(&Some(other.clone())).unwrap_or_default(),
    }
}

/// One line for what a step changed in config.toml ("Second line: Status, Density: Compact").
fn summary_in(step: usize, before: &Snapshot, text: &str) -> Option<String> {
    let parts: Vec<String> = changed_in(before, text)
        .into_iter()
        .filter_map(|(k, _)| {
            let set = SETTINGS[step].iter().find(|s| s.key == *k)?;
            let now = crate::config::get_raw_in(text, k).map(|v| plain(&v, set.options)).unwrap_or_else(|| "default".into());
            Some(format!("{}: {now}", set.name))
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// The shortcut in effect for a keymap id ("⌘P"), or None when it has none.
fn key_of(ws: &Workspace, id: &str) -> Option<String> {
    let b = crate::keymap::ALL.iter().find(|b| b.id == id)?;
    crate::keymap::keys_for(b, &ws.config.keys).first().map(|k| crate::keymap::display(k))
}

impl Workspace {
    /// Open setup at `step`: 0 starts over (every control shows the current config), a later
    /// step continues a run left halfway.
    pub fn open_setup(&mut self, step: usize, window: &mut Window, cx: &mut Context<Self>) {
        // A new run: progress counts from where it starts (Continue keeps its place).
        self.setup = SetupState::new(Some(step.min(STEPS) as u8));
        for d in self.setup.done.iter_mut().take(step) {
            *d = true;
        }
        self.save();
        if self.matcher.is_none() {
            self.scan_for_match(cx);
        }
        self.setup_enter(step);
        self.open_overlay(Overlay::Setup { step }, window, cx);
    }

    /// First time this run shows a step: remember its settings for Undo.
    fn setup_enter(&mut self, step: usize) {
        if step < STEPS && self.setup.before[step].is_none() {
            let keys: Vec<CfgKey> = SETTINGS[step].iter().map(|s| s.key).collect();
            self.setup.before[step] = Some(snapshot_in(&config_text(), &keys));
        }
        if step == AGENTS {
            self.setup.refresh_agents();
        }
    }

    fn setup_go(&mut self, step: usize, cx: &mut Context<Self>) {
        self.fonts.close();
        self.setup_enter(step);
        if step == DONE {
            self.setup.progress = Some(STEPS as u8);
            self.save();
        }
        self.overlay = Overlay::Setup { step };
        cx.notify();
    }

    /// Next (or Skip this step): the step counts as finished either way.
    fn setup_next(&mut self, step: usize, next: bool, cx: &mut Context<Self>) {
        if step >= STEPS {
            return;
        }
        self.setup.done[step] |= next;
        self.setup.progress = Some((step + 1) as u8);
        self.save();
        self.setup_go(step + 1, cx);
    }

    /// Skip setup: it doesn't come back as "Finish setting up".
    fn setup_skip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.setup.progress = Some(STEPS as u8);
        self.save();
        self.close_overlay(window, cx);
    }

    pub fn setup_key(&mut self, step: usize, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        // The open font picker takes the typing for its search, as in Settings.
        if self.fonts.open {
            if let Key::Cancel = input::handle(&mut self.fonts.query, ev) {
                if self.fonts.query.is_empty() {
                    self.fonts.close();
                } else {
                    self.fonts.query.clear();
                }
            }
            return Some(Overlay::Setup { step });
        }
        match ev.keystroke.key.as_str() {
            // Esc keeps everything set so far; "Finish setting up" picks up from here.
            "escape" => None,
            "enter" if step == DONE => None,
            "enter" => {
                self.setup_next(step, true, cx);
                Some(std::mem::replace(&mut self.overlay, Overlay::None))
            }
            _ => Some(Overlay::Setup { step }),
        }
    }

    /// Remember to come back to the open step when the surface about to open closes.
    pub fn setup_leave(&mut self) {
        if let Overlay::Setup { step } = self.overlay {
            self.setup.returning = Some((step, None));
        }
    }

    /// From `close_overlay`: back to setup after something opened from it.
    pub fn setup_return(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((step, tried)) = self.setup.returning.take() else { return };
        if let Some(t) = tried {
            self.setup.tick(t, None);
        }
        self.setup_enter(step);
        self.open_overlay(Overlay::Setup { step }, window, cx);
    }

    /// A shortcut pressed on the Keys step tries it (and ticks its row) instead of its usual
    /// effect only where they differ. Returns true when setup handled it.
    pub fn setup_intercept(&mut self, t: Try, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !matches!(self.overlay, Overlay::Setup { step: KEYS }) {
            return false;
        }
        self.setup_try(t, window, cx);
        true
    }

    fn setup_try(&mut self, t: Try, window: &mut Window, cx: &mut Context<Self>) {
        let back = |this: &mut Self| this.setup.returning = Some((KEYS, Some(t)));
        match t {
            Try::Find => {
                back(self);
                self.open_overlay(Overlay::Switcher { query: String::new(), selected: 0 }, window, cx);
            }
            Try::Overview => {
                back(self);
                let selected = self.visual_order().iter().position(|&i| i == self.active).unwrap_or(0);
                self.open_overlay(Overlay::Overview { selected }, window, cx);
            }
            Try::Palette => {
                back(self);
                self.open_overlay(Overlay::Palette { query: String::new(), selected: 0 }, window, cx);
            }
            Try::Jump => {
                let key = key_of(self, "jump_to_needs").unwrap_or_else(|| "It".into());
                let note = if self.needs_queue().is_empty() {
                    format!("Nothing needs you right now, so {key} stays put")
                } else {
                    self.jump_next_needs(window, cx);
                    let title = self.tabs.get(self.active).map(|t| t.title()).unwrap_or_default();
                    format!("Took you to {title}")
                };
                // The jump focused the terminal; the sheet stays in front.
                window.focus(&self.overlay_focus);
                self.setup.tick(t, Some(note));
            }
            Try::Split => {
                self.open_split(self.active, false, window, cx);
                window.focus(&self.overlay_focus);
                let close = key_of(self, "close_tab").unwrap_or_else(|| "Close tab".into());
                self.setup.tick(t, Some(format!("{close} closes the split")));
            }
            // A click can't hold a key: say how, and tick it when ⌥ is let go.
            Try::Alt => self.setup.alt_hint = true,
        }
        cx.notify();
    }

    /// ⌥ on the Keys step: holding it swaps the preview's titles for status lines; letting go
    /// ticks the row.
    pub fn setup_alt(&mut self, down: bool, cx: &mut Context<Self>) {
        if !matches!(self.overlay, Overlay::Setup { step: KEYS }) {
            return;
        }
        if down {
            self.setup.alt_down = true;
        } else if std::mem::take(&mut self.setup.alt_down) {
            self.setup.tick(Try::Alt, None);
            cx.notify();
        }
    }

    fn setup_switch(&mut self, t: Target, on: bool, cx: &mut Context<Self>) {
        let was = self.setup.is_on(t);
        self.setup.flipped(t, was);
        let r = match (t, on) {
            (Target::Agent(k), true) => crate::agent_setup::install(k),
            (Target::Agent(k), false) => crate::agent_setup::remove(k),
            (Target::Limits, on) => crate::agent_setup::set_claude_limits(on),
        };
        if let Err(e) = r {
            log::error!("setup {t:?} {}: {e}", if on { "on" } else { "off" });
        }
        self.setup.refresh_agents();
        crate::settings::forget_agent_status();
        self.hooks_installed = crate::settings::claude_hooks_installed();
        cx.notify();
    }

    /// What a step changed in this run, as one line (None: nothing).
    fn setup_summary(&self, step: usize, text: &str) -> Option<String> {
        if step == AGENTS {
            let parts: Vec<String> = self
                .setup
                .agents_before
                .iter()
                .filter(|(t, was)| self.setup.is_on(*t) != *was)
                .map(|(t, was)| match t {
                    Target::Agent(k) => format!("{} {}", k.label(), if *was { "removed" } else { "connected" }),
                    Target::Limits => format!("usage limits {}", if *was { "off" } else { "on" }),
                })
                .collect();
            return (!parts.is_empty()).then(|| parts.join("; "));
        }
        summary_in(step, self.setup.before.get(step)?.as_ref()?, text)
    }

    fn setup_undo(&mut self, step: usize, cx: &mut Context<Self>) {
        if step == AGENTS {
            for (t, was) in self.setup.agents_before.clone() {
                if self.setup.is_on(t) != was {
                    self.setup_switch(t, was, cx);
                }
            }
        } else if let Some(before) = self.setup.before.get(step).cloned().flatten() {
            if let Some(p) = crate::config::path() {
                let text = config_text();
                let out = restore_in(&text, &before);
                if out != text {
                    let _ = std::fs::write(p, out);
                }
            }
        }
        self.setup.undone[step] = true;
        cx.notify();
    }
}

/// `T1K_DEBUG_OVERLAY=setup` / `setup-<n>` (1-7) / `setup-done`, with `T1K_DEBUG_SETUP=tried`
/// (every key tried) or `changed` (each step started from the defaults, so "All set" lists
/// whatever the test config changed).
pub fn debug_overlay(ws: &mut Workspace, which: &str, cx: &mut Context<Workspace>) -> Option<Overlay> {
    let step = match which.strip_prefix("setup-") {
        None => 0,
        Some("done") => DONE,
        Some(n) => n.parse::<usize>().ok()?.clamp(1, STEPS) - 1,
    };
    ws.setup = SetupState::new(Some(step as u8));
    for d in ws.setup.done.iter_mut().take(step) {
        *d = true;
    }
    // The first frame is the one a background window shows: have the match offer there.
    if ws.matcher.is_none() && step == LOOK {
        let fonts = cx.text_system().all_font_names();
        ws.matcher = Some(crate::match_ui::MatchState { found: crate::match_term::scan(), fonts, src: 0, off: Vec::new() });
    }
    match std::env::var("T1K_DEBUG_SETUP").as_deref() {
        Ok("tried") => {
            let key = key_of(ws, "jump_to_needs").unwrap_or_default();
            let close = key_of(ws, "close_tab").unwrap_or_default();
            for t in Try::ALL {
                let note = match t {
                    Try::Jump => Some(format!("Nothing needs you right now, so {key} stays put")),
                    Try::Split => Some(format!("{close} closes the split")),
                    _ => None,
                };
                ws.setup.tick(t, note);
            }
        }
        Ok("changed") => {
            for (i, b) in ws.setup.before.iter_mut().enumerate() {
                let keys: Vec<CfgKey> = SETTINGS[i].iter().map(|s| s.key).collect();
                *b = Some(snapshot_in(crate::config::template(), &keys));
            }
        }
        _ => {}
    }
    ws.setup_enter(step.min(STEPS));
    Some(Overlay::Setup { step })
}

/// The welcome card's section: start setup, or finish it.
pub fn welcome_section(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let (title, body, label, at) = match ws.setup.progress {
        None => ("Make T1K your own".to_string(), "7 short steps: agents, look, sidebar, tabs, attention. About 2 minutes.".to_string(), "Set up…", 0),
        Some(_) => {
            let at = ws.setup.resume_at()?;
            ("Finish setting up".to_string(), format!("You stopped at step {} of {STEPS}, {}.", at + 1, NAMES[at]), "Continue", at)
        }
    };
    Some(
        div()
            .w_full()
            .p(px(14.))
            .rounded(px(10.))
            .bg(ui.bg_terminal)
            .border_1()
            .border_color(ui.border)
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(title))
                    .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_tertiary).child(body)),
            )
            .child(button(ui, "welcome-setup", label, true, true).on_click(cx.listener(move |this, _, window, cx| this.open_setup(at, window, cx))))
            .into_any_element(),
    )
}

// ---- rendering ------------------------------------------------------------------------------

const ROW: f32 = 42.;
const STACKED: f32 = 60.;

/// A choice: label (and a line under it), control on the right. Fixed height.
fn line(ui: UiColors, label: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .h(px(ROW))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.))
        .border_t_1()
        .border_color(ui.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().truncate().text_size(px(13.)).text_color(ui.text_primary).child(label.into()))
                .children(desc.map(|d| div().truncate().text_size(px(11.5)).text_color(ui.text_tertiary).child(d))),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// A choice with more options than fit beside its label: the control goes under it.
fn stacked(ui: UiColors, label: &'static str, control: AnyElement) -> AnyElement {
    div()
        .h(px(STACKED))
        .flex_none()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(6.))
        .border_t_1()
        .border_color(ui.border)
        .child(div().text_size(px(13.)).text_color(ui.text_primary).child(label))
        .child(control)
        .into_any_element()
}

fn caption(ui: UiColors, text: impl Into<SharedString>) -> AnyElement {
    div().text_size(px(11.5)).line_height(px(16.)).text_color(ui.text_tertiary).child(text.into()).into_any_element()
}

/// A row in the preview sidebar.
struct Mini {
    title: String,
    sub: Option<(String, Hsla)>,
    icon: Hsla,
    /// Shows the Claude session's /color.
    colored: bool,
    selected: bool,
    /// A tab that isn't there yet (where a new one would go).
    ghost: bool,
}

/// Three sample tabs, drawn with the current sidebar settings (⌥ held: status in place of
/// titles, as in the sidebar).
fn samples(ws: &Workspace) -> Vec<Mini> {
    let ui = ws.ui;
    let c = &ws.config.sidebar;
    let claude: Hsla = gpui::rgb(0xD97757).into();
    // (title, command, folder, branch, status, needs you, is Claude)
    let rows = [
        ("Link hint mode", "claude", "t1k", "feat/hints", "Working · editing view.rs", false, true),
        ("Upgrade EKS to 1.31", "claude", "t1k", "eks-1.31", "Needs you · approve apply", true, true),
        ("cargo watch", "cargo watch", "t1k", "main", "Running", false, false),
    ];
    rows.iter()
        .enumerate()
        .map(|(i, &(title, command, folder, branch, status, needs, agent))| {
            let shown = match c.title.as_str() {
                "command" => command,
                "folder" => folder,
                "branch" => branch,
                _ => title,
            };
            let status_color = if needs { ui.status_needs } else { ui.text_tertiary };
            let sub = match c.second_line.as_str() {
                "status" => Some((status.to_string(), status_color)),
                "branch" => Some((branch.to_string(), ui.text_tertiary)),
                "folder" => Some((format!("~/git-repos/{folder}"), ui.text_tertiary)),
                _ => None,
            };
            let (title, sub) = if ws.alt_held { (status.to_string(), Some((shown.to_string(), ui.text_tertiary))) } else { (shown.to_string(), sub) };
            Mini { title, sub, icon: if agent { claude } else { ui.bg_control }, colored: i == 0, selected: i == 0, ghost: false }
        })
        .collect()
}

fn mini_sidebar(ws: &Workspace, rows: Vec<Mini>) -> AnyElement {
    let ui = ws.ui;
    let iface = &ws.config.interface;
    let session = crate::session_color::color("orange", ws.theme);
    let mode = ws.config.sidebar.session_color.as_str();
    let group = (ws.config.sidebar.group_colors != "manual").then(|| -> Hsla { gpui::rgb(ws.theme.ansi[4]).into() });
    let mut list = div().relative().flex().flex_col();
    for r in rows {
        let two = r.sub.is_some();
        let h = if two { iface.two_line_height() } else { iface.row_height() };
        let icon = (iface.icon_size(two) * 0.8).round();
        let tint = session.filter(|_| r.colored && mode == "tint");
        let line = session.filter(|_| r.colored && mode == "line");
        let swatch = session.filter(|_| r.colored && mode == "swatch");
        let corner = session.filter(|_| r.colored && mode == "corner");
        list = list.child(
            div()
                .relative()
                .h(px(h))
                .flex_none()
                .mx(px(4.))
                .pl(px(12.))
                .pr(px(8.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .when(r.selected, |d| d.bg(ui.bg_selected))
                .when_some(tint, |d, c| d.bg(c.opacity(0.14)))
                .when(r.ghost, |d| d.border_1().border_color(ui.accent))
                .when_some(line, |d, c| d.child(div().absolute().left(px(2.)).top_0().bottom_0().w(px(2.)).bg(c)))
                .child(
                    div()
                        .relative()
                        .size(px(icon))
                        .flex_none()
                        .rounded(px((icon * 0.24).round()))
                        .overflow_hidden()
                        .bg(if r.ghost { ui.bg_control.opacity(0.5) } else { r.icon })
                        .children(corner.map(|c| crate::session_color::corner(c, icon))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .children(swatch.map(crate::session_color::swatch))
                                .child(div().truncate().text_size(px(12.)).text_color(if r.ghost { ui.text_tertiary } else { ui.text_primary }).child(r.title)),
                        )
                        .children(r.sub.map(|(s, c)| div().truncate().text_size(px(10.5)).text_color(c).child(s))),
                ),
        );
    }
    div()
        .w_full()
        .py(px(10.))
        .rounded(px(8.))
        .bg(ui.bg_sidebar)
        .border_1()
        .border_color(ui.border)
        .relative()
        .child(div().pl(px(16.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_secondary).child("t1k"))
        .children(group.map(|c| div().absolute().left(px(6.)).top(px(30.)).bottom(px(10.)).w(px(2.)).bg(c.opacity(0.6))))
        .child(list)
        .into_any_element()
}

fn agents_step(ws: &Workspace, cx: &mut Context<Workspace>) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let ui = ws.ui;
    let mut rows = Vec::new();
    for &(kind, found, on) in &ws.setup.agents {
        let desc = if !found { "Not installed" } else if on { "Connected" } else { "Found on this Mac" };
        let control = if found {
            switch(ui, SharedString::from(format!("setup-agent-{}", kind.id())), on)
                .on_click(cx.listener(move |this, _, _, cx| this.setup_switch(Target::Agent(kind), !on, cx)))
                .into_any_element()
        } else {
            div().text_size(px(11.5)).text_color(ui.text_disabled).child("Not installed").into_any_element()
        };
        rows.push(line(ui, kind.label(), Some(desc.into()), control));
    }
    let limits = ws.setup.limits;
    rows.push(line(
        ui,
        "Show Claude usage limits",
        Some("Adds T1K's status line to Claude Code".into()),
        switch(ui, "setup-limits", limits).on_click(cx.listener(move |this, _, _, cx| this.setup_switch(Target::Limits, !limits, cx))),
    ));
    let preview = vec![mini_sidebar(ws, samples(ws)), caption(ui, "Live status comes from each agent's hooks. Switching one changes its settings file, with a backup.")];
    (rows, preview)
}

fn look_step(ws: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let ui = ws.ui;
    let c = &ws.config;
    let mut rows: Vec<AnyElement> = crate::match_ui::setup_offer(ws, cx).into_iter().collect();
    rows.push(stacked(ui, "Appearance", seg(ui, "setup-mode", &c.appearance, APPEARANCE, &["appearance"], cx)));
    let names = match c.appearance.as_str() {
        "dark" => ws.theme_slot(true).name.to_string(),
        "light" => ws.theme_slot(false).name.to_string(),
        _ => format!("{} · {}", ws.theme_slot(true).name, ws.theme_slot(false).name),
    };
    let theme = div()
        .id("setup-theme")
        .max_w(px(200.))
        .overflow_hidden()
        .h(px(26.))
        .px(px(10.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .bg(ui.bg_control)
        .cursor_pointer()
        .hover(move |d| d.bg(ui.accent.opacity(0.18)))
        .on_click(cx.listener(|this, _, window, cx| {
            let dark = this.theme.is_dark();
            this.setup_leave();
            this.open_theme_picker_for(dark, window, cx);
        }))
        .child(div().min_w_0().truncate().text_size(px(12.)).text_color(ui.text_primary).child(names))
        .child(div().text_size(px(13.)).text_color(ui.text_tertiary).child("›"));
    rows.push(line(ui, "Theme", None, theme));
    rows.push(line(ui, "Font", None, crate::fonts_ui::family_control(ws, window, cx)));
    let f = &c.terminal.font;
    rows.push(line(ui, "Size", None, stepper(ui, "setup-size", f.size as f64, 1., (8., 32.), &["terminal", "font", "size"], |v| format!("{v:.0} px"), cx)));
    rows.push(line(ui, "Line height", None, stepper(ui, "setup-lh", f.line_height as f64, 0.05, (1., 2.), &["terminal", "font", "line_height"], |v| format!("{v:.2}"), cx)));
    let preview = vec![
        crate::match_ui::preview(ws.theme, &f.family, f.size, f.line_height, 230., 150.).into_any_element(),
        caption(ui, "Every tab, open and new, changes as you pick."),
    ];
    (rows, preview)
}

fn sidebar_step(ws: &Workspace, cx: &mut Context<Workspace>) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let ui = ws.ui;
    let c = &ws.config;
    let rows = vec![
        stacked(ui, "Density", seg(ui, "setup-density", &c.interface.density, DENSITY, &["interface", "density"], cx)),
        stacked(ui, "Second line", seg(ui, "setup-second", &c.sidebar.second_line, SECOND_LINE, &["sidebar", "second_line"], cx)),
        stacked(ui, "Titles", seg(ui, "setup-title", &c.sidebar.title, TITLE, &["sidebar", "title"], cx)),
        stacked(ui, "Group colors", seg(ui, "setup-gcolors", &c.sidebar.group_colors, GROUP_COLORS, &["sidebar", "group_colors"], cx)),
        stacked(ui, "Claude /color", seg(ui, "setup-scolor", &c.sidebar.session_color, SESSION_COLOR, &["sidebar", "session_color"], cx)),
    ];
    (rows, vec![mini_sidebar(ws, samples(ws))])
}

fn tabs_step(ws: &Workspace, cx: &mut Context<Workspace>) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let ui = ws.ui;
    let c = &ws.config;
    let mut rows = vec![
        stacked(ui, "New tabs open", seg(ui, "setup-newtab", &c.tabs.new_tab, NEW_TAB, &["tabs", "new_tab"], cx)),
        stacked(ui, "After closing a tab, go to", seg(ui, "setup-afterclose", &c.tabs.after_close, AFTER_CLOSE, &["tabs", "after_close"], cx)),
        line(ui, "Stay in the same group", None, toggle(ui, "setup-stay", c.tabs.close_stays_in_group, &["tabs", "close_stays_in_group"], cx)),
        line(ui, "Keep each tab's output", Some("Scrollback comes back after a restart".into()), toggle(ui, "setup-rsb", c.terminal.restore_scrollback, &["terminal", "restore_scrollback"], cx)),
        line(ui, "Resume Claude sessions", Some("Sessions running at quit continue".into()), toggle(ui, "setup-rag", c.terminal.resume_agents, &["terminal", "resume_agents"], cx)),
    ];
    if let Some(found) = ws.import_found.as_ref().filter(|f| !f.is_empty()) {
        let first = found[0].source.label();
        let app = if found.iter().all(|f| f.source.label() == first) { first } else { "other terminals" };
        let n = found.len();
        rows.push(line(
            ui,
            format!("{n} tab{} open in {app}", if n == 1 { "" } else { "s" }),
            None,
            button(ui, "setup-import", "Bring them over…", false, true).on_click(cx.listener(|this, _, window, cx| {
                this.setup_leave();
                this.open_import(window, cx);
            })),
        ));
    }
    // Where ⌘T would put a tab, beside the current one (the first).
    let mut sample = samples(ws);
    let at = match c.tabs.new_tab.as_str() {
        "before" => 0,
        "after" => 1,
        _ => sample.len(),
    };
    sample.insert(at, Mini { title: "New tab".into(), sub: None, icon: ui.bg_control, colored: false, selected: false, ghost: true });
    for m in &mut sample {
        m.sub = None;
    }
    let after = if c.tabs.after_close == "above" { "above" } else { "below" };
    let stay = if c.tabs.close_stays_in_group { ", in the same group" } else { "" };
    let preview = vec![mini_sidebar(ws, sample), caption(ui, format!("Closing a tab goes to the one {after} it{stay}."))];
    (rows, preview)
}

fn attention_step(ws: &Workspace, cx: &mut Context<Workspace>) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let ui = ws.ui;
    let n = &ws.config.notify;
    let jump = match key_of(ws, "jump_to_needs") {
        Some(k) => format!("{k} jumps to the next tab that needs you, oldest first."),
        None => "Jumping to the next tab that needs you has no shortcut; Settings › Keybindings sets one.".into(),
    };
    let rows = vec![
        line(ui, "Notify when a tab needs you", None, toggle(ui, "setup-n-needs", n.needs_input, &["notify", "needs_input"], cx)),
        line(ui, "Notify on errors", None, toggle(ui, "setup-n-err", n.error, &["notify", "error"], cx)),
        line(ui, "Notify when a reminder is due", None, toggle(ui, "setup-n-rem", n.reminder, &["notify", "reminder"], cx)),
        line(ui, "Notify when an agent finishes", None, toggle(ui, "setup-n-done", n.done, &["notify", "done"], cx)),
        line(ui, "Dock badge", Some("Counts tabs that need you".into()), toggle(ui, "setup-n-badge", n.dock_badge, &["notify", "dock_badge"], cx)),
        div().pt(px(10.)).border_t_1().border_color(ui.border).text_size(px(12.)).text_color(ui.text_secondary).child(jump).into_any_element(),
    ];
    let banner = div()
        .p(px(10.))
        .rounded(px(8.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_md()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().text_size(px(10.5)).text_color(ui.text_tertiary).child("T1K"))
        .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child("Upgrade EKS to 1.31"))
        .child(div().text_size(px(11.)).text_color(ui.text_secondary).child("Needs you: approve terraform apply"));
    let dock = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .relative()
                .size(px(44.))
                .rounded(px(10.))
                .bg(ui.bg_control)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .font_weight(FontWeight::BOLD)
                .text_color(ui.text_secondary)
                .child("T1K")
                .when(n.dock_badge, |d| {
                    d.child(
                        div().absolute().top(px(-5.)).right(px(-5.)).size(px(18.)).rounded_full().bg(ui.status_error).flex().items_center().justify_center()
                            .text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()).child("2"),
                    )
                }),
        )
        .child(caption(ui, if n.dock_badge { "Dock badge" } else { "No badge" }));
    let mut preview = Vec::new();
    if n.needs_input {
        preview.push(banner.into_any_element());
    }
    preview.push(dock.into_any_element());
    preview.push(caption(ui, "Banners show while you're in another app."));
    (rows, preview)
}

fn helper_step(ws: &Workspace, cx: &mut Context<Workspace>) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let ui = ws.ui;
    let h = &ws.config.helper;
    let mut opts: Vec<(&'static str, &'static str)> = vec![("off", "Off")];
    for b in t1k_agent::helper::Backend::ALL {
        if b.find().is_some() || h.backend == b.id() {
            opts.push((b.id(), b.label()));
        }
    }
    let search = match key_of(ws, "find_tab") {
        Some(k) => format!("Smarter {k} search"),
        None => "Smarter tab search".into(),
    };
    let rows = vec![
        stacked(ui, "Helper", seg(ui, "setup-helper", &h.backend, &opts, &["helper", "backend"], cx)),
        line(ui, "Write tab titles", None, toggle(ui, "setup-h-titles", h.titles, &["helper", "titles"], cx)),
        line(ui, "Write recaps when a turn ends", None, toggle(ui, "setup-h-recaps", h.recaps, &["helper", "recaps"], cx)),
        line(ui, search, None, toggle(ui, "setup-h-search", h.search, &["helper", "search"], cx)),
        div()
            .pt(px(10.))
            .border_t_1()
            .border_color(ui.border)
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(ui.text_tertiary)
            .child("Each call is a short prompt with no tools, about 700 tokens. With Claude that's roughly $0.0001 per title. Every call is logged to helper.jsonl.")
            .into_any_element(),
    ];
    let on = h.backend().is_some();
    let codex = Mini {
        title: if on && h.titles { "Fix the flaky login test".into() } else { "codex".into() },
        sub: Some((if on && h.recaps { "Retries the token refresh once".into() } else { "Done".into() }, ui.text_tertiary)),
        icon: ui.text_secondary,
        colored: false,
        selected: true,
        ghost: false,
    };
    let mut rows_preview = vec![codex];
    rows_preview.extend(samples(ws).into_iter().skip(1).map(|mut m| {
        m.selected = false;
        m
    }));
    let preview = vec![mini_sidebar(ws, rows_preview), caption(ui, if on { "Titles and recaps like these come from the helper." } else { "Off: agents that don't name their sessions keep the program's name." })];
    (rows, preview)
}

fn keys_step(ws: &Workspace, cx: &mut Context<Workspace>) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let ui = ws.ui;
    let mut rows = Vec::new();
    for t in Try::ALL {
        let keys = match t.bind() {
            Some(id) => key_of(ws, id).unwrap_or_else(|| "—".into()),
            None => "⌥".into(),
        };
        let tried = ws.setup.tried.iter().find(|(x, _)| *x == t);
        let end = if tried.is_some() {
            div().flex_none().text_size(px(11.5)).text_color(ui.accent).child("✓ Tried").into_any_element()
        } else {
            div()
                .id(SharedString::from(format!("setup-try-{t:?}")))
                .flex_none()
                .px(px(6.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(ui.text_tertiary)
                .cursor_pointer()
                .hover(move |d| d.bg(ui.bg_hover).text_color(ui.text_primary))
                .on_click(cx.listener(move |this, _, window, cx| this.setup_try(t, window, cx)))
                .child("Try it")
                .into_any_element()
        };
        rows.push(
            div()
                .h(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(12.))
                .border_t_1()
                .border_color(ui.border)
                .child(
                    div().min_w(px(44.)).h(px(24.)).px(px(7.)).flex_none().rounded(px(5.)).bg(ui.bg_control).flex().items_center().justify_center()
                        .text_size(px(12.)).text_color(ui.text_primary).child(keys),
                )
                .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(ui.text_primary).child(t.label()))
                .child(end)
                .into_any_element(),
        );
    }
    // What the last try did (or how to try ⌥), on one line under the rows, so they keep
    // their height.
    let alt_tried = ws.setup.tried.iter().any(|(x, _)| *x == Try::Alt);
    let note = if ws.setup.alt_hint && !alt_tried {
        Some("Hold ⌥ now and watch the preview, then let go.".to_string())
    } else {
        ws.setup.tried.last().and_then(|(_, n)| n.clone()).map(|n| format!("{n}."))
    };
    rows.push(div().pt(px(10.)).border_t_1().border_color(ui.border).text_size(px(12.)).text_color(ui.text_secondary).children(note).into_any_element());
    let preview = vec![mini_sidebar(ws, samples(ws)), caption(ui, if ws.alt_held { "Let go of ⌥ to see the titles again." } else { "Hold ⌥ to see what each tab is doing." })];
    (rows, preview)
}

/// "All set": a line per step that changed something, each with Undo.
fn done_page(ws: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let ui = ws.ui;
    let text = config_text();
    let mut rows = Vec::new();
    for (step, name) in NAMES.iter().enumerate() {
        let undone = ws.setup.undone[step];
        let Some(summary) = ws.setup_summary(step, &text).or_else(|| undone.then(String::new)) else { continue };
        let end = if undone || summary.is_empty() {
            div().flex_none().text_size(px(12.)).text_color(ui.text_tertiary).child("Undone").into_any_element()
        } else {
            div()
                .id(SharedString::from(format!("setup-undo-{step}")))
                .flex_none()
                .text_size(px(12.))
                .text_color(ui.accent)
                .cursor_pointer()
                .hover(|d| d.underline())
                .on_click(cx.listener(move |this, _, _, cx| this.setup_undo(step, cx)))
                .child("Undo")
                .into_any_element()
        };
        rows.push(
            div()
                .h(px(38.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(10.))
                .border_t_1()
                .border_color(ui.border)
                .child(div().w(px(80.)).flex_none().text_size(px(12.5)).text_color(ui.text_tertiary).child(*name))
                .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(if undone { ui.text_tertiary } else { ui.text_primary }).child(if undone { "Back to how it was".to_string() } else { summary }))
                .child(end)
                .into_any_element(),
        );
    }
    if rows.is_empty() {
        rows.push(
            div().pt(px(10.)).border_t_1().border_color(ui.border).text_size(px(13.)).text_color(ui.text_tertiary)
                .child("Nothing changed. Your settings already fit.").into_any_element(),
        );
    }
    rows
}

pub fn render(ws: &Workspace, step: usize, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let st = &ws.setup;

    let mut steps = div()
        .w(px(180.))
        .flex_none()
        .h_full()
        .px(px(10.))
        .py(px(16.))
        .bg(ui.bg_sidebar)
        .border_r_1()
        .border_color(ui.border)
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().px(px(10.)).pb(px(10.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Set up T1K"));
    for (i, name) in NAMES.iter().enumerate() {
        let current = i == step;
        let done = !current && (st.done[i] || step == DONE);
        let circle = div()
            .size(px(18.))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.5))
            .map(|d| {
                if current {
                    d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::SEMIBOLD)
                } else if done {
                    d.bg(ui.bg_control).text_color(ui.text_secondary)
                } else {
                    d.border_1().border_color(ui.border).text_color(ui.text_tertiary)
                }
            })
            .child(if done { "✓".to_string() } else { (i + 1).to_string() });
        steps = steps.child(
            div()
                .id(SharedString::from(format!("setup-step-{i}")))
                .h(px(30.))
                .flex_none()
                .px(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(9.))
                .cursor_pointer()
                .when(current, |d| d.bg(ui.bg_selected))
                .when(!current, |d| d.hover(move |d| d.bg(ui.bg_hover)))
                .on_click(cx.listener(move |this, _, _, cx| this.setup_go(i, cx)))
                .child(circle)
                .child(div().text_size(px(13.)).text_color(if current { ui.text_primary } else if done { ui.text_secondary } else { ui.text_tertiary }).child(*name)),
        );
    }
    steps = steps.child(div().flex_1()).child(
        div()
            .id("setup-skip-all")
            .px(px(10.))
            .text_size(px(12.))
            .text_color(ui.text_tertiary)
            .cursor_pointer()
            .hover(move |d| d.text_color(ui.text_primary))
            .on_click(cx.listener(|this, _, window, cx| this.setup_skip(window, cx)))
            .child("Skip setup"),
    );

    let (title, intro) = if step == DONE { ("All set", "Everything here is also in Settings. You can run setup again any time.") } else { TITLES[step] };
    let (choices, preview) = match step {
        AGENTS => agents_step(ws, cx),
        LOOK => look_step(ws, window, cx),
        SIDEBAR => sidebar_step(ws, cx),
        TABS => tabs_step(ws, cx),
        ATTENTION => attention_step(ws, cx),
        HELPER => helper_step(ws, cx),
        KEYS => keys_step(ws, cx),
        _ => (done_page(ws, cx), Vec::new()),
    };
    let mut body = div().flex_1().min_h_0().flex().gap(px(18.)).child(div().flex_1().min_w_0().flex().flex_col().children(choices));
    if !preview.is_empty() {
        body = body.child(div().w(px(230.)).flex_none().flex().flex_col().gap(px(8.)).children(preview));
    }

    let enter = || div().text_size(px(11.)).opacity(0.7).child("↵");
    let footer = div().flex_none().flex().items_center().gap(px(8.)).pt(px(12.)).border_t_1().border_color(ui.border);
    let footer = if step == DONE {
        footer
            .child(button(ui, "setup-settings", "Open Settings", false, true).on_click(cx.listener(|this, _, window, cx| {
                this.open_overlay(Overlay::Settings { section: crate::settings::Section::Appearance }, window, cx)
            })))
            .child(div().flex_1())
            .child(button(ui, "setup-done", "Done", true, true).child(enter()).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
    } else {
        footer
            .child(
                div()
                    .id("setup-skip")
                    .text_size(px(12.5))
                    .text_color(ui.text_tertiary)
                    .cursor_pointer()
                    .hover(move |d| d.text_color(ui.text_primary))
                    .on_click(cx.listener(move |this, _, _, cx| this.setup_next(step, false, cx)))
                    .child("Skip this step"),
            )
            .child(div().flex_1())
            .when(step > 0, |d| d.child(button(ui, "setup-back", "Back", false, true).on_click(cx.listener(move |this, _, _, cx| this.setup_go(step - 1, cx)))))
            .child(
                button(ui, "setup-next", if step == KEYS { "Finish" } else { "Next" }, true, true)
                    .child(enter())
                    .on_click(cx.listener(move |this, _, _, cx| this.setup_next(step, true, cx))),
            )
    };

    div()
        .w(px(760.))
        .h(px(500.))
        .rounded(px(14.))
        .overflow_hidden()
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(steps)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .px(px(20.))
                .py(px(18.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(title))
                        .child(div().text_size(px(12.5)).text_color(ui.text_tertiary).child(intro)),
                )
                .child(body)
                .child(footer),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS3: &[CfgKey] = &[&["sidebar", "second_line"], &["interface", "density"], &["tabs", "new_tab"]];

    #[test]
    fn undo_puts_back_what_a_step_changed() {
        let start = crate::config::template().to_string();
        let before = snapshot_in(&start, KEYS3);
        assert!(changed_in(&before, &start).is_empty());
        // The step writes two keys; one is then removed by hand.
        let mut now = crate::config::edit(&start, &["sidebar", "second_line"], "status".into()).unwrap();
        now = crate::config::remove_in(&now, &["tabs", "new_tab"]).unwrap();
        let changed: Vec<CfgKey> = changed_in(&before, &now).into_iter().map(|(k, _)| *k).collect();
        assert_eq!(changed, vec![&["sidebar", "second_line"][..], &["tabs", "new_tab"][..]]);
        let back = restore_in(&now, &before);
        assert!(changed_in(&before, &back).is_empty());
        // Comments survive the round trip.
        assert!(back.contains("# An always-on second line"));
    }

    #[test]
    fn undo_removes_keys_that_werent_there() {
        let start = "[sidebar]\nwidth = 300\n";
        let before = snapshot_in(start, KEYS3);
        let now = crate::config::edit(start, &["interface", "density"], "compact".into()).unwrap();
        let back = restore_in(&now, &before);
        assert!(crate::config::get_raw_in(&back, &["interface", "density"]).is_none());
        assert!(changed_in(&before, &back).is_empty());
        assert!(back.starts_with(start));
    }

    #[test]
    fn summary_uses_the_controls_words() {
        let start = crate::config::template().to_string();
        let before = snapshot_in(&start, &SETTINGS[SIDEBAR].iter().map(|s| s.key).collect::<Vec<_>>());
        assert_eq!(summary_in(SIDEBAR, &before, &start), None);
        let now = crate::config::edit(&start, &["sidebar", "second_line"], "status".into()).unwrap();
        let now = crate::config::edit(&now, &["sidebar", "group_colors"], "manual".into()).unwrap();
        assert_eq!(summary_in(SIDEBAR, &before, &now).as_deref(), Some("Second line: Status, Group colors: Only when I pick"));
        // A comment or spacing change isn't a change.
        let spaced = start.replace("second_line = \"none\"", "second_line   =   \"none\"  # mine");
        assert_eq!(summary_in(SIDEBAR, &before, &spaced), None);
    }

    #[test]
    fn progress_resumes_after_the_last_finished_step() {
        assert_eq!(SetupState::new(None).resume_at(), None);
        assert_eq!(SetupState::new(Some(0)).resume_at(), Some(0));
        assert_eq!(SetupState::new(Some(3)).resume_at(), Some(3));
        assert_eq!(NAMES[3], "Tabs");
        // Done or skipped: nothing to finish.
        assert_eq!(SetupState::new(Some(STEPS as u8)).resume_at(), None);
        // Saved and read back with the session (an old file has no field).
        let s = crate::session::Session { setup_step: Some(3), ..Default::default() };
        let back: crate::session::Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.setup_step, Some(3));
        let old: crate::session::Session = serde_json::from_str("{\"tabs\": []}").unwrap();
        assert_eq!(old.setup_step, None);
    }

    #[test]
    fn every_step_key_is_in_the_template() {
        for (i, step) in SETTINGS.iter().enumerate() {
            for s in step.iter() {
                assert!(crate::config::get_raw_in(crate::config::template(), s.key).is_some(), "step {i}: {:?}", s.key);
            }
        }
    }
}
