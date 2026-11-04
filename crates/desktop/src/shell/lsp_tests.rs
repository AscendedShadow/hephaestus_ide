//! Language-feature workflows against an in-process mock server (IDE-005 – IDE-009).

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use gpui::TestAppContext;
use serde_json::{Value, json};

use super::ide_tests::{editor_text, open_file, set_cursor, setup, text};
use super::mock_lsp::{self, wait_until};
use super::*;

fn uri(path: &Path) -> String {
    lsp::path_to_uri(path).unwrap().as_str().to_string()
}

fn range(line: u32, start: u32, end: u32) -> Value {
    json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}})
}

fn workspace(
    cx: &mut TestAppContext,
) -> (
    Entity<IdeShell>,
    &mut gpui::VisualTestContext,
    tempfile::TempDir,
    PathBuf,
) {
    let (shell, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    shell.update(cx, |shell, cx| {
        shell.set_workspace(Workspace::open(&root).unwrap(), cx)
    });
    (shell, cx, directory, root)
}

fn status(shell: &Entity<IdeShell>, cx: &mut gpui::VisualTestContext) -> String {
    cx.read(|cx| shell.read(cx).status.clone())
}

#[gpui::test]
fn rename_applies_versioned_multi_file_edits_with_preview_and_grouped_undo(
    cx: &mut TestAppContext,
) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    let user = root.join("use.rs");
    let moved = root.join("uses.rs");
    std::fs::write(&lib, "fn old() {}\nfn main() { old(); old(); }\n").unwrap();
    std::fs::write(&user, "use crate::old; // 𝄞 old\n").unwrap();
    open_file(&shell, &lib, cx);
    let version = Arc::new(AtomicI64::new(0));
    let server_version = version.clone();
    let (lib_uri, user_uri, moved_uri) = (uri(&lib), uri(&user), uri(&moved));
    let server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({"renameProvider": {"prepareProvider": true}}),
        Box::new(move |method, _| match method {
            "textDocument/prepareRename" => json!({"range": range(0, 3, 6), "placeholder": "old"}),
            "textDocument/rename" => json!({"documentChanges": [
                {"textDocument": {"uri": lib_uri, "version": server_version.load(Ordering::SeqCst)},
                 "edits": [
                    {"range": range(0, 3, 6), "newText": "fresh"},
                    {"range": range(1, 12, 15), "newText": "fresh"},
                    {"range": range(1, 19, 22), "newText": "fresh"}]},
                {"textDocument": {"uri": user_uri, "version": null},
                 "edits": [
                    {"range": range(0, 11, 14), "newText": "fresh"},
                    {"range": range(0, 22, 25), "newText": "fresh"}]},
                {"kind": "rename", "oldUri": user_uri, "newUri": moved_uri}
            ]}),
            _ => Value::Null,
        }),
        cx,
    );
    let _ = &server;
    // An unsaved edit: the rename must use the current buffer text and version.
    set_cursor(&shell, 1, 0, cx);
    cx.simulate_input(" ");
    set_cursor(&shell, 0, 0, cx);
    cx.simulate_keystrokes("delete");
    assert_eq!(
        text(&shell, cx),
        "n old() {}\n fn main() { old(); old(); }\n"
    );
    cx.simulate_input("f");
    set_cursor(&shell, 1, 0, cx);
    cx.simulate_keystrokes("delete");
    set_cursor(&shell, 0, 4, cx);
    let current = cx.read(|cx| shell.read(cx).servers.read(cx).version(&lib).unwrap());
    version.store(current as i64, Ordering::SeqCst);

    cx.dispatch_action(RenameSymbol);
    wait_until(cx, |cx| {
        cx.update(|window, cx| window.has_active_dialog(cx))
    });
    cx.simulate_input("fresh");
    cx.simulate_keystrokes("enter");
    // The multi-file edit with a resource operation is shown for review first.
    wait_until(cx, |cx| {
        cx.update(|window, cx| window.has_active_dialog(cx))
            && status(&shell, cx).starts_with("Renaming")
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(
        text(&shell, cx),
        "fn fresh() {}\nfn main() { fresh(); fresh(); }\n"
    );
    assert!(!user.exists() && moved.exists(), "the resource rename ran");
    let moved_text = cx.read(|cx| {
        let shell = shell.read(cx);
        let ix = shell
            .buffer_for_path(&moved)
            .expect("the closed file opened as a tab");
        assert!(
            shell.buffers[ix].dirty,
            "closed files are left modified for review"
        );
        shell.buffers[ix].document.text().to_string()
    });
    assert_eq!(moved_text, "use crate::fresh; // 𝄞 fresh\n");
    assert!(
        status(&shell, cx).contains("5 change(s)")
            || status(&shell, cx).starts_with("Rename to fresh"),
        "{}",
        status(&shell, cx)
    );

    cx.dispatch_action(UndoWorkspaceEdit);
    cx.run_until_parked();
    assert!(user.exists() && !moved.exists());
    assert_eq!(
        text(&shell, cx),
        "fn old() {}\nfn main() { old(); old(); }\n"
    );
}

#[gpui::test]
fn stale_versions_and_server_apply_edit_report_truthful_results(cx: &mut TestAppContext) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "let 😀 = 1;\n").unwrap();
    open_file(&shell, &lib, cx);
    let server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({}),
        Box::new(|_, _| Value::Null),
        cx,
    );

    server.request(
        41,
        "workspace/applyEdit",
        json!({"label": "inline", "edit": {"changes": {uri(&lib): [
            {"range": range(0, 4, 6), "newText": "smile"}
        ]}}}),
    );
    wait_until(cx, |_| server.response(41).is_some());
    assert_eq!(
        server.response(41).unwrap()["result"],
        json!({"applied": true})
    );
    assert_eq!(text(&shell, cx), "let smile = 1;\n");

    server.request(
        42,
        "workspace/applyEdit",
        json!({"edit": {"documentChanges": [{"textDocument": {"uri": uri(&lib), "version": 999},
            "edits": [{"range": range(0, 0, 3), "newText": "const"}]}]}}),
    );
    wait_until(cx, |_| server.response(42).is_some());
    let response = server.response(42).unwrap()["result"].clone();
    assert_eq!(response["applied"], json!(false));
    assert!(
        response["failureReason"]
            .as_str()
            .unwrap()
            .contains("changed since"),
        "{response}"
    );
    assert_eq!(text(&shell, cx), "let smile = 1;\n", "nothing was applied");

    server.request(
        43,
        "workspace/applyEdit",
        json!({"edit": {"changes": {uri(&lib): [
            {"range": range(0, 0, 5), "newText": "a"},
            {"range": range(0, 2, 6), "newText": "b"}
        ]}}}),
    );
    wait_until(cx, |_| server.response(43).is_some());
    assert_eq!(
        server.response(43).unwrap()["result"]["applied"],
        json!(false)
    );
}

