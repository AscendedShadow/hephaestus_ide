//! Git branches, history, blame and hunk staging.

use super::*;

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
