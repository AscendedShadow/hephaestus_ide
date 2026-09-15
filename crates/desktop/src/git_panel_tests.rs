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
        panel.select(selection, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        diff_text(&panel, cx),
        ["-fn main() {}", "+fn main() {", "+    run();", "+}"]
    );

    panel.update_in(cx, |panel, window, cx| panel.commit(&Commit, window, cx));
    assert_eq!(
        notice(&panel, cx).as_deref(),
        Some("error: Stage changes to commit them")
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
        let notes = panel.files(true).remove(0);
        panel.move_files(vec![notes], true, window, cx);
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
    panel.update(cx, |panel, cx| panel.refresh(cx));
    cx.run_until_parked();
    assert_eq!(rows(&panel, cx), ["Staged (1)", " M src/main.rs"]);
    panel.update_in(cx, |panel, window, cx| {
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
