//! Project search with case, whole-word and regex options, include/exclude globs, folder
//! scopes, explicit truncation and cancellation (IDE-003), plus replacement planning with
//! stale-preview detection (IDE-004).

use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use regex::{Regex, RegexBuilder};

use crate::{document::MAX_OPEN_BYTES, git::Repository, glob::Glob, search::Match};

/// Directories skipped by default (when "use ignore files" is on), besides `.gitignore`.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "bin",
    "obj",
    "zig-cache",
    "zig-out",
    ".zig-cache",
];

pub const DEFAULT_FILE_LIMIT: usize = 20_000;
pub const DEFAULT_MATCH_LIMIT: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Comma-separated include globs; empty includes everything.
    pub include: String,
    /// Comma-separated exclude globs.
    pub exclude: String,
    /// Respect `.gitignore` and the default excluded directories.
    pub use_ignore_files: bool,
    /// Restrict the search to this folder (inside the workspace).
    pub scope: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            query: String::new(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            include: String::new(),
            exclude: String::new(),
            use_ignore_files: true,
            scope: None,
        }
    }
}

/// A compiled query.
#[derive(Clone, Debug)]
pub struct Matcher {
    regex: Regex,
    is_regex: bool,
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

impl Matcher {
    pub fn new(options: &Options) -> Result<Self, String> {
        if options.query.is_empty() {
            return Err("Enter a search term".into());
        }
        let mut pattern = if options.regex {
            options.query.clone()
        } else {
            regex::escape(&options.query)
        };
        if options.whole_word {
            // Only add a boundary where the query itself starts/ends with a word character,
            // so `->` or `.foo` still match.
            let literal_start = options.query.chars().next().is_some_and(is_word);
            let literal_end = options.query.chars().last().is_some_and(is_word);
            let (start, end) = if options.regex {
                (true, true)
            } else {
                (literal_start, literal_end)
            };
            pattern = format!(
                "{}(?:{pattern}){}",
                if start { r"\b" } else { "" },
                if end { r"\b" } else { "" }
            );
        }
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!options.case_sensitive)
            .multi_line(true)
            .unicode(true)
            .size_limit(10 * (1 << 20))
            .build()
            .map_err(|error| match error {
                regex::Error::Syntax(message) => {
                    let last = message
                        .lines()
                        .last()
                        .unwrap_or("invalid regular expression");
                    format!(
                        "Invalid regular expression: {}",
                        last.trim_start_matches("error: ")
                    )
                }
                other => format!("Invalid regular expression: {other}"),
            })?;
        Ok(Self {
            regex,
            is_regex: options.regex,
        })
    }

    /// All matches in `source` (non-empty matches only).
    pub fn find(&self, path: &Path, source: &str, limit: usize) -> Vec<Hit> {
        let mut hits = Vec::new();
        let mut line_number = 1;
        let mut line_start = 0;
        let mut counted_to = 0;
        for found in self.regex.find_iter(source) {
            if found.is_empty() {
                continue;
            }
            if hits.len() >= limit {
                break;
            }
            // Advance the line counter incrementally.
            for (ix, byte) in source.as_bytes()[counted_to..found.start()]
                .iter()
                .enumerate()
            {
                if *byte == b'\n' {
                    line_number += 1;
                    line_start = counted_to + ix + 1;
                }
            }
            counted_to = found.start();
            let line_end = source[found.start()..]
                .find('\n')
                .map_or(source.len(), |ix| found.start() + ix);
            let line_text = &source[line_start..line_end];
            hits.push(Hit {
                path: path.to_path_buf(),
                line: line_number,
                column: source[line_start..found.start()].encode_utf16().count() + 1,
                range: found.range(),
                line_range: line_start..line_end,
                matched: found.as_str().to_string(),
                line_text: line_text.trim_end_matches('\r').to_string(),
            });
        }
        hits
    }

    /// The replacement text for a match at `range` in `source`. Regex replacements expand
    /// `$1`, `${name}` and `$$`; literal replacements are inserted verbatim.
    pub fn replacement(&self, source: &str, range: &Range<usize>, replace: &str) -> Option<String> {
        if !self.is_regex {
            return Some(replace.to_string());
        }
        let captures = self.regex.captures_at(source, range.start)?;
        let whole = captures.get(0)?;
        if whole.range() != *range {
            return None;
        }
        let mut expanded = String::new();
        captures.expand(replace, &mut expanded);
        Some(expanded)
    }
}

