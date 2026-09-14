use super::*;
use gpui::{Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, point, size};
use gpui_component::{Root, Theme, ThemeMode};

fn setup(cx: &mut TestAppContext) -> (Entity<IdeShell>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::set_mode(ThemeMode::Dark, None, cx);
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

fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    let none = Modifiers::none();
    cx.simulate_mouse_down(from, MouseButton::Left, none);
    // The first move crosses the drag threshold; the second lands after the resize starts.
    cx.simulate_mouse_move(from + point(px(4.), px(4.)), MouseButton::Left, none);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_up(to, MouseButton::Left, none);
    cx.run_until_parked();
}

#[gpui::test]
fn sidebar_and_tool_panel_resize_by_dragging_their_edges(cx: &mut TestAppContext) {
    let (_, cx) = setup(cx);
    cx.simulate_resize(size(px(1200.), px(800.)));
    cx.run_until_parked();

    let sidebar = cx.debug_bounds("sidebar").unwrap();
    let edge = point(sidebar.right(), sidebar.center().y);
    drag(cx, edge, edge + point(px(100.), px(0.)));
    let resized = cx.debug_bounds("sidebar").unwrap();
    assert!(
        (resized.size.width - (sidebar.size.width + px(100.))).abs() < px(1.),
        "sidebar {:?} -> {:?}",
        sidebar.size.width,
        resized.size.width
    );

    let panel = cx.debug_bounds("tool-panel").unwrap();
    let edge = point(panel.center().x, panel.top());
    drag(cx, edge, edge - point(px(0.), px(120.)));
    let resized = cx.debug_bounds("tool-panel").unwrap();
    assert!(
        (resized.size.height - (panel.size.height + px(120.))).abs() < px(1.),
        "tool panel {:?} -> {:?}",
        panel.size.height,
        resized.size.height
    );
}

#[gpui::test]
fn typing_undo_and_new_document_confirmation(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    cx.simulate_input("Hello 世界 👋");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Second line");
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.document.text().to_string(),
            "Hello 世界 👋\nSecond line"
        );
        assert!(shell.dirty);
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    });
    cx.read(|cx| assert!(!shell.read(cx).dirty));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    });
    cx.read(|cx| assert!(shell.read(cx).dirty));
    cx.dispatch_action(NewFile);
    cx.read(|cx| assert!(matches!(shell.read(cx).pending, Some(PendingAction::New))));
    shell.update_in(cx, |shell, window, cx| {
        shell.pending = None;
        shell
            .editor
            .update(cx, |editor, cx| editor.focus(window, cx));
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a backspace"
    } else {
        "ctrl-a backspace"
    });
    cx.read(|cx| assert!(!shell.read(cx).dirty));
}

#[gpui::test]
fn settings_dialog_toggles_light_mode(cx: &mut TestAppContext) {
    let (_, cx) = setup(cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-,"
    } else {
        "ctrl-,"
    });
    cx.run_until_parked();
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));

    let toggle = cx.debug_bounds("light-mode").unwrap();
    let switch = point(toggle.left() + px(12.), toggle.center().y);
    cx.simulate_click(switch, Modifiers::none());
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(theme::mode(), ThemeMode::Light);
        assert!(!Theme::global(cx).is_dark());
    });

    cx.simulate_click(switch, Modifiers::none());
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(theme::mode(), ThemeMode::Dark);
        assert!(Theme::global(cx).is_dark());
    });
}

#[gpui::test]
fn loaded_document_save_and_cancelled_save_as(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("edit.txt");
    std::fs::write(&path, "original\r\n").unwrap();
    let document = Document::open(&path).unwrap();
    let (shell, cx) = setup(cx);
    shell.update_in(cx, |shell, window, cx| {
        shell.install_document(document, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    cx.simulate_input("edited 👋\n");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited 👋\r\n");
    cx.read(|cx| {
        assert!(!shell.read(cx).dirty);
        assert!(!shell.read(cx).busy);
    });
    cx.dispatch_action(SaveFileAs);
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(!shell.read(cx).busy);
        assert_eq!(
            shell.read(cx).document.path().unwrap(),
            path.canonicalize().unwrap()
        );
    });
}

#[gpui::test]
fn failed_save_keeps_pending_document_then_successful_save_continues(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (shell, cx) = setup(cx);
    cx.simulate_input("Do not lose these edits");
    cx.dispatch_action(NewFile);
    cx.dispatch_action(SaveFile);
    cx.simulate_new_path_selection(|_| Some(directory.path().join("missing/file.txt")));
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(shell.dirty);
        assert!(!shell.busy);
        assert!(matches!(shell.pending, Some(PendingAction::New)));
        assert!(shell.status.starts_with("Save failed:"));
    });
    cx.dispatch_action(SaveFile);
    let path = directory.path().join("saved.txt");
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "Do not lose these edits"
    );
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(!shell.dirty);
        assert!(shell.pending.is_none());
        assert!(shell.document.path().is_none());
        assert_eq!(shell.document.text().to_string(), "");
    });
    // Opening a new document must not inherit the old editor's undo history.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    });
    cx.read(|cx| assert_eq!(shell.read(cx).document.text().to_string(), ""));
}

