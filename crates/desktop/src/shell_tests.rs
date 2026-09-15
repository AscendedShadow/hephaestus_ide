use super::*;
use gpui::{
    EntityInputHandler as _, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext,
    point, size,
};
use gpui_component::{Root, Theme, ThemeMode};
use ide_core::vim::Mode;

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

fn primary(keys: &str) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    format!("{modifier}-{keys}")
}

fn text(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> String {
    cx.read(|cx| shell.read(cx).document().text().to_string())
}

fn dirty(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| shell.read(cx).buffer().dirty)
}

fn tabs(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.read(|cx| {
        let shell = shell.read(cx);
        let marker = |active: bool| if active { "*" } else { "" };
        shell
            .buffers
            .iter()
            .enumerate()
            .map(|(ix, buffer)| {
                let active = ix == shell.active && !shell.show_diff;
                format!("{}{}", marker(active), shell.tab_title(buffer))
            })
            .chain(
                shell
                    .diff_view
                    .read(cx)
                    .title()
                    .map(|title| format!("{}{title}", marker(shell.show_diff))),
            )
            .collect()
    })
}

fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    let none = Modifiers::none();
    cx.simulate_mouse_down(from, MouseButton::Left, none);
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
fn typing_undo_and_tabs_for_new_files(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    cx.simulate_input("Hello 世界 👋");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Second line");
    assert_eq!(text(&shell, cx), "Hello 世界 👋\nSecond line");
    assert!(dirty(&shell, cx));
    cx.simulate_keystrokes(&primary("z"));
    assert!(!dirty(&shell, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    });
    assert!(dirty(&shell, cx));

    cx.dispatch_action(NewFile);
    assert_eq!(tabs(&shell, cx), ["Untitled", "*Untitled"]);
    assert_eq!(text(&shell, cx), "");
    cx.simulate_input("x");
    cx.simulate_keystrokes(&primary("a"));
    cx.simulate_keystrokes("backspace");
    assert!(!dirty(&shell, cx));

    cx.simulate_keystrokes(&primary("w"));
    assert_eq!(tabs(&shell, cx), ["*Untitled"]);
    assert_eq!(text(&shell, cx), "Hello 世界 👋\nSecond line");

    cx.dispatch_action(CloseTab);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.pending,
            Some(PendingAction::CloseBuffer(shell.buffer().id))
        );
    });
    shell.update_in(cx, |shell, window, cx| shell.discard(window, cx));
    assert_eq!(tabs(&shell, cx), ["*Untitled"]);
    assert_eq!(text(&shell, cx), "");
    assert!(!dirty(&shell, cx));
}

#[gpui::test]
fn settings_dialog_toggles_light_mode_and_vim_keys(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    cx.simulate_keystrokes(&primary(","));
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

    let toggle = cx.debug_bounds("vim-mode").unwrap();
    let switch = point(toggle.left() + px(12.), toggle.center().y);
    cx.simulate_click(switch, Modifiers::none());
    cx.run_until_parked();
    cx.read(|cx| assert!(shell.read(cx).vim.is_some()));
    cx.simulate_click(switch, Modifiers::none());
    cx.run_until_parked();
    cx.read(|cx| assert!(shell.read(cx).vim.is_none()));
}

#[gpui::test]
fn brace_folding_only_changes_the_editor_projection(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fold.rs");
    let source = "fn main() {\n  work();\n}\n";
    std::fs::write(&path, source).unwrap();
    let document = Document::open(&path).unwrap();
    let (shell, cx) = setup(cx);
    shell.update_in(cx, |shell, window, cx| {
        shell.open_document(document, window, cx);
        shell.editor().update(cx, |editor, cx| {
            editor.set_cursor_position(Position::new(1, 2), window, cx)
        });
    });

    cx.dispatch_action(ToggleFold);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.editor().read(cx).text().to_string(),
            "fn main() {...}\n"
        );
        assert_eq!(shell.document().text().to_string(), source);
        assert!(!shell.buffer().dirty);
    });

    cx.dispatch_action(ToggleFold);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.editor().read(cx).text().to_string(), source);
        assert_eq!(shell.document().text().to_string(), source);
        assert!(!shell.buffer().dirty);
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
        shell.open_document(document, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(tabs(&shell, cx), ["*edit.txt"]);
    cx.simulate_keystrokes(&primary("a"));
    cx.simulate_input("edited 👋\n");
    cx.simulate_keystrokes(&primary("s"));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited 👋\r\n");
    assert!(!dirty(&shell, cx));
    cx.read(|cx| assert!(!shell.read(cx).busy));
    cx.dispatch_action(SaveFileAs);
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(!shell.read(cx).busy);
        assert_eq!(
            shell.read(cx).document().path().unwrap(),
            path.canonicalize().unwrap()
        );
    });
}

