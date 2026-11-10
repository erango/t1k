//! Command palette (⌘⇧P): every action, theme, Settings page and common toggle, searchable.

use gpui::{div, prelude::*, px, Action, AnyElement, Context, MouseButton, SharedString, Window};

use crate::actions::*;
use crate::overlay::{fuzzy, Overlay};
use crate::settings::Section;
use crate::workspace::Workspace;

pub enum Run {
    Action(Box<dyn Action>),
    Settings(Section),
    /// Bring tabs over from other terminals.
    Import,
    Match,
    /// Flip a boolean in config.toml.
    Toggle(&'static [&'static str], bool),
}

pub struct Command {
    pub label: String,
    pub keys: String,
    pub run: Run,
}

fn act(label: &str, keys: &'static str, a: impl Action) -> Command {
    Command { label: label.into(), keys: keys.into(), run: Run::Action(Box::new(a)) }
}

/// All commands, in a sensible browsing order.
pub fn commands(ws: &Workspace) -> Vec<Command> {
    use t1k_terminal::view as term;
    let c = &ws.config;
    let mut v = vec![
        act("New Tab", "⌘T", NewTab),
        act("Close Tab", "⌘W", CloseTab),
        act("Find Tab…", "⌘P", ToggleSwitcher),
        act("Overview", "⌘O", ToggleOverview),
        act("Jump to Next Needing You", "⌘J", JumpToNeeds),
        act("Jump to Previous Needing You", "⌘⇧J", JumpToNeedsBack),
        act("Back", "⌘[", GoBack),
        act("Forward", "⌘]", GoForward),
        act("Next Tab", "⌘⇧]", NextTab),
        act("Previous Tab", "⌘⇧[", PrevTab),
        act("Rename Tab…", "⌘⇧R", RenameTab),
        act("Split Right", "⌘D", SplitRight),
        act("Split Down", "⌘⇧D", SplitDown),
        act("Zoom Pane", "⌘⇧↵", ZoomPane),
        act("Pane to the Right", "⌘⌥→", FocusPaneRight),
        act("Pane to the Left", "⌘⌥←", FocusPaneLeft),
        act("Find in Terminal…", "⌘F", term::Find),
        act("Find Next", "⌘G", term::FindNext),
        act("Find Previous", "⌘⇧G", term::FindPrevious),
        act("Open Link…", "⌘⇧U", term::ShowLinkHints),
        act("Previous Prompt", "⌘↑", term::PreviousPrompt),
        act("Next Prompt", "⌘↓", term::NextPrompt),
        act("Scroll to Bottom", "⌘⇧↓", term::ScrollToBottom),
        act("Clear Terminal", "⌘K", term::ClearScrollback),
        act("Copy", "⌘C", term::Copy),
        act("Paste", "⌘V", term::Paste),
        act("Copy Last Command's Output", "⌘⇧C", term::CopyLastOutput),
        act("Copy Last Command", "", term::CopyLastCommand),
        act("Select Last Command's Output", "", term::SelectLastOutput),
        act("Toggle Sidebar", "⌘\\", ToggleSidebar),
        act("Toggle Outputs Panel", "⌘⇧O", ToggleOutputs),
        act(if c.terminal.secure_input { "Turn Off Secure Keyboard Entry" } else { "Turn On Secure Keyboard Entry" }, "", ToggleSecureInput),
        act("Toggle Dark / Light", "⌘⇧L", ToggleAppearance),
        act("Follow System Appearance", "", FollowSystemAppearance),
        act("Bigger Text", "⌘+", IncreaseFontSize),
        act("Smaller Text", "⌘−", DecreaseFontSize),
        act("Actual Size", "⌘0", ResetFontSize),
        act("Settings", "⌘,", OpenSettings),
        act("Set up T1K…", "", RunSetup),
        act("Choose Theme…", "", ShowThemePicker),
        act("Check for Updates…", "", CheckForUpdates),
        act("Report a Bug…", "", ReportBug),
    ];
    let found = ws.import_found.as_ref().filter(|f| !f.is_empty()).map(|f| format!("Found {} in {}", f.len(), f[0].source.label())).unwrap_or_default();
    v.push(Command { label: "Import Tabs from Other Terminals…".into(), keys: found, run: Run::Import });
    v.push(Command { label: "Match Your Terminal's Font and Colors…".into(), keys: String::new(), run: Run::Match });
    for s in Section::ALL {
        v.push(Command { label: format!("Settings: {}", s.label()), keys: String::new(), run: Run::Settings(s) });
    }
    let toggles: [(&str, &'static [&'static str], bool); 6] = [
        ("Copy on Select", &["terminal", "copy_on_select"], c.terminal.copy_on_select),
        ("Option as Meta", &["terminal", "option_as_meta"], c.terminal.option_as_meta),
        ("Shell Integration (new tabs)", &["terminal", "shell_integration"], c.terminal.shell_integration),
        ("Banner When a Session Finishes", &["notify", "done"], c.notify.done),
        ("Banner When a Tab Needs You", &["notify", "needs_input"], c.notify.needs_input),
        ("Dock Badge", &["notify", "dock_badge"], c.notify.dock_badge),
    ];
    for (label, keys, on) in toggles {
        let verb = if on { "Turn Off" } else { "Turn On" };
        v.push(Command { label: format!("{verb} {label}"), keys: String::new(), run: Run::Toggle(keys, on) });
    }
    for t in t1k_themes::all() {
        let slot = if t.appearance == t1k_themes::Appearance::Light { "light" } else { "dark" };
        v.push(Command {
            label: format!("Theme: {} ({slot})", t.name),
            keys: String::new(),
            run: Run::Action(Box::new(SelectTheme { name: t.name.to_string().into() })),
        });
    }
    // Show the shortcuts in effect (the literals above are only the defaults).
    for cmd in &mut v {
        if let Run::Action(a) = &cmd.run {
            if crate::keymap::ALL.iter().any(|b| (b.action)().partial_eq(a.as_ref())) {
                cmd.keys = crate::keymap::display_for(a.as_ref(), &c.keys);
            }
        }
    }
    v
}

