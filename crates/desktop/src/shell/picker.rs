//! Generic item picker used by references, symbols and code actions.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PickerKind {
    Locations,
    DocumentSymbols,
    WorkspaceSymbols,
    CodeActions,
}

#[derive(Clone, Debug)]
pub(super) enum PickerAction {
    Open(SearchMatch),
    CodeAction(usize),
    WorkspaceSymbol(usize),
    TerminalProfile(Option<String>),
    ReopenWithEncoding(ide_core::document::Encoding),
    SaveWithEncoding(ide_core::document::Encoding),
    SetLineEnding(ide_core::document::LineEnding),
    OpenWithEncoding(PathBuf, ide_core::document::Encoding),
    RunTask(String),
    AttachProcess(u32),
    DebugProfile(String),
    Git(super::git_features::GitAction),
    Nothing,
}

#[derive(Clone, Debug)]
pub(super) struct PickerItem {
    pub label: String,
    pub detail: String,
    /// Text matched against the query.
    pub filter: String,
    pub depth: usize,
    pub action: PickerAction,
}

pub(super) struct Picker {
    pub kind: PickerKind,
    pub title: String,
    pub items: Vec<PickerItem>,
}

impl IdeShell {
    pub(super) fn show_picker(
        &mut self,
        kind: PickerKind,
        title: String,
        items: Vec<PickerItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker = Some(Picker { kind, title, items });
        self.show_search(SearchMode::Items, window, cx);
    }

    /// Item indices matching `query`, best first (original order when unfiltered).
    pub(super) fn ranked_items(&self, query: &str) -> Vec<usize> {
        let Some(picker) = &self.picker else {
            return Vec::new();
        };
        if query.is_empty() || picker.kind == PickerKind::WorkspaceSymbols {
            return (0..picker.items.len()).collect();
        }
        let mut ranked: Vec<_> = picker
            .items
            .iter()
            .enumerate()
            .filter_map(|(ix, item)| {
                search::rank(Path::new(&item.filter), query).map(|score| (score, ix))
            })
            .collect();
        ranked.sort();
        ranked.into_iter().map(|(_, ix)| ix).collect()
    }

    pub(super) fn picker_labels(&self, query: &str) -> Vec<String> {
        let Some(picker) = &self.picker else {
            return Vec::new();
        };
        let mut labels: Vec<String> = self
            .ranked_items(query)
            .into_iter()
            .map(|ix| {
                let item = &picker.items[ix];
                let indent = "  ".repeat(item.depth);
                if item.detail.is_empty() {
                    format!("{indent}{}", item.label)
                } else {
                    format!("{indent}{}  —  {}", item.label, item.detail)
                }
            })
            .collect();
        if labels.is_empty() {
            labels.push(format!("{} — no matches", picker.title));
        }
        labels
    }

    pub(super) fn choose_picker_item(
        &mut self,
        ix: usize,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item_ix) = self.ranked_items(query).get(ix).copied() else {
            return;
        };
        let Some(action) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.items.get(item_ix))
            .map(|item| item.action.clone())
        else {
            return;
        };
        self.focus_editor(window, cx);
        match action {
            PickerAction::Open(location) => {
                self.record_location(&location, cx);
                self.open_at(location, window, cx);
            }
            PickerAction::CodeAction(index) => self.run_code_action(index, window, cx),
            PickerAction::WorkspaceSymbol(index) => self.open_workspace_symbol(index, window, cx),
            PickerAction::TerminalProfile(name) => self.start_profile(name, window, cx),
            PickerAction::ReopenWithEncoding(encoding) => {
                self.apply_reopen_encoding(encoding, window, cx)
            }
            PickerAction::SaveWithEncoding(encoding) => {
                self.apply_save_encoding(encoding, window, cx)
            }
            PickerAction::SetLineEnding(ending) => self.set_line_ending(ending, cx),
            PickerAction::OpenWithEncoding(path, encoding) => {
                self.open_with_encoding(path, encoding, window, cx)
            }
            PickerAction::RunTask(id) => self.run_task_then(id, None, cx),
            PickerAction::AttachProcess(pid) => self.attach_to(pid, window, cx),
            PickerAction::DebugProfile(name) => self.choose_debug_profile(name, cx),
            PickerAction::Git(action) => self.run_git_action(action, window, cx),
            PickerAction::Nothing => {}
        }
    }
}