#[gpui::test]
fn failed_save_keeps_the_tab_open_then_a_successful_save_closes_it(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (shell, cx) = setup(cx);
    cx.simulate_input("Do not lose these edits");
    cx.dispatch_action(CloseTab);
    cx.dispatch_action(SaveFile);
    cx.simulate_new_path_selection(|_| Some(directory.path().join("missing/file.txt")));
    cx.run_until_parked();
    assert!(dirty(&shell, cx));
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(!shell.busy);
        assert!(matches!(shell.pending, Some(PendingAction::CloseBuffer(_))));
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
    assert_eq!(tabs(&shell, cx), ["*Untitled"]);
    assert!(!dirty(&shell, cx));
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert!(shell.pending.is_none());
        assert!(shell.document().path().is_none());
    });
    cx.simulate_keystrokes(&primary("z"));
    assert_eq!(text(&shell, cx), "");
}

#[gpui::test]
fn closing_the_window_asks_about_each_modified_file(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    cx.simulate_input("first");
    cx.dispatch_action(NewFile);
    cx.simulate_input("second");
    cx.dispatch_action(NewFile);
    assert_eq!(tabs(&shell, cx), ["Untitled", "Untitled", "*Untitled"]);

    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(tabs(&shell, cx), ["*Untitled", "Untitled", "Untitled"]);
    cx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(tabs(&shell, cx), ["Untitled", "Untitled", "*Untitled"]);

    let closes = shell.update_in(cx, |shell, window, cx| shell.can_close(window, cx));
    assert!(!closes);
    cx.read(|cx| assert_eq!(shell.read(cx).pending, Some(PendingAction::CloseWindow)));
    assert_eq!(text(&shell, cx), "first");
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(text(&shell, cx), "first");

    shell.update_in(cx, |shell, window, cx| shell.discard(window, cx));
    cx.read(|cx| assert_eq!(shell.read(cx).pending, Some(PendingAction::CloseWindow)));
    assert_eq!(text(&shell, cx), "second");
    shell.update(cx, |shell, _| shell.pending = None);
    assert_eq!(tabs(&shell, cx), ["*Untitled", "Untitled"]);
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

fn click_tree_row(cx: &mut VisualTestContext, ix: usize) {
    let sidebar = cx.debug_bounds("sidebar").unwrap();
    let y = sidebar.top() + px(34. + 24. * ix as f32 + 12.);
    cx.simulate_click(point(sidebar.left() + px(80.), y), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn project_tree_expands_folders_and_opens_files_in_tabs(cx: &mut TestAppContext) {
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
    cx.read(|cx| assert_eq!(shell.read(cx).document().path(), Some(main_rs.as_path())));
    assert_eq!(text(&shell, cx), "fn main() {}\n");
    assert_eq!(tabs(&shell, cx), ["*main.rs"]);

    cx.simulate_input("// edit\n");
    click_tree_row(cx, 2);
    assert_eq!(tabs(&shell, cx), ["main.rs", "*README.md"]);
    assert_eq!(text(&shell, cx), "readme\n");
    click_tree_row(cx, 1);
    assert_eq!(tabs(&shell, cx), ["*main.rs", "README.md"]);
    assert_eq!(text(&shell, cx), "// edit\nfn main() {}\n");
    assert!(dirty(&shell, cx));
    click_tree_row(cx, 2);

    cx.dispatch_action(SaveFileAs);
    cx.simulate_new_path_selection(|_| Some(root.join("src/new.txt")));
    cx.run_until_parked();
    assert_eq!(
        tree_names(&shell, cx),
        ["src", "  main.rs", "  new.txt", "README.md"]
    );
    let new_txt = root.join("src/new.txt").canonicalize().unwrap();
    cx.read(|cx| assert_eq!(shell.read(cx).document().path(), Some(new_txt.as_path())));
    assert_eq!(tabs(&shell, cx), ["main.rs", "*new.txt"]);

    click_tree_row(cx, 0);
    assert_eq!(tree_names(&shell, cx), ["src", "README.md"]);
}

#[gpui::test]
fn tabs_with_the_same_file_name_show_their_folder(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    for folder in ["a", "b"] {
        std::fs::create_dir(directory.path().join(folder)).unwrap();
        std::fs::write(directory.path().join(folder).join("mod.rs"), "").unwrap();
    }
    let (shell, cx) = setup(cx);
    for folder in ["a", "b"] {
        let document = Document::open(&directory.path().join(folder).join("mod.rs")).unwrap();
        shell.update_in(cx, |shell, window, cx| {
            shell.open_document(document, window, cx)
        });
    }
    assert_eq!(tabs(&shell, cx), ["mod.rs — a", "*mod.rs — b"]);
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
    cx.simulate_keystrokes("ctrl-n ctrl-w tab");
    assert!(terminal_focused(cx));
    assert_eq!(tabs(&shell, cx), ["*Untitled"]);

    cx.simulate_keystrokes("ctrl-`");
    assert!(!terminal_focused(cx));
    cx.simulate_keystrokes(&primary("n"));
    assert_eq!(tabs(&shell, cx), ["Untitled", "*Untitled"]);
}

#[gpui::test]
fn git_status_colors_the_tree_and_follows_saves(cx: &mut TestAppContext) {
    use crate::git_panel::tests::repository;

    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    std::fs::write(root.join("notes.txt"), "todo\n").unwrap();
    let (shell, cx) = setup(cx);
    cx.simulate_resize(size(px(1200.), px(800.)));
    let workspace = ide_core::workspace::Workspace::open(&root).unwrap();
    shell.update(cx, |shell, cx| shell.set_workspace(workspace, cx));
    cx.run_until_parked();
    let change = |path: &str, cx: &mut VisualTestContext| {
        let path = ide_core::git::absolute(&root, path);
        cx.read(|cx| shell.read(cx).git.read(cx).tree_change(&path))
    };
    assert_eq!(change("notes.txt", cx), Some(Change::Untracked));
    assert_eq!(change("src", cx), None);
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).git.read(cx).branch().as_deref(),
            Some("main")
        )
    });

    click_tree_row(cx, 1);
    assert_eq!(
        tree_names(&shell, cx),
        [".git", "src", "  main.rs", "notes.txt"]
    );
    click_tree_row(cx, 2);
    assert_eq!(tabs(&shell, cx), ["*main.rs"]);
    cx.simulate_input("// edit\n");
    cx.simulate_keystrokes(&primary("s"));
    cx.run_until_parked();
    assert_eq!(change("src/main.rs", cx), Some(Change::Modified));
    assert_eq!(change("src", cx), Some(Change::Modified));

    cx.simulate_keystrokes(&primary("shift-g"));
    cx.read(|cx| assert!(shell.read(cx).active_sidebar == SidebarPanel::Git));
    assert!(cx.debug_bounds("git-panel").is_some());
    cx.simulate_keystrokes(&primary("shift-e"));
    cx.read(|cx| assert!(shell.read(cx).active_sidebar == SidebarPanel::Folder));
    cx.simulate_keystrokes(&primary("shift-g"));
    let notes = root.join("notes.txt");
    shell.update(cx, |shell, cx| {
        shell
            .git
            .update(cx, |_, cx| cx.emit(GitPanelEvent::OpenFile(notes)))
    });
    cx.run_until_parked();
    assert_eq!(tabs(&shell, cx), ["main.rs", "*notes.txt"]);
}

