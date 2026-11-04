use std::{
    cmp::Ordering,
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub entry: DirEntry,
    pub depth: usize,
    pub expanded: bool,
}

/// File extension of saved multi-root workspace definitions.
pub const DEFINITION_EXTENSION: &str = "hephaestus-workspace";

/// One workspace folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub path: PathBuf,
    /// Included in project search and Quick Open.
    pub search: bool,
    /// Why the folder could not be read (it stays in the workspace so it can be
    /// retried or removed).
    pub error: Option<String>,
}

/// A workspace: one or more root folders (IDE-017), optionally saved to a definition
/// file. The first root is the primary one.
#[derive(Debug, Default)]
pub struct Workspace {
    roots: Vec<Root>,
    listings: HashMap<PathBuf, Vec<DirEntry>>,
    name: Option<String>,
    file: Option<PathBuf>,
}

impl Workspace {
    pub fn open(path: &Path) -> io::Result<Self> {
        let root = path.canonicalize()?;
        if !fs::metadata(&root)?.is_dir() {
            return Err(io::Error::other("Choose a folder"));
        }
        let listing = read_dir(&root)?;
        Ok(Self {
            listings: HashMap::from([(root.clone(), listing)]),
            roots: vec![Root {
                path: root,
                search: true,
                error: None,
            }],
            name: None,
            file: None,
        })
    }