fn tree_names(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.read(|cx| {
        shell
            .read(cx)
            .tree_rows
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.entry.name))
            .collect()
    })
}

/// Click the project tree row at `ix` (34px header, then 24px rows).
fn click_tree_row(cx: &mut VisualTestContext, ix: usize) {
    let sidebar = cx.debug_bounds("sidebar").unwrap();
    let y = sidebar.top() + px(34. + 24. * ix as f32 + 12.);
    cx.simulate_click(point(sidebar.left() + px(80.), y), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn project_tree_expands_folders_and_opens_files(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("README.md"), "readme\n").unwrap();
    let (shell, cx) = setup(cx);
    cx.simulate_resize(size(px(1200.), px(800.)));
    let workspace = ide_core::workspace::Workspace::open(root).unwrap();
    shell.update(cx, |shell, cx| shell.set_workspace(workspace, cx));
    cx.run_until_parked();
    assert_eq!(tree_names(&shell, cx), ["src", "README.md"]);

    click_tree_row(cx, 0);
    assert_eq!(tree_names(&shell, cx), ["src", "  main.rs", "README.md"]);

    click_tree_row(cx, 1);
    let main_rs = root.join("src/main.rs").canonicalize().unwrap();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.document.path(), Some(main_rs.as_path()));
        assert_eq!(shell.document.text().to_string(), "fn main() {}\n");
    });

    // Opening another file from the tree still protects unsaved edits.
    cx.simulate_input("// edit\n");
    click_tree_row(cx, 2);
    cx.read(|cx| {
        assert!(matches!(
            &shell.read(cx).pending,
            Some(PendingAction::OpenPath(path)) if path.ends_with("README.md")
        ))
    });
    shell.update_in(cx, |shell, window, cx| {
        let action = shell.pending.take().unwrap();
        shell.perform(action, window, cx);
    });
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(shell.read(cx).document.text().to_string(), "readme\n"));

    // Saving a new file into an expanded folder refreshes that folder.
    cx.dispatch_action(SaveFileAs);
    cx.simulate_new_path_selection(|_| Some(root.join("src/new.txt")));
    cx.run_until_parked();
    assert_eq!(
        tree_names(&shell, cx),
        ["src", "  main.rs", "  new.txt", "README.md"]
    );
    let new_txt = root.join("src/new.txt").canonicalize().unwrap();
    cx.read(|cx| assert_eq!(shell.read(cx).document.path(), Some(new_txt.as_path())));

    click_tree_row(cx, 0);
    assert_eq!(tree_names(&shell, cx), ["src", "README.md"]);
}

#[gpui::test]
fn terminal_shortcut_moves_focus_and_shell_keys_skip_app_shortcuts(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (shell, cx) = setup(cx);
    let workspace = ide_core::workspace::Workspace::open(directory.path()).unwrap();
    shell.update(cx, |shell, cx| shell.set_workspace(workspace, cx));
    cx.simulate_input("unsaved");
    let terminal_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| shell.read(cx).terminal.focus_handle(cx).is_focused(window))
    };

    cx.simulate_keystrokes("ctrl-`");
    assert!(terminal_focused(cx));
    // The shell receives Ctrl+N and Tab instead of a new document or a focus change.
    cx.simulate_keystrokes("ctrl-n tab");
    assert!(terminal_focused(cx));
    cx.read(|cx| assert!(shell.read(cx).pending.is_none()));

    cx.simulate_keystrokes("ctrl-`");
    assert!(!terminal_focused(cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-n"
    } else {
        "ctrl-n"
    });
    cx.read(|cx| assert!(matches!(shell.read(cx).pending, Some(PendingAction::New))));
}
