//! Multiple cursors and column selection.

use super::*;

fn selection(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> Range<usize> {
    cx.read(|cx| shell.read(cx).editor().read(cx).selected_byte_range())
}

fn secondary(shell: &Entity<IdeShell>, cx: &mut VisualTestContext) -> Vec<Range<usize>> {
    cx.read(|cx| shell.read(cx).secondary_cursors().to_vec())
}

#[gpui::test]
fn select_next_occurrence_edits_every_match_as_one_undo_step(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    open_source(
        &shell,
        directory.path(),
        "a.txt",
        "foo bar\nfoo baz\nfoofoo",
        cx,
    );
    set_cursor(&shell, 0, 1, cx);
    cx.simulate_keystrokes(&primary("d"));
    assert_eq!(
        selection(&shell, cx),
        0..3,
        "the word under the caret is selected"
    );
    cx.simulate_keystrokes(&primary("d"));
    cx.simulate_keystrokes(&primary("d"));
    assert_eq!(selection(&shell, cx), 16..19);
    assert_eq!(secondary(&shell, cx), vec![0..3, 8..11]);
    cx.simulate_input("qux");
    assert_eq!(text(&shell, cx), "qux bar\nqux baz\nquxfoo");
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&shell, cx), "qu bar\nqu baz\nqufoo");
    // Undo never leaves some cursors' edits behind (typing may group into one step).
    cx.simulate_keystrokes(&primary("z"));
    assert!(
        secondary(&shell, cx).is_empty(),
        "undo returns to a single caret"
    );
    if text(&shell, cx) != "foo bar\nfoo baz\nfoofoo" {
        assert_eq!(text(&shell, cx), "qux bar\nqux baz\nquxfoo");
        cx.simulate_keystrokes(&primary("z"));
    }
    assert_eq!(text(&shell, cx), "foo bar\nfoo baz\nfoofoo");

    // Select All Occurrences, overlapping matches merge.
    set_cursor(&shell, 2, 1, cx);
    cx.simulate_keystrokes(&primary("f2"));
    assert_eq!(selection(&shell, cx), 16..22, "\"foofoo\" is its own word");
    shell.update_in(cx, |shell, window, cx| {
        shell.editor().update(cx, |editor, cx| {
            editor.select_byte_range(0..3, false, window, cx)
        })
    });
    cx.simulate_keystrokes(&primary("f2"));
    assert_eq!(selection(&shell, cx), 0..3);
    assert_eq!(secondary(&shell, cx), vec![8..11, 16..19, 19..22]);
    cx.simulate_keystrokes("escape");
    assert!(secondary(&shell, cx).is_empty());
}

#[gpui::test]
fn add_cursor_below_keeps_the_visual_column_over_tabs_and_short_lines(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    // Tab width 4: "\tx" puts x at column 4; "漢字y" puts y at column 4.
    std::fs::write(
        directory.path().join(".editorconfig"),
        "root = true\n[*]\ntab_width = 4\n",
    )
    .unwrap();
    open_source(
        &shell,
        directory.path(),
        "a.txt",
        "\tx1\n漢字y2\nab\n    z3\n",
        cx,
    );
    set_cursor(&shell, 0, 2, cx);
    let down = primary("alt-down");
    cx.simulate_keystrokes(&format!("{down} {down} {down}"));
    let source = text(&shell, cx);
    let y = source.find("y2").unwrap() + 1;
    let short = source.find("ab").unwrap() + 2;
    let z = source.find("z3").unwrap() + 1;
    assert_eq!(selection(&shell, cx), z..z);
    assert_eq!(secondary(&shell, cx), vec![2..2, y..y, short..short]);
    cx.simulate_input("_");
    assert_eq!(text(&shell, cx), "\tx_1\n漢字y_2\nab_\n    z_3\n");
    cx.simulate_keystrokes("backspace");
    cx.simulate_keystrokes("backspace");
    assert_eq!(
        text(&shell, cx),
        "\t1\n漢字2\na\n    3\n",
        "wide characters are removed whole"
    );
    // Arrow keys move every caret.
    cx.simulate_keystrokes("home");
    cx.simulate_input("#");
    assert_eq!(text(&shell, cx), "#\t1\n#漢字2\n#a\n#    3\n");
    cx.simulate_keystrokes("escape");
    cx.simulate_input("!");
    assert_eq!(text(&shell, cx), "#\t1\n#漢字2\n#a\n#!    3\n");
}

