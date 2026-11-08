//! "Match your terminal" (canvas v3-z, option A): a section in the welcome card that makes T1K
//! look like the terminal the user came from in one click, the sheet behind its Details…
//! (also opened from Settings › Advanced and the palette), and Undo.

use std::time::Instant;

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, SharedString, Window};
use t1k_themes::Theme;

use crate::fonts::Phase;
use crate::import_ui::{button, checkbox, mono};
use crate::match_term::{self, Current, Found, Item, Kind, ThemeSrc};
use crate::overlay::Overlay;
use crate::workspace::Workspace;

/// What the scan found, and the sheet's choices.
pub struct MatchState {
    pub found: Vec<Found>,
    /// Installed font families, to map the other app's font name.
    pub fonts: Vec<String>,
    /// Index into `found`.
    pub src: usize,
    /// Rows the user unticked.
    pub off: Vec<Kind>,
}

/// config.toml values before a match, to put back on Undo (None: the key wasn't there).
pub type MatchUndo = Vec<(Vec<&'static str>, Option<toml_edit::Value>)>;

const KEYS: [&[&str]; 6] = [
    &["terminal", "font", "family"],
    &["terminal", "font", "size"],
    &["terminal", "font", "line_height"],
    &["theme", "dark"],
    &["theme", "light"],
    &["appearance"],
];

impl Workspace {
    /// Read the other terminals' settings (at launch, and when the sheet opens).
    pub fn scan_for_match(&mut self, cx: &mut Context<Self>) {
        let fonts = cx.text_system().all_font_names();
        let task = cx.background_executor().spawn(async { match_term::scan() });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |ws, cx| {
                let off = ws.matcher.as_ref().map(|m| m.off.clone()).unwrap_or_default();
                ws.matcher = Some(MatchState { found, fonts, src: 0, off });
                cx.notify();
            });
        })
        .detach();
    }

    fn match_current(&self) -> Current {
        let f = &self.config.terminal.font;
        Current { font: f.family.clone(), size: f.size, line_height: f.line_height, theme: self.theme.name.to_string() }
    }

    /// The selected source and its rows.
    pub fn match_plan(&self) -> Option<(&Found, Vec<Item>)> {
        let m = self.matcher.as_ref()?;
        let f = m.found.get(m.src)?;
        Some((f, match_term::plan(f, &self.match_current(), &m.fonts)))
    }

    pub fn open_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.matcher.is_none() {
            let fonts = cx.text_system().all_font_names();
            self.matcher = Some(MatchState { found: match_term::scan(), fonts, src: 0, off: Vec::new() });
        }
        self.open_overlay(Overlay::Match, window, cx);
    }

    pub fn match_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        match ev.keystroke.key.as_str() {
            "escape" => None,
            "enter" => {
                self.apply_match(window, cx);
                None
            }
            _ => Some(Overlay::Match),
        }
    }

    fn toggle_match(&mut self, k: Kind, cx: &mut Context<Self>) {
        if let Some(m) = &mut self.matcher {
            if let Some(i) = m.off.iter().position(|x| *x == k) {
                m.off.remove(i);
            } else {
                m.off.push(k);
            }
        }
        cx.notify();
    }

    fn next_match_source(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = &mut self.matcher {
            if !m.found.is_empty() {
                m.src = (m.src + 1) % m.found.len();
                m.off.clear();
            }
        }
        cx.notify();
    }

    /// Write every ticked row that applies to config.toml (the watcher applies it live), and
    /// offer Undo.
    pub fn apply_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((f, items)) = self.match_plan() else { return };
        let f = f.clone();
        let off = self.matcher.as_ref().map(|m| m.off.clone()).unwrap_or_default();
        let picked: Vec<Item> = items.into_iter().filter(|i| i.applies() && !off.contains(&i.kind)).collect();
        if matches!(self.overlay, Overlay::Match) {
            self.close_overlay(window, cx);
        }
        if picked.is_empty() {
            return;
        }
        self.match_undo = Some(KEYS.iter().map(|k| (k.to_vec(), crate::config::get_raw(k))).collect());
        let mut said = Vec::new();
        for it in &picked {
            let to = it.to.clone().unwrap_or_default();
            match it.kind {
                Kind::Font => crate::config::set(&["terminal", "font", "family"], &to),
                Kind::Size => crate::config::set_value(&["terminal", "font", "size"], num_value(&to)),
                Kind::LineHeight => crate::config::set_value(&["terminal", "font", "line_height"], num_value(&to)),
                Kind::Theme => {
                    if let Some((name, text)) = match_term::theme_file(&f) {
                        if let Some(dir) = crate::config::themes_dir() {
                            let _ = std::fs::create_dir_all(&dir);
                            let file = if f.app == match_term::App::Alacritty { format!("{name}.toml") } else { name.clone() };
                            if let Err(e) = std::fs::write(dir.join(file), text) {
                                log::error!("match theme write: {e}");
                            }
                            t1k_themes::load_user_themes(&dir);
                        }
                    }
                    if let Some(t) = t1k_themes::find(&to) {
                        crate::config::set(&["theme", if t.is_dark() { "dark" } else { "light" }], &t.name);
                        // Show it now unless the system already shows that slot.
                        let showing_dark = self.theme.is_dark();
                        if showing_dark != t.is_dark() {
                            crate::config::set(&["appearance"], if t.is_dark() { "dark" } else { "light" });
                        }
                    }
                }
            }
            said.push(match it.kind {
                Kind::Font => to,
                Kind::Size => format!("{to} pt"),
                Kind::LineHeight => format!("line height {to}"),
                Kind::Theme => to,
            });
        }
        self.notice = Some((format!("Matched {}: {}", f.app.label(), said.join(", ")), Instant::now()));
        self.notice_undo = true;
        cx.notify();
    }

    pub fn undo_match(&mut self, cx: &mut Context<Self>) {
        let Some(before) = self.match_undo.take() else { return };
        for (keys, v) in before {
            match v {
                Some(v) => crate::config::set_value(&keys, v),
                None => crate::config::remove(&keys),
            }
        }
        self.notice = Some(("Put back your previous look".into(), Instant::now()));
        self.notice_undo = false;
        cx.notify();
    }
}

