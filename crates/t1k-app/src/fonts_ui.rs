//! Settings › Terminal › Font (canvas v3-ac, option A): the font picker with its search, the
//! installed fonts, and "Get more fonts" below them; and the installs it starts, which "Match
//! your terminal" uses too.

use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use gpui::{anchored, deferred, div, img, prelude::*, px, relative, AnyElement, Context, Corner, FontWeight, Hsla, MouseButton, SharedString, Window};
use t1k_themes::UiColors;

use crate::fonts::{self, Font, Job, Phase};
use crate::workspace::Workspace;

#[derive(Default)]
pub struct FontPicker {
    pub open: bool,
    pub query: String,
    /// When a click outside closed it; the same click landing on the Family button must not
    /// open it again.
    closed_at: Option<Instant>,
    /// Rows with "with icons" ticked.
    icons: Vec<&'static str>,
    /// This session's installs, newest per font.
    pub jobs: Vec<Arc<Job>>,
    /// A font whose install should finish with "Match your terminal" (the welcome card's
    /// one-click Match).
    then_match: Option<&'static str>,
    polling: bool,
    /// The "Get more fonts" row under the pointer (it shows its source and a primary Install).
    hovered: Option<&'static str>,
}

impl FontPicker {
    pub fn job(&self, id: &str) -> Option<&Arc<Job>> {
        self.jobs.iter().find(|j| j.font.id == id)
    }
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.hovered = None;
    }
}

impl Workspace {
    /// Start installing a catalog font (or its Nerd Font build). `then_match`: apply "Match your
    /// terminal" when it's done, so the font is part of it.
    pub fn install_font(&mut self, font: &'static Font, icons: bool, then_match: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.fonts.job(font.id).is_some_and(|j| j.running()) {
            return;
        }
        let Some(dest) = fonts::fonts_dir() else { return };
        let job = Job::new(font, icons);
        self.fonts.jobs.retain(|j| j.font.id != font.id);
        self.fonts.jobs.push(job.clone());
        if then_match {
            self.fonts.then_match = Some(font.id);
        }
        let ex = cx.background_executor().clone();
        cx.background_executor().spawn(fonts::run(job, dest, ex)).detach();
        self.poll_font_jobs(window, cx);
        cx.notify();
    }

    pub fn cancel_font(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(j) = self.fonts.job(id) {
            j.cancel();
        }
        cx.notify();
    }

    /// Redraw progress while a download runs, and take each finished one in.
    fn poll_font_jobs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fonts.polling {
            return;
        }
        self.fonts.polling = true;
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let more = this
                .update_in(cx, |ws, window, cx| {
                    ws.font_jobs_tick(window, cx);
                    cx.notify();
                    let more = ws.fonts.jobs.iter().any(|j| j.running());
                    ws.fonts.polling = more;
                    more
                })
                .unwrap_or(false);
            if !more {
                break;
            }
        })
        .detach();
    }

    fn font_jobs_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let finished: Vec<Arc<Job>> = self.fonts.jobs.iter().filter(|j| !j.running() && !j.registered.swap(true, Ordering::Relaxed)).cloned().collect();
        for job in finished {
            let phase = job.phase();
            if let Phase::Done(files) = &phase {
                // Hand the files to the text system so they draw now, without a restart (the
                // Fonts folder is only read by GPUI at launch).
                let bytes: Vec<std::borrow::Cow<'static, [u8]>> = files.iter().filter_map(|p| std::fs::read(p).ok()).map(std::borrow::Cow::Owned).collect();
                if let Err(e) = cx.text_system().add_fonts(bytes) {
                    log::warn!("register fonts {}: {e}", job.font.id);
                }
                crate::settings::forget_fonts();
                if let Some(m) = &mut self.matcher {
                    m.fonts = cx.text_system().all_font_names();
                }
            }
            if self.fonts.then_match == Some(job.font.id) {
                self.fonts.then_match = None;
                if matches!(phase, Phase::Cancelled) {
                    continue;
                }
                self.apply_match(window, cx);
                if let Phase::Failed(r) = &phase {
                    let said = self.notice.take().map(|(s, _)| s).unwrap_or_default();
                    self.notice = Some((format!("{} {}", r, said).trim().to_string(), Instant::now()));
                }
            }
        }
    }

    /// "Match <app>" in the welcome card when the other terminal's font is one T1K can get:
    /// install it first, then match with it.
    pub fn match_with_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dl = self.match_plan().and_then(|(_, items)| items.iter().find_map(|i| i.download));
        match dl {
            Some((font, icons)) => {
                self.install_font(font, icons, true, window, cx);
                self.notice = Some((format!("Installing {}, then matching…", font.name), Instant::now()));
                self.notice_undo = false;
            }
            None => self.apply_match(window, cx),
        }
    }

    /// Debug aids, read at launch: `T1K_DEBUG_FONTS=states[:query]` fakes a row in each
    /// state (and the pointer on Hack);
    /// `install:<id>[+icons]` starts a real install; `match` runs the welcome card's one-click
    /// Match (with its download).
    pub fn debug_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(v) = std::env::var("T1K_DEBUG_FONTS") else { return };
        if let Some(q) = v.strip_prefix("states") {
            // "states:<query>" also types a search, to bring the lower rows into view.
            self.fonts.query = q.trim_start_matches(':').to_string();
            let f = |id| fonts::by_id(id).unwrap();
            self.fonts.jobs = vec![
                Job::fake(f("fira-code"), false, Phase::Downloading, 1_100_000),
                Job::fake(f("cascadia-code"), false, Phase::Done(Vec::new()), 0),
                Job::fake(f("iosevka"), false, Phase::Failed(fonts::NO_RESPONSE.into()), 0),
                Job::fake(f("geist-mono"), false, Phase::Failed(fonts::BAD_CHECKSUM.into()), 0),
            ];
            self.fonts.icons.push("monaspace");
            // As if the pointer rested on Hack (a background window gets no mouse moves).
            self.fonts.hovered = Some("hack");
        } else if let Some(spec) = v.strip_prefix("install:") {
            let (id, icons) = spec.strip_suffix("+icons").map(|i| (i, true)).unwrap_or((spec, false));
            if let Some(font) = fonts::by_id(id) {
                self.install_font(font, icons, false, window, cx);
            }
        } else if v == "match" {
            if self.matcher.is_none() {
                let fonts = cx.text_system().all_font_names();
                self.matcher = Some(crate::match_ui::MatchState { found: crate::match_term::scan(), fonts, src: 0, off: Vec::new() });
            }
            self.match_with_download(window, cx);
        }
    }
}

