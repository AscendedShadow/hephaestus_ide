use std::{fs, process::Command};

use super::*;
use gpui::{TestAppContext, VisualTestContext};
use gpui_component::{Root, ThemeMode};
use ide_core::git::LineKind;

fn setup(cx: &mut TestAppContext) -> (Entity<GitPanel>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::set_mode(ThemeMode::Dark, None, cx);
        crate::commands::init(cx);
    });
    let mut panel = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| GitPanel::new(window, cx));
        panel = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    (panel.unwrap(), cx)
}

pub(crate) fn git(root: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(crate) fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q", "--initial-branch=main"]);
    git(root, &["config", "user.name", "Hephaestus Tests"]);
    git(root, &["config", "user.email", "tests@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["config", "core.hooksPath", ".no-hooks"]);
    git(root, &["config", "core.autocrlf", "false"]);
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "Initial"]);
    directory
}

fn rows(panel: &Entity<GitPanel>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.read(|cx| {
        let panel = panel.read(cx);
        panel
            .rows
            .iter()
            .map(|row| match *row {
                Row::Header { staged: true } => format!("Staged ({})", panel.staged_count),
                Row::Header { staged: false } => format!("Changes ({})", panel.unstaged_count),
                Row::File { ix, staged } => {
                    let file = &panel.status.files[ix];
                    let selection = Selection {
                        relative: file.relative.clone(),
                        staged,
                    };
                    let marker = if panel.selected.as_ref() == Some(&selection) {
                        ">"
                    } else {
                        " "
                    };
                    let change = side(file, staged).unwrap();
                    format!("{marker}{} {}", change.letter(), file.relative)
                }
            })
            .collect()
    })
}

fn diff_text(panel: &Entity<GitPanel>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.read(|cx| match panel.read(cx).diff_view.read(cx).loaded() {
        Some(Ok(diff)) => diff
            .lines
            .iter()
            .filter(|line| matches!(line.kind, LineKind::Added | LineKind::Removed))
            .map(|line| {
                let sign = if line.kind == LineKind::Added {
                    "+"
                } else {
                    "-"
                };
                format!("{sign}{}", line.text)
            })
            .collect(),
        other => panic!("no diff: {:?}", other.map(Result::is_ok)),
    })
}

fn notice(panel: &Entity<GitPanel>, cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| {
        panel.read(cx).notice.as_ref().map(|notice| match notice {
            Notice::Info(text) => format!("info: {text}"),
            Notice::Error(text) => format!("error: {text}"),
        })
    })
}

