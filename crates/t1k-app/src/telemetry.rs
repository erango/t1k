//! Crash and error reporting through Sentry.
//!
//! On only in builds that carry a DSN (baked in at build time from `T1K_SENTRY_DSN`, which
//! the official release sets; or set at runtime), and only while `privacy.crash_reports` is
//! true. `T1K_TELEMETRY=off` always wins. Source builds without a DSN send nothing.
//!
//! What leaves the machine: panics, `log::error!` lines (T1K's own, and GPUI's renderer) (with the warn/info lines before them
//! as breadcrumbs), the app version and macOS details. Never terminal contents: T1K doesn't
//! log them. Before sending, the home folder becomes `~`, other `/Users/<name>` paths lose
//! the name, email addresses become `<email>`, URL query strings are dropped, and text a
//! panic quotes in backticks is redacted.
//!
//! Release health: each launch is one anonymous session (version, whether it ended cleanly
//! or crashed), tied to a random install id kept in the support folder, so Sentry can count
//! installs per release. The id is made up on first launch and means nothing else.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use sentry::protocol::{Breadcrumb, Event, Value};

pub type Guard = sentry::ClientInitGuard;

/// How many recent warnings and errors stay in memory for a bug report.
const RECENT_MAX: usize = 50;

static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static LAST_EVENT: Mutex<Option<String>> = Mutex::new(None);

const RELEASE: &str = concat!("t1k@", env!("CARGO_PKG_VERSION"));

fn dsn() -> Option<String> {
    if std::env::var("T1K_TELEMETRY").is_ok_and(|v| v.eq_ignore_ascii_case("off") || v == "0") {
        return None;
    }
    std::env::var("T1K_SENTRY_DSN")
        .ok()
        .or(option_env!("T1K_SENTRY_DSN").map(String::from))
        .filter(|d| !d.trim().is_empty())
}

/// A random id for this install, made on first launch (`<support>/install-id`).
fn install_id() -> Option<String> {
    let path = crate::session::support_dir()?.join("install-id");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let s = s.trim();
        if uuid::Uuid::parse_str(s).is_ok() {
            return Some(s.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, &id).ok()?;
    Some(id)
}

/// Ends this launch's session as a clean exit (a crash ends it on its own).
pub fn end_session() {
    sentry::end_session();
}

/// True when this build can send reports at all (Settings hides the toggle otherwise).
pub fn available() -> bool {
    dsn().is_some()
}

/// Set up logging (stderr, `RUST_LOG`, `~/Library/Logs/T1K/t1k.log`) and, when allowed, Sentry. Keep the guard for the
/// life of the process so pending events flush on exit.
pub fn init(allowed: bool) -> Option<Guard> {
    let guard = dsn().filter(|_| allowed).map(|dsn| {
        let mut opts = sentry::ClientOptions::default();
        opts.release = Some(Cow::Borrowed(RELEASE));
        opts.environment = Some(Cow::Borrowed(if cfg!(debug_assertions) { "development" } else { "production" }));
        opts.send_default_pii = false;
        opts.attach_stacktrace = true;
        opts.before_send = Some(Arc::new(|mut e| {
            scrub_event(&mut e);
            // Kept so a bug report can point at it.
            if let Ok(mut last) = LAST_EVENT.lock() {
                *last = Some(e.event_id.simple().to_string());
            }
            Some(e)
        }));
        opts.before_breadcrumb = Some(Arc::new(|mut b| {
            scrub_breadcrumb(&mut b);
            Some(b)
        }));
        let guard = sentry::init((dsn, opts));
        // One session per launch, counted against this install's id.
        sentry::configure_scope(|scope| {
            scope.set_user(install_id().map(|id| sentry::User { id: Some(id), ..Default::default() }));
        });
        sentry::start_session();
        guard
    });

    let dest = Recorder(env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).build());
    // Warnings and errors are always recorded for bug reports, whatever RUST_LOG prints.
    let max = dest.0.filter().max(log::LevelFilter::Warn);
    if guard.is_some() {
        // error! becomes a Sentry event, warn!/info! breadcrumbs on it (from T1K's crates
        // only; dependencies stay out).
        let logger = sentry::integrations::log::SentryLogger::with_dest(dest).filter(|md| {
            if !kept(md.target()) {
                return sentry::integrations::log::LogFilter::Ignore;
            }
            match md.level() {
                log::Level::Error => sentry::integrations::log::LogFilter::Event,
                log::Level::Warn | log::Level::Info => sentry::integrations::log::LogFilter::Breadcrumb,
                _ => sentry::integrations::log::LogFilter::Ignore,
            }
        });
        if log::set_boxed_logger(Box::new(logger)).is_ok() {
            log::set_max_level(max.max(log::LevelFilter::Info));
        }
        log::info!("crash reporting on ({RELEASE})");
    } else if log::set_boxed_logger(Box::new(dest)).is_ok() {
        log::set_max_level(max);
    }
    guard
}