/// Indices into `cmds` matching `query`, best first; everything in order when empty.
pub fn results(cmds: &[Command], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..cmds.len()).collect();
    }
    let hays: Vec<String> = cmds.iter().map(|c| c.label.clone()).collect();
    let mut r = fuzzy(query, &hays);
    r.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    r.into_iter().map(|(_, i)| i).collect()
}

impl Workspace {
    /// Close the palette and run a command. Actions go to the terminal, so terminal commands
    /// (Find, Copy...) work as if typed there.
    pub fn run_command(&mut self, run: Run, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlay(window, cx);
        match run {
            Run::Action(a) => window.dispatch_action(a, cx),
            Run::Settings(section) => self.open_overlay(Overlay::Settings { section }, window, cx),
            Run::Import => self.open_import(window, cx),
            Run::Match => self.open_match(window, cx),
            Run::Toggle(keys, on) => crate::config::set_value(keys, toml_edit::Value::from(!on)),
        }
    }
}

pub fn render(ws: &Workspace, query: &str, selected: usize, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = ws.ui;
    let cmds = commands(ws);
    let hits = results(&cmds, query);
    // Keep the selection in view: show a window of 12 rows around it.
    let start = selected.saturating_sub(11);
    let rows = hits.iter().enumerate().skip(start).take(12).map(|(n, &i)| {
        let c = &cmds[i];
        let is_sel = n == selected;
        div()
            .id(SharedString::from(format!("cmd-{i}")))
            .h(px(32.))
            .px(px(12.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(10.))
            .when(is_sel, |d| d.bg(ui.bg_selected))
            .hover(move |d| d.bg(ui.bg_hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                let cmds = commands(this);
                if let Some(c) = cmds.into_iter().nth(i) {
                    this.run_command(c.run, window, cx);
                }
            }))
            .child(div().flex_1().truncate().text_size(px(14.)).text_color(ui.text_primary).child(c.label.clone()))
            .child(crate::overlay::shortcut(ui, &c.keys))
    });
    let panel = div()
        .w(px(620.))
        .mt(px(90.))
        .p(px(8.))
        .rounded(px(12.))
        .bg(ui.bg_raised)
        .border_1()
        .border_color(ui.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(2.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .h(px(44.))
                .px(px(12.))
                .flex()
                .items_center()
                .text_size(px(16.))
                .text_color(if query.is_empty() { ui.text_tertiary } else { ui.text_primary })
                .child(if query.is_empty() { "Type a command, setting or theme…".to_string() } else { format!("{query}▏") }),
        )
        .child(div().h(px(1.)).bg(ui.border).mb(px(4.)))
        .children(rows)
        .when(hits.is_empty(), |d| d.child(div().p(px(14.)).text_size(px(13.)).text_color(ui.text_tertiary).child("No matching commands")));
    panel.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(label: &str) -> Command {
        Command { label: label.into(), keys: String::new(), run: Run::Toggle(&["x"], false) }
    }

    #[test]
    fn finds_by_words() {
        let cmds = vec![cmd("New Tab"), cmd("Split Right"), cmd("Theme: Dracula (dark)"), cmd("Toggle Sidebar")];
        assert_eq!(results(&cmds, "drac").first(), Some(&2));
        assert_eq!(results(&cmds, "split").first(), Some(&1));
        assert_eq!(results(&cmds, "").len(), 4);
        assert!(results(&cmds, "zzz").is_empty());
    }
}
