//! Settings › Notifications & access, the sidebar's "Notifications are off" card and its
//! dot (canvas v3-bc). What T1K may do on this Mac comes from `permissions`; the sound theme
//! from `sound`.

use std::time::{Duration, Instant};

use gpui::{deferred, div, prelude::*, px, AnyElement, Context, FontWeight, SharedString};
use t1k_themes::UiColors;

use crate::permissions::{self, FullDisk, Notif};
use crate::sound;
use crate::workspace::Workspace;

pub struct AccessState {
    /// The card was put off until this time (unix seconds).
    pub snooze: Option<u64>,
    /// "Don't show again": the card stays away; the dot stays until notifications work.
    pub never: bool,
    card_menu: bool,
    sound_menu: bool,
    /// A preview playing: which theme, and until when (the wave shows meanwhile).
    playing: Option<(String, Instant)>,
    /// The menu item a preview last played for, so hovering doesn't repeat it.
    hovered: Option<&'static str>,
}

impl AccessState {
    pub fn new(snooze: Option<u64>, never: bool) -> Self {
        Self { snooze, never, card_menu: false, sound_menu: false, playing: None, hovered: None }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The notifications state, or T1K_DEBUG_PERMS=off|notset|nobanners|on for screenshots.
fn notif() -> Notif {
    match std::env::var("T1K_DEBUG_PERMS").as_deref() {
        Ok("off") => Notif::Off,
        Ok("notset") => Notif::NotSetUp,
        Ok("nobanners") => Notif::NoBanners,
        Ok("on") => Notif::On,
        _ => permissions::notifications(),
    }
}

/// Full Disk Access, or T1K_DEBUG_FDA=no|since|on.
fn full_disk() -> FullDisk {
    match std::env::var("T1K_DEBUG_FDA").as_deref() {
        Ok("no") => FullDisk::NotGranted,
        Ok("since") => FullDisk::GrantedSinceLaunch,
        Ok("on") => FullDisk::On,
        _ => permissions::full_disk(),
    }
}

pub fn needs_fixing() -> bool {
    notif().needs_fixing()
}

/// The 6 pt amber dot: notifications need fixing.
pub fn dot(ui: UiColors) -> impl IntoElement {
    div().flex_none().size(px(6.)).rounded_full().bg(ui.status_needs)
}

fn button(ui: UiColors, id: &'static str, label: &'static str, primary: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(26.))
        .px(px(10.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_size(px(12.))
        .cursor_pointer()
        .when(primary, |d| d.bg(ui.accent).text_color(ui.on_accent).font_weight(FontWeight::MEDIUM).hover(|d| d.opacity(0.88)))
        .when(!primary, move |d| d.bg(ui.bg_control).text_color(ui.text_primary).hover(move |d| d.bg(ui.accent.opacity(0.18))))
        .child(label)
}

/// "● Off", "✓ On", "○ Not granted": the state beside a row's button.
fn state(ui: UiColors, mark: Mark, text: &'static str) -> impl IntoElement {
    let glyph = match mark {
        Mark::Needs => dot(ui).into_any_element(),
        Mark::Hollow => div().flex_none().size(px(7.)).rounded_full().border_1().border_color(ui.text_tertiary).into_any_element(),
        Mark::Check => div().text_size(px(12.)).text_color(ui.text_secondary).child("✓").into_any_element(),
    };
    div().flex_none().flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(ui.text_secondary).child(glyph).child(text)
}

#[derive(Clone, Copy)]
enum Mark {
    Needs,
    Hollow,
    Check,
}

/// A row: title, what it's for, a note on what to do, then state and action on the right.
fn row(ui: UiColors, title: &'static str, desc: &'static str, note: Option<AnyElement>, right: AnyElement, last: bool) -> impl IntoElement {
    div()
        .px(px(14.))
        .py(px(12.))
        .flex()
        .items_center()
        .gap(px(16.))
        .when(!last, |d| d.border_b_1().border_color(ui.border))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().text_size(px(13.)).text_color(ui.text_primary).child(title))
                .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_tertiary).child(desc))
                .children(note),
        )
        .child(right)
}

