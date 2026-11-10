//! "Set up T1K" (canvas v3-ah, option A, with the v3-ah′ design pass): a sheet that walks
//! through the settings people miss, one step at a time, with a step list on the left and a
//! live preview beside the choices. Every choice writes config.toml at once (the watcher
//! applies it), so the window behind the sheet changes as you pick; nothing waits for Done.
//! "All set" lists what each step changed, with Undo. Agent switches change the agents' own
//! settings files, and only when clicked.
//!
//! Every preview sits in one 230 × 284 frame with its caption inside, and shows what the
//! control used last does. The Keys step tries each shortcut on sample tabs (`SAMPLES`), so
//! it teaches the same thing on a fresh install as on a busy one, and nothing real moves.

use std::ops::Range;
use std::sync::{Arc, LazyLock};

use gpui::{div, img, prelude::*, px, AnyElement, Context, FontWeight, HighlightStyle, Hsla, KeyDownEvent, MouseButton, SharedString, StyledText, Window};
use t1k_agent::agents::AgentKind;
use t1k_themes::UiColors;
use toml_edit::Value;

use crate::import_ui::button;
use crate::input::{self, Key};
use crate::overlay::{Overlay, SwitcherFilter};
use crate::settings::{seg, stepper, switch, toggle};
use crate::sidebar::Mark;
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
    ("A helper for titles and recaps", "Optional. Claude tabs are titled either way."),
    ("Keys worth knowing", "Press a key to try it on sample tabs. Your own tabs aren't touched."),
];

type CfgKey = &'static [&'static str];

const DENSITY_KEY: CfgKey = &["interface", "density"];
const SECOND_KEY: CfgKey = &["sidebar", "second_line"];
const TITLE_KEY: CfgKey = &["sidebar", "title"];
const GCOLORS_KEY: CfgKey = &["sidebar", "group_colors"];
const SCOLOR_KEY: CfgKey = &["sidebar", "session_color"];
const NEW_TAB_KEY: CfgKey = &["tabs", "new_tab"];
const AFTER_CLOSE_KEY: CfgKey = &["tabs", "after_close"];
const STAY_KEY: CfgKey = &["tabs", "close_stays_in_group"];
const NEEDS_KEY: CfgKey = &["notify", "needs_input"];
const ERROR_KEY: CfgKey = &["notify", "error"];
const REMINDER_KEY: CfgKey = &["notify", "reminder"];
const FINISHED_KEY: CfgKey = &["notify", "done"];
const BADGE_KEY: CfgKey = &["notify", "dock_badge"];

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
        s(DENSITY_KEY, "Density", DENSITY),
        s(SECOND_KEY, "Second line", SECOND_LINE),
        s(TITLE_KEY, "Titles", TITLE),
        s(GCOLORS_KEY, "Group colors", GROUP_COLORS),
        s(SCOLOR_KEY, "Claude /color", SESSION_COLOR),
    ],
    &[
        s(NEW_TAB_KEY, "New tabs", NEW_TAB),
        s(AFTER_CLOSE_KEY, "After closing", AFTER_CLOSE),
        s(STAY_KEY, "Stay in the group", &[]),
        s(&["terminal", "restore_scrollback"], "Keep output", &[]),
        s(&["terminal", "resume_agents"], "Resume Claude", &[]),
    ],
    &[
        s(NEEDS_KEY, "Needs you", &[]),
        s(ERROR_KEY, "Errors", &[]),
        s(REMINDER_KEY, "Reminders", &[]),
        s(FINISHED_KEY, "Finished", &[]),
        s(BADGE_KEY, "Dock badge", &[]),
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
            Try::Overview => "See every tab at once",
            Try::Palette => "Run any command",
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

// ---- sample tabs ------------------------------------------------------------------------------

/// What a sample tab is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SampleState {
    Needs,
    Working,
    /// Finished, not looked at yet.
    Unread,
    Done,
    /// A shell running a program.
    Running,
    /// A shell at its prompt.
    Shell,
}

/// A tab that only exists in setup's previews, ⌘P and ⌘O. Plain data: never a `Tab`, never
/// in session.json, gone when setup closes.
pub struct SampleTab {
    pub title: &'static str,
    pub group: &'static str,
    /// For its icon (None: a shell prompt).
    pub program: Option<&'static str>,
    pub command: &'static str,
    pub branch: &'static str,
    pub state: SampleState,
    /// The status line ("Needs you · approve terraform apply").
    pub status: &'static str,
    /// How long it's been in its state ("21m"); empty for shells.
    pub age: &'static str,
    pub goal: &'static str,
    pub recap: &'static str,
    /// What ⌥ shows on a two-line row whose second line is the status.
    facts: &'static str,
}

#[allow(clippy::too_many_arguments)]
const fn sample(
    title: &'static str,
    group: &'static str,
    program: Option<&'static str>,
    command: &'static str,
    branch: &'static str,
    state: SampleState,
    status: &'static str,
    age: &'static str,
    goal: &'static str,
    recap: &'static str,
    facts: &'static str,
) -> SampleTab {
    SampleTab { title, group, program, command, branch, state, status, age, goal, recap, facts }
}

/// Seven tabs in three groups: one needs you (21m), one working, one done and unread, one
/// done, and shells. The first is the "current" tab; the previews draw the first five.
pub const SAMPLES: [SampleTab; 7] = {
    use SampleState::*;
    [
        sample("Link hint mode", "t1k", Some("claude"), "claude", "feat/hints", Working, "Working · editing view.rs", "2m", "Stable hint labels", "Labels in; wiring the overlay", "⎇ feat/hints · 41% context · 2m"),
        sample("Dedupe date utils", "t1k", Some("claude"), "claude", "dedupe-dates", Unread, "Done · 3 helpers merged", "8m", "One date helper", "3 helpers merged, tests pass", "⎇ dedupe-dates · #418 ✓ · 8m"),
        sample("cargo watch", "t1k", Some("cargo"), "cargo watch", "main", Running, "Running", "", "", "", "⎇ main"),
        sample("Upgrade EKS to 1.31", "infra-terraform", Some("claude"), "claude", "eks-1.31", Needs, "Needs you · approve terraform apply", "21m", "Move the cluster to 1.31", "Plan is clean; waiting on apply", "⎇ eks-1.31 · 63% context · 21m"),
        sample("Plan staging", "infra-terraform", None, "zsh", "ops/staging", Shell, "Shell", "", "", "", "⎇ ops/staging"),
        sample("Migrate charts to v5", "web-dashboard", Some("codex"), "codex", "charts-v5", Done, "Done · PR #421 opened", "1h", "Charts on v5", "PR #421 opened, CI green", "⎇ charts-v5 · #421 ✓ · 1h"),
        sample("npm run dev", "web-dashboard", Some("npm"), "npm run dev", "main", Running, "Running", "", "", "", "⎇ main"),
    ]
};

