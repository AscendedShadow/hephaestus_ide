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

fn set_terminal(
    view: &Entity<TerminalView>,
    output: &[u8],
    exited: bool,
    cx: &mut VisualTestContext,
) {
    let terminal = Terminal::from_test_output(output, GridSize::default(), exited);
    view.update_in(cx, |view, window, cx| {
        let events = cx.spawn(async move |_, _| {});
        view.session = Some(Session {
            terminal,
            options: Options::default(),
            piped: false,
            _events: events,
        });
        window.focus(&view.focus_handle);
        cx.notify();
    });
    cx.run_until_parked();
}

fn row_showing(view: &TerminalView, text: &str) -> Option<usize> {
    let lines = view.session.as_ref()?.terminal.snapshot().lines();
    lines.iter().position(|line| line.trim_end() == text)
}

#[gpui::test]
fn output_can_be_selected_and_copied(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    set_terminal(&view, b"hephaestus-ok\r\n", false, cx);

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
fn exited_terminal_ignores_input_other_than_restart(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    set_terminal(&view, b"finished\r\n", true, cx);
    cx.simulate_keystrokes("a");
    cx.read(|cx| {
        let view = view.read(cx);
        assert!(!view.is_running());
        assert_eq!(row_showing(view, "finished"), Some(0));
    });
}

#[gpui::test]
fn find_searches_scrollback_without_typing_into_the_shell(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let mut output = Vec::new();
    for ix in 0..200 {
        output.extend(
            format!("line {ix} {}\r\n", if ix % 50 == 0 { "MARK" } else { "-" }).into_bytes(),
        );
    }
    set_terminal(&view, &output, false, cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-shift-f"
    });
    cx.read(|cx| assert!(view.read(cx).find_state().is_some()));
    cx.simulate_input("mark");
    let state = |view: &Entity<TerminalView>, cx: &mut VisualTestContext| {
        cx.read(|cx| {
            let find = view.read(cx).find_state().unwrap();
            (find.matches.len(), find.current)
        })
    };
    assert_eq!(
        state(&view, cx),
        (4, Some(3)),
        "starts at the most recent match"
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(state(&view, cx), (4, Some(0)), "wraps around");
    view.update(cx, |view, cx| view.step_find(-1, cx));
    assert_eq!(state(&view, cx), (4, Some(3)));
    // The first match is far up in the scrollback; revealing it scrolls there.
    view.update(cx, |view, cx| view.step_find(1, cx));
    cx.run_until_parked();
    cx.read(|cx| assert!(row_showing(view.read(cx), "line 0 MARK").is_some()));
    // Case sensitivity.
    view.update(cx, |view, cx| {
        view.find.as_mut().unwrap().case_sensitive = true;
        view.update_find(true, cx);
    });
    assert_eq!(state(&view, cx).0, 0);
    cx.simulate_keystrokes("escape");
    cx.read(|cx| assert!(view.read(cx).find_state().is_none()));
    cx.update(|window, cx| assert!(view.read(cx).focus_handle.is_focused(window)));
}
