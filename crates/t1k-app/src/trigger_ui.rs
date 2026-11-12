//! Settings › Triggers and the trigger editor (canvas v3-az). Triggers live in config.toml as
//! [[triggers]]; the list writes them back on every change, so the file stays the truth.
//!
//! The editor can ask the cheap-model helper to write the pattern from a description
//! ("lines where cargo test says a test failed").

use std::time::Instant;

use gpui::{div, prelude::*, px, AnyElement, Context, Div, FontWeight, KeyDownEvent, MouseButton, SharedString, Stateful, Window};
use t1k_themes::UiColors;

use crate::config::{ActionCfg, TriggerCfg};
use crate::input::{self, Key, Sel};
use crate::overlay::Overlay;
use crate::triggers::{action_label, compile, describe_then, describe_when, ACTIONS};
use crate::workspace::Workspace;

/// A text field in the editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Name,
    Pattern,
    Test,
    Describe,
    Program,
    Groups,
    Timeout,
    Cooldown,
    RunLimit,
    Arg(usize),
    Label(usize),
}

#[derive(Clone, Debug)]
pub struct Editor {
    /// Which trigger it edits (None: a new one).
    pub index: Option<usize>,
    pub t: TriggerCfg,
    pub focus: Option<Field>,
    pub sel: Sel,
    pub test: String,
    pub describe: String,
    /// The helper is writing the pattern.
    pub asking: Option<Instant>,
    pub helper_note: Option<String>,
    /// Which action's kind list is open.
    pub picking: Option<usize>,
    /// Save asked: "This trigger types into terminals by itself".
    pub warn: bool,
    /// ⌘Z / ⌘⇧Z in the fields: (field, its text, selection) before each change, and after
    /// an undo.
    undo: Vec<(Field, String, Sel)>,
    redo: Vec<(Field, String, Sel)>,
    /// The field being typed into word by word (one undo step per word).
    typing: Option<Field>,
    groups_text: String,
    timeout_text: String,
    cooldown_text: String,
    limit_text: String,
}

impl Editor {
    pub fn new(index: Option<usize>, t: TriggerCfg) -> Self {
        let groups_text = t.groups.join(", ");
        let (timeout_text, cooldown_text, limit_text) = (t.timeout.to_string(), t.cooldown.to_string(), t.run_limit.to_string());
        Self { index, t, focus: Some(Field::Name), sel: Sel::default(), test: String::new(), describe: String::new(), asking: None, helper_note: None, picking: None, warn: false, undo: Vec::new(), redo: Vec::new(), typing: None, groups_text, timeout_text, cooldown_text, limit_text }
    }

    fn text_mut(&mut self, f: Field) -> Option<&mut String> {
        Some(match f {
            Field::Name => &mut self.t.name,
            Field::Pattern => &mut self.t.pattern,
            Field::Test => &mut self.test,
            Field::Describe => &mut self.describe,
            Field::Program => &mut self.t.program,
            Field::Groups => &mut self.groups_text,
            Field::Timeout => &mut self.timeout_text,
            Field::Cooldown => &mut self.cooldown_text,
            Field::RunLimit => &mut self.limit_text,
            Field::Arg(i) => &mut self.t.actions.get_mut(i)?.arg,
            Field::Label(i) => &mut self.t.actions.get_mut(i)?.label,
        })
    }

    fn text(&self, f: Field) -> String {
        match f {
            Field::Name => self.t.name.clone(),
            Field::Pattern => self.t.pattern.clone(),
            Field::Test => self.test.clone(),
            Field::Describe => self.describe.clone(),
            Field::Program => self.t.program.clone(),
            Field::Groups => self.groups_text.clone(),
            Field::Timeout => self.timeout_text.clone(),
            Field::Cooldown => self.cooldown_text.clone(),
            Field::RunLimit => self.limit_text.clone(),
            Field::Arg(i) => self.t.actions.get(i).map(|a| a.arg.clone()).unwrap_or_default(),
            Field::Label(i) => self.t.actions.get(i).map(|a| a.label.clone()).unwrap_or_default(),
        }
    }