/// The sample tab that's "current" (its group is "This project" in the switcher).
const CURRENT: usize = 0;
/// The sample ⌘J goes to.
const WAITING: usize = 3;

impl SampleTab {
    pub fn needs(&self) -> bool {
        self.state == SampleState::Needs
    }

    pub fn agent(&self) -> bool {
        !matches!(self.state, SampleState::Running | SampleState::Shell)
    }

    pub fn mark(&self) -> Mark {
        match self.state {
            SampleState::Needs => Mark::Needs,
            SampleState::Working => Mark::Working,
            SampleState::Unread => Mark::Unread,
            SampleState::Running => Mark::Running,
            SampleState::Done | SampleState::Shell => Mark::None,
        }
    }

    /// The status word(s) and the rest: ("Needs you", "approve terraform apply").
    pub fn status_parts(&self) -> (&'static str, &'static str) {
        let (head, rest) = self.status.split_once(" · ").unwrap_or((self.status, ""));
        match self.state {
            SampleState::Unread => ("Done · unread", rest),
            _ => (head, rest),
        }
    }

    fn rank(&self) -> u8 {
        self.state as u8
    }

    fn keeps(&self, f: SwitcherFilter) -> bool {
        match f {
            SwitcherFilter::All => true,
            SwitcherFilter::NeedsYou => self.needs(),
            SwitcherFilter::Working => self.state == SampleState::Working,
            SwitcherFilter::Unread => self.state == SampleState::Unread,
            SwitcherFilter::Project => self.group == SAMPLES[CURRENT].group,
        }
    }
}

