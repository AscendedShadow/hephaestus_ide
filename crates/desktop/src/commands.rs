use std::{collections::BTreeMap, rc::Rc};

use gpui::{
    Action, App, AsKeystroke as _, Global, KeyBinding, KeyBindingContextPredicate, Menu, MenuItem,
    NoAction, actions,
};
use gpui_component::kbd::Kbd;
use serde::{Deserialize, Serialize};

use crate::{
    git_panel,
    shell::EDITOR_CONTEXT,
    terminal_view::{self, KEY_CONTEXT as TERMINAL},
    vim,
};

actions!(
    hephaestus,
    [
        NewFile,
        OpenFile,
        OpenFolder,
        CloneRepository,
        SaveFile,
        SaveFileAs,
        CloseTab,
        NextTab,
        PreviousTab,
        CloseWindow,
        OpenSettings,
        EditSettings,
        ReloadSettings,
        ShowFolderPanel,
        ToggleTerminal,
        ShowGitPanel,
        ShowDebugPanel,
        ToggleVimMode,
        ToggleFold,
        QuickOpen,
        CommandPalette,
        SearchProject,
        FindInFile,
        RunCommand,
        StopCommand,
        StartDebugger,
        ReloadFile,
        ShowDiagnostics,
        GoToDefinition,
        HoverInfo,
        CompleteCode,
        ToggleBreakpoint,
        DebugContinue,
        DebugStepOver,
        DebugStepInto,
        DebugStepOut,
        DebugInterrupt,
        GoBack,
        GoForward,
        GoToLine
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
pub enum Command {
    NewFile,
    OpenFile,
    OpenFolder,
    CloneRepository,
    SaveFile,
    SaveFileAs,
    CloseTab,
    NextTab,
    PreviousTab,
    CloseWindow,
    OpenSettings,
    EditSettings,
    ReloadSettings,
    ShowFolderPanel,
    ShowGitPanel,
    ToggleTerminal,
    ShowDebugPanel,
    ToggleVimMode,
    ToggleFold,
    Commit,
    StageAll,
    Pull,
    Push,
    Fetch,
    TerminalCopy,
    TerminalPaste,
    QuickOpen,
    CommandPalette,
    SearchProject,
    FindInFile,
    RunCommand,
    StopCommand,
    StartDebugger,
    ReloadFile,
    ShowDiagnostics,
    GoToDefinition,
    HoverInfo,
    CompleteCode,
    ToggleBreakpoint,
    DebugContinue,
    DebugStepOver,
    DebugStepInto,
    DebugStepOut,
    DebugInterrupt,
    GoBack,
    GoForward,
    GoToLine,
}

impl Command {
    pub const ALL: [Self; 47] = [
        Self::NewFile,
        Self::OpenFile,
        Self::OpenFolder,
        Self::CloneRepository,
        Self::SaveFile,
        Self::SaveFileAs,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::CloseWindow,
        Self::OpenSettings,
        Self::EditSettings,
        Self::ReloadSettings,
        Self::ShowFolderPanel,
        Self::ShowGitPanel,
        Self::ToggleTerminal,
        Self::ShowDebugPanel,
        Self::ToggleVimMode,
        Self::ToggleFold,
        Self::Commit,
        Self::StageAll,
        Self::Pull,
        Self::Push,
        Self::Fetch,
        Self::TerminalCopy,
        Self::TerminalPaste,
        Self::QuickOpen,
        Self::CommandPalette,
        Self::SearchProject,
        Self::FindInFile,
        Self::RunCommand,
        Self::StopCommand,
        Self::StartDebugger,
        Self::ReloadFile,
        Self::ShowDiagnostics,
        Self::GoToDefinition,
        Self::HoverInfo,
        Self::CompleteCode,
        Self::ToggleBreakpoint,
        Self::DebugContinue,
        Self::DebugStepOver,
        Self::DebugStepInto,
        Self::DebugStepOut,
        Self::DebugInterrupt,
        Self::GoBack,
        Self::GoForward,
        Self::GoToLine,
    ];

    pub fn name(self) -> String {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::String(name)) => name,
            _ => format!("{self:?}"),
        }
    }

    pub(crate) fn action(self) -> Box<dyn Action> {
        match self {
            Self::NewFile => Box::new(NewFile),
            Self::OpenFile => Box::new(OpenFile),
            Self::OpenFolder => Box::new(OpenFolder),
            Self::CloneRepository => Box::new(CloneRepository),
            Self::SaveFile => Box::new(SaveFile),
            Self::SaveFileAs => Box::new(SaveFileAs),
            Self::CloseTab => Box::new(CloseTab),
            Self::NextTab => Box::new(NextTab),
            Self::PreviousTab => Box::new(PreviousTab),
            Self::CloseWindow => Box::new(CloseWindow),
            Self::OpenSettings => Box::new(OpenSettings),
            Self::EditSettings => Box::new(EditSettings),
            Self::ReloadSettings => Box::new(ReloadSettings),
            Self::ShowFolderPanel => Box::new(ShowFolderPanel),
            Self::ShowGitPanel => Box::new(ShowGitPanel),
            Self::ToggleTerminal => Box::new(ToggleTerminal),
            Self::ShowDebugPanel => Box::new(ShowDebugPanel),
            Self::ToggleVimMode => Box::new(ToggleVimMode),
            Self::ToggleFold => Box::new(ToggleFold),
            Self::Commit => Box::new(git_panel::Commit),
            Self::StageAll => Box::new(git_panel::StageAll),
            Self::Pull => Box::new(git_panel::Pull),
            Self::Push => Box::new(git_panel::Push),
            Self::Fetch => Box::new(git_panel::Fetch),
            Self::TerminalCopy => Box::new(terminal_view::Copy),
            Self::TerminalPaste => Box::new(terminal_view::Paste),
            Self::QuickOpen => Box::new(QuickOpen),
            Self::CommandPalette => Box::new(CommandPalette),
            Self::SearchProject => Box::new(SearchProject),
            Self::FindInFile => Box::new(FindInFile),
            Self::RunCommand => Box::new(RunCommand),
            Self::StopCommand => Box::new(StopCommand),
            Self::StartDebugger => Box::new(StartDebugger),
            Self::ReloadFile => Box::new(ReloadFile),
            Self::ShowDiagnostics => Box::new(ShowDiagnostics),
            Self::GoToDefinition => Box::new(GoToDefinition),
            Self::HoverInfo => Box::new(HoverInfo),
            Self::CompleteCode => Box::new(CompleteCode),
            Self::ToggleBreakpoint => Box::new(ToggleBreakpoint),
            Self::DebugContinue => Box::new(DebugContinue),
            Self::DebugStepOver => Box::new(DebugStepOver),
            Self::DebugStepInto => Box::new(DebugStepInto),
            Self::DebugStepOut => Box::new(DebugStepOut),
            Self::DebugInterrupt => Box::new(DebugInterrupt),
            Self::GoBack => Box::new(GoBack),
            Self::GoForward => Box::new(GoForward),
            Self::GoToLine => Box::new(GoToLine),
        }
    }

    fn context(self) -> Option<String> {
        match self {
            Self::ToggleFold => Some(EDITOR_CONTEXT.into()),
            Self::Commit => Some(format!("{} > Input", git_panel::COMMIT_CONTEXT)),
            Self::TerminalCopy | Self::TerminalPaste => Some(TERMINAL.into()),
            _ => None,
        }
    }

    pub fn default_keys(self) -> Vec<String> {
        let mac = cfg!(target_os = "macos");
        let keys: &[&str] = match self {
            Self::NewFile => &["secondary-n"],
            Self::OpenFile => &["secondary-o"],
            Self::OpenFolder => &["secondary-k secondary-o"],
            Self::SaveFile => &["secondary-s"],
            Self::SaveFileAs => &["secondary-shift-s"],
            Self::CloseTab => &["secondary-w"],
            Self::NextTab => &["ctrl-tab", "ctrl-pagedown"],
            Self::PreviousTab => &["ctrl-shift-tab", "ctrl-pageup"],
            Self::CloseWindow => &["secondary-shift-w"],
            Self::OpenSettings => &["secondary-,"],
            Self::CloneRepository
            | Self::EditSettings
            | Self::ReloadSettings
            | Self::ToggleVimMode => &[],
            Self::ShowFolderPanel => &["secondary-shift-e"],
            Self::ShowGitPanel => &["secondary-shift-g"],
            Self::ToggleTerminal => &["ctrl-`"],
            Self::ShowDebugPanel => &["secondary-shift-d"],
            Self::ToggleFold => &["secondary-shift-[", "secondary-k secondary-l"],
            Self::Commit => &["secondary-enter"],
            Self::StageAll => &["secondary-shift-a"],
            Self::Pull => &["secondary-shift-l"],
            Self::Push => &["secondary-shift-k"],
            Self::Fetch => &["secondary-shift-j"],
            Self::TerminalCopy => &[if mac { "cmd-c" } else { "ctrl-shift-c" }],
            Self::TerminalPaste => &[if mac { "cmd-v" } else { "ctrl-shift-v" }],
            Self::QuickOpen => &["secondary-p"],
            Self::CommandPalette => &["secondary-shift-p"],
            Self::SearchProject => &["secondary-shift-f"],
            Self::FindInFile => &[],
            Self::RunCommand => &["secondary-shift-b"],
            Self::StopCommand => &["secondary-shift-x"],
            Self::StartDebugger => &["f5"],
            Self::ReloadFile => &[],
            Self::ShowDiagnostics => &["secondary-shift-m"],
            Self::GoToDefinition => &["f12"],
            Self::HoverInfo => &["secondary-k secondary-i"],
            Self::CompleteCode => &["ctrl-space"],
            Self::ToggleBreakpoint => &["f9"],
            Self::DebugContinue => &["f6"],
            Self::DebugStepOver => &["f10"],
            Self::DebugStepInto => &["f11"],
            Self::DebugStepOut => &["shift-f11"],
            Self::DebugInterrupt => &["ctrl-f6"],
            Self::GoBack => &["alt-left"],
            Self::GoForward => &["alt-right"],
            Self::GoToLine => &["secondary-g"],
        };
        let primary = if mac { "cmd" } else { "ctrl" };
        keys.iter()
            .map(|keys| keys.replace("secondary", primary))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Keys {
    One(String),
    Many(Vec<String>),
}

impl Keys {
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        match self {
            Self::One(keys) => std::slice::from_ref(keys),
            Self::Many(keys) => keys.as_slice(),
        }
        .iter()
        .map(String::as_str)
    }
}

impl From<Vec<String>> for Keys {
    fn from(mut keys: Vec<String>) -> Self {
        if keys.len() == 1 {
            Self::One(keys.remove(0))
        } else {
            Self::Many(keys)
        }
    }
}

pub type Keybindings = BTreeMap<Command, Keys>;

pub fn shortcut(action: &dyn Action, cx: &App) -> Option<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let binding = keymap.bindings_for_action(action).last()?;
    let keys: Vec<_> = binding
        .keystrokes()
        .iter()
        .map(|key| Kbd::format(key.as_keystroke()))
        .collect();
    Some(keys.join(" "))
}

