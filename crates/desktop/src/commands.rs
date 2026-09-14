use gpui::{App, KeyBinding, actions};

actions!(
    hephaestus,
    [NewFile, OpenFile, SaveFile, SaveFileAs, CloseWindow]
);

pub fn init(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-n"), NewFile, None),
        KeyBinding::new(&format!("{modifier}-o"), OpenFile, None),
        KeyBinding::new(&format!("{modifier}-s"), SaveFile, None),
        KeyBinding::new(&format!("{modifier}-shift-s"), SaveFileAs, None),
        KeyBinding::new(&format!("{modifier}-shift-w"), CloseWindow, None),
    ]);
}