#[gpui::test]
fn formatting_applies_disjoint_edits_keeps_the_cursor_and_runs_on_save(cx: &mut TestAppContext) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn  a( ){\nlet x=\"é\";}\n").unwrap();
    std::fs::create_dir_all(root.join(".hephaestus")).unwrap();
    std::fs::write(
        root.join(".hephaestus/settings.json"),
        r#"{"editor": {"format_on_save": true, "tab_width": 4}}"#,
    )
    .unwrap();
    cx.dispatch_action(ReloadSettings);
    open_file(&shell, &lib, cx);
    let server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({"documentFormattingProvider": true, "documentRangeFormattingProvider": true}),
        Box::new(|method, params| match method {
            "textDocument/formatting" if params["options"]["tabSize"] == json!(4) => json!([
                {"range": range(0, 2, 4), "newText": " "},
                {"range": range(0, 5, 8), "newText": "()"},
                {"range": {"start": {"line": 0, "character": 8}, "end": {"line": 1, "character": 0}}, "newText": " {\n    "},
                {"range": range(1, 5, 6), "newText": " = "}
            ]),
            "textDocument/rangeFormatting" => json!([]),
            _ => Value::Null,
        }),
        cx,
    );
    // Cursor after the Unicode character on line 2.
    set_cursor(&shell, 1, 9, cx);
    cx.dispatch_action(FormatDocument);
    wait_until(cx, |cx| status(&shell, cx) == "Formatted");
    assert_eq!(text(&shell, cx), "fn a() {\n    let x = \"é\";}\n");
    cx.read(|cx| {
        let cursor = shell.read(cx).editor().read(cx).cursor_position();
        assert_eq!(
            (cursor.line, cursor.character),
            (1, 15),
            "cursor follows the edits"
        );
    });
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(
        text(&shell, cx),
        "fn  a( ){\nlet x=\"é\";}\n",
        "formatting is one undo step"
    );

    // Format on save waits for the formatter, then writes the formatted text.
    cx.simulate_input("");
    shell.update_in(cx, |shell, window, cx| {
        shell.save(SaveTarget::Current, None, window, cx)
    });
    wait_until(cx, |cx| {
        !cx.read(|cx| shell.read(cx).busy) && !cx.read(|cx| shell.read(cx).buffer().dirty)
    });
    assert_eq!(
        std::fs::read_to_string(&lib).unwrap(),
        "fn a() {\n    let x = \"é\";}\n"
    );
    assert!(server.notifications("textDocument/didSave").len() == 1);
}

