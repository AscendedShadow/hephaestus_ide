//! Layered configuration: built-in defaults, user settings and workspace settings.
//!
//! Precedence (lowest to highest): defaults → user `settings.json` → workspace
//! `<root>/.hephaestus/settings.json`. Layers are merged as JSON before deserializing:
//!
//! - **Objects** merge key by key, recursively (a workspace can override one keybinding
//!   or one theme color without restating the rest).
//! - **`null`** removes the key, resetting it to the built-in default, so an omitted key
//!   ("inherit") differs from an explicit reset.
//! - **Named arrays** (`commands`, `tasks`, `debug_profiles`, `terminal_profiles`) merge
//!   by their `name` (or `id`) field: a workspace entry replaces the user entry with the
//!   same name and other entries are appended. `language_servers` merges by `program`.
//! - **Other arrays** replace the lower layer's array.
//! - **Relative paths** in the workspace layer (`working_directory`, `cwd`, and programs
//!   starting with `./` or `../`) resolve against the workspace root; relative paths in
//!   user settings keep resolving against the active project directory at launch time.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};

use super::Settings;

pub const WORKSPACE_DIRECTORY: &str = ".hephaestus";

/// Where an effective top-level value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    User,
    Workspace,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::User => "user",
            Self::Workspace => "workspace",
        }
    }
}

const NAMED_ARRAYS: &[(&str, &str)] = &[
    ("commands", "name"),
    ("tasks", "id"),
    ("debug_profiles", "name"),
    ("terminal_profiles", "name"),
    ("language_servers", "program"),
];

const PATH_KEYS: &[&str] = &["working_directory", "cwd"];

pub fn workspace_path(root: &Path) -> PathBuf {
    root.join(WORKSPACE_DIRECTORY).join(super::FILE_NAME)
}

