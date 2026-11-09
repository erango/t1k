//! Right-hand panel listing what sessions produced: PRs (with live state), artifacts, docs,
//! tickets, deploys and edited files. Scope: this tab, its project, or everything.

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use gpui::{div, prelude::*, px, AnyElement, ClipboardItem, Context, Div, FontWeight, Hsla, SharedString};
use t1k_agent::outputs::{Kind, Output};
use t1k_themes::UiColors;

use crate::workspace::Workspace;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    Tab,
    Project,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrInfo {
    pub state: String,
    pub draft: bool,
    pub title: String,
    /// (passed, failed, pending)
    pub checks: (u32, u32, u32),
    /// `reviewDecision`: APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, or empty when the
    /// repo requires no review.
    pub decision: String,
    /// Logins whose latest review approved / requested changes. Comment-only reviews say
    /// nothing about where the PR stands, so they're left out.
    pub approvers: Vec<String>,
    pub changes: Vec<String>,
    /// When it merged (or closed unmerged); None while open or when gh didn't say.
    pub ended: Option<SystemTime>,
    pub fetched: Instant,
}

/// Where CI stands, worst first: a failing check is what needs a person.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Checks {
    None,
    Failing(u32),
    Running,
    Passing(u32),
}

/// Where review stands, in the card's words.
#[derive(Clone, Debug, PartialEq)]
pub enum Review {
    None,
    Approved(String),
    Changes(String),
    Needed,
}

impl PrInfo {
    /// Merged or closed: nothing left to check or review.
    pub fn finished(&self) -> bool {
        matches!(self.state.as_str(), "MERGED" | "CLOSED")
    }

    pub fn ci(&self) -> Checks {
        match self.checks {
            (_, bad, _) if bad > 0 => Checks::Failing(bad),
            (_, _, wait) if wait > 0 => Checks::Running,
            (ok, _, _) if ok > 0 => Checks::Passing(ok),
            _ => Checks::None,
        }
    }

    pub fn review(&self) -> Review {
        let by = |who: &[String], one: &str, many: String| match who {
            [] => one.to_string(),
            [a] => format!("{one} by {a}"),
            _ => many,
        };
        if self.decision == "CHANGES_REQUESTED" || !self.changes.is_empty() {
            return Review::Changes(by(&self.changes, "Changes requested", format!("Changes requested by {} people", self.changes.len())));
        }
        // Without a required review the decision stays empty, but an approval still says something.
        if self.decision == "APPROVED" || (self.decision.is_empty() && !self.approvers.is_empty()) {
            return Review::Approved(by(&self.approvers, "Approved", format!("{} approvals", self.approvers.len())));
        }
        if self.decision == "REVIEW_REQUIRED" {
            return Review::Needed;
        }
        Review::None
    }
}

#[derive(Default)]
pub struct PanelState {
    pub open: bool,
    pub scope: Scope,
    pub filter: Option<Kind>,
    /// The folded "N artifact links" row is open.
    pub links_open: bool,
    /// Sections showing everything, past the limit (by section label).
    pub expanded: HashSet<&'static str>,
    /// FILES lists the files that are gone from disk (canvas v3-at A).
    pub show_deleted: bool,
    pub prs: HashMap<String, Option<PrInfo>>,
    pub(crate) inflight: HashSet<String>,
    /// PR info faked by `T1K_DEBUG_PRS`: gh is never asked, so it isn't overwritten.
    pub(crate) fake: bool,
}

const PR_TTL: Duration = Duration::from_secs(300);

/// `gh pr view` → state, title, CI and review. None if gh is missing or the PR isn't
/// reachable. Still one request per PR.
fn fetch_pr(url: &str) -> Option<PrInfo> {
    let out = Command::new("gh")
        .env("PATH", crate::platform::tool_path())
        .args(["pr", "view", url, "--json", "state,isDraft,statusCheckRollup,title,reviewDecision,latestReviews,mergedAt,closedAt"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    parse_pr(&out.stdout)
}

/// The JSON `fetch_pr` asks gh for, as a `PrInfo`.
fn parse_pr(json: &[u8]) -> Option<PrInfo> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let mut checks = (0, 0, 0);
    for c in v["statusCheckRollup"].as_array().into_iter().flatten() {
        // Check runs report "" as the conclusion while they run; status contexts use `state`.
        let concl = c["conclusion"].as_str().filter(|s| !s.is_empty()).or(c["state"].as_str()).unwrap_or("");
        match concl {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => checks.0 += 1,
            "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" => checks.1 += 1,
            _ => checks.2 += 1,
        }
    }
    let reviewers = |state: &str| -> Vec<String> {
        v["latestReviews"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["state"].as_str() == Some(state))
            .filter_map(|r| r["author"]["login"].as_str().map(str::to_string))
            .collect()
    };
    let state = v["state"].as_str().unwrap_or("").to_string();
    let ended = match state.as_str() {
        "MERGED" => v["mergedAt"].as_str().and_then(parse_time),
        "CLOSED" => v["closedAt"].as_str().and_then(parse_time),
        _ => None,
    };
    Some(PrInfo {
        state,
        draft: v["isDraft"].as_bool().unwrap_or(false),
        title: v["title"].as_str().unwrap_or("").trim().to_string(),
        checks,
        decision: v["reviewDecision"].as_str().unwrap_or("").to_string(),
        approvers: reviewers("APPROVED"),
        changes: reviewers("CHANGES_REQUESTED"),
        ended,
        fetched: Instant::now(),
    })
}

/// GitHub's UTC timestamps ("2026-11-08T09:30:00Z"), without pulling in a date crate.
fn parse_time(s: &str) -> Option<SystemTime> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let secs = (era * 146097 + doe - 719468) * 86400 + h * 3600 + mi * 60 + sec;
    u64::try_from(secs).ok().map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s))
}

