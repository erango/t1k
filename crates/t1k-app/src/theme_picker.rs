//! The theme picker: a searchable list for the dark or light slot. Moving through it previews
//! each theme across the whole window; ↵ keeps it, esc goes back to what you had.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, KeyDownEvent, MouseButton, SharedString, Window};
use t1k_themes::Theme;

use crate::input::{self, Key};
use crate::overlay::{fuzzy, Overlay};
use crate::workspace::Workspace;

/// Themes for one slot matching `query`, best first (all, by name, when empty).
pub fn results(query: &str, dark: bool) -> Vec<&'static Theme> {
    let mut all: Vec<&'static Theme> = t1k_themes::all().into_iter().filter(|t| t.is_dark() == dark).collect();
    all.sort_by_key(|t| t.name.to_lowercase());
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
                    return None;
                }
                Key::Submit => {
                    let pick = results(&query, dark).get(selected).map(|t| t.name.to_string());
                    self.end_theme_preview(pick.as_deref(), window, cx);
                    return None;
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
        .p(px(16.))
        .rounded(px(8.))
        .bg(c(t.bg))
        .border_1()
        .border_color(c(t.fg).opacity(0.12))
        .font_family("JetBrains Mono")
        .text_size(px(12.5))
        .line_height(px(20.))
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
        .child(div().h(px(8.)))
        .child(div().flex().gap(px(4.)).children((0..16).map(|i| div().size(px(14.)).rounded(px(3.)).bg(c(a[i])))))
}

pub fn render(ws: &Workspace, query: &str, selected: usize, dark: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let hits = results(query, dark);
    let current = ws.theme_slot(dark).name.clone();
    let tab = |label: &'static str, on: bool, to_dark: bool| {
        div()
            .id(label)
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .text_size(px(12.))
            .text_color(if on { ui.text_primary } else { ui.text_tertiary })
            .when(on, |d| d.bg(ui.bg_selected).font_weight(FontWeight::MEDIUM))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Overlay::ThemePicker { query, dark, .. } = &this.overlay {
                    if *dark != to_dark {
                        let q = query.clone();
                        let slot = this.theme_slot(to_dark).name.clone();
                        let selected = results(&q, to_dark).iter().position(|t| t.name == slot).unwrap_or(0);
                        this.overlay = Overlay::ThemePicker { query: q, selected, dark: to_dark };
                        cx.notify();
                    }
                }
                let _ = window;
            }))
            .child(label)
    };
    // Keep the selection in view: a window of 14 rows around it.
    let start = selected.saturating_sub(7).min(hits.len().saturating_sub(14));
    let rows = hits.iter().enumerate().skip(start).take(14).map(|(i, t)| {
        let on = i == selected;
        let name = t.name.to_string();
        let in_use = t.name == current;
        div()
            .id(SharedString::from(format!("theme-{name}")))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(6.))
            .when(on, |d| d.bg(ui.bg_selected))
            .hover(move |d| d.bg(ui.bg_hover))
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
                    .w(px(40.))
                    .h(px(20.))
                    .flex_none()
                    .rounded(px(4.))
                    .bg(c(t.bg))
                    .border_1()
                    .border_color(c(t.fg).opacity(0.2))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(2.))
                    .children([1usize, 2, 3, 4].map(|k| div().size(px(5.)).rounded_full().bg(c(t.ansi[k])))),
            )
            .child(div().flex_1().truncate().text_size(px(13.)).text_color(ui.text_primary).child(t.name.to_string()))
            .when(in_use, |d| d.child(div().text_size(px(11.)).text_color(ui.text_tertiary).child("in use")))
    });
    let preview = hits.get(selected).copied().unwrap_or(ws.theme_slot(dark));
    div()
        .w(px(860.))
        .h(px(560.))
        .mt(px(70.))
        .rounded(px(12.))
        .overflow_hidden()
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .h(px(48.))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(10.))
                .border_b_1()
                .border_color(ui.border)
                .child(crate::icons::search(ui.text_tertiary))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(15.))
                        .text_color(if query.is_empty() { ui.text_tertiary } else { ui.text_primary })
                        .child(if query.is_empty() { "Search themes".to_string() } else { format!("{query}▏") }),
                )
                .child(div().flex().gap(px(2.)).p(px(2.)).rounded(px(7.)).bg(ui.bg_sidebar).child(tab("Dark", dark, true)).child(tab("Light", !dark, false))),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(
                    div()
                        .w(px(300.))
                        .flex_none()
                        .p(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .border_r_1()
                        .border_color(ui.border)
                        .children(rows)
                        .when(hits.is_empty(), |d| d.child(div().p(px(10.)).text_size(px(13.)).text_color(ui.text_tertiary).child("No matching themes"))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .p(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(px(8.))
                                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(preview.name.to_string()))
                                .child(div().text_size(px(12.)).text_color(ui.text_tertiary).child(if dark { "for dark mode" } else { "for light mode" })),
                        )
                        .child(sample(preview)),
                ),
        )
        .child(
            div()
                .h(px(36.))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(16.))
                .border_t_1()
                .border_color(ui.border)
                .text_size(px(12.))
                .text_color(ui.text_tertiary)
                .child("↑↓ preview")
                .child("↵ keep")
                .child("esc go back")
                .child("⇥ dark / light")
                .child(div().flex_1())
                .child("Drop a Ghostty, iTerm2, Alacritty or base16 file on the window to import"),
        )
        .into_any_element()
}