pub fn default_keybindings() -> Keybindings {
    Command::ALL
        .into_iter()
        .map(|command| (command, command.default_keys().into()))
        .collect()
}

struct BaseKeymap(Vec<KeyBinding>);

impl Global for BaseKeymap {}

#[cfg(test)]
pub fn init(cx: &mut App) {
    rebind(cx, &Keybindings::new());
}

pub fn rebind(cx: &mut App, keybindings: &Keybindings) -> Vec<String> {
    if !cx.has_global::<BaseKeymap>() {
        let base = cx.key_bindings().borrow().bindings().cloned().collect();
        cx.set_global(BaseKeymap(base));
    }
    let base = cx.global::<BaseKeymap>().0.clone();
    cx.clear_key_bindings();
    cx.bind_keys(base);

    let keyboard_mapper = cx.keyboard_mapper().clone();
    let mut problems = Vec::new();
    let mut bindings = Vec::new();
    for command in Command::ALL {
        let context = command
            .context()
            .map(|context| Rc::new(KeyBindingContextPredicate::parse(&context).unwrap()));
        let keys = match keybindings.get(&command) {
            Some(keys) => keys.iter().map(String::from).collect(),
            None => command.default_keys(),
        };
        for keys in keys.iter().filter(|keys| !keys.trim().is_empty()) {
            match KeyBinding::load(
                keys,
                command.action(),
                context.clone(),
                false,
                None,
                keyboard_mapper.as_ref(),
            ) {
                Ok(binding) => bindings.push(binding),
                Err(error) => problems.push(format!("{}: {error}", command.name())),
            }
        }
    }
    cx.bind_keys(bindings);

    cx.bind_keys([
        KeyBinding::new("tab", NoAction, Some(TERMINAL)),
        KeyBinding::new("shift-tab", NoAction, Some(TERMINAL)),
    ]);
    if !cfg!(target_os = "macos") {
        cx.bind_keys(
            ["ctrl-n", "ctrl-o", "ctrl-k", "ctrl-k ctrl-o", "ctrl-w"]
                .map(|keys| KeyBinding::new(keys, NoAction, Some(TERMINAL))),
        );
    }
    let vim_normal = format!("{} > Input && !SearchPanel", vim::NORMAL_CONTEXT);
    let vim_insert = format!("{} > Input && !SearchPanel", vim::INSERT_CONTEXT);
    cx.bind_keys(
        [
            "escape",
            "ctrl-[",
            "enter",
            "backspace",
            "delete",
            "tab",
            "shift-tab",
            "up",
            "down",
            "left",
            "right",
            "home",
            "end",
        ]
        .map(|keys| KeyBinding::new(keys, NoAction, Some(&vim_normal))),
    );
    cx.bind_keys(
        ["escape", "ctrl-["].map(|keys| KeyBinding::new(keys, NoAction, Some(&vim_insert))),
    );

    cx.set_menus(menus());
    problems
}

