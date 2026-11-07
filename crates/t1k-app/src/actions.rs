//! App-level actions.

use gpui::{actions, Action, SharedString};

actions!(t1k, [Quit, NewTab, CloseTab, NextTab, PrevTab, SelectTab1, SelectTab2, SelectTab3,
    SelectTab4, SelectTab5, SelectTab6, SelectTab7, SelectTab8, SelectTab9,
    ToggleAppearance, FollowSystemAppearance, ToggleSidebar, JumpToNeeds, ToggleSwitcher, ToggleOverview, RenameTab, ToggleOutputs, OpenSettings, JumpToNeedsBack,
    IncreaseFontSize, DecreaseFontSize, ResetFontSize, SplitRight, SplitDown, FocusOtherPane, GoBack, GoForward, ToggleCommandPalette, ShowAbout, CheckForUpdates, ShowUpdate, ReportBug, OpenRepo, ShowThemePicker]);

/// Pick a theme by name. It fills the dark or light slot, matching the theme's appearance.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = t1k, no_json)]
pub struct SelectTheme {
    pub name: SharedString,
}
