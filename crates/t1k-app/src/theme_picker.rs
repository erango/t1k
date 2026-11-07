//! The theme picker (canvas board 7): built-in and imported themes for the dark or light slot,
//! the appearance mode, and a preview of the whole window in the theme under the cursor.
//! Moving through the list previews it live; ↵ keeps it, esc goes back to what you had.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, KeyDownEvent, MouseButton, PathPromptOptions, SharedString, Window};
use t1k_themes::{AppearanceMode, Theme};

use crate::input::{self, Key};
use crate::overlay::{fuzzy, Overlay};
use crate::workspace::Workspace;

pub fn is_builtin(t: &Theme) -> bool {
    t1k_themes::BUILTIN.iter().any(|b| std::ptr::eq(b, t))
}

/// Themes for one slot matching `query`, best first (built-ins then imported, by name, when empty).
pub fn results(query: &str, dark: bool) -> Vec<&'static Theme> {
    let mut all: Vec<&'static Theme> = t1k_themes::all().into_iter().filter(|t| t.is_dark() == dark).collect();
    all.sort_by_key(|t| (!is_builtin(t), t.name.to_lowercase()));
    if query.trim().is_empty() {
        return all;
    }
    let hays: Vec<String> = all.iter().map(|t| t.name.to_string()).collect();
    let mut r = fuzzy(query, &hays);
    r.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    r.into_iter().map(|(_, i)| all[i]).collect()
}

impl Workspace {
    pub fn open_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dark = self.theme.is_dark();
        let selected = results("", dark).iter().position(|t| t.name == self.theme.name).unwrap_or(0);
        self.open_overlay(Overlay::ThemePicker { query: String::new(), selected, dark }, window, cx);
    }

    /// Key handling; returns the overlay to keep showing, or None when it closed.
    pub fn theme_picker_key(&mut self, mut query: String, mut selected: usize, mut dark: bool, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        if ev.keystroke.key == "tab" {
            dark = !dark;
            let slot = self.theme_slot(dark).name.clone();
            selected = results(&query, dark).iter().position(|t| t.name == slot).unwrap_or(0);
        } else {
            match input::handle(&mut query, ev) {
                Key::Cancel => {
                    self.end_theme_preview(None, window, cx);
                    return self.settings_return.take().map(|section| Overlay::Settings { section });
                }
                Key::Submit => {
                    let pick = results(&query, dark).get(selected).map(|t| t.name.to_string());
                    self.end_theme_preview(pick.as_deref(), window, cx);
                    return self.settings_return.take().map(|section| Overlay::Settings { section });
                }
                Key::Edited => selected = 0,
                Key::Up => selected = selected.saturating_sub(1),
                Key::Down => selected = (selected + 1).min(results(&query, dark).len().saturating_sub(1)),
                _ => return Some(Overlay::ThemePicker { query, selected, dark }),
            }
        }
        if let Some(t) = results(&query, dark).get(selected) {
            self.preview_theme(&t.name, window, cx);
        }
        Some(Overlay::ThemePicker { query, selected, dark })
    }

    /// Import theme files chosen from the picker's Import button, then show the first one.
    pub fn import_into_picker(&mut self, paths: &[std::path::PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let mut first = None;
        for p in paths {
            match crate::cli::import_theme(p) {
                Ok(name) => {
                    first.get_or_insert(name);
                }
                Err(e) => log::error!("theme import {}: {e}", p.display()),
            }
        }
        let Some(name) = first else { return };
        self.apply_config(window, cx);
        let Some(t) = t1k_themes::find(&name) else { return };
        if let Overlay::ThemePicker { .. } = self.overlay {
            let dark = t.is_dark();
            let selected = results("", dark).iter().position(|x| x.name == t.name).unwrap_or(0);
            self.overlay = Overlay::ThemePicker { query: String::new(), selected, dark };
            self.preview_theme(&name, window, cx);
        }
    }
}

fn c(v: u32) -> Hsla {
    gpui::rgb(v).into()
}

