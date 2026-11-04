//! Multi-root workspaces.

use super::*;

#[gpui::test]
fn multi_root_workspaces_cover_every_root_and_restore(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let base = directory.path().canonicalize().unwrap();
    let settings_path = base.join("config").join(settings::FILE_NAME);
    cx.update(|_, cx| cx.set_global(settings::SettingsPath(settings_path.clone())));
    let (a, b) = (base.join("repos/app"), base.join("repos/lib"));
    for (root, tab) in [(&a, 2), (&b, 8)] {
        write(&root.join("src/main.rs"), "fn main() { needle(); }\n");
        write(
            &root.join(".hephaestus/settings.json"),
            &format!("{{\"editor\": {{\"tab_width\": {tab}}}}}"),
        );
        git_init(root);
    }
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&a).unwrap(), cx);
        shell.add_workspace_root(&b, cx).unwrap();
    });
    cx.run_until_parked();

    // Explorer rows are grouped by root; Quick Open names are root-qualified.
    cx.read(|cx| {
        let shell = shell.read(cx);
        let roots: Vec<_> = shell
            .tree_rows
            .iter()
            .filter(|row| row.depth == 0)
            .map(|row| row.entry.name.clone())
            .collect();
        assert_eq!(roots, ["app", "lib"]);
        let mut files: Vec<String> = shell
            .ranked_paths("main")
            .iter()
            .map(|path| shell.workspace.qualified(path))
            .collect();
        files.sort();
        assert_eq!(files, ["app/src/main.rs", "lib/src/main.rs"]);
    });

    // Project search spans both roots, unless a root is excluded.
    panel_search(&shell, "needle", cx);
    cx.read(|cx| assert_eq!(shell.read(cx).search_panel.read(cx).hit_count(), 2));
    shell.update_in(cx, |shell, window, cx| {
        shell.run_root_action(roots::RootAction::ToggleSearch(b.clone()), window, cx)
    });
    cx.run_until_parked();
    panel_search(&shell, "needle", cx);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.search_panel.read(cx).hit_count(), 1);
        assert_eq!(shell.ranked_paths("main").len(), 1);
    });

    // Root-scoped features follow the explicitly selected root, not the open file.
    open_file(&shell, &b.join("src/main.rs"), cx);
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.active_root(), Some(a.as_path()));
        assert_eq!(shell.project_directory(), Some(a.as_path()));
        assert_eq!(shell.buffer().preferences.tab_width, 2);
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.run_root_action(roots::RootAction::Activate(b.clone()), window, cx)
    });
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.project_directory(), Some(b.as_path()));
        assert_eq!(
            shell.buffer().preferences.tab_width,
            8,
            "lib's workspace settings"
        );
    });

    // Removing a root keeps its (unsaved) buffers open.
    cx.simulate_input("x");
    shell.update(cx, |shell, cx| shell.remove_workspace_root(&b, cx));
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.workspace.root_paths(), std::slice::from_ref(&a));
        assert!(shell.buffer().dirty);
        assert_eq!(shell.active_root(), Some(a.as_path()));
    });

    // The session restores roots, search scope and the active root; a folder that is
    // gone stays listed with an error.
    shell.update(cx, |shell, cx| {
        shell.add_workspace_root(&b, cx).unwrap();
        shell.workspace.set_search(&b, false);
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.run_root_action(roots::RootAction::Activate(b.clone()), window, cx)
    });
    let c = base.join("repos/extra");
    std::fs::create_dir_all(&c).unwrap();
    shell.update(cx, |shell, cx| {
        shell.add_workspace_root(&c, cx).unwrap();
        shell.save_session(cx);
    });
    std::fs::remove_dir_all(&c).unwrap();
    let restored = cx.update(|window, cx| {
        let restored = cx.new(|cx| IdeShell::new(window, cx));
        restored.update(cx, |shell, cx| shell.restore_session(window, cx));
        restored
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = restored.read(cx);
        let roots = shell.workspace.roots();
        assert_eq!(
            shell.workspace.root_paths(),
            [a.clone(), b.clone(), c.clone()]
        );
        assert!(!roots[1].search);
        assert!(roots[2].error.is_some());
        assert_eq!(shell.active_root(), Some(b.as_path()));
    });

    // Workspace definitions round-trip through Open.
    let file = base.join("team.hephaestus-workspace");
    shell.update(cx, |shell, cx| {
        shell.remove_workspace_root(&c, cx);
        shell.save_workspace_definition(&file, cx);
        shell.set_workspace(Workspace::open(&a).unwrap(), cx);
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.open(Some(file.clone()), window, cx)
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.workspace.root_paths(), [a.clone(), b.clone()]);
        assert_eq!(shell.workspace.file(), Some(file.as_path()));
        assert!(!shell.workspace.roots()[1].search);
    });
}