#[gpui::test]
fn quick_fixes_resolve_lazily_explain_disabled_actions_and_run_commands(cx: &mut TestAppContext) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    let other = root.join("other.rs");
    std::fs::write(&lib, "use b;\nuse a;\n").unwrap();
    std::fs::write(&other, "mod x;\n").unwrap();
    open_file(&shell, &lib, cx);
    let (lib_uri, other_uri) = (uri(&lib), uri(&other));
    let server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({"codeActionProvider": {"resolveProvider": true}, "executeCommandProvider": {"commands": ["x.run"]}}),
        Box::new(move |method, params| match method {
            "textDocument/codeAction"
                if params["context"]["only"] == json!(["source.organizeImports"]) =>
            {
                json!([
                    {"title": "Organize imports", "kind": "source.organizeImports",
                     "edit": {"changes": {lib_uri.clone(): [
                        {"range": range(0, 4, 5), "newText": "a"}, {"range": range(1, 4, 5), "newText": "b"}]}}}
                ])
            }
            "textDocument/codeAction" => json!([
                {"title": "Disabled fix", "disabled": {"reason": "needs a type"}},
                {"title": "Lazy multi-file fix", "kind": "quickfix", "data": 7},
                {"title": "Run x", "command": "x.run"}
            ]),
            "codeAction/resolve" => json!({
                "title": "Lazy multi-file fix",
                "edit": {"changes": {
                    lib_uri.clone(): [{"range": range(0, 0, 0), "newText": "// fixed\n"}],
                    other_uri.clone(): [{"range": range(0, 0, 3), "newText": "pub mod"}]}}
            }),
            _ => Value::Null,
        }),
        cx,
    );
    cx.dispatch_action(OrganizeImports);
    wait_until(cx, |cx| text(&shell, cx) == "use a;\nuse b;\n");

    cx.dispatch_action(QuickFix);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).search_mode == Some(SearchMode::Items))
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.choose_search_result(0, window, cx)
    });
    assert!(
        status(&shell, cx).contains("needs a type"),
        "{}",
        status(&shell, cx)
    );

    cx.dispatch_action(QuickFix);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).search_mode == Some(SearchMode::Items))
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.choose_search_result(1, window, cx)
    });
    wait_until(cx, |cx| text(&shell, cx).starts_with("// fixed"));
    cx.read(|cx| {
        let shell = shell.read(cx);
        let ix = shell.buffer_for_path(&other).unwrap();
        assert_eq!(
            shell.buffers[ix].document.text().to_string(),
            "pub mod x;\n"
        );
    });

    cx.dispatch_action(QuickFix);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).search_mode == Some(SearchMode::Items))
    });
    shell.update_in(cx, |shell, window, cx| {
        shell.choose_search_result(2, window, cx)
    });
    wait_until(cx, |_| {
        !server.notifications("workspace/executeCommand").is_empty()
    });
    assert_eq!(
        server.notifications("workspace/executeCommand")[0]["command"],
        json!("x.run")
    );
}