#[gpui::test]
fn git_changes_open_their_diff_as_an_editor_tab(cx: &mut TestAppContext) {
    use crate::git_panel::tests::{git, repository};

    let directory = repository();
    let root = directory.path().canonicalize().unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {\n    run();\n}\n").unwrap();
    let (shell, cx) = setup(cx);
    cx.simulate_resize(size(px(1200.), px(800.)));
    let workspace = ide_core::workspace::Workspace::open(&root).unwrap();
    shell.update(cx, |shell, cx| shell.set_workspace(workspace, cx));
    cx.simulate_keystrokes(&primary("shift-g"));
    cx.run_until_parked();
    let click_change = |cx: &mut VisualTestContext| {
        let row = cx.debug_bounds("git-row-1").unwrap();
        cx.simulate_click(row.center(), Modifiers::none());
        cx.run_until_parked();
    };

    click_change(cx);
    assert_eq!(tabs(&shell, cx), ["Untitled", "*main.rs (Unstaged)"]);
    let diff = cx.debug_bounds("git-diff").unwrap();
    assert!(diff.left() >= cx.debug_bounds("sidebar").unwrap().right());
    cx.read(|cx| {
        let diff_view = shell.read(cx).diff_view.read(cx);
        assert!(matches!(diff_view.loaded(), Some(Ok(diff)) if !diff.lines.is_empty()));
    });

    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(tabs(&shell, cx), ["*Untitled", "main.rs (Unstaged)"]);
    cx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(tabs(&shell, cx), ["Untitled", "*main.rs (Unstaged)"]);
    let diff_tab = cx.debug_bounds("diff-tab").unwrap();
    cx.simulate_click(diff_tab.center(), Modifiers::none());
    assert_eq!(tabs(&shell, cx), ["Untitled", "*main.rs (Unstaged)"]);

    cx.simulate_keystrokes(&primary("w"));
    assert_eq!(tabs(&shell, cx), ["*Untitled"]);

    click_change(cx);
    assert_eq!(tabs(&shell, cx), ["Untitled", "*main.rs (Unstaged)"]);
    git(&root, &["commit", "-q", "-am", "Call run"]);
    shell.update(cx, |shell, cx| shell.refresh_git(cx));
    cx.run_until_parked();
    assert_eq!(tabs(&shell, cx), ["*Untitled"]);
}