    /// The fields in Tab order.
    fn order(&self) -> Vec<Field> {
        let mut v = vec![Field::Name];
        if self.t.when != "exit" {
            v.push(Field::Describe);
            v.push(Field::Pattern);
            v.push(Field::Test);
        }
        match self.t.scope.as_str() {
            "groups" => v.push(Field::Groups),
            "program" => v.push(Field::Program),
            _ => {}
        }
        for i in 0..self.t.actions.len() {
            if takes_arg(&self.t.actions[i].kind) {
                v.push(Field::Arg(i));
            }
            if self.t.mode == "ask" {
                v.push(Field::Label(i));
            }
        }
        if self.t.mode == "ask" {
            v.push(Field::Timeout);
        }
        v.push(Field::Cooldown);
        if self.runs() {
            v.push(Field::RunLimit);
        }
        v
    }

    /// It runs background commands (their time limit applies).
    fn runs(&self) -> bool {
        self.t.actions.iter().any(|a| a.kind == "run")
    }

    /// What's typed into the number and list fields, back onto the trigger.
    fn settle(&mut self) {
        self.t.groups = self.groups_text.split(',').map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect();
        self.t.timeout = self.timeout_text.trim().parse().unwrap_or(30).clamp(1, 3600);
        self.t.cooldown = self.cooldown_text.trim().parse().unwrap_or(10).min(86400);
        self.t.run_limit = self.limit_text.trim().parse().unwrap_or(10).clamp(1, 1440);
        if self.t.default >= self.t.actions.len() as i64 {
            self.t.default = -1;
        }
    }

    /// Typing into the terminal or running commands by itself.
    fn risky(&self) -> bool {
        self.t.mode == "auto" && self.t.actions.iter().any(|a| matches!(a.kind.as_str(), "send" | "prompt" | "run"))
    }
}

fn takes_arg(kind: &str) -> bool {
    ACTIONS.iter().find(|a| a.0 == kind).is_some_and(|a| !a.2.is_empty())
}

impl Workspace {
    pub fn edit_trigger(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let t = index.and_then(|i| self.config.triggers.get(i).cloned()).unwrap_or_default();
        let mut ed = Editor::new(index, t);
        ed.sel = Sel::all(&ed.t.name);
        self.open_overlay(Overlay::TriggerEdit(Box::new(ed)), window, cx);
    }

    fn write_triggers(&mut self, list: Vec<TriggerCfg>, cx: &mut Context<Self>) {
        crate::config::set_triggers(&list);
        self.config.triggers = list;
        cx.notify();
    }

    fn save_trigger(&mut self, mut ed: Editor, window: &mut Window, cx: &mut Context<Self>) {
        ed.settle();
        let mut list = self.config.triggers.clone();
        match ed.index.filter(|&i| i < list.len()) {
            Some(i) => list[i] = ed.t,
            None => list.push(ed.t),
        }
        self.write_triggers(list, cx);
        self.open_overlay(Overlay::Settings { section: crate::settings::Section::Triggers }, window, cx);
    }

    /// Keys while the editor is open.
    pub fn trigger_edit_key(&mut self, mut ed: Box<Editor>, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        let ks = &ev.keystroke;
        if ks.key == "escape" && ed.picking.is_some() {
            ed.picking = None;
            return Some(Overlay::TriggerEdit(ed));
        }
        if ks.key == "escape" && ed.warn {
            ed.warn = false;
            return Some(Overlay::TriggerEdit(ed));
        }
        // Esc leaves the field first; with none focused it closes the editor.
        if ks.key == "escape" && ed.focus.is_some() {
            ed.focus = None;
            return Some(Overlay::TriggerEdit(ed));
        }
        if ks.key == "escape" {
            self.open_overlay(Overlay::Settings { section: crate::settings::Section::Triggers }, window, cx);
            return None;
        }
        // ⌘Z / ⌘⇧Z: undo and redo typing in the fields.
        if ks.modifiers.platform && ks.key == "z" {
            let popped = if ks.modifiers.shift { ed.redo.pop() } else { ed.undo.pop() };
            if let Some((f, text, sel)) = popped {
                let now = ed.text(f);
                let cur = ed.sel;
                if ks.modifiers.shift {
                    ed.undo.push((f, now, cur));
                } else {
                    ed.redo.push((f, now, cur));
                }
                if let Some(t) = ed.text_mut(f) {
                    *t = text;
                }
                ed.focus = Some(f);
                ed.sel = sel;
                ed.typing = None;
            }
            return Some(Overlay::TriggerEdit(ed));
        }
        // ⌘S / ⌘↵ save.
        if ks.modifiers.platform && (ks.key == "s" || ks.key == "enter") {
            return self.try_save(ed, window, cx);
        }
        if ks.key == "tab" {
            let order = ed.order();
            let at = ed.focus.and_then(|f| order.iter().position(|x| *x == f)).unwrap_or(0);
            let n = order.len() as isize;
            let next = order[((at as isize + if ks.modifiers.shift { -1 } else { 1 }).rem_euclid(n)) as usize];
            ed.focus = Some(next);
            ed.sel = Sel::all(&ed.text(next));
            return Some(Overlay::TriggerEdit(ed));
        }
        let Some(f) = ed.focus else { return Some(Overlay::TriggerEdit(ed)) };
        let before = (ed.text(f), ed.sel);
        let mut sel = ed.sel;
        let key = match ed.text_mut(f) {
            Some(text) => input::edit(text, &mut sel, ev, cx),
            None => Key::Ignored,
        };
        ed.sel = sel;
        if ed.text(f) != before.0 {
            // Typing within a word is one step; a space, a paste or a delete starts another.
            let word = !ks.modifiers.platform && ks.key_char.as_deref().is_some_and(|c| !c.is_empty() && c.chars().all(char::is_alphanumeric));
            let joins = word && ed.typing == Some(f);
            ed.typing = word.then_some(f);
            if !joins {
                ed.undo.push((f, before.0, before.1));
                if ed.undo.len() > 200 {
                    ed.undo.remove(0);
                }
            }
            ed.redo.clear();
        }
        if matches!(key, Key::Submit) && f == Field::Describe {
            self.helper_regex(&mut ed, cx);
        }
        Some(Overlay::TriggerEdit(ed))
    }