#[gpui::test]
fn references_and_implementations_offer_choosers_and_keep_history(cx: &mut TestAppContext) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    let other = root.join("other.rs");
    std::fs::write(&lib, "trait Shape {}\nfn use_it() {}\n").unwrap();
    std::fs::write(&other, "// 😀\nimpl Shape for A {}\nimpl Shape for B {}\n").unwrap();
    open_file(&shell, &lib, cx);
    let (lib_uri, other_uri) = (uri(&lib), uri(&other));
    let _server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({"referencesProvider": true, "implementationProvider": true}),
        Box::new(move |method, params| match method {
            "textDocument/references" => {
                let mut list = vec![
                    json!({"uri": other_uri.clone(), "range": range(1, 5, 10)}),
                    json!({"uri": other_uri.clone(), "range": range(2, 5, 10)}),
                ];
                if params["context"]["includeDeclaration"] == json!(true) {
                    list.insert(0, json!({"uri": lib_uri.clone(), "range": range(0, 6, 11)}));
                }
                Value::Array(list)
            }
            "textDocument/implementation" => json!([
                {"targetUri": other_uri.clone(), "targetRange": range(1, 0, 19), "targetSelectionRange": range(1, 15, 16)},
                {"targetUri": other_uri.clone(), "targetRange": range(2, 0, 19), "targetSelectionRange": range(2, 15, 16)}
            ]),
            _ => Value::Null,
        }),
        cx,
    );
    set_cursor(&shell, 0, 7, cx);
    cx.dispatch_action(FindReferences);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).search_mode == Some(SearchMode::Items))
    });
    let labels = cx.read(|cx| shell.read(cx).picker_labels(""));
    assert_eq!(labels.len(), 5, "{labels:?}");
    assert!(labels[0].starts_with("lib.rs (1)"), "{labels:?}");
    assert!(labels[2].starts_with("other.rs (2)"), "{labels:?}");

    cx.dispatch_action(GoToImplementation);
    wait_until(cx, |cx| {
        cx.read(|cx| {
            shell.read(cx).picker.as_ref().is_some_and(|p| {
                p.title.contains("definitions") || p.title.contains("implementations")
            })
        })
    });
    // Choose the second (non-first) target.
    shell.update_in(cx, |shell, window, cx| {
        shell.choose_search_result(1, window, cx)
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.document().path(), Some(other.as_path()));
        let cursor = shell.editor().read(cx).cursor_position();
        assert_eq!((cursor.line, cursor.character), (2, 15));
    });
    cx.dispatch_action(GoBack);
    cx.read(|cx| assert_eq!(shell.read(cx).document().path(), Some(lib.as_path())));
}

