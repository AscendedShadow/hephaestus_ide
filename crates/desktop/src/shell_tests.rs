use super::*;
use gpui::{TestAppContext, VisualTestContext};
use gpui_component::{Root, Theme, ThemeMode};

fn setup(cx: &mut TestAppContext) -> (Entity<IdeShell>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        crate::commands::init(cx);
    });
    let mut shell = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| IdeShell::new(window, cx));
        shell = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    (shell.unwrap(), cx)
}

#[gpui::test]
fn typing_undo_and_new_document_confirmation(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    cx.simulate_input("Hello 世界 👋");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Second line");
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.document.text().to_string(),
            "Hello 世界 👋\nSecond line"
        );
        assert!(shell.dirty);
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    });
    cx.read(|cx| assert!(!shell.read(cx).dirty));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    });
    cx.read(|cx| assert!(shell.read(cx).dirty));
    cx.dispatch_action(NewFile);
    cx.read(|cx| assert!(matches!(shell.read(cx).pending, Some(PendingAction::New))));
    shell.update_in(cx, |shell, window, cx| {
        shell.pending = None;
        shell
            .editor
            .update(cx, |editor, cx| editor.focus(window, cx));
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a backspace"
    } else {
        "ctrl-a backspace"
    });
    cx.read(|cx| assert!(!shell.read(cx).dirty));
}

#[gpui::test]
fn loaded_document_save_and_cancelled_save_as(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("edit.txt");
    std::fs::write(&path, "original\r\n").unwrap();
    let document = Document::open(&path).unwrap();
    let (shell, cx) = setup(cx);
    shell.update_in(cx, |shell, window, cx| {
        shell.install_document(document, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    cx.simulate_input("edited 👋\n");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited 👋\r\n");
    cx.read(|cx| {
        assert!(!shell.read(cx).dirty);
        assert!(!shell.read(cx).busy);
    });
    cx.dispatch_action(SaveFileAs);
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(!shell.read(cx).busy);
        assert_eq!(
            shell.read(cx).document.path().unwrap(),
            path.canonicalize().unwrap()
        );
    });
}

#[gpui::test]
fn failed_save_keeps_pending_document_then_successful_save_continues(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (shell, cx) = setup(cx);
    cx.simulate_input("Do not lose these edits");
    cx.dispatch_action(NewFile);
    cx.dispatch_action(SaveFile);
    cx.simulate_new_path_selection(|_| Some(directory.path().join("missing/file.txt")));
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(shell.dirty);
        assert!(!shell.busy);
        assert!(matches!(shell.pending, Some(PendingAction::New)));
        assert!(shell.status.starts_with("Save failed:"));
    });
    cx.dispatch_action(SaveFile);
    let path = directory.path().join("saved.txt");
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "Do not lose these edits"
    );
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(!shell.dirty);
        assert!(shell.pending.is_none());
        assert!(shell.document.path().is_none());
        assert_eq!(shell.document.text().to_string(), "");
    });
    // Opening a new document must not inherit the old editor's undo history.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    });
    cx.read(|cx| assert_eq!(shell.read(cx).document.text().to_string(), ""));
}