    fn try_save(&mut self, mut ed: Box<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Option<Overlay> {
        if compile(&ed.t.pattern).is_err() || ed.t.actions.is_empty() {
            return Some(Overlay::TriggerEdit(ed));
        }
        if ed.risky() && !ed.warn && ed.t.enabled {
            ed.warn = true;
            return Some(Overlay::TriggerEdit(ed));
        }
        self.save_trigger(*ed, window, cx);
        None
    }

    /// Ask the helper for a pattern from the description (and the test line, as an example).
    fn helper_regex(&mut self, ed: &mut Editor, cx: &mut Context<Self>) {
        if ed.describe.trim().is_empty() {
            return;
        }
        if self.config.helper.backend().is_none() {
            ed.helper_note = Some("Turn on the helper in Settings › Agents to have patterns written for you.".into());
            return;
        }
        ed.asking = Some(Instant::now());
        ed.helper_note = None;
        let what = if ed.t.when == "agent" { "what a coding agent asks for (e.g. \"Claude needs your permission to use Bash · Running cargo test\")" } else { "one line of terminal output, with colors removed" };
        let example = if ed.test.trim().is_empty() { String::new() } else { format!("\nAn example of a line it should match:\n{}\n", ed.test) };
        let prompt = format!(
            "Write one regular expression (Rust regex crate syntax) that matches {what}, described as:\n{}\n{example}\nUse capture groups for the parts someone may want to reuse (a file, a URL, a number). Prefer anchors and word boundaries over .* where you can. Reply with the regex alone: no quotes, slashes, backticks or explanation.",
            ed.describe.trim()
        );
        self.helper_job_regex(prompt, cx);
    }

    /// The helper's answer: the pattern, if it compiles.
    pub fn helper_regex_done(&mut self, text: &str, cx: &mut Context<Self>) {
        let Overlay::TriggerEdit(ed) = &mut self.overlay else { return };
        ed.asking = None;
        let pat = text.trim().trim_matches('`').trim().trim_start_matches("regex").trim().trim_matches('/').trim().to_string();
        match compile(&pat) {
            Ok(Some(_)) => {
                ed.t.pattern = pat;
                ed.helper_note = Some("Written by the helper. Check it against a line in Test.".into());
                ed.focus = Some(Field::Test);
                ed.sel = Sel::all(&ed.test);
            }
            _ => ed.helper_note = Some("The helper's answer wasn't a pattern I can use. Try describing it differently.".into()),
        }
        cx.notify();
    }
}

// ---------- drawing ----------

fn label(ui: UiColors, text: &'static str) -> Div {
    div().w(px(110.)).flex_none().pt(px(6.)).text_size(px(12.)).text_color(ui.text_secondary).child(text)
}

fn seg_row<T: Clone + PartialEq + 'static>(
    ui: UiColors,
    id: &str,
    current: &T,
    opts: &[(T, &'static str)],
    cx: &mut Context<Workspace>,
    set: impl Fn(&mut Editor, T) + Clone + 'static,
) -> Div {
    let mut d = div().flex().flex_wrap().gap(px(4.));
    for (n, (v, l)) in opts.iter().enumerate() {
        let on = v == current;
        let (v, set) = (v.clone(), set.clone());
        d = d.child(
            div()
                .id(SharedString::from(format!("{id}-{n}")))
                .h(px(26.))
                .px(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |d| d.bg(ui.bg_selected).text_color(ui.text_primary).font_weight(FontWeight::MEDIUM))
                .when(!on, |d| d.bg(ui.bg_control).text_color(ui.text_secondary))
                .hover(move |d| d.text_color(ui.text_primary))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                        set(ed, v.clone());
                        cx.notify();
                    }
                }))
                .child(*l),
        );
    }
    d
}

