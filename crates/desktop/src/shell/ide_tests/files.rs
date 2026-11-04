//! Settings layers, saving, recovery, the explorer and changes on disk.

use super::*;

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
