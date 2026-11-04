//! Project search and workspace replace.

use super::*;

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
