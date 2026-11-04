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

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

#[gpui::test]
fn workspace_settings_layer_over_user_settings(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let settings_file = first.path().join(".hephaestus/settings.json");
    write(
        &settings_file,
        r#"{"commands": [{"name": "build", "program": "./build.sh", "working_directory": "out"}]}"#,
    );
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(first.path()).unwrap(), cx)
    });
    let root = first.path().canonicalize().unwrap();
    cx.read(|cx| {
        let commands = settings::current(cx).commands;
        assert_eq!(commands.len(), 1);
        assert_eq!(Path::new(&commands[0].program), root.join("./build.sh"));
        assert_eq!(
            commands[0].working_directory.as_deref(),
            Some(root.join("out").as_path())
        );
        assert_eq!(
            settings::state(cx).unwrap().resolved.sources["commands"],
            settings::layers::Source::Workspace
        );
    });

    // A malformed edit is reported with its location and the last valid layer stays.
    write(&settings_file, "{\n  \"commands\": [\n");
    let status = shell.update_in(cx, |shell, window, cx| shell.apply_settings(window, cx));
    assert!(status.contains("settings.json:3:"), "{status}");
    cx.read(|cx| assert_eq!(settings::current(cx).commands.len(), 1));

    // Switching workspaces drops the old overrides.
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(second.path()).unwrap(), cx)
    });
    cx.read(|cx| assert!(settings::current(cx).commands.is_empty()));

    cx.dispatch_action(ShowEffectiveSettings);
    let view = text(&shell, cx);
    assert!(view.starts_with("// Effective settings"), "{view}");
    cx.read(|cx| assert_eq!(shell.read(cx).buffer().language, "jsonc"));
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
fn save_all_and_auto_save_after_delay(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(&root.join("a.txt"), "a");
    write(&root.join("b.txt"), "b");
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(root).unwrap(), cx)
    });
    open_file(&shell, &root.join("a.txt"), cx);
    cx.simulate_input("1");
    open_file(&shell, &root.join("b.txt"), cx);
    cx.simulate_input("2");
    cx.dispatch_action(NewFile);
    cx.simulate_input("scratch");
    cx.dispatch_action(SaveAll);
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "1a");
    assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "2b");
    cx.read(|cx| {
        let status = &shell.read(cx).status;
        assert!(status.contains("1 untitled buffer not saved"), "{status}");
    });

    write(
        &root.join(".hephaestus/settings.json"),
        r#"{"auto_save": {"mode": "after_delay", "delay_ms": 200}}"#,
    );
    cx.dispatch_action(ReloadSettings);
    cx.read(|cx| {
        assert_eq!(
            IdeShell::auto_save_settings(cx).label().as_deref(),
            Some("Auto Save: 200ms")
        )
    });
    open_file(&shell, &root.join("a.txt"), cx);
    cx.simulate_input("x");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    cx.simulate_input("y");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "1a",
        "debounced"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "1xya");
    cx.read(|cx| assert!(!shell.read(cx).buffer().dirty));

    // A conflicting external change is not overwritten, and the failure is not retried.
    write(&root.join("a.txt"), "theirs");
    cx.simulate_input("z");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "theirs"
    );
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(
            shell.status.starts_with("Auto-save failed"),
            "{}",
            shell.status
        );
        assert!(shell.buffer().external_change && shell.buffer().dirty);
    });
}