fn field(ws: &Workspace, ed: &Editor, f: Field, placeholder: &str, mono: bool, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let ui = ws.ui;
    let text = ed.text(f);
    let focused = ed.focus == Some(f);
    let bad = f == Field::Pattern && compile(&text).is_err();
    let r = ed.sel.range();
    let (lo, hi) = (r.start.min(text.len()), r.end.min(text.len()));
    let caret = || div().w(px(1.5)).h(px(15.)).flex_none().bg(ui.accent);
    let body = if text.is_empty() && !focused {
        div().text_color(ui.text_disabled).child(placeholder.to_string())
    } else if focused {
        div()
            .flex()
            .items_center()
            .whitespace_nowrap()
            .child(div().child(text[..lo].to_string()))
            .when(lo == hi, |d| d.child(caret()))
            .child(div().bg(ui.accent.opacity(0.3)).child(text[lo..hi].to_string()))
            .child(div().child(text[hi..].to_string()))
    } else {
        div().truncate().child(text)
    };
    div()
        .id(SharedString::from(format!("tf-{f:?}")))
        .flex_1()
        .min_w_0()
        .h(px(30.))
        .px(px(9.))
        .rounded(px(6.))
        .bg(ui.bg_control)
        .border_1()
        .border_color(if bad { ui.status_error } else if focused { ui.accent.opacity(0.6) } else { ui.border })
        .flex()
        .items_center()
        .overflow_hidden()
        .cursor(gpui::CursorStyle::IBeam)
        .text_size(px(12.5))
        .text_color(ui.text_primary)
        .when(mono, |d| d.font_family("JetBrains Mono"))
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                ed.focus = Some(f);
                let len = ed.text(f).len();
                ed.sel = Sel { anchor: len, cursor: len };
                ed.picking = None;
                cx.notify();
            }
        }))
        .child(body)
}

fn small_button(ui: UiColors, id: impl Into<SharedString>, text: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id.into())
        .h(px(24.))
        .px(px(8.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(ui.text_secondary)
        .bg(ui.bg_control)
        .cursor_pointer()
        .hover(move |d| d.text_color(ui.text_primary))
        .child(text.into())
}

/// The test line against the pattern: what matched and its groups, or why not.
fn test_result(ui: UiColors, ed: &Editor) -> Option<Div> {
    if ed.test.is_empty() {
        return None;
    }
    let re = match compile(&ed.t.pattern) {
        Err(_) => return None,
        Ok(None) => return Some(div().text_size(px(11.5)).text_color(ui.text_tertiary).child("Matches every line (no pattern)")),
        Ok(Some(re)) => re,
    };
    let Some(caps) = re.captures(&ed.test) else {
        return Some(div().text_size(px(11.5)).text_color(ui.text_tertiary).child("No match"));
    };
    let m = caps.get(0).unwrap();
    let line = div()
        .flex()
        .font_family("JetBrains Mono")
        .text_size(px(11.5))
        .text_color(ui.text_secondary)
        .child(ed.test[..m.start()].to_string())
        .child(div().bg(ui.accent.opacity(0.3)).text_color(ui.text_primary).child(m.as_str().to_string()))
        .child(ed.test[m.end()..].to_string());
    let groups: Vec<String> = (1..caps.len()).map(|i| format!("${i} = {}", caps.get(i).map(|g| g.as_str()).unwrap_or(""))).collect();
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(line)
            .when(!groups.is_empty(), |d| d.child(div().font_family("JetBrains Mono").text_size(px(11.)).text_color(ui.text_tertiary).child(groups.join("   ")))),
    )
}

