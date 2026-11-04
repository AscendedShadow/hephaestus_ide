use std::{fs, path::Path, process::Command};

use super::*;
use crate::git_panel::tests::{git, repository};
use gpui::{ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, size};
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
    conflicted_repository_with(BASE)
}

fn conflicted_repository_with(base: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    git(&root, &["config", "merge.conflictStyle", "merge"]);
    fs::write(root.join("notes.txt"), base).unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "Notes"]);
    git(&root, &["checkout", "-q", "-b", "feature"]);
    fs::write(
        root.join("notes.txt"),
        edit(base, "beta feature", "theta feature"),
    )
    .unwrap();
    git(&root, &["commit", "-q", "-am", "Feature"]);
    git(&root, &["checkout", "-q", "main"]);
    fs::write(
        root.join("notes.txt"),
        edit(base, "beta main", "theta main"),
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

fn spaced_base() -> String {
    let filler = |name: &str| {
        (0..100)
            .map(|ix| format!("{name} {ix}\n"))
            .collect::<String>()
    };
    format!("alpha\nbeta\n{}theta\n{}", filler("middle"), filler("tail"))
}

fn active(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> usize {
    cx.read(|cx| view.read(cx).active)
}

fn pane_tops(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> Vec<usize> {
    cx.read(|cx| {
        view.read(cx)
            .panes
            .iter()
            .map(|pane| {
                let offset = pane.scroll.0.borrow().base_handle.offset();
                (-offset.y / px(ROW_HEIGHT)).round() as usize
            })
            .collect()
    })
}

fn headers(view: &Entity<MergeView>, conflict: usize, cx: &mut VisualTestContext) -> Vec<usize> {
    cx.read(|cx| {
        view.read(cx)
            .panes
            .iter()
            .map(|pane| pane.header_row(conflict).unwrap())
            .collect()
    })
}

#[gpui::test]
fn side_panes_follow_the_result_cursor_and_scroll(cx: &mut TestAppContext) {
    let (_directory, root) = conflicted_repository_with(&spaced_base());
    let (view, cx) = open(cx, &root);
    cx.simulate_resize(size(px(1200.), px(800.)));
    cx.run_until_parked();
    assert_eq!(active(&view, cx), 0);
    assert_eq!(pane_tops(&view, cx), [0, 0]);

    let text = result(&view, cx);
    let second = text[..text.rfind("<<<<<<<").unwrap()].matches('\n').count() as u32;
    let editor = cx.read(|cx| view.read(cx).result.clone());
    editor.update_in(cx, |editor, window, cx| {
        editor.set_cursor_position(Position::new(second + 1, 2), window, cx)
    });
    cx.run_until_parked();
    assert_eq!(active(&view, cx), 1);
    assert_eq!(pane_tops(&view, cx), headers(&view, 1, cx));

    let scroll = |lines: f32, cx: &mut VisualTestContext| {
        let viewport = cx
            .read(|cx| editor.read(cx).text_geometry())
            .unwrap()
            .viewport;
        cx.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta: ScrollDelta::Lines(point(0., lines)),
            ..Default::default()
        });
        cx.run_until_parked();
    };
    scroll(1000., cx);
    assert_eq!(active(&view, cx), 0);
    assert_eq!(pane_tops(&view, cx), headers(&view, 0, cx));

    scroll(-(second as f32 - 3.), cx);
    assert_eq!(active(&view, cx), 1);
    assert_eq!(pane_tops(&view, cx), headers(&view, 1, cx));

    view.update_in(cx, |view, window, cx| view.navigate(-1, window, cx));
    cx.run_until_parked();
    assert_eq!(active(&view, cx), 0);
    cx.read(|cx| {
        let editor = view.read(cx).result.read(cx);
        assert_eq!(editor.cursor_position().line, 1);
    });
}

#[gpui::test]
fn the_result_is_dirty_until_it_matches_the_saved_file(cx: &mut TestAppContext) {
    let (_directory, root) = conflicted_repository();
    let (view, cx) = open(cx, &root);
    let dirty = |cx: &mut VisualTestContext| cx.read(|cx| view.read(cx).is_dirty());
    assert!(!dirty(cx));

    view.update_in(cx, |view, window, cx| {
        view.resolve(0, Choice::Incoming, window, cx)
    });
    assert!(dirty(cx));
    view.update_in(cx, |view, window, cx| {
        view.resolve(0, Choice::Unresolved, window, cx)
    });
    assert!(!dirty(cx));

    view.update_in(cx, |view, window, cx| {
        view.resolve(1, Choice::Current, window, cx)
    });
    let saved = view.update(cx, |view, cx| view.save_task(cx)).unwrap();
    assert!(cx.read(|cx| view.read(cx).is_busy()));
    assert!(view.update(cx, |view, cx| view.save_task(cx)).is_none());
    cx.run_until_parked();
    assert!(cx.executor().block_test(saved));
    assert!(!dirty(cx));

    let editor = cx.read(|cx| view.read(cx).result.clone());
    editor.update_in(cx, |editor, window, cx| {
        editor.set_cursor_position(Position::new(0, 0), window, cx)
    });
    cx.simulate_input("X");
    cx.run_until_parked();
    assert!(dirty(cx));
    view.update_in(cx, |view, window, cx| view.reset(window, cx));
    assert!(!dirty(cx));
}
