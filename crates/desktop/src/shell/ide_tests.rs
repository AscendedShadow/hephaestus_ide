//! Workflow tests for the IDE backlog features (docs/tickets).

use gpui::{TestAppContext, VisualTestContext};
use gpui_component::Root;

use super::*;

mod editing;
mod files;
mod git;
mod groups;
mod multicursor;
mod run;
mod search;
mod workspaces;

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

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
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

fn git_init(directory: &Path) {
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(directory)
        .status()
        .unwrap();
    assert!(status.success());
}
