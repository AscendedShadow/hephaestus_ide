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
        // Either auto-save hit the conflict, or the file watcher flagged it first.
        assert!(
            shell.status.starts_with("Auto-save failed")
                || shell.status.contains("changed on disk"),
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

#[gpui::test]
fn explorer_creates_renames_moves_and_deletes_with_open_buffers(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root.join("src/lib.rs"), "fn lib() {}\n");
    write(&root.join("src/nested/deep.rs"), "deep\n");
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&root).unwrap(), cx)
    });
    open_file(&shell, &root.join("src/nested/deep.rs"), cx);
    cx.simulate_input("unsaved ");
    cx.run_until_parked();

    // Create inside the selected folder, including Unicode names and a collision.
    shell.update_in(cx, |shell, window, cx| {
        shell.tree_selection = Some(root.join("src"));
        let directory = shell.target_directory().unwrap();
        assert_eq!(directory, root.join("src"));
        ide_core::workspace::create(&directory, "名前.rs", false).unwrap();
        shell.after_tree_change(Some(&directory.join("名前.rs")), cx);
        let _ = window;
    });
    assert!(root.join("src/名前.rs").is_file());
    let collision = ide_core::workspace::create(&root.join("src"), "lib.rs", false).unwrap_err();
    assert_eq!(collision, "lib.rs already exists");

    // Move a folder holding a dirty open file: the buffer follows with its edits.
    let result = shell.update(cx, |shell, cx| {
        shell.move_item_to(&root.join("src/nested"), &root.join("moved"), cx)
    });
    assert!(result.is_ok(), "{result:?}");
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.document().path(),
            Some(root.join("moved/deep.rs").as_path())
        );
        assert!(shell.buffer().dirty);
        assert!(
            shell
                .search_paths
                .iter()
                .any(|path| path.ends_with("moved/deep.rs"))
        );
    });
    assert_eq!(text(&shell, cx), "unsaved deep\n");
    let into_itself = shell.update(cx, |shell, cx| {
        shell.move_item_to(&root.join("moved"), &root.join("moved/inside"), cx)
    });
    assert!(into_itself.unwrap_err().contains("into itself"));

    // Saving after the move writes to the new location only.
    cx.simulate_keystrokes(&primary("s"));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("moved/deep.rs")).unwrap(),
        "unsaved deep\n"
    );
    assert!(!root.join("src/nested").exists());

    // Deleting a folder closes its open files.
    shell.update_in(cx, |shell, window, cx| {
        shell.delete_folder(root.join("moved"), window, cx)
    });
    cx.run_until_parked();
    assert!(!root.join("moved").exists());
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(shell.buffer_for_path(&root.join("moved/deep.rs")).is_none());
        assert!(
            shell
                .tree_rows
                .iter()
                .all(|row| !row.entry.path.starts_with(root.join("moved")))
        );
    });
}

#[gpui::test]
fn terminal_sessions_are_independent(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root.join(".hephaestus/settings.json"),
        r#"{"terminal_profiles": [{"name": "Broken", "program": "/no/such/shell-binary"}]}"#,
    );
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&root).unwrap(), cx)
    });
    cx.dispatch_action(NewTerminal);
    cx.dispatch_action(NewTerminal);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.terminal_sessions.len(), 3);
        assert_eq!(shell.active_terminal, 2);
        assert_eq!(shell.terminal_sessions[2].name, "Terminal 3");
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.start_profile(Some("Broken".into()), window, cx)
    });
    cx.read(|cx| {
        let status = &shell.read(cx).status;
        assert!(
            status.contains("Broken") && status.contains("/no/such/shell-binary"),
            "{status}"
        );
    });
    let others: Vec<_> = cx.read(|cx| {
        shell
            .read(cx)
            .terminal_sessions
            .iter()
            .map(|session| session.view.clone())
            .collect()
    });
    cx.dispatch_action(CloseTerminal);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.terminal_sessions.len(), 3);
        assert_eq!(
            others[1].read(cx).process_state(),
            Some(true),
            "other terminals keep running"
        );
    });
}