/// Passes records on to stderr (and Sentry), keeping T1K's own warnings and errors in memory,
/// scrubbed, for "Report a bug". It works with crash reports off: nothing leaves the machine
/// unless the person submits the report on GitHub.
struct Recorder<L>(L);

impl<L: log::Log> log::Log for Recorder<L> {
    fn enabled(&self, md: &log::Metadata<'_>) -> bool {
        md.level() <= log::Level::Warn || self.0.enabled(md)
    }

    fn log(&self, record: &log::Record<'_>) {
        // Only T1K's crates, as for Sentry: a dependency could quote terminal output.
        if record.level() <= log::Level::Warn && kept(record.target()) {
            let line = log_line(&clock(), record.level().as_str(), record.target(), &record.args().to_string());
            write_file(&line);
            if let Ok(mut r) = RECENT.lock() {
                if r.len() == RECENT_MAX {
                    r.pop_front();
                }
                r.push_back(line);
            }
        }
        if self.0.enabled(record.metadata()) {
            self.0.log(record);
        }
    }

    fn flush(&self) {
        self.0.flush();
    }
}

/// Targets whose records are kept (memory, log file, Sentry): T1K's own crates, and GPUI's
/// Metal renderer, whose messages carry only sizes and counts. Other dependencies stay out,
/// since one could quote terminal output.
fn kept(target: &str) -> bool {
    target.starts_with("t1k") || target.starts_with("gpui::platform::mac::metal_renderer")
}

/// Largest the log file grows before it is moved to `t1k.log.1` at the next launch.
const LOG_FILE_MAX: u64 = 1024 * 1024;