// ---- rendering ----------------------------------------------------------------------------------

/// System UI text with tabular digits, so a counting "0.7 of 1.2 MB" doesn't jitter.
pub fn tabular() -> gpui::Font {
    static FEATURES: LazyLock<gpui::FontFeatures> = LazyLock::new(|| gpui::FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
    let mut f = gpui::font(".SystemUIFont");
    f.features = FEATURES.clone();
    f
}

/// "0.7 of 1.2 MB" for a running job.
pub fn job_progress(j: &Job) -> String {
    fonts::progress(j.got.load(Ordering::Relaxed), j.asset().bytes)
}

pub fn job_fraction(j: &Job) -> f32 {
    let total = j.asset().bytes.max(1);
    (j.got.load(Ordering::Relaxed) as f64 / total as f64).clamp(0., 1.) as f32
}

/// A 24-tall button.
pub fn small_button(ui: UiColors, id: impl Into<SharedString>, label: impl Into<SharedString>, primary: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .h(px(24.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .text_size(px(12.))
        .cursor_pointer()
        .map(|d| if primary { d.bg(ui.accent).text_color(gpui::white()).font_weight(FontWeight::MEDIUM) } else { d.bg(ui.bg_control).text_color(ui.text_primary) })
        .hover(|d| d.opacity(0.88))
        .child(label.into())
}

/// The 2px accent line along the bottom of a row with a download running.
pub fn progress_line(ui: UiColors, fraction: f32) -> impl IntoElement {
    div().absolute().left_0().right_0().bottom_0().h(px(2.)).child(div().h_full().w(relative(fraction)).bg(ui.accent))
}

fn matches(q: &str, name: &str) -> bool {
    let n = name.to_lowercase();
    q.split_whitespace().all(|w| n.contains(&w.to_lowercase()))
}

/// The Family control: the current font, which opens the picker.
pub fn family_control(ws: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let open = ws.fonts.open;
    let family = ws.config.terminal.font.family.clone();
    let button = div()
        .id("font-family")
        .h(px(26.))
        .px(px(10.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(8.))
        .bg(ui.bg_control)
        .border_1()
        .border_color(if open { ui.accent } else { gpui::transparent_black() })
        .cursor_pointer()
        .hover(move |d| d.bg(ui.accent.opacity(0.18)))
        .on_click(cx.listener(|this, _, _, cx| {
            if this.fonts.closed_at.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                return;
            }
            this.fonts.open = !this.fonts.open;
            this.fonts.query.clear();
            cx.notify();
        }))
        .child(div().text_size(px(12.5)).text_color(ui.text_primary).font_family(SharedString::from(family.clone())).child(family))
        .child(crate::icons::caret(ui.text_tertiary, open));
    let mut wrap = div().relative().child(button);
    if open {
        wrap = wrap.child(
            div().absolute().top(px(30.)).right_0().child(
                deferred(anchored().anchor(Corner::TopRight).snap_to_window_with_margin(px(8.)).child(panel(ws, window, cx))).with_priority(2),
            ),
        );
    }
    wrap.into_any_element()
}

fn label(ui: UiColors, text: &'static str) -> impl IntoElement {
    div().px(px(10.)).pt(px(8.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(text)
}

fn panel(ws: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let q = ws.fonts.query.clone();
    let current = ws.config.terminal.font.family.clone();
    let (all, mono) = crate::settings::font_lists(window);
    let max_h = (f32::from(window.viewport_size().height) - 140.).clamp(240., 620.);

    let search = div()
        .mx(px(4.))
        .mb(px(2.))
        .h(px(28.))
        .px(px(9.))
        .flex_none()
        .rounded(px(6.))
        .bg(ui.bg_terminal)
        .border_1()
        .border_color(if q.is_empty() { ui.border } else { ui.accent })
        .flex()
        .items_center()
        .gap(px(7.))
        .text_size(px(12.5))
        .child(crate::icons::search(ui.text_tertiary))
        .child(div().flex_1().truncate().text_color(if q.is_empty() { ui.text_tertiary } else { ui.text_primary }).child(if q.is_empty() { "Search fonts".to_string() } else { format!("{q}▏") }));

    let installed: Vec<String> = mono.into_iter().filter(|f| matches(&q, f)).collect();
    let mut list = div().flex().flex_col().child(label(ui, "Installed"));
    if installed.is_empty() {
        list = list.child(div().h(px(30.)).px(px(10.)).flex().items_center().text_size(px(12.)).text_color(ui.text_disabled).child("No installed fonts match"));
    }
    for f in installed {
        let on = f == current;
        let name = f.clone();
        list = list.child(
            div()
                .id(SharedString::from(format!("font-{f}")))
                .h(px(30.))
                .px(px(10.))
                .flex_none()
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    crate::config::set(&["terminal", "font", "family"], &name);
                    this.fonts.close();
                    cx.notify();
                }))
                .child(div().w(px(14.)).flex_none().text_size(px(12.)).text_color(ui.accent).child(if on { "✓" } else { "" }))
                .child(div().flex_1().truncate().text_size(px(13.)).text_color(ui.text_primary).font_family(SharedString::from(f.clone())).child(f)),
        );
    }

    // A font installed this session keeps its row (with "Use"); one that was here before
    // never shows.
    let get: Vec<&'static Font> =
        fonts::CATALOG.iter().filter(|f| ws.fonts.job(f.id).is_some_and(|j| !matches!(j.phase(), Phase::Cancelled)) || !f.installed(&all)).filter(|f| matches(&q, f.name)).collect();
    if !get.is_empty() {
        list = list.child(div().mx(px(10.)).my(px(6.)).h(px(1.)).flex_none().bg(ui.border)).child(label(ui, "Get more fonts"));
        let dark = ws.theme.is_dark();
        for f in get {
            list = list.child(get_row(ws, f, dark, cx));
        }
        list = list.child(
            div()
                .px(px(10.))
                .pt(px(8.))
                .pb(px(6.))
                .text_size(px(11.5))
                .line_height(px(16.))
                .text_color(ui.text_tertiary)
                .child("Downloads from each font's official GitHub release into your Fonts folder (~/Library/Fonts). No password needed. \u{201c}With icons\u{201d} gets the Nerd Font version, for prompts that use Powerline and icon glyphs."),
        );
    }

    div()
        .w(px(540.))
        .p(px(6.))
        .rounded(px(10.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.fonts.close();
            this.fonts.closed_at = Some(Instant::now());
            cx.notify();
        }))
        .child(search)
        .child(div().id("font-picker-list").max_h(px(max_h - 40.)).overflow_y_scroll().child(list))
}

/// One "Get more fonts" row (canvas v3-ac): name and details, the sample, the source on hover;
/// on the right, "with icons" and Install, or the install's state.
fn get_row(ws: &Workspace, f: &'static Font, dark: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let id = f.id;
    let gid = SharedString::from(format!("get-{id}"));
    let job = ws.fonts.job(id).cloned();
    let hovered = ws.fonts.hovered == Some(id);
    let phase = job.as_ref().map(|j| j.phase());
    let icons = job.as_ref().map(|j| j.icons).unwrap_or_else(|| ws.fonts.icons.contains(&id));
    let (details, details_color): (String, Hsla) = match &phase {
        Some(Phase::Failed(r)) => (r.clone(), ui.status_error),
        _ => (f.details(icons), ui.text_tertiary),
    };
    let sample = Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, if dark { f.sample.0 } else { f.sample.1 }.to_vec()));
    let right: AnyElement = match &phase {
        Some(Phase::Downloading) | Some(Phase::Installing) => {
            let j = job.clone().unwrap();
            let downloading = matches!(phase, Some(Phase::Downloading));
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().text_size(px(11.5)).text_color(ui.text_secondary).font(tabular()).child(if downloading { job_progress(&j) } else { "Installing…".to_string() }))
                .when(downloading, |d| {
                    d.child(
                        div()
                            .id(SharedString::from(format!("cancel-{id}")))
                            .size(px(22.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |d| d.bg(ui.bg_hover))
                            .tooltip(crate::tooltip::text(ui, "Cancel"))
                            .on_click(cx.listener(move |this, _, _, cx| this.cancel_font(id, cx)))
                            .child(crate::icons::close(ui.text_secondary)),
                    )
                })
                .into_any_element()
        }
        Some(Phase::Done(_)) => {
            let family = f.asset(icons).family;
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().text_size(px(12.)).text_color(ui.status_done).child("✓ Installed"))
                .child(small_button(ui, format!("use-{id}"), "Use", true).on_click(cx.listener(move |this, _, _, cx| {
                    crate::config::set(&["terminal", "font", "family"], family);
                    this.fonts.close();
                    cx.notify();
                })))
                .into_any_element()
        }
        Some(Phase::Failed(_)) => small_button(ui, format!("retry-{id}"), "Retry", hovered)
            .on_click(cx.listener(move |this, _, window, cx| this.install_font(f, icons, false, window, cx)))
            .into_any_element(),
        // Not started, or cancelled.
        _ => div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .id(SharedString::from(format!("icons-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(i) = this.fonts.icons.iter().position(|x| *x == id) {
                            this.fonts.icons.remove(i);
                        } else {
                            this.fonts.icons.push(id);
                        }
                        cx.notify();
                    }))
                    .child(crate::import_ui::checkbox(ui, Some(icons)))
                    .child(div().text_size(px(11.5)).text_color(ui.text_secondary).child("with icons")),
            )
            .child(
                small_button(ui, format!("install-{id}"), "Install", hovered)
                    .on_click(cx.listener(move |this, _, window, cx| this.install_font(f, icons, false, window, cx))),
            )
            .into_any_element(),
    };
    let fraction = job.as_ref().filter(|_| matches!(phase, Some(Phase::Downloading))).map(|j| job_fraction(j));
    div()
        .id(gid)
        .on_hover(cx.listener(move |this, on: &bool, _, cx| {
            if *on {
                this.fonts.hovered = Some(id);
            } else if this.fonts.hovered == Some(id) {
                this.fonts.hovered = None;
            }
            cx.notify();
        }))
        .relative()
        .flex_none()
        .px(px(14.))
        .py(px(9.))
        .rounded(px(6.))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(12.))
        .when(hovered, |d| d.bg(ui.bg_selected))
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
                        .items_baseline()
                        .gap(px(8.))
                        .child(div().flex_none().text_size(px(13.)).text_color(ui.text_primary).child(f.name))
                        .child(div().min_w_0().truncate().text_size(px(11.5)).text_color(details_color).child(details)),
                )
                .child(img(sample).w(px(300.)).h(px(16.)).flex_none())
                // Always laid out, shown on hover, so the row keeps its height.
                .child(div().h(px(14.)).text_size(px(11.)).text_color(ui.text_disabled).when(hovered, |d| d.child(f.repo))),
        )
        .child(div().flex_none().child(right))
        .children(fraction.map(|fr| progress_line(ui, fr)))
        .into_any_element()
}