impl Workspace {
    /// A tab's newest PR that's still open, with its title, for ⌘P. Merged and closed PRs
    /// are history there, and one gh hasn't answered for yet has no title to show.
    pub fn open_pr(&self, tab: usize) -> Option<(u64, String)> {
        let a = self.tabs.get(tab)?.agent.as_ref()?;
        a.prs.iter().rev().find_map(|(n, url)| {
            let info = self.panel.prs.get(url)?.as_ref()?;
            (!info.finished() && !info.title.is_empty()).then(|| (*n, info.title.clone()))
        })
    }

    /// Outputs in scope, newest first, with the tab they came from.
    pub fn outputs_in_scope(&self) -> Vec<(usize, Output)> {
        let active_group = self.tabs.get(self.active).map(|t| t.group.clone());
        let mut v: Vec<(usize, Output)> = Vec::new();
        for (i, t) in self.tabs.iter().enumerate() {
            let in_scope = match self.panel.scope {
                Scope::Tab => i == self.active,
                Scope::Project => Some(&t.group) == active_group.as_ref(),
                Scope::All => true,
            };
            if !in_scope {
                continue;
            }
            for o in t.agent.iter().flat_map(|a| a.outputs.iter()) {
                if !v.iter().any(|(_, x)| x.target == o.target) {
                    v.push((i, o.clone()));
                }
            }
        }
        v.sort_by(|a, b| b.1.at.cmp(&a.1.at));
        v
    }

    /// Fetch PR state for PRs in view (the panel's scope, or the active tab for the session
    /// strip) that we don't know yet or that went stale.
    pub fn refresh_prs(&mut self, cx: &mut Context<Self>) {
        if self.panel.fake {
            return;
        }
        let mut candidates: Vec<(usize, Output)> = if self.panel.open {
            self.outputs_in_scope()
        } else {
            let active = self.active;
            self.tabs
                .get(active)
                .and_then(|t| t.agent.as_ref())
                .map(|a| a.outputs.iter().filter(|o| o.kind == Kind::Pr).map(|o| (active, o.clone())).collect())
                .unwrap_or_default()
        };
        // ⌘P shows each tab's newest PR by title, so it needs every tab's, not just the active one's.
        if matches!(self.overlay, crate::overlay::Overlay::Switcher { .. }) {
            for (i, t) in self.tabs.iter().enumerate() {
                if let Some(o) = t.agent.iter().flat_map(|a| a.outputs.iter()).rev().find(|o| o.kind == Kind::Pr) {
                    candidates.push((i, o.clone()));
                }
            }
        }
        let stale: Vec<String> = candidates
            .into_iter()
            .filter(|(_, o)| o.kind == Kind::Pr)
            .map(|(_, o)| o.target)
            .filter(|u| !self.panel.inflight.contains(u))
            .filter({
                let mut seen = HashSet::new();
                move |u| seen.insert(u.clone())
            })
            .filter(|u| match self.panel.prs.get(u) {
                None => true,
                Some(Some(info)) => info.fetched.elapsed() > PR_TTL,
                // Failed lookups aren't retried until restart.
                Some(None) => false,
            })
            .take(6)
            .collect();
        for url in stale {
            self.panel.inflight.insert(url.clone());
            let task = cx.background_spawn({
                let url = url.clone();
                async move { fetch_pr(&url) }
            });
            cx.spawn(async move |this, cx| {
                let info = task.await;
                let _ = this.update(cx, |ws, cx| {
                    ws.panel.inflight.remove(&url);
                    ws.panel.prs.insert(url, info);
                    cx.notify();
                });
            })
            .detach();
        }
    }
}

/// Short text that identifies an output in terminal output: what Claude most likely printed.
fn needle(o: &Output) -> String {
    let path = o.target.split(['?', '#']).next().unwrap_or(&o.target);
    match o.kind {
        // ".../pull/12" matches full URLs; Claude often prints just the URL.
        Kind::Pr => path.rsplit_once("/pull/").map(|(_, n)| format!("pull/{n}")).unwrap_or_else(|| o.label.clone()),
        Kind::File | Kind::Ticket => o.label.clone(),
        _ => path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).to_string(),
    }
}

