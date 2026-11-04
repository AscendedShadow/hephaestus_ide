//! Planning LSP workspace edits (IDE-005).
//!
//! A [`lsp_types::WorkspaceEdit`] is converted into an ordered [`Plan`] of steps that has
//! been fully validated before anything is mutated:
//!
//! - text edits are resolved against the *current* text of the target (the open buffer's
//!   unsaved text, or the file on disk), converted from UTF-16 positions to byte ranges,
//!   and checked for bounds and overlaps;
//! - versioned edits (`documentChanges`) must match the version the language server last
//!   saw for that open document;
//! - create/rename/delete operations are simulated in order against a virtual view of the
//!   filesystem, so later edits can target a created or renamed file and destination
//!   conflicts are reported before any change is made.

use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    path::{Path, PathBuf},
};

use ide_core::text_edit;
use lsp_types::{
    DocumentChangeOperation, DocumentChanges, OneOf, ResourceOp, TextEdit, WorkspaceEdit,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Byte-range edits against the target's current text.
    Edit {
        path: PathBuf,
        edits: Vec<(Range<usize>, String)>,
        /// The target is open in an editor (otherwise it is read from disk).
        open: bool,
    },
    Create {
        path: PathBuf,
        overwrite: bool,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        overwrite: bool,
    },
    Delete {
        path: PathBuf,
        recursive: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Files touched by the plan (for previews), with the number of text edits each.
    pub fn summary(&self) -> Vec<(PathBuf, String)> {
        let mut edits: BTreeMap<PathBuf, usize> = BTreeMap::new();
        let mut lines = Vec::new();
        for step in &self.steps {
            match step {
                Step::Edit {
                    path, edits: list, ..
                } => {
                    *edits.entry(path.clone()).or_default() += list.len();
                }
                Step::Create { path, .. } => lines.push((path.clone(), "create".to_string())),
                Step::Rename { from, to, .. } => lines.push((
                    from.clone(),
                    format!(
                        "rename to {}",
                        to.file_name().unwrap_or_default().to_string_lossy()
                    ),
                )),
                Step::Delete { path, .. } => lines.push((path.clone(), "delete".to_string())),
            }
        }
        for (path, count) in edits {
            lines.push((
                path,
                format!("{count} edit{}", if count == 1 { "" } else { "s" }),
            ));
        }
        lines
    }

    pub fn files(&self) -> usize {
        let mut paths: Vec<&Path> = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Edit { path, .. } | Step::Create { path, .. } | Step::Delete { path, .. } => {
                    path.as_path()
                }
                Step::Rename { from, .. } => from.as_path(),
            })
            .collect();
        paths.sort();
        paths.dedup();
        paths.len()
    }

    pub fn has_resource_operations(&self) -> bool {
        self.steps
            .iter()
            .any(|step| !matches!(step, Step::Edit { .. }))
    }

    pub fn edit_count(&self) -> usize {
        self.steps
            .iter()
            .map(|step| match step {
                Step::Edit { edits, .. } => edits.len(),
                _ => 1,
            })
            .sum()
    }
}

/// What the planner needs to know about the current workspace state.
pub trait Snapshot {
    /// Current text and LSP version of an open document.
    fn open_document(&self, path: &Path) -> Option<(String, Option<i32>)>;
    /// Current text of a closed file on disk.
    fn read_file(&self, path: &Path) -> Option<String>;
    fn exists(&self, path: &Path) -> bool;
}

#[derive(Clone, Debug)]
enum Virtual {
    Created,
    /// Renamed from another path: text comes from the source.
    RenamedFrom(PathBuf),
    Absent,
}

struct Planner<'a> {
    snapshot: &'a dyn Snapshot,
    state: HashMap<PathBuf, Virtual>,
    /// Text edited so far in this plan, per (virtual) path.
    texts: HashMap<PathBuf, String>,
}

