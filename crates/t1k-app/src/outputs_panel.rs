//! Right-hand panel listing what sessions produced: PRs (with live state), artifacts, docs,
//! tickets, deploys and edited files. Scope: this tab, its project, or everything.

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use gpui::{div, prelude::*, px, AnyElement, ClipboardItem, Context, FontWeight, SharedString};
use t1k_agent::outputs::{Kind, Output};

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
    /// (passed, failed, pending)
    pub checks: (u32, u32, u32),
    pub fetched: Instant,
}

#[derive(Default)]
pub struct PanelState {
    pub open: bool,
    pub scope: Scope,
    pub filter: Option<Kind>,
    pub prs: HashMap<String, Option<PrInfo>>,
    pub(crate) inflight: HashSet<String>,
}

const PR_TTL: Duration = Duration::from_secs(300);

/// `gh pr view` → state, draft, CI summary. None if gh is missing or the PR isn't reachable.
fn fetch_pr(url: &str) -> Option<PrInfo> {
    let out = Command::new("gh")
        .env("PATH", crate::platform::tool_path())
        .args(["pr", "view", url, "--json", "state,isDraft,statusCheckRollup"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut checks = (0, 0, 0);
    for c in v["statusCheckRollup"].as_array().into_iter().flatten() {
        let concl = c["conclusion"].as_str().or(c["state"].as_str()).unwrap_or("");
        match concl {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => checks.0 += 1,
            "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" => checks.1 += 1,
            _ => checks.2 += 1,
        }
    }
    Some(PrInfo {
        state: v["state"].as_str().unwrap_or("").to_string(),
        draft: v["isDraft"].as_bool().unwrap_or(false),
        checks,
        fetched: Instant::now(),
    })
}

impl Workspace {
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

    /// Fetch PR state for PRs in view that we don't know yet (or that went stale).
    /// Fetch PR state for PRs in view (the panel's scope, or the active tab for the session
    /// strip) that we don't know yet or that went stale.
    pub fn refresh_prs(&mut self, cx: &mut Context<Self>) {
        let candidates: Vec<(usize, Output)> = if self.panel.open {
            self.outputs_in_scope()
        } else {
            let active = self.active;
            self.tabs
                .get(active)
                .and_then(|t| t.agent.as_ref())
                .map(|a| a.outputs.iter().filter(|o| o.kind == Kind::Pr).map(|o| (active, o.clone())).collect())
                .unwrap_or_default()
        };
        let stale: Vec<String> = candidates
            .into_iter()
            .filter(|(_, o)| o.kind == Kind::Pr)
            .map(|(_, o)| o.target)
            .filter(|u| !self.panel.inflight.contains(u))
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

    let mut list = div().id("outputs-list").flex_1().min_h_0().overflow_y_scroll().px(px(6.)).pb(px(12.)).flex().flex_col();
    // Sections by kind (design): PRs as cards first, then docs/artifacts, tickets, deploys, files.
    let sections: [(&str, &[Kind]); 5] = [
        ("PULL REQUESTS", &[Kind::Pr]),
        ("DOCS + ARTIFACTS", &[Kind::Doc, Kind::Artifact]),
        ("TICKETS", &[Kind::Ticket]),
        ("DEPLOYS", &[Kind::Deploy]),
        ("FILES", &[Kind::File]),
    ];
    let mut n = 0usize;
    for (label, kinds) in sections {
        let in_section: Vec<&&(usize, Output)> = items.iter().filter(|(_, o)| kinds.contains(&o.kind)).collect();
        if in_section.is_empty() {
            continue;
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
                .child(div().font_family("JetBrains Mono").text_color(ui.text_disabled).child(in_section.len().to_string())),
        );
        for (tab, o) in in_section.into_iter().map(|x| (x.0, &x.1)) {
            n += 1;
            let target = o.target.clone();
            let copy_target = o.target.clone();
            let is_file = o.kind == Kind::File;
            let tab_title = ws.tabs.get(tab).map(|t| t.title()).unwrap_or_default();
            let row_id = SharedString::from(format!("out-{n}"));
            let open = cx.listener(move |_, _, _, cx| {
                if is_file {
                    cx.open_with_system(std::path::Path::new(&target));
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
                    .child(
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
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("copy-{n}")))
                            .px(px(6.))
                            .h(px(22.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .text_size(px(11. * k))
                            .text_color(ui.text_tertiary)
                            .hover(move |d| d.bg(ui.bg_control).text_color(ui.text_primary))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_target.clone()));
                            }))
                            .child("Copy"),
                    )
            };
            if o.kind == Kind::Pr {
                // PR card: number, state, CI summary, tab and age.
                let pr = ws.panel.prs.get(&o.target).cloned().flatten();
                let (state, state_color) = match pr.as_ref() {
                    Some(p) => crate::github::pr_state(&p.state, p.draft),
                    None => ("…", ui.text_disabled),
                };
                let checks = pr.map(|p| p.checks).unwrap_or((0, 0, 0));
                list = list.child(
                    div()
                        .id(row_id.clone())
                        .group(row_id.clone())
                        .mx(px(4.))
                        .mb(px(6.))
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(ui.bg_raised)
                        .border_1()
                        .border_color(ui.border)
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .hover(move |d| d.border_color(ui.text_disabled))
                        .on_click(open)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().text_size(px(13. * k)).font_family("JetBrains Mono").font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(o.label.clone()))
                                .child(
                                    // GitHub's solid state pill, white on the state color, in every theme.
                                    div().px(px(7.)).h(px(18.)).flex().items_center().rounded(px(9.)).bg(state_color).text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()).child(state),
                                )
                                .child(div().flex_1())
                                .child(hover_buttons),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(10.))
                                .text_size(px(11.5 * k))
                                .when(checks.0 > 0, |d| d.child(div().text_color(crate::github::open()).child(format!("✓ {} passed", checks.0))))
                                .when(checks.1 > 0, |d| d.child(div().text_color(crate::github::closed()).child(format!("✗ {} failed", checks.1))))
                                .when(checks.2 > 0, |d| d.child(div().text_color(crate::github::pending()).child(format!("● {} running", checks.2))))
                                .when(checks == (0, 0, 0), |d| d.child(div().text_color(ui.text_tertiary).child("No checks"))),
                        )
                        .child(div().truncate().text_size(px(11. * k)).text_color(ui.text_tertiary).child(format!("{tab_title} · {}", age(o.at)))),
                );
                continue;
            }
            list = list.child(
                div()
                    .id(row_id.clone())
                    .group(row_id.clone())
                    .min_h(px(34.))
                    .flex_none()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .hover(move |d| d.bg(ui.bg_hover))
                    .on_click(open)
                    // Artifacts (v3-ae): their type's icon in a tile, the title, the description.
                    .map(|d| match o.kind {
                        Kind::Artifact => d.child(
                            div()
                                .size(px(28.))
                                .flex_none()
                                .rounded(px(6.))
                                .bg(ui.text_primary.opacity(0.06))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(crate::icons::artifact(o.icon.as_deref().unwrap_or("link"), ui.text_secondary, 16.)),
                        ),
                        _ => d.child(div().w(px(16.)).flex_none().text_size(px(13. * k)).text_color(ui.text_tertiary).child(icon(o.kind))),
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(div().truncate().text_size(px(12.5 * k)).text_color(ui.text_primary).child(o.label.clone()))
                            .child(div().truncate().text_size(px(11. * k)).text_color(ui.text_tertiary).child(match (&o.kind, &o.detail, &o.icon) {
                                (Kind::Artifact, Some(d), _) => d.clone(),
                                (Kind::Artifact, None, None) => "claude.ai · link mentioned in this session".to_string(),
                                _ => format!("{tab_title} · {}", age(o.at)),
                            })),
                    )
                    .child(hover_buttons),
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
        .bg(ui.bg_sidebar)
        .child(header)
        .child(chips)
        .child(list)
        .into_any_element()
}