pub fn editor(ws: &Workspace, ed: &Editor, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let ui = ws.ui;
    let row = |l: &'static str, content: Div| div().flex().gap(px(10.)).child(label(ui, l)).child(div().flex_1().min_w_0().flex().flex_col().gap(px(6.)).child(content));
    let mut body = div().flex().flex_col().gap(px(12.));
    body = body.child(row("Name", div().flex().child(field(ws, ed, Field::Name, "What it's for", false, cx))));
    body = body.child(row(
        "When",
        seg_row(ui, "when", &ed.t.when, &[("output".to_string(), "A line is printed"), ("agent".to_string(), "An agent event"), ("exit".to_string(), "A command fails")], cx, |e, v| e.t.when = v),
    ));
    if ed.t.when == "agent" {
        body = body.child(row(
            "Event",
            seg_row(ui, "event", &ed.t.event, &[("needs".to_string(), "Needs you"), ("done".to_string(), "Finished"), ("error".to_string(), "Error"), ("watching_ends".to_string(), "Watching ends")], cx, |e, v| e.t.event = v),
        ));
    }
    if ed.t.when != "exit" {
        // Describe it in words; the helper writes the pattern.
        let busy = ed.asking.is_some();
        let describe = div()
            .flex()
            .gap(px(6.))
            .child(field(ws, ed, Field::Describe, "Describe it: \u{201c}cargo test says a test failed\u{201d}", false, cx))
            .child(
                small_button(ui, "tr-write", if busy { "Writing…" } else { "Write the pattern" })
                    .when(busy, |d| d.opacity(0.6))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mut ov = std::mem::replace(&mut this.overlay, Overlay::None);
                        if let Overlay::TriggerEdit(ed) = &mut ov {
                            if ed.asking.is_none() {
                                this.helper_regex(ed, cx);
                            }
                        }
                        this.overlay = ov;
                        cx.notify();
                    })),
            );
        let err = compile(&ed.t.pattern).err();
        let pattern = div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(describe)
            .children(ed.helper_note.clone().map(|n| div().text_size(px(11.5)).text_color(ui.text_tertiary).child(n)))
            .child(div().flex().child(field(ws, ed, Field::Pattern, "A regex; empty matches anything", true, cx)))
            .children(err.map(|e| div().text_size(px(11.5)).text_color(ui.status_error).child(e)))
            .child(div().flex().child(field(ws, ed, Field::Test, "Test it: paste a line here", true, cx)))
            .children(test_result(ui, ed));
        body = body.child(row("Pattern", pattern));
    }
    let scope = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(seg_row(ui, "scope", &ed.t.scope, &[("all".to_string(), "All tabs"), ("groups".to_string(), "Groups…"), ("program".to_string(), "While a program runs…")], cx, |e, v| e.t.scope = v))
        .when(ed.t.scope == "groups", |d| d.child(div().flex().child(field(ws, ed, Field::Groups, "Group names, comma separated", false, cx))))
        .when(ed.t.scope == "program", |d| d.child(div().flex().child(field(ws, ed, Field::Program, "Program name, e.g. terraform", true, cx))));
    body = body.child(row("In", scope));
    body = body.child(row(
        "Mode",
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(seg_row(ui, "mode", &ed.t.mode, &[("ask".to_string(), "Ask me"), ("auto".to_string(), "Run automatically")], cx, |e, v| e.t.mode = v))
            .child(div().text_size(px(11.5)).text_color(ui.text_tertiary).child(if ed.t.mode == "ask" {
                "Shows the actions as buttons under the terminal (⌃1, ⌃2…). The default runs if you don't choose in time."
            } else {
                "Runs every action, in order, as soon as it matches."
            })),
    ));

    // Actions, in order.
    let ask = ed.t.mode == "ask";
    let mut acts = div().flex().flex_col().gap(px(6.));
    for (i, a) in ed.t.actions.iter().enumerate() {
        let kind_name = ACTIONS.iter().find(|x| x.0 == a.kind).map(|x| x.1).unwrap_or("Notify");
        let is_default = ed.t.default == i as i64;
        let mut line = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().w(px(18.)).flex_none().text_size(px(11.)).font_family("JetBrains Mono").text_color(ui.text_tertiary).child(if ask { format!("⌃{}", i + 1) } else { format!("{}.", i + 1) }))
            .child(small_button(ui, SharedString::from(format!("tr-kind-{i}")), format!("{kind_name} ▾")).on_click(cx.listener(move |this, _, _, cx| {
                if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                    ed.picking = if ed.picking == Some(i) { None } else { Some(i) };
                    cx.notify();
                }
            })));
        if takes_arg(&a.kind) {
            let hint = ACTIONS.iter().find(|x| x.0 == a.kind).map(|x| x.2).unwrap_or("");
            line = line.child(field(ws, ed, Field::Arg(i), hint, true, cx));
        } else {
            line = line.child(div().flex_1());
        }
        if ask {
            line = line.child(
                small_button(ui, SharedString::from(format!("tr-def-{i}")), if is_default { "● Default" } else { "○ Default" })
                    .when(is_default, |d| d.text_color(ui.text_primary))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                            ed.t.default = if ed.t.default == i as i64 { -1 } else { i as i64 };
                            cx.notify();
                        }
                    })),
            );
        }
        line = line
            .child(small_button(ui, SharedString::from(format!("tr-up-{i}")), "↑").when(i == 0, |d| d.opacity(0.4)).on_click(cx.listener(move |this, _, _, cx| {
                if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                    if i > 0 {
                        ed.t.actions.swap(i, i - 1);
                        if ed.t.default == i as i64 {
                            ed.t.default -= 1;
                        } else if ed.t.default == i as i64 - 1 {
                            ed.t.default += 1;
                        }
                        cx.notify();
                    }
                }
            })))
            .child(small_button(ui, SharedString::from(format!("tr-del-{i}")), "×").on_click(cx.listener(move |this, _, _, cx| {
                if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                    ed.t.actions.remove(i);
                    if ed.t.default == i as i64 {
                        ed.t.default = -1;
                    } else if ed.t.default > i as i64 {
                        ed.t.default -= 1;
                    }
                    ed.focus = None;
                    ed.picking = None;
                    cx.notify();
                }
            })));
        acts = acts.child(line);
        if ask {
            acts = acts.child(div().flex().gap(px(6.)).pl(px(24.)).child(field(ws, ed, Field::Label(i), &format!("Button label (empty: {})", action_label(&ActionCfg { label: String::new(), ..a.clone() })), false, cx)));
        }
        if ed.picking == Some(i) {
            let mut kinds = div().flex().flex_wrap().gap(px(4.)).pl(px(24.));
            for (k, name, _) in ACTIONS {
                let k = k.to_string();
                kinds = kinds.child(small_button(ui, SharedString::from(format!("tr-k-{i}-{k}")), *name).when(a.kind == k, |d| d.text_color(ui.text_primary).bg(ui.bg_selected)).on_click(cx.listener(move |this, _, _, cx| {
                    if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                        if let Some(a) = ed.t.actions.get_mut(i) {
                            a.kind = k.clone();
                        }
                        ed.picking = None;
                        cx.notify();
                    }
                })));
            }
            acts = acts.child(kinds);
        }
    }
    acts = acts.child(div().flex().child(small_button(ui, "tr-add", "+ Add action").on_click(cx.listener(|this, _, _, cx| {
        if let Overlay::TriggerEdit(ed) = &mut this.overlay {
            ed.t.actions.push(ActionCfg { kind: "notify".into(), ..Default::default() });
            ed.picking = Some(ed.t.actions.len() - 1);
            cx.notify();
        }
    }))));
    body = body.child(row("Then", acts));
    if ask {
        body = body.child(row(
            "Timeout",
            div().flex().items_center().gap(px(8.)).child(div().w(px(70.)).flex().child(field(ws, ed, Field::Timeout, "30", true, cx))).child(div().text_size(px(12.)).text_color(ui.text_tertiary).child(match usize::try_from(ed.t.default).ok().and_then(|d| ed.t.actions.get(d)) {
                Some(a) => format!("seconds, then: {}", action_label(a)),
                None => "seconds, then: do nothing".into(),
            })),
        ));
    }
    body = body.child(row(
        "Cooldown",
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().w(px(70.)).flex().child(field(ws, ed, Field::Cooldown, "10", true, cx)))
            .child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("seconds before it fires again"))
            .child(seg_row(ui, "cool", &ed.t.cooldown_scope, &[("tab".to_string(), "in the same tab"), ("all".to_string(), "in any tab")], cx, |e, v| e.t.cooldown_scope = v)),
    ));

    if ed.runs() {
        body = body.child(row(
            "Time limit",
            div().flex().items_center().gap(px(8.)).child(div().w(px(70.)).flex().child(field(ws, ed, Field::RunLimit, "10", true, cx))).child(div().text_size(px(12.)).text_color(ui.text_tertiary).child("minutes a background command may run; then it's stopped")),
        ));
    }
    let bad = compile(&ed.t.pattern).is_err() || ed.t.actions.is_empty();
    let footer = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().text_size(px(11.5)).text_color(ui.text_tertiary).child("Tab moves between fields · ⌘Z undoes · ⌘S saves · esc leaves a field, then closes"))
        .child(small_button(ui, "tr-cancel", "Cancel").on_click(cx.listener(|this, _, window, cx| {
            this.open_overlay(Overlay::Settings { section: crate::settings::Section::Triggers }, window, cx);
        })))
        .child(
            small_button(ui, "tr-save", "Save")
                .bg(ui.accent)
                .text_color(ui.on_accent)
                .when(bad, |d| d.opacity(0.45))
                .on_click(cx.listener(|this, _, window, cx| {
                    let ov = std::mem::replace(&mut this.overlay, Overlay::None);
                    if let Overlay::TriggerEdit(ed) = ov {
                        this.overlay = this.try_save(ed, window, cx).unwrap_or(Overlay::None);
                        if matches!(this.overlay, Overlay::None) {
                            this.open_overlay(Overlay::Settings { section: crate::settings::Section::Triggers }, window, cx);
                        }
                    }
                    cx.notify();
                })),
        );
    let warn = ed.warn.then(|| {
        div()
            .p(px(12.))
            .rounded(px(8.))
            .bg(ui.status_needs.opacity(0.1))
            .border_1()
            .border_color(ui.status_needs.opacity(0.5))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child("This trigger types into terminals by itself"))
            .child(div().text_size(px(12.)).line_height(px(17.)).text_color(ui.text_secondary).child(
                "Whatever it sends runs as if you typed it, every time a line matches, in every tab it covers. Narrow where it runs, or let it ask first.",
            ))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(small_button(ui, "tr-make-ask", "Make it ask instead").on_click(cx.listener(|this, _, _, cx| {
                        if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                            ed.t.mode = "ask".into();
                            ed.warn = false;
                            cx.notify();
                        }
                    })))
                    .child(small_button(ui, "tr-save-anyway", "Save anyway").on_click(cx.listener(|this, _, window, cx| {
                        let ov = std::mem::replace(&mut this.overlay, Overlay::None);
                        if let Overlay::TriggerEdit(ed) = ov {
                            this.save_trigger(*ed, window, cx);
                        }
                    }))),
            )
    });
    div()
        .id("trigger-editor")
        .w(px(720.))
        .max_h(px(720.))
        .mt(px(60.))
        .p(px(20.))
        .rounded(px(12.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .font_family(".SystemUIFont")
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(16.))
        // A click anywhere but a field leaves the field (a field's own click focuses it again).
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            if let Overlay::TriggerEdit(ed) = &mut this.overlay {
                if ed.focus.take().is_some() {
                    cx.notify();
                }
            }
        }))
        .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_primary).child(if ed.index.is_some() { "Edit trigger" } else { "New trigger" }))
        .child(body)
        .children(warn)
        .child(footer)
}