/// The samples ⌘P lists for `query` (indices into `SAMPLES`): needs you first, then agent
/// sessions, then shells, as the real switcher orders tabs.
pub fn sample_results(query: &str, filter: SwitcherFilter) -> Vec<usize> {
    let keep: Vec<usize> = (0..SAMPLES.len()).filter(|&i| SAMPLES[i].keeps(filter)).collect();
    if query.trim().is_empty() {
        let mut v = keep;
        v.sort_by_key(|&i| (!SAMPLES[i].needs(), !SAMPLES[i].agent()));
        return v;
    }
    let hays: Vec<String> = keep.iter().map(|&i| {
        let s = &SAMPLES[i];
        format!("{} {} {} {}", s.title, s.group, s.status, s.branch)
    }).collect();
    let mut scored: Vec<(u32, usize)> = crate::overlay::fuzzy(query, &hays).into_iter().map(|(sc, k)| (sc, keep[k])).collect();
    scored.sort_by_key(|&(sc, i)| (!SAMPLES[i].needs(), std::cmp::Reverse(sc)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// How many samples a switcher chip keeps.
pub fn sample_count(filter: SwitcherFilter) -> usize {
    SAMPLES.iter().filter(|s| s.keeps(filter)).count()
}

/// The overview's sample cards: the agent sessions, needs you first, then working, then done.
pub fn sample_cards() -> Vec<usize> {
    let mut v: Vec<usize> = (0..SAMPLES.len()).filter(|&i| SAMPLES[i].agent()).collect();
    v.sort_by_key(|&i| SAMPLES[i].rank());
    v
}

/// Where each word of `query` appears in `text` (ignoring case), for the switcher's highlight.
pub fn highlights(text: &str, query: &str) -> Vec<Range<usize>> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; highlight nothing rather than split
    // a character.
    if lower.len() != text.len() {
        return Vec::new();
    }
    let mut out: Vec<Range<usize>> = query
        .split_whitespace()
        .filter_map(|w| lower.find(&w.to_lowercase()).map(|at| at..at + w.len()))
        .collect();
    out.sort_by_key(|r| r.start);
    out.dedup_by(|b, a| b.start < a.end);
    out
}

/// The "Example" tag on every surface that shows sample tabs.
pub fn example_tag(ui: UiColors) -> gpui::Div {
    div()
        .flex_none()
        .h(px(18.))
        .px(px(5.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(ui.accent.opacity(0.5))
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ui.accent)
        .child("EXAMPLE")
}

// ---- state ------------------------------------------------------------------------------------

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
    /// Keys tried on the Keys step.
    tried: Vec<Try>,
    /// A surface opened from setup: the step to come back to when it closes, and the key it
    /// was trying.
    returning: Option<(usize, Option<Try>)>,
    /// ⌥ went down on the Keys step; letting go ticks it.
    alt_down: bool,
    /// "Try it" on the ⌥ row was clicked: it says to hold the key.
    alt_hint: bool,
    /// Controls used in this run, oldest first. A preview shows what the last of its own
    /// step's controls does.
    touched: Vec<CfgKey>,
    /// ⌘J was tried: the Keys preview's selection is on the sample that waits.
    jumped: bool,
    /// ⌘D was tried: the Keys preview shows a sample tab split in two.
    split: bool,
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
            touched: Vec::new(),
            jumped: false,
            split: false,
        }
    }

    /// Where "Continue" picks up: the step after the last one finished, if setup was left
    /// halfway.
    pub fn resume_at(&self) -> Option<usize> {
        self.progress.map(usize::from).filter(|&p| p < STEPS)
    }

    fn tick(&mut self, t: Try) {
        if !self.tried.contains(&t) {
            self.tried.push(t);
        }
    }

    fn touch(&mut self, key: CfgKey) {
        self.touched.retain(|k| *k != key);
        self.touched.push(key);
    }

    /// The control among `keys` used last in this run.
    fn last_touched(&self, keys: &[CfgKey]) -> Option<CfgKey> {
        self.touched.iter().rev().find(|k| keys.contains(k)).copied()
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
            self.setup.tick(t);
        }
        self.setup_enter(step);
        self.open_overlay(Overlay::Setup { step }, window, cx);
    }

    /// The switcher and overview show the sample tabs instead of yours: they were opened by
    /// trying their key on the Keys step.
    pub fn setup_samples(&self) -> bool {
        matches!(self.setup.returning, Some((KEYS, Some(Try::Find | Try::Overview))))
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
        match t {
            // ⌘P and ⌘O open the real surfaces, filled with the samples.
            Try::Find => {
                self.setup.returning = Some((KEYS, Some(t)));
                self.open_overlay(Overlay::Switcher { query: String::new(), selected: 0 }, window, cx);
            }
            Try::Overview => {
                self.setup.returning = Some((KEYS, Some(t)));
                self.open_overlay(Overlay::Overview { selected: 0 }, window, cx);
            }
            Try::Palette => {
                self.setup.returning = Some((KEYS, Some(t)));
                self.open_overlay(Overlay::Palette { query: String::new(), selected: 0 }, window, cx);
            }
            // ⌘J and ⌘D happen in the preview: the real window doesn't jump or split.
            Try::Jump => {
                self.setup.jumped = true;
                self.setup.split = false;
                self.setup.tick(t);
            }
            Try::Split => {
                self.setup.split = true;
                self.setup.tick(t);
            }
            // A click can't hold a key: say how, and tick it when ⌥ is let go.
            Try::Alt => {
                self.setup.alt_hint = true;
                self.setup.split = false;
            }
        }
        cx.notify();
    }

    /// ⌥ on the Keys step: holding it shows the preview's rows the way ⌥ shows the sidebar's;
    /// letting go ticks the row.
    pub fn setup_alt(&mut self, down: bool, cx: &mut Context<Self>) {
        if !matches!(self.overlay, Overlay::Setup { step: KEYS }) {
            return;
        }
        if down {
            self.setup.alt_down = true;
            self.setup.split = false;
        } else if std::mem::take(&mut self.setup.alt_down) {
            self.setup.tick(Try::Alt);
        }
        cx.notify();
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

/// `T1K_DEBUG_OVERLAY=setup` / `setup-<n>` (1-7) / `setup-done`, or `setup-switcher` /
/// `setup-overview` (⌘P / ⌘O tried on the Keys step, showing the samples).
/// `T1K_DEBUG_SETUP` takes a comma-separated list: `tried` (every key tried), `changed` (each
/// step started from the defaults, so "All set" lists whatever the test config changed),
/// `touch:<key>` (that control used last, by its last config key segment, e.g.
/// `touch:new_tab`), `jump` / `split` / `alt` / `alt-hint` (the Keys preview after ⌘J, ⌘D,
/// with ⌥ held, or after Try it on ⌥), `query:<text>` (typed into the sample switcher).
pub fn debug_overlay(ws: &mut Workspace, which: &str, cx: &mut Context<Workspace>) -> Option<Overlay> {
    let sample = match which {
        "setup-switcher" => Some(Try::Find),
        "setup-overview" => Some(Try::Overview),
        _ => None,
    };
    let step = match (sample, which.strip_prefix("setup-")) {
        (Some(_), _) => KEYS,
        (None, None) => 0,
        (None, Some("done")) => DONE,
        (None, Some(n)) => n.parse::<usize>().ok()?.clamp(1, STEPS) - 1,
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
    let mut query = String::new();
    let flags = std::env::var("T1K_DEBUG_SETUP").unwrap_or_default();
    for flag in flags.split(',').map(str::trim) {
        match flag {
            "tried" => {
                for t in Try::ALL {
                    ws.setup.tick(t);
                }
            }
            "changed" => {
                for (i, b) in ws.setup.before.iter_mut().enumerate() {
                    let keys: Vec<CfgKey> = SETTINGS[i].iter().map(|s| s.key).collect();
                    *b = Some(snapshot_in(crate::config::template(), &keys));
                }
            }
            "jump" => ws.setup.jumped = true,
            "split" => ws.setup.split = true,
            "alt" => ws.alt_held = true,
            "alt-hint" => ws.setup.alt_hint = true,
            f => {
                if let Some(name) = f.strip_prefix("touch:") {
                    if let Some(k) = SETTINGS.iter().flat_map(|s| s.iter()).map(|s| s.key).find(|k| k.last() == Some(&name)) {
                        ws.setup.touch(k);
                    }
                } else if let Some(q) = f.strip_prefix("query:") {
                    query = q.to_string();
                }
            }
        }
    }
    ws.setup_enter(step.min(STEPS));
    match sample {
        Some(t) => {
            ws.setup.returning = Some((KEYS, Some(t)));
            Some(if t == Try::Find { Overlay::Switcher { query, selected: 0 } } else { Overlay::Overview { selected: 0 } })
        }
        None => Some(Overlay::Setup { step }),
    }
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

// ---- rendering: choices -----------------------------------------------------------------------

const ROW: f32 = 42.;
/// A row whose sub-line takes two lines.
const ROW_TALL: f32 = 58.;
const STACKED: f32 = 60.;
/// Characters of a sub-line that fit on one line beside a switch (and under a stacked label).
/// The copy is fixed, so a row's height is decided by its words, never by its state.
const ONE_LINE: usize = 40;
const ONE_LINE_STACKED: usize = 56;

fn sub_line(ui: UiColors, d: SharedString) -> gpui::Div {
    div().text_size(px(11.5)).line_height(px(15.)).text_color(ui.text_tertiary).line_clamp(2).child(d)
}

/// A choice: label (and a sub-line under it), control on the right. Fixed height: one line
/// of sub-line fits in 42, a longer one wraps to two in 58.
fn line(ui: UiColors, label: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    let tall = desc.as_ref().is_some_and(|d| d.chars().count() > ONE_LINE);
    div()
        .h(px(if tall { ROW_TALL } else { ROW }))
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
                .children(desc.map(|d| sub_line(ui, d))),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// A choice with more options than fit beside its label: the control goes under it (and its
/// sub-line, when it has one, between them).
fn stacked(ui: UiColors, label: &'static str, desc: Option<&'static str>, control: AnyElement) -> AnyElement {
    let extra = match desc {
        None => 0.,
        Some(d) if d.chars().count() <= ONE_LINE_STACKED => 16.,
        Some(_) => 31.,
    };
    div()
        .h(px(STACKED + extra))
        .flex_none()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(6.))
        .border_t_1()
        .border_color(ui.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().text_size(px(13.)).text_color(ui.text_primary).child(label))
                .children(desc.map(|d| sub_line(ui, d.into()))),
        )
        .child(control)
        .into_any_element()
}

/// A row whose control the preview follows: using it (or clicking its label) makes it the
/// one the preview shows.
fn touched(key: CfgKey, row: AnyElement, cx: &mut Context<Workspace>) -> AnyElement {
    div()
        .flex_none()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.setup.touch(key);
                cx.notify();
            }),
        )
        .child(row)
        .into_any_element()
}

// ---- rendering: the preview frame -------------------------------------------------------------

const FRAME_W: f32 = 230.;
const FRAME_H: f32 = 284.;
/// The caption's height at two lines (padding 9 + 10, two 16 pt lines, its hairline).
const CAPTION_H: f32 = 52.;