static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// `~/Library/Logs/T1K/t1k.log`: the kept warnings and errors, scrubbed, so a freeze or crash
/// leaves a trail. Opened on first use; a large file rolls over to `t1k.log.1`.
fn open_log_file() -> Option<std::fs::File> {
    let dir = std::path::PathBuf::from(std::env::var_os("HOME")?).join("Library/Logs/T1K");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("t1k.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_FILE_MAX) {
        let _ = std::fs::rename(&path, dir.join("t1k.log.1"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

fn write_file(line: &str) {
    use std::io::Write;
    static TRIED: std::sync::Once = std::sync::Once::new();
    let Ok(mut file) = LOG_FILE.lock() else { return };
    TRIED.call_once(|| *file = open_log_file());
    if let Some(f) = file.as_mut() {
        let _ = writeln!(f, "{} {line}", today());
    }
}

/// Local date, YYYY-MM-DD.
fn today() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as libc::time_t;
    // SAFETY: localtime_r only writes the tm we own.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return "----------".into();
    }
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

/// "14:02:40 ERROR hooks: Codex hook timed out", scrubbed like a Sentry report. `target` is
/// shortened to its last module ("t1k_agent::hooks" → "hooks").
pub fn log_line(time: &str, level: &str, target: &str, msg: &str) -> String {
    let short = target.rsplit("::").next().unwrap_or(target);
    scrub(&format!("{time} {level:<5} {short}: {msg}")).into_owned()
}

/// Local time of day, HH:MM:SS.
pub fn clock() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as libc::time_t;
    // SAFETY: localtime_r only writes the tm we own.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return "--:--:--".into();
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// The recorded warnings and errors, oldest first.
pub fn recent() -> Vec<String> {
    RECENT.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
}

/// The id of the last event sent to Sentry in this run, if any.
pub fn last_event_id() -> Option<String> {
    LAST_EVENT.lock().ok().and_then(|l| l.clone())
}

fn is_local(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-' | '\'')
}

fn is_domain(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-')
}

/// Shorten the home folder to `~` and drop the user name from other `/Users/<name>` paths.
fn scrub_paths(s: &str) -> Cow<'_, str> {
    let home = std::env::var_os("HOME").and_then(|h| h.into_string().ok()).filter(|h| h.len() > 1);
    if !s.contains("/Users/") && !home.as_ref().is_some_and(|h| s.contains(h.as_str())) {
        return Cow::Borrowed(s);
    }
    let mut out = s.to_string();
    if let Some(home) = home {
        out = out.replace(&home, "~");
    }
    let mut from = 0;
    while let Some(off) = out[from..].find("/Users/") {
        let i = from + off + 7;
        let end = out[i..].find(|c: char| c == '/' || c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']')).map_or(out.len(), |e| i + e);
        out.replace_range(i..end, "<user>");
        from = i + 6;
    }
    Cow::Owned(out)
}