fn menus() -> Vec<Menu> {
    let file_menu = Menu {
        name: "File".into(),
        items: vec![
            MenuItem::action("New", NewFile),
            MenuItem::action("Open…", OpenFile),
            MenuItem::action("Open Folder…", OpenFolder),
            MenuItem::action("Clone Repository…", CloneRepository),
            MenuItem::separator(),
            MenuItem::action("Save", SaveFile),
            MenuItem::action("Save As…", SaveFileAs),
            MenuItem::separator(),
            MenuItem::action("Close Tab", CloseTab),
        ],
    };
    let view_menu = Menu {
        name: "View".into(),
        items: vec![MenuItem::action("Toggle Fold", ToggleFold)],
    };
    let settings_items = |open: &'static str| {
        vec![
            MenuItem::action(open, OpenSettings),
            MenuItem::action("Edit settings.json", EditSettings),
            MenuItem::action("Reload Settings", ReloadSettings),
            MenuItem::separator(),
            MenuItem::action("Toggle Vim Keys", ToggleVimMode),
        ]
    };
    if cfg!(target_os = "macos") {
        vec![
            Menu {
                name: "Hephaestus".into(),
                items: settings_items("Settings…"),
            },
            file_menu,
            view_menu,
        ]
    } else {
        vec![
            file_menu,
            view_menu,
            Menu {
                name: "Settings".into(),
                items: settings_items("Open Settings…"),
            },
        ]
    }
}