/// The one preview frame every step uses, so the column never changes size: content from
/// `top`, and the caption pinned inside at the bottom, saying what the preview shows now.
fn frame(ui: UiColors, top: f32, content: impl IntoElement, caption: impl Into<SharedString>) -> AnyElement {
    div()
        .w(px(FRAME_W))
        .h(px(FRAME_H))
        .flex_none()
        .relative()
        .overflow_hidden()
        .rounded(px(10.))
        .bg(ui.bg_sidebar)
        .border_1()
        .border_color(ui.border)
        .child(div().absolute().top(px(top)).left_0().right_0().child(content))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .pt(px(9.))
                .px(px(12.))
                .pb(px(10.))
                // Covers rows that run long; rounded so the frame's corners stay round.
                .bg(ui.bg_sidebar)
                .rounded_b(px(9.))
                .border_t_1()
                .border_color(ui.border)
                .text_size(px(11.5))
                .line_height(px(16.))
                .text_color(ui.text_tertiary)
                .child(caption.into()),
        )
        .into_any_element()
}

/// A row in a preview sidebar.
struct Mini {
    title: String,
    sub: Option<String>,
    program: Option<&'static str>,
    state: SampleState,
    /// Shows the Claude session's /color.
    colored: bool,
    selected: bool,
    /// A tab that isn't there yet (where a new one would go).
    ghost: bool,
    /// The tab being closed.
    closing: bool,
    /// Where something lands (the tab after a close, ⌘J's jump): an accent ring.
    ring: bool,
    /// At the right end: "next", a wait.
    tag: Option<(String, Hsla, bool)>,
    /// ⌥ on a one-line row: the status stands in for the title, quiet, as in the sidebar.
    quiet: bool,
}

impl Mini {
    fn of(s: &SampleTab) -> Self {
        Mini {
            title: s.title.into(),
            sub: None,
            program: s.program,
            state: s.state,
            colored: false,
            selected: false,
            ghost: false,
            closing: false,
            ring: false,
            tag: None,
            quiet: false,
        }
    }
}

struct Group {
    label: &'static str,
    rows: Vec<Mini>,
}

/// The group's line color when groups get colors on their own (the rainbow, in order).
fn group_hue(ws: &Workspace, label: &str) -> Option<Hsla> {
    if ws.config.sidebar.group_colors == "manual" {
        return None;
    }
    let i = match label {
        "t1k" => 1,
        "infra-terraform" => 2,
        _ => 3,
    };
    Some(crate::groups::rainbow(i, ws.theme.is_dark()))
}

/// (one-line row, two-line row, icon) in the preview: the sidebar's sizes at 0.88, so
/// density still shows (30 / 42 / 18 at Comfortable).
fn metrics(ws: &Workspace) -> (f32, f32, f32) {
    match ws.config.interface.density.as_str() {
        "compact" => (26., 36., 16.),
        "spacious" => (35., 48., 20.),
        _ => (30., 42., 18.),
    }
}

/// The samples' icon with its status badge, as the sidebar draws it: amber dot when it needs
/// you, mercury when unread, the turning ring while it works; idle shells fade.
fn sample_icon(ws: &Workspace, program: Option<&'static str>, state: SampleState, size: f32, ring_bg: Hsla) -> gpui::Div {
    let ui = ws.ui;
    let d = (size * 0.42).round().max(8.);
    let off = -(d / 3.).round();
    let dot = match state {
        SampleState::Needs => Some(ui.status_needs),
        SampleState::Unread => Some(ui.accent),
        _ => None,
    };
    let ring = size + 6.;
    div()
        .relative()
        .size(px(size))
        .flex_none()
        .child(crate::app_icon::program_badge(program, ui, size).when(state == SampleState::Shell, |d| d.opacity(0.45)))
        .when(state == SampleState::Working, |d| {
            d.child(div().absolute().top(px(-3.)).left(px(-3.)).size(px(ring)).child(crate::sidebar::spinner_sized(ui.status_working, ws.started, ring / 2. - 1.)))
        })
        .children(dot.map(|c| div().absolute().right(px(off)).bottom(px(off)).size(px(d)).rounded_full().bg(c).border_2().border_color(ring_bg)))
}

fn mini_row(ws: &Workspace, r: Mini, two: bool) -> AnyElement {
    let ui = ws.ui;
    let (one_h, two_h, icon) = metrics(ws);
    let session = crate::session_color::color("orange", ws.theme).filter(|_| r.colored);
    let mode = ws.config.sidebar.session_color.as_str();
    let pick = |m: &str| session.filter(|_| mode == m);
    let bold = !r.ghost && !r.quiet && (r.selected || r.state == SampleState::Unread);
    let title_color = if r.ghost {
        ui.accent
    } else if r.quiet {
        ui.text_quiet
    } else if r.selected || r.state == SampleState::Unread {
        ui.text_primary
    } else {
        ui.text_secondary
    };
    let lit = r.selected || r.ring;
    let ring_bg = if lit { ui.bg_terminal } else { ui.bg_sidebar };
    let icon_el = if r.ghost {
        div().size(px(icon)).flex_none().rounded(px((icon * 0.24).round())).border_1().border_dashed().border_color(ui.accent).into_any_element()
    } else {
        sample_icon(ws, r.program, r.state, icon, ring_bg)
            .children(pick("corner").map(|c| div().absolute().top_0().left_0().size(px(icon)).rounded(px((icon * 0.24).round())).overflow_hidden().child(crate::session_color::corner(c, icon))))
            .into_any_element()
    };
    div()
        .relative()
        .h(px(if two { two_h } else { one_h }))
        .flex_none()
        .ml(px(12.))
        .pl(px(8.))
        .pr(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        // As in the sidebar, the selected row joins the right edge (the terminal's side).
        .map(|d| if r.selected && !r.ring { d.bg(ui.bg_terminal).rounded_l(px(6.)) } else { d.mr(px(6.)).rounded(px(6.)) })
        .when(r.ring, |d| d.bg(ui.bg_terminal).border(px(1.5)).border_color(ui.accent))
        .when(r.ghost, |d| d.bg(ui.accent.opacity(0.08)).border_1().border_dashed().border_color(ui.accent))
        .when(r.closing, |d| d.opacity(0.5))
        .when_some(pick("tint"), |d, c| d.bg(c.opacity(0.14)))
        .when_some(pick("line"), |d, c| d.child(div().absolute().left(px(-5.)).top_0().bottom_0().w(px(2.)).bg(c)))
        .child(icon_el)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .line_height(gpui::relative(1.25))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .children(pick("swatch").map(crate::session_color::swatch))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .when(bold, |d| d.font_weight(FontWeight::SEMIBOLD))
                                .text_color(title_color)
                                .when(r.closing, |d| d.line_through())
                                .child(r.title),
                        ),
                )
                .when(two, |d| {
                    d.child(div().truncate().text_size(px(10.5)).text_color(if lit { ui.text_quiet } else { ui.text_tertiary }).child(r.sub.unwrap_or_default()))
                }),
        )
        .children(r.closing.then(|| div().flex_none().text_size(px(11.)).text_color(ui.text_tertiary).child("×")))
        .children(r.tag.map(|(t, c, mono)| div().flex_none().text_size(px(10.5)).when(mono, |d| d.font_family("JetBrains Mono")).text_color(c).child(t)))
        .into_any_element()
}