fn num_value(s: &str) -> toml_edit::Value {
    match s.parse::<i64>() {
        Ok(i) => i.into(),
        Err(_) => s.parse::<f64>().map(Into::into).unwrap_or_else(|_| s.into()),
    }
}

fn source_label(f: &Found) -> String {
    match &f.profile {
        Some(p) => format!("{} · {p}", f.app.label()),
        None => f.app.label().to_string(),
    }
}

/// The theme a row would apply (for the preview), or the current one.
fn after_theme(f: &Found, items: &[Item], off: &[Kind], now: &'static Theme) -> Theme {
    let on = |k: Kind| items.iter().any(|i| i.kind == k && i.applies() && !off.contains(&k));
    if !on(Kind::Theme) {
        return now.clone();
    }
    match &f.theme {
        Some(ThemeSrc::Builtin(n)) => t1k_themes::find(n).cloned().unwrap_or_else(|| now.clone()),
        Some(ThemeSrc::Colors { name, text }) => t1k_themes::import::parse(text.as_bytes(), name).unwrap_or_else(|_| now.clone()),
        _ => now.clone(),
    }
}

/// A few lines of a shell in a theme, font, size and line height.
fn preview(t: &Theme, font: &str, size: f32, lh: f32, w: f32, h: f32) -> gpui::Div {
    let c = |x: u32| gpui::rgb(x);
    let size = size * 0.85;
    let line = |parts: Vec<(String, u32)>| {
        div().flex().h(px((size * lh).round())).items_center().children(parts.into_iter().map(|(s, col)| div().text_color(c(col)).child(s)))
    };
    div()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .overflow_hidden()
        .rounded(px(8.))
        .border_1()
        .border_color(gpui::black().opacity(0.25))
        .bg(c(t.bg))
        .p(px(10.))
        .font_family(SharedString::from(font.to_string()))
        .text_size(px(size))
        .child(line(vec![("~/git-repos ".into(), t.ansi[2]), ("main".into(), t.ansi[4]), (" ❯ git status".into(), t.fg)]))
        .child(line(vec![("modified: ".into(), t.ansi[1]), ("src/app.rs".into(), t.fg)]))
        .child(line(vec![("✻ Worked for 38s".into(), t.ansi[8])]))
        .child(line(vec![("❯ ".into(), t.ansi[5]), ("▏".into(), t.cursor)]))
}

/// One line saying what a match changes: "Solarized Dark, 13 pt, line height 1.5."
fn summary(f: &Found, items: &[Item]) -> String {
    let mut parts = Vec::new();
    for k in [Kind::Theme, Kind::Font, Kind::Size, Kind::LineHeight] {
        if let Some(i) = items.iter().find(|i| i.kind == k && (i.applies() || i.download.is_some())) {
            let to = i.to.clone().unwrap_or_default();
            parts.push(match k {
                Kind::Size => format!("{to} pt"),
                Kind::LineHeight => format!("line height {to}"),
                _ => to,
            });
        }
    }
    let mut s = format!("{}.", parts.join(", "));
    if let Some((font, icons)) = items.iter().find_map(|i| i.download) {
        s += &format!(" {} is a {} download from GitHub.", font.asset(icons).family, crate::fonts::mb(font.asset(icons).bytes));
    } else if items.iter().any(|i| i.kind == Kind::Font && i.to.is_some() && i.reason.as_deref().is_some_and(|r| r.contains("isn't installed"))) {
        s += &format!(" {}'s font isn't installed, so the font stays.", f.app.label());
    }
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(s)
}