fn age(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

fn icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Pr => "⇄",
        Kind::Artifact => "◫",
        Kind::Doc => "≡",
        Kind::Ticket => "#",
        Kind::Deploy => "▲",
        Kind::File => "✎",
    }
}

/// "3h ago", or "just now".
fn ago(t: SystemTime) -> String {
    match age(t).as_str() {
        "now" => "just now".into(),
        a => format!("{a} ago"),
    }
}

/// GitHub's solid state pill, white on the state color, in every theme.
/// The line box matches the pill (an inherited taller line height sat the text low).
pub fn state_pill(label: &'static str, color: Hsla) -> Div {
    div()
        .flex_none()
        .px(px(7.))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(color)
        .text_size(px(10.5))
        .line_height(px(18.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::white())
        .child(label)
}

/// The amber diamond for changes requested: 7 pt square turned 45°.
fn diamond(color: Hsla) -> Div {
    div().size(px(10.)).flex_none().child(
        gpui::canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let c = bounds.center();
                let r = px(4.95);
                let mut p = gpui::PathBuilder::fill();
                p.move_to(gpui::point(c.x, c.y - r));
                p.line_to(gpui::point(c.x + r, c.y));
                p.line_to(gpui::point(c.x, c.y + r));
                p.line_to(gpui::point(c.x - r, c.y));
                p.close();
                if let Ok(path) = p.build() {
                    window.paint_path(path, color);
                }
            },
        )
        .size_full(),
    )
}

/// CI in a few words: ✓ 6 checks, the working ring and Checks running, ✗ 1 failing.
/// None without checks.
pub fn checks_part(ui: UiColors, info: &PrInfo, started: Instant) -> Option<Div> {
    let row = div().flex_none().flex().items_center().gap(px(5.));
    Some(match info.ci() {
        Checks::None => return None,
        Checks::Passing(n) => row
            .child(div().text_color(ui.status_done).child("✓"))
            .child(div().text_color(ui.text_secondary).child(if n == 1 { "1 check".to_string() } else { format!("{n} checks") })),
        Checks::Running => row.child(div().size(px(10.)).flex_none().child(crate::sidebar::spinner(ui.status_working, started))).child(div().text_color(ui.text_secondary).child("Checks running")),
        // Failing CI needs the person, so it's the one colored check state.
        Checks::Failing(n) => row.child(div().text_color(ui.status_error).child("✗")).child(div().text_color(ui.status_error).child(format!("{n} failing"))),
    })
}

/// Review in a few words, from the decision and the latest reviews. None when no review is
/// required and nobody has reviewed.
pub fn review_part(ui: UiColors, info: &PrInfo) -> Option<Div> {
    let row = div().min_w_0().flex().items_center().gap(px(5.));
    let words = |s: String, c: Hsla| div().min_w_0().truncate().text_color(c).child(s);
    Some(match info.review() {
        Review::None => return None,
        Review::Approved(s) => row.child(div().flex_none().text_color(ui.status_done).child("✓")).child(words(s, ui.text_secondary)),
        Review::Changes(s) => row.child(diamond(ui.status_needs)).child(words(s, ui.status_needs)),
        Review::Needed => row.child(div().flex_none().text_color(ui.text_tertiary).child("○")).child(words("Review needed".into(), ui.text_tertiary)),
    })
}

/// Checks, a quiet dot, then review; the dot only when both are there.
fn status_line(ui: UiColors, info: &PrInfo, started: Instant) -> Option<Div> {
    let checks = checks_part(ui, info, started);
    let review = review_part(ui, info);
    if checks.is_none() && review.is_none() {
        return None;
    }
    let dot = (checks.is_some() && review.is_some()).then(|| div().flex_none().text_color(ui.text_disabled).child("·"));
    Some(div().min_w_0().flex().items_center().gap(px(6.)).children(checks).children(dot).children(review))
}

/// The title-bar PR chip's hover card: number and title, the state with checks, review.
pub struct PrTip {
    pub ui: UiColors,
    pub number: u64,
    pub info: PrInfo,
    pub started: Instant,
}