impl Planner<'_> {
    fn exists(&self, path: &Path) -> bool {
        match self.state.get(path) {
            Some(Virtual::Absent) => false,
            Some(_) => true,
            None => self.snapshot.exists(path) || self.snapshot.open_document(path).is_some(),
        }
    }

    fn text(&self, path: &Path) -> Result<(String, bool), String> {
        if let Some(text) = self.texts.get(path) {
            return Ok((text.clone(), self.snapshot.open_document(path).is_some()));
        }
        match self.state.get(path) {
            Some(Virtual::Absent) => Err(format!("{} does not exist", path.display())),
            Some(Virtual::Created) => Ok((String::new(), false)),
            Some(Virtual::RenamedFrom(source)) => {
                let source = source.clone();
                self.text(&source)
            }
            None => {
                if let Some((text, _)) = self.snapshot.open_document(path) {
                    return Ok((text, true));
                }
                self.snapshot
                    .read_file(path)
                    .map(|text| (text, false))
                    .ok_or_else(|| format!("cannot read {}", path.display()))
            }
        }
    }

    fn edit(
        &mut self,
        path: PathBuf,
        version: Option<i32>,
        edits: &[TextEdit],
        steps: &mut Vec<Step>,
    ) -> Result<(), String> {
        if let Some(expected) = version {
            match self.snapshot.open_document(&path) {
                Some((_, Some(current))) if current == expected => {}
                Some((_, current)) => {
                    return Err(format!(
                        "{} changed since the server computed the edit (version {expected}, now {})",
                        path.display(),
                        current.map_or("unknown".into(), |current| current.to_string())
                    ));
                }
                None => {
                    return Err(format!(
                        "{} is not open, but the edit expects version {expected}",
                        path.display()
                    ));
                }
            }
        }
        let (text, open) = self.text(&path)?;
        let mut converted = Vec::with_capacity(edits.len());
        for edit in edits {
            let start = text_edit::offset(&text, edit.range.start.line, edit.range.start.character)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let end = text_edit::offset(&text, edit.range.end.line, edit.range.end.character)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if end < start {
                return Err(format!(
                    "{}: edit range ends before it starts",
                    path.display()
                ));
            }
            // Servers may send CRLF; buffers are LF-normalized.
            converted.push((start..end, edit.new_text.replace("\r\n", "\n")));
        }
        let updated = text_edit::apply(&text, &converted)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        self.texts.insert(path.clone(), updated);
        steps.push(Step::Edit {
            path,
            edits: converted,
            open,
        });
        Ok(())
    }
}

fn path(uri: &lsp_types::Uri) -> Result<PathBuf, String> {
    lsp::uri_to_path(uri).ok_or_else(|| format!("unsupported URI {}", uri.as_str()))
}