/// A preview sidebar: group headers (20 pt, 11/600), the group line at x 7, rows. Two-line
/// rows when any row has a second line, as the sidebar keeps every row the same height.
fn mini_sidebar(ws: &Workspace, groups: Vec<Group>) -> AnyElement {
    let ui = ws.ui;
    let two = groups.iter().flat_map(|g| &g.rows).any(|r| r.sub.is_some());
    let a = if ws.theme.is_dark() { 0.5 } else { 0.55 };
    let mut col = div().flex().flex_col();
    for (n, g) in groups.into_iter().enumerate() {
        let mut rows = div().relative().flex().flex_col().gap(px(2.));
        if let Some(c) = group_hue(ws, g.label) {
            rows = rows.child(div().absolute().left(px(7.)).top_0().bottom_0().w(px(2.)).rounded(px(1.)).bg(c.opacity(a)));
        }
        for r in g.rows {
            rows = rows.child(mini_row(ws, r, two));
        }
        col = col
            .child(
                div()
                    .h(px(20.))
                    .when(n > 0, |d| d.mt(px(6.)))
                    .pl(px(20.))
                    .flex()
                    .items_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui.text_primary)
                    .child(g.label),
            )
            .child(rows);
    }
    col.into_any_element()
}

/// The first five samples (t1k and infra-terraform) as the sidebar would show them with the
/// current settings, ⌥ included: one-line rows swap the title for the status; two-line rows
/// keep the title and swap the second line (status for the facts, branch or folder for the
/// status), as `sidebar::row` does.
fn sample_groups(ws: &Workspace, plain: bool) -> Vec<Group> {
    let c = &ws.config.sidebar;
    let second = if plain { "none" } else { c.second_line.as_str() };
    let two = second != "none";
    let mut groups: Vec<Group> = Vec::new();
    for (i, s) in SAMPLES.iter().enumerate().take(5) {
        let title = if plain {
            s.title.to_string()
        } else {
            match c.title.as_str() {
                "command" => s.command.to_string(),
                "folder" => s.group.to_string(),
                "branch" => s.branch.to_string(),
                _ => s.title.to_string(),
            }
        };
        let sub = match second {
            "status" => Some(s.status.to_string()),
            "branch" => Some(s.branch.to_string()),
            "folder" => Some(format!("~/git-repos/{}", s.group)),
            _ => None,
        };
        let (title, sub) = match (ws.alt_held && !plain, two) {
            (true, false) => (s.status.to_string(), None),
            (true, true) => (title, Some(if second == "status" { s.facts.to_string() } else { s.status.to_string() })),
            _ => (title, sub),
        };
        let quiet = ws.alt_held && !plain && !two;
        let row = Mini { title, sub, quiet, colored: i == CURRENT, selected: i == CURRENT, ..Mini::of(s) };
        match groups.last_mut() {
            Some(g) if g.label == s.group => g.rows.push(row),
            _ => groups.push(Group { label: s.group, rows: vec![row] }),
        }
    }
    groups
}

/// The T1K icon the user picked (Settings › Interface), for the banner and the Dock.
fn app_icon(ws: &Workspace, size: f32) -> impl IntoElement {
    static ICONS: LazyLock<[Arc<gpui::Image>; 3]> = LazyLock::new(|| {
        let img = |b: &'static [u8]| Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, b.to_vec()));
        [
            img(include_bytes!("../../../resources/icon-256.png")),
            img(crate::platform::dock_icon_png("prompt", false).unwrap_or_default()),
            img(crate::platform::dock_icon_png("wordmark", false).unwrap_or_default()),
        ]
    });
    let i = match ws.config.interface.app_icon.as_str() {
        "prompt" => 1,
        "wordmark" => 2,
        _ => 0,
    };
    img(ICONS[i].clone()).size(px(size)).flex_none()
}

// ---- steps ------------------------------------------------------------------------------------

type Step = (Vec<AnyElement>, Option<AnyElement>);

fn agents_step(ws: &Workspace, cx: &mut Context<Workspace>) -> Step {
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
    let preview = frame(ui, 12., mini_sidebar(ws, sample_groups(ws, false)), "Status comes from agent hooks. A switch backs up the settings first.");
    (rows, Some(preview))
}

