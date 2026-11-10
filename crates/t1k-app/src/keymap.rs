//! Keyboard shortcuts: one table of every bindable action with its default keys, plus the
//! user's overrides from `[keys]` in config.toml. Settings › Keybindings edits those overrides;
//! the palette and menus read the effective keys from here.

use std::collections::BTreeMap;

use gpui::{Action, App, KeyBinding, Keystroke};

use crate::actions::*;
use t1k_terminal::view as term;

pub struct Bind {
    /// Key in `[keys]`.
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub defaults: &'static [&'static str],
    /// Only while a terminal has focus.
    pub terminal: bool,
    pub action: fn() -> Box<dyn Action>,
}

macro_rules! b {
    ($id:literal, $label:literal, $group:literal, [$($k:literal),*], $act:expr) => {
        Bind { id: $id, label: $label, group: $group, defaults: &[$($k),*], terminal: false, action: || Box::new($act) }
    };
    (term $id:literal, $label:literal, $group:literal, [$($k:literal),*], $act:expr) => {
        Bind { id: $id, label: $label, group: $group, defaults: &[$($k),*], terminal: true, action: || Box::new($act) }
    };
}

pub const ALL: &[Bind] = &[
    b!("new_tab", "New tab", "Tabs", ["cmd-t"], NewTab),
    b!("close_tab", "Close tab", "Tabs", ["cmd-w"], CloseTab),
    b!("reopen_closed_tab", "Reopen closed tab", "Tabs", ["cmd-shift-t"], ReopenClosedTab),
    b!("undo", "Undo (a tab move while its notice shows, else typing)", "Terminal", ["cmd-z"], Undo),
    b!("undo_move", "Undo moving tabs", "Tabs", [], UndoMove),
    b!("next_tab", "Next tab", "Tabs", ["cmd-shift-]", "ctrl-tab"], NextTab),
    b!("prev_tab", "Previous tab", "Tabs", ["cmd-shift-[", "ctrl-shift-tab"], PrevTab),
    b!("back", "Back", "Tabs", ["cmd-["], GoBack),
    b!("forward", "Forward", "Tabs", ["cmd-]"], GoForward),
    b!("rename_tab", "Rename tab", "Tabs", ["cmd-shift-r"], RenameTab),
    b!("tab_1", "Tab 1", "Tabs", ["cmd-1"], SelectTab1),
    b!("tab_2", "Tab 2", "Tabs", ["cmd-2"], SelectTab2),
    b!("tab_3", "Tab 3", "Tabs", ["cmd-3"], SelectTab3),
    b!("tab_4", "Tab 4", "Tabs", ["cmd-4"], SelectTab4),
    b!("tab_5", "Tab 5", "Tabs", ["cmd-5"], SelectTab5),
    b!("tab_6", "Tab 6", "Tabs", ["cmd-6"], SelectTab6),
    b!("tab_7", "Tab 7", "Tabs", ["cmd-7"], SelectTab7),
    b!("tab_8", "Tab 8", "Tabs", ["cmd-8"], SelectTab8),
    b!("tab_9", "Tab 9", "Tabs", ["cmd-9"], SelectTab9),
    b!("jump_to_needs", "Next tab needing you", "Agents", ["cmd-j"], JumpToNeeds),
    b!("jump_to_needs_back", "Previous tab needing you", "Agents", ["cmd-shift-j"], JumpToNeedsBack),
    b!("find_tab", "Find tab", "Agents", ["cmd-p"], ToggleSwitcher),
    b!("move_group_up", "Move group up", "Tabs", ["cmd-ctrl-up"], MoveGroupUp),
    b!("move_group_down", "Move group down", "Tabs", ["cmd-ctrl-down"], MoveGroupDown),
    b!("overview", "Overview", "Agents", ["cmd-o"], ToggleOverview),
    b!("outputs", "Outputs panel", "Agents", ["cmd-shift-o"], ToggleOutputs),
    b!("command_palette", "Command palette", "Window", ["cmd-shift-p"], ToggleCommandPalette),
    b!("settings", "Settings", "Window", ["cmd-,"], OpenSettings),
    b!("setup", "Set up T1K", "Window", [], RunSetup),
    b!("secure_input", "Secure Keyboard Entry", "Window", [], ToggleSecureInput),
    b!("toggle_sidebar", "Show / hide sidebar", "Window", ["cmd-\\"], ToggleSidebar),
    b!("toggle_appearance", "Dark / light", "Window", ["cmd-shift-l"], ToggleAppearance),
    b!("theme_picker", "Choose theme", "Window", [], ShowThemePicker),
    b!("split_right", "Split right", "Window", ["cmd-d"], SplitRight),
    b!("split_down", "Split down", "Window", ["cmd-shift-d"], SplitDown),
    b!("pane_left", "Pane to the left", "Window", ["cmd-alt-left"], FocusPaneLeft),
    b!("pane_right", "Pane to the right", "Window", ["cmd-alt-right"], FocusPaneRight),
    b!("pane_up", "Pane above", "Window", ["cmd-alt-up"], FocusPaneUp),
    b!("pane_down", "Pane below", "Window", ["cmd-alt-down"], FocusPaneDown),
    b!("next_pane", "Next pane", "Window", [], FocusNextPane),
    b!("toggle_broadcast", "Broadcast input to this tab's panes (or stop)", "Window", ["cmd-alt-i"], ToggleBroadcast),
    b!("zoom_pane", "Zoom pane", "Window", ["cmd-shift-enter"], ZoomPane),
    b!("bigger_text", "Bigger text", "Window", ["cmd-=", "cmd-+"], IncreaseFontSize),
    b!("smaller_text", "Smaller text", "Window", ["cmd--"], DecreaseFontSize),
    b!("actual_size", "Actual size", "Window", ["cmd-0"], ResetFontSize),
    b!("quit", "Quit", "Window", ["cmd-q"], Quit),
    b!(term "copy", "Copy", "Terminal", ["cmd-c"], term::Copy),
    b!(term "paste", "Paste", "Terminal", ["cmd-v"], term::Paste),
    b!(term "copy_last_output", "Copy last command's output", "Terminal", ["cmd-shift-c"], term::CopyLastOutput),
    b!(term "copy_last_command", "Copy last command", "Terminal", [], term::CopyLastCommand),
    b!(term "select_last_output", "Select last command's output", "Terminal", [], term::SelectLastOutput),
    b!(term "clear", "Clear screen and scrollback", "Terminal", ["cmd-k"], term::ClearScrollback),
    b!(term "find", "Find", "Terminal", ["cmd-f"], term::Find),
    b!(term "find_next", "Find next", "Terminal", ["cmd-g"], term::FindNext),
    b!(term "find_previous", "Find previous", "Terminal", ["cmd-shift-g"], term::FindPrevious),
    b!(term "link_hints", "Open link by letter", "Terminal", ["cmd-shift-u"], term::ShowLinkHints),
    b!(term "previous_prompt", "Previous prompt", "Terminal", ["cmd-up"], term::PreviousPrompt),
    b!(term "next_prompt", "Next prompt", "Terminal", ["cmd-down"], term::NextPrompt),
    b!(term "page_up", "Scroll page up", "Terminal", ["shift-pageup"], term::ScrollPageUp),
    b!(term "page_down", "Scroll page down", "Terminal", ["shift-pagedown"], term::ScrollPageDown),
    b!(term "scroll_to_bottom", "Scroll to bottom", "Terminal", ["cmd-shift-down"], term::ScrollToBottom),
];