    /// The primary root folder.
    pub fn root(&self) -> Option<&Path> {
        self.roots.first().map(|root| root.path.as_path())
    }

    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    pub fn root_paths(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|root| root.path.clone()).collect()
    }

    pub fn is_multi_root(&self) -> bool {
        self.roots.len() > 1 || self.file.is_some()
    }

    /// The saved definition file, when the workspace was opened from or saved to one.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// The root folder containing `path` (the innermost one when roots are nested).
    pub fn root_of(&self, path: &Path) -> Option<&Path> {
        self.roots
            .iter()
            .map(|root| root.path.as_path())
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
    }

    /// Add a root folder; returns `false` when it is already part of the workspace.
    pub fn add_root(&mut self, path: &Path) -> io::Result<bool> {
        let root = path.canonicalize()?;
        if !fs::metadata(&root)?.is_dir() {
            return Err(io::Error::other("Choose a folder"));
        }
        if self.roots.iter().any(|existing| existing.path == root) {
            return Ok(false);
        }
        let listing = read_dir(&root)?;
        self.listings.insert(root.clone(), listing);
        self.roots.push(Root {
            path: root,
            search: true,
            error: None,
        });
        Ok(true)
    }

    /// Keep a folder that cannot be read right now (e.g. restored from a session), so it
    /// can be retried or removed.
    pub fn add_unavailable_root(&mut self, path: PathBuf, error: String) {
        if self.roots.iter().all(|root| root.path != path) {
            self.roots.push(Root {
                path,
                search: true,
                error: Some(error),
            });
        }
    }

    /// Remove a root folder (and its expanded listings). The last root cannot be removed.
    pub fn remove_root(&mut self, path: &Path) -> bool {
        if self.roots.len() < 2 {
            return false;
        }
        let Some(ix) = self.roots.iter().position(|root| root.path == path) else {
            return false;
        };
        self.roots.remove(ix);
        let remaining: Vec<PathBuf> = self.root_paths();
        self.listings.retain(|listed, _| {
            !listed.starts_with(path) || remaining.iter().any(|root| listed.starts_with(root))
        });
        true
    }

    /// Include or exclude a root from project search and Quick Open.
    pub fn set_search(&mut self, path: &Path, search: bool) {
        if let Some(root) = self.roots.iter_mut().find(|root| root.path == path) {
            root.search = search;
        }
    }

    /// Try to read a root that could not be read before.
    pub fn retry(&mut self, path: &Path) -> io::Result<()> {
        let listing = read_dir(path)?;
        self.listings.insert(path.to_path_buf(), listing);
        if let Some(root) = self.roots.iter_mut().find(|root| root.path == path) {
            root.error = None;
        }
        Ok(())
    }

    /// A short, unambiguous label for a root: its folder name, qualified by the parent
    /// folder's name when two roots share a name.
    pub fn label(&self, root: &Path) -> String {
        let name = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        };
        let own = name(root);
        let clash = self
            .roots
            .iter()
            .any(|other| other.path != root && name(&other.path) == own);
        match root.parent().filter(|_| clash) {
            Some(parent) => format!("{} ({})", own, name(parent)),
            None => own,
        }
    }

    /// `path` for display: relative to its root, prefixed with the root's label when the
    /// workspace has several roots.
    pub fn qualified(&self, path: &Path) -> String {
        self.qualifier().qualify(path)
    }

    /// Precomputed root labels for qualifying many paths (e.g. Quick Open).
    pub fn qualifier(&self) -> Qualifier {
        Qualifier {
            roots: self
                .roots
                .iter()
                .map(|root| (root.path.clone(), self.label(&root.path)))
                .collect(),
        }
    }

    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        match self.roots.as_slice() {
            [] => "No project open".into(),
            [root] => self.label(&root.path),
            [first, rest @ ..] => format!("{} (+{})", self.label(&first.path), rest.len()),
        }
    }

    pub fn is_expanded(&self, directory: &Path) -> bool {
        self.listings.contains_key(directory)
    }

    fn is_root(&self, directory: &Path) -> bool {
        self.roots.iter().any(|root| root.path == directory)
    }

    pub fn expand(&mut self, directory: PathBuf, listing: Vec<DirEntry>) -> bool {
        let reachable = self.is_root(&directory)
            || directory
                .parent()
                .and_then(|parent| self.listings.get(parent))
                .is_some_and(|siblings| {
                    siblings
                        .iter()
                        .any(|entry| entry.is_dir && entry.path == directory)
                });
        if reachable {
            self.listings.insert(directory, listing);
        }
        reachable
    }

    /// Collapse a folder. With several roots, a root folder collapses too.
    pub fn collapse(&mut self, directory: &Path) {
        if self.is_root(directory) && self.roots.len() < 2 {
            return;
        }
        self.listings.retain(|path, _| !path.starts_with(directory));
    }

    /// Explorer rows. With several roots each root is a top-level row.
    pub fn rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        match self.roots.as_slice() {
            [] => {}
            [root] if self.file.is_none() => self.push_rows(&root.path, 0, &mut rows),
            roots => {
                for root in roots {
                    let expanded = self.is_expanded(&root.path);
                    rows.push(TreeRow {
                        entry: DirEntry {
                            path: root.path.clone(),
                            name: self.label(&root.path),
                            is_dir: true,
                        },
                        depth: 0,
                        expanded,
                    });
                    if expanded {
                        self.push_rows(&root.path, 1, &mut rows);
                    }
                }
            }
        }
        rows
    }

    fn push_rows(&self, directory: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
        for entry in self.listings.get(directory).into_iter().flatten() {
            let expanded = entry.is_dir && self.is_expanded(&entry.path);
            rows.push(TreeRow {
                entry: entry.clone(),
                depth,
                expanded,
            });
            if expanded {
                self.push_rows(&entry.path, depth + 1, rows);
            }
        }
    }
}