#[gpui::test]
fn external_changes_update_explorer_index_and_buffers(cx: &mut TestAppContext) {
    use super::mock_lsp::wait_until;
    let (shell, cx) = setup(cx);
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root.join("clean.rs"), "one\ntwo\nthree\n");
    write(&root.join("dirty.rs"), "dirty\n");
    write(&root.join("doomed.rs"), "bye\n");
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&root).unwrap(), cx)
    });
    wait_until(cx, |cx| cx.read(|cx| shell.read(cx).fs_sync_ready()));
    open_file(&shell, &root.join("doomed.rs"), cx);
    open_file(&shell, &root.join("dirty.rs"), cx);
    cx.simulate_input("mine ");
    open_file(&shell, &root.join("clean.rs"), cx);
    set_cursor(&shell, 1, 2, cx);

    // Another process edits, creates and deletes files while the window stays active.
    std::fs::write(root.join("clean.rs"), "one\nTWO\nthree\nfour\n").unwrap();
    std::fs::write(root.join("dirty.rs"), "theirs\n").unwrap();
    std::fs::remove_file(root.join("doomed.rs")).unwrap();
    // Atomic-save pattern: write a temporary file and rename it over a new name.
    std::fs::write(root.join(".new.rs.tmp"), "fresh\n").unwrap();
    std::fs::rename(root.join(".new.rs.tmp"), root.join("new.rs")).unwrap();

    wait_until(cx, |cx| {
        text(&shell, cx) == "one\nTWO\nthree\nfour\n"
            && cx.read(|cx| {
                shell
                    .read(cx)
                    .search_paths
                    .iter()
                    .any(|path| path.ends_with("new.rs"))
            })
    });
    cx.read(|cx| {
        let shell = shell.read(cx);
        let cursor = shell.editor().read(cx).cursor_position();
        assert_eq!(
            (cursor.line, cursor.character),
            (1, 2),
            "cursor preserved on reload"
        );
        let dirty = &shell.buffers[shell.buffer_for_path(&root.join("dirty.rs")).unwrap()];
        assert!(dirty.external_change && dirty.dirty);
        assert_eq!(dirty.document.text().to_string(), "mine dirty\n");
        let doomed = &shell.buffers[shell.buffer_for_path(&root.join("doomed.rs")).unwrap()];
        assert!(doomed.deleted_on_disk);
        assert!(shell.tree_rows.iter().any(|row| row.entry.name == "new.rs"));
        assert!(
            shell
                .tree_rows
                .iter()
                .all(|row| row.entry.name != "doomed.rs")
        );
    });

    // Our own save does not trigger a reload; switching workspaces drops old events.
    cx.simulate_input("x");
    cx.simulate_keystrokes(&primary("s"));
    cx.run_until_parked();
    let other = tempfile::tempdir().unwrap();
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(other.path()).unwrap(), cx)
    });
    std::fs::write(root.join("late.rs"), "late\n").unwrap();
    for _ in 0..40 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(
            shell
                .search_paths
                .iter()
                .all(|path| !path.ends_with("late.rs"))
        );
        assert!(!shell.buffer().external_change);
    });
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