fn note(ui: UiColors, text: &'static str) -> AnyElement {
    div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_secondary).child(text).into_any_element()
}

/// Open the fix for notifications: the prompt if it was never answered, else System Settings.
fn fix_notifications(n: Notif, cx: &mut gpui::App) {
    if n == Notif::NotSetUp {
        permissions::ask_again();
    } else {
        cx.open_url(permissions::NOTIFICATIONS_URL);
    }
}

pub fn section(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let n = notif();
    let dev = !crate::platform::BUNDLED.load(std::sync::atomic::Ordering::SeqCst) && std::env::var_os("T1K_DEBUG_PERMS").is_none();

    // Notifications.
    let (notif_note, notif_right): (Option<AnyElement>, AnyElement) = if dev {
        (None, div().text_size(px(12.)).text_color(ui.text_tertiary).child("Dev build: uses osascript, not checked").into_any_element())
    } else {
        let open = |label: &'static str| button(ui, "access-notif-fix", label, false).on_click(move |_, _, cx| fix_notifications(n, cx));
        match n {
            Notif::On => (None, state(ui, Mark::Check, "On").into_any_element()),
            Notif::Off => (
                Some(note(ui, "Turn on \u{201c}Allow notifications\u{201d} for T1K.")),
                div().flex().items_center().gap(px(12.)).child(state(ui, Mark::Needs, "Off")).child(open("Open System Settings")).into_any_element(),
            ),
            Notif::NotSetUp => (
                None,
                div().flex().items_center().gap(px(12.)).child(state(ui, Mark::Needs, "Not set up")).child(open("Ask again")).into_any_element(),
            ),
            Notif::NoBanners => (
                Some(note(ui, "Alert style is set to None, so nothing appears. Choose Banners.")),
                div().flex().items_center().gap(px(12.)).child(state(ui, Mark::Needs, "On, but no banners")).child(open("Open System Settings")).into_any_element(),
            ),
            Notif::Unknown => (None, div().text_size(px(12.)).text_color(ui.text_tertiary).child("Checking…").into_any_element()),
        }
    };

    // Sound.
    let theme = ws.config.notify.sound.clone();
    let silent = theme == sound::SILENT;
    let off = n.needs_fixing() && !dev;
    let sound_desc = if silent {
        "Banners still show, without sound."
    } else if off {
        "Sounds play with notifications, which are off."
    } else {
        "Needs you and errors get the brighter sound; done, reminders and program banners the softer one. Focus silences both."
    };
    let sound_note = off.then(|| {
        div()
            .id("access-sound-fix")
            .text_size(px(12.))
            .text_color(ui.link)
            .cursor_pointer()
            .on_click(move |_, _, cx| fix_notifications(n, cx))
            .child("Turn on notifications ↑")
            .into_any_element()
    });
    // While notifications are off, the control dims; the link to fix them doesn't.
    let sound_right = div().when(off, |d| d.opacity(0.5)).child(sound_control(ws, &theme, cx)).into_any_element();

    // Full Disk Access.
    let fd = full_disk();
    let (fd_note, fd_right): (Option<AnyElement>, AnyElement) = match fd {
        FullDisk::NotGranted => (
            Some(note(ui, "Click + and add T1K, or turn on its switch. Then restart T1K.")),
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(state(ui, Mark::Hollow, "Not granted"))
                .child(button(ui, "access-fda-open", "Open System Settings", false).on_click(|_, _, cx| cx.open_url(permissions::FULL_DISK_URL)))
                .into_any_element(),
        ),
        FullDisk::GrantedSinceLaunch => (
            Some(note(ui, "Takes effect after a restart. Your tabs come back.")),
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(state(ui, Mark::Check, "Granted"))
                .child(button(ui, "access-restart", "Restart T1K", true).on_click(|_, window, cx| permissions::restart(window, cx)))
                .into_any_element(),
        ),
        FullDisk::On => (None, state(ui, Mark::Check, "On").into_any_element()),
    };

    let rows = div()
        .rounded(px(9.))
        .bg(ui.bg_terminal)
        .border_1()
        .border_color(ui.border)
        .flex()
        .flex_col()
        .child(row(ui, "Notifications", "Banners when an agent needs you, hits an error, or a reminder is due.", notif_note, notif_right, false))
        .child(row(ui, "Sound", sound_desc, sound_note, sound_right, false))
        .child(row(
            ui,
            "Full Disk Access",
            "Lets shells and agents read Desktop, Documents, Downloads and iCloud Drive without asking per folder. Optional.",
            fd_note,
            fd_right,
            true,
        ));
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(rows)
        .child(
            div()
                .text_size(px(12.))
                .text_color(ui.text_tertiary)
                .child("Not needed: Accessibility (the quick terminal\u{2019}s shortcut needs no permission) and Automation."),
        )
        .into_any_element()
}

