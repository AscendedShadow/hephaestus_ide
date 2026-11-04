use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env, fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use gpui::{App, Global, Window};
use serde::{Deserialize, Serialize};

use crate::{
    commands::{self, Keybindings},
    theme::{self, Themes},
};

pub mod layers;

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;

pub const FILE_NAME: &str = "settings.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub keybindings: Keybindings,
    pub theme: Themes,
    pub show_sidebar: bool,
    pub commands: Vec<RunConfig>,
    pub debugger: Option<DebuggerConfig>,
    pub language_servers: Vec<LanguageServer>,
    /// File associations: glob pattern (matched against the file name, or the
    /// workspace-relative path when it contains `/`) to language mode id or name.
    pub file_associations: BTreeMap<String, String>,
    /// Editor preferences (see `preferences`), with per-language overrides.
    pub editor: crate::preferences::EditorSettings,
    /// Auto-save policy: `{"mode": "off" | "after_delay" | "on_focus_change", "delay_ms": 1000}`.
    pub auto_save: crate::preferences::AutoSave,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunConfig {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DebuggerConfig {
    pub name: String,
    pub adapter: String,
    pub adapter_args: Vec<String>,
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub stop_on_entry: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LanguageServer {
    pub languages: Vec<String>,
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub initialization_options: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
    pub enabled: bool,
}

impl Default for LanguageServer {
    fn default() -> Self {
        Self {
            languages: Vec::new(),
            program: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            initialization_options: None,
            settings: None,
            enabled: true,
        }
    }
}

impl LanguageServer {
    fn new(languages: &[&str], program: &str, args: &[&str]) -> Self {
        Self {
            languages: languages
                .iter()
                .map(|language| language.to_string())
                .collect(),
            program: program.into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            ..Self::default()
        }
    }

    pub fn serves(&self, language: &str) -> bool {
        self.enabled && self.languages.iter().any(|served| served == language)
    }
}

pub fn default_language_servers() -> Vec<LanguageServer> {
    vec![
        LanguageServer::new(&["rust"], "rust-analyzer", &[]),
        LanguageServer::new(&["c", "cpp"], "clangd", &[]),
        LanguageServer::new(&["go"], "gopls", &[]),
        LanguageServer::new(&["zig"], "zls", &[]),
        LanguageServer::new(&["java"], "jdtls", &[]),
        LanguageServer::new(&["c_sharp"], "omnisharp", &["-lsp"]),
        LanguageServer::new(
            &["javascript", "jsx", "typescript", "tsx"],
            "typescript-language-server",
            &["--stdio"],
        ),
    ]
}

impl Settings {
    pub fn defaults() -> Self {
        Self {
            keybindings: commands::default_keybindings(),
            theme: Themes::defaults(),
            show_sidebar: false,
            commands: Vec::new(),
            debugger: None,
            language_servers: default_language_servers(),
            file_associations: BTreeMap::new(),
            editor: Default::default(),
            auto_save: Default::default(),
        }
    }

    pub fn servers(&self) -> Vec<LanguageServer> {
        let configured: HashSet<&str> = self
            .language_servers
            .iter()
            .flat_map(|server| server.languages.iter().map(String::as_str))
            .collect();
        let mut servers: Vec<LanguageServer> = self
            .language_servers
            .iter()
            .filter(|server| server.enabled && !server.program.is_empty())
            .cloned()
            .collect();
        servers.extend(default_language_servers().into_iter().filter(|server| {
            server
                .languages
                .iter()
                .all(|language| !configured.contains(language.as_str()))
        }));
        servers
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn parse(source: &str) -> Result<Self, String> {
        serde_json::from_str(source).map_err(|error| format!("{FILE_NAME}: {error}"))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(source) => Self::parse(&source),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub fn to_json(&self) -> String {
        let mut json = serde_json::to_string_pretty(self).expect("settings serialize to JSON");
        json.push('\n');
        json
    }

    pub fn apply(&self, window: Option<&mut Window>, cx: &mut App) -> Vec<String> {
        cx.set_global(ActiveLanguageServers(self.servers()));
        let mut problems = crate::syntax::set_associations(
            self.file_associations
                .iter()
                .map(|(pattern, language)| (pattern.as_str(), language.as_str())),
        );
        problems.extend(self.editor.problems());
        theme::set_themes(self.theme.clone());
        theme::set_mode(theme::mode(), window, cx);
        problems.extend(commands::rebind(cx, &self.keybindings));
        problems
    }
}

pub struct SettingsPath(pub PathBuf);

impl Global for SettingsPath {}

pub struct ActiveLanguageServers(pub Vec<LanguageServer>);

impl Global for ActiveLanguageServers {}

pub fn language_servers(cx: &App) -> Vec<LanguageServer> {
    cx.try_global::<ActiveLanguageServers>()
        .map(|servers| servers.0.clone())
        .unwrap_or_else(default_language_servers)
}

pub fn default_path() -> Option<PathBuf> {
    let var = |name| env::var_os(name).filter(|value| !value.is_empty());
    if cfg!(target_os = "windows") {
        var("APPDATA").map(|base| PathBuf::from(base).join("Hephaestus"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| PathBuf::from(home).join("Library/Application Support/Hephaestus"))
    } else {
        var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|base| base.join("hephaestus"))
    }
    .map(|directory| directory.join(FILE_NAME))
}

pub fn path(cx: &App) -> Option<PathBuf> {
    cx.try_global::<SettingsPath>()
        .map(|settings| settings.0.clone())
}

pub fn is_settings_file(file: &Path, cx: &App) -> bool {
    path(cx)
        .into_iter()
        .chain(workspace_path(cx))
        .any(|settings| {
            settings == file
                || settings
                    .canonicalize()
                    .is_ok_and(|settings| settings == file)
        })
}

/// The layered settings currently in effect (see [`layers`]).
#[derive(Default)]
pub struct SettingsState {
    pub workspace_root: Option<PathBuf>,
    user_good: serde_json::Value,
    workspace_good: Option<(PathBuf, serde_json::Value)>,
    pub resolved: layers::Resolved,
}

impl Global for SettingsState {}

/// The effective settings (defaults, user and workspace layers merged).
pub fn current(cx: &App) -> Settings {
    cx.try_global::<SettingsState>()
        .map(|state| state.resolved.settings.clone())
        .unwrap_or_default()
}

pub fn state(cx: &App) -> Option<&SettingsState> {
    cx.try_global::<SettingsState>()
}

/// The workspace settings file of the current workspace, if a folder is open.
pub fn workspace_path(cx: &App) -> Option<PathBuf> {
    state(cx)?
        .workspace_root
        .as_deref()
        .map(layers::workspace_path)
}

/// Re-read the user and workspace layers, then apply the effective settings. Invalid
/// layers are reported and their last-known-good contents stay in effect.
pub fn reload(
    workspace_root: Option<&Path>,
    window: Option<&mut Window>,
    cx: &mut App,
) -> Vec<String> {
    let mut state = cx
        .try_global::<SettingsState>()
        .map_or_else(SettingsState::default, |state| SettingsState {
            workspace_root: state.workspace_root.clone(),
            user_good: state.user_good.clone(),
            workspace_good: state.workspace_good.clone(),
            resolved: state.resolved.clone(),
        });
    let mut problems = Vec::new();
    if state.user_good.is_null() {
        state.user_good = serde_json::json!({});
    }
    if let Some(path) = path(cx) {
        match layers::read_layer(&path) {
            Ok(layer) => state.user_good = layer,
            Err(error) => problems.push(error),
        }
    }
    let root = workspace_root.map(Path::to_path_buf);
    if state.workspace_root != root {
        state.workspace_good = None;
    }
    state.workspace_root = root.clone();
    if let Some(root) = &root {
        let file = layers::workspace_path(root);
        match layers::read_layer(&file) {
            Ok(mut layer) => {
                layers::resolve_paths(&mut layer, root);
                state.workspace_good = Some((root.clone(), layer));
            }
            Err(error) => {
                problems.push(format!("{error} (using the last valid workspace settings)"))
            }
        }
    }
    let workspace = state.workspace_good.as_ref().map(|(_, layer)| layer);
    match layers::resolve(&state.user_good, workspace) {
        Ok(resolved) => state.resolved = resolved,
        Err(error) => problems.push(error),
    }
    let settings = state.resolved.settings.clone();
    cx.set_global(state);
    problems.extend(settings.apply(window, cx));
    problems
}

pub fn init(cx: &mut App) -> Vec<String> {
    if let Some(path) = default_path() {
        cx.set_global(SettingsPath(path));
    }
    reload(None, None, cx)
}

pub fn create_if_missing(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(Settings::defaults().to_json().as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn summary(problems: &[String]) -> Option<String> {
    let first = problems.first()?;
    Some(match problems.len() {
        1 => format!("Settings error — {first}"),
        count => format!("Settings error — {first} (and {} more)", count - 1),
    })
}