/// The font row's second line when T1K can get the font (canvas v3-ac): an Install button, the
/// install's progress or failure, or where it came from once it's in.
fn font_note(ws: &Workspace, it: &Item, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    if it.kind != Kind::Font {
        return None;
    }
    let text = |s: String, c| div().text_size(px(11.5)).text_color(c).child(s);
    if let Some((font, icons)) = it.download {
        let name = font.asset(icons).family;
        let job = ws.fonts.job(font.id).cloned();
        let row = div().flex().items_center().gap(px(8.));
        return Some(match job.as_ref().map(|j| j.phase()) {
            Some(Phase::Downloading) | Some(Phase::Installing) => {
                let j = job.unwrap();
                row.child(text(format!("Installing {name}… {}", crate::fonts_ui::job_progress(&j)), ui.text_secondary).font(crate::fonts_ui::tabular())).into_any_element()
            }
            Some(Phase::Failed(r)) => row
                .child(text(r, ui.status_error))
                .child(crate::fonts_ui::small_button(ui, "match-font-retry", "Retry", false).on_click(cx.listener(move |this, _, window, cx| this.install_font(font, icons, false, window, cx))))
                .into_any_element(),
            _ => row
                .child(text(it.reason.clone().unwrap_or_default(), ui.text_tertiary))
                .child(
                    crate::fonts_ui::small_button(ui, "match-font-install", format!("Install {name} ({})", crate::fonts::mb(font.asset(icons).bytes)), false)
                        .on_click(cx.listener(move |this, _, window, cx| this.install_font(font, icons, false, window, cx))),
                )
                .into_any_element(),
        });
    }
    // Installed by this sheet (or the picker) this session.
    let (font, _) = it.to.as_deref().filter(|_| it.applies()).and_then(crate::fonts::find)?;
    let done = ws.fonts.job(font.id).is_some_and(|j| matches!(j.phase(), Phase::Done(_)));
    done.then(|| text(format!("Installed just now from {}", font.repo), ui.text_tertiary).into_any_element())
}

/// The welcome card's section, when another terminal's look differs from T1K's.
pub fn welcome_section(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    if ws.match_hidden {
        return None;
    }
    let (f, items) = ws.match_plan()?;
    if !items.iter().any(|i| i.applies() || i.download.is_some()) {
        return None;
    }
    let app = f.app.label();
    let after = after_theme(f, &items, &[], ws.theme);
    let pick = |k: Kind, now: String| items.iter().find(|i| i.kind == k && i.applies()).and_then(|i| i.to.clone()).unwrap_or(now);
    let font = pick(Kind::Font, ws.config.terminal.font.family.clone());
    let size: f32 = pick(Kind::Size, ws.config.terminal.font.size.to_string()).parse().unwrap_or(13.);
    let lh: f32 = pick(Kind::LineHeight, ws.config.terminal.font.line_height.to_string()).parse().unwrap_or(1.45);
    Some(
        div()
            .w_full()
            .p(px(14.))
            .rounded(px(10.))
            .bg(ui.bg_terminal)
            .border_1()
            .border_color(ui.border)
            .flex()
            .gap(px(14.))
            .child(preview(&after, &font, size, lh, 150., 78.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(format!("Look like {app}?")))
                    .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_secondary).child(summary(f, &items)))
                    .child(
                        div()
                            .pt(px(2.))
                            .flex()
                            .gap(px(8.))
                            .child(div().flex_1())
                            .child(button(ui, "match-details", "Details…", false, true).on_click(cx.listener(|this, _, window, cx| this.open_match(window, cx))))
                            .child(button(ui, "match-go", format!("Match {app}"), true, true).on_click(cx.listener(|this, _, window, cx| {
                                // Includes the font's download when it needs one.
                                this.match_with_download(window, cx);
                                this.match_hidden = true;
                                cx.notify();
                            }))),
                    ),
            )
            .into_any_element(),
    )
}