fn look_step(ws: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Step {
    let ui = ws.ui;
    let c = &ws.config;
    let mut rows: Vec<AnyElement> = crate::match_ui::setup_offer(ws, cx).into_iter().collect();
    rows.push(stacked(ui, "Appearance", None, seg(ui, "setup-mode", &c.appearance, APPEARANCE, &["appearance"], cx)));
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
    let h = FRAME_H - 2. - 12. - CAPTION_H - 10.;
    let term = div().px(px(10.)).child(crate::match_ui::preview(ws.theme, &f.family, f.size, f.line_height, FRAME_W - 22., h));
    (rows, Some(frame(ui, 12., term, "Every tab, open and new, changes as you pick.")))
}

fn sidebar_step(ws: &Workspace, cx: &mut Context<Workspace>) -> Step {
    let ui = ws.ui;
    let c = &ws.config;
    let rows = vec![
        touched(DENSITY_KEY, stacked(ui, "Density", None, seg(ui, "setup-density", &c.interface.density, DENSITY, DENSITY_KEY, cx)), cx),
        touched(SECOND_KEY, stacked(ui, "Second line", None, seg(ui, "setup-second", &c.sidebar.second_line, SECOND_LINE, SECOND_KEY, cx)), cx),
        touched(TITLE_KEY, stacked(ui, "Titles", None, seg(ui, "setup-title", &c.sidebar.title, TITLE, TITLE_KEY, cx)), cx),
        touched(GCOLORS_KEY, stacked(ui, "Group colors", None, seg(ui, "setup-gcolors", &c.sidebar.group_colors, GROUP_COLORS, GCOLORS_KEY, cx)), cx),
        touched(
            SCOLOR_KEY,
            stacked(ui, "Claude /color", Some("Type /color red in a Claude session to mark its tab."), seg(ui, "setup-scolor", &c.sidebar.session_color, SESSION_COLOR, SCOLOR_KEY, cx)),
            cx,
        ),
    ];
    let s = &c.sidebar;
    let caption = match ws.setup.last_touched(&[DENSITY_KEY, SECOND_KEY, TITLE_KEY, GCOLORS_KEY, SCOLOR_KEY]) {
        Some(DENSITY_KEY) => match c.interface.density.as_str() {
            "compact" => "Compact: shorter rows, so more tabs fit.",
            "spacious" => "Spacious: more room around each row.",
            _ => "Comfortable: the default spacing.",
        }
        .to_string(),
        Some(TITLE_KEY) => match s.title.as_str() {
            "command" => "Titles: the command running in each tab.",
            "folder" => "Titles: the folder each tab is in.",
            "branch" => "Titles: each tab's git branch.",
            _ => "Titles: Auto. Agent sessions show their title, shells what's running.",
        }
        .to_string(),
        Some(GCOLORS_KEY) if s.group_colors == "manual" => "Group lines show only for colors you pick.".into(),
        Some(GCOLORS_KEY) => "Each group gets a color for its line.".into(),
        Some(SCOLOR_KEY) if s.session_color == "off" => "Off: a session's /color isn't shown.".into(),
        Some(SCOLOR_KEY) => format!("Link hint mode has /color orange, shown as a {}.", s.session_color),
        _ => match s.second_line.as_str() {
            "status" => "Second line: status. Each row shows what its agent is doing.",
            "branch" => "Second line: branch. Each row shows its git branch.",
            "folder" => "Second line: folder. Each row shows where it runs.",
            _ => "No second line: one line per tab. Hold ⌥ to see each status.",
        }
        .to_string(),
    };
    (rows, Some(frame(ui, 12., mini_sidebar(ws, sample_groups(ws, false)), caption)))
}

fn tabs_step(ws: &Workspace, cx: &mut Context<Workspace>) -> Step {
    let ui = ws.ui;
    let c = &ws.config;
    let mut rows = vec![
        touched(NEW_TAB_KEY, stacked(ui, "New tabs open", None, seg(ui, "setup-newtab", &c.tabs.new_tab, NEW_TAB, NEW_TAB_KEY, cx)), cx),
        touched(AFTER_CLOSE_KEY, stacked(ui, "After closing a tab, go to", None, seg(ui, "setup-afterclose", &c.tabs.after_close, AFTER_CLOSE, AFTER_CLOSE_KEY, cx)), cx),
        touched(
            STAY_KEY,
            line(
                ui,
                "Stay in the group",
                Some("Closing a group's last tab picks a tab from the same group, not the next group.".into()),
                toggle(ui, "setup-stay", c.tabs.close_stays_in_group, STAY_KEY, cx),
            ),
            cx,
        ),
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
    // The preview follows the control used last; the restore switches don't change it.
    let mut groups = sample_groups(ws, true);
    let caption = if ws.setup.last_touched(&[NEW_TAB_KEY, AFTER_CLOSE_KEY, STAY_KEY]) == Some(NEW_TAB_KEY) {
        // Where ⌘T puts a tab, beside the current one (Link hint mode, first in t1k).
        let (at, caption) = match c.tabs.new_tab.as_str() {
            "before" => (0, "New tabs open right before the current one."),
            "after" => (1, "New tabs open right after the current one."),
            _ => (groups[0].rows.len(), "New tabs open at the end of the current group."),
        };
        let ghost = Mini { title: "New tab".into(), ghost: true, ..Mini::of(&SAMPLES[CURRENT]) };
        groups[0].rows.insert(at, ghost);
        // The extra row would push Plan staging half under the caption.
        groups[1].rows.truncate(1);
        caption.to_string()
    } else {
        // cargo watch, the last tab in t1k, closes. Below with Stay off goes on to the next
        // group; Stay (or Above) keeps to t1k.
        let stay = c.tabs.close_stays_in_group;
        let up = stay || c.tabs.after_close == "above";
        for g in &mut groups {
            for r in &mut g.rows {
                r.selected = false;
            }
        }
        groups[0].rows[2].closing = true;
        let dest = if up { &mut groups[0].rows[1] } else { &mut groups[1].rows[0] };
        dest.ring = true;
        dest.tag = Some(("next".into(), ui.accent, false));
        let name = dest.title.clone();
        format!("Closing cargo watch, the last tab in t1k, goes to {name}.{}", if stay { " Stays in t1k." } else { "" })
    };
    (rows, Some(frame(ui, 12., mini_sidebar(ws, groups), caption)))
}

fn attention_step(ws: &Workspace, cx: &mut Context<Workspace>) -> Step {
    let ui = ws.ui;
    let n = &ws.config.notify;
    let jump = match key_of(ws, "jump_to_needs") {
        Some(k) => format!("{k} jumps to the next tab that needs you, oldest first."),
        None => "Jumping to the next tab that needs you has no shortcut; Settings › Keybindings sets one.".into(),
    };
    let rows = vec![
        touched(NEEDS_KEY, line(ui, "Notify when a tab needs you", None, toggle(ui, "setup-n-needs", n.needs_input, NEEDS_KEY, cx)), cx),
        touched(ERROR_KEY, line(ui, "Notify on errors", None, toggle(ui, "setup-n-err", n.error, ERROR_KEY, cx)), cx),
        touched(
            REMINDER_KEY,
            line(ui, "Notify when a reminder is due", Some("Right-click a tab › Remind me… to come back to it later.".into()), toggle(ui, "setup-n-rem", n.reminder, REMINDER_KEY, cx)),
            cx,
        ),
        touched(FINISHED_KEY, line(ui, "Notify when an agent finishes", None, toggle(ui, "setup-n-done", n.done, FINISHED_KEY, cx)), cx),
        touched(BADGE_KEY, line(ui, "Dock badge", Some("Counts tabs that need you".into()), toggle(ui, "setup-n-badge", n.dock_badge, BADGE_KEY, cx)), cx),
        div().pt(px(10.)).border_t_1().border_color(ui.border).text_size(px(12.)).text_color(ui.text_secondary).child(jump).into_any_element(),
    ];
    // The banner shows the kind toggled last.
    let kinds = [NEEDS_KEY, ERROR_KEY, REMINDER_KEY, FINISHED_KEY];
    let kind = ws.setup.last_touched(&kinds).unwrap_or(NEEDS_KEY);
    let (title, reason, on) = match kind {
        ERROR_KEY => ("Checkout flake", "Error: 529 overloaded", n.error),
        REMINDER_KEY => ("Release notes draft", "Reminder: check the PR comments", n.reminder),
        FINISHED_KEY => ("Migrate charts to v5", "Done: PR opened, CI green", n.done),
        _ => ("Upgrade EKS to 1.31", "Needs you: approve terraform apply", n.needs_input),
    };
    let banner = div()
        .p(px(10.))
        .rounded(px(12.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_md()
        .flex()
        .items_start()
        .gap(px(9.))
        .when(!on, |d| d.opacity(0.35))
        .child(app_icon(ws, 22.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .flex()
                        .text_size(px(10.5))
                        .text_color(ui.text_tertiary)
                        .child(div().flex_1().child("T1K"))
                        .child("now"),
                )
                .child(div().truncate().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(title))
                .child(div().truncate().text_size(px(11.5)).text_color(ui.text_secondary).child(reason)),
        );
    // A slice of the Dock: T1K between two neighbors, with the count when the badge is on.
    let tile = |c: u32| div().size(px(34.)).flex_none().rounded(px(8.)).bg(gpui::rgb(c));
    let slot = |icon: AnyElement, running: bool| {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(3.))
            .child(icon)
            .child(div().size(px(4.)).rounded_full().when(running, |d| d.bg(ui.text_tertiary)))
    };
    let t1k = div()
        .relative()
        .size(px(34.))
        .child(app_icon(ws, 34.))
        .when(n.dock_badge, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(-5.))
                    .right(px(-5.))
                    .size(px(16.))
                    .rounded_full()
                    .bg(ui.status_error)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(gpui::white())
                    .child("2"),
            )
        });
    let dock = div()
        .h(px(52.))
        .rounded(px(14.))
        .bg(ui.bg_control.opacity(0.6))
        .border_1()
        .border_color(ui.border)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .pt(px(4.))
        .child(slot(tile(ws.theme.ansi[5]).into_any_element(), false))
        .child(slot(t1k.into_any_element(), true))
        .child(slot(tile(ws.theme.ansi[4]).into_any_element(), false));
    let content = div()
        .h(px(FRAME_H - 2. - CAPTION_H))
        .p(px(10.))
        .flex()
        .flex_col()
        .justify_between()
        .child(banner)
        .child(dock);
    let badge_last = ws.setup.last_touched(&[NEEDS_KEY, ERROR_KEY, REMINDER_KEY, FINISHED_KEY, BADGE_KEY]) == Some(BADGE_KEY);
    let caption = if badge_last && !n.dock_badge {
        "Dock badge off: the count shows only inside T1K."
    } else if !on {
        "Off for this kind: the tab still turns amber, with no banner."
    } else {
        "Banners show while you're in another app."
    };
    (rows, Some(frame(ui, 0., content, caption)))
}

