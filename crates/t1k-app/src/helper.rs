//! Running the cheap-model helper from the app: jobs are queued by hooks and the switcher,
//! run off the UI thread (two at a time, at most 120 an hour), measured to `helper.jsonl`,
//! and their answers written back onto the tab.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use gpui::Context;
use t1k_agent::helper::{self, Backend, Record, Stats};
use t1k_agent::Status;

use crate::workspace::Workspace;

const MAX_INFLIGHT: usize = 2;
const MAX_PER_HOUR: usize = 120;

#[derive(Clone, Debug)]
pub enum JobKind {
    Title,
    Recap,
    /// Rank tabs for a switcher query; `ids` maps the numbers offered to tab ids.
    Search { query: String, ids: Vec<String> },
    /// Emoji suggestions for a group's or a tab's icon.
    Icon { key: crate::group_icons::IconFor },
}

#[derive(Clone, Debug)]
pub struct Job {
    pub tab_id: String,
    pub session_id: String,
    pub kind: JobKind,
    pub prompt: String,
}

#[derive(Default)]
pub struct HelperState {
    queue: VecDeque<Job>,
    inflight: usize,
    started: VecDeque<Instant>,
    /// Sessions already given a title, so each gets one attempt.
    titled: HashSet<String>,
    /// The switcher query being ranked, and the result once it lands (tab ids, best first).
    pub search: Option<(String, Option<Vec<String>>)>,
}

impl Workspace {
    fn helper_backend(&self) -> Option<Backend> {
        self.config.helper.backend()
    }

    /// A turn just finished on tab `ix`: queue a title and/or recap if the agent didn't give one.
    pub fn helper_after_turn(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(_) = self.helper_backend() else { return };
        let cfg = self.config.helper.clone();
        let tab = &self.tabs[ix];
        let Some(a) = tab.agent.as_ref() else { return };
        let is_claude = a.kind == t1k_agent::agents::AgentKind::Claude;
        let sid = a.session_id.clone();
        let mut jobs = Vec::new();
        if cfg.titles && a.goal.is_none() && !self.helper.titled.contains(&sid) {
            if let Some(p) = a.first_prompt_full.as_deref() {
                jobs.push((JobKind::Title, helper::title_prompt(p)));
            }
        }
        // Claude writes its own recaps (away_summary); others never do.
        if cfg.recaps && (!is_claude || a.recap.is_none()) && !matches!(a.status, Status::Ended) {
            let screen = tab.view.read(cx).terminal.last_lines(60).join("\n");
            jobs.push((JobKind::Recap, helper::recap_prompt(a.last_prompt.as_deref(), &screen)));
        }
        let tab_id = tab.id.clone();
        for (kind, prompt) in jobs {
            if matches!(kind, JobKind::Title) {
                self.helper.titled.insert(sid.clone());
            }
            // A newer recap for the same tab replaces a queued one.
            if matches!(kind, JobKind::Recap) {
                self.helper.queue.retain(|j| !(j.tab_id == tab_id && matches!(j.kind, JobKind::Recap)));
            }
            self.helper.queue.push_back(Job { tab_id: tab_id.clone(), session_id: sid.clone(), kind, prompt });
        }
        self.pump_helper(cx);
    }

    /// ⌘↵ in the switcher: rank every tab by meaning.
    pub fn helper_search(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.helper_backend().is_none() || !self.config.helper.search || query.trim().is_empty() {
            return;
        }
        let mut ids = Vec::new();
        let mut cards = Vec::new();
        for t in &self.tabs {
            ids.push(t.id.clone());
            let mut card = t.title();
            card += &format!(" · {}", self.group_label(&t.group));
            if let Some(b) = t.branch() {
                card += &format!(" · {b}");
            }
            let sub = t.subtitle();
            if !sub.is_empty() {
                card += &format!(" · {sub}");
            }
            cards.push((ids.len(), card));
        }
        self.helper.search = Some((query.to_string(), None));
        self.helper.queue.retain(|j| !matches!(j.kind, JobKind::Search { .. }));
        let prompt = helper::search_prompt(query, &cards);
        self.helper.queue.push_front(Job { tab_id: String::new(), session_id: String::new(), kind: JobKind::Search { query: query.to_string(), ids }, prompt });
        self.pump_helper(cx);
        cx.notify();
    }

    /// Ask the helper for a group's icon suggestions (once; the answer is saved).
    pub fn helper_icon(&mut self, key: &std::path::Path, label: &str, titles: &[String], cx: &mut Context<Self>) {
        if self.helper_backend().is_none() {
            return;
        }
        let readme = key.is_dir().then(|| std::fs::read_to_string(key.join("README.md")).ok()).flatten();
        let heading = readme.as_deref().and_then(|r| r.lines().find(|l| l.starts_with('#'))).map(|h| h.trim_start_matches('#').trim().to_string());
        let prompt = crate::group_icons::icon_prompt(label, heading.as_deref(), titles);
        self.helper.queue.push_back(Job { tab_id: String::new(), session_id: String::new(), kind: JobKind::Icon { key: crate::group_icons::IconFor::Group(key.to_path_buf()) }, prompt });
        self.pump_helper(cx);
    }