fn panel_search(shell: &Entity<IdeShell>, query: &str, cx: &mut VisualTestContext) {
    let query = query.to_string();
    shell.update_in(cx, |shell, window, cx| {
        shell.search_project(&SearchProject, window, cx);
        shell.search_panel.update(cx, |panel, cx| {
            panel
                .query
                .update(cx, |input, cx| input.set_value(query, window, cx));
        });
        shell.run_project_search(cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn project_search_options_and_far_results(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let many: String = (0..300)
        .map(|ix| format!("item_{ix} = needle;\n"))
        .collect();
    write(&root.join("many.rs"), &many);
    write(&root.join("other.md"), "Needle in docs\n");
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(root).unwrap(), cx)
    });

    panel_search(&shell, "needle", cx);
    cx.read(|cx| {
        let panel = shell.read(cx).search_panel.read(cx);
        assert_eq!(panel.hit_count(), 301, "case-insensitive by default");
        assert!(!panel.results().truncated);
    });
    shell.update(cx, |shell, cx| {
        shell.search_panel.update(cx, |panel, cx| {
            panel.case_sensitive = true;
            panel.regex = true;
            panel.select_hit(0, cx);
        });
        shell.run_project_search(cx);
    });
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(shell.read(cx).search_panel.read(cx).hit_count(), 300));

    panel_search(&shell, "item_(2[5-9]", cx);
    cx.read(|cx| {
        let error = shell
            .read(cx)
            .search_panel
            .read(cx)
            .error()
            .unwrap()
            .to_string();
        assert!(error.starts_with("Invalid regular expression"), "{error}");
    });

    // A match far beyond the first 30 can be selected and opened from the keyboard.
    panel_search(&shell, r"item_250 =", cx);
    panel_search(&shell, "needle", cx);
    shell.update(cx, |shell, cx| {
        shell
            .search_panel
            .update(cx, |panel, cx| panel.select_hit(250, cx))
    });
    shell.update_in(cx, |shell, window, cx| {
        shell
            .search_panel
            .update(cx, |panel, cx| panel.focus_query(window, cx))
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(shell.document().path().unwrap().ends_with("many.rs"));
        assert_eq!(shell.editor().read(cx).cursor_position().line, 250);
    });
}

#[gpui::test]
fn workspace_replace_handles_dirty_closed_stale_and_read_only_files(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(&root.join("open.rs"), "let a = old_name(1);\n");
    write(
        &root.join("closed.rs"),
        "fn x() {\r\n    old_name(2); // ünï\r\n}\r\n",
    );
    write(&root.join("stale.rs"), "old_name(3);\n");
    write(&root.join("locked.rs"), "old_name(4);\n");
    let locked = root.join("locked.rs");
    let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&locked, permissions).unwrap();
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(root).unwrap(), cx)
    });
    open_file(&shell, &root.join("open.rs"), cx);
    set_cursor(&shell, 0, 0, cx);
    cx.simulate_input("// dirty\n");

    shell.update(cx, |shell, cx| {
        shell.search_panel.update(cx, |panel, _| {
            panel.regex = true;
            panel.show_replace = true;
        })
    });
    panel_search(&shell, r"old_name\((\d)\)", cx);
    cx.read(|cx| assert_eq!(shell.read(cx).search_panel.read(cx).hit_count(), 4));
    shell.update_in(cx, |shell, window, cx| {
        shell.search_panel.update(cx, |panel, cx| {
            panel.replace.update(cx, |input, cx| {
                input.set_value("new_name($1, 0)", window, cx)
            })
        })
    });
    // Another process changes a file after the preview.
    write(&root.join("stale.rs"), "// moved\nold_name(3);\n");
    shell.update_in(cx, |shell, window, cx| {
        shell.replace_in_workspace(window, cx)
    });
    cx.run_until_parked();

    assert_eq!(text(&shell, cx), "// dirty\nlet a = new_name(1, 0);\n");
    cx.read(|cx| assert!(shell.read(cx).buffer().dirty, "open files stay unsaved"));
    assert_eq!(
        std::fs::read_to_string(root.join("closed.rs")).unwrap(),
        "fn x() {\r\n    new_name(2, 0); // ünï\r\n}\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("stale.rs")).unwrap(),
        "// moved\nold_name(3);\n"
    );
    assert_eq!(std::fs::read_to_string(&locked).unwrap(), "old_name(4);\n");
    cx.read(|cx| {
        let status = &shell.read(cx).status;
        assert!(status.contains("Replaced 2 matches"), "{status}");
        assert!(
            status.contains("modified (unsaved) open files: open.rs"),
            "{status}"
        );
        assert!(status.contains("saved: closed.rs"), "{status}");
        assert!(
            status.contains("changed since the preview: stale.rs"),
            "{status}"
        );
        assert!(status.contains("failed: locked.rs"), "{status}");
    });

    cx.dispatch_action(UndoWorkspaceReplace);
    cx.run_until_parked();
    assert_eq!(text(&shell, cx), "// dirty\nlet a = old_name(1);\n");
    assert_eq!(
        std::fs::read_to_string(root.join("closed.rs")).unwrap(),
        "fn x() {\r\n    old_name(2); // ünï\r\n}\r\n"
    );
}
