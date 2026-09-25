use super::*;
use gpui::TestAppContext;
use gpui_component::Root;

#[test]
fn debugger_breakpoints_use_selected_backend_syntax() {
    let path = Path::new("src/main.rs");
    assert_eq!(
        breakpoint_command(path, 12, true, false),
        "break \"src/main.rs\":12"
    );
    assert_eq!(
        breakpoint_command(path, 12, false, false),
        "clear \"src/main.rs\":12"
    );
    assert_eq!(
        breakpoint_command(path, 12, true, true),
        "breakpoint set --file \"src/main.rs\" --line 12"
    );
}

#[gpui::test]
fn quick_open_uses_workspace_index_and_opens_selected_file(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::commands::init(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("interesting.rs");
    std::fs::write(&path, "fn interesting() {}\n").unwrap();
    std::fs::write(directory.path().join("other.rs"), "other\n").unwrap();
    let mut shell = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| IdeShell::new(window, cx));
        shell = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let shell = shell.unwrap();
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(directory.path()).unwrap(), cx);
    });
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(shell.read(cx).search_paths.len(), 2));
    cx.dispatch_action(QuickOpen);
    cx.read(|cx| assert_eq!(shell.read(cx).search_mode, Some(SearchMode::Files)));
    cx.simulate_input("interesting");
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).search_input.read(cx).text().to_string(),
            "interesting"
        )
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.read(|cx| {
        let view = shell.read(cx);
        assert_eq!(
            view.document().path(),
            Some(path.canonicalize().unwrap().as_path()),
            "status: {} mode: {:?} query: {:?}",
            view.status,
            view.search_mode,
            view.search_input.read(cx).text().to_string()
        );
    });
}

#[gpui::test]
fn dirty_buffer_reports_external_change_and_save_preserves_disk(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::commands::init(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.rs");
    std::fs::write(&path, "original").unwrap();
    let mut shell = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| IdeShell::new(window, cx));
        shell = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let shell = shell.unwrap();
    shell.update_in(cx, |shell, window, cx| {
        shell.open_document(Document::open(&path).unwrap(), window, cx);
    });
    cx.simulate_input("ours");
    std::fs::write(&path, "theirs").unwrap();
    shell.update_in(cx, |shell, window, cx| shell.check_disk_changes(window, cx));
    cx.run_until_parked();
    cx.read(|cx| assert!(shell.read(cx).buffer().external_change));
    cx.dispatch_action(SaveFile);
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
    cx.read(|cx| assert!(shell.read(cx).buffer().dirty));
}

#[gpui::test]
fn project_search_opens_matching_line(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::commands::init(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.rs");
    std::fs::write(&path, "first\n🦀 needle\n").unwrap();
    let home = directory.path().join("home.rs");
    std::fs::write(&home, "home\n").unwrap();
    let mut shell = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| IdeShell::new(window, cx));
        shell = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let shell = shell.unwrap();
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(directory.path()).unwrap(), cx)
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.open_document(Document::open(&home).unwrap(), window, cx)
    });
    cx.run_until_parked();
    cx.dispatch_action(SearchProject);
    cx.simulate_input("needle");
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(shell.read(cx).search_hits.len(), 1));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.document().path(),
            Some(path.canonicalize().unwrap().as_path())
        );
        assert_eq!(
            shell.editor().read(cx).cursor_position(),
            Position::new(1, 3)
        );
    });
    cx.dispatch_action(GoBack);
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).document().path(),
            Some(home.canonicalize().unwrap().as_path())
        )
    });
    cx.dispatch_action(GoForward);
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).document().path(),
            Some(path.canonicalize().unwrap().as_path())
        )
    });
}

#[gpui::test]
fn completion_from_server_replaces_word_as_one_edit(cx: &mut TestAppContext) {
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
    let shell = shell.unwrap();
    cx.simulate_input("let greet");
    shell.update_in(cx, |shell, window, cx| {
        shell.pending_completion = Some((2, shell.buffer().id));
        shell.handle_lsp_event(
            serde_json::json!({
                "jsonrpc":"2.0", "id":2,
                "result":{"items":[{"label":"greeting", "insertText":"greeting"}]}
            }),
            window,
            cx,
        );
        assert_eq!(shell.search_mode, Some(SearchMode::Completion));
        shell.choose_search_result(0, window, cx);
    });
    cx.read(|cx| assert_eq!(shell.read(cx).document().text().to_string(), "let greeting"));
}

#[gpui::test]
fn palette_dispatches_existing_action(cx: &mut TestAppContext) {
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
    let shell = shell.unwrap();
    cx.dispatch_action(CommandPalette);
    cx.simulate_input("new_file");
    cx.simulate_keystrokes("enter");
    cx.read(|cx| assert_eq!(shell.read(cx).buffers.len(), 2));
    cx.dispatch_action(CommandPalette);
    cx.simulate_keystrokes("escape");
    cx.read(|cx| assert_eq!(shell.read(cx).search_mode, None));
}

#[gpui::test]
fn go_to_line_accepts_line_and_column(cx: &mut TestAppContext) {
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
    let shell = shell.unwrap();
    cx.simulate_input("first\nsecond\n");
    cx.dispatch_action(GoToLine);
    cx.simulate_input("2:3");
    cx.simulate_keystrokes("enter");
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).editor().read(cx).cursor_position(),
            Position::new(1, 2)
        )
    });
}