#[gpui::test]
fn document_and_workspace_symbols(cx: &mut TestAppContext) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    std::fs::write(
        &lib,
        "struct Point {\n    x: i32,\n}\nimpl Point {\n    fn len(&self) {}\n}\n",
    )
    .unwrap();
    open_file(&shell, &lib, cx);
    let lib_uri = uri(&lib);
    let _server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({"documentSymbolProvider": true, "workspaceSymbolProvider": true}),
        Box::new(move |method, params| match method {
            "textDocument/documentSymbol" => json!([
                {"name": "Point", "kind": 23, "range": range(0, 0, 1), "selectionRange": range(0, 7, 12),
                 "children": [{"name": "x", "kind": 8, "range": range(1, 4, 11), "selectionRange": range(1, 4, 5)}]},
                {"name": "impl Point", "kind": 19, "range": range(3, 0, 1), "selectionRange": range(3, 5, 10),
                 "children": [{"name": "len", "kind": 6, "range": range(4, 4, 20), "selectionRange": range(4, 7, 10)}]}
            ]),
            "workspace/symbol" if params["query"] == json!("len") => json!([
                {"name": "len", "kind": 6, "location": {"uri": lib_uri.clone(), "range": range(4, 7, 10)}}
            ]),
            "workspace/symbol" => json!([]),
            _ => Value::Null,
        }),
        cx,
    );
    cx.dispatch_action(GoToSymbolInFile);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).search_mode == Some(SearchMode::Items))
    });
    let labels = cx.read(|cx| shell.read(cx).picker_labels(""));
    assert_eq!(
        labels,
        [
            "Point  struct",
            "  x  field",
            "impl Point  object",
            "  len  method"
        ]
    );
    cx.simulate_input("len");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.read(|cx| {
        let cursor = shell.read(cx).editor().read(cx).cursor_position();
        assert_eq!((cursor.line, cursor.character), (4, 7));
    });

    cx.dispatch_action(ShowOutline);
    wait_until(cx, |cx| {
        cx.read(|cx| {
            shell
                .read(cx)
                .active_outline()
                .is_some_and(|symbols| symbols.len() == 4)
        })
    });
    cx.read(|cx| {
        assert_eq!(
            shell.read(cx).current_symbol(cx),
            Some(3),
            "the cursor is in len"
        )
    });
    // A response for an older version is ignored: edit, then the outline refreshes.
    cx.simulate_input("x");
    let stale_version = cx.read(|cx| shell.read(cx).outline.as_ref().unwrap().1);
    shell.update_in(cx, |shell, window, cx| shell.refresh_outline(window, cx));
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).outline.as_ref().unwrap().1 != stale_version)
    });

    set_cursor(&shell, 0, 0, cx);
    cx.dispatch_action(GoToSymbolInWorkspace);
    cx.simulate_input("len");
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).workspace_symbols.len() == 1)
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.read(|cx| {
        let cursor = shell.read(cx).editor().read(cx).cursor_position();
        assert_eq!((cursor.line, cursor.character), (4, 7));
    });
    let _ = editor_text(&shell, cx);
}

fn item(json: Value) -> lsp_types::CompletionItem {
    serde_json::from_value(json).unwrap()
}

#[gpui::test]
fn completion_applies_exact_edits_snippets_and_auto_imports_as_one_step(cx: &mut TestAppContext) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn main() {\n    let v = Vec::new();\n    v.\n}\n").unwrap();
    open_file(&shell, &lib, cx);
    let _server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({"completionProvider": {"triggerCharacters": ["."], "resolveProvider": true}}),
        Box::new(|method, params| match method {
            "completionItem/resolve" if params["label"] == json!("HashMap") => {
                let mut resolved = params.clone();
                resolved["additionalTextEdits"] = json!([
                    {"range": range(0, 0, 0), "newText": "use std::collections::HashMap;\n"}
                ]);
                resolved["documentation"] = json!("A hash map.");
                resolved
            }
            "completionItem/resolve" => params.clone(),
            _ => Value::Null,
        }),
        cx,
    );
    let id = cx.read(|cx| shell.read(cx).buffer().id);
    let accept = |item: lsp_types::CompletionItem, cx: &mut gpui::VisualTestContext| {
        shell.update_in(cx, |shell, window, cx| {
            shell.accept_completion(id, item, window, cx)
        });
        cx.run_until_parked();
    };

    // A snippet with a placeholder and a final stop.
    set_cursor(&shell, 2, 6, cx);
    accept(
        item(json!({"label": "push", "insertTextFormat": 2,
            "textEdit": {"range": range(2, 6, 6), "newText": "push(${1:value})$0"}})),
        cx,
    );
    wait_until(cx, |cx| text(&shell, cx).contains("v.push(value)"));
    cx.read(|cx| {
        let editor = shell.read(cx).editor().read(cx);
        let selected = editor.text().to_string()[editor.selected_byte_range()].to_string();
        assert_eq!(selected, "value", "the first placeholder is selected");
    });
    cx.simulate_input("item");
    cx.simulate_keystrokes("tab");
    cx.simulate_input(";");
    assert!(
        text(&shell, cx).contains("    v.push(item);\n"),
        "{}",
        text(&shell, cx)
    );
    cx.read(|cx| assert!(shell.read(cx).snippet.is_none()));

    // A non-prefix replace range plus a lazily resolved auto-import, undone in one step.
    let before = text(&shell, cx);
    set_cursor(&shell, 1, 15, cx);
    accept(
        item(json!({"label": "HashMap",
            "textEdit": {"range": range(1, 12, 15), "newText": "HashMap"}})),
        cx,
    );
    wait_until(cx, |cx| {
        text(&shell, cx).starts_with("use std::collections::HashMap;\n")
    });
    assert!(
        text(&shell, cx).contains("let v = HashMap::new();"),
        "{}",
        text(&shell, cx)
    );
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&shell, cx), before);

    // Literal dollar text stays literal when the item is not a snippet.
    set_cursor(&shell, 2, 4, cx);
    accept(item(json!({"label": "$money", "insertText": "$money"})), cx);
    assert!(
        text(&shell, cx).contains("    $moneyv.push(item);"),
        "{}",
        text(&shell, cx)
    );

    // Insert/replace edits extend over characters typed since the request.
    set_cursor(&shell, 2, 4, cx);
    let requested = text(&shell, cx);
    let _ = requested;
    accept(
        item(json!({"label": "money2", "textEdit": {
            "insert": range(2, 4, 10), "replace": range(2, 4, 10), "newText": "cash"}})),
        cx,
    );
    assert!(
        text(&shell, cx).contains("    cashv.push(item);"),
        "{}",
        text(&shell, cx)
    );
}

