//! Planning a debug session: choosing the adapter and building launch/attach
//! arguments from a debug profile, plus small helpers for memory and variables.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub adapter: String,
    pub adapter_args: Vec<String>,
    pub adapter_id: String,
    pub program: String,
    pub args: Vec<String>,
}

impl Plan {
    fn lldb(&self) -> bool {
        self.adapter_id.contains("lldb")
    }

    fn gdb(&self) -> bool {
        self.adapter_id.contains("gdb")
    }
}

pub(super) fn stem(program: &str) -> String {
    Path::new(program)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

pub(super) fn legacy_target(args: &[String]) -> Option<(String, Vec<String>)> {
    if let Some(ix) = args.iter().position(|arg| arg == "--args") {
        let mut rest = args[ix + 1..].iter().cloned();
        return Some((rest.next()?, rest.collect()));
    }
    let ix = args.iter().position(|arg| !arg.starts_with('-'))?;
    let rest = &args[ix + 1..];
    let program_args = match rest.iter().position(|arg| arg == "--") {
        Some(separator) => rest[separator + 1..].to_vec(),
        None => Vec::new(),
    };
    Some((args[ix].clone(), program_args))
}

pub(super) fn sibling(program: &str, name: &str) -> String {
    let path = Path::new(program);
    match path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        Some(parent) => parent
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
            .to_string_lossy()
            .into_owned(),
        None => name.into(),
    }
}

pub(super) fn default_adapter() -> (String, Vec<String>) {
    if cfg!(target_os = "linux") {
        ("gdb".into(), vec!["--interpreter=dap".into()])
    } else {
        ("lldb-dap".into(), Vec::new())
    }
}

pub fn plan(config: &DebuggerConfig) -> Result<Plan, String> {
    // Attaching does not need a program: plan the adapter, then drop the placeholder.
    if config.request == crate::settings::DebugRequest::Attach && config.program.trim().is_empty() {
        let mut with_program = config.clone();
        with_program.program = "attach".into();
        let mut plan = plan(&with_program)?;
        plan.program.clear();
        plan.args.clear();
        return Ok(plan);
    }
    let missing =
        || "Set \"program\" in the debugger settings to the executable to debug".to_string();
    let (adapter, adapter_args, program, args) = if !config.adapter.trim().is_empty() {
        if config.program.trim().is_empty() {
            return Err(missing());
        }
        (
            config.adapter.clone(),
            config.adapter_args.clone(),
            config.program.clone(),
            config.args.clone(),
        )
    } else {
        let name = stem(&config.program);
        if config.program.trim().is_empty() {
            return Err(missing());
        } else if name.contains("dap") || name.contains("vscode") {
            let (program, args) = config
                .args
                .split_first()
                .map(|(program, args)| (program.clone(), args.to_vec()))
                .ok_or_else(missing)?;
            (config.program.clone(), Vec::new(), program, args)
        } else if name.contains("gdb") {
            let (program, args) = legacy_target(&config.args).ok_or_else(missing)?;
            (
                config.program.clone(),
                vec!["--interpreter=dap".into()],
                program,
                args,
            )
        } else if name.contains("lldb") {
            let (program, args) = legacy_target(&config.args).ok_or_else(missing)?;
            (
                sibling(&config.program, "lldb-dap"),
                Vec::new(),
                program,
                args,
            )
        } else {
            let (adapter, adapter_args) = default_adapter();
            (
                adapter,
                adapter_args,
                config.program.clone(),
                config.args.clone(),
            )
        }
    };
    let adapter_id = match stem(&adapter) {
        name if name.contains("gdb") => "gdb".into(),
        name if name.contains("lldb") => "lldb-dap".into(),
        name => name,
    };
    Ok(Plan {
        adapter,
        adapter_args,
        adapter_id,
        program,
        args,
    })
}

pub fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

pub(super) fn same_file(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Merge adapter-specific `arguments` from the profile into a request.
pub(super) fn merge_arguments(mut arguments: Value, config: &DebuggerConfig) -> Value {
    if let (Some(Value::Object(extra)), Value::Object(target)) = (&config.arguments, &mut arguments)
    {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    arguments
}

pub fn attach_arguments(plan: &Plan, config: &DebuggerConfig, directory: Option<&Path>) -> Value {
    let mut arguments = json!({
        "name": if config.name.is_empty() { "Attach" } else { config.name.as_str() },
        "type": plan.adapter_id,
        "request": "attach",
    });
    if let Some(pid) = config.process_id {
        // Adapters disagree on the key: lldb-dap/gdb use `pid`, others `processId`.
        arguments["pid"] = json!(pid);
        arguments["processId"] = json!(pid);
    }
    if !plan.program.trim().is_empty() {
        arguments["program"] = json!(plan.program);
    }
    if let Some(directory) = directory {
        arguments["cwd"] = json!(plain_path(directory));
    }
    merge_arguments(arguments, config)
}

pub fn launch_arguments(plan: &Plan, config: &DebuggerConfig, directory: Option<&Path>) -> Value {
    let program = match directory {
        Some(directory) if Path::new(&plan.program).is_relative() => {
            plain_path(&directory.join(&plan.program))
        }
        _ => PathBuf::from(&plan.program),
    };
    let mut arguments = json!({
        "name": if config.name.is_empty() { "Debug" } else { config.name.as_str() },
        "type": plan.adapter_id,
        "request": "launch",
        "program": program,
        "args": plan.args,
        "stopOnEntry": config.stop_on_entry,
    });
    if let Some(directory) = directory {
        arguments["cwd"] = json!(plain_path(directory));
    }
    if !config.env.is_empty() {
        arguments["env"] = if plan.lldb() {
            let mut pairs: Vec<String> = config
                .env
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            pairs.sort();
            json!(pairs)
        } else {
            json!(config.env)
        };
    }
    if plan.gdb() && config.stop_on_entry {
        arguments["stopAtBeginningOfMainSubprogram"] = json!(true);
    }
    merge_arguments(arguments, config)
}

pub fn hex_rows(start: Option<u64>, offset: i64, bytes: &[u8]) -> Vec<(String, String, String)> {
    bytes
        .chunks(MEMORY_ROW)
        .enumerate()
        .map(|(ix, chunk)| {
            let row = (ix * MEMORY_ROW) as i64;
            let address = match start {
                Some(start) => format!("{:016x}", start.wrapping_add(row as u64)),
                None => format!("+{:06x}", offset + row),
            };
            let hex = chunk
                .iter()
                .enumerate()
                .map(|(ix, byte)| {
                    if ix == MEMORY_ROW / 2 {
                        format!(" {byte:02x}")
                    } else {
                        format!("{byte:02x}")
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let ascii = chunk
                .iter()
                .map(|byte| {
                    if byte.is_ascii_graphic() || *byte == b' ' {
                        *byte as char
                    } else {
                        '·'
                    }
                })
                .collect();
            (address, hex, ascii)
        })
        .collect()
}

pub(super) fn pointer_reference(variable: &Variable) -> Option<String> {
    if let Some(reference) = &variable.memory_reference {
        return Some(reference.clone());
    }
    let token = variable
        .value
        .split_whitespace()
        .find(|token| token.starts_with("0x"))?
        .trim_end_matches([',', ')', '}']);
    dap::parse_address(token).map(|_| token.to_string())
}