impl Workspace {
    /// Open a saved workspace definition (`{"name": …, "folders": [{"path": …,
    /// "search": false}]}`; relative paths resolve against the file's folder). Folders
    /// that cannot be read are kept with an error so they can be retried or removed.
    pub fn open_definition(file: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(file)?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| io::Error::other(format!("{}: {error}", file.display())))?;
        let base = file
            .canonicalize()?
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let folders = value
            .get("folders")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| io::Error::other(format!("{}: missing \"folders\"", file.display())))?;
        let mut workspace = Self {
            name: value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            file: Some(base.join(file.file_name().unwrap_or_default())),
            ..Self::default()
        };
        for folder in folders {
            let Some(path) = folder
                .get("path")
                .and_then(serde_json::Value::as_str)
                .or_else(|| folder.as_str())
            else {
                continue;
            };
            let search = folder
                .get("search")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let joined = base.join(path);
            let resolved = joined.canonicalize().unwrap_or_else(|_| normalize(&joined));
            if workspace.roots.iter().any(|root| root.path == resolved) {
                continue;
            }
            let error = match read_dir(&resolved) {
                Ok(listing) => {
                    workspace.listings.insert(resolved.clone(), listing);
                    None
                }
                Err(error) => Some(error.to_string()),
            };
            workspace.roots.push(Root {
                path: resolved,
                search,
                error,
            });
        }
        if workspace.roots.is_empty() {
            return Err(io::Error::other(format!(
                "{}: the workspace has no folders",
                file.display()
            )));
        }
        Ok(workspace)
    }

    /// Save the workspace definition with folder paths relative to the file, so the
    /// workspace keeps working when its folder tree is moved together.
    pub fn save_definition(&mut self, file: &Path) -> io::Result<()> {
        let parent = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let base = parent.canonicalize()?;
        let folders: Vec<serde_json::Value> = self
            .roots
            .iter()
            .map(|root| {
                let path = relative_to(&base, &root.path)
                    .unwrap_or_else(|| root.path.clone())
                    .to_string_lossy()
                    .replace('\\', "/");
                let mut folder = serde_json::json!({ "path": path });
                if !root.search {
                    folder["search"] = serde_json::Value::Bool(false);
                }
                folder
            })
            .collect();
        let mut value = serde_json::json!({ "folders": folders });
        if let Some(name) = &self.name {
            value["name"] = serde_json::Value::String(name.clone());
        }
        let text = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
        fs::write(file, text + "\n")?;
        self.file = Some(base.join(file.file_name().unwrap_or_default()));
        Ok(())
    }

    /// Follow a rename/move of `from` to `to`: expanded folders below it stay expanded.
    pub fn remap(&mut self, from: &Path, to: &Path) {
        let moved: Vec<PathBuf> = self
            .listings
            .keys()
            .filter(|path| path.starts_with(from))
            .cloned()
            .collect();
        for old in moved {
            if let Some(listing) = self.listings.remove(&old)
                && let Ok(suffix) = old.strip_prefix(from)
            {
                self.listings.insert(join(to, suffix), listing);
            }
        }
    }

    /// Directories currently expanded (including roots).
    pub fn expanded(&self) -> Vec<PathBuf> {
        let mut expanded: Vec<PathBuf> = self.listings.keys().cloned().collect();
        expanded.sort();
        expanded
    }

    /// Forget listings of folders that no longer exist.
    pub fn prune(&mut self) {
        let roots = self.root_paths();
        self.listings
            .retain(|path, _| roots.contains(path) || path.is_dir());
    }
}

/// Qualifies paths like [`Workspace::qualified`] with the root labels computed once.
pub struct Qualifier {
    roots: Vec<(PathBuf, String)>,
}

impl Qualifier {
    pub fn qualify(&self, path: &Path) -> String {
        let Some((root, label)) = self
            .roots
            .iter()
            .filter(|(root, _)| path.starts_with(root))
            .max_by_key(|(root, _)| root.components().count())
        else {
            return path.display().to_string();
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        match (self.roots.len() > 1, relative.is_empty()) {
            (false, _) => relative,
            (true, true) => label.clone(),
            (true, false) => format!("{label}/{relative}"),
        }
    }
}

/// `target` relative to the directory `base` (both absolute), using `..` as needed.
/// `None` when they share no common prefix (e.g. different drives).
pub fn relative_to(base: &Path, target: &Path) -> Option<PathBuf> {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = target.components().collect();
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return None;
    }
    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Some(relative)
}

/// Resolve `.` and `..` lexically (for folders that do not exist).
fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn join(base: &Path, suffix: &Path) -> PathBuf {
    if suffix.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(suffix)
    }
}

/// Validate a file or folder name typed by the user. `nested` allows `a/b/c` paths
/// (folders are created as needed).
pub fn validate_name(name: &str, nested: bool) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Enter a name".into());
    }
    if trimmed != name {
        return Err("Names cannot start or end with spaces".into());
    }
    let parts: Vec<&str> = name.split(['/', '\\']).collect();
    if parts.len() > 1 && !nested {
        return Err("Names cannot contain / or \\".into());
    }
    for part in parts {
        match part {
            "" => return Err("Empty path segment".into()),
            "." | ".." => return Err(format!("\"{part}\" is not a valid name")),
            _ => {}
        }
        if part.contains(['\0', ':', '*', '?', '"', '<', '>', '|']) {
            return Err(format!(
                "\"{part}\" contains a character that is not allowed in file names"
            ));
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit());
        if reserved {
            return Err(format!("\"{part}\" is a reserved name on Windows"));
        }
    }
    Ok(())
}