#[gpui::test]
fn unsaved_buffers_are_recovered_after_a_crash(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let store = root.join("recovery-store");
    write(&root.join("a.rs"), "fn a() {}\n");
    write(&root.join("b.rs"), "fn b() {}\n");
    let (first, cx1) = setup(cx);
    first.update(cx1, |shell, _| {
        shell.recovery = recovery::RecoveryState::new(Some(store.clone()))
    });
    open_file(&first, &root.join("a.rs"), cx1);
    set_cursor(&first, 0, 3, cx1);
    cx1.simulate_input("new_");
    open_file(&first, &root.join("b.rs"), cx1);
    cx1.simulate_input("// mine\n");
    cx1.dispatch_action(NewFile);
    cx1.simulate_input("scratch 😀");
    first.update(cx1, |shell, cx| shell.write_recovery_snapshots(cx));
    cx1.run_until_parked();
    assert_eq!(ide_core::recovery::load(&store).0.len(), 3);
    // The process "crashes" here; meanwhile b.rs changes on disk.
    std::fs::write(root.join("b.rs"), "fn b() { changed }\n").unwrap();

    let (second, cx2) = setup(&mut cx1.cx);
    second.update_in(cx2, |shell, window, cx| {
        shell.recovery = recovery::RecoveryState::new(Some(store.clone()));
        shell.restore_recovery(window, cx);
    });
    cx2.run_until_parked();
    let tabs: Vec<(String, String, bool)> = cx2.read(|cx| {
        let shell = second.read(cx);
        shell
            .buffers
            .iter()
            .map(|buffer| {
                (
                    shell.tab_title(buffer),
                    buffer.document.text().to_string(),
                    buffer.dirty,
                )
            })
            .collect()
    });
    assert!(
        tabs.contains(&("a.rs (recovered)".into(), "fn new_a() {}\n".into(), true)),
        "{tabs:?}"
    );
    assert!(
        tabs.contains(&("b.rs".into(), "fn b() { changed }\n".into(), false)),
        "{tabs:?}"
    );
    assert!(
        tabs.contains(&(
            "b.rs (recovered)".into(),
            "// mine\nfn b() {}\n".into(),
            true
        )),
        "{tabs:?}"
    );
    assert!(
        tabs.contains(&("Untitled (recovered)".into(), "scratch 😀".into(), true)),
        "{tabs:?}"
    );
    cx2.read(|cx| {
        let status = &second.read(cx).status;
        assert!(
            status.contains("Recovered 3") && status.contains("b.rs changed on disk"),
            "{status}"
        );
    });

    // Saving clears that buffer's snapshot.
    second.update_in(cx2, |shell, window, cx| {
        let ix = shell.buffer_for_path(&root.join("a.rs")).unwrap();
        shell.activate(ix, window, cx);
    });
    cx2.simulate_keystrokes(&primary("s"));
    cx2.run_until_parked();
    let remaining = ide_core::recovery::load(&store).0;
    assert_eq!(remaining.len(), 2, "{remaining:?}");
    assert!(
        remaining
            .iter()
            .all(|entry| entry.path.as_deref() != Some(root.join("a.rs").as_path()))
    );
}

#[cfg(unix)]
#[gpui::test]
fn task_pipelines_run_dependencies_report_problems_and_gate_launches(cx: &mut TestAppContext) {
    use super::mock_lsp::wait_until;
    use super::tasks::RunState;
    let (shell, cx) = setup(cx);
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root.join("src/lib.rs"), "fn lib() {}\n");
    write(
        &root.join(".hephaestus/settings.json"),
        r#"{
          "tasks": [
            {"id": "gen", "program": "sh", "args": ["-c", "echo generated > gen.txt"]},
            {"id": "build", "group": "build", "default": true, "depends_on": ["gen"], "program": "sh",
             "args": ["-c", "printf 'error[E0001]: oops in ${workspaceFolderBasename}\n --> src/lib.rs:1:4\n'"]},
            {"id": "fail", "program": "sh", "args": ["-c", "echo 'src/lib.rs:1:1: warning: careful'; exit 2"]},
            {"id": "chain", "depends_on": ["fail"], "program": "sh", "args": ["-c", "touch chain.txt"]},
            {"id": "serve", "background": true, "ready_pattern": "listen(ing)?", "program": "sh",
             "args": ["-c", "echo starting; echo listening; sleep 30"]},
            {"id": "after", "depends_on": ["serve"], "program": "sh", "args": ["-c", "echo done"]},
            {"id": "loop", "depends_on": ["loop"], "program": "true"}
          ],
          "debug_profiles": [{"name": "App", "program": "/bin/true", "pre_launch_task": "fail"}]
        }"#,
    );
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&root).unwrap(), cx)
    });
    let states =
        |shell: &Entity<IdeShell>, cx: &mut VisualTestContext| -> Vec<(String, RunState)> {
            cx.read(|cx| {
                shell
                    .read(cx)
                    .tasks
                    .runs
                    .iter()
                    .map(|run| (run.task.clone(), run.state.clone()))
                    .collect()
            })
        };

    cx.dispatch_action(RunBuildTask);
    wait_until(cx, |cx| {
        states(&shell, cx)
            .iter()
            .any(|(id, state)| id == "build" && *state == RunState::Succeeded)
    });
    assert!(root.join("gen.txt").exists(), "the dependency ran first");
    cx.read(|cx| {
        let shell = shell.read(cx);
        let problem = &shell.problems.build[0];
        assert_eq!(problem.path, root.join("src/lib.rs"));
        assert_eq!((problem.line, problem.column), (1, 4));
        assert!(
            problem.message.contains("oops in"),
            "variables expanded: {}",
            problem.message
        );
        assert!(shell.tasks.pipeline.is_none());
    });

    // The Problems view filters by severity and navigates to the location.
    shell.update(cx, |shell, _| shell.problems.show[0] = false);
    cx.read(|cx| assert!(shell.read(cx).problem_entries().is_empty()));
    shell.update(cx, |shell, _| shell.problems.show[0] = true);
    cx.dispatch_action(NextProblem);
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.document().path(),
            Some(root.join("src/lib.rs").as_path())
        );
        assert_eq!(shell.editor().read(cx).cursor_position().character, 3);
    });

    shell.update(cx, |shell, cx| {
        shell.run_task_then("chain".into(), None, cx)
    });
    wait_until(cx, |cx| {
        states(&shell, cx)
            .iter()
            .any(|(id, state)| id == "fail" && *state == RunState::Failed(Some(2)))
    });
    assert!(
        !root.join("chain.txt").exists(),
        "a failed prerequisite stops the dependent task"
    );
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(
            shell.status.contains("\"chain\" was not started"),
            "{}",
            shell.status
        );
        assert!(
            shell
                .problems
                .build
                .iter()
                .any(|problem| problem.source == "task:fail")
        );
    });

    shell.update(cx, |shell, cx| {
        shell.run_task_then("after".into(), None, cx)
    });
    wait_until(cx, |cx| {
        states(&shell, cx)
            .iter()
            .any(|(id, state)| id == "after" && *state == RunState::Succeeded)
    });
    assert!(
        states(&shell, cx)
            .iter()
            .any(|(id, state)| id == "serve" && *state == RunState::Ready)
    );
    cx.dispatch_action(CancelTask);
    assert!(
        states(&shell, cx)
            .iter()
            .any(|(id, state)| id == "serve" && *state == RunState::Cancelled)
    );

    shell.update(cx, |shell, cx| shell.run_task_then("loop".into(), None, cx));
    cx.read(|cx| assert_eq!(shell.read(cx).status, "Task dependency cycle: loop → loop"));

    // A debug profile whose pre-launch task fails is not launched.
    cx.dispatch_action(StartDebugger);
    wait_until(cx, |cx| {
        cx.read(|cx| {
            shell
                .read(cx)
                .status
                .contains("pending launch were not started")
        })
    });
    cx.read(|cx| assert!(!shell.read(cx).debugger.read(cx).is_active()));
}