/// Merge `overlay` into `base` following the rules in the module documentation.
pub fn merge(base: &mut Value, overlay: &Value, key: Option<&str>) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (name, value) in overlay {
                if value.is_null() {
                    base.remove(name);
                } else if let Some(existing) = base.get_mut(name) {
                    merge(existing, value, Some(name));
                } else {
                    base.insert(name.clone(), value.clone());
                }
            }
        }
        (Value::Array(base), Value::Array(overlay))
            if let Some((_, field)) =
                NAMED_ARRAYS.iter().find(|(array, _)| Some(*array) == key) =>
        {
            for item in overlay {
                let name = item.get(*field).filter(|name| !name.is_null());
                match name
                    .and_then(|name| base.iter().position(|old| old.get(*field) == Some(name)))
                {
                    Some(ix) => base[ix] = item.clone(),
                    None => base.push(item.clone()),
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

/// Resolve workspace-relative paths in a workspace layer against `root`.
pub fn resolve_paths(value: &mut Value, root: &Path) {
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if let Value::String(text) = value {
                    let relative_path = PATH_KEYS.contains(&key.as_str())
                        && !text.is_empty()
                        && Path::new(text.as_str()).is_relative()
                        && !text.starts_with("${");
                    let relative_program =
                        key == "program" && (text.starts_with("./") || text.starts_with("../"));
                    if relative_path || relative_program {
                        *text = root.join(text.as_str()).to_string_lossy().into_owned();
                    }
                } else {
                    resolve_paths(value, root);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| resolve_paths(item, root)),
        _ => {}
    }
}

/// Parse one settings layer, reporting the file and line/column of syntax errors.
pub fn parse_layer(path: &Path, source: &str) -> Result<Value, String> {
    if source.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_json::from_str(source).map_err(|error| {
        format!(
            "{}:{}:{}: {error}",
            path.display(),
            error.line(),
            error.column()
        )
    })?;
    if !value.is_object() {
        return Err(format!(
            "{}: settings must be a JSON object",
            path.display()
        ));
    }
    // Validate the layer on its own so errors point at the file that contains them.
    serde_json::from_value::<Settings>(strip_nulls(&value))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(value)
}

fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key.clone(), strip_nulls(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Read a layer file; a missing file is an empty layer.
pub fn read_layer(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(source) => parse_layer(path, &source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// The effective settings and the source of each top-level key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    pub settings: Settings,
    pub sources: BTreeMap<String, Source>,
}

pub fn resolve(user: &Value, workspace: Option<&Value>) -> Result<Resolved, String> {
    let mut merged = Value::Object(Map::new());
    let mut sources = BTreeMap::new();
    for (layer, source) in [(Some(user), Source::User), (workspace, Source::Workspace)] {
        let Some(Value::Object(layer_map)) = layer else {
            continue;
        };
        for (key, value) in layer_map {
            if value.is_null() {
                sources.insert(key.clone(), Source::Default);
            } else {
                sources.insert(key.clone(), source);
            }
        }
        merge(&mut merged, layer.unwrap(), None);
    }
    let settings: Settings =
        serde_json::from_value(merged).map_err(|error| format!("merged settings: {error}"))?;
    Ok(Resolved { settings, sources })
}

/// Render the effective settings with the source of each top-level value.
pub fn describe(
    resolved: &Resolved,
    user_path: Option<&Path>,
    workspace_path: Option<&Path>,
) -> String {
    let mut text = String::from("// Effective settings (read-only view).\n");
    text.push_str(
        "// Precedence: default < user < workspace. `null` resets a key to its default.\n",
    );
    if let Some(path) = user_path {
        text.push_str(&format!("// user:      {}\n", path.display()));
    }
    match workspace_path {
        Some(path) => text.push_str(&format!("// workspace: {}\n", path.display())),
        None => text.push_str("// workspace: (no folder open)\n"),
    }
    text.push_str("//\n// Sources of overridden keys:\n");
    let overridden: Vec<_> = resolved
        .sources
        .iter()
        .filter(|(_, source)| **source != Source::Default)
        .collect();
    if overridden.is_empty() {
        text.push_str("//   (none — everything uses the defaults)\n");
    }
    for (key, source) in overridden {
        text.push_str(&format!("//   {key}: {}\n", source.label()));
    }
    text.push_str(&resolved.settings.to_json());
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn objects_merge_named_arrays_merge_by_name_and_null_resets() {
        let user = json!({
            "show_sidebar": true,
            "keybindings": {"save_file": "alt-s", "new_file": "alt-n"},
            "commands": [
                {"name": "build", "program": "make"},
                {"name": "test", "program": "make", "args": ["test"]}
            ],
            "debugger": {"name": "user", "adapter": "lldb-dap", "program": "a.out"}
        });
        let workspace = json!({
            "keybindings": {"save_file": "ctrl-alt-s"},
            "commands": [
                {"name": "test", "program": "cargo", "args": ["test"]},
                {"name": "lint", "program": "cargo", "args": ["clippy"]}
            ],
            "debugger": null
        });
        let resolved = resolve(&user, Some(&workspace)).unwrap();
        let settings = &resolved.settings;
        assert!(settings.show_sidebar);
        assert_eq!(settings.keybindings.len(), 2);
        let names: Vec<_> = settings
            .commands
            .iter()
            .map(|c| (c.name.as_str(), c.program.as_str()))
            .collect();
        assert_eq!(
            names,
            [("build", "make"), ("test", "cargo"), ("lint", "cargo")]
        );
        assert!(settings.debugger.is_none(), "null resets to the default");
        assert_eq!(resolved.sources["show_sidebar"], Source::User);
        assert_eq!(resolved.sources["commands"], Source::Workspace);
        assert_eq!(resolved.sources["debugger"], Source::Default);
        assert!(!resolved.sources.contains_key("theme"));
    }

    #[test]
    fn workspace_paths_resolve_against_the_root() {
        let mut layer = json!({
            "commands": [{"name": "x", "program": "./run.sh", "working_directory": "sub"}],
            "debugger": {"program": "target/app", "working_directory": "/abs"},
        });
        resolve_paths(&mut layer, Path::new("/project"));
        assert_eq!(layer["commands"][0]["program"], "/project/./run.sh");
        assert_eq!(layer["commands"][0]["working_directory"], "/project/sub");
        assert_eq!(layer["debugger"]["program"], "target/app");
        assert_eq!(layer["debugger"]["working_directory"], "/abs");
    }

    #[test]
    fn layer_errors_name_the_file_and_location() {
        let path = Path::new("/p/.hephaestus/settings.json");
        let error = parse_layer(path, "{\n  \"show_sidebar\": true,\n  oops\n}").unwrap_err();
        assert!(
            error.starts_with("/p/.hephaestus/settings.json:3:"),
            "{error}"
        );
        let error = parse_layer(path, "{\"unknown_key\": 1}").unwrap_err();
        assert!(error.contains("unknown field"), "{error}");
        assert!(parse_layer(path, "[]").is_err());
        assert_eq!(parse_layer(path, "  ").unwrap(), json!({}));
        assert!(parse_layer(path, "{\"debugger\": null}").is_ok());
    }
}
