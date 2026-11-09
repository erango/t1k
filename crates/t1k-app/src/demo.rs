//! Demo mode (`T1K_DEMO=1`): a made-up workspace for screenshots and screen recordings.
//!
//! Fifteen tabs in five projects, each with a fake agent state and a terminal that shows a
//! plausible screen, so every surface (sidebar, hover card, ⌘P, ⌘O, the outputs panel, the
//! title bar) works on it for real. Nothing comes from the user's machine: no session.json,
//! no config.toml, no other terminals, no transcripts. Nothing is saved either, and no agent
//! socket is opened. Terminals run a silent placeholder instead of a shell, so no prompt (with
//! a user or host name) ever shows.
//!
//! `T1K_DEMO=needs` opens on the tab waiting for `terraform apply`, with its hover card;
//! `T1K_DEMO=outputs` opens the outputs panel. Any other value is the default scene.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use gpui::{Context, Window};
use t1k_agent::agents::AgentKind;
use t1k_agent::outputs::{Kind, Output};
use t1k_agent::{AgentState, Status};

use crate::custom_icon::Icon;
use crate::outputs_panel::PrInfo;
use crate::workspace::Workspace;

/// Whether demo mode is on (`T1K_DEMO` set to anything but empty or 0).
pub fn on() -> bool {
    std::env::var("T1K_DEMO").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn scene() -> String {
    std::env::var("T1K_DEMO").unwrap_or_default()
}

/// The demo window's size in points.
pub const WINDOW: (f32, f32) = (1440., 900.);

/// What the demo terminals run: nothing visible, and typing doesn't echo.
pub fn command() -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), "stty -echo 2>/dev/null; exec cat >/dev/null".into()]
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/Users/demo"))
}

fn ago(secs: u64) -> Instant {
    Instant::now().checked_sub(Duration::from_secs(secs)).unwrap_or_else(Instant::now)
}

fn ago_sys(secs: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(secs)
}

/// A project group: its folder under ~/code (also its name), ANSI color (1-6), and icon.
struct Group {
    dir: &'static str,
    color: u8,
    icon: Option<Icon>,
}