#[gpui::test]
fn panel_shortcuts_do_not_enter_vim_commands(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    cx.simulate_resize(size(px(1200.), px(800.)));
    shell.update(cx, |shell, cx| shell.set_vim_mode(true, cx));

    let git = cx.debug_bounds("quick-git").unwrap();
    cx.simulate_click(git.center(), Modifiers::none());
    cx.read(|cx| assert!(shell.read(cx).active_sidebar == SidebarPanel::Git));
    let folder = cx.debug_bounds("quick-folder").unwrap();
    cx.simulate_click(folder.center(), Modifiers::none());
    cx.read(|cx| assert!(shell.read(cx).active_sidebar == SidebarPanel::Folder));
    let debug = cx.debug_bounds("quick-debug").unwrap();
    cx.simulate_click(debug.center(), Modifiers::none());
    cx.read(|cx| assert!(shell.read(cx).active_panel == ToolPanel::Debug));
    let terminal = cx.debug_bounds("quick-terminal").unwrap();
    cx.simulate_click(terminal.center(), Modifiers::none());
    cx.read(|cx| assert!(shell.read(cx).active_panel == ToolPanel::Terminal));
    shell.update_in(cx, |shell, window, cx| shell.focus_editor(window, cx));

    cx.simulate_keystrokes(&primary("shift-g"));
    cx.read(|cx| assert!(shell.read(cx).active_sidebar == SidebarPanel::Git));
    cx.simulate_keystrokes(&primary("shift-e"));
    cx.read(|cx| assert!(shell.read(cx).active_sidebar == SidebarPanel::Folder));
    cx.simulate_keystrokes(&primary("shift-d"));
    cx.read(|cx| assert!(shell.read(cx).active_panel == ToolPanel::Debug));
}

#[gpui::test]
fn without_a_folder_git_follows_the_open_file(cx: &mut TestAppContext) {
    let directory = crate::git_panel::tests::repository();
    let (shell, cx) = setup(cx);
    cx.run_until_parked();
    cx.read(|cx| assert!(shell.read(cx).git.read(cx).branch().is_none()));
    let document = Document::open(&directory.path().join("src/main.rs")).unwrap();
    shell.update_in(cx, |shell, window, cx| {
        shell.open_document(document, window, cx)
    });
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).git.read(cx).branch().as_deref(),
            Some("main")
        )
    });
}