fn helper_step(ws: &Workspace, cx: &mut Context<Workspace>) -> Step {
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
        stacked(ui, "Helper", Some("Uses an agent CLI you already have"), seg(ui, "setup-helper", &h.backend, &opts, &["helper", "backend"], cx)),
        line(ui, "Title tabs that have none", Some("Codex, Gemini and Grok. Claude names its own.".into()), toggle(ui, "setup-h-titles", h.titles, &["helper", "titles"], cx)),
        line(ui, "Fill in missing recaps", None, toggle(ui, "setup-h-recaps", h.recaps, &["helper", "recaps"], cx)),
        line(ui, search, Some("Ranks tabs by meaning, not only words".into()), toggle(ui, "setup-h-search", h.search, &["helper", "search"], cx)),
        div()
            .pt(px(10.))
            .border_t_1()
            .border_color(ui.border)
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(ui.text_tertiary)
            .child("About 700 tokens per call, roughly $0.0001 per title with Claude. Every call is logged to helper.jsonl.")
            .into_any_element(),
    ];
    // The same three rows either way: Claude names its own sessions and writes recaps, so its
    // title never depends on the helper; Codex and Gemini get theirs from it.
    let on = h.backend().is_some();
    let (titles, recaps) = (on && h.titles, on && h.recaps);
    let row = |title: &str, sub: &str, program: &'static str, state: SampleState| Mini {
        title: title.into(),
        sub: Some(sub.into()),
        program: Some(program),
        state,
        ..Mini::of(&SAMPLES[CURRENT])
    };
    let claude = row("Fix flaky checkout test", "Recap: retries removed, tests pass", "claude", SampleState::Done);
    let codex = Mini {
        selected: true,
        ..row(if titles { "Migrate charts to v5" } else { "codex" }, if recaps { "Recap: switched to the v5 API" } else { "Done" }, "codex", SampleState::Done)
    };
    let gemini = row(if titles { "Explain the cache layer" } else { "gemini" }, "Working", "gemini", SampleState::Working);
    let note = |text: String, names: &[&str]| {
        let marks: Vec<(Range<usize>, HighlightStyle)> = names
            .iter()
            .filter_map(|n| text.find(n).map(|at| (at..at + n.len(), HighlightStyle { color: Some(ui.text_secondary), ..Default::default() })))
            .collect();
        div().text_size(px(10.5)).line_height(px(13.)).text_color(ui.text_tertiary).child(StyledText::new(text).with_highlights(marks))
    };
    let content = div()
        .flex()
        .flex_col()
        .child(mini_sidebar(ws, vec![Group { label: "web-dashboard", rows: vec![claude, codex, gemini] }]))
        .child(
            div()
                .px(px(12.))
                .pt(px(6.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(note("Claude names its own sessions and writes recaps.".into(), &["Claude"]))
                .child(note(
                    if titles { "The helper titled Codex, Gemini." } else { "Codex, Gemini stay untitled." }.into(),
                    &["Codex", "Gemini"],
                )),
        );
    let caption = if on { "Helper on: Codex and Gemini tabs get titles and recaps too." } else { "Helper off: Claude tabs still have titles; others show the program name." };
    (rows, Some(frame(ui, 12., content, caption)))
}

fn keys_step(ws: &Workspace, cx: &mut Context<Workspace>) -> Step {
    let ui = ws.ui;
    let st = &ws.setup;
    let mut rows = Vec::new();
    for t in Try::ALL {
        let keys = match t.bind() {
            Some(id) => key_of(ws, id).unwrap_or_else(|| "—".into()),
            None => "⌥".into(),
        };
        let tried = st.tried.contains(&t);
        // ⌥ can't be tried by a click: after Try it, the row asks for the key.
        let waiting = t == Try::Alt && st.alt_hint && !tried;
        let end = if tried {
            div().flex_none().text_size(px(11.5)).text_color(ui.accent).child("✓ Tried").into_any_element()
        } else if waiting {
            div().flex_none().px(px(6.)).text_size(px(11.5)).text_color(ui.text_secondary).child("Press it").into_any_element()
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
                .when(waiting, |d| d.bg(ui.bg_hover))
                .child(
                    div().min_w(px(44.)).h(px(24.)).px(px(7.)).flex_none().rounded(px(5.)).bg(ui.bg_control).flex().items_center().justify_center()
                        .text_size(px(12.)).text_color(ui.text_primary).child(keys),
                )
                .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(ui.text_primary).child(t.label()))
                .child(end)
                .into_any_element(),
        );
    }

    let jump = key_of(ws, "jump_to_needs").unwrap_or_else(|| "⌘J".into());
    let split_key = key_of(ws, "split_right").unwrap_or_else(|| "⌘D".into());
    let header = |label: &'static str| {
        div().px(px(12.)).pb(px(8.)).flex().items_center().gap(px(6.)).child(example_tag(ui)).child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(label))
    };
    let (content, caption) = if st.split && !ws.alt_held {
        (div().flex().flex_col().child(header("Sample tab")).child(split_preview(ws)).into_any_element(),format!("{split_key} split this sample tab. Yours get a second shell beside the first."))
    } else {
        let mut groups = sample_groups(ws, false);
        // Two-line rows don't leave room for all five above the caption: drop the shells so
        // the tab that waits (⌘J's lesson) stays in view.
        if groups.iter().flat_map(|g| &g.rows).any(|r| r.sub.is_some()) {
            for g in &mut groups {
                g.rows.retain(|r| !matches!(r.state, SampleState::Running | SampleState::Shell));
            }
        }
        let waiting = &SAMPLES[WAITING];
        for g in &mut groups {
            for r in &mut g.rows {
                if r.state == SampleState::Needs {
                    r.tag = Some((waiting.age.into(), ui.status_needs, true));
                }
                if st.jumped {
                    r.selected = false;
                }
            }
        }
        if st.jumped {
            // Upgrade EKS to 1.31: the first row of infra-terraform.
            let r = &mut groups[1].rows[0];
            r.selected = true;
            r.ring = true;
        }
        let caption = if ws.alt_held {
            "Holding ⌥: each row shows what its tab is doing.".to_string()
        } else if st.alt_hint && !st.tried.contains(&Try::Alt) {
            "Hold ⌥ now and watch these rows, then let go.".to_string()
        } else if st.jumped {
            format!("Jumped to {}, waiting {}. Press again for the next one.", waiting.title, waiting.age)
        } else {
            format!("Press {jump}. It jumps to the tab that has waited longest for you.")
        };
        (div().flex().flex_col().child(header("Sample tabs")).child(mini_sidebar(ws, groups)).into_any_element(), caption)
    };
    (rows, Some(frame(ui, 12., content, caption)))
}

