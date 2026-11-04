//! Workflow tests for the IDE backlog features (docs/tickets).

use gpui::{TestAppContext, VisualTestContext};
use gpui_component::Root;

use super::*;

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

#[gpui::test]
fn language_mode_is_detected_and_can_be_changed(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("deploy");
    std::fs::write(&script, "#!/usr/bin/env bash\necho hi\n").unwrap();
    let manifest = directory.path().join("Cargo.toml");
    std::fs::write(&manifest, "[package]\n").unwrap();
    open_file(&shell, &script, cx);
    cx.read(|cx| assert_eq!(shell.read(cx).buffer().language, "shell"));
    open_file(&shell, &manifest, cx);
    cx.read(|cx| assert_eq!(shell.read(cx).buffer().language, "toml"));

    cx.dispatch_action(NewFile);
    cx.read(|cx| assert_eq!(shell.read(cx).buffer().language, "plain_text"));
    cx.dispatch_action(ChangeLanguageMode);
    cx.read(|cx| assert_eq!(shell.read(cx).search_mode, Some(SearchMode::Languages)));
    cx.simulate_input("python");
    cx.simulate_keystrokes("enter");
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.search_mode, None);
        assert_eq!(shell.buffer().language, "python");
        assert!(shell.buffer().language_overridden);
    });
}
