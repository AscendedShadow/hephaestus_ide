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
