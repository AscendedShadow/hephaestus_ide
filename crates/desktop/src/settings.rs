use std::{
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
}

impl Settings {
    pub fn defaults() -> Self {
        Self {
            keybindings: commands::default_keybindings(),
            theme: Themes::defaults(),
        }
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
        theme::set_themes(self.theme.clone());
        theme::set_mode(theme::mode(), window, cx);
        commands::rebind(cx, &self.keybindings)
    }
}

pub struct SettingsPath(pub PathBuf);

impl Global for SettingsPath {}

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