#[gpui::test]
fn git_branches_history_blame_and_hunks(cx: &mut TestAppContext) {
    use super::mock_lsp::wait_until;
    let git = |root: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(root)
            .args([
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .is_ok_and(|output| output.status.success())
    };
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    if !git(&root, &["init", "-q", "-b", "main"]) {
        return;
    }
    let lines: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    write(&root.join("a.txt"), &lines);
    assert!(git(&root, &["add", "."]) && git(&root, &["commit", "-qm", "first"]));
    write(
        &root.join("a.txt"),
        &lines
            .replace("line 2\n", "line two\n")
            .replace("line 19\n", "line nineteen\n"),
    );

    let (shell, cx) = setup(cx);
    cx.executor().allow_parking();
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&root).unwrap(), cx)
    });
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).git.read(cx).file_status("a.txt").is_some())
    });

    // Stage only the second hunk.
    open_file(&shell, &root.join("a.txt"), cx);
    cx.dispatch_action(StageHunk);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).search_mode == Some(SearchMode::Items))
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.choose_search_result(1, window, cx)
    });
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).status == "Staged the hunk")
    });
    let staged = std::process::Command::new("git")
        .current_dir(&root)
        .args(["diff", "--cached"])
        .output()
        .unwrap();
    let staged = String::from_utf8_lossy(&staged.stdout);
    assert!(
        staged.contains("nineteen") && !staged.contains("line two"),
        "{staged}"
    );

    // History and blame.
    cx.dispatch_action(ShowFileHistory);
    wait_until(cx, |cx| {
        cx.read(|cx| {
            shell
                .read(cx)
                .picker
                .as_ref()
                .is_some_and(|p| p.title == "History of a.txt")
        })
    });
    cx.read(|cx| {
        assert!(
            shell.read(cx).picker_labels("")[0].contains("  first  —  T · "),
            "{:?}",
            shell.read(cx).picker_labels("")
        )
    });
    shell.update_in(cx, |shell, window, cx| shell.focus_editor(window, cx));
    set_cursor(&shell, 1, 0, cx);
    cx.dispatch_action(BlameLine);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).status.starts_with("Line 2"))
    });
    cx.read(|cx| assert_eq!(shell.read(cx).status, "Line 2: not committed yet"));
    set_cursor(&shell, 0, 0, cx);
    cx.dispatch_action(BlameLine);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).status.starts_with("Line 1:"))
    });
    cx.read(|cx| {
        assert!(
            shell.read(cx).status.ends_with("— first"),
            "{}",
            shell.read(cx).status
        )
    });

    // Creating a branch switches to it; switching with unsaved edits asks first.
    shell.update_in(cx, |shell, window, cx| {
        assert!(shell.create_branch("topic".into(), "HEAD".into(), window, cx));
    });
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).status == "Switched to topic")
    });
    cx.simulate_input("unsaved ");
    shell.update_in(cx, |shell, window, cx| {
        shell.run_git_action(git_features::GitAction::Switch("main".into()), window, cx)
    });
    cx.update(|window, cx| {
        assert!(
            window.has_active_dialog(cx),
            "unsaved edits need a decision"
        )
    });
}

