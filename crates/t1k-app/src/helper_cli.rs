//! `t1k helper bench` and `t1k helper stats`: is the cheap-model helper worth it?

use std::time::Duration;

use t1k_agent::helper::{self, Backend, Record, Stats};

const SAMPLE_PROMPT: &str = "the login page redirects back to itself after a successful sign-in when the \
session cookie is set on the apex domain. can you find why and fix it, then add a regression test";

const SAMPLE_SCREEN: &str = "\
● Read(src/auth/session.ts)
● Edit(src/auth/session.ts)
  - cookie.domain = req.hostname
  + cookie.domain = config.cookieDomain ?? req.hostname
● Bash(npm test -- auth)
  PASS  src/auth/session.test.ts (12 tests)
  PASS  src/auth/redirect.test.ts (4 tests)
● The redirect loop came from the cookie being scoped to app.acme.io while the login page
  checked it on acme.io. I now read the domain from config and added a regression test for
  the apex case. All 16 auth tests pass. Want me to open a PR?";

fn sample_cards() -> Vec<(usize, String)> {
    [
        "Fix login redirect loop · web-app · fix/login-loop · Fixed the cookie domain; next, open a PR",
        "Upgrade React to 19 · web-app · chore/react-19 · Waiting on a peer-dependency conflict",
        "Flaky checkout e2e test · web-app · test/checkout · Retry logic added, CI rerunning",
        "Rotate S3 credentials · infra · main · Rotated keys in staging; prod needs approval",
        "Add CSV export to reports · api · feat/csv · Endpoint done, writing tests",
        "Investigate slow dashboard query · api · perf/dashboard · Found a missing index on events.created_at",
        "Terraform drift in eu-west · infra · fix/drift · Plan shows 3 changes",
        "Write release notes for 2.4 · docs · main · Draft ready for review",
        "Dark mode for settings page · web-app · feat/dark-settings · Styling the toggles",
        "Rate limit the public API · api · feat/ratelimit · Picked a token bucket, implementing",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, s)| (i + 1, s.to_string()))
    .collect()
}

pub fn run(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("bench") => bench(&args[1..]),
        Some("stats") => stats(),
        _ => {
            println!("t1k helper bench [claude|codex|agy|gemini|grok ...] [--runs N] [--model M]");
            println!("    Run the helper's jobs (title, recap, search) on each installed CLI and compare");
            println!("    time, tokens and cost. Calls use your existing CLI logins.");
            println!("t1k helper stats");
            println!("    What the helper cost over the last 7 and 30 days, per CLI and job.");
        }
    }
}

fn bench(args: &[String]) {
    let mut runs = 2usize;
    let mut model = String::new();
    let mut picked: Vec<Backend> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--runs" => runs = it.next().and_then(|n| n.parse().ok()).unwrap_or(2).clamp(1, 20),
            "--model" => model = it.next().cloned().unwrap_or_default(),
            b => match Backend::parse(b) {
                Some(b) => picked.push(b),
                None => eprintln!("unknown CLI {b:?} (claude, codex, agy, gemini, grok)"),
            },
        }
    }
    if picked.is_empty() {
        picked = Backend::ALL.into_iter().filter(|b| b.find().is_some()).collect();
    }
    if picked.is_empty() {
        println!("No agent CLI found (claude, codex, agy, gemini, grok).");
        return;
    }
    let dir = crate::session::support_dir();
    if let Some(d) = &dir {
        let _ = std::fs::create_dir_all(d);
    }
    let cards = sample_cards();
    let offered: Vec<usize> = cards.iter().map(|(i, _)| *i).collect();
    let jobs: [(&str, String); 3] = [
        ("title", helper::title_prompt(SAMPLE_PROMPT)),
        ("recap", helper::recap_prompt(Some(SAMPLE_PROMPT), SAMPLE_SCREEN)),
        ("search", helper::search_prompt("the one fixing sign-in", &cards)),
    ];
    println!("Benchmarking {} run{} per job. Calls use your CLI logins.\n", runs, if runs == 1 { "" } else { "s" });
    let mut all: Vec<Record> = Vec::new();
    for b in &picked {
        let Some(_) = b.find() else {
            println!("{}: not installed\n", b.label());
            continue;
        };
        let m = if model.is_empty() { b.default_model().to_string() } else { model.clone() };
        println!("{} ({})", b.label(), if m.is_empty() { "default model" } else { &m });
        for (job, prompt) in &jobs {
            for n in 0..runs {
                let r = helper::ask(*b, &m, &format!("bench-{job}"), prompt, Duration::from_secs(90));
                let shown = if *job == "search" {
                    let ids = helper::parse_ids(&r.text, &offered);
                    format!("{:?}{}", ids, if ids.first() == Some(&1) { " ✓" } else { " ✗ (expected 1 first)" })
                } else {
                    r.text.chars().take(90).collect()
                };
                let u = &r.record.usage;
                println!(
                    "  {job:<6} #{}  {:>5.1}s  {:>6} in {:>5} cached {:>4} out  {:<10}  {}",
                    n + 1,
                    r.record.wall_ms as f64 / 1000.,
                    u.input_tokens,
                    u.cached_tokens,
                    u.output_tokens,
                    u.cost_usd.map(|c| format!("${c:.5}")).unwrap_or_else(|| "n/a".into()),
                    if r.record.ok { shown } else { format!("FAILED: {}", r.record.error.clone().unwrap_or_default()) },
                );
                if let Some(d) = &dir {
                    helper::append(d, &r.record);
                }
                all.push(r.record);
            }
        }
        let mine: Vec<&Record> = all.iter().filter(|r| r.backend == *b).collect();
        println!("  → {}\n", Stats::of(&mine).line());
    }
    println!("Logged to {}", dir.map(|d| helper::log_path(&d).display().to_string()).unwrap_or_default());
}

fn stats() {
    let Some(dir) = crate::session::support_dir() else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    for (label, days) in [("Last 7 days", 7u64), ("Last 30 days", 30)] {
        let rs = helper::read_log(&dir, now.saturating_sub(days * 86400));
        let live: Vec<&Record> = rs.iter().filter(|r| !r.job.starts_with("bench")).collect();
        println!("{label}: {}", Stats::of(&live).line());
        for b in Backend::ALL {
            for job in ["title", "recap", "search"] {
                let sel: Vec<&Record> = live.iter().copied().filter(|r| r.backend == b && r.job == job).collect();
                if !sel.is_empty() {
                    println!("  {} {job}: {}", b.label(), Stats::of(&sel).line());
                }
            }
        }
        let bench = rs.iter().filter(|r| r.job.starts_with("bench")).count();
        if bench > 0 {
            println!("  (plus {bench} benchmark calls, not counted)");
        }
    }
}