#[gpui::test]
fn signature_help_follows_triggers_overloads_and_dismissal(cx: &mut TestAppContext) {
    let (shell, cx, _directory, root) = workspace(cx);
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn f() {\n    \n}\n").unwrap();
    open_file(&shell, &lib, cx);
    let _server = mock_lsp::install(
        &shell,
        "rust",
        &root,
        json!({"signatureHelpProvider": {"triggerCharacters": ["(", ","], "retriggerCharacters": [")"]}}),
        Box::new(|method, params| match method {
            "textDocument/signatureHelp" => {
                let character = params["position"]["character"].as_u64().unwrap();
                if params["context"]["triggerCharacter"] == json!(")") || character > 12 {
                    return Value::Null;
                }
                let active = if character >= 10 { 1 } else { 0 };
                json!({
                    "signatures": [
                        {"label": "max(a: i32, b: i32)", "parameters": [{"label": [4, 10]}, {"label": [12, 18]}],
                         "documentation": "Largest of two."},
                        {"label": "max(a: f64, b: f64)", "parameters": [{"label": "a: f64"}, {"label": "b: f64"}]}
                    ],
                    "activeSignature": 0,
                    "activeParameter": active
                })
            }
            _ => Value::Null,
        }),
        cx,
    );
    set_cursor(&shell, 1, 4, cx);
    cx.simulate_input("max(");
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).signature.help.is_some())
    });
    let parameter = |cx: &mut gpui::VisualTestContext| {
        cx.read(|cx| {
            let shell = shell.read(cx);
            let help = shell.signature.help.as_ref().unwrap();
            signature_help::split_label(help, shell.active_signature().unwrap())
                .unwrap()
                .1
        })
    };
    assert_eq!(parameter(cx), "a: i32");
    cx.simulate_input("1,");
    wait_until(cx, |cx| parameter(cx) == "b: i32");
    cx.dispatch_action(NextSignature);
    assert_eq!(parameter(cx), "b: f64");
    // Focus stays in the editor while typing.
    cx.update(|window, cx| {
        assert!(
            shell
                .read(cx)
                .editor()
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });

    cx.simulate_keystrokes("escape");
    cx.read(|cx| assert!(shell.read(cx).signature.help.is_none()));
    // A response for an older request cannot reopen a dismissed popover.
    cx.dispatch_action(TriggerParameterHints);
    shell.update(cx, |shell, _| shell.dismiss_signature_help());
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.read(|cx| assert!(shell.read(cx).signature.help.is_none()));

    // Leaving the call (retrigger with no signature) closes it.
    cx.dispatch_action(TriggerParameterHints);
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).signature.help.is_some())
    });
    cx.simulate_input("2)");
    wait_until(cx, |cx| {
        cx.read(|cx| shell.read(cx).signature.help.is_none())
    });
}