#[gpui::test]
fn test_explorer_discovers_runs_and_reruns_cargo_tests(cx: &mut TestAppContext) {
    use super::mock_lsp::wait_until;
    use ide_core::testing::TestState;
    if std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let (shell, cx) = setup(cx);
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        &root.join("src/lib.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn passes() {}\n\n    #[test]\n    fn fails() {\n        assert_eq!(1, 2, \"boom\");\n    }\n\n    #[test]\n    #[ignore]\n    fn slow() {}\n}\n",
    );
    write(&root.join("tests/it.rs"), "#[test]\nfn passes() {}\n");
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&root).unwrap(), cx)
    });
    cx.dispatch_action(ShowTesting);
    let deadline = std::time::Duration::from_secs(120);
    let start = std::time::Instant::now();
    while cx.read(|cx| shell.read(cx).testing.discovering) {
        assert!(start.elapsed() < deadline, "discovery timed out");
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let ids: Vec<String> = cx.read(|cx| {
        let testing = &shell.read(cx).testing;
        assert_eq!(testing.error, None);
        testing
            .entries
            .iter()
            .map(|entry| format!("{}::{}", entry.id.target.label(), entry.id.path))
            .collect()
    });
    assert_eq!(
        ids,
        [
            "demo (lib)::tests::fails",
            "demo (lib)::tests::passes",
            "demo (lib)::tests::slow",
            "demo (test it)::passes"
        ],
        "same-named tests in different targets are distinct"
    );
    cx.read(|cx| {
        let entry = &shell.read(cx).testing.entries[0];
        assert_eq!(entry.location, Some((root.join("src/lib.rs"), 7)));
    });

    cx.dispatch_action(RunAllTests);
    let states = |shell: &Entity<IdeShell>, cx: &mut VisualTestContext| -> Vec<TestState> {
        cx.read(|cx| {
            shell
                .read(cx)
                .testing
                .entries
                .iter()
                .map(|entry| entry.result.state.clone())
                .collect()
        })
    };
    wait_until(cx, |cx| {
        states(&shell, cx)
            .iter()
            .all(|state| !matches!(state, TestState::Queued | TestState::Running))
    });
    assert_eq!(
        states(&shell, cx),
        [
            TestState::Failed,
            TestState::Passed,
            TestState::Skipped,
            TestState::Passed
        ]
    );
    cx.read(|cx| {
        let failed = &shell.read(cx).testing.entries[0].result;
        assert!(failed.output.contains("boom"), "{}", failed.output);
        assert_eq!(failed.failure, Some((root.join("src/lib.rs"), 8, 9)));
    });

    // Rerun failed runs only the failing test, by exact identity, with a duration.
    cx.dispatch_action(RerunFailedTests);
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).testing.entries[1].result.state,
            TestState::Passed
        )
    });
    wait_until(cx, |cx| {
        states(&shell, cx)[0] == TestState::Failed
            && cx.read(|cx| shell.read(cx).testing.run.is_none())
    });
    cx.read(|cx| assert!(shell.read(cx).testing.entries[0].result.duration.is_some()));
}