/// A few lines of a typical agent session, drawn in the theme's own colors.
fn sample(t: &Theme) -> impl IntoElement {
    let a = t.ansi;
    let line = |parts: Vec<(&'static str, u32)>| div().flex().children(parts.into_iter().map(|(s, col)| div().text_color(c(col)).child(s)));
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .px(px(18.))
        .py(px(14.))
        .bg(c(t.bg))
        .font_family("JetBrains Mono")
        .text_size(px(12.))
        .line_height(px(18.))
        .flex()
        .flex_col()
        .child(line(vec![("~/code/t1k", a[4]), (" main", a[5]), (" ❯ ", a[2]), ("cargo test", t.fg)]))
        .child(line(vec![("   Compiling ", a[2]), ("t1k-app v0.1.0", t.fg)]))
        .child(line(vec![("test keymap::overrides ... ", t.fg), ("ok", a[2])]))
        .child(line(vec![("test session::restore ... ", t.fg), ("FAILED", a[1])]))
        .child(line(vec![("warning", a[3]), (": unused import `Hsla`", t.fg)]))
        .child(div().h(px(8.)))
        .child(line(vec![("● ", a[8]), ("Edit(src/session.rs)", t.fg)]))
        .child(line(vec![("  - ", a[1]), ("let saved = load()?;", a[1])]))
        .child(line(vec![("  + ", a[2]), ("let saved = load().unwrap_or_default();", a[2])]))
        .child(div().h(px(8.)))
        .child(line(vec![("✳ ", a[3]), ("Do you want to make this edit?", t.fg)]))
        .child(line(vec![("❯ 1. Yes", a[6]), ("   2. No", a[8])]))
}

fn hex(v: u32) -> String {
    format!("#{v:06X}")
}

/// The 9-color strip under each theme's name: background, the six hues, foreground, cursor.
fn swatch_bar(t: &Theme) -> impl IntoElement {
    let a = t.ansi;
    let cols = [t.bg, a[1], a[2], a[3], a[4], a[5], a[6], t.fg, t.cursor];
    div().h(px(8.)).flex().rounded(px(2.)).overflow_hidden().border_1().border_color(c(t.fg).opacity(0.12)).children(cols.map(|v| div().flex_1().h_full().bg(c(v))))
}

/// A small picture of the T1K window in theme `t`: its own derived chrome, a sidebar with
/// tabs in each state, and an agent session in the terminal.
fn mini_window(t: &'static Theme, started: std::time::Instant) -> impl IntoElement {
    let u = t.ui();
    let row = |title: &'static str, sub: &'static str, age: &'static str, status: AnyElement, selected: bool, sub_color: Hsla| {
        div()
            .mx(px(6.))
            .px(px(10.))
            .py(px(5.))
            .rounded(px(6.))
            .when(selected, |d| d.bg(u.bg_terminal).border_1().border_color(u.border))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(u.text_primary)
                    .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(div().flex_1().truncate().child(title))
                    .child(div().text_size(px(10.5)).font_family("JetBrains Mono").text_color(u.text_tertiary).child(age))
                    .child(status),
            )
            .child(div().truncate().text_size(px(11.5)).text_color(sub_color).child(sub))
    };
    let dot = |col: Hsla| div().size(px(7.)).rounded_full().bg(col).into_any_element();
    let needs = |title: &'static str, age: &'static str| {
        div()
            .h(px(22.))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(u.text_primary)
            .child(div().size(px(7.)).rounded_full().bg(u.status_needs))
            .child(div().flex_1().truncate().child(title))
            .child(div().text_size(px(10.5)).font_family("JetBrains Mono").text_color(u.status_needs).child(age))
    };
    div()
        .flex_1()
        .min_h(px(180.))
        .rounded(px(10.))
        .overflow_hidden()
        .border_1()
        .border_color(c(t.fg).opacity(0.18))
        .flex()
        .child(
            div()
                .w(px(240.))
                .flex_none()
                .rounded_l(px(9.))
                .bg(u.bg_sidebar)
                .border_r_1()
                .border_color(u.border)
                .py(px(12.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .mx(px(8.))
                        .mb(px(6.))
                        .px(px(8.))
                        .pt(px(4.))
                        .pb(px(6.))
                        .rounded(px(7.))
                        .bg(u.bg_terminal)
                        .border_1()
                        .border_color(u.border)
                        .child(div().h(px(22.)).flex().items_center().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(u.status_needs).child("NEEDS YOU  2"))
                        .child(needs("Upgrade EKS to 1.31", "21m"))
                        .child(needs("Dark mode tokens audit", "14m")),
                )
                .child(
                    div()
                        .h(px(24.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(u.text_secondary)
                        .child("web-dashboard")
                        .child(div().text_size(px(10.5)).font_family("JetBrains Mono").font_weight(FontWeight::NORMAL).text_color(u.text_tertiary).child("4")),
                )
                .child(row(
                    "Fix flaky checkout test",
                    "Running pnpm test",
                    "6m",
                    div().size(px(10.)).flex_none().child(crate::sidebar::spinner_sized(u.status_working, started, 4.)).into_any_element(),
                    true,
                    u.text_quiet,
                ))
                .child(row("Migrate charts to v5", "PR #418 opened; next: review CI", "now", dot(u.accent), false, u.text_quiet))
                .child(row("Bump Next to 15.2", "Editing next.config.mjs", "2m", div().size(px(10.)).flex_none().child(crate::sidebar::spinner_sized(u.status_working, started, 4.)).into_any_element(), false, u.text_quiet))
                .child(row(
                    "GPU atlas eviction",
                    "API error 529 · overloaded",
                    "3m",
                    div().size(px(6.)).bg(u.status_error).into_any_element(),
                    false,
                    u.status_error,
                )),
        )
        .child(sample(t))
}

pub fn render(ws: &Workspace, query: &str, selected: usize, dark: bool, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let hits = results(query, dark);
    let current = ws.saved_slot(dark).name.clone();
    let vp = window.viewport_size();
    // Design: 1180×800, shrunk to fit smaller windows.
    let w = (f32::from(vp.width) - 64.).clamp(720., 1180.);
    let h = (f32::from(vp.height) - 96.).clamp(480., 800.);
    let section = |label: &'static str| {
        div().h(px(22.)).px(px(8.)).flex().items_center().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child(label)
    };

    // Rows that fit between the header, the slot note and the import area; a window of them
    // follows the selection.
    let fit = (((h - 52. - 56. - 30. - 150.) / 50.).floor() as usize).max(3);
    let start = selected.saturating_sub(fit / 2).min(hits.len().saturating_sub(fit));
    let mut list = div().flex().flex_col().gap(px(2.));
    let mut last_builtin = None;
    for (i, t) in hits.iter().enumerate().skip(start).take(fit) {
        let builtin = is_builtin(t);
        if query.trim().is_empty() && last_builtin != Some(builtin) {
            list = list.child(section(if builtin { "BUILT IN" } else { "IMPORTED" }));
            last_builtin = Some(builtin);
        }
        let on = i == selected;
        let name = t.name.to_string();
        let tag = if t.name == current {
            "in use".to_string()
        } else if on {
            "previewing".to_string()
        } else if t.is_high_contrast() {
            "AAA".to_string()
        } else if !builtin {
            if t.is_dark() { "dark".into() } else { "light".into() }
        } else {
            String::new()
        };
        list = list.child(
            div()
                .id(SharedString::from(format!("theme-{name}")))
                .h(px(48.))
                .flex_none()
                .px(px(10.))
                .rounded(px(7.))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(5.))
                .when(on, |d| d.bg(ui.bg_selected).border_1().border_color(ui.border))
                .when(!on, |d| d.hover(move |d| d.bg(ui.bg_hover)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Overlay::ThemePicker { query, dark, .. } = &this.overlay {
                        let (q, d) = (query.clone(), *dark);
                        this.overlay = Overlay::ThemePicker { query: q, selected: i, dark: d };
                        this.preview_theme(&name, window, cx);
                    }
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().truncate().text_size(px(13.)).text_color(ui.text_primary).child(t.name.to_string()))
                        .child(div().text_size(px(11.)).text_color(if on && tag == "previewing" { ui.accent } else { ui.text_tertiary }).child(tag)),
                )
                .child(swatch_bar(t)),
        );
    }
    if hits.is_empty() {
        list = list.child(div().p(px(10.)).text_size(px(13.)).text_color(ui.text_tertiary).child("No matching themes"));
    }

    let header = div()
        .h(px(52.))
        .flex_none()
        .px(px(16.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(ui.border)
        .children(ws.settings_return.map(|_| {
            div()
                .id("theme-back")
                .h(px(26.))
                .pl(px(2.))
                .pr(px(6.))
                .ml(px(-6.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(2.))
                .text_size(px(12.5))
                .text_color(ui.text_secondary)
                .cursor_pointer()
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.end_theme_preview(None, window, cx);
                    if let Some(section) = this.settings_return.take() {
                        this.overlay = Overlay::Settings { section };
                        cx.notify();
                    }
                }))
                .child(crate::icons::back(ui.text_secondary))
                .child("Settings")
        }))
        .child(if query.is_empty() {
            div().flex_1().flex().items_center().gap(px(8.)).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Themes")).child(div().text_size(px(11.5)).text_color(ui.text_disabled).child("type to filter"))
        } else {
            div().flex_1().flex().items_center().gap(px(8.)).child(crate::icons::search(ui.text_tertiary)).child(div().text_size(px(14.)).text_color(ui.text_primary).child(format!("{query}▏")))
        });

    let slot_note = div()
        .mx(px(2.))
        .mb(px(6.))
        .px(px(8.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(ui.accent.opacity(0.08))
        .text_size(px(11.5))
        .text_color(ui.text_secondary)
        .flex()
        .gap(px(4.))
        .child("Assigning to")
        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(if dark { "Dark slot" } else { "Light slot" }))
        .child(if dark { "· dark themes" } else { "· light themes" });

    let drop_zone = div()
        .rounded(px(9.))
        .border_1()
        .border_dashed()
        .border_color(ui.text_disabled)
        .p(px(14.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .child(div().text_size(px(12.5)).text_color(ui.text_secondary).child("Drop a theme file on the window to import"))
        .child(
            div()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(ui.text_tertiary)
                .text_center()
                .child("iTerm2 .itermcolors · Ghostty config · Alacritty .toml / .yml · Windows Terminal .json · base16 .yaml"),
        );
    let toggle_row = div()
        .mt(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(11.5))
        .text_color(ui.text_tertiary)
        .child("Toggle dark/light")
        .child(div().flex_1())
        .child(div().font_family("JetBrains Mono").text_color(ui.text_secondary).child(crate::keymap::display_for(&crate::actions::ToggleAppearance, &ws.config.keys)));
    let import_btn = div()
        .id("theme-import")
        .mt(px(8.))
        .h(px(32.))
        .flex_none()
        .rounded(px(7.))
        .bg(ui.bg_control)
        .border_1()
        .border_color(ui.border)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ui.text_primary)
        .cursor_pointer()
        .hover(move |d| d.bg(ui.bg_hover))
        .on_click(cx.listener(|_, _, window, cx| {
            let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Import".into()) });
            cx.spawn_in(window, async move |this, cx| {
                if let Ok(Ok(Some(paths))) = rx.await {
                    let _ = this.update_in(cx, |ws, window, cx| ws.import_into_picker(&paths, window, cx));
                }
            })
            .detach();
        }))
        .child("Import theme…");

    let left = div()
        .w(px(300.))
        .flex_none()
        .rounded_l(px(11.))
        .border_r_1()
        .border_color(ui.border)
        .flex()
        .flex_col()
        .child(header)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .px(px(8.))
                .py(px(10.))
                .flex()
                .flex_col()
                .child(slot_note)
                .child(div().flex_1().min_h_0().overflow_hidden().child(list))
                .child(drop_zone)
                .child(toggle_row)
                .child(import_btn),
        );

    // Right: appearance and both slots, then the preview.
    let mode = ws.chosen_mode();
    let label = |text: &'static str, color: Hsla| div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(text);
    let mode_seg = |m: AppearanceMode, text: &'static str| {
        let on = mode == m;
        div()
            .id(text)
            .px(px(12.))
            .py(px(5.))
            .rounded(px(5.))
            .text_size(px(12.5))
            .text_color(if on { ui.text_primary } else { ui.text_tertiary })
            .when(on, |d| d.bg(ui.bg_selected).font_weight(FontWeight::MEDIUM))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.set_mode(m, window, cx)))
            .child(text)
    };
    let slot_btn = |for_dark: bool| {
        let choosing = for_dark == dark;
        let t = if choosing { hits.get(selected).copied().unwrap_or(ws.saved_slot(for_dark)) } else { ws.saved_slot(for_dark) };
        let a = t.ansi;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(
                match (for_dark, choosing) {
                    (true, true) => "DARK SLOT · CHOOSING",
                    (true, false) => "DARK SLOT",
                    (false, true) => "LIGHT SLOT · CHOOSING",
                    (false, false) => "LIGHT SLOT",
                },
                if choosing { ui.text_secondary } else { ui.text_tertiary },
            ))
            .child(
                div()
                    .id(if for_dark { "slot-dark" } else { "slot-light" })
                    .h(px(34.))
                    .px(px(10.))
                    .rounded(px(7.))
                    .bg(ui.bg_sidebar)
                    .border_1()
                    .border_color(if choosing { ui.accent } else { ui.border })
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Overlay::ThemePicker { query, dark, .. } = &this.overlay {
                            if *dark != for_dark {
                                let q = query.clone();
                                let slot = this.saved_slot(for_dark).name.clone();
                                let selected = results(&q, for_dark).iter().position(|t| t.name == slot).unwrap_or(0);
                                this.overlay = Overlay::ThemePicker { query: q, selected, dark: for_dark };
                                cx.notify();
                            }
                        }
                    }))
                    .child(
                        div()
                            .w(px(36.))
                            .h(px(14.))
                            .flex_none()
                            .flex()
                            .rounded(px(3.))
                            .overflow_hidden()
                            .border_1()
                            .border_color(c(t.fg).opacity(0.15))
                            .child(div().flex_grow().flex_basis(px(12.)).h_full().bg(c(t.bg)))
                            .children([a[1], a[2], a[4]].map(|v| div().w(px(6.)).h_full().bg(c(v)))),
                    )
                    .child(div().flex_1().truncate().text_size(px(13.)).text_color(ui.text_primary).child(t.name.to_string()))
                    .child(div().text_size(px(11.)).text_color(if choosing { ui.accent } else { ui.text_tertiary }).child(if choosing { "previewing" } else { "⇥ change" })),
            )
    };
    let preview = hits.get(selected).copied().unwrap_or(ws.saved_slot(dark));
    let ansi_grid = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(label("ANSI 16 · NORMAL / BRIGHT", ui.text_tertiary))
        .child(
            div().w(px(360.)).flex().flex_col().gap(px(4.)).children([0usize, 8].map(|row| {
                div().flex().gap(px(4.)).children((row..row + 8).map(|i| div().flex_1().h(px(24.)).rounded(px(4.)).border_1().border_color(c(preview.fg).opacity(0.1)).bg(c(preview.ansi[i]))))
            })),
        )
        .child(
            div()
                .flex()
                .gap(px(14.))
                .text_size(px(11.))
                .font_family("JetBrains Mono")
                .text_color(ui.text_tertiary)
                .child(format!("fg {}", hex(preview.fg)))
                .child(format!("bg {}", hex(preview.bg)))
                .child(format!("cursor {}", hex(preview.cursor)))
                .child(format!("sel {}", hex(preview.selection))),
        );
    let keys = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(label("KEYS", ui.text_tertiary))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .text_size(px(12.5))
                .text_color(ui.text_secondary)
                .child("↑↓ preview · ↵ keep · esc go back")
                .child("⇥ switch slot · type to filter")
                .child(div().text_color(ui.text_tertiary).child("The sidebar, bars and dialogs take their colors from the theme too.")),
        );
    let right = div()
        .flex_1()
        .min_w_0()
        .px(px(22.))
        .py(px(18.))
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            div()
                .flex()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(label("APPEARANCE", ui.text_tertiary))
                        .child(
                            div()
                                .flex()
                                .p(px(2.))
                                .rounded(px(7.))
                                .bg(ui.bg_sidebar)
                                .border_1()
                                .border_color(ui.border)
                                .child(mode_seg(AppearanceMode::Dark, "Dark"))
                                .child(mode_seg(AppearanceMode::Light, "Light"))
                                .child(mode_seg(AppearanceMode::System, "System")),
                        ),
                )
                .child(slot_btn(true))
                .child(slot_btn(false)),
        )
        .child(
            div()
                .mt(px(-6.))
                .text_size(px(12.))
                .text_color(ui.text_tertiary)
                .child("System follows macOS and switches live. ⌘⇧L flips between Dark and Light."),
        )
        .child(mini_window(preview, ws.started))
        .child(div().flex().gap(px(28.)).child(ansi_grid).child(keys));

    div()
        .w(px(w))
        .h(px(h))
        .mt(px(48.))
        .rounded(px(12.))
        .overflow_hidden()
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(left)
        .child(right)
        .into_any_element()
}
