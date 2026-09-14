use std::time::{Duration, Instant};

use super::*;
use gpui::{Entity, Modifiers as KeyModifiers, TestAppContext, VisualTestContext};
use gpui_component::Root;

fn setup(cx: &mut TestAppContext) -> (Entity<TerminalView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::commands::init(cx);
    });
    let mut view = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(TerminalView::new);
        view = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    (view.unwrap(), cx)
}

/// Start the platform's basic shell in a temporary directory and focus the terminal.
fn start_shell(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let shell = if cfg!(target_os = "windows") {
        ("cmd.exe".into(), Vec::new())
    } else {
        ("/bin/sh".into(), Vec::new())
    };
    view.update_in(cx, |view, window, cx| {
        view.spawn(
            Options {
                shell: Some(shell),
                working_directory: Some(directory.path().into()),
                ..Default::default()
            },
            cx,
        );
        window.focus(&view.focus_handle);
    });
    cx.run_until_parked();
    directory
}

/// Let the shell run in real time until `condition` holds.
fn wait_until(
    cx: &mut VisualTestContext,
    view: &Entity<TerminalView>,
    what: &str,
    condition: impl Fn(&TerminalView) -> bool,
) {
    cx.executor().allow_parking();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if cx.read(|cx| condition(view.read(cx))) {
            return;
        }
        let lines = cx.read(|cx| {
            view.read(cx)
                .session
                .as_ref()
                .map(|session| session.terminal.snapshot().lines())
        });
        assert!(Instant::now() < deadline, "waiting for {what}: {lines:#?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The visible row that reads exactly `text`.
fn row_showing(view: &TerminalView, text: &str) -> Option<usize> {
    let lines = view.session.as_ref()?.terminal.snapshot().lines();
    lines.iter().position(|line| line.trim_end() == text)
}

#[gpui::test]
fn typed_commands_run_and_output_can_be_selected_and_copied(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let _directory = start_shell(&view, cx);
    cx.simulate_input("echo hephaestus-ok");
    cx.simulate_keystrokes("enter");
    wait_until(cx, &view, "command output", |view| {
        row_showing(view, "hephaestus-ok").is_some()
    });

    // Double-clicking a word selects it; the copy shortcut puts it on the clipboard.
    let position = cx.read(|cx| {
        let view = view.read(cx);
        let row = row_showing(view, "hephaestus-ok").unwrap();
        view.layout.unwrap().cell_bounds(row, 2, 1).center()
    });
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: KeyModifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position,
        modifiers: KeyModifiers::none(),
        click_count: 2,
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-shift-c"
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("hephaestus-ok".into())
    );
}

#[gpui::test]
fn exited_shell_restarts_on_enter(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let _directory = start_shell(&view, cx);
    cx.simulate_input("exit");
    cx.simulate_keystrokes("enter");
    wait_until(cx, &view, "the shell to exit", |view| {
        view.session
            .as_ref()
            .is_some_and(|session| session.terminal.exited())
    });
    cx.simulate_keystrokes("enter");
    cx.read(|cx| assert!(view.read(cx).is_running()));
    cx.simulate_input("echo restarted");
    cx.simulate_keystrokes("enter");
    wait_until(cx, &view, "output from the new shell", |view| {
        row_showing(view, "restarted").is_some()
    });
}