    /// Ask the helper for a tab's icon suggestions (when its picker first opens).
    pub fn helper_tab_icon(&mut self, id: &str, title: &str, project: &str, detail: Option<&str>, cx: &mut Context<Self>) {
        if self.helper_backend().is_none() {
            return;
        }
        let prompt = crate::group_icons::tab_icon_prompt(title, project, detail);
        self.helper.queue.push_front(Job { tab_id: id.to_string(), session_id: String::new(), kind: JobKind::Icon { key: crate::group_icons::IconFor::Tab(id.to_string()) }, prompt });
        self.pump_helper(cx);
    }

    pub fn pump_helper(&mut self, cx: &mut Context<Self>) {
        let Some(backend) = self.helper_backend() else {
            self.helper.queue.clear();
            return;
        };
        let model = self.config.helper.model.clone();
        while self.helper.inflight < MAX_INFLIGHT {
            let hour_ago = Instant::now() - Duration::from_secs(3600);
            while self.helper.started.front().is_some_and(|t| *t < hour_ago) {
                self.helper.started.pop_front();
            }
            if self.helper.started.len() >= MAX_PER_HOUR {
                break;
            }
            let Some(job) = self.helper.queue.pop_front() else { break };
            self.helper.inflight += 1;
            self.helper.started.push_back(Instant::now());
            let model = model.clone();
            let name = match &job.kind {
                JobKind::Title => "title",
                JobKind::Recap => "recap",
                JobKind::Search { .. } => "search",
                JobKind::Icon { .. } => "icon",
            };
            let prompt = job.prompt.clone();
            let task = cx.background_executor().spawn(async move { helper::ask(backend, &model, name, &prompt, Duration::from_secs(60)) });
            cx.spawn(async move |this, cx| {
                let reply = task.await;
                let _ = this.update(cx, |ws, cx| ws.helper_done(job, reply, cx));
            })
            .detach();
        }
    }

    fn helper_done(&mut self, job: Job, reply: helper::Reply, cx: &mut Context<Self>) {
        self.helper.inflight = self.helper.inflight.saturating_sub(1);
        if let Some(d) = crate::session::support_dir() {
            helper::append(&d, &reply.record);
        }
        if let Ok(mut w) = WEEK.lock() {
            *w = None;
        }
        if !reply.record.ok {
            log::warn!("helper {}: {}", reply.record.job, reply.record.error.clone().unwrap_or_default());
        }
        let text = reply.text;
        match job.kind {
            JobKind::Search { query, ids } => {
                if self.helper.search.as_ref().is_some_and(|(q, _)| *q == query) {
                    let offered: Vec<usize> = (1..=ids.len()).collect();
                    let ranked = helper::parse_ids(&text, &offered).into_iter().map(|n| ids[n - 1].clone()).collect();
                    self.helper.search = Some((query, Some(ranked)));
                    if let crate::overlay::Overlay::Switcher { selected, .. } = &mut self.overlay {
                        *selected = 0;
                    }
                }
            }
            JobKind::Icon { key } => {
                let picks = crate::group_icons::parse_emoji(&text);
                if picks.len() >= 3 {
                    match key {
                        crate::group_icons::IconFor::Group(k) => {
                            self.icon_suggestions.insert(k, picks);
                            self.save();
                        }
                        crate::group_icons::IconFor::Tab(id) => {
                            self.tab_icon_suggestions.insert(id, picks);
                        }
                    }
                    cx.notify();
                }
            }
            kind if !text.is_empty() => {
                let Some(tab) = self.tabs.iter_mut().find(|t| t.id == job.tab_id) else { return };
                let Some(a) = tab.agent.as_mut().filter(|a| a.session_id == job.session_id) else { return };
                match kind {
                    JobKind::Title if a.goal.is_none() => a.goal = Some(text.chars().take(60).collect()),
                    JobKind::Recap => a.recap = Some(text.chars().take(140).collect()),
                    _ => {}
                }
                self.save();
            }
            _ => {}
        }
        self.pump_helper(cx);
        cx.notify();
    }
}

static WEEK: std::sync::Mutex<Option<(Instant, Stats)>> = std::sync::Mutex::new(None);

/// Helper use over the last 7 days (live calls, not benchmarks). Re-read at most every 10 s.
pub fn week() -> Stats {
    let mut w = WEEK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, s)) = w.as_ref() {
        if at.elapsed() < Duration::from_secs(10) {
            return s.clone();
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let rs: Vec<Record> = crate::session::support_dir().map(|d| helper::read_log(&d, now.saturating_sub(7 * 86400))).unwrap_or_default();
    let live: Vec<&Record> = rs.iter().filter(|r| !r.job.starts_with("bench")).collect();
    let s = Stats::of(&live);
    *w = Some((Instant::now(), s.clone()));
    s
}