/// Validate and plan a workspace edit against the current workspace state.
pub fn plan(edit: &WorkspaceEdit, snapshot: &dyn Snapshot) -> Result<Plan, String> {
    let mut planner = Planner {
        snapshot,
        state: HashMap::new(),
        texts: HashMap::new(),
    };
    let mut steps = Vec::new();
    if let Some(changes) = &edit.document_changes {
        let operations: Vec<DocumentChangeOperation> = match changes {
            DocumentChanges::Edits(edits) => edits
                .iter()
                .cloned()
                .map(DocumentChangeOperation::Edit)
                .collect(),
            DocumentChanges::Operations(operations) => operations.clone(),
        };
        for operation in operations {
            match operation {
                DocumentChangeOperation::Edit(document) => {
                    let edits: Vec<TextEdit> = document
                        .edits
                        .iter()
                        .map(|edit| match edit {
                            OneOf::Left(edit) => edit.clone(),
                            OneOf::Right(annotated) => annotated.text_edit.clone(),
                        })
                        .collect();
                    planner.edit(
                        path(&document.text_document.uri)?,
                        document.text_document.version,
                        &edits,
                        &mut steps,
                    )?;
                }
                DocumentChangeOperation::Op(ResourceOp::Create(create)) => {
                    let target = path(&create.uri)?;
                    let overwrite = create
                        .options
                        .as_ref()
                        .and_then(|o| o.overwrite)
                        .unwrap_or(false);
                    let ignore = create.options.as_ref().and_then(|o| o.ignore_if_exists);
                    if planner.exists(&target) && !overwrite {
                        if ignore.unwrap_or(false) {
                            continue;
                        }
                        return Err(format!("{} already exists", target.display()));
                    }
                    planner.texts.remove(&target);
                    planner.state.insert(target.clone(), Virtual::Created);
                    steps.push(Step::Create {
                        path: target,
                        overwrite,
                    });
                }
                DocumentChangeOperation::Op(ResourceOp::Rename(rename)) => {
                    let from = path(&rename.old_uri)?;
                    let to = path(&rename.new_uri)?;
                    let overwrite = rename
                        .options
                        .as_ref()
                        .and_then(|o| o.overwrite)
                        .unwrap_or(false);
                    let ignore = rename.options.as_ref().and_then(|o| o.ignore_if_exists);
                    if !planner.exists(&from) {
                        return Err(format!("cannot rename missing {}", from.display()));
                    }
                    if planner.exists(&to) && !overwrite {
                        if ignore.unwrap_or(false) {
                            continue;
                        }
                        return Err(format!("{} already exists", to.display()));
                    }
                    if let Some(text) = planner.texts.remove(&from) {
                        planner.texts.insert(to.clone(), text);
                    }
                    let origin = match planner.state.get(&from) {
                        Some(Virtual::RenamedFrom(origin)) => origin.clone(),
                        _ => from.clone(),
                    };
                    planner
                        .state
                        .insert(to.clone(), Virtual::RenamedFrom(origin));
                    planner.state.insert(from.clone(), Virtual::Absent);
                    steps.push(Step::Rename {
                        from,
                        to,
                        overwrite,
                    });
                }
                DocumentChangeOperation::Op(ResourceOp::Delete(delete)) => {
                    let target = path(&delete.uri)?;
                    let options = delete.options.as_ref();
                    if !planner.exists(&target) {
                        if options
                            .and_then(|o| o.ignore_if_not_exists)
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        return Err(format!("cannot delete missing {}", target.display()));
                    }
                    planner.texts.remove(&target);
                    planner.state.insert(target.clone(), Virtual::Absent);
                    steps.push(Step::Delete {
                        path: target,
                        recursive: options.and_then(|o| o.recursive).unwrap_or(false),
                    });
                }
            }
        }
    } else if let Some(changes) = &edit.changes {
        let mut ordered: Vec<_> = changes.iter().collect();
        ordered.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (uri, edits) in ordered {
            planner.edit(path(uri)?, None, edits, &mut steps)?;
        }
    }
    Ok(Plan { steps })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use lsp_types::{
        CreateFile, OptionalVersionedTextDocumentIdentifier, Position, RenameFile, TextDocumentEdit,
    };
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Fake {
        open: HashMap<PathBuf, (String, Option<i32>)>,
        disk: RefCell<HashMap<PathBuf, String>>,
    }

    impl Snapshot for Fake {
        fn open_document(&self, path: &Path) -> Option<(String, Option<i32>)> {
            self.open.get(path).cloned()
        }
        fn read_file(&self, path: &Path) -> Option<String> {
            self.disk.borrow().get(path).cloned()
        }
        fn exists(&self, path: &Path) -> bool {
            self.disk.borrow().contains_key(path)
        }
    }

    fn uri(path: &str) -> lsp_types::Uri {
        lsp::path_to_uri(Path::new(path)).unwrap()
    }

    fn text_edit(line: u32, start: u32, end: u32, text: &str) -> TextEdit {
        TextEdit {
            range: lsp_types::Range::new(Position::new(line, start), Position::new(line, end)),
            new_text: text.into(),
        }
    }

    fn fake() -> Fake {
        let mut fake = Fake::default();
        fake.open.insert(
            PathBuf::from("/p/a.rs"),
            ("let 😀 = old; old()\n".into(), Some(3)),
        );
        fake.disk
            .borrow_mut()
            .insert(PathBuf::from("/p/b.rs"), "use old;\n".into());
        fake.disk
            .borrow_mut()
            .insert(PathBuf::from("/p/a.rs"), "stale disk text".into());
        fake
    }

    #[test]
    fn changes_use_open_buffer_text_and_utf16_columns() {
        let fake = fake();
        let edit = WorkspaceEdit {
            changes: Some(
                [
                    (
                        uri("/p/a.rs"),
                        vec![text_edit(0, 14, 17, "new"), text_edit(0, 9, 12, "new")],
                    ),
                    (uri("/p/b.rs"), vec![text_edit(0, 4, 7, "new")]),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };
        let plan = plan(&edit, &fake).unwrap();
        assert_eq!(
            plan.steps,
            vec![
                Step::Edit {
                    path: "/p/a.rs".into(),
                    edits: vec![(16..19, "new".into()), (11..14, "new".into())],
                    open: true
                },
                Step::Edit {
                    path: "/p/b.rs".into(),
                    edits: vec![(4..7, "new".into())],
                    open: false
                },
            ]
        );
        assert_eq!(plan.files(), 2);
        assert_eq!(plan.edit_count(), 3);
    }

    #[test]
    fn versions_overlaps_and_resource_operations_are_validated() {
        let fake = fake();
        let versioned = |version| WorkspaceEdit {
            document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: uri("/p/a.rs"),
                    version: Some(version),
                },
                edits: vec![OneOf::Left(text_edit(0, 9, 12, "x"))],
            }])),
            ..Default::default()
        };
        assert!(plan(&versioned(3), &fake).is_ok());
        let stale = plan(&versioned(2), &fake).unwrap_err();
        assert!(stale.contains("changed since"), "{stale}");

        let overlapping = WorkspaceEdit {
            changes: Some(
                [(
                    uri("/p/b.rs"),
                    vec![text_edit(0, 0, 5, "a"), text_edit(0, 3, 6, "b")],
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };
        assert!(plan(&overlapping, &fake).unwrap_err().contains("overlap"));

        let operations = |options: Option<lsp_types::RenameFileOptions>| WorkspaceEdit {
            document_changes: Some(DocumentChanges::Operations(vec![
                DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                    uri: uri("/p/new.rs"),
                    options: None,
                    annotation_id: None,
                })),
                DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier {
                        uri: uri("/p/new.rs"),
                        version: None,
                    },
                    edits: vec![OneOf::Left(text_edit(0, 0, 0, "fn main() {}"))],
                }),
                DocumentChangeOperation::Op(ResourceOp::Rename(RenameFile {
                    old_uri: uri("/p/b.rs"),
                    new_uri: uri("/p/a.rs"),
                    options,
                    annotation_id: None,
                })),
            ])),
            ..Default::default()
        };
        let conflict = plan(&operations(None), &fake).unwrap_err();
        assert!(conflict.contains("already exists"), "{conflict}");
        let plan = plan(
            &operations(Some(lsp_types::RenameFileOptions {
                overwrite: Some(true),
                ignore_if_exists: None,
            })),
            &fake,
        )
        .unwrap();
        assert!(plan.has_resource_operations());
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(
            plan.steps[1],
            Step::Edit {
                path: "/p/new.rs".into(),
                edits: vec![(0..0, "fn main() {}".into())],
                open: false
            }
        );
        let parsed: WorkspaceEdit = serde_json::from_value(json!({
            "documentChanges": [{"kind": "delete", "uri": "file:///p/missing.rs"}]
        }))
        .unwrap();
        assert!(super::plan(&parsed, &fake).unwrap_err().contains("missing"));
    }
}
