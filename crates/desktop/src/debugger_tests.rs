use super::*;

fn config(program: &str, args: &[&str]) -> DebuggerConfig {
    DebuggerConfig {
        program: program.into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn an_explicit_adapter_launches_the_program_directly() {
    let plan = plan(&DebuggerConfig {
        adapter: "/opt/llvm/bin/lldb-dap".into(),
        program: "target/debug/app".into(),
        args: vec!["--fast".into()],
        ..Default::default()
    })
    .unwrap();
    assert_eq!(plan.adapter, "/opt/llvm/bin/lldb-dap");
    assert_eq!(plan.adapter_id, "lldb-dap");
    assert_eq!(plan.program, "target/debug/app");
    assert_eq!(plan.args, vec!["--fast"]);
}

#[test]
fn legacy_gdb_settings_switch_gdb_into_dap_mode() {
    let plan = plan(&config("gdb", &["--args", "./app", "one", "two"])).unwrap();
    assert_eq!(plan.adapter, "gdb");
    assert_eq!(plan.adapter_args, vec!["--interpreter=dap"]);
    assert_eq!(plan.adapter_id, "gdb");
    assert_eq!(plan.program, "./app");
    assert_eq!(plan.args, vec!["one", "two"]);

    let plan = plan_for("gdb", &["-q", "./app"]);
    assert_eq!((plan.program.as_str(), plan.args.len()), ("./app", 0));
}

fn plan_for(program: &str, args: &[&str]) -> Plan {
    plan(&config(program, args)).unwrap()
}

#[test]
fn legacy_lldb_settings_use_the_neighbouring_lldb_dap() {
    let plan = plan_for("/usr/bin/lldb", &["./app", "--", "input.txt"]);
    assert_eq!(
        PathBuf::from(&plan.adapter),
        Path::new("/usr/bin").join(format!("lldb-dap{}", std::env::consts::EXE_SUFFIX))
    );
    assert_eq!(plan.adapter_id, "lldb-dap");
    assert_eq!(plan.program, "./app");
    assert_eq!(plan.args, vec!["input.txt"]);
    assert_eq!(plan_for("lldb", &["./app"]).adapter, "lldb-dap");
}

#[test]
fn a_bare_program_uses_the_platform_adapter() {
    let plan = plan_for("target/debug/app", &["x"]);
    assert_eq!(
        (plan.adapter.clone(), plan.adapter_args.clone()),
        default_adapter()
    );
    assert_eq!(plan.program, "target/debug/app");
    assert_eq!(plan.args, vec!["x"]);
}

#[test]
fn missing_programs_are_reported() {
    assert!(plan(&DebuggerConfig::default()).is_err());
    assert!(plan(&config("gdb", &[])).is_err());
    assert!(
        plan(&DebuggerConfig {
            adapter: "lldb-dap".into(),
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn launch_arguments_resolve_the_program_and_format_env_per_adapter() {
    let directory = Path::new("/work");
    let mut settings = DebuggerConfig {
        name: "app".into(),
        adapter: "lldb-dap".into(),
        program: "bin/app".into(),
        args: vec!["-v".into()],
        stop_on_entry: true,
        ..Default::default()
    };
    settings.env.insert("B".into(), "2".into());
    settings.env.insert("A".into(), "1".into());
    let arguments = launch_arguments(&plan(&settings).unwrap(), &settings, Some(directory));
    assert_eq!(arguments["request"], "launch");
    assert_eq!(arguments["program"], json!(directory.join("bin/app")));
    assert_eq!(arguments["cwd"], json!(directory));
    assert_eq!(arguments["args"], json!(["-v"]));
    assert_eq!(arguments["stopOnEntry"], json!(true));
    assert_eq!(arguments["env"], json!(["A=1", "B=2"]));

    settings.adapter = "gdb".into();
    let arguments = launch_arguments(&plan(&settings).unwrap(), &settings, None);
    assert_eq!(arguments["program"], json!("bin/app"));
    assert_eq!(arguments["env"], json!({"A": "1", "B": "2"}));
    assert_eq!(arguments["stopAtBeginningOfMainSubprogram"], json!(true));
    assert!(arguments.get("cwd").is_none());
}

#[test]
fn verbatim_windows_prefixes_are_removed_for_adapters() {
    assert_eq!(
        plain_path(Path::new(r"\\?\C:\src\main.rs")),
        PathBuf::from(r"C:\src\main.rs")
    );
    assert_eq!(
        plain_path(Path::new("/src/main.rs")),
        PathBuf::from("/src/main.rs")
    );
}

#[test]
fn memory_renders_as_a_hex_dump_with_ascii() {
    let bytes: Vec<u8> = (0x40..0x40 + 20).collect();
    let rows = hex_rows(Some(0x1000), 0, &bytes);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "0000000000001000");
    assert_eq!(
        rows[0].1,
        "40 41 42 43 44 45 46 47  48 49 4a 4b 4c 4d 4e 4f"
    );
    assert_eq!(rows[0].2, "@ABCDEFGHIJKLMNO");
    assert_eq!(rows[1].0, "0000000000001010");
    assert_eq!(rows[1].2, "PQRS");

    let rows = hex_rows(None, 256, &[0, b'a']);
    assert_eq!(rows[0], ("+000100".into(), "00 61".into(), "·a".into()));
}

#[test]
fn pointer_values_offer_a_memory_reference() {
    let variable = |value: &str, reference: Option<&str>| Variable {
        value: value.into(),
        memory_reference: reference.map(str::to_string),
        ..Default::default()
    };
    assert_eq!(
        pointer_reference(&variable("42", Some("0x10"))).as_deref(),
        Some("0x10")
    );
    assert_eq!(
        pointer_reference(&variable("(char *) 0x7ffe12, \"hi\"", None)).as_deref(),
        Some("0x7ffe12")
    );
    assert_eq!(pointer_reference(&variable("42", None)), None);
}

mod state {
    use std::path::{Path, PathBuf};

    use gpui::TestAppContext;
    use serde_json::json;

    use super::super::*;

    fn debugger(cx: &mut TestAppContext) -> (gpui::Entity<Debugger>, &mut gpui::VisualTestContext) {
        cx.update(gpui_component::init);
        let mut debugger = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let entity = cx.new(|cx| Debugger::new(window, cx));
            debugger = Some(entity.clone());
            gpui_component::Root::new(entity, window, cx)
        });
        (debugger.unwrap(), cx)
    }

    #[gpui::test]
    fn breakpoint_options_respect_adapter_capabilities_and_follow_edits(cx: &mut TestAppContext) {
        let (debugger, cx) = debugger(cx);
        let path = PathBuf::from("/w/src/main.rs");
        debugger.update(cx, |debugger, cx| {
            debugger.toggle_breakpoint(path.clone(), 3, cx);
            debugger.set_breakpoint_options(
                path.clone(),
                7,
                BreakpointOptions {
                    enabled: true,
                    condition: Some("i > 2".into()),
                    hit_condition: Some("4".into()),
                    log_message: Some("i = {i}".into()),
                },
                cx,
            );
            debugger.toggle_breakpoint(path.clone(), 10, cx);
            assert_eq!(
                debugger.toggle_breakpoint_enabled(path.clone(), 10, cx),
                Some(false)
            );
            debugger.adopt_capabilities(json!({
                "supportsConditionalBreakpoints": true,
                "supportsHitConditionalBreakpoints": true,
                "exceptionBreakpointFilters": [
                    {"filter": "rust_panic", "label": "Rust panics", "default": true},
                    {"filter": "cpp_throw", "label": "C++ throw"}
                ]
            }));
            let (lines, sent) = debugger.source_breakpoints(&path);
            assert_eq!(lines, [3, 7], "disabled breakpoints are not sent");
            assert_eq!(sent[1].condition.as_deref(), Some("i > 2"));
            assert_eq!(sent[1].hit_condition.as_deref(), Some("4"));
            assert_eq!(
                sent[1].log_message, None,
                "log points are unsupported by this adapter"
            );
            assert_eq!(debugger.enabled_filters(), ["rust_panic"]);

            // Two lines inserted above line 7, then line 4 deleted.
            debugger.shift_breakpoints(&path, 5, 0, 2, cx);
            assert_eq!(
                debugger
                    .lines(&path)
                    .unwrap()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                [3, 9, 12]
            );
            assert_eq!(
                debugger.options_at(&path, 9).condition.as_deref(),
                Some("i > 2")
            );
            debugger.shift_breakpoints(&path, 3, 1, 0, cx);
            assert_eq!(
                debugger
                    .lines(&path)
                    .unwrap()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                [3, 8, 11]
            );
            assert!(!debugger.options_at(&path, 11).enabled);
        });
    }

    #[gpui::test]
    fn state_persists_with_relative_paths_and_watches_keep_order(cx: &mut TestAppContext) {
        let (debugger, cx) = debugger(cx);
        let root = Path::new("/w");
        debugger.update(cx, |debugger, cx| {
            debugger.toggle_breakpoint(root.join("a.rs"), 2, cx);
            debugger.adopt_capabilities(
                json!({"exceptionBreakpointFilters": [{"filter": "all", "label": "All"}]}),
            );
            debugger.set_exception_filter("all", true, cx);
            for expression in ["first", "second", "third"] {
                debugger.add_watch(expression.into(), cx);
            }
            debugger.move_watch(2, true, cx);
            debugger.remove_watch(0, cx);
            let state = debugger.export_state(root, Some("Attach".into()));
            assert_eq!(state.breakpoints[0].path, PathBuf::from("a.rs"));
            assert_eq!(state.watches, ["third", "second"]);
            assert_eq!(state.exception_filters.get("all"), Some(&true));

            let other = Path::new("/elsewhere");
            debugger.import_state(other, &state, cx);
            assert!(debugger.lines(&other.join("a.rs")).is_some());
            assert!(debugger.lines(&root.join("a.rs")).is_none());
            assert!(
                debugger
                    .watches()
                    .iter()
                    .all(|watch| matches!(watch.value, features::WatchValue::Stale(None)))
            );
            assert!(debugger.exception_filters()[0].enabled);
        });
    }

    #[test]
    fn attach_requests_carry_the_process_and_adapter_arguments() {
        let config = DebuggerConfig {
            name: "Attach".into(),
            request: crate::settings::DebugRequest::Attach,
            adapter: "lldb-dap".into(),
            process_id: Some(4242),
            arguments: Some(json!({"waitFor": true})),
            ..Default::default()
        };
        let plan = plan(&config).unwrap();
        assert_eq!(plan.program, "");
        let arguments = attach_arguments(&plan, &config, Some(Path::new("/w")));
        assert_eq!(arguments["request"], json!("attach"));
        assert_eq!(arguments["pid"], json!(4242));
        assert_eq!(arguments["processId"], json!(4242));
        assert_eq!(arguments["waitFor"], json!(true));
        assert!(arguments.get("program").is_none());
    }
}