/// The sheet: source, before / after, a row per setting.
pub fn sheet(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let card = div()
        .w(px(560.))
        .mt(px(90.))
        .rounded(px(14.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .font_family(".SystemUIFont")
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    let Some(m) = ws.matcher.as_ref() else { return card.into_any_element() };
    let Some((f, items)) = ws.match_plan() else {
        return card
            .p(px(20.))
            .gap(px(12.))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Match your terminal"))
            .child(div().text_size(px(12.5)).text_color(ui.text_secondary).child("No settings found from Warp, iTerm2, Ghostty, kitty or Alacritty."))
            .child(div().flex().child(div().flex_1()).child(button(ui, "match-close", "Close", false, true).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))))
            .into_any_element();
    };
    let off = m.off.clone();
    let several = m.found.len() > 1;
    let n = items.iter().filter(|i| i.applies() && !off.contains(&i.kind)).count();
    let possible = items.iter().filter(|i| i.applies()).count();
    let cur = &ws.config.terminal.font;
    let pick = |k: Kind, now: String| items.iter().find(|i| i.kind == k && i.applies() && !off.contains(&k)).and_then(|i| i.to.clone()).unwrap_or(now);
    let after = after_theme(f, &items, &off, ws.theme);
    let font = pick(Kind::Font, cur.family.clone());
    let size: f32 = pick(Kind::Size, cur.size.to_string()).parse().unwrap_or(cur.size);
    let lh: f32 = pick(Kind::LineHeight, cur.line_height.to_string()).parse().unwrap_or(cur.line_height);

    let chip = div()
        .id("match-src")
        .h(px(26.))
        .px(px(10.))
        .rounded(px(7.))
        .bg(ui.bg_control)
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(ui.text_primary)
        .child(source_label(f))
        .when(several, |d| {
            d.cursor_pointer()
                .hover(|d| d.opacity(0.85))
                .child(div().text_color(ui.text_tertiary).child("▾"))
                .on_click(cx.listener(|this, _, _, cx| this.next_match_source(cx)))
        })
        .when(several, |d| d.tooltip(crate::tooltip::text(ui, "Next terminal")));

    let mut rows = div().flex().flex_col().px(px(12.));
    for it in &items {
        let ok = it.applies();
        let on = ok && !off.contains(&it.kind);
        let k = it.kind;
        let to = it.to.clone().unwrap_or_else(|| "—".into());
        let note = font_note(ws, it, cx);
        let job = it.download.and_then(|(font, _)| ws.fonts.job(font.id)).filter(|j| matches!(j.phase(), Phase::Downloading)).cloned();
        rows = rows.child(
            div()
                .id(SharedString::from(format!("match-{}", k.label())))
                .relative()
                .overflow_hidden()
                .min_h(px(40.))
                .py(px(8.))
                .px(px(8.))
                .rounded(px(8.))
                .flex()
                .gap(px(10.))
                .when(!ok && it.download.is_none(), |d| d.opacity(0.62))
                .when(ok, |d| d.cursor_pointer().hover(move |d| d.bg(ui.bg_hover)).on_click(cx.listener(move |this, _, _, cx| this.toggle_match(k, cx))))
                .child(div().pt(px(1.)).child(checkbox(ui, Some(on))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(div().w(px(84.)).flex_none().text_size(px(12.5)).text_color(ui.text_primary).child(k.label()))
                                .child(mono(format!("{} → {to}", it.now), if ok { ui.text_secondary } else { ui.text_tertiary }).text_size(px(11.5)).truncate()),
                        )
                        .children(note.or_else(|| it.reason.clone().map(|r| div().text_size(px(11.5)).text_color(ui.text_tertiary).child(r).into_any_element())).map(|n| div().pl(px(84.)).child(n))),
                )
                .children(job.map(|j| crate::fonts_ui::progress_line(ui, crate::fonts_ui::job_fraction(&j)))),
        );
    }

    card.child(
        div()
            .px(px(20.))
            .pt(px(18.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Match your terminal"))
                    .child(chip),
            )
            .child(div().text_size(px(12.5)).text_color(ui.text_secondary).child("Use the font, size, spacing and colors you already know. You can undo it.")),
    )
    .child(
        div()
            .px(px(20.))
            .py(px(14.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().flex().flex_col().gap(px(4.)).child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("Now")).child(preview(ws.theme, &cur.family, cur.size, cur.line_height, 240., 118.)))
            .child(div().pt(px(14.)).text_color(ui.text_tertiary).child("→"))
            .child(div().flex().flex_col().gap(px(4.)).child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("After")).child(preview(&after, &font, size, lh, 240., 118.))),
    )
    .child(rows)
    .child(
        div()
            .mt(px(10.))
            .px(px(20.))
            .py(px(12.))
            .border_t_1()
            .border_color(ui.border)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().text_size(px(12.)).text_color(ui.text_tertiary).child("Applies to every tab, open and new."))
            .child(button(ui, "match-cancel", "Cancel", false, true).on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))))
            .child(
                button(ui, "match-apply", if n == possible { "Apply".to_string() } else { format!("Apply {n}") }, true, n > 0)
                    .child(div().text_size(px(11.)).opacity(0.7).child("↵"))
                    .when(n > 0, |d| d.on_click(cx.listener(|this, _, window, cx| this.apply_match(window, cx)))),
            ),
    )
    .into_any_element()
}
