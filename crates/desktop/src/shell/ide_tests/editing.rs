//! Language modes, line commands, editor preferences and encodings.

use super::*;

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

#[gpui::test]
fn editor_preferences_follow_settings_language_overrides_and_editorconfig(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        &root.join(".hephaestus/settings.json"),
        r#"{"editor": {"tab_width": 3, "font_size": 16, "languages": {"go": {"insert_spaces": false, "tab_width": 8}}}}"#,
    );
    write(
        &root.join("web/.editorconfig"),
        "root = true\n[*.js]\nindent_size = 4\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n",
    );
    write(&root.join("main.go"), "package main\n");
    write(&root.join("lib.rs"), "fn main() {}\n");
    write(&root.join("web/app.js"), "let a = 1;   \nlet b = 2;");
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(root).unwrap(), cx)
    });

    let tab = |shell: &Entity<IdeShell>, cx: &mut VisualTestContext| {
        cx.read(|cx| {
            let shell = shell.read(cx);
            let setting = shell.editor().read(cx).tab_size_setting();
            (
                setting.tab_size,
                setting.hard_tabs,
                shell.buffer().preferences.font_size,
            )
        })
    };
    open_file(&shell, &root.join("lib.rs"), cx);
    assert_eq!(tab(&shell, cx), (3, false, 16.));
    open_file(&shell, &root.join("main.go"), cx);
    assert_eq!(tab(&shell, cx), (8, true, 16.));
    open_file(&shell, &root.join("web/app.js"), cx);
    assert_eq!(tab(&shell, cx), (4, false, 16.));
    cx.read(|cx| assert_eq!(shell.read(cx).indentation_label(), "Spaces: 4"));

    // Opening does not reformat; saving applies the EditorConfig policies.
    assert_eq!(text(&shell, cx), "let a = 1;   \nlet b = 2;");
    set_cursor(&shell, 1, 10, cx);
    cx.simulate_input("!");
    cx.simulate_keystrokes(&primary("s"));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(root.join("web/app.js")).unwrap();
    assert_eq!(saved, "let a = 1;\nlet b = 2;!\n");
    cx.read(|cx| assert!(!shell.read(cx).buffer().dirty));
}

#[gpui::test]
fn encodings_and_line_endings_convert_explicitly(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::write(root.join("latin.txt"), b"caf\xe9\nna\xefve\n").unwrap();
    open_file(&shell, &root.join("latin.txt"), cx);
    cx.read(|cx| assert_eq!(shell.read(cx).search_mode, Some(SearchMode::Items)));
    cx.simulate_input("1252");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&shell, cx), "café\nnaïve\n");
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.document().encoding().label(), "windows-1252");
        assert!(!shell.buffer().dirty);
    });

    // A line-ending conversion is a pending change even though the text is unchanged.
    shell.update(cx, |shell, cx| {
        shell.set_line_ending(ide_core::document::LineEnding::CrLf, cx)
    });
    cx.read(|cx| assert!(shell.read(cx).buffer().dirty));
    shell.update_in(cx, |shell, window, cx| {
        shell.apply_save_encoding(ide_core::document::Encoding::Utf16Be, window, cx)
    });
    cx.run_until_parked();
    let bytes = std::fs::read(root.join("latin.txt")).unwrap();
    let expected: Vec<u8> = [0xfe, 0xff]
        .into_iter()
        .chain(
            "café\r\nnaïve\r\n"
                .encode_utf16()
                .flat_map(u16::to_be_bytes),
        )
        .collect();
    assert_eq!(bytes, expected);
    cx.read(|cx| assert!(!shell.read(cx).buffer().dirty));

    // Unrepresentable text is refused without touching buffer or disk.
    cx.simulate_input("😀");
    shell.update_in(cx, |shell, window, cx| {
        shell.apply_save_encoding(
            ide_core::document::Encoding::Legacy("windows-1252"),
            window,
            cx,
        )
    });
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(
            shell.status.starts_with("Not converted"),
            "{}",
            shell.status
        );
        assert_eq!(shell.document().encoding().label(), "UTF-16 BE");
    });
    assert_eq!(std::fs::read(root.join("latin.txt")).unwrap(), expected);
}
