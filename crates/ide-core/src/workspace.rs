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
    fn rejects_files_as_workspace_roots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.txt");
        fs::write(&path, "").unwrap();
        assert!(Workspace::open(&path).is_err());
        assert!(Workspace::open(&directory.path().join("missing")).is_err());
    }
}