#[gpui::test]
fn column_selection_tab_indentation_and_paste_distribution(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    open_source(
        &shell,
        directory.path(),
        "a.txt",
        "let a = 1;\nx\nlet b = 2;\nlet c = 3;",
        cx,
    );
    // Rectangle from column 4 of line 0 to column 5 of line 3; "x" is too short.
    let source = text(&shell, cx);
    let anchor = 4;
    let head = source.find("c = 3").unwrap() + 1;
    shell.update_in(cx, |shell, window, cx| {
        shell.select_column(anchor, head, window, cx)
    });
    assert_eq!(selection(&shell, cx), head - 1..head);
    assert_eq!(secondary(&shell, cx).len(), 2);
    cx.simulate_input("v");
    assert_eq!(text(&shell, cx), "let v = 1;\nx\nlet v = 2;\nlet v = 3;");

    // Copy joins the selections; paste with a matching line count distributes them.
    shell.update_in(cx, |shell, window, cx| {
        shell.select_column(
            source.find("1;").unwrap(),
            source.rfind("3;").unwrap() + 1,
            window,
            cx,
        )
    });
    cx.simulate_keystrokes(&primary("c"));
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("1\n2\n3")
    );
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("one\ntwo\nthree\n".into()));
    cx.simulate_keystrokes(&primary("v"));
    assert_eq!(
        text(&shell, cx),
        "let v = one;\nx\nlet v = two;\nlet v = three;"
    );
    // One undo step.
    cx.simulate_keystrokes(&primary("z"));
    assert_eq!(text(&shell, cx), "let v = 1;\nx\nlet v = 2;\nlet v = 3;");

    // Tab indents every caret's line; a clipboard with a different line count is pasted
    // whole at each caret.
    set_cursor(&shell, 0, 0, cx);
    let down = primary("alt-down");
    cx.simulate_keystrokes(&down);
    cx.simulate_keystrokes(&down);
    assert_eq!(secondary(&shell, cx).len(), 2);
    cx.simulate_keystrokes("tab");
    assert!(text(&shell, cx).starts_with("  let v = 1;\n  x\n  let v = 2;"));
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("// ".into()));
    cx.simulate_keystrokes(&primary("v"));
    assert_eq!(
        text(&shell, cx),
        "  // let v = 1;\n  // x\n  // let v = 2;\nlet v = 3;"
    );
}

#[gpui::test]
fn multiple_cursors_stay_single_in_vim_mode_and_expand_folds(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    open_source(
        &shell,
        directory.path(),
        "a.rs",
        "fn a() {\n    x();\n}\nfn b() {}\n",
        cx,
    );
    set_cursor(&shell, 0, 8, cx);
    cx.dispatch_action(ToggleFold);
    assert!(editor_text(&shell, cx).contains("..."));
    cx.dispatch_action(AddCursorBelow);
    assert!(
        !editor_text(&shell, cx).contains("..."),
        "folds are expanded"
    );
    assert_eq!(secondary(&shell, cx).len(), 1);
    cx.simulate_input("!");
    assert_eq!(text(&shell, cx), "fn a() {!\n    x();!\n}\nfn b() {}\n");

    // IME composition edits the primary caret only and returns to a single caret.
    cx.dispatch_action(AddCursorAbove);
    assert_eq!(secondary(&shell, cx).len(), 1);
    shell.update_in(cx, |shell, window, cx| {
        shell.editor().update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "か", None, window, cx)
        })
    });
    assert_eq!(
        secondary(&shell, cx).len(),
        1,
        "composition is shown at the primary caret"
    );
    shell.update_in(cx, |shell, window, cx| {
        shell.editor().update(cx, |editor, cx| {
            editor.replace_text_in_range(None, "か", window, cx)
        })
    });
    cx.run_until_parked();
    assert!(secondary(&shell, cx).is_empty());
    cx.read(|cx| assert!(shell.read(cx).status.contains("IME")));
    assert_eq!(text(&shell, cx).matches('か').count(), 1);

    cx.simulate_keystrokes("escape");
    cx.dispatch_action(ToggleVimMode);
    cx.dispatch_action(AddCursorBelow);
    assert!(secondary(&shell, cx).is_empty());
    cx.read(|cx| assert!(shell.read(cx).status.contains("Vim mode")));
}