impl Render for PrTip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        let ui = self.ui;
        let info = &self.info;
        let (state, color) = crate::github::pr_state(&info.state, info.draft);
        let number = format!("#{}", self.number);
        let head = if info.title.is_empty() { number.clone() } else { format!("{number} · {}", info.title) };
        let font = |family: &'static str| gpui::Font { weight: FontWeight::SEMIBOLD, ..gpui::font(family) };
        let run = |len: usize, family: &'static str| gpui::TextRun { len, font: font(family), color: ui.text_primary, background_color: None, underline: None, strikethrough: None };
        let mut runs = vec![run(number.len(), "JetBrains Mono")];
        if head.len() > number.len() {
            runs.push(run(head.len() - number.len(), ".SystemUIFont"));
        }
        let live = !info.finished();
        div()
            .w(px(300.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(8.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .shadow_md()
            .font_family(".SystemUIFont")
            .text_size(px(12.))
            .line_height(px(17.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().text_size(px(12.5)).child(gpui::StyledText::new(head).with_runs(runs)))
            .child(div().flex().items_center().gap(px(8.)).child(state_pill(state, color)).children(live.then(|| checks_part(ui, info, self.started)).flatten()))
            .children(live.then(|| review_part(ui, info)).flatten())
            .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("Click to open on GitHub"))
    }
}

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let k = ws.config.interface.text_scale();
    let ui = ws.ui;
    let all = ws.outputs_in_scope();
    let items: Vec<&(usize, Output)> = all.iter().filter(|(_, o)| ws.panel.filter.is_none_or(|k| o.kind == k)).collect();

    let seg = |id: &str, label: &str, on: bool| {
        div()
            .id(SharedString::from(format!("scope-{id}")))
            .px(px(8.))
            .h(px(22.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(12. * k))
            .text_color(if on { ui.text_primary } else { ui.text_tertiary })
            .when(on, |d| d.bg(ui.bg_control))
            .hover(move |d| d.text_color(ui.text_primary))
            .child(label.to_string())
    };
    let scope = ws.panel.scope;
    let header = div()
        .h(px(38.))
        .flex_none()
        .pl(px(8.))
        .pr(px(14.))
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            div()
                .id("outputs-hide")
                .size(px(24.))
                .flex_none()
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |d| d.bg(ui.bg_hover))
                .tooltip(crate::tooltip::text(ui, format!("Hide outputs  {}", crate::keymap::display_for(&crate::actions::ToggleOutputs, &ws.config.keys))))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_outputs(cx)))
                .child(crate::icons::collapse_right(ui.text_tertiary)),
        )
        .child(div().flex_1().ml(px(2.)).text_size(px(13. * k)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Outputs"))
        .child(seg("tab", "Tab", scope == Scope::Tab).on_click(cx.listener(|this, _, _, cx| {
            this.panel.scope = Scope::Tab;
            this.refresh_prs(cx);
            cx.notify();
        })))
        .child(seg("project", "Project", scope == Scope::Project).on_click(cx.listener(|this, _, _, cx| {
            this.panel.scope = Scope::Project;
            this.refresh_prs(cx);
            cx.notify();
        })))
        .child(seg("all", "All", scope == Scope::All).on_click(cx.listener(|this, _, _, cx| {
            this.panel.scope = Scope::All;
            this.refresh_prs(cx);
            cx.notify();
        })));

    // Filter chips: only kinds present, with counts.
    let mut chips = div().flex().flex_wrap().gap(px(4.)).px(px(12.)).pb(px(8.));
    let filter = ws.panel.filter;
    chips = chips.child(seg("k-all", &format!("All {}", all.len()), filter.is_none()).on_click(cx.listener(|this, _, _, cx| {
        this.panel.filter = None;
        cx.notify();
    })));
    for k in Kind::ALL {
        let n = all.iter().filter(|(_, o)| o.kind == k).count();
        if n == 0 {
            continue;
        }
        chips = chips.child(
            seg(k.label(), &format!("{} {n}", k.label()), filter == Some(k)).on_click(cx.listener(move |this, _, _, cx| {
                this.panel.filter = if this.panel.filter == Some(k) { None } else { Some(k) };
                cx.notify();
            })),
        );
    }

    let mut list = div()
        .id("outputs-list")
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&ws.outputs_scroll)
        // Scrolling alone doesn't re-render the workspace, and the scrollbar is drawn by it.
        .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify())).px(px(6.)).pb(px(12.)).flex().flex_col();
    // Sections by kind (design): PRs as cards first, then docs/artifacts, tickets, deploys, files.
    let sections: [(&str, &[Kind]); 5] = [
        ("PULL REQUESTS", &[Kind::Pr]),
        ("DOCS + ARTIFACTS", &[Kind::Doc, Kind::Artifact]),
        ("TICKETS", &[Kind::Ticket]),
        ("DEPLOYS", &[Kind::Deploy]),
        ("FILES", &[Kind::File]),
    ];
    // The demo's files are made up: they'd all read as deleted.
    let demo = crate::demo::on();
    let mut n = 0usize;
    for (label, kinds) in sections {
        let in_section: Vec<&&(usize, Output)> = items.iter().filter(|(_, o)| kinds.contains(&o.kind)).collect();
        if in_section.is_empty() {
            continue;
        }
        // Canvas v3-at A: files gone from disk (temp handover notes) leave the list and its
        // count, behind a quiet "N deleted" row at the end.
        let (in_section, deleted): (Vec<&&(usize, Output)>, Vec<&&(usize, Output)>) =
            in_section.into_iter().partition(|x| x.1.kind != Kind::File || demo || std::path::Path::new(&x.1.target).exists());
        // Artifacts only seen as links (not published in the session: canvases it read, types,
        // links in pasted text) say nothing on their own, so they fold into one row at the end.
        let total = in_section.len();
        let link_only = |o: &Output| o.kind == Kind::Artifact && o.icon.is_none();
        let links = in_section.iter().filter(|x| link_only(&x.1)).count();
        let links_open = ws.panel.links_open;
        let in_section: Vec<&&(usize, Output)> = {
            let (mut main, rest): (Vec<_>, Vec<_>) = in_section.into_iter().partition(|x| !link_only(&x.1));
            if links_open || links < 2 {
                main.extend(rest);
            }
            main
        };
        // The newest few per section, then "+N more". Open PRs always show, first; merged and
        // closed ones fill the slots left. One past the limit shows (a "+1" row saves nothing).
        let limit = ws.config.interface.outputs_section_limit.max(1);
        let expanded = ws.panel.expanded.contains(label);
        let is_finished = |o: &Output| o.kind == Kind::Pr && ws.panel.prs.get(&o.target).is_some_and(|p| p.as_ref().is_some_and(|p| p.finished()));
        let (pinned, rest): (Vec<_>, Vec<_>) = in_section.into_iter().partition(|x| x.1.kind == Kind::Pr && !is_finished(&x.1));
        let room = limit.saturating_sub(pinned.len());
        let folds = rest.len() > room + 1;
        let hidden = if folds && !expanded { rest.len() - room } else { 0 };
        let shown = rest.len() - hidden;
        let mut rows: Vec<Row> = pinned.into_iter().chain(rest.into_iter().take(shown)).map(|x| Row::Out(x, false)).collect();
        if folds {
            let more = if label == "PULL REQUESTS" { format!("+{hidden} merged or closed") } else { format!("+{hidden} more") };
            rows.push(Row::Fold { open: expanded, text: if expanded { "Show fewer".into() } else { more } });
        }
        if !deleted.is_empty() {
            rows.push(Row::Deleted { n: deleted.len(), open: ws.panel.show_deleted });
            if ws.panel.show_deleted {
                rows.extend(deleted.into_iter().map(|x| Row::Out(x, true)));
            }
        }
        list = list.child(
            div()
                .h(px(24.))
                .mt(px(6.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(10.5 * k))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui.text_tertiary)
                .child(label)
                .child(div().font_family("JetBrains Mono").text_color(ui.text_disabled).child(total.to_string())),
        );
        for row in rows {
            let (x, gone) = match row {
                Row::Out(x, gone) => (x, gone),
                Row::Fold { open, text } => {
                    list = list.child(fold_row(ui, k, SharedString::from(format!("fold-{label}")), text, open).on_click(cx.listener(move |this, _, _, cx| {
                        if !this.panel.expanded.remove(label) {
                            this.panel.expanded.insert(label);
                        }
                        cx.notify();
                    })));
                    continue;
                }
                Row::Deleted { n, open } => {
                    let text = if open { "Hide deleted".to_string() } else { format!("{n} deleted") };
                    list = list.child(fold_row(ui, k, "deleted-files".into(), text, open).on_click(cx.listener(|this, _, _, cx| {
                        this.panel.show_deleted = !this.panel.show_deleted;
                        cx.notify();
                    })));
                    continue;
                }
            };
            let (tab, o) = (x.0, &x.1);
            n += 1;
            let target = o.target.clone();
            let copy_target = o.target.clone();
            let is_file = o.kind == Kind::File;
            let tab_title = ws.tabs.get(tab).map(|t| t.title()).unwrap_or_default();
            let row_id = SharedString::from(format!("out-{n}"));
            let open = cx.listener(move |this, _, _, cx| {
                if is_file {
                    // Text files open in the viewer, as ⌘-clicked paths do (links.text_files);
                    // closing it comes back here.
                    let path = std::path::Path::new(&target);
                    if this.config.links.text_files != "editor" && this.view_file(path, None, cx) {
                        if let Some(v) = this.viewer.as_mut() {
                            v.from_outputs = true;
                        }
                        return;
                    }
                    cx.open_with_system(path);
                } else {
                    cx.open_url(&target);
                }
            });
            let hover_buttons = {
                let q = needle(o);
                let tab_ix = tab;
                let rid = row_id.clone();
                div()
                    .flex()
                    .gap(px(2.))
                    .invisible()
                    .group_hover(rid, |s| s.visible())
                    .when(!gone, |d| d.child(
                        div()
                            .id(SharedString::from(format!("show-{n}")))
                            .px(px(6.))
                            .h(px(22.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .text_size(px(11. * k))
                            .text_color(ui.text_tertiary)
                            .hover(move |d| d.bg(ui.bg_control).text_color(ui.text_primary))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.activate(tab_ix, window, cx);
                                let view = this.tabs[tab_ix].view.clone();
                                view.update(cx, |v, cx| v.find_text(&q, cx));
                            }))
                            .child("Show"),
                    ))
                    // A link icon copies the URL; a folder, a file's path.
                    .child(
                        div()
                            .id(SharedString::from(format!("copy-{n}")))
                            .size(px(22.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(move |d| d.bg(ui.bg_control))
                            .tooltip(crate::tooltip::text(ui, if is_file { "Copy path" } else { "Copy URL" }.to_string()))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_target.clone()));
                            }))
                            .child(crate::icons::artifact(if is_file { "folder" } else { "chain" }, ui.text_tertiary, 14.)),
                    )
            };
            if o.kind == Kind::Pr && is_finished(o) {
                // Canvas v3-at A: merged and closed PRs are history, one quiet line each.
                let pr = ws.panel.prs.get(&o.target).cloned().flatten();
                let merged = pr.as_ref().is_some_and(|p| p.state == "MERGED");
                let title = pr.as_ref().map(|p| p.title.clone()).unwrap_or_default();
                let when = match pr.as_ref().and_then(|p| p.ended) {
                    Some(t) => format!("{} {}", if merged { "merged" } else { "closed" }, ago(t)),
                    None => age(o.at),
                };
                let rid = row_id.clone();
                list = list.child(
                    div()
                        .id(row_id.clone())
                        .group(row_id.clone())
                        .h(px(34.))
                        .flex_none()
                        .relative()
                        .px(px(10.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .hover(move |d| d.bg(ui.bg_hover))
                        .on_click(open)
                        .child(div().size(px(28.)).flex_none().flex().items_center().justify_center().child(crate::icons::artifact(if merged { "merge" } else { "closed" }, ui.text_tertiary, 16.)))
                        .child(div().flex_none().text_size(px(12. * k)).font_family("JetBrains Mono").text_color(ui.text_secondary).child(format!("#{}", o.label.rsplit('#').next().unwrap_or(&o.label))))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(12.5 * k)).text_color(ui.text_secondary).child(title))
                        .child(div().flex_none().text_size(px(11.5 * k)).text_color(ui.text_tertiary).group_hover(rid, |s| s.invisible()).child(when))
                        .child(hover_buttons.absolute().right(px(6.)).top_0().bottom_0().items_center().pl(px(8.)).bg(ui.bg_hover)),
                );
                continue;
            }
            if o.kind == Kind::Pr {
                // PR card (v3-aj A): number, state and age; title; checks · review; where it
                // came from. Until gh answers (or without gh) it's the number and the tab.
                let pr = ws.panel.prs.get(&o.target).cloned().flatten();
                let (state, state_color) = match pr.as_ref() {
                    Some(p) => crate::github::pr_state(&p.state, p.draft),
                    None => ("…", ui.text_disabled),
                };
                let finished = pr.as_ref().is_some_and(|p| p.finished());
                let ink = if finished { ui.text_secondary } else { ui.text_primary };
                let title = pr.as_ref().map(|p| p.title.clone()).filter(|t| !t.is_empty());
                let status = pr.as_ref().filter(|_| !finished).and_then(|p| status_line(ui, p, ws.started));
                let from = match pr.as_ref().filter(|_| finished) {
                    Some(p) => {
                        let verb = if p.state == "MERGED" { "merged" } else { "closed" };
                        match p.ended {
                            Some(t) => format!("from {tab_title} · {verb} {}", ago(t)),
                            None => format!("from {tab_title} · {}", age(o.at)),
                        }
                    }
                    None => format!("from {tab_title}"),
                };
                let rid = row_id.clone();
                list = list.child(
                    div()
                        .id(row_id.clone())
                        .group(row_id.clone())
                        .mx(px(4.))
                        .mb(px(6.))
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(8.))
                        .bg(ui.bg_raised)
                        .border_1()
                        .border_color(ui.border)
                        .when(finished, |d| d.opacity(0.8))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .hover(move |d| d.border_color(ui.text_disabled))
                        .on_click(open)
                        .child(
                            // Fixed height: the age gives way to Show / Copy on hover without
                            // moving anything.
                            div()
                                .h(px(22.))
                                .relative()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().flex_none().text_size(px(12.5 * k)).font_family("JetBrains Mono").font_weight(FontWeight::SEMIBOLD).text_color(ink).child(o.label.clone()))
                                .child(state_pill(state, state_color))
                                .child(div().flex_1())
                                .child(div().flex_none().text_size(px(11.5 * k)).text_color(ui.text_tertiary).group_hover(rid, |s| s.invisible()).child(age(o.at)))
                                .child(hover_buttons.absolute().right(px(-6.)).top_0()),
                        )
                        .children(title.map(|t| {
                            div()
                                .id(SharedString::from(format!("title-{n}")))
                                .truncate()
                                .text_size(px(13. * k))
                                .line_height(px(18. * k))
                                .text_color(ink)
                                .tooltip(crate::tooltip::text(ui, t.clone()))
                                .child(t)
                        }))
                        .children(status.map(|s| s.text_size(px(12. * k)).line_height(px(18. * k))))
                        .child(div().truncate().text_size(px(11.5 * k)).text_color(ui.text_tertiary).child(from)),
                );
                continue;
            }
            list = list.child(
                div()
                    .id(row_id.clone())
                    .group(row_id.clone())
                    .min_h(px(34.))
                    .flex_none()
                    .relative()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .hover(move |d| d.bg(ui.bg_hover))
                    // A deleted file: nothing to open, only its path to copy.
                    .when(!gone, |d| d.on_click(open))
                    .when(gone, |d| d.opacity(0.6))
                    // Artifacts (v3-ae): their type's icon in a tile, the title, the description.
                    // Docs and files (v3-am) get the same tile, with an icon for their type.
                    .map(|d| match o.kind {
                        // Files: the type icon alone, bigger, in the same 28 pt slot (no tile).
                        Kind::File => d.child(
                            div()
                                .size(px(28.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(crate::icons::artifact(t1k_agent::outputs::file_icon(&o.target), if gone { ui.text_disabled } else { ui.text_secondary }, 24.)),
                        ),
                        Kind::Artifact | Kind::Doc => d.child(
                            div()
                                .size(px(28.))
                                .flex_none()
                                .rounded(px(6.))
                                .bg(ui.text_primary.opacity(0.06))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(crate::icons::artifact(
                                    match o.kind {
                                        Kind::Doc => t1k_agent::outputs::doc_kind(&o.target).0,
                                        _ => o.icon.as_deref().unwrap_or("link"),
                                    },
                                    ui.text_secondary,
                                    16.,
                                )),
                        ),
                        // Tickets: their tracker's mark (Linear, Jira, GitHub issues).
                        Kind::Ticket => d.child(
                            div()
                                .size(px(28.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(crate::icons::artifact(t1k_agent::outputs::ticket_icon(&o.target), ui.text_secondary, 20.)),
                        ),
                        // In the same 28 pt slot as the rest, so every title lines up.
                        _ => d.child(div().size(px(28.)).flex_none().flex().items_center().justify_center().text_size(px(13. * k)).text_color(ui.text_tertiary).child(icon(o.kind))),
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(div().truncate().text_size(px(12.5 * k)).text_color(ui.text_primary).child(o.label.clone()))
                            // Descriptions get two lines, so the sentence isn't cut mid-word.
                            .child(div().when(o.detail.is_some(), |d| d.line_clamp(2)).when(o.detail.is_none(), |d| d.truncate()).text_size(px(11. * k)).text_color(ui.text_tertiary).child(match (&o.kind, &o.detail, &o.icon) {
                                (Kind::Artifact, Some(d), _) => d.clone(),
                                // The link itself is all there is to tell these apart.
                                (Kind::Artifact, None, None) => o.target.trim_start_matches("https://").to_string(),
                                (Kind::Doc, ..) => format!("{} · {tab_title} · {}", t1k_agent::outputs::doc_kind(&o.target).1, age(o.at)),
                                _ if gone => format!("Deleted · was in {}", crate::groups::tilde(std::path::Path::new(&o.target).parent().unwrap_or(std::path::Path::new("/")))),
                                _ => format!("{tab_title} · {}", age(o.at)),
                            })),
                    )
                    // Over the text on hover instead of beside it, so names and descriptions get the
                    // full width the rest of the time.
                    .child(hover_buttons.absolute().right(px(6.)).top_0().bottom_0().items_center().pl(px(8.)).bg(ui.bg_hover)),
            );
        }
        if links >= 2 {
            list = list.child(
                div()
                    .id(SharedString::from(format!("links-{label}")))
                    .min_h(px(34.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .hover(move |d| d.bg(ui.bg_hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel.links_open = !this.panel.links_open;
                        cx.notify();
                    }))
                    .child(div().size(px(28.)).flex_none().rounded(px(6.)).bg(ui.text_primary.opacity(0.06)).flex().items_center().justify_center().child(crate::icons::artifact("link", ui.text_tertiary, 16.)))
                    .child(div().flex_1().text_size(px(12.5 * k)).text_color(ui.text_secondary).child(if links_open { "Hide artifact links".to_string() } else { format!("{links} more artifact links") }))
                    .child(crate::icons::caret(ui.text_tertiary, links_open)),
            );
        }
    }
    if items.is_empty() {
        let hint = match scope {
            Scope::Tab => "Nothing from this tab yet. PRs, artifacts, docs, tickets and edited files from its Claude session show up here.",
            Scope::Project => "Nothing from this project's tabs yet.",
            Scope::All => "No session outputs yet.",
        };
        list = list.child(div().p(px(14.)).text_size(px(12. * k)).text_color(ui.text_tertiary).child(hint));
    }

    div()
        .id("outputs-panel")
        .w(px(380.))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(ui.bg_sidebar.opacity(ws.config.window.alpha()))
        .child(header)
        .child(chips)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(list)
                .child(crate::scroll_bar::overlay(&ws.outputs_scroll, &ws.outputs_bar, ui.text_tertiary)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(json: &str) -> PrInfo {
        parse_pr(json.as_bytes()).expect("parses")
    }

    #[test]
    fn parses_title_checks_and_reviews() {
        let p = pr(r#"{
            "state": "OPEN", "isDraft": false, "title": "  Score coding outcomes per turn ",
            "reviewDecision": "APPROVED",
            "statusCheckRollup": [
                {"__typename": "CheckRun", "conclusion": "SUCCESS", "status": "COMPLETED"},
                {"__typename": "CheckRun", "conclusion": "", "status": "IN_PROGRESS"},
                {"__typename": "StatusContext", "state": "FAILURE"}
            ],
            "latestReviews": [
                {"author": {"login": "ana"}, "state": "APPROVED"},
                {"author": {"login": "bo"}, "state": "COMMENTED"},
                {"author": {"login": "cy"}, "state": "CHANGES_REQUESTED"}
            ]
        }"#);
        assert_eq!(p.title, "Score coding outcomes per turn");
        assert_eq!(p.checks, (1, 1, 1));
        assert_eq!(p.ci(), Checks::Failing(1));
        assert_eq!(p.decision, "APPROVED");
        assert_eq!(p.approvers, vec!["ana"]);
        assert_eq!(p.changes, vec!["cy"]);
        assert_eq!(p.ended, None);
    }

    #[test]
    fn missing_fields_are_empty() {
        let p = pr(r#"{"state": "OPEN", "isDraft": true, "reviewDecision": null, "latestReviews": [], "statusCheckRollup": []}"#);
        assert!(p.draft);
        assert_eq!(p.title, "");
        assert_eq!(p.ci(), Checks::None);
        assert_eq!(p.review(), Review::None);
        assert!(parse_pr(b"not json").is_none());
    }

    #[test]
    fn review_states() {
        let base = pr(r#"{"state": "OPEN"}"#);
        let with = |decision: &str, approvers: &[&str], changes: &[&str]| PrInfo {
            decision: decision.into(),
            approvers: approvers.iter().map(|s| s.to_string()).collect(),
            changes: changes.iter().map(|s| s.to_string()).collect(),
            ..base.clone()
        };
        assert_eq!(with("APPROVED", &["ana"], &[]).review(), Review::Approved("Approved by ana".into()));
        assert_eq!(with("APPROVED", &["ana", "bo"], &[]).review(), Review::Approved("2 approvals".into()));
        // No required review, but someone approved anyway.
        assert_eq!(with("", &["ana"], &[]).review(), Review::Approved("Approved by ana".into()));
        assert_eq!(with("CHANGES_REQUESTED", &["bo"], &["ana"]).review(), Review::Changes("Changes requested by ana".into()));
        assert_eq!(with("REVIEW_REQUIRED", &[], &[]).review(), Review::Needed);
        // Only comments: no state to show.
        let commented = pr(r#"{"state": "OPEN", "reviewDecision": "", "latestReviews": [{"author": {"login": "bo"}, "state": "COMMENTED"}]}"#);
        assert_eq!(commented.review(), Review::None);
    }

    #[test]
    fn merged_and_closed_times() {
        let merged = pr(r#"{"state": "MERGED", "mergedAt": "2026-11-08T09:30:00Z", "closedAt": "2026-11-08T09:30:00Z"}"#);
        assert!(merged.finished());
        let secs = merged.ended.unwrap().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 1_794_130_200);
        let closed = pr(r#"{"state": "CLOSED", "mergedAt": null, "closedAt": "1970-01-02T00:00:00Z"}"#);
        assert_eq!(closed.ended, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(86400)));
        let open = pr(r#"{"state": "OPEN", "closedAt": "2026-11-08T09:30:00Z"}"#);
        assert_eq!(open.ended, None);
        assert_eq!(parse_time("garbage"), None);
    }
}

/// A row in a section: an output (and whether it's a file gone from disk), the "+N more" /
/// "Show fewer" fold, or the deleted files' toggle.
enum Row<'a> {
    Out(&'a &'a (usize, Output), bool),
    Fold { open: bool, text: String },
    Deleted { n: usize, open: bool },
}

/// A quiet 28 pt row that folds or unfolds part of a section (canvas v3-at A).
fn fold_row(ui: UiColors, k: f32, id: SharedString, text: String, open: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(28.))
        .flex_none()
        .px(px(10.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(10.))
        .cursor_pointer()
        .hover(move |d| d.bg(ui.bg_hover))
        .child(div().w(px(28.)).flex_none())
        .child(div().flex_1().text_size(px(12. * k)).text_color(ui.text_tertiary).child(text))
        .child(crate::icons::caret(ui.text_tertiary, open))
}