/// Create an empty file (`directory` = false) or a folder named `name` inside `parent`.
pub fn create(parent: &Path, name: &str, directory: bool) -> Result<PathBuf, String> {
    validate_name(name, true)?;
    let target = parent.join(name);
    if target.exists() {
        return Err(format!("{name} already exists"));
    }
    let result = if directory {
        fs::create_dir_all(&target)
    } else {
        target
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map(|_| ())
            })
    };
    result.map_err(|error| format!("Could not create {name}: {error}"))?;
    Ok(target)
}

/// Move or rename `from` to `to`, refusing to overwrite or to move a folder into itself.
pub fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err(format!("{} no longer exists", from.display()));
    }
    if to == from {
        return Ok(());
    }
    if to.starts_with(from) {
        return Err("A folder cannot be moved into itself".into());
    }
    // Allow case-only renames on case-insensitive file systems.
    let same_file = to.exists()
        && from
            .canonicalize()
            .ok()
            .zip(to.canonicalize().ok())
            .is_some_and(|(a, b)| a == b);
    if to.exists() && !same_file {
        return Err(format!(
            "{} already exists",
            to.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    if let Some(parent) = to.parent()
        && !parent.is_dir()
    {
        return Err(format!("{} is not a folder", parent.display()));
    }
    fs::rename(from, to).map_err(|error| format!("Could not move {}: {error}", from.display()))
}

pub fn read_dir(directory: &Path) -> io::Result<Vec<DirEntry>> {
    let mut entries = fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            let path = entry.path();
            Ok(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir()),
                path,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            match a.name.to_lowercase().cmp(&b.name.to_lowercase()) {
                Ordering::Equal => a.name.cmp(&b.name),
                ordering => ordering,
            }
        })
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.entry.name))
            .collect()
    }

    #[test]
    fn lists_directories_first_and_expands_lazily() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(root.join("src/nested/deep.rs"), "").unwrap();
        fs::write(root.join("b.txt"), "").unwrap();
        fs::write(root.join("A.txt"), "").unwrap();
        fs::create_dir(root.join("empty")).unwrap();

        let mut workspace = Workspace::open(root).unwrap();
        assert_eq!(
            workspace.display_name(),
            root.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(names(&workspace.rows()), ["empty", "src", "A.txt", "b.txt"]);

        let src = workspace.root().unwrap().join("src");
        assert!(workspace.expand(src.clone(), read_dir(&src).unwrap()));
        let nested = src.join("nested");
        assert!(workspace.expand(nested.clone(), read_dir(&nested).unwrap()));
        assert_eq!(
            names(&workspace.rows()),
            [
                "empty",
                "src",
                "  nested",
                "    deep.rs",
                "  lib.rs",
                "A.txt",
                "b.txt"
            ]
        );

        workspace.collapse(&src);
        assert!(!workspace.is_expanded(&nested));
        assert_eq!(names(&workspace.rows()), ["empty", "src", "A.txt", "b.txt"]);
    }

    #[test]
    fn ignores_listings_that_are_no_longer_reachable() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("a/b")).unwrap();
        let mut workspace = Workspace::open(root).unwrap();
        let b = workspace.root().unwrap().join("a/b");
        assert!(!workspace.expand(b.clone(), Vec::new()));
        assert!(!workspace.is_expanded(&b));
        let outside = tempfile::tempdir().unwrap();
        assert!(!workspace.expand(outside.path().to_path_buf(), Vec::new()));
    }

    #[test]
    fn creates_validates_and_moves_items() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        assert!(validate_name("ok.rs", false).is_ok());
        assert!(validate_name("ünïcødé 名前.txt", false).is_ok());
        for bad in ["", " x", "a/b", "..", "con.txt", "a:b", "LPT1"] {
            assert!(validate_name(bad, false).is_err(), "{bad}");
        }
        assert!(validate_name("src/new/mod.rs", true).is_ok());
        assert!(validate_name("src//mod.rs", true).is_err());

        let file = create(&root, "src/new/mod.rs", false).unwrap();
        assert!(file.is_file());
        assert_eq!(
            create(&root, "src/new/mod.rs", false).unwrap_err(),
            "src/new/mod.rs already exists"
        );
        let folder = create(&root, "ünï", true).unwrap();
        assert!(folder.is_dir());

        let target = root.join("ünï/new");
        move_path(&root.join("src/new"), &target).unwrap();
        assert!(target.join("mod.rs").is_file());
        assert!(move_path(&root.join("ünï"), &root.join("ünï/inner")).is_err());
        create(&root, "taken", true).unwrap();
        assert!(
            move_path(&target, &root.join("taken"))
                .unwrap_err()
                .contains("already exists")
        );
        assert!(move_path(&root.join("missing"), &root.join("x")).is_err());

        let mut workspace = Workspace::open(&root).unwrap();
        let unicode = root.join("ünï");
        assert!(workspace.expand(unicode.clone(), read_dir(&unicode).unwrap()));
        move_path(&unicode, &root.join("renamed")).unwrap();
        workspace.remap(&unicode, &root.join("renamed"));
        assert!(workspace.is_expanded(&root.join("renamed")));
    }

    #[test]
    fn rejects_files_as_workspace_roots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.txt");
        fs::write(&path, "").unwrap();
        assert!(Workspace::open(&path).is_err());
        assert!(Workspace::open(&directory.path().join("missing")).is_err());
    }

    #[test]
    fn multi_root_rows_labels_and_definitions() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().canonicalize().unwrap();
        for repository in ["one/app", "two/app", "lib"] {
            fs::create_dir_all(base.join(repository).join("src")).unwrap();
            fs::write(base.join(repository).join("src/main.rs"), "").unwrap();
        }
        let mut workspace = Workspace::open(&base.join("one/app")).unwrap();
        assert_eq!(names(&workspace.rows()), ["src"]);
        assert!(workspace.add_root(&base.join("two/app")).unwrap());
        assert!(!workspace.add_root(&base.join("two/app")).unwrap());
        workspace.add_root(&base.join("lib")).unwrap();
        // Duplicate folder names are qualified by their parent.
        assert_eq!(
            names(&workspace.rows()),
            ["app (one)", "  src", "app (two)", "  src", "lib", "  src"]
        );
        let main = base.join("two/app/src/main.rs");
        assert_eq!(
            workspace.root_of(&main),
            Some(base.join("two/app").as_path())
        );
        assert_eq!(workspace.qualified(&main), "app (two)/src/main.rs");
        assert_eq!(workspace.display_name(), "app (one) (+2)");
        workspace.collapse(&base.join("lib"));
        assert_eq!(names(&workspace.rows()).len(), 5);

        // Saved definitions use relative paths and survive moving the whole tree.
        workspace.set_search(&base.join("lib"), false);
        let file = base.join("one/team.hephaestus-workspace");
        workspace.save_definition(&file).unwrap();
        let saved = fs::read_to_string(&file).unwrap();
        assert!(saved.contains("\"path\": \"app\""), "{saved}");
        assert!(saved.contains("\"../two/app\""), "{saved}");
        assert!(saved.contains("\"search\": false"), "{saved}");
        let moved = directory.path().join("moved");
        fs::create_dir_all(&moved).unwrap();
        for entry in ["one", "two", "lib"] {
            fs::rename(base.join(entry), moved.join(entry)).unwrap();
        }
        let moved = moved.canonicalize().unwrap();
        let reopened =
            Workspace::open_definition(&moved.join("one/team.hephaestus-workspace")).unwrap();
        assert_eq!(
            reopened.root_paths(),
            [
                moved.join("one/app"),
                moved.join("two/app"),
                moved.join("lib")
            ]
        );
        assert!(!reopened.roots()[2].search);

        // Removing roots keeps at least one; missing folders stay with an error.
        let mut reopened = reopened;
        assert!(reopened.remove_root(&moved.join("lib")));
        fs::rename(moved.join("two"), moved.join("gone")).unwrap();
        reopened
            .save_definition(&moved.join("one/team.hephaestus-workspace"))
            .unwrap();
        let broken =
            Workspace::open_definition(&moved.join("one/team.hephaestus-workspace")).unwrap();
        assert!(broken.roots()[1].error.is_some());
        assert_eq!(names(&broken.rows()), ["app (one)", "  src", "app (two)"]);
        assert_eq!(
            relative_to(Path::new("/a/b"), Path::new("/a/c/d")),
            Some(PathBuf::from("../c/d"))
        );
    }
}