fn editor_selection(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> Range<usize> {
    shell.update_in(cx, |shell, window, cx| {
        shell.editor().update(cx, |editor, cx| {
            editor.selected_text_range(false, window, cx).unwrap().range
        })
    })
}

fn vim_mode(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> Mode {
    cx.read(|cx| shell.read(cx).vim.as_ref().unwrap().mode())
}

#[gpui::test]
fn vim_keys_edit_select_undo_and_save(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vim.txt");
    std::fs::write(&path, "one two\nthree\n").unwrap();
    let document = Document::open(&path).unwrap();
    let (shell, cx) = setup(cx);
    shell.update_in(cx, |shell, window, cx| {
        shell.open_document(document, window, cx);
        shell.set_vim_mode(true, cx);
    });
    cx.run_until_parked();
    assert_eq!(vim_mode(&shell, cx), Mode::Normal);

    cx.simulate_keystrokes("d w");
    assert_eq!(text(&shell, cx), "two\nthree\n");
    cx.simulate_keystrokes("u");
    assert_eq!(text(&shell, cx), "one two\nthree\n");
    assert!(!dirty(&shell, cx));
    cx.simulate_keystrokes("ctrl-r");
    assert_eq!(text(&shell, cx), "two\nthree\n");
    cx.simulate_keystrokes("u");
    cx.simulate_keystrokes("enter d d");
    assert_eq!(text(&shell, cx), "one two\n");
    cx.simulate_keystrokes("g g x");
    assert_eq!(text(&shell, cx), "ne two\n");

    cx.simulate_keystrokes("i");
    assert_eq!(vim_mode(&shell, cx), Mode::Insert);
    cx.simulate_input("o");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&shell, cx), "o\nne two\n");
    cx.simulate_keystrokes("escape");
    assert_eq!(vim_mode(&shell, cx), Mode::Normal);
    cx.simulate_keystrokes("k shift-j");
    assert_eq!(text(&shell, cx), "o ne two\n");

    cx.simulate_keystrokes("0 v e");
    assert_eq!(vim_mode(&shell, cx), Mode::Visual);
    assert_eq!(editor_selection(&shell, cx), 0..4);
    cx.simulate_keystrokes("e");
    assert_eq!(editor_selection(&shell, cx), 0..8);
    cx.simulate_keystrokes("h o");
    assert_eq!(editor_selection(&shell, cx), 0..7);
    cx.read(|cx| assert_eq!(shell.read(cx).editor().read(cx).cursor(), 0));
    cx.simulate_keystrokes("o l");
    assert_eq!(editor_selection(&shell, cx), 0..8);
    cx.simulate_keystrokes("shift-u");
    assert_eq!(text(&shell, cx), "O NE TWO\n");
    assert_eq!(editor_selection(&shell, cx), 0..0);
    cx.simulate_keystrokes("shift-v j");
    assert_eq!(editor_selection(&shell, cx), 0..9);
    cx.simulate_keystrokes("escape");

    cx.simulate_keystrokes(": w");
    cx.read(|cx| {
        let vim = shell.read(cx).vim.as_ref().unwrap();
        assert_eq!(vim.prompt().as_deref(), Some(":w"));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "O NE TWO\n");
    assert!(!dirty(&shell, cx));

    cx.simulate_keystrokes(&primary("f"));
    cx.simulate_keystrokes("d d x enter");
    assert_eq!(text(&shell, cx), "O NE TWO\n");
    cx.simulate_keystrokes("escape");

    cx.simulate_keystrokes(&primary("n"));
    assert_eq!(tabs(&shell, cx), ["vim.txt", "*Untitled"]);
    assert_eq!(vim_mode(&shell, cx), Mode::Normal);
}

#[gpui::test]
fn vim_commands_open_switch_and_close_tabs(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    let (shell, cx) = setup(cx);
    let workspace = ide_core::workspace::Workspace::open(root).unwrap();
    shell.update(cx, |shell, cx| {
        shell.set_workspace(workspace, cx);
        shell.set_vim_mode(true, cx);
    });

    cx.simulate_keystrokes(": e space a . t x t enter");
    cx.run_until_parked();
    cx.simulate_keystrokes(": e space b . t x t enter");
    cx.run_until_parked();
    assert_eq!(tabs(&shell, cx), ["a.txt", "*b.txt"]);
    cx.simulate_keystrokes("g t");
    assert_eq!(tabs(&shell, cx), ["*a.txt", "b.txt"]);
    cx.simulate_keystrokes("g shift-t");
    assert_eq!(tabs(&shell, cx), ["a.txt", "*b.txt"]);

    cx.simulate_keystrokes("x : q enter");
    cx.read(|cx| {
        assert!(matches!(
            shell.read(cx).pending,
            Some(PendingAction::CloseBuffer(_))
        ))
    });
    shell.update(cx, |shell, _| shell.pending = None);
    cx.simulate_keystrokes(": w q enter");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "\n");
    assert_eq!(tabs(&shell, cx), ["*a.txt"]);
    cx.simulate_keystrokes("x : q ! enter");
    assert_eq!(tabs(&shell, cx), ["*Untitled"]);
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "a\n");
}
