use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{document::MAX_OPEN_BYTES, git::Repository};

const MAX_FILES: usize = 20_000;
const SKIP: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "bin",
    "obj",
    "zig-cache",
    "zig-out",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

pub fn files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let root = root.canonicalize()?;
    let ignored = Repository::discover(&root)
        .ok()
        .flatten()
        .and_then(|repository| repository.ignored_paths().ok())
        .unwrap_or_default();
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) if directory != root => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink()
                || SKIP.iter().any(|name| entry.file_name() == *name)
                || path
                    .ancestors()
                    .take_while(|parent| parent.starts_with(&root))
                    .any(|parent| ignored.contains(parent))
            {
                continue;
            }
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() <= MAX_OPEN_BYTES)
            {
                found.push(path);
                if found.len() >= MAX_FILES {
                    found.sort();
                    return Ok(found);
                }
            }
        }
    }
    found.sort();
    Ok(found)
}

pub fn rank(path: &Path, query: &str) -> Option<usize> {
    let text = path.to_string_lossy().to_lowercase();
    let query = query.to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let mut from = 0;
    let mut score = 0;
    for character in query.chars() {
        let next = text[from..].find(character)? + from;
        score += next - from;
        from = next + character.len_utf8();
    }
    if text
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|name| name.contains(&query))
    {
        score = score.saturating_sub(5);
    }
    Some(score)
}

pub fn search_text(paths: &[PathBuf], query: &str, limit: usize) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for path in paths {
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(source) = std::str::from_utf8(&bytes) else {
            continue;
        };
        if source.contains('\0') {
            continue;
        }
        hits.extend(search_source(path, source, query, limit - hits.len()));
        if hits.len() >= limit {
            return hits;
        }
    }
    hits
}

pub fn search_source(path: &Path, source: &str, query: &str, limit: usize) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    source
        .lines()
        .enumerate()
        .flat_map(|(line, text)| {
            text.match_indices(query)
                .map(move |(column, _)| Match {
                    path: path.to_path_buf(),
                    line: line + 1,
                    column: text[..column].encode_utf16().count() + 1,
                    preview: text.trim().chars().take(160).collect(),
                })
                .collect::<Vec<_>>()
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_generated_directories_and_finds_unicode_columns() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/ignored.rs"), "needle").unwrap();
        fs::write(dir.path().join("source.rs"), "🦀 needle\n").unwrap();
        let paths = files(dir.path()).unwrap();
        assert_eq!(paths.len(), 1);
        let hits = search_text(&paths, "needle", 100);
        assert_eq!((hits[0].line, hits[0].column), (1, 4));
        assert!(rank(Path::new("src/source.rs"), "srs").is_some());
        assert!(rank(Path::new("src/source.rs"), "zzz").is_none());
    }

    #[test]
    fn respects_git_ignored_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let init = std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(root)
            .output();
        if !init.is_ok_and(|output| output.status.success()) {
            return;
        }
        fs::write(root.join(".gitignore"), "private/\n").unwrap();
        fs::create_dir(root.join("private")).unwrap();
        fs::write(root.join("private/secret.rs"), "hidden").unwrap();
        fs::write(root.join("visible.rs"), "shown").unwrap();
        let paths = files(root).unwrap();
        assert!(paths.iter().any(|path| path.ends_with("visible.rs")));
        assert!(!paths.iter().any(|path| path.ends_with("secret.rs")));
    }
}