/// One match, with enough context to preview and verify a replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    /// One-based line and UTF-16 column (as shown to users and used by navigation).
    pub line: usize,
    pub column: usize,
    /// Byte range of the match in the searched text.
    pub range: Range<usize>,
    /// Byte range of the match's first line.
    pub line_range: Range<usize>,
    pub matched: String,
    pub line_text: String,
}

impl Hit {
    pub fn location(&self) -> Match {
        Match {
            path: self.path.clone(),
            line: self.line,
            column: self.column,
            preview: self.line_text.trim().chars().take(160).collect(),
        }
    }

    /// The match's position within `line_text` (byte range), clamped to the first line.
    pub fn range_in_line(&self) -> Range<usize> {
        let start = self.range.start - self.line_range.start;
        let end = (self.range.end - self.line_range.start).min(self.line_text.len());
        start.min(end)..end
    }
}

/// File filters derived from [`Options`].
#[derive(Clone, Debug, Default)]
pub struct Filter {
    include: Vec<Glob>,
    exclude: Vec<Glob>,
    use_ignore_files: bool,
}

impl Filter {
    pub fn new(options: &Options) -> Result<Self, String> {
        let include = crate::glob::parse_list(&options.include)
            .map_err(|error| format!("files to include: {error}"))?;
        let exclude = crate::glob::parse_list(&options.exclude)
            .map_err(|error| format!("files to exclude: {error}"))?;
        Ok(Self {
            include,
            exclude,
            use_ignore_files: options.use_ignore_files,
        })
    }

    /// Exclusions win over inclusions; an empty include list includes everything.
    pub fn accepts(&self, relative: &Path) -> bool {
        if self.exclude.iter().any(|glob| glob.matches(relative)) {
            return false;
        }
        self.include.is_empty() || self.include.iter().any(|glob| glob.matches(relative))
    }

