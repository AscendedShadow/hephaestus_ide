//! Workflow tests for the IDE backlog features (docs/tickets).

use gpui::{TestAppContext, VisualTestContext};
use gpui_component::Root;

use super::*;

pub(super) fn setup(cx: &mut TestAppContext) -> (Entity<IdeShell>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
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

pub(super) fn open_file(shell: &Entity<IdeShell>, path: &Path, cx: &mut VisualTestContext) {
    let path = path.to_path_buf();
    shell.update_in(cx, |shell, window, cx| shell.open(Some(path), window, cx));
    cx.run_until_parked();
}

pub(super) fn text(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> String {
    cx.read(|cx| shell.read(cx).document().text().to_string())
}

pub(super) fn editor_text(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> String {
    cx.read(|cx| shell.read(cx).editor().read(cx).text().to_string())
}

#[gpui::test]
fn language_mode_is_detected_and_can_be_changed(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("deploy");
    std::fs::write(&script, "#!/usr/bin/env bash\necho hi\n").unwrap();
    let manifest = directory.path().join("Cargo.toml");
    std::fs::write(&manifest, "[package]\n").unwrap();
    open_file(&shell, &script, cx);
    cx.read(|cx| assert_eq!(shell.read(cx).buffer().language, "shell"));
    open_file(&shell, &manifest, cx);
    cx.read(|cx| assert_eq!(shell.read(cx).buffer().language, "toml"));

    cx.dispatch_action(NewFile);
    cx.read(|cx| assert_eq!(shell.read(cx).buffer().language, "plain_text"));
    cx.dispatch_action(ChangeLanguageMode);
    cx.read(|cx| assert_eq!(shell.read(cx).search_mode, Some(SearchMode::Languages)));
    cx.simulate_input("python");
    cx.simulate_keystrokes("enter");
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.search_mode, None);
        assert_eq!(shell.buffer().language, "python");
        assert!(shell.buffer().language_overridden);
    });
}

pub(super) fn primary(keys: &str) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    format!("{modifier}-{keys}")
}

pub(super) fn set_cursor(
    shell: &Entity<IdeShell>,
    line: u32,
    column: u32,
    cx: &mut VisualTestContext,
) {
    shell.update_in(cx, |shell, window, cx| {
        shell.editor().update(cx, |editor, cx| {
            editor.set_cursor_position(Position::new(line, column), window, cx)
        });
    });
}

fn open_source(
    shell: &Entity<IdeShell>,
    directory: &Path,
    name: &str,
    source: &str,
    cx: &mut VisualTestContext,
) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, source).unwrap();
    open_file(shell, &path, cx);
    path
}

#[gpui::test]
fn line_commands_are_single_undo_steps(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    open_source(&shell, directory.path(), "lib.rs", "one\ntwo 🦀\nthree", cx);
    set_cursor(&shell, 1, 2, cx);

    cx.simulate_keystrokes("alt-up");
    assert_eq!(text(&shell, cx), "two 🦀\none\nthree");
    cx.read(|cx| {
        let position = shell.read(cx).editor().read(cx).cursor_position();
        assert_eq!((position.line, position.character), (0, 2));
    });
    cx.simulate_keystrokes("alt-down alt-down");
    assert_eq!(text(&shell, cx), "one\nthree\ntwo 🦀");
    cx.simulate_keystrokes(&primary("/"));
    assert_eq!(text(&shell, cx), "one\nthree\n// two 🦀");
    cx.simulate_keystrokes("shift-alt-down");
    assert_eq!(text(&shell, cx), "one\nthree\n// two 🦀\n// two 🦀");
    cx.simulate_keystrokes(&primary("shift-backspace"));
    assert_eq!(text(&shell, cx), "one\nthree\n// two 🦀");
    cx.simulate_keystrokes(&primary("enter"));
    cx.simulate_input("x");
    assert_eq!(text(&shell, cx), "one\nthree\n// two 🦀\nx");

    for expected in [
        "one\nthree\n// two 🦀\n",
        "one\nthree\n// two 🦀",
        "one\nthree\n// two 🦀\n// two 🦀",
        "one\nthree\n// two 🦀",
        "one\nthree\ntwo 🦀",
    ] {
        cx.simulate_keystrokes(&primary("z"));
        assert_eq!(text(&shell, cx), expected);
    }
}

#[gpui::test]
fn line_commands_never_write_fold_placeholders_into_the_source(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let source = "fn a() {\n    body();\n}\nfn b() {}\n";
    open_source(&shell, directory.path(), "fold.rs", source, cx);
    set_cursor(&shell, 0, 0, cx);
    cx.dispatch_action(ToggleFold);
    assert_eq!(editor_text(&shell, cx), "fn a() {...}\nfn b() {}\n");
    // Select both display lines and comment them: the fold sits inside the edit span.
    shell.update_in(cx, |shell, window, cx| {
        shell.editor().update(cx, |editor, cx| {
            let len = editor.text().len();
            editor.select_byte_range(0..len, false, window, cx)
        });
    });
    cx.dispatch_action(ToggleLineComment);
    assert_eq!(
        text(&shell, cx),
        "// fn a() {\n//     body();\n// }\n// fn b() {}\n"
    );
    assert!(!text(&shell, cx).contains("..."));
    cx.read(|cx| assert!(shell.read(cx).buffer().folds.is_empty()));
}