/// ⌘D on the Keys step: a sample terminal split in two, in the user's terminal font and theme.
fn split_preview(ws: &Workspace) -> AnyElement {
    let t = ws.theme;
    let ui = ws.ui;
    let pane = |lines: Vec<(&'static str, u32)>| {
        div().flex_1().min_w_0().p(px(8.)).flex().flex_col().children(lines.into_iter().map(|(s, c)| div().truncate().text_color(gpui::rgb(c)).child(s)))
    };
    div()
        .mx(px(10.))
        .h(px(FRAME_H - 2. - 12. - 26. - CAPTION_H - 10.))
        .rounded(px(6.))
        .overflow_hidden()
        .bg(gpui::rgb(t.bg))
        .border_1()
        .border_color(ui.border)
        .flex()
        .font_family(SharedString::from(ws.config.terminal.font.family.clone()))
        .text_size(px(10.5))
        .line_height(px(14.))
        .child(pane(vec![("❯ cargo test", t.fg), ("  48 passed", t.ansi[2]), ("❯", t.ansi[5])]))
        .child(div().w(px(1.)).h_full().bg(ui.border))
        .child(pane(vec![("❯ git log -3", t.fg), ("a41c2e9 hints", t.ansi[3]), ("9b2f10d links", t.ansi[3]), ("❯", t.ansi[5])]))
        .into_any_element()
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
        _ => (done_page(ws, cx), None),
    };
    let body = div()
        .flex_1()
        .min_h_0()
        .flex()
        .gap(px(18.))
        .child(div().flex_1().min_w_0().flex().flex_col().children(choices))
        .children(preview.map(|p| div().w(px(FRAME_W)).flex_none().child(p)));

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
        .h(px(520.))
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

    #[test]
    fn previews_follow_the_control_used_last() {
        let mut s = SetupState::new(None);
        assert_eq!(s.last_touched(&[NEW_TAB_KEY, STAY_KEY]), None);
        s.touch(NEW_TAB_KEY);
        s.touch(BADGE_KEY);
        s.touch(STAY_KEY);
        assert_eq!(s.last_touched(&[NEW_TAB_KEY, STAY_KEY]), Some(STAY_KEY));
        // Using a control again makes it the latest.
        s.touch(NEW_TAB_KEY);
        assert_eq!(s.last_touched(&[NEW_TAB_KEY, STAY_KEY]), Some(NEW_TAB_KEY));
        assert_eq!(s.last_touched(&[BADGE_KEY, NEEDS_KEY]), Some(BADGE_KEY));
    }

    #[test]
    fn samples_lead_with_what_needs_you() {
        let all = sample_results("", SwitcherFilter::All);
        assert_eq!(all.len(), SAMPLES.len());
        assert_eq!(SAMPLES[all[0]].title, "Upgrade EKS to 1.31");
        // Agent sessions before shells.
        let first_shell = all.iter().position(|&i| !SAMPLES[i].agent()).unwrap();
        assert!(all[first_shell..].iter().all(|&i| !SAMPLES[i].agent()));
        // Fuzzy, like the real switcher: what needs you leads.
        assert_eq!(sample_results("eks", SwitcherFilter::All).first(), Some(&WAITING));
        assert_eq!(sample_count(SwitcherFilter::NeedsYou), 1);
        assert_eq!(sample_count(SwitcherFilter::Project), 3);
        let cards: Vec<&str> = sample_cards().into_iter().map(|i| SAMPLES[i].title).collect();
        assert_eq!(cards, ["Upgrade EKS to 1.31", "Link hint mode", "Dedupe date utils", "Migrate charts to v5"]);
        assert_eq!(highlights("Upgrade EKS to 1.31", "eks"), vec![8..11]);
    }

    #[test]
    fn sub_lines_fit_their_rows() {
        // Every copy that sits beside a switch on one line must fit ONE_LINE.
        for d in ["Scrollback comes back after a restart", "Sessions running at quit continue", "Counts tabs that need you", "Ranks tabs by meaning, not only words"] {
            assert!(d.chars().count() <= ONE_LINE, "{d}");
        }
    }
}
