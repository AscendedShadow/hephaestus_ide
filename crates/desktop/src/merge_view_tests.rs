use std::{fs, path::Path, process::Command};

use super::*;
use crate::git_panel::tests::{git, repository};
use gpui::{TestAppContext, VisualTestContext};
use gpui_component::{Root, ThemeMode, input::Position};

const BASE: &str = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\n";

fn edit(text: &str, beta: &str, theta: &str) -> String {
    text.replace("beta", beta).replace("theta", theta)
}

fn merge_fails(root: &Path, branch: &str) {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["merge", "--no-edit", branch]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(!output.status.success(), "the merge should conflict");
}

pub(crate) fn conflicted_repository() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    git(&root, &["config", "merge.conflictStyle", "merge"]);
    fs::write(root.join("notes.txt"), BASE).unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "Notes"]);
    git(&root, &["checkout", "-q", "-b", "feature"]);
    fs::write(
        root.join("notes.txt"),
        edit(BASE, "beta feature", "theta feature"),
    )
    .unwrap();
    git(&root, &["commit", "-q", "-am", "Feature"]);
    git(&root, &["checkout", "-q", "main"]);
    fs::write(
        root.join("notes.txt"),
        edit(BASE, "beta main", "theta main"),
    )
    .unwrap();
    git(&root, &["commit", "-q", "-am", "Main"]);
    merge_fails(&root, "feature");
    (directory, root)
}

fn unmerged(root: &Path) -> String {
    git(root, &["diff", "--name-only", "--diff-filter=U"])
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    root: &Path,
) -> (Entity<MergeView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::set_mode(ThemeMode::Dark, None, cx);
        crate::commands::init(cx);
    });
    let repository = Repository::discover(root).unwrap().unwrap();
    let file = repository
        .status()
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.relative == "notes.txt")
        .unwrap();
    let path = root.join("notes.txt");
    let document = Document::open(&path).unwrap();
    let mut view = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| MergeView::new(repository, file, path, document, window, cx));
        view = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    (view.unwrap(), cx)
}

fn result(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> String {
    cx.read(|cx| view.read(cx).result.read(cx).text().to_string())
}

fn choices(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> Vec<Choice> {
    cx.read(|cx| {
        let merge = &view.read(cx).merge;
        (0..merge.conflicts().len())
            .map(|ix| merge.choice(ix))
            .collect()
    })
}

fn notice(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| {
        view.read(cx).notice.as_ref().map(|notice| match notice {
            Notice::Info(text) => format!("info: {text}"),
            Notice::Error(text) => format!("error: {text}"),
        })
    })
}

#[gpui::test]
fn picks_sides_tracks_edits_and_completes_the_merge(cx: &mut TestAppContext) {
    let (_directory, root) = conflicted_repository();
    let (view, cx) = open(cx, &root);
    cx.run_until_parked();
    assert_eq!(choices(&view, cx), [Choice::Unresolved, Choice::Unresolved]);
    assert!(result(&view, cx).contains("<<<<<<< HEAD\nbeta main\n=======\nbeta feature\n"));
    cx.read(|cx| {
        let view = view.read(cx);
        assert_eq!(view.merge.conflicts()[0].incoming_label, "feature");
        assert_eq!(view.result.read(cx).diagnostics().unwrap().len(), 2);
        let [incoming, current] = &view.panes;
        assert_eq!(incoming.side, Side::Incoming);
        assert_eq!(incoming.header_row(0), Some(1));
        assert!(matches!(
            &current.rows[2],
            PaneRow::Line { number: 2, text, conflict: Some(0) } if text == "beta main"
        ));
    });

    view.update_in(cx, |view, window, cx| {
        view.toggle(0, Side::Incoming, true, window, cx)
    });
    assert!(result(&view, cx).starts_with("alpha\nbeta feature\ngamma\n"));
    view.update_in(cx, |view, window, cx| {
        view.toggle(0, Side::Current, true, window, cx)
    });
    assert!(result(&view, cx).starts_with("alpha\nbeta feature\nbeta main\ngamma\n"));
    assert_eq!(choices(&view, cx)[0], Choice::IncomingThenCurrent);
    view.update_in(cx, |view, window, cx| {
        view.toggle(0, Side::Incoming, false, window, cx)
    });
    assert_eq!(choices(&view, cx), [Choice::Current, Choice::Unresolved]);
    cx.run_until_parked();
    cx.read(|cx| {
        let editor = view.read(cx).result.read(cx);
        assert_eq!(editor.diagnostics().unwrap().len(), 1);
    });

    let editor = cx.read(|cx| view.read(cx).result.clone());
    editor.update_in(cx, |editor, window, cx| {
        editor.set_cursor_position(Position::new(1, 0), window, cx)
    });
    cx.simulate_input("X");
    cx.run_until_parked();
    assert!(result(&view, cx).starts_with("alpha\nXbeta main\ngamma\n"));
    assert_eq!(choices(&view, cx), [Choice::Custom, Choice::Unresolved]);

    view.update(cx, |view, cx| view.complete(cx));
    assert_eq!(
        notice(&view, cx).as_deref(),
        Some("error: Resolve the remaining 1 conflict before completing the merge")
    );
    cx.run_until_parked();
    assert_eq!(unmerged(&root).trim(), "notes.txt");

    view.update_in(cx, |view, window, cx| {
        view.accept_all(Side::Incoming, window, cx)
    });
    assert_eq!(choices(&view, cx), [Choice::Custom, Choice::Incoming]);
    view.update(cx, |view, cx| view.complete(cx));
    cx.run_until_parked();
    assert_eq!(
        notice(&view, cx).as_deref(),
        Some("info: Resolved conflicts in notes.txt")
    );
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        edit(BASE, "Xbeta main", "theta feature")
    );
    assert_eq!(unmerged(&root), "");
}

#[gpui::test]
fn saving_and_resetting_keep_the_conflict_open(cx: &mut TestAppContext) {
    let (_directory, root) = conflicted_repository();
    let (view, cx) = open(cx, &root);
    view.update_in(cx, |view, window, cx| {
        view.resolve(1, Choice::Current, window, cx)
    });
    view.update(cx, |view, cx| view.save(cx));
    cx.run_until_parked();
    assert_eq!(
        notice(&view, cx).as_deref(),
        Some("info: Saved the merge result")
    );
    let saved = fs::read_to_string(root.join("notes.txt")).unwrap();
    assert!(saved.contains("theta main\n") && saved.contains("<<<<<<< HEAD\nbeta main\n"));
    assert_eq!(unmerged(&root).trim(), "notes.txt");

    view.update_in(cx, |view, window, cx| {
        view.accept_all(Side::Current, window, cx);
        view.reset(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(result(&view, cx), saved);
    assert_eq!(choices(&view, cx), [Choice::Unresolved]);
    cx.read(|cx| {
        let [incoming, _] = &view.read(cx).panes;
        assert!(incoming.rows.iter().any(|row| matches!(
            row,
            PaneRow::Line { text, conflict: None, .. } if text == "theta main"
        )));
    });
}

#[gpui::test]
fn files_without_markers_can_be_marked_resolved(cx: &mut TestAppContext) {
    let (_directory, root) = conflicted_repository();
    fs::write(root.join("notes.txt"), "resolved by hand\n").unwrap();
    let (view, cx) = open(cx, &root);
    cx.read(|cx| assert!(view.read(cx).merge.conflicts().is_empty()));
    view.update(cx, |view, cx| view.mark_resolved(cx));
    cx.run_until_parked();
    assert_eq!(unmerged(&root), "");
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "resolved by hand\n"
    );
}
