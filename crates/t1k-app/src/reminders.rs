//! Typed reminder times: "45m", "2h", "1h30m", "16:30", "4pm", "tomorrow 9:15".

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Local wall-clock pieces of `t`: (days since epoch in local time, seconds into the day).
fn local(t: SystemTime) -> (i64, i64) {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let off = unsafe {
        let tt = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tt, &mut tm);
        tm.tm_gmtoff
    };
    let l = secs + off;
    (l.div_euclid(86400), l.rem_euclid(86400))
}

/// When a typed reminder should fire, relative to `now`. None if it can't be read.
pub fn parse(text: &str, now: SystemTime) -> Option<SystemTime> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let (t, tomorrow) = match t.strip_prefix("tomorrow") {
        Some(rest) => (rest.trim().trim_start_matches("at").trim().to_string(), true),
        None => (t.strip_prefix("in ").unwrap_or(&t).trim().to_string(), false),
    };
    if !tomorrow {
        if let Some(d) = duration(&t) {
            return Some(now + d);
        }
    }
    let secs_of_day = clock(if t.is_empty() && tomorrow { "9:00" } else { &t })?;
    let (_, now_secs) = local(now);
    let mut delta = secs_of_day - now_secs;
    if tomorrow {
        delta += 86400;
    } else if delta <= 0 {
        // A time that has passed today means tomorrow.
        delta += 86400;
    }
    Some(now + Duration::from_secs(delta as u64))
}

/// "45m", "2h", "1h30m", "90" (minutes), "1.5h".
fn duration(t: &str) -> Option<Duration> {
    let t = t.replace(' ', "");
    if let Ok(m) = t.parse::<u64>() {
        return (m > 0).then(|| Duration::from_secs(m * 60));
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut any = false;
    for c in t.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        total += match c {
            'h' => n * 3600.,
            'm' => n * 60.,
            'd' => n * 86400.,
            _ => return None,
        };
        any = true;
    }
    if !num.is_empty() {
        return None;
    }
    (any && total > 0.).then(|| Duration::from_secs(total as u64))
}

/// "16:30", "4pm", "4:15pm", "9" is not a clock (that's minutes). Seconds into the day.
fn clock(t: &str) -> Option<i64> {
    let t = t.replace(' ', "");
    let (body, pm, am) = if let Some(b) = t.strip_suffix("pm") {
        (b.to_string(), true, false)
    } else if let Some(b) = t.strip_suffix("am") {
        (b.to_string(), false, true)
    } else {
        (t.clone(), false, false)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?),
        None if pm || am => (body.parse::<i64>().ok()?, 0),
        None => return None,
    };
    if !(0..60).contains(&m) || h > 23 || ((pm || am) && !(1..=12).contains(&h)) {
        return None;
    }
    let h = match (pm, am) {
        (true, _) if h != 12 => h + 12,
        (_, true) if h == 12 => 0,
        _ => h,
    };
    Some(h * 3600 + m * 60)
}

/// "at 16:30 today", "at 9:15 tomorrow", "in 45 min".
pub fn describe(at: SystemTime, now: SystemTime) -> String {
    let d = at.duration_since(now).unwrap_or_default().as_secs();
    let (day_at, s) = local(at);
    let (day_now, _) = local(now);
    let clock = format!("{}:{:02}", s / 3600, (s % 3600) / 60);
    if d < 3600 {
        format!("in {} min, at {clock}", d.div_ceil(60).max(1))
    } else if day_at == day_now {
        format!("at {clock} today")
    } else if day_at == day_now + 1 {
        format!("at {clock} tomorrow")
    } else {
        format!("at {clock} in {} days", day_at - day_now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        let now = SystemTime::now();
        let in_ = |t: &str| parse(t, now).map(|a| a.duration_since(now).unwrap().as_secs());
        assert_eq!(in_("45m"), Some(2700));
        assert_eq!(in_("in 2h"), Some(7200));
        assert_eq!(in_("1h30m"), Some(5400));
        assert_eq!(in_("90"), Some(5400));
        assert_eq!(in_("1.5h"), Some(5400));
        assert_eq!(in_("soon"), None);
        assert_eq!(in_("0"), None);
    }

    #[test]
    fn clocks() {
        let now = SystemTime::now();
        let (_, s) = local(now);
        for t in ["16:30", "4:30pm", "tomorrow 9:15", "tomorrow"] {
            let at = parse(t, now).unwrap();
            let d = at.duration_since(now).unwrap().as_secs() as i64;
            assert!(d > 0 && d <= 2 * 86400, "{t}: {d}");
            let (_, as_) = local(at);
            let want = match t {
                "16:30" | "4:30pm" => 16 * 3600 + 1800,
                "tomorrow 9:15" => 9 * 3600 + 900,
                _ => 9 * 3600,
            };
            assert_eq!(as_, want, "{t}");
            let _ = s;
        }
        assert_eq!(clock("12am"), Some(0));
        assert_eq!(clock("12pm"), Some(12 * 3600));
        assert_eq!(clock("25:00"), None);
    }
}