/// `[keys]` values: one shortcut, several separated by commas, or "" for none.
pub fn parse_list(v: &str) -> Vec<String> {
    v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The keys in effect for `b`.
pub fn keys_for(b: &Bind, overrides: &BTreeMap<String, String>) -> Vec<String> {
    // "switch_pane" was one key for the other pane before there were four; a custom key set
    // for it keeps working as Next pane.
    let old = (b.id == "next_pane").then(|| overrides.get("switch_pane")).flatten();
    match overrides.get(b.id).or(old) {
        Some(v) => parse_list(v),
        None => b.defaults.iter().map(|s| s.to_string()).collect(),
    }
}

#[cfg(test)]
fn is_valid(keys: &str) -> bool {
    !keys.trim().is_empty() && keys.split_whitespace().all(|k| Keystroke::parse(k).is_ok())
}

/// Replace every binding with the defaults plus overrides. Bad overrides are skipped (logged).
pub fn install(cx: &mut App, overrides: &BTreeMap<String, String>) {
    let mut out = Vec::new();
    for b in ALL {
        let ctx = b.terminal.then_some("Terminal");
        for k in keys_for(b, overrides) {
            match KeyBinding::load(
                &k,
                (b.action)(),
                ctx.map(|c| gpui::KeyBindingContextPredicate::parse(c).unwrap().into()),
                false,
                None,
                &gpui::DummyKeyboardMapper,
            ) {
                Ok(kb) => out.push(kb),
                Err(_) => log::warn!("keys.{}: can't read shortcut {k:?}", b.id),
            }
        }
    }
    cx.clear_key_bindings();
    cx.bind_keys(out);
}

/// "cmd-shift-p" → "⌘⇧P"; chords joined by a space.
pub fn display(keys: &str) -> String {
    keys.split_whitespace().map(display_one).collect::<Vec<_>>().join(" ")
}

fn display_one(k: &str) -> String {
    let Ok(ks) = Keystroke::parse(k) else { return k.to_string() };
    let m = ks.modifiers;
    let mut s = String::new();
    if m.control {
        s.push('⌃');
    }
    if m.alt {
        s.push('⌥');
    }
    if m.shift {
        s.push('⇧');
    }
    if m.platform {
        s.push('⌘');
    }
    let key = match ks.key.as_str() {
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "enter" => "↵".into(),
        "escape" => "esc".into(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        "tab" => "⇥".into(),
        "space" => "Space".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "-" => "−".into(),
        k if k.len() == 1 => k.to_uppercase(),
        k if k.starts_with('f') && k[1..].parse::<u8>().is_ok() => k.to_uppercase(),
        k => k.to_string(),
    };
    s + &key
}

/// The first shortcut for an action, for display next to it (palette, menus).
pub fn display_for(action: &dyn Action, overrides: &BTreeMap<String, String>) -> String {
    ALL.iter()
        .find(|b| (b.action)().partial_eq(action))
        .and_then(|b| keys_for(b, overrides).into_iter().next())
        .map(|k| display(&k))
        .unwrap_or_default()
}

/// Turn a recorded keystroke into config syntax ("cmd-shift-p"). None for a bare modifier.
pub fn from_keystroke(ks: &Keystroke) -> Option<String> {
    if ks.key.is_empty() || matches!(ks.key.as_str(), "shift" | "control" | "alt" | "platform" | "function" | "capslock") {
        return None;
    }
    Some(ks.unparse())
}

/// Other actions already using `keys` (same context or a global one).
pub fn conflicts(id: &str, keys: &str, overrides: &BTreeMap<String, String>) -> Vec<&'static Bind> {
    let norm = |k: &str| Keystroke::parse(k).map(|k| k.unparse()).unwrap_or_else(|_| k.to_string());
    let want = norm(keys);
    ALL.iter()
        .filter(|b| b.id != id)
        .filter(|b| keys_for(b, overrides).iter().any(|k| norm(k) == want))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_dont_clash() {
        let none = BTreeMap::new();
        for b in ALL {
            for k in b.defaults {
                assert!(is_valid(k), "{}: {k}", b.id);
                assert!(conflicts(b.id, k, &none).is_empty(), "{} clashes on {k}", b.id);
            }
        }
        let ids: std::collections::HashSet<_> = ALL.iter().map(|b| b.id).collect();
        assert_eq!(ids.len(), ALL.len());
    }

    #[test]
    fn overrides_and_display() {
        let mut o = BTreeMap::new();
        o.insert("find_tab".to_string(), "cmd-shift-t, cmd-e".to_string());
        o.insert("overview".to_string(), String::new());
        let ft = ALL.iter().find(|b| b.id == "find_tab").unwrap();
        assert_eq!(keys_for(ft, &o), ["cmd-shift-t", "cmd-e"]);
        let ov = ALL.iter().find(|b| b.id == "overview").unwrap();
        assert!(keys_for(ov, &o).is_empty());
        assert_eq!(display("cmd-shift-p"), "⇧⌘P");
        assert_eq!(display("cmd-alt-left"), "⌥⌘←");
        assert_eq!(conflicts("overview", "cmd-e", &o).iter().map(|b| b.id).collect::<Vec<_>>(), ["find_tab"]);
    }
}

impl crate::workspace::Workspace {
    /// A key pressed while Settings › Keybindings waits for a new shortcut.
    pub fn record_key(&mut self, ks: &Keystroke, cx: &mut gpui::Context<Self>) {
        let Some(id) = self.key_recording else { return };
        let plain = !ks.modifiers.modified();
        if plain && ks.key == "escape" {
            self.key_recording = None;
            cx.notify();
            return;
        }
        if plain && matches!(ks.key.as_str(), "backspace" | "delete") {
            crate::config::set(&["keys", id], "");
            self.config.keys.insert(id.to_string(), String::new());
            self.key_recording = None;
            self.key_note = Some((id, "No shortcut".into()));
            cx.notify();
            return;
        }
        let Some(k) = from_keystroke(ks) else { return };
        // A shortcut does one thing: take it from whatever had it.
        let mut note = None;
        for other in conflicts(id, &k, &self.config.keys) {
            let norm = |s: &str| Keystroke::parse(s).map(|k| k.unparse()).unwrap_or_default();
            let left: Vec<String> = keys_for(other, &self.config.keys).into_iter().filter(|x| norm(x) != k).collect();
            let v = left.join(", ");
            crate::config::set(&["keys", other.id], &v);
            self.config.keys.insert(other.id.to_string(), v);
            note = Some(format!("Was on {}", other.label));
        }
        crate::config::set(&["keys", id], &k);
        self.config.keys.insert(id.to_string(), k);
        self.key_recording = None;
        self.key_note = note.map(|n| (id, n));
        install(cx, &self.config.keys);
        cx.notify();
    }

    pub fn reset_key(&mut self, id: &'static str, cx: &mut gpui::Context<Self>) {
        crate::config::remove(&["keys", id]);
        self.config.keys.remove(id);
        self.key_note = None;
        install(cx, &self.config.keys);
        cx.notify();
    }
}
