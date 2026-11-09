//! Copy one command's output (canvas v3-ao): ⌘⇧C copies the last finished command's output
//! with a toast; resting on a prompt line shows "Copy output" / "Copy command" chips at its
//! right end. Commands come from shell integration's prompt marks (`crate::blocks`).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use gpui::{div, prelude::*, px, AnyElement, ClipboardItem, Context, MouseButton, SharedString, Task, Window};

use super::{hsla, mix, CopyLastCommand, CopyLastOutput, SelectLastOutput, TerminalView};
use crate::blocks::{self, Block};

const SHOW: Duration = Duration::from_millis(300);
const HIDE: Duration = Duration::from_millis(200);
const FLASH: Duration = Duration::from_millis(400);
const COPIED: Duration = Duration::from_millis(1200);
const TOAST: Duration = Duration::from_millis(1500);
const TOAST_LONG: Duration = Duration::from_secs(3);

const NO_MARKS: &str = "Turn on shell integration to copy command output.";
const ALT_SCREEN: &str = "Quit the full-screen program to copy command output.";
const NONE_YET: &str = "No command has finished yet.";

#[derive(Default)]
pub(super) struct CopyUi {
    /// The prompt line under the pointer (a grid line).
    target: Option<i32>,
    /// The prompt line whose chips show, and its command.
    shown: Option<(i32, Block)>,
    task: Option<Task<()>>,
    /// Chip feedback: the prompt line, whether it was "Copy command", the label, when.
    copied: Option<(i32, bool, String, Instant)>,
    /// Output rows that were just copied, flashed briefly.
    flash: Option<((i32, i32), Instant)>,
    feedback_task: Option<Task<()>>,
    toast: Option<(String, bool, Instant, Duration)>,
    toast_task: Option<Task<()>>,
}

/// "48212" → "48,212".
fn count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn lines(n: usize) -> String {
    format!("{} {}", count(n), if n == 1 { "line" } else { "lines" })
}