/// "All set" beside the section title, when everything is on.
pub fn all_set() -> bool {
    notif() == Notif::On && full_disk() == FullDisk::On
}

impl Workspace {
    /// Play the theme's pair (alert, then soft) and show the wave while it plays.
    fn preview_sound(&mut self, theme: &str, cx: &mut Context<Self>) {
        sound::play(theme, sound::Kind::Alert);
        let gap = sound::pair_length(theme).min(Duration::from_secs(4));
        let until = Instant::now() + gap;
        self.access.playing = Some((theme.to_string(), until));
        let t = theme.to_string();
        let soft_at = Duration::from_secs_f32(0.9_f32.min(gap.as_secs_f32()) + 0.25);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(soft_at).await;
            let _ = this.update(cx, |this, _| {
                if this.access.playing.as_ref().is_some_and(|(p, _)| *p == t) && t != sound::SYSTEM {
                    sound::play(&t, sound::Kind::Soft);
                }
            });
            cx.background_executor().timer(gap.saturating_sub(soft_at)).await;
            let _ = this.update(cx, |this, cx| {
                if this.access.playing.as_ref().is_some_and(|(_, u)| *u <= Instant::now()) {
                    this.access.playing = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

fn wave(ui: UiColors) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(14.))
        .flex()
        .items_center()
        .gap(px(2.))
        .children([5., 9., 13., 8., 11., 6., 4.].into_iter().map(move |h| div().w(px(2.)).h(px(h)).rounded(px(1.)).bg(ui.accent)))
}

fn sound_control(ws: &Workspace, theme: &str, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let silent = theme == sound::SILENT;
    let open = ws.access.sound_menu;
    let playing = ws.access.playing.as_ref().is_some_and(|(_, u)| *u > Instant::now());
    let menu_button = div()
        .id("access-sound-menu")
        .w(px(150.))
        .h(px(26.))
        .px(px(9.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(7.))
        .bg(ui.bg_control)
        .border_1()
        .border_color(if open { ui.accent } else { gpui::transparent_black() })
        .cursor_pointer()
        .hover(move |d| d.bg(ui.accent.opacity(0.18)))
        .on_click(cx.listener(|this, _, _, cx| {
            this.access.sound_menu = !this.access.sound_menu;
            this.access.hovered = None;
            cx.notify();
        }))
        .child(crate::icons::speaker_at(ui.text_secondary, 13., silent))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.5)).text_color(ui.text_primary).child(sound::label(theme)))
        .child(crate::icons::caret(ui.text_tertiary, open));
    let mut wrap = div().relative().child(menu_button);
    if open {
        wrap = wrap.child(div().absolute().top(px(30.)).right_0().child(deferred(gpui::anchored().snap_to_window_with_margin(px(8.)).child(menu(ws, theme, cx))).with_priority(2)));
    }
    let t = theme.to_string();
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .when(playing, |d| d.child(wave(ui)))
        .child(wrap)
        .when(!silent, |d| {
            d.child(
                div()
                    .id("access-sound-play")
                    .size(px(24.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(ui.bg_control)
                    .text_size(px(10.))
                    .text_color(ui.text_secondary)
                    .cursor_pointer()
                    .hover(move |d| d.bg(ui.accent.opacity(0.18)))
                    .on_click(cx.listener(move |this, _, _, cx| this.preview_sound(&t, cx)))
                    .child("▶"),
            )
        })
}

fn menu(ws: &Workspace, current: &str, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ui = ws.ui;
    let mut list = div()
        .id("access-sound-list")
        .w(px(250.))
        .p(px(4.))
        .rounded(px(8.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.access.sound_menu = false;
            cx.notify();
        }));
    for (i, (id, label, blurb)) in sound::choices().into_iter().enumerate() {
        if id == sound::SYSTEM {
            list = list.child(div().my(px(4.)).mx(px(6.)).h(px(1.)).bg(ui.border));
        }
        let on = id == current;
        list = list.child(
            div()
                .id(SharedString::from(format!("sound-{i}")))
                .h(px(40.))
                .px(px(8.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .hover(move |d| d.bg(ui.bg_hover))
                .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    // Hovering an item previews it, once per visit.
                    if *over && this.access.hovered != Some(id) {
                        this.access.hovered = Some(id);
                        if id != sound::SILENT {
                            sound::play(id, sound::Kind::Alert);
                        }
                    }
                    let _ = cx;
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    crate::config::set_value(&["notify", "sound"], id.into());
                    this.config.notify.sound = id.to_string();
                    sound::set_theme(id);
                    this.access.sound_menu = false;
                    if id != sound::SILENT {
                        this.preview_sound(id, cx);
                    }
                    cx.notify();
                }))
                .child(div().w(px(12.)).flex_none().text_size(px(12.)).text_color(ui.accent).child(if on { "✓" } else { "" }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(12.5)).text_color(ui.text_primary).child(label))
                        .child(div().text_size(px(11.)).text_color(ui.text_tertiary).child(blurb)),
                ),
        );
    }
    list
}