enum Agent {
    Working(&'static str),
    Needs(&'static str),
    Error(&'static str),
    /// Finished; `true` when you haven't looked yet.
    Done(bool),
}

struct Spec {
    group: usize,
    title: &'static str,
    /// None: a plain shell running `program`.
    agent: Option<(AgentKind, Agent)>,
    program: &'static str,
    branch: &'static str,
    recap: Option<&'static str>,
    /// Seconds in the current state.
    secs: u64,
    context: Option<u64>,
    color: Option<&'static str>,
    screen: Screen,
}

#[derive(Clone, Copy)]
enum Screen {
    /// The hero: a finished Claude turn with a summary.
    Refunds,
    /// Claude asking to run `terraform apply`.
    Terraform,
    /// Claude asking to run `npm publish`.
    Publish,
    /// Mid-turn: what it's doing now, after the step before it (tool, argument, result).
    Working(&'static str, (&'static str, &'static str, &'static str)),
    /// A finished turn, generic.
    Finished(&'static str),
    Error,
    NextDev,
    CargoWatch,
}

fn groups() -> Vec<Group> {
    vec![
        Group { dir: "payments-api", color: 5, icon: Some(Icon::Emoji("💳".into())) },
        Group { dir: "acme-web", color: 4, icon: Some(Icon::Emoji("🛒".into())) },
        Group { dir: "infra-terraform", color: 3, icon: Some(Icon::Emoji("🏗️".into())) },
        Group { dir: "mobile-app", color: 2, icon: Some(Icon::Emoji("📱".into())) },
        Group { dir: "docs", color: 6, icon: Some(Icon::App("com.apple.Notes".into())) },
    ]
}

fn specs() -> Vec<Spec> {
    use AgentKind::*;
    let s = |group, title, agent, branch, recap, secs, context, screen| Spec { group, title, agent, program: "", branch, recap, secs, context, color: None, screen };
    let mut v = vec![
        // payments-api
        s(0, "Idempotency keys for refunds", Some((Claude, Agent::Done(false))), "feat/refund-idempotency", Some("Retries with the same key now return the original refund. PR #1287 is green."), 240, Some(76_000), Screen::Refunds),
        s(0, "Webhook retries with backoff", Some((Claude, Agent::Working("Running cargo test -p webhooks"))), "fix/webhook-backoff", Some("Exponential backoff with jitter; capped at 6 attempts."), 95, Some(118_000), Screen::Working("Running cargo test -p webhooks", ("Update", "src/webhooks/retry.rs", "Updated src/webhooks/retry.rs with 22 additions and 4 removals"))),
        s(0, "Stripe API 2026-10 upgrade", Some((Codex, Agent::Working("Reading src/stripe/client.rs"))), "chore/stripe-2026-10", None, 410, Some(64_000), Screen::Working("Reading src/stripe/client.rs", ("Search", "pattern: \"2025-09\"", "Found 14 files"))),
        Spec { program: "cargo", ..s(0, "cargo watch -x check", None, "main", None, 0, None, Screen::CargoWatch) },
        // acme-web
        s(1, "Checkout: Apple Pay button", Some((Claude, Agent::Working("Editing src/checkout.ts"))), "feat/apple-pay", Some("Button renders on Safari; wiring the payment sheet next."), 52, Some(91_000), Screen::Working("Editing src/checkout.ts", ("Write", "src/components/ApplePayButton.tsx", "Wrote 58 lines to src/components/ApplePayButton.tsx"))),
        s(1, "Release @acme/ui 4.2", Some((Claude, Agent::Needs("Allow Bash: npm publish?"))), "release/ui-4.2", Some("Changelog written, version bumped, build passes."), 130, Some(41_000), Screen::Publish),
        s(1, "Migrate to React 19", Some((Claude, Agent::Done(true))), "chore/react-19", Some("Upgraded 41 components; 3 need a manual look (listed in PR #812)."), 600, Some(152_000), Screen::Finished("Upgraded 41 components; 3 need a manual look (listed in PR #812).")),
        Spec { program: "node", ..s(1, "npm run dev", None, "main", None, 0, None, Screen::NextDev) },
        // infra-terraform
        s(2, "Add staging RDS replica", Some((Claude, Agent::Needs("Approve terraform apply?"))), "infra/staging-replica", Some("Plan is ready: replica, parameter group and a lag alarm."), 340, Some(58_000), Screen::Terraform),
        s(2, "Rotate CI deploy keys", Some((Claude, Agent::Error("API error 529 · overloaded"))), "chore/rotate-deploy-keys", None, 75, Some(23_000), Screen::Error),
        s(2, "Cost report for October", Some((Claude, Agent::Done(false))), "main", Some("NAT gateways are 38% of spend; drafted a cheaper egress plan."), 3600, Some(67_000), Screen::Finished("NAT gateways are 38% of spend; drafted a cheaper egress plan.")),
        // mobile-app
        s(3, "Offline mode for orders list", Some((Gemini, Agent::Working("Editing lib/orders/cache.dart"))), "feat/offline-orders", None, 180, None, Screen::Working("Editing lib/orders/cache.dart", ("Read", "lib/orders/repository.dart", "Read 140 lines"))),
        s(3, "Push notification deep links", Some((Claude, Agent::Done(true))), "feat/deep-links", Some("Deep links open the right screen on iOS and Android; PR #233 is ready."), 900, Some(104_000), Screen::Finished("Deep links open the right screen on iOS and Android; PR #233 is ready.")),
        // docs
        s(4, "Write the v3 migration guide", Some((Claude, Agent::Working("Writing guides/migrate-to-v3.md"))), "docs/v3-migration", Some("Outline done; writing the breaking-changes table."), 260, Some(88_000), Screen::Working("Writing guides/migrate-to-v3.md", ("Read", "CHANGELOG.md", "Read 412 lines"))),
        s(4, "Fix broken links", Some((Claude, Agent::Done(false))), "fix/broken-links", Some("Fixed 27 dead links and added a link check to CI."), 5400, Some(31_000), Screen::Finished("Fixed 27 dead links and added a link check to CI.")),
    ];
    // Session colors (`/color`), shown as a swatch before the title.
    v[0].color = Some("purple");
    v[12].color = Some("orange");
    v
}

/// Fill the workspace with the demo tabs. Replaces session restore.
pub fn populate(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let code = home().join("code");
    let groups = groups();
    let keys: Vec<PathBuf> = groups.iter().map(|g| code.join(g.dir)).collect();
    for (g, key) in groups.iter().zip(&keys) {
        ws.group_colors.insert(key.clone(), g.color);
        if let Some(icon) = &g.icon {
            ws.group_icons.insert(key.clone(), icon.clone());
        }
    }
    ws.group_order = keys.clone();
    if scene() == "needs" {
        // The waiting tab near the top, so its card doesn't cover its terminal.
        ws.group_order.swap(0, 2);
    }
    ws.hooks_installed = true;
    ws.match_hidden = true;
    ws.config.sidebar.second_line = "status".into();
    ws.config.sidebar.width = 340.;
    ws.config.terminal.font.family = "JetBrains Mono".into();

    for (i, spec) in specs().into_iter().enumerate() {
        ws.open_tab(None, None, window, cx);
        let screen = spec.screen;
        let tab = ws.tabs.last_mut().expect("tab just opened");
        let key = keys[spec.group].clone();
        let repo = groups[spec.group].dir;
        tab.cwd = Some(key.clone());
        tab.group = key.clone();
        tab.pinned_group = Some(key);
        tab.branch = Some(spec.branch.into());
        tab.view.update(cx, |v, _| {
            v.terminal.feed(&render(screen, 80, 24));
            v.feed_on_size = Some(Box::new(move |cols, rows| render(screen, cols, rows)));
        });
        let Some((kind, state)) = spec.agent else {
            tab.program = Some(spec.program.into());
            tab.osc_title = Some(spec.title.into());
            continue;
        };
        tab.program = Some(kind.program().into());
        let mut a = AgentState::new(format!("demo-{i}"));
        a.kind = kind;
        a.goal = Some(spec.title.into());
        a.branch = Some(spec.branch.into());
        a.recap = spec.recap.map(Into::into);
        a.context_tokens = spec.context;
        if kind == AgentKind::Claude {
            a.model = Some("claude-opus-4-5".into());
        }
        a.color = spec.color.map(Into::into);
        a.status = match state {
            Agent::Working(now) => {
                a.now = Some(now.into());
                Status::Working
            }
            Agent::Needs(reason) => Status::NeedsInput { reason: reason.into() },
            Agent::Error(message) => Status::Error { message: message.into() },
            Agent::Done(unread) => {
                tab.unread = unread;
                Status::Done
            }
        };
        a.since = ago(spec.secs);
        outputs(&mut a, repo, i, &mut ws.panel.prs);
        tab.agent = Some(a);
    }
    // A reminder later today on the cost report.
    if let Some(t) = ws.tabs.iter_mut().find(|t| t.title() == "Cost report for October") {
        t.reminder_at = Some(SystemTime::now() + Duration::from_secs(45 * 60));
    }
    // gh is never asked: the PRs above carry made-up answers.
    ws.panel.fake = true;
    // Claude's plan usage in the title bar.
    {
        use t1k_agent::statusline::{Usage, Window as UsageWindow};
        let t = crate::usage::now();
        let w = |name: &str, used: f32, mins: i64| UsageWindow { name: name.into(), used, resets_at: Some(t + mins * 60) };
        ws.usage.insert(AgentKind::Claude, Usage { windows: vec![w("5h", 34., 140), w("7d", 22., 3 * 1440 + 300)], at: t - 60 });
    }

    let find = |ws: &Workspace, title: &str| ws.tabs.iter().position(|t| t.title() == title).unwrap_or(0);
    let active = match scene().as_str() {
        "needs" => find(ws, "Add staging RDS replica"),
        _ => find(ws, "Idempotency keys for refunds"),
    };
    ws.activate(active, window, cx);
    // Opening the tabs one by one isn't history for ⌘[ / ⌘].
    ws.back.clear();
    ws.forward.clear();
    match scene().as_str() {
        "needs" => ws.card = ws.tabs.get(active).map(|t| t.id.clone()),
        "outputs" => ws.panel.open = true,
        _ => {}
    }
}

/// PRs, artifacts, tickets and edited files for a demo session, with made-up gh answers.
fn outputs(a: &mut AgentState, repo: &str, i: usize, prs: &mut std::collections::HashMap<String, Option<PrInfo>>) {
    let mut out = |kind, target: String, label: String, mins: u64, icon: Option<&str>, detail: Option<&str>| {
        a.outputs.push(Output { kind, target, label, at: ago_sys(mins * 60), icon: icon.map(Into::into), detail: detail.map(Into::into) });
    };
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // (number, title, checks passed/failed/pending, review, approvers, changes requested, state, minutes ago)
    type Pr = (u64, &'static str, (u32, u32, u32), &'static str, Vec<String>, Vec<String>, &'static str, u64);
    let list: Vec<Pr> = match i {
        0 => vec![
            (1271, "Add refunds.idempotency_keys table", (9, 0, 0), "APPROVED", s(&["maya"]), vec![], "MERGED", 95),
            (1287, "Idempotency keys for POST /v1/refunds", (12, 0, 0), "APPROVED", s(&["maya", "jonas"]), vec![], "OPEN", 4),
        ],
        1 => vec![(1290, "Webhook retries: exponential backoff with jitter", (7, 0, 3), "REVIEW_REQUIRED", vec![], vec![], "OPEN", 12)],
        5 => vec![(806, "Release @acme/ui 4.2.0", (14, 0, 0), "APPROVED", s(&["lena"]), vec![], "OPEN", 20)],
        6 => vec![(812, "Upgrade to React 19", (21, 2, 0), "CHANGES_REQUESTED", vec![], s(&["sam"]), "OPEN", 30)],
        8 => vec![(98, "Staging RDS read replica", (4, 0, 0), "REVIEW_REQUIRED", vec![], vec![], "OPEN", 15)],
        12 => vec![(233, "Push notification deep links", (11, 0, 0), "APPROVED", s(&["priya"]), vec![], "OPEN", 25)],
        14 => vec![(57, "Fix 27 dead links, add a link check", (3, 0, 0), "APPROVED", s(&["alex"]), vec![], "MERGED", 70)],
        _ => vec![],
    };
    for (n, title, checks, decision, approvers, changes, state, mins) in list {
        let url = format!("https://github.com/acme/{repo}/pull/{n}");
        a.prs.push((n, url.clone()));
        out(Kind::Pr, url.clone(), format!("{repo}#{n}"), mins, None, None);
        let ended = (state == "MERGED").then(|| ago_sys(mins.saturating_sub(30) * 60));
        prs.insert(url, Some(PrInfo { state: state.into(), draft: false, title: title.into(), checks, decision: decision.into(), approvers, changes, ended, fetched: Instant::now() }));
    }
    let root = home().join("code").join(repo);
    let file = |p: &str| root.join(p).display().to_string();
    match i {
        0 => {
            out(Kind::Ticket, "https://linear.app/acme/issue/PAY-412".into(), "PAY-412".into(), 50, None, None);
            out(Kind::Artifact, "https://claude.ai/artifact/demo-refund-flow".into(), "Refund retry flow".into(), 40, Some("chart"), Some("How a retried refund is matched to its first response"));
            for f in ["src/refunds/handler.rs", "src/refunds/idempotency.rs", "migrations/0042_idempotency_keys.sql", "tests/refunds_replay.rs"] {
                out(Kind::File, file(f), f.rsplit('/').next().unwrap_or(f).into(), 10, None, None);
            }
            out(Kind::Deploy, "https://payments-api-pr-1287.preview.acme.dev".into(), "pr-1287 preview".into(), 3, None, None);
        }
        8 => {
            out(Kind::File, file("staging/rds.tf"), "rds.tf".into(), 8, None, None);
            out(Kind::File, file("staging/alarms.tf"), "alarms.tf".into(), 7, None, None);
        }
        10 => out(Kind::Artifact, "https://claude.ai/artifact/demo-cost-report".into(), "October cloud costs".into(), 60, Some("chart"), Some("Spend by service, with the NAT gateway breakdown")),
        13 => out(Kind::Doc, "https://docs.acme.dev/guides/migrate-to-v3".into(), "Migrating to v3 (draft)".into(), 5, None, None),
        _ => {}
    }
}

// ---- terminal screens -----------------------------------------------------------------------

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GRAY: &str = "\x1b[90m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const CYAN: &str = "\x1b[36m";
/// Claude's orange.
const CLAUDE: &str = "\x1b[38;2;215;119;87m";
/// Permission prompts' blue-violet.
const ASK: &str = "\x1b[38;2;177;185;249m";

fn rule(cols: usize) -> String {
    format!("{GRAY}{}{RESET}", "─".repeat(cols))
}

/// Claude Code's input box and status line, under the transcript.
fn input_box(lines: &mut Vec<String>, cols: usize, status: &str) {
    lines.push(String::new());
    lines.push(rule(cols));
    lines.push(format!("{BOLD}>{RESET} "));
    lines.push(rule(cols));
    lines.push(format!("  {GRAY}{status}{RESET}"));
}

/// Words of `text` in lines of at most `w` characters.
fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > w {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    out.push(line);
    out
}

/// Text width for prose: the pane less the two-column gutter, at most 96.
fn prose(cols: usize) -> usize {
    cols.saturating_sub(4).clamp(30, 96)
}

fn user(lines: &mut Vec<String>, cols: usize, text: &str) {
    for (i, part) in wrap(text, prose(cols)).iter().enumerate() {
        let mark = if i == 0 { format!("{GRAY}>{RESET}") } else { " ".into() };
        lines.push(format!("{mark} {DIM}{part}{RESET}"));
    }
    lines.push(String::new());
}

fn say(lines: &mut Vec<String>, cols: usize, text: &str) {
    for (i, part) in wrap(text, prose(cols)).iter().enumerate() {
        let mark = if i == 0 { format!("{BOLD}⏺{RESET}") } else { " ".into() };
        lines.push(format!("{mark} {part}"));
    }
    lines.push(String::new());
}

fn tool(lines: &mut Vec<String>, name: &str, arg: &str, result: &str) {
    lines.push(format!("{GREEN}⏺{RESET} {BOLD}{name}{RESET}({arg})"));
    lines.push(format!("  {GRAY}⎿{RESET}  {result}"));
    lines.push(String::new());
}

/// A permission box: the command, why, and the three choices.
fn ask(lines: &mut Vec<String>, cols: usize, kind: &str, command: &str, why: &str, always: &str) {
    let w = cols.saturating_sub(2).max(40);
    let row = |s: &str, visible: usize| format!("{ASK}│{RESET}{s}{}{ASK}│{RESET}", " ".repeat(w.saturating_sub(visible)));
    let blank = row("", 0);
    lines.push(format!("{ASK}╭{}╮{RESET}", "─".repeat(w)));
    lines.push(row(&format!(" {BOLD}{kind}{RESET}"), kind.chars().count() + 1));
    lines.push(blank.clone());
    lines.push(row(&format!("   {command}"), command.chars().count() + 3));
    lines.push(row(&format!("   {GRAY}{why}{RESET}"), why.chars().count() + 3));
    lines.push(blank.clone());
    lines.push(row(" Do you want to proceed?", 24));
    lines.push(row(&format!(" {ASK}❯ 1. Yes{RESET}"), 9));
    let two = format!("2. Yes, and don't ask again for {always} commands in this project");
    lines.push(row(&format!("   {two}"), two.chars().count() + 3));
    let three = "3. No, and tell Claude what to do differently (esc)";
    lines.push(row(&format!("   {three}"), three.chars().count() + 3));
    lines.push(format!("{ASK}╰{}╯{RESET}", "─".repeat(w)));
}

fn header(lines: &mut Vec<String>, repo: &str) {
    lines.push(format!("{CLAUDE}✻{RESET} {BOLD}Welcome back!{RESET}  {GRAY}~/code/{repo}{RESET}"));
    lines.push(String::new());
}

/// The bytes for a demo screen `cols` × `rows`: printed from the bottom row up, so the last
/// line sits at the bottom, then the cursor goes where the program would leave it.
fn render(screen: Screen, cols: usize, rows: usize) -> Vec<u8> {
    let mut l: Vec<String> = Vec::new();
    // Where the cursor ends: in Claude's input box, or hidden.
    let mut cursor_in_box = false;
    match screen {
        Screen::Refunds => {
            header(&mut l, "payments-api");
            user(&mut l, cols, "Where do refunds get created?");
            say(&mut l, cols, "In src/refunds/handler.rs: create_refund() checks the charge, calls Stripe, then writes the ledger row. Nothing stops the same request from running twice.");
            user(&mut l, cols, "Add idempotency keys to POST /v1/refunds so a retried request can't refund twice.");
            say(&mut l, cols, "I'll accept an Idempotency-Key header, store the first response per key for 24 hours, and replay it on retries.");
            tool(&mut l, "Read", "src/refunds/handler.rs", "Read 182 lines");
            tool(&mut l, "Write", "migrations/0042_idempotency_keys.sql", "Wrote 14 lines to migrations/0042_idempotency_keys.sql");
            tool(&mut l, "Update", "src/refunds/handler.rs", &format!("Updated src/refunds/handler.rs with {GREEN}34 additions{RESET} and {RED}6 removals{RESET}"));
            tool(&mut l, "Bash", "cargo test -p refunds", &format!("test result: {GREEN}ok{RESET}. 48 passed; 0 failed; 0 ignored"));
            tool(&mut l, "Bash", "gh pr create --fill", &format!("{BLUE}https://github.com/acme/payments-api/pull/1287{RESET}"));
            say(&mut l, cols, "Done. Retries with the same key now return the original refund instead of making a new one.");
            for b in [
                "- New table idempotency_keys (key, request hash, response, expires_at)",
                "- Reusing a key with a different body returns 422",
                "- Three new tests cover replay, mismatch and expiry",
            ] {
                l.push(format!("  {b}"));
            }
            l.push(String::new());
            l.push("  PR #1287 is open and CI is green.".into());
            l.push(String::new());
            l.push(format!("{CLAUDE}✻{RESET} {GRAY}Worked for 38s{RESET}"));
            input_box(&mut l, cols, "⏵⏵ accept edits on (shift+tab to cycle)");
            cursor_in_box = true;
        }
        Screen::Terraform => {
            header(&mut l, "infra-terraform");
            user(&mut l, cols, "Add a read replica for the staging Postgres, with an alarm on replica lag.");
            tool(&mut l, "Write", "staging/rds.tf", "Wrote 46 lines to staging/rds.tf");
            tool(&mut l, "Write", "staging/alarms.tf", "Wrote 21 lines to staging/alarms.tf");
            tool(&mut l, "Bash", "terraform plan -out=tfplan", &format!("Plan: {GREEN}3 to add{RESET}, {YELLOW}1 to change{RESET}, 0 to destroy."));
            say(&mut l, cols, "The plan adds the replica, a parameter group and the lag alarm, and changes the primary's backup window. Applying it now.");
            ask(&mut l, cols, "Bash command", "terraform apply tfplan", "Apply the staging plan: RDS read replica, parameter group, lag alarm", "terraform apply");
        }
        Screen::Publish => {
            header(&mut l, "acme-web");
            user(&mut l, cols, "Cut the 4.2 release of @acme/ui.");
            tool(&mut l, "Update", "packages/ui/package.json", "Updated packages/ui/package.json with 1 addition and 1 removal");
            tool(&mut l, "Write", "packages/ui/CHANGELOG.md", "Wrote 32 lines to packages/ui/CHANGELOG.md");
            tool(&mut l, "Bash", "npm run build -w @acme/ui", &format!("{GREEN}✓{RESET} built in 4.1s"));
            ask(&mut l, cols, "Bash command", "npm publish -w @acme/ui --access public", "Publish @acme/ui 4.2.0 to npm", "npm publish");
        }
        Screen::Working(now, (name, arg, result)) => {
            tool(&mut l, name, arg, result);
            l.push(format!("{CLAUDE}✶{RESET} {CLAUDE}{now}…{RESET} {GRAY}(1m 12s · ↓ 2.4k tokens · esc to interrupt){RESET}"));
            input_box(&mut l, cols, "⏵⏵ accept edits on (shift+tab to cycle)");
        }
        Screen::Finished(recap) => {
            say(&mut l, cols, recap);
            l.push(format!("{CLAUDE}✻{RESET} {GRAY}Worked for 2m 41s{RESET}"));
            input_box(&mut l, cols, "? for shortcuts");
            cursor_in_box = true;
        }
        Screen::Error => {
            tool(&mut l, "Bash", "gh secret list --env production", "6 secrets");
            l.push(format!("  {RED}⎿  API Error: 529 {{\"type\":\"error\",\"error\":{{\"type\":\"overloaded_error\"}}}}{RESET}"));
            input_box(&mut l, cols, "? for shortcuts");
            cursor_in_box = true;
        }
        Screen::NextDev => {
            l.push(format!("{CYAN}~/code/acme-web{RESET} on {YELLOW}main{RESET}"));
            l.push(format!("{GREEN}❯{RESET} npm run dev"));
            l.push(String::new());
            l.push("> acme-web@2.14.0 dev".into());
            l.push("> next dev --turbo".into());
            l.push(String::new());
            l.push(format!("   {BOLD}▲ Next.js 15.1.0{RESET} (Turbopack)"));
            l.push("   - Local:        http://localhost:3000".into());
            l.push(String::new());
            l.push(format!(" {GREEN}✓{RESET} Starting..."));
            l.push(format!(" {GREEN}✓{RESET} Ready in 1.2s"));
            l.push(" ○ Compiling /checkout ...".into());
            l.push(format!(" {GREEN}✓{RESET} Compiled /checkout in 812ms"));
            l.push(format!(" GET /checkout {GREEN}200{RESET} in 941ms"));
            l.push(format!(" GET /api/cart {GREEN}200{RESET} in 38ms"));
            l.push(String::new());
        }
        Screen::CargoWatch => {
            l.push(format!("{CYAN}~/code/payments-api{RESET} on {YELLOW}main{RESET}"));
            l.push(format!("{GREEN}❯{RESET} cargo watch -x check"));
            l.push(format!("{YELLOW}[Running 'cargo check']{RESET}"));
            l.push(format!("    {GREEN}{BOLD}Checking{RESET} webhooks v0.4.1"));
            l.push(format!("    {GREEN}{BOLD}Checking{RESET} payments-api v1.9.0"));
            l.push(format!("    {GREEN}{BOLD}Finished{RESET} `dev` profile [unoptimized + debuginfo] target(s) in 2.31s"));
            l.push(format!("{YELLOW}[Finished running. Exit status: 0]{RESET}"));
            l.push(String::new());
        }
    }
    // Fill the screen from the bottom up, on a cleared screen and history: the first feed is
    // for the 80 × 24 grid (overview previews), the second for the pane's real size.
    if l.len() < rows {
        let mut padded = vec![String::new(); rows - l.len()];
        padded.append(&mut l);
        l = padded;
    }
    let mut out = String::from(RESET);
    out.push_str("\x1b[?25l\x1b[3J\x1b[H\x1b[2J");
    // Whole lines only: what doesn't fit scrolls into history, like real output.
    for (i, line) in l.iter().enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        out.push_str(line);
    }
    if cursor_in_box && rows > 3 {
        // The input box's prompt row: three rows above the bottom (rule, status below it).
        out.push_str(&format!("\x1b[{};3H\x1b[?25h", rows - 2));
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Visible width of a rendered line (escape sequences dropped).
    fn width(line: &str) -> usize {
        let mut n = 0;
        let mut esc = false;
        for c in line.chars() {
            match (esc, c) {
                (false, '\x1b') => esc = true,
                (true, c) if c.is_ascii_alphabetic() => esc = false,
                (true, _) => {}
                _ => n += 1,
            }
        }
        n
    }

    #[test]
    fn screens_fit_the_pane() {
        // The narrowest pane the screenshots use: the outputs panel open.
        let cols = 84;
        for s in specs() {
            let text = String::from_utf8(render(s.screen, cols, 40)).unwrap();
            for line in text.split("\r\n") {
                assert!(width(line) <= cols, "{}: {line}", s.title);
            }
        }
    }

    #[test]
    fn scene_has_every_state() {
        let specs = specs();
        let has = |f: &dyn Fn(&Agent) -> bool| specs.iter().any(|s| s.agent.as_ref().is_some_and(|(_, a)| f(a)));
        assert!(has(&|a| matches!(a, Agent::Needs(_))));
        assert!(has(&|a| matches!(a, Agent::Error(_))));
        assert!(has(&|a| matches!(a, Agent::Working(_))));
        assert!(has(&|a| matches!(a, Agent::Done(true))));
        assert!(specs.iter().any(|s| s.agent.is_none()));
    }
}
