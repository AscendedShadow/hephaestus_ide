//! Split editor groups.

use super::*;

fn group_files(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> Vec<Vec<String>> {
    cx.read(|cx| {
        let shell = shell.read(cx);
        shell
            .groups
            .list
            .iter()
            .map(|group| {
                shell
                    .group_tabs(group.id)
                    .into_iter()
                    .map(|ix| {
                        let path = shell.buffers[ix].document.path().unwrap();
                        path.file_name().unwrap().to_string_lossy().into_owned()
                    })
                    .collect()
            })
            .collect()
    })
}

#[gpui::test]
fn split_views_share_text_and_undo_but_keep_their_own_cursor(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    open_source(&shell, directory.path(), "a.txt", "one\ntwo\n", cx);
    set_cursor(&shell, 0, 3, cx);
    cx.dispatch_action(SplitEditorRight);
    let first = cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.groups.list.len(), 2);
        assert_eq!(shell.groups.focused, 1);
        assert_eq!(shell.buffers[shell.active].views.len(), 2);
        shell
            .view_in(shell.active, shell.groups.list[0].id)
            .unwrap()
            .clone()
    });
    set_cursor(&shell, 1, 3, cx);
    cx.simulate_input("!");
    assert_eq!(text(&shell, cx), "one\ntwo!\n");
    cx.read(|cx| {
        let first = first.read(cx);
        assert_eq!(first.text().to_string(), "one\ntwo!\n");
        let cursor = first.cursor_position();
        assert_eq!(
            (cursor.line, cursor.character),
            (0, 3),
            "cursors are per view"
        );
    });

    // Undo from the other group reverts the edit in both views.
    cx.dispatch_action(FocusPreviousGroup);
    cx.read(|cx| assert_eq!(shell.read(cx).groups.focused, 0));
    cx.simulate_keystrokes(&primary("z"));
    assert_eq!(text(&shell, cx), "one\ntwo\n");
    assert_eq!(editor_text(&shell, cx), "one\ntwo\n");
    cx.read(|cx| {
        let shell = shell.read(cx);
        let view = shell
            .view_in(shell.active, shell.groups.list[1].id)
            .unwrap();
        assert_eq!(view.read(cx).text().to_string(), "one\ntwo\n");
        assert!(!shell.buffer().dirty);
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    });
    assert_eq!(text(&shell, cx), "one\ntwo!\n");

    // Focusing the other group's editor (e.g. by clicking it) makes it the target.
    let second = cx.read(|cx| {
        let shell = shell.read(cx);
        shell
            .view_in(shell.active, shell.groups.list[1].id)
            .unwrap()
            .clone()
    });
    let group = cx.read(|cx| shell.read(cx).groups.list[1].id);
    let selector = format!("editor-group-{group}");
    let bounds = cx
        .debug_bounds(Box::leak(selector.into_boxed_str()))
        .unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.groups.focused, 1);
        assert_eq!(shell.editor(), &second);
    });

    // Closing one view of a dirty document keeps it open without a prompt.
    cx.dispatch_action(CloseTab);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(shell.pending.is_none());
        assert_eq!(shell.groups.list.len(), 1, "the empty group closes");
        assert_eq!(shell.buffers.len(), 1);
        assert!(shell.buffer().dirty);
        assert_eq!(shell.buffer().views.len(), 1);
    });
    // The last view of a dirty document asks before closing.
    cx.dispatch_action(CloseTab);
    cx.read(|cx| assert!(shell.read(cx).pending.is_some()));
}

#[gpui::test]
fn tabs_move_between_groups_and_the_layout_is_restored(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let settings_path = directory.path().join("config").join(settings::FILE_NAME);
    cx.update(|_, cx| cx.set_global(settings::SettingsPath(settings_path.clone())));
    open_source(&shell, directory.path(), "a.txt", "alpha\n", cx);
    open_source(&shell, directory.path(), "b.txt", "beta\n", cx);
    cx.dispatch_action(MoveTabToNextGroup);
    assert_eq!(group_files(&shell, cx), [vec!["a.txt"], vec!["b.txt"]]);
    cx.read(|cx| assert_eq!(shell.read(cx).groups.focused, 1));

    // Split the first group downwards; it shows a.txt in a new group after it.
    cx.dispatch_action(FocusPreviousGroup);
    cx.dispatch_action(SplitEditorDown);
    assert_eq!(
        group_files(&shell, cx),
        [vec!["a.txt"], vec!["a.txt"], vec!["b.txt"]]
    );
    set_cursor(&shell, 0, 2, cx);
    // Opening a file opens it in the focused group.
    open_file(&shell, &directory.path().join("b.txt"), cx);
    assert_eq!(
        group_files(&shell, cx),
        [vec!["a.txt"], vec!["a.txt", "b.txt"], vec!["b.txt"]]
    );
    cx.read(|cx| assert_eq!(shell.read(cx).groups.focused, 1));

    shell.update(cx, |shell, cx| shell.save_session(cx));
    let saved = session::load(&session::path(&settings_path));
    assert_eq!(
        saved
            .groups
            .iter()
            .map(|group| group.down)
            .collect::<Vec<_>>(),
        [false, true, false]
    );
    assert_eq!(saved.focused_group, 1);

    let restored = cx.update(|window, cx| {
        let restored = cx.new(|cx| IdeShell::new(window, cx));
        restored.update(cx, |shell, cx| shell.restore_session(window, cx));
        restored
    });
    cx.run_until_parked();
    assert_eq!(
        group_files(&restored, cx),
        [vec!["a.txt"], vec!["a.txt", "b.txt"], vec!["b.txt"]]
    );
    cx.read(|cx| {
        let shell = restored.read(cx);
        assert_eq!(shell.groups.focused, 1);
        assert_eq!(shell.groups.list[1].axis, gpui::Axis::Vertical);
        let a = shell
            .buffer_for_path(&directory.path().join("a.txt"))
            .unwrap();
        let cursor = shell
            .view_in(a, shell.groups.list[1].id)
            .unwrap()
            .read(cx)
            .cursor_position();
        assert_eq!((cursor.line, cursor.character), (0, 2));
    });

    // Closing a group drops views shown elsewhere and keeps documents only shown there.
    cx.update(|window, cx| {
        restored.update(cx, |shell, cx| {
            shell.focus_next_group(&FocusNextGroup, window, cx);
            shell.close_editor_group(&CloseEditorGroup, window, cx);
        })
    });
    assert_eq!(
        group_files(&restored, cx),
        [vec!["a.txt"], vec!["a.txt", "b.txt"]]
    );
    cx.update(|window, cx| {
        restored.update(cx, |shell, cx| {
            shell.focus_previous_group(&FocusPreviousGroup, window, cx);
            let a = shell.active;
            let first = shell.groups.list[0].id;
            let second = shell.groups.list[1].id;
            shell.move_view(a, first, second, window, cx);
        })
    });
    assert_eq!(group_files(&restored, cx), [vec!["a.txt", "b.txt"]]);
}