    fn skips_directory(&self, name: &std::ffi::OsStr) -> bool {
        self.use_ignore_files && DEFAULT_EXCLUDES.iter().any(|skip| name == *skip)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileList {
    pub paths: Vec<PathBuf>,
    /// The file limit was reached; later files were not listed.
    pub truncated: bool,
}

/// List searchable files below `root` (or `scope`), applying ignore files and filters.
pub fn list_files(
    root: &Path,
    filter: &Filter,
    scope: Option<&Path>,
    limit: usize,
    cancelled: &AtomicBool,
) -> std::io::Result<FileList> {
    let root = root.canonicalize()?;
    let start = match scope {
        Some(scope) => scope.canonicalize()?,
        None => root.clone(),
    };
    let ignored = if filter.use_ignore_files {
        Repository::discover(&root)
            .ok()
            .flatten()
            .and_then(|repository| repository.ignored_paths().ok())
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let mut pending = vec![start.clone()];
    let mut list = FileList::default();
    while let Some(directory) = pending.pop() {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || ignored.contains(&path) {
                continue;
            }
            let relative = path.strip_prefix(&root).unwrap_or(&path);
            if kind.is_dir() {
                if !filter.skips_directory(&entry.file_name())
                    && !filter.exclude.iter().any(|glob| glob.matches(relative))
                {
                    pending.push(path);
                }
            } else if kind.is_file()
                && filter.accepts(relative)
                && entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() <= MAX_OPEN_BYTES)
            {
                if list.paths.len() >= limit {
                    list.truncated = true;
                    list.paths.sort();
                    return Ok(list);
                }
                list.paths.push(path);
            }
        }
    }
    list.paths.sort();
    Ok(list)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Results {
    pub hits: Vec<Hit>,
    pub files_searched: usize,
    /// The match limit was reached.
    pub truncated: bool,
    /// The file list was truncated by the file limit.
    pub files_truncated: bool,
    pub cancelled: bool,
}

/// Search open buffers (their current, possibly unsaved, text) and closed files.
#[allow(clippy::too_many_arguments)]
pub fn search(
    matcher: &Matcher,
    files: &FileList,
    open: &[(PathBuf, String)],
    filter: &Filter,
    roots: &[PathBuf],
    scope: Option<&Path>,
    limit: usize,
    cancelled: &AtomicBool,
) -> Results {
    let mut results = Results {
        files_truncated: files.truncated,
        ..Default::default()
    };
    let in_scope = |path: &Path| {
        let relative = roots
            .iter()
            .filter_map(|root| path.strip_prefix(root).ok())
            .min_by_key(|relative| relative.components().count())
            .unwrap_or(path);
        scope.is_none_or(|scope| path.starts_with(scope)) && filter.accepts(relative)
    };
    let push = |path: &Path, source: &str, results: &mut Results| {
        results.files_searched += 1;
        let remaining = limit.saturating_sub(results.hits.len());
        let hits = matcher.find(path, source, remaining + 1);
        if hits.len() > remaining {
            results.truncated = true;
        }
        results.hits.extend(hits.into_iter().take(remaining));
    };
    for (path, source) in open {
        if cancelled.load(Ordering::Relaxed) {
            results.cancelled = true;
            return results;
        }
        if in_scope(path) {
            push(path, source, &mut results);
        }
        if results.truncated {
            return results;
        }
    }
    for path in &files.paths {
        if cancelled.load(Ordering::Relaxed) {
            results.cancelled = true;
            return results;
        }
        if open.iter().any(|(opened, _)| opened == path) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(source) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        if source.contains('\0') {
            continue;
        }
        // Match the editor's view of the file (CRLF normalized), so hit offsets are
        // valid against an opened `Document`.
        let normalized;
        let source = if source.contains("\r\n") {
            normalized = source.replace("\r\n", "\n");
            normalized.as_str()
        } else {
            source
        };
        push(path, source, &mut results);
        if results.truncated {
            return results;
        }
    }
    results
        .hits
        .sort_by(|a, b| a.path.cmp(&b.path).then(a.range.start.cmp(&b.range.start)));
    results
}

/// Group consecutive hits by file: `(path, hit indices)`.
pub fn group(hits: &[Hit]) -> Vec<(PathBuf, Vec<usize>)> {
    let mut groups: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    for (ix, hit) in hits.iter().enumerate() {
        match groups.last_mut() {
            Some((path, indices)) if *path == hit.path => indices.push(ix),
            _ => groups.push((hit.path.clone(), vec![ix])),
        }
    }
    groups
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// The file no longer contains the previewed match at the previewed position.
    Stale,
    /// The replacement could not be computed (e.g. a regex capture mismatch).
    Invalid(String),
}

/// Plan replacements for the selected hits of one file against its current `source`.
/// Every hit must still match exactly the previewed text at the same position, otherwise
/// the preview is stale and nothing is planned for the file.
pub fn plan_replacements(
    matcher: &Matcher,
    source: &str,
    hits: &[&Hit],
    replace: &str,
) -> Result<Vec<(Range<usize>, String)>, PlanError> {
    let current = matcher.find(Path::new(""), source, usize::MAX);
    let mut edits = Vec::new();
    for hit in hits {
        let still_there = current
            .iter()
            .any(|found| found.range == hit.range && found.matched == hit.matched);
        if !still_there {
            return Err(PlanError::Stale);
        }
        let replacement = matcher
            .replacement(source, &hit.range, replace)
            .ok_or_else(|| PlanError::Invalid(format!("cannot expand {replace:?}")))?;
        edits.push((hit.range.clone(), replacement));
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(query: &str) -> Options {
        Options {
            query: query.into(),
            ..Default::default()
        }
    }

    fn found(options: &Options, source: &str) -> Vec<String> {
        Matcher::new(options)
            .unwrap()
            .find(Path::new("f"), source, 100)
            .into_iter()
            .map(|hit| hit.matched)
            .collect()
    }

    #[test]
    fn case_whole_word_and_regex_options() {
        let source = "Needle needle needles _needle über-needle";
        assert_eq!(found(&options("needle"), source).len(), 5);
        let case = Options {
            case_sensitive: true,
            ..options("Needle")
        };
        assert_eq!(found(&case, source), ["Needle"]);
        let word = Options {
            whole_word: true,
            ..options("needle")
        };
        assert_eq!(found(&word, source), ["Needle", "needle", "needle"]);
        let unicode = Options {
            whole_word: true,
            ..options("über")
        };
        assert_eq!(found(&unicode, "überall über"), ["über"]);
        let arrow = Options {
            whole_word: true,
            ..options("->")
        };
        assert_eq!(found(&arrow, "a->b"), ["->"]);
        let regex = Options {
            regex: true,
            ..options(r"fn (\w+)\(")
        };
        assert_eq!(
            found(&regex, "fn alpha(x) fn beta()"),
            ["fn alpha(", "fn beta("]
        );
        let error = Matcher::new(&Options {
            regex: true,
            ..options("(unclosed")
        })
        .unwrap_err();
        assert!(error.starts_with("Invalid regular expression"), "{error}");
        assert!(Matcher::new(&options("")).is_err());
    }

    #[test]
    fn hits_report_lines_utf16_columns_and_line_text() {
        let matcher = Matcher::new(&options("x")).unwrap();
        let hits = matcher.find(Path::new("f"), "a\n😀 x\r\nz x", 10);
        assert_eq!((hits[0].line, hits[0].column), (2, 4));
        assert_eq!(hits[0].line_text, "😀 x");
        assert_eq!(hits[0].range_in_line(), 5..6);
        assert_eq!((hits[1].line, hits[1].column), (3, 3));
    }

    #[test]
    fn regex_replacements_expand_captures() {
        let regex = Options {
            regex: true,
            ..options(r"(?P<name>\w+)\.len\(\)")
        };
        let matcher = Matcher::new(&regex).unwrap();
        let source = "a.len() + bb.len()";
        let hits = matcher.find(Path::new("f"), source, 10);
        let refs: Vec<_> = hits.iter().collect();
        let edits = plan_replacements(&matcher, source, &refs, "len(&${name})").unwrap();
        assert_eq!(
            crate::text_edit::apply(source, &edits).unwrap(),
            "len(&a) + len(&bb)"
        );
        let literal = Matcher::new(&options("a")).unwrap();
        let hits = literal.find(Path::new("f"), "a", 10);
        let edits = plan_replacements(&literal, "a", &[&hits[0]], "$1").unwrap();
        assert_eq!(edits[0].1, "$1", "literal mode keeps dollars");
        // A changed file is detected as stale.
        assert_eq!(
            plan_replacements(&matcher, "x a.len()", &refs, "y"),
            Err(PlanError::Stale)
        );
    }

    #[test]
    fn filters_scopes_limits_and_ignore_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for (path, text) in [
            ("src/main.rs", "needle"),
            ("src/lib.rs", "needle needle"),
            ("src/gen/out.rs", "needle"),
            ("docs/readme.md", "needle"),
            ("target/debug/x.rs", "needle"),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let never = AtomicBool::new(false);
        let run = |options: Options, limit: usize| {
            let filter = Filter::new(&options).unwrap();
            let scope = options.scope.as_deref();
            let files = list_files(root, &filter, scope, 100, &never).unwrap();
            let matcher = Matcher::new(&options).unwrap();
            let root = root.canonicalize().unwrap();
            search(
                &matcher,
                &files,
                &[],
                &filter,
                std::slice::from_ref(&root),
                None,
                limit,
                &never,
            )
        };
        assert_eq!(run(options("needle"), 100).hits.len(), 5);
        let rust_only = Options {
            include: "*.rs".into(),
            exclude: "src/gen".into(),
            ..options("needle")
        };
        assert_eq!(run(rust_only, 100).hits.len(), 3);
        let everything = Options {
            use_ignore_files: false,
            ..options("needle")
        };
        assert_eq!(run(everything, 100).hits.len(), 6);
        let scoped = Options {
            scope: Some(root.join("docs")),
            ..options("needle")
        };
        assert_eq!(run(scoped, 100).hits.len(), 1);
        let limited = run(options("needle"), 2);
        assert_eq!(limited.hits.len(), 2);
        assert!(limited.truncated);
        assert!(
            Filter::new(&Options {
                include: "{bad".into(),
                ..options("x")
            })
            .is_err()
        );
        let cancelled = AtomicBool::new(true);
        let filter = Filter::new(&options("x")).unwrap();
        let files = FileList {
            paths: vec![root.join("src/main.rs")],
            truncated: false,
        };
        let matcher = Matcher::new(&options("needle")).unwrap();
        assert!(search(&matcher, &files, &[], &filter, &[], None, 10, &cancelled).cancelled);
    }

    #[test]
    fn open_buffers_are_the_source_of_truth() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.txt");
        fs::write(&path, "disk needle").unwrap();
        let files = FileList {
            paths: vec![path.clone()],
            truncated: false,
        };
        let matcher = Matcher::new(&options("needle")).unwrap();
        let never = AtomicBool::new(false);
        let open = vec![(path.clone(), "unsaved\nneedle".to_string())];
        let results = search(
            &matcher,
            &files,
            &open,
            &Filter::default(),
            &[],
            None,
            10,
            &never,
        );
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].line, 2);
        assert_eq!(group(&results.hits), vec![(path, vec![0])]);
    }
}
