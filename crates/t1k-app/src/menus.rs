//! Menu bar. Rebuilt when the theme list changes (imports).

use gpui::{App, Menu, MenuItem};

use crate::actions::*;

pub fn install(cx: &mut App) {
        let theme_items = |dark: bool| {
            t1k_themes::all()
                .into_iter()
                .filter(|t| t.is_dark() == dark)
                .map(|t| MenuItem::action(t.name.to_string(), SelectTheme { name: t.name.to_string().into() }))
                .collect::<Vec<_>>()
        };
        cx.set_menus(vec![
            Menu {
                name: "T1K".into(),
                items: vec![
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action("New Tab", NewTab),
                    MenuItem::action("Quit T1K", Quit),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Find Tab…", ToggleSwitcher),
                    MenuItem::action("Overview", ToggleOverview),
                    MenuItem::action("Jump to Next Needing You", JumpToNeeds),
                    MenuItem::action("Rename Tab…", RenameTab),
                    MenuItem::action("Open Link…", t1k_terminal::view::ShowLinkHints),
                    MenuItem::separator(),
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    MenuItem::action("Toggle Outputs Panel", ToggleOutputs),
                    MenuItem::separator(),
                    MenuItem::action("Bigger Text", IncreaseFontSize),
                    MenuItem::action("Smaller Text", DecreaseFontSize),
                    MenuItem::action("Actual Size", ResetFontSize),
                    MenuItem::separator(),
                    MenuItem::action("Toggle Dark / Light", ToggleAppearance),
                    MenuItem::action("Follow System Appearance", FollowSystemAppearance),
                    MenuItem::separator(),
                    MenuItem::submenu(Menu { name: "Dark Theme".into(), items: theme_items(true) }),
                    MenuItem::submenu(Menu { name: "Light Theme".into(), items: theme_items(false) }),
                ],
            },
        ]);
}