/// Replace email addresses with `<email>` and drop URL query strings.
fn scrub_mail_and_queries(s: &str) -> Cow<'_, str> {
    if !s.contains('@') && !s.contains('?') {
        return Cow::Borrowed(s);
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '@' {
            // Walk back over the local part already copied to `out`.
            let mut start = out.len();
            for (idx, ch) in out.char_indices().rev() {
                if is_local(ch) {
                    start = idx;
                } else {
                    break;
                }
            }
            let mut end = i + 1;
            while end < chars.len() && is_domain(chars[end]) {
                end += 1;
            }
            let domain: String = chars[i + 1..end].iter().collect();
            let domain = domain.trim_end_matches('.');
            if start < out.len() && domain.contains('.') {
                out.truncate(start);
                out.push_str("<email>");
                i += 1 + domain.chars().count();
                continue;
            }
        }
        if c == '?' {
            let tail: String = out.chars().rev().take_while(|ch| !ch.is_whitespace()).collect();
            if tail.contains("//") || tail.starts_with('/') || tail.ends_with('/') {
                out.push_str("?<redacted>");
                while i < chars.len() && !(chars[i].is_whitespace() || matches!(chars[i], '"' | '\'' | ')' | ']' | '>')) {
                    i += 1;
                }
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    Cow::Owned(out)
}

pub fn scrub(s: &str) -> Cow<'_, str> {
    match scrub_mail_and_queries(s) {
        Cow::Borrowed(b) => scrub_paths(b),
        Cow::Owned(o) => Cow::Owned(scrub_paths(&o).into_owned()),
    }
}

/// Panic messages quote the offending data in backticks; keep the shape, drop the content.
pub fn redact_quoted(s: &str) -> Cow<'_, str> {
    if s.matches('`').count() < 2 {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for part in s.split('`') {
        out.push_str(if inside { "`<redacted>`" } else { part });
        inside = !inside;
    }
    Cow::Owned(out)
}

fn scrub_value(v: &mut Value) {
    match v {
        Value::String(s) => {
            if let Cow::Owned(new) = scrub(s) {
                *s = new;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(scrub_value),
        Value::Object(o) => o.values_mut().for_each(scrub_value),
        _ => {}
    }
}

fn scrub_opt(s: &mut Option<String>) {
    if let Some(v) = s {
        if let Cow::Owned(new) = scrub(v) {
            *v = new;
        }
    }
}

fn redact_opt(s: &mut Option<String>) {
    if let Some(v) = s {
        if let Cow::Owned(new) = redact_quoted(v) {
            *v = new;
        }
    }
}

pub fn scrub_event(e: &mut Event<'static>) {
    redact_opt(&mut e.message);
    scrub_opt(&mut e.message);
    if let Some(le) = e.logentry.as_mut() {
        le.message = scrub(&le.message).into_owned();
    }
    for ex in e.exception.values.iter_mut() {
        redact_opt(&mut ex.value);
        scrub_opt(&mut ex.value);
    }
    for v in e.extra.values_mut() {
        scrub_value(v);
    }
    for c in e.contexts.values_mut() {
        if let sentry::protocol::Context::Other(map) = c {
            map.values_mut().for_each(scrub_value);
        }
    }
    for v in e.tags.values_mut() {
        *v = scrub(v).into_owned();
    }
    for b in e.breadcrumbs.values.iter_mut() {
        scrub_breadcrumb(b);
    }
    // Stack frames carry absolute source paths from the build machine only; no user data.
    e.user = None;
    e.server_name = None;
}

pub fn scrub_breadcrumb(b: &mut Breadcrumb) {
    scrub_opt(&mut b.message);
    b.data.values_mut().for_each(scrub_value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_paths_emails_and_queries() {
        std::env::set_var("HOME", "/Users/dana");
        assert_eq!(scrub("couldn't read /Users/dana/code/x.toml"), "couldn't read ~/code/x.toml");
        assert_eq!(scrub("socket at /Users/sam/Library/t1k.sock"), "socket at /Users/<user>/Library/t1k.sock");
        assert_eq!(scrub("hook from sam@acme.io failed"), "hook from <email> failed");
        assert_eq!(scrub("GET https://x.io/a?token=abc: 500"), "GET https://x.io/a?<redacted> 500");
        assert_eq!(scrub("what? nothing"), "what? nothing");
        assert!(matches!(scrub("plain"), Cow::Borrowed(_)));
    }

    struct Quiet;
    impl log::Log for Quiet {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            false
        }
        fn log(&self, _: &log::Record<'_>) {}
        fn flush(&self) {}
    }

    #[test]
    fn recent_lines_are_scrubbed_and_t1k_only() {
        use log::Log;
        std::env::set_var("HOME", "/Users/dana");
        assert_eq!(
            log_line("14:02:40", "ERROR", "t1k_agent::hooks", "hook from sam@acme.io failed at /Users/dana/x"),
            "14:02:40 ERROR hooks: hook from <email> failed at ~/x"
        );
        let r = Recorder(Quiet);
        let rec = |target: &'static str, level, msg: &'static str| {
            r.log(&log::Record::builder().target(target).level(level).args(format_args!("{msg}")).build());
        };
        rec("t1k_app::updater", log::Level::Warn, "feed at https://x.io/a?key=1 failed");
        rec("gpui::window", log::Level::Error, "from a dependency");
        rec("t1k_app::fonts", log::Level::Info, "just info");
        let lines = recent();
        assert!(lines.iter().any(|l| l.ends_with("WARN  updater: feed at https://x.io/a?<redacted> failed")));
        assert!(!lines.iter().any(|l| l.contains("dependency") || l.contains("just info")));
    }

    #[test]
    fn panic_payloads_are_redacted() {
        let mut e = Event::default();
        e.exception.values.push(sentry::protocol::Exception { value: Some("index out of range in `secret prompt text`".into()), ..Default::default() });
        scrub_event(&mut e);
        assert_eq!(e.exception.values[0].value.as_deref(), Some("index out of range in `<redacted>`"));
    }
}