/// Settings › Triggers: the list, Pause all, New, and ready-made ones to start from.
pub fn section(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let paused = ws.config.triggers_paused;
    let fired = |name: &str| -> (usize, Option<std::time::SystemTime>) {
        let all: Vec<&crate::triggers::Fired> = ws.trig.log.values().flatten().filter(|f| f.name == name).collect();
        (all.len(), all.iter().map(|f| f.at).max())
    };
    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().text_size(px(12.)).text_color(ui.text_tertiary).child("They run in this order. Each fires at most once per cooldown, per tab or across tabs."))
        .child(small_button(ui, "tr-pause", if paused { "Resume all" } else { "Pause all" }).on_click(cx.listener(|this, _, _, cx| this.toggle_triggers_paused(cx))))
        .child(small_button(ui, "tr-new", "+ New trigger").bg(ui.accent).text_color(ui.on_accent).on_click(cx.listener(|this, _, window, cx| this.edit_trigger(None, window, cx))));
    let mut list = div().flex().flex_col().rounded(px(8.)).border_1().border_color(ui.border).overflow_hidden();
    let n = ws.config.triggers.len();
    for (i, t) in ws.config.triggers.iter().enumerate() {
        let (count, last) = fired(&t.name);
        let when = last.and_then(|l| l.elapsed().ok()).map(|d| if d.as_secs() < 60 { "now".to_string() } else { crate::usage::span(d.as_secs() as i64) });
        let auto = t.mode == "auto";
        let on = t.enabled;
        list = list.child(
            div()
                .id(SharedString::from(format!("tr-row-{i}")))
                .h(px(46.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(10.))
                .when(i + 1 < n, |d| d.border_b_1().border_color(ui.border))
                .when(!on, |d| d.opacity(0.6))
                .cursor_pointer()
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, window, cx| this.edit_trigger(Some(i), window, cx)))
                .child(
                    div()
                        .id(SharedString::from(format!("tr-on-{i}")))
                        .w(px(30.))
                        .h(px(18.))
                        .flex_none()
                        .rounded_full()
                        .bg(if on { ui.accent } else { ui.bg_control })
                        .border_1()
                        .border_color(ui.border)
                        .relative()
                        .child(div().absolute().top(px(2.)).left(px(if on { 14. } else { 2. })).size(px(12.)).rounded_full().bg(if on { ui.on_accent } else { ui.text_tertiary }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let mut list = this.config.triggers.clone();
                            if let Some(t) = list.get_mut(i) {
                                t.enabled = !t.enabled;
                            }
                            this.write_triggers(list, cx);
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().text_size(px(13.)).text_color(ui.text_primary).child(t.name.clone()))
                        .child(div().truncate().text_size(px(11.5)).text_color(ui.text_tertiary).child(format!("{} → {}", describe_when(t), describe_then(t)))),
                )
                .child(div().flex_none().text_size(px(11.)).text_color(ui.text_tertiary).child(if auto { "⚡ Runs automatically" } else { "Asks" }))
                .child(div().w(px(80.)).flex_none().flex().justify_end().font_family("JetBrains Mono").text_size(px(11.)).text_color(ui.text_tertiary).child(match (count, when) {
                    (0, _) => "never".to_string(),
                    (c, Some(w)) => format!("{c}× · {w}"),
                    (c, None) => format!("{c}×"),
                }))
                .child(small_button(ui, SharedString::from(format!("tr-up-row-{i}")), "↑").when(i == 0, |d| d.opacity(0.4)).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if i > 0 {
                        let mut list = this.config.triggers.clone();
                        list.swap(i, i - 1);
                        this.write_triggers(list, cx);
                    }
                })))
                .child(small_button(ui, SharedString::from(format!("tr-dup-{i}")), "Duplicate").on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let mut list = this.config.triggers.clone();
                    if let Some(t) = list.get(i).cloned() {
                        list.insert(i + 1, TriggerCfg { name: format!("{} copy", t.name), ..t });
                        this.write_triggers(list, cx);
                    }
                })))
                .child(small_button(ui, SharedString::from(format!("tr-delete-{i}")), "Delete").on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let mut list = this.config.triggers.clone();
                    if i < list.len() {
                        list.remove(i);
                        this.write_triggers(list, cx);
                    }
                }))),
        );
    }
    if n == 0 {
        list = list.child(div().p(px(14.)).text_size(px(12.5)).text_color(ui.text_tertiary).child("No triggers yet. Start from one below, or make a new one."));
    }
    let mut starters = div().flex().flex_wrap().gap(px(8.));
    for (k, s) in crate::triggers::starters().into_iter().enumerate() {
        let have = ws.config.triggers.iter().any(|t| t.name == s.name);
        let (title, sub) = (s.name.clone(), format!("{} → {}", describe_when(&s), describe_then(&s)));
        starters = starters.child(
            div()
                .id(SharedString::from(format!("tr-start-{k}")))
                .w(px(250.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(ui.bg_control)
                .flex()
                .flex_col()
                .gap(px(4.))
                .cursor_pointer()
                .when(have, |d| d.opacity(0.5))
                .hover(move |d| d.bg(ui.bg_hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !have {
                        // Added switched off, open in the editor to check and turn on.
                        let mut list = this.config.triggers.clone();
                        list.push(s.clone());
                        let i = list.len() - 1;
                        this.write_triggers(list, cx);
                        this.edit_trigger(Some(i), window, cx);
                    }
                }))
                .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(ui.text_primary).child(title))
                .child(div().text_size(px(11.)).line_height(px(15.)).text_color(ui.text_tertiary).line_clamp(2).child(sub)),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(head)
        .child(list)
        .child(div().pt(px(6.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_tertiary).child("START FROM"))
        .child(starters)
        .into_any_element()
}
