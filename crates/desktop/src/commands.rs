use gpui::{App, KeyBinding, Menu, MenuItem, NoAction, actions};

use crate::terminal_view::{self, KEY_CONTEXT as TERMINAL};

actions!(
    hephaestus,
    [
        NewFile,
        OpenFile,
        OpenFolder,
        SaveFile,
        SaveFileAs,
        CloseWindow,
        OpenSettings,
        ToggleTerminal
    ]
);

pub fn init(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    // Ctrl+C and Ctrl+V belong to the shell, so other platforms add Shift.
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
        KeyBinding::new(&format!("{modifier}-shift-w"), CloseWindow, None),
        KeyBinding::new(&format!("{modifier}-,"), OpenSettings, None),
        KeyBinding::new("ctrl-`", ToggleTerminal, None),
        KeyBinding::new(copy, terminal_view::Copy, Some(TERMINAL)),
        KeyBinding::new(paste, terminal_view::Paste, Some(TERMINAL)),
        // The shell uses Tab for completion rather than focus navigation.
        KeyBinding::new("tab", NoAction, Some(TERMINAL)),
        KeyBinding::new("shift-tab", NoAction, Some(TERMINAL)),
    ]);
    if !cfg!(target_os = "macos") {
        // Readline and PSReadLine use these Ctrl chords, so they reach the shell. These
        // must be bound after the shortcuts they override; the chord is unbound too, or
        // Ctrl+K would wait for its second key.
        cx.bind_keys(
            ["ctrl-n", "ctrl-o", "ctrl-k", "ctrl-k ctrl-o"]
                .map(|keys| KeyBinding::new(keys, NoAction, Some(TERMINAL))),
        );
    }

    let file_menu = Menu {
        name: "File".into(),
        items: vec![
            MenuItem::action("New", NewFile),
            MenuItem::action("Open…", OpenFile),
            MenuItem::action("Open Folder…", OpenFolder),
            MenuItem::separator(),
            MenuItem::action("Save", SaveFile),
            MenuItem::action("Save As…", SaveFileAs),
        ],
    };
    // macOS always titles the first menu with the app name, so File must come second there,
    // and Settings goes in that app menu as the platform expects.
    let menus = if cfg!(target_os = "macos") {
        vec![
            Menu {
                name: "Hephaestus".into(),
                items: vec![MenuItem::action("Settings…", OpenSettings)],
            },
            file_menu,
        ]
    } else {
        vec![
            file_menu,
            Menu {
                name: "Settings".into(),
                items: vec![MenuItem::action("Open Settings…", OpenSettings)],
            },
        ]
    };
    cx.set_menus(menus);
}
