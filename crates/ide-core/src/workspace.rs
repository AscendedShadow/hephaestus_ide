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

#[derive(Debug, Default)]
pub struct Workspace {
    root: Option<PathBuf>,
    listings: HashMap<PathBuf, Vec<DirEntry>>,
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
            root: Some(root),
        })
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn display_name(&self) -> &str {
        self.root()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("No project open")
    }

    pub fn is_expanded(&self, directory: &Path) -> bool {
        self.listings.contains_key(directory)
    }

    pub fn expand(&mut self, directory: PathBuf, listing: Vec<DirEntry>) -> bool {
        let reachable = self.root() == Some(directory.as_path())
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

    pub fn collapse(&mut self, directory: &Path) {
        if self.root() == Some(directory) {
            return;
        }
        self.listings.retain(|path, _| !path.starts_with(directory));
    }

    pub fn rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        if let Some(root) = self.root() {
            self.push_rows(root, 0, &mut rows);
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

    /// Directories currently expanded (including the root).
    pub fn expanded(&self) -> Vec<PathBuf> {
        let mut expanded: Vec<PathBuf> = self.listings.keys().cloned().collect();
        expanded.sort();
        expanded
    }

    /// Forget listings of folders that no longer exist.
    pub fn prune(&mut self) {
        let root = self.root.clone();
        self.listings
            .retain(|path, _| Some(path) == root.as_ref() || path.is_dir());
    }
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
}