impl TerminalView {
    /// Pointer moved: chips follow the prompt line it rests on, after `SHOW`, and go `HIDE`
    /// after it leaves. Never while dragging a selection.
    pub(super) fn copy_hover(&mut self, row: Option<usize>, dragging: bool, cx: &mut Context<Self>) {
        let line = if dragging { None } else { row.and_then(|r| self.prompt_line_at(r)) };
        if line == self.copy_ui.target {
            return;
        }
        self.copy_ui.target = line;
        // Another line of the prompt already showing: move at once.
        if let (Some(l), Some((_, b))) = (line, self.copy_ui.shown) {
            if (b.prompt.0..=b.prompt.1).contains(&l) {
                self.copy_ui.shown = Some((l, b));
                self.copy_ui.task = None;
                cx.notify();
                return;
            }
        }
        let wait = if line.is_some() { SHOW } else { HIDE };
        self.copy_ui.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |v, cx| v.settle_chips(line, cx)).ok();
        }));
    }

    fn settle_chips(&mut self, line: Option<i32>, cx: &mut Context<Self>) {
        if self.copy_ui.target != line {
            return;
        }
        let shown = line.and_then(|l| {
            let term = self.terminal.term.lock();
            blocks::blocks(&term).into_iter().find(|b| (b.prompt.0..=b.prompt.1).contains(&l)).map(|b| (l, b))
        });
        if shown != self.copy_ui.shown {
            self.copy_ui.shown = shown;
            cx.notify();
        }
    }

    /// Rows the frame is shifted by (prompt gravity) plus the scrollback offset.
    fn row_offset<T>(&self, term: &alacritty_terminal::Term<T>) -> i32 {
        term.grid().display_offset() as i32 + self.terminal.shift.load(Ordering::Relaxed) as i32
    }

    /// The grid line at viewport `row`, when it's a prompt line on the main screen.
    fn prompt_line_at(&self, row: usize) -> Option<i32> {
        let term = self.terminal.term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return None;
        }
        let line = row as i32 - self.row_offset(&term);
        (line >= term.topmost_line().0 && line <= term.bottommost_line().0 && blocks::marked(&term, line)).then_some(line)
    }

    /// The command ⌘⇧C means: the last finished one, else the one running.
    fn last_block(&self) -> Result<Block, &'static str> {
        let term = self.terminal.term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return Err(ALT_SCREEN);
        }
        let all = blocks::blocks(&term);
        if let Some(b) = all.iter().rev().find(|b| !b.running).or(all.last()) {
            return Ok(*b);
        }
        let any = (term.topmost_line().0..=term.bottommost_line().0).any(|l| blocks::marked(&term, l));
        Err(if any { NONE_YET } else { NO_MARKS })
    }

    fn texts(&self, b: &Block) -> (String, String) {
        let term = self.terminal.term.lock();
        (blocks::output_text(&term, b), blocks::command_text(&term, b))
    }

    pub(super) fn copy_last_output(&mut self, _: &CopyLastOutput, _: &mut Window, cx: &mut Context<Self>) {
        let b = match self.last_block() {
            Ok(b) => b,
            Err(msg) => return self.toast(msg.into(), false, TOAST_LONG, cx),
        };
        let (text, cmd) = self.texts(&b);
        let name = cmd.split_whitespace().next().unwrap_or("The command").to_string();
        if text.is_empty() {
            return self.toast(format!("{name} printed nothing to copy"), false, TOAST, cx);
        }
        let n = text.lines().count();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let so_far = if b.running { "so far " } else { "" };
        self.toast(format!("Copied the output {so_far}of {name} ({})", lines(n)), true, TOAST, cx);
        self.flash(&b, cx);
    }

    pub(super) fn copy_last_command(&mut self, _: &CopyLastCommand, _: &mut Window, cx: &mut Context<Self>) {
        let b = match self.last_block() {
            Ok(b) => b,
            Err(msg) => return self.toast(msg.into(), false, TOAST_LONG, cx),
        };
        let (_, cmd) = self.texts(&b);
        if cmd.is_empty() {
            return self.toast(NONE_YET.into(), false, TOAST, cx);
        }
        let shown: String = if cmd.chars().count() > 48 { cmd.chars().take(47).chain(['…']).collect() } else { cmd.clone() };
        cx.write_to_clipboard(ClipboardItem::new_string(cmd));
        self.toast(format!("Copied {}", shown.replace('\n', " ")), true, TOAST, cx);
    }

    /// Selects the output, so ⌘C or a drag works as usual.
    pub(super) fn select_last_output(&mut self, _: &SelectLastOutput, _: &mut Window, cx: &mut Context<Self>) {
        let b = match self.last_block() {
            Ok(b) => b,
            Err(msg) => return self.toast(msg.into(), false, TOAST_LONG, cx),
        };
        let Some((s, e)) = b.output else {
            let (_, cmd) = self.texts(&b);
            let name = cmd.split_whitespace().next().unwrap_or("The command").to_string();
            return self.toast(format!("{name} printed nothing to select"), false, TOAST, cx);
        };
        let mut term = self.terminal.term.lock();
        let last = Column(term.columns().saturating_sub(1));
        let mut sel = Selection::new(SelectionType::Simple, GridPoint::new(Line(s), Column(0)), Side::Left);
        sel.update(GridPoint::new(Line(e), last), Side::Right);
        term.selection = Some(sel);
        drop(term);
        cx.notify();
    }

    fn chip_copy(&mut self, line: i32, b: Block, command: bool, cx: &mut Context<Self>) {
        let (out, cmd) = self.texts(&b);
        let label = if command {
            cx.write_to_clipboard(ClipboardItem::new_string(cmd));
            "Copied".to_string()
        } else {
            let n = out.lines().count();
            cx.write_to_clipboard(ClipboardItem::new_string(out));
            self.flash(&b, cx);
            format!("Copied {}", lines(n))
        };
        self.copy_ui.copied = Some((line, command, label, Instant::now()));
        self.copy_ui.feedback_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH).await;
            this.update(cx, |_, cx| cx.notify()).ok();
            cx.background_executor().timer(COPIED - FLASH).await;
            this.update(cx, |v, cx| {
                v.copy_ui.copied = None;
                // The pointer left while it said "Copied": hide now.
                if v.copy_ui.target.is_none() {
                    v.copy_ui.shown = None;
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn flash(&mut self, b: &Block, cx: &mut Context<Self>) {
        if let Some(rows) = b.output {
            self.copy_ui.flash = Some((rows, Instant::now()));
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FLASH).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
            cx.notify();
        }
    }

    fn toast(&mut self, text: String, ok: bool, dur: Duration, cx: &mut Context<Self>) {
        self.copy_ui.toast = Some((text, ok, Instant::now(), dur));
        self.copy_ui.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(dur).await;
            this.update(cx, |v, cx| {
                v.copy_ui.toast = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// `T1K_DEBUG_COPY=chips|toast`: about 2.5 s after launch, shows the chips on the first
    /// command's prompt line, or the toast ⌘⇧C would show (without touching the clipboard).
    pub(super) fn debug_copy(cx: &mut Context<Self>) {
        let Ok(what) = std::env::var("T1K_DEBUG_COPY") else { return };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(2500)).await;
            this.update(cx, |v, cx| {
                let first = {
                    let term = v.terminal.term.lock();
                    blocks::blocks(&term).first().copied()
                };
                match (what.as_str(), first) {
                    ("chips", Some(b)) => {
                        v.copy_ui.target = Some(b.prompt.1);
                        v.copy_ui.shown = Some((b.prompt.1, b));
                    }
                    ("toast", _) => match v.last_block() {
                        Ok(b) => {
                            let (text, cmd) = v.texts(&b);
                            let name = cmd.split_whitespace().next().unwrap_or("The command").to_string();
                            v.toast(format!("Copied the output of {name} ({})", lines(text.lines().count())), true, Duration::from_secs(60), cx);
                        }
                        Err(m) => v.toast(m.into(), false, Duration::from_secs(60), cx),
                    },
                    _ => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The line tint, chips, flash and toast, positioned in the pane (padding included).
    pub(super) fn copy_overlay(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let p = &self.palette;
        let fg = hsla(p.foreground);
        let green = hsla(p.ansi[2]);
        let (rest, hover) = (mix(p.background, p.foreground, 0.10), mix(p.background, p.foreground, 0.16));
        if let Some((_, g)) = self.layout {
            let (pad, ch) = (f32::from(self.style.padding), g.cell_height);
            let (offset, alt) = {
                let term = self.terminal.term.lock();
                (self.row_offset(&term), term.mode().contains(TermMode::ALT_SCREEN))
            };
            let row = |line: i32| line + offset;
            if let Some(((s, e), _)) = self.copy_ui.flash.filter(|(_, t)| t.elapsed() < FLASH) {
                let (r0, r1) = (row(s).max(0), row(e).min(g.rows as i32 - 1));
                if r1 >= r0 {
                    out.push(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(pad + r0 as f32 * ch))
                            .h(px((r1 - r0 + 1) as f32 * ch))
                            .bg(hsla(p.link).opacity(0.10))
                            .into_any_element(),
                    );
                }
            }
            if let Some((line, b)) = self.copy_ui.shown.filter(|_| !alt) {
                let r = row(line);
                if (0..g.rows as i32).contains(&r) {
                    let y = pad + r as f32 * ch;
                    out.push(div().absolute().left_0().right_0().top(px(y - 1.)).h(px(ch + 2.)).bg(fg.opacity(0.05)).into_any_element());
                    let copied = self.copy_ui.copied.as_ref().filter(|c| c.0 == line);
                    let chip = |command: bool, label: &str| {
                        let done = copied.filter(|c| c.1 == command);
                        let text: SharedString = match done {
                            Some(c) => format!("✓ {}", c.2).into(),
                            None => label.to_string().into(),
                        };
                        div()
                            .id(if command { "copy-command" } else { "copy-output" })
                            .h(px(20.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .text_size(px(11.5))
                            .bg(rest)
                            .hover(move |d| d.bg(hover))
                            .border_1()
                            .border_color(fg.opacity(0.12))
                            .text_color(if done.is_some() { green } else { fg })
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |v, _, _, cx| v.chip_copy(line, b, command, cx)))
                            .child(text)
                    };
                    let output_label = if b.running { "Copy output so far" } else { "Copy output" };
                    out.push(
                        div()
                            .absolute()
                            .right(px(16.))
                            .top(px(y + (ch - 20.) / 2.))
                            .flex()
                            .gap(px(6.))
                            .children(b.output.is_some().then(|| chip(false, output_label)))
                            .child(chip(true, "Copy command"))
                            .into_any_element(),
                    );
                }
            }
        }
        if let Some((text, ok, _, _)) = self.copy_ui.toast.as_ref().filter(|t| t.2.elapsed() < t.3) {
            out.push(
                div()
                    .absolute()
                    .bottom(px(16.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .h(px(32.))
                            .px(px(12.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .bg(rest)
                            .border_1()
                            .border_color(fg.opacity(0.12))
                            .shadow_md()
                            .text_size(px(12.))
                            .text_color(if *ok { fg } else { fg.opacity(0.7) })
                            .when(*ok, |d| d.child(div().text_color(green).child("✓")))
                            .child(text.clone()),
                    )
                    .into_any_element(),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_with_separators() {
        assert_eq!(super::count(48212), "48,212");
        assert_eq!(super::count(999), "999");
        assert_eq!(super::lines(1), "1 line");
        assert_eq!(super::lines(1000), "1,000 lines");
    }
}