#[gpui::test]
fn lists_changes_shows_diffs_stages_and_commits(cx: &mut TestAppContext) {
    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {\n    run();\n}\n").unwrap();
    fs::write(root.join("notes.txt"), "todo\n").unwrap();
    let (panel, cx) = setup(cx);
    panel.update(cx, |panel, cx| panel.set_directory(Some(root.clone()), cx));
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(panel.read(cx).branch().as_deref(), Some("main")));
    assert_eq!(
        rows(&panel, cx),
        ["Changes (2)", " U notes.txt", " M src/main.rs"]
    );

    panel.update(cx, |panel, cx| {
        let selection = Selection {
            relative: "src/main.rs".into(),
            staged: false,
        };
        panel.select(selection.clone(), cx);
        panel.set_checked(selection, true, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        diff_text(&panel, cx),
        ["-fn main() {}", "+fn main() {", "+    run();", "+}"]
    );

    panel.update_in(cx, |panel, window, cx| panel.commit(&Commit, window, cx));
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("error: Write a commit message first")
    );

    panel.update_in(cx, |panel, window, cx| {
        let files = panel.files(false);
        panel.move_files(files, false, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        rows(&panel, cx),
        ["Staged (2)", " A notes.txt", ">M src/main.rs"]
    );
    assert_eq!(diff_text(&panel, cx).len(), 4);

    panel.update_in(cx, |panel, window, cx| {
        panel.set_checked(
            Selection {
                relative: "src/main.rs".into(),
                staged: true,
            },
            false,
            cx,
        );
        panel.set_checked(
            Selection {
                relative: "notes.txt".into(),
                staged: true,
            },
            true,
            cx,
        );
        panel.unstage_checked(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        rows(&panel, cx),
        [
            "Staged (1)",
            ">M src/main.rs",
            "Changes (1)",
            " U notes.txt"
        ]
    );
    panel.update(cx, |panel, cx| {
        panel.set_checked(
            Selection {
                relative: "src/main.rs".into(),
                staged: true,
            },
            true,
            cx,
        );
        panel.set_checked(
            Selection {
                relative: "notes.txt".into(),
                staged: false,
            },
            false,
            cx,
        );
    });

    panel.update_in(cx, |panel, window, cx| {
        panel
            .commit_message
            .update(cx, |input, cx| input.focus(window, cx))
    });
    cx.simulate_input("Call run\n\nFrom main.");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    });
    cx.run_until_parked();
    assert_eq!(
        git(&root, &["log", "-1", "--format=%B"]).trim_end(),
        "Call run\n\nFrom main."
    );
    assert!(notice(&panel, cx).unwrap().contains("Call run"));
    cx.read(|cx| assert_eq!(panel.read(cx).commit_message.read(cx).value(), ""));
    assert_eq!(rows(&panel, cx), ["Changes (1)", " U notes.txt"]);
    cx.read(|cx| assert!(panel.read(cx).diff_view.read(cx).target().is_none()));
}

#[gpui::test]
fn reports_git_errors_and_follows_changes_made_elsewhere(cx: &mut TestAppContext) {
    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    let (panel, cx) = setup(cx);
    panel.update(cx, |panel, cx| panel.set_directory(Some(root.clone()), cx));
    cx.run_until_parked();
    assert!(rows(&panel, cx).is_empty());

    let hooks = tempfile::tempdir().unwrap();
    let hook = hooks.path().join("pre-commit");
    fs::write(&hook, "#!/bin/sh\necho 'Refused by hook' >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let hooks_path = hooks.path().to_string_lossy().replace('\\', "/");
    git(&root, &["config", "core.hooksPath", &hooks_path]);
    fs::write(root.join("src/main.rs"), "changed\n").unwrap();
    git(&root, &["add", "-A"]);
    cx.run_until_parked();
    assert!(rows(&panel, cx).is_empty());
    cx.update(|window, _| window.activate_window());
    cx.executor().advance_clock(POLL_INTERVAL);
    cx.run_until_parked();
    assert_eq!(rows(&panel, cx), ["Staged (1)", " M src/main.rs"]);
    panel.update_in(cx, |panel, window, cx| {
        panel.set_checked(
            Selection {
                relative: "src/main.rs".into(),
                staged: true,
            },
            true,
            cx,
        );
        panel
            .commit_message
            .update(cx, |input, cx| input.set_value("Change", window, cx));
        panel.commit(&Commit, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("error: Refused by hook")
    );
    cx.read(|cx| assert_eq!(panel.read(cx).commit_message.read(cx).value(), "Change"));
    assert_eq!(rows(&panel, cx), ["Staged (1)", " M src/main.rs"]);

    let src = root.join("src");
    let reloaded = panel.update(cx, |panel, cx| panel.set_directory(Some(src), cx));
    assert!(!reloaded);
    assert_eq!(rows(&panel, cx), ["Staged (1)", " M src/main.rs"]);
}

pub(crate) fn with_remote(root: &Path) -> tempfile::TempDir {
    let remote = tempfile::tempdir().unwrap();
    git(
        remote.path(),
        &["init", "-q", "--bare", "--initial-branch=main"],
    );
    git(
        root,
        &["remote", "add", "origin", &remote.path().to_string_lossy()],
    );
    remote
}

fn choose(cx: &mut VisualTestContext, item: &'static str) {
    let more = cx.debug_bounds("git-actions").unwrap();
    cx.simulate_click(more.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    let item = cx.debug_bounds(item).unwrap();
    cx.simulate_click(item.center(), gpui::Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn actions_menu_stages_commits_and_syncs_with_the_remote(cx: &mut TestAppContext) {
    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    let remote = with_remote(&root);
    fs::write(root.join("notes.txt"), "todo\n").unwrap();
    let (panel, cx) = setup(cx);
    panel.update(cx, |panel, cx| panel.set_directory(Some(root.clone()), cx));
    cx.run_until_parked();

    let push_keys = cx.update(|_, cx| crate::commands::shortcut(&Push, cx));
    let expected = if cfg!(target_os = "macos") {
        "⇧⌘K"
    } else {
        "Ctrl+Shift+K"
    };
    assert_eq!(push_keys.as_deref(), Some(expected));
    let commit_keys = cx.update(|_, cx| crate::commands::shortcut(&Commit, cx));
    assert!(commit_keys.is_some_and(|keys| keys.contains("Enter") || keys.contains('⏎')));

    choose(cx, "git-menu-Stage All Changes");
    assert_eq!(rows(&panel, cx), ["Staged (1)", " A notes.txt"]);
    panel.update(cx, |panel, cx| {
        panel.set_checked(
            Selection {
                relative: "notes.txt".into(),
                staged: true,
            },
            true,
            cx,
        )
    });
    panel.update_in(cx, |panel, window, cx| {
        panel.stage_all(&StageAll, window, cx)
    });
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("info: No changes to stage")
    );

    choose(cx, "git-menu-Commit");
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("error: Write a commit message first")
    );
    cx.update(|window, cx| {
        let focused = panel.read(cx).commit_message.read(cx).focus_handle(cx);
        assert!(focused.is_focused(window));
    });
    cx.simulate_input("Add notes");
    choose(cx, "git-menu-Commit");
    assert_eq!(
        git(&root, &["log", "-1", "--format=%s"]).trim(),
        "Add notes"
    );
    assert!(rows(&panel, cx).is_empty());

    choose(cx, "git-menu-Push");
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("info: Pushed main to origin")
    );
    let pushed = git(remote.path(), &["log", "-1", "--format=%s", "main"]);
    assert_eq!(pushed.trim(), "Add notes");
    cx.read(|cx| assert_eq!(panel.read(cx).branch().as_deref(), Some("main")));

    choose(cx, "git-menu-Fetch");
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("info: Fetched from all remotes")
    );
    choose(cx, "git-menu-Pull");
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("info: Already up to date")
    );

    git(&root, &["remote", "remove", "origin"]);
    choose(cx, "git-menu-Push");
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("error: This repository has no remotes")
    );
}

#[gpui::test]
fn offers_to_create_a_repository(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let (panel, cx) = setup(cx);
    panel.update(cx, |panel, cx| panel.set_directory(Some(root.clone()), cx));
    cx.run_until_parked();
    cx.read(|cx| {
        let panel = panel.read(cx);
        assert!(panel.loaded && panel.repository.is_none() && panel.branch().is_none());
    });

    fs::write(root.join("new.txt"), "").unwrap();
    panel.update_in(cx, |panel, window, cx| panel.init_repository(window, cx));
    cx.run_until_parked();
    cx.read(|cx| {
        let panel = panel.read(cx);
        assert_eq!(panel.repository.as_ref().unwrap().root(), root);
        assert!(panel.branch().is_some());
        assert_eq!(
            panel.tree_change(&root.join("new.txt")),
            Some(Change::Untracked)
        );
    });
    assert_eq!(rows(&panel, cx), ["Changes (1)", " U new.txt"]);
}

#[gpui::test]
fn explorer_ignore_state_refreshes_with_gitignore(cx: &mut TestAppContext) {
    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("build/nested")).unwrap();
    fs::write(root.join("build/nested/out.txt"), "").unwrap();
    fs::write(root.join("notes.log"), "").unwrap();
    let (panel, cx) = setup(cx);
    panel.update(cx, |panel, cx| panel.set_directory(Some(root.clone()), cx));
    cx.run_until_parked();
    cx.read(|cx| assert!(!panel.read(cx).tree_ignored(&root.join("build"))));

    fs::write(root.join(".gitignore"), "build/\n*.log\n").unwrap();
    panel.update(cx, |panel, cx| panel.refresh(cx));
    cx.run_until_parked();
    cx.read(|cx| {
        let panel = panel.read(cx);
        assert!(panel.tree_ignored(&root.join("build")));
        assert!(panel.tree_ignored(&root.join("build/nested/out.txt")));
        assert!(panel.tree_ignored(&root.join("notes.log")));
        assert!(!panel.tree_ignored(&root.join(".gitignore")));
    });

    fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    panel.update(cx, |panel, cx| panel.refresh(cx));
    cx.run_until_parked();
    cx.read(|cx| {
        let panel = panel.read(cx);
        assert!(!panel.tree_ignored(&root.join("build")));
        assert!(panel.tree_ignored(&root.join("notes.log")));
    });
}
