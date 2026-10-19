use std::{
    collections::{HashMap, HashSet},
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
        LanguageServer::new(&["c"], "clangd", &[]),
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

    pub fn parse(source: &str) -> Result<Self, String> {
        serde_json::from_str(source).map_err(|error| format!("{FILE_NAME}: {error}"))
    }

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
        theme::set_themes(self.theme.clone());
        theme::set_mode(theme::mode(), window, cx);
        commands::rebind(cx, &self.keybindings)
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
    path(cx).is_some_and(|settings| {
        settings == file
            || settings
                .canonicalize()
                .is_ok_and(|settings| settings == file)
    })
}

pub fn init(cx: &mut App) -> Vec<String> {
    let Some(path) = default_path() else {
        return Settings::default().apply(None, cx);
    };
    let loaded = Settings::load(&path);
    cx.set_global(SettingsPath(path));
    match loaded {
        Ok(settings) => settings.apply(None, cx),
        Err(error) => {
            let mut problems = vec![error];
            problems.extend(Settings::default().apply(None, cx));
            problems
        }
    }
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