/// The once-a-day card at the bottom of the sidebar while notifications need fixing.
pub fn card(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = ws.ui;
    let busy = !matches!(ws.overlay, crate::overlay::Overlay::None) || ws.tabs.is_empty();
    let snoozed = ws.access.snooze.is_some_and(|t| t > now_secs());
    if !needs_fixing() || ws.access.never || snoozed || busy {
        return None;
    }
    let menu_open = ws.access.card_menu;
    let item = |id: &'static str, label: &'static str| {
        div()
            .id(id)
            .h(px(26.))
            .px(px(8.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(12.))
            .text_color(ui.text_primary)
            .cursor_pointer()
            .hover(move |d| d.bg(ui.bg_hover))
            .child(label)
    };
    let settings = |this: &mut Workspace, window: &mut gpui::Window, cx: &mut Context<Workspace>| {
        this.access.card_menu = false;
        this.open_overlay(crate::overlay::Overlay::Settings { section: crate::settings::Section::Access }, window, cx);
    };
    Some(
        div()
            .absolute()
            .left(px(8.))
            .right(px(8.))
            .bottom(px(8.))
            .p(px(10.))
            .rounded(px(8.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(dot(ui))
                    .child(div().flex_1().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("Notifications are off"))
                    .child(
                        div()
                            .id("notif-card-x")
                            .size(px(18.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |d| d.bg(ui.bg_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.access.card_menu = !this.access.card_menu;
                                cx.notify();
                            }))
                            .child(crate::icons::close(ui.text_tertiary)),
                    ),
            )
            .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_secondary).child("You won\u{2019}t hear when an agent needs you."))
            .when(!menu_open, |d| {
                d.child(
                    div()
                        .id("notif-card-on")
                        .text_size(px(12.))
                        .text_color(ui.link)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| settings(this, window, cx)))
                        .child("Turn on\u{2026}"),
                )
            })
            .when(menu_open, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .pt(px(2.))
                        .child(item("notif-card-tomorrow", "Remind me tomorrow").on_click(cx.listener(|this, _, _, cx| {
                            this.access.snooze = Some(now_secs() + 24 * 3600);
                            this.access.card_menu = false;
                            this.save();
                            cx.notify();
                        })))
                        .child(item("notif-card-never", "Don\u{2019}t show again").on_click(cx.listener(|this, _, _, cx| {
                            this.access.never = true;
                            this.access.card_menu = false;
                            this.save();
                            cx.notify();
                        })))
                        .child(item("notif-card-open", "Open Notifications & access").on_click(cx.listener(move |this, _, window, cx| settings(this, window, cx)))),
                )
            })
            .into_any_element(),
    )
}
