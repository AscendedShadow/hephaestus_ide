use gpui::{App, KeyBinding, Menu, MenuItem, NoAction, actions};

use crate::{
    git_panel,
    terminal_view::{self, KEY_CONTEXT as TERMINAL},
    vim,
};

actions!(
    hephaestus,
    [
        NewFile,
        OpenFile,
        OpenFolder,
        SaveFile,
        SaveFileAs,
        CloseTab,
        NextTab,
        PreviousTab,
        CloseWindow,
        OpenSettings,
        ShowFolderPanel,
        ToggleTerminal,
        ShowGitPanel,
        ShowDebugPanel,
        ToggleVimMode,
        ToggleFold
    ]
);

pub fn init(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let (copy, paste) = if cfg!(target_os = "macos") {
        ("cmd-c", "cmd-v")
    } else {
        ("ctrl-shift-c", "ctrl-shift-v")
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-n"), NewFile, None),
        KeyBinding::new(&format!("{modifier}-o"), OpenFile, None),
        KeyBinding::new(&format!("{modifier}-k {modifier}-o"), OpenFolder, None),
        KeyBinding::new(&format!("{modifier}-s"), SaveFile, None),
        KeyBinding::new(&format!("{modifier}-shift-s"), SaveFileAs, None),
        KeyBinding::new(&format!("{modifier}-w"), CloseTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-pagedown", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new("ctrl-pageup", PreviousTab, None),
        KeyBinding::new(&format!("{modifier}-shift-w"), CloseWindow, None),
        KeyBinding::new(&format!("{modifier}-,"), OpenSettings, None),
        KeyBinding::new(&format!("{modifier}-shift-e"), ShowFolderPanel, None),
        KeyBinding::new("ctrl-`", ToggleTerminal, None),
        KeyBinding::new(&format!("{modifier}-shift-g"), ShowGitPanel, None),
        KeyBinding::new(&format!("{modifier}-shift-d"), ShowDebugPanel, None),
        KeyBinding::new(&format!("{modifier}-shift-["), ToggleFold, None),
        KeyBinding::new(
            "secondary-enter",
            git_panel::Commit,
            Some(&format!("{} > Input", git_panel::COMMIT_CONTEXT)),
        ),
        KeyBinding::new(copy, terminal_view::Copy, Some(TERMINAL)),
        KeyBinding::new(paste, terminal_view::Paste, Some(TERMINAL)),
        KeyBinding::new("tab", NoAction, Some(TERMINAL)),
        KeyBinding::new("shift-tab", NoAction, Some(TERMINAL)),
    ]);
    if !cfg!(target_os = "macos") {
        cx.bind_keys(
            ["ctrl-n", "ctrl-o", "ctrl-k", "ctrl-k ctrl-o", "ctrl-w"]
                .map(|keys| KeyBinding::new(keys, NoAction, Some(TERMINAL))),
        );
    }
    let vim_normal = format!("{} > Input && !SearchPanel", vim::NORMAL_CONTEXT);
    let vim_insert = format!("{} > Input && !SearchPanel", vim::INSERT_CONTEXT);
    cx.bind_keys(
        [
            "escape",
            "ctrl-[",
            "enter",
            "backspace",
            "delete",
            "tab",
            "shift-tab",
            "up",
            "down",
            "left",
            "right",
            "home",
            "end",
        ]
        .map(|keys| KeyBinding::new(keys, NoAction, Some(&vim_normal))),
    );
    cx.bind_keys(
        ["escape", "ctrl-["].map(|keys| KeyBinding::new(keys, NoAction, Some(&vim_insert))),
    );

    let file_menu = Menu {
        name: "File".into(),
        items: vec![
            MenuItem::action("New", NewFile),
            MenuItem::action("Open…", OpenFile),
            MenuItem::action("Open Folder…", OpenFolder),
            MenuItem::separator(),
            MenuItem::action("Save", SaveFile),
            MenuItem::action("Save As…", SaveFileAs),
            MenuItem::separator(),
            MenuItem::action("Close Tab", CloseTab),
        ],
    };
    let view_menu = Menu {
        name: "View".into(),
        items: vec![MenuItem::action("Toggle Fold", ToggleFold)],
    };
    let menus = if cfg!(target_os = "macos") {
        vec![
            Menu {
                name: "Hephaestus".into(),
                items: vec![
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::action("Toggle Vim Keys", ToggleVimMode),
                ],
            },
            file_menu,
            view_menu,
        ]
    } else {
        vec![
            file_menu,
            view_menu,
            Menu {
                name: "Settings".into(),
                items: vec![
                    MenuItem::action("Open Settings…", OpenSettings),
                    MenuItem::action("Toggle Vim Keys", ToggleVimMode),
                ],
            },
        ]
    };
    cx.set_menus(menus);
}
