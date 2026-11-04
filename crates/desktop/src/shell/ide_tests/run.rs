//! Terminals, task pipelines and the Testing view.

use super::*;

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
