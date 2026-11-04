//! Branches (IDE-026), history and blame (IDE-027) and hunk staging (IDE-028).

use std::{
    io::{self, Write as _},
    process::Stdio,
};

use crate::git::{Diff, FileStatus, Repository, failure, not_installed};

// ----- branches -----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    /// Short name (`main`, `origin/feature`).
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: Option<String>,
    /// `[ahead 1, behind 2]`, `[gone]` or empty.
    pub track: String,
    pub commit: String,
    pub subject: String,
}

impl Repository {
    pub fn branches(&self) -> io::Result<Vec<BranchInfo>> {
        let output = self.run([
            "for-each-ref",
            "--format=%(refname)%00%(refname:short)%00%(HEAD)%00%(upstream:short)%00%(upstream:track)%00%(objectname:short)%00%(contents:subject)",
            "refs/heads",
            "refs/remotes",
        ])?;
        let text = String::from_utf8_lossy(&output);
        Ok(text
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\0').collect();
                let [refname, name, head, upstream, track, commit, subject] = fields[..] else {
                    return None;
                };
                // Skip symbolic `origin/HEAD`.
                if refname.ends_with("/HEAD") {
                    return None;
                }
                Some(BranchInfo {
                    name: name.to_string(),
                    remote: refname.starts_with("refs/remotes/"),
                    current: head == "*",
                    upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
                    track: track.to_string(),
                    commit: commit.to_string(),
                    subject: subject.to_string(),
                })
            })
            .collect())
    }

    /// Validate a new branch name with Git's own rules.
    pub fn check_branch_name(&self, name: &str) -> Result<(), String> {
        let output = self
            .output(["check-ref-format", "--branch", name])
            .map_err(|error| error.to_string())?;
        if output.status.success() && !name.trim().is_empty() {
            Ok(())
        } else {
            Err(format!("\"{name}\" is not a valid branch name"))
        }
    }

    /// Create `name` from `from` (a branch, tag or commit), optionally switching to it.
    pub fn create_branch(&self, name: &str, from: &str, switch: bool) -> io::Result<()> {
        self.check_branch_name(name).map_err(io::Error::other)?;
        if switch {
            self.run(["switch", "-c", name, from]).map(|_| ())
        } else {
            self.run(["branch", name, from]).map(|_| ())
        }
    }

    pub fn switch_branch(&self, name: &str) -> io::Result<()> {
        self.run(["switch", name]).map(|_| ())
    }

    /// Check out a remote branch as a new local branch, tracking it or not.
    pub fn checkout_remote(&self, remote: &str, local: &str, track: bool) -> io::Result<()> {
        self.check_branch_name(local).map_err(io::Error::other)?;
        let mode = if track { "--track" } else { "--no-track" };
        self.run(["switch", "-c", local, mode, remote]).map(|_| ())
    }
}

// ----- history -----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Unix timestamp (seconds).
    pub time: i64,
    pub summary: String,
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.hash[..self.hash.len().min(8)]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    pub status: char,
    pub path: String,
    /// Previous path for renames and copies.
    pub original: Option<String>,
}

impl Repository {
    /// One page of history (newest first), optionally for one file following renames.
    pub fn log(&self, skip: usize, limit: usize, path: Option<&str>) -> io::Result<Vec<Commit>> {
        let mut args = vec![
            "log".to_string(),
            "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s%x1e".into(),
            format!("--skip={skip}"),
            format!("-n{limit}"),
        ];
        if let Some(path) = path {
            args.extend(["--follow".into(), "--".into(), path.to_string()]);
        }
        let output = self.output(&args)?;
        if !output.status.success() {
            // A repository without commits has no history.
            if String::from_utf8_lossy(&output.stderr).contains("does not have any commits") {
                return Ok(Vec::new());
            }
            return Err(failure(&output));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text
            .split('\u{1e}')
            .filter_map(|record| {
                let record = record.trim_start_matches('\n');
                let fields: Vec<&str> = record.split('\0').collect();
                let [hash, parents, author, email, time, summary] = fields[..] else {
                    return None;
                };
                Some(Commit {
                    hash: hash.to_string(),
                    parents: parents.split_whitespace().map(str::to_string).collect(),
                    author: author.to_string(),
                    email: email.to_string(),
                    time: time.parse().unwrap_or(0),
                    summary: summary.to_string(),
                })
            })
            .collect())
    }

    /// Files changed by `commit` relative to its `parent`-th parent (0 = first; root
    /// commits compare with the empty tree).
    pub fn changed_files(&self, commit: &Commit, parent: usize) -> io::Result<Vec<ChangedFile>> {
        let base = commit
            .parents
            .get(parent)
            .cloned()
            .unwrap_or_else(|| EMPTY_TREE.to_string());
        let output = self.run(["diff", "--name-status", "-M", "-z", &base, &commit.hash])?;
        let mut fields = output
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty());
        let mut files = Vec::new();
        while let Some(status) = fields.next() {
            let status = String::from_utf8_lossy(status);
            let letter = status.chars().next().unwrap_or('M');
            let first = fields
                .next()
                .map(|path| String::from_utf8_lossy(path).into_owned());
            if matches!(letter, 'R' | 'C') {
                let second = fields
                    .next()
                    .map(|path| String::from_utf8_lossy(path).into_owned());
                if let (Some(original), Some(path)) = (first, second) {
                    files.push(ChangedFile {
                        status: letter,
                        path,
                        original: Some(original),
                    });
                }
            } else if let Some(path) = first {
                files.push(ChangedFile {
                    status: letter,
                    path,
                    original: None,
                });
            }
        }
        Ok(files)
    }

    /// The diff of one file in `commit` against its `parent`-th parent.
    pub fn commit_diff(
        &self,
        commit: &Commit,
        parent: usize,
        file: &ChangedFile,
    ) -> io::Result<Diff> {
        let base = commit
            .parents
            .get(parent)
            .cloned()
            .unwrap_or_else(|| EMPTY_TREE.to_string());
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "-M",
            &base,
            &commit.hash,
            "--",
        ];
        if let Some(original) = &file.original {
            args.push(original);
        }
        args.push(&file.path);
        let output = self.run(args)?;
        Ok(Diff::parse(&String::from_utf8_lossy(&output)))
    }

    /// File content at a revision, without touching the worktree.
    pub fn show_file(&self, revision: &str, path: &str) -> io::Result<Vec<u8>> {
        self.run(["show", &format!("{revision}:{path}")])
    }
}

/// Git's well-known empty tree object, used as the parent of root commits.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

// ----- blame -----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// One-based line in the blamed contents.
    pub line: usize,
    pub commit: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
    /// The line is not committed (unsaved or uncommitted change).
    pub uncommitted: bool,
}

impl Repository {
    /// Blame `path` using `contents` (the editor's current text, so unsaved edits show as
    /// uncommitted lines).
    pub fn blame(&self, path: &str, contents: &str) -> io::Result<Vec<BlameLine>> {
        let mut child = self
            .command()
            .args(["blame", "--porcelain", "--contents", "-", "--", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(not_installed)?;
        if let Some(mut stdin) = child.stdin.take() {
            let contents = contents.to_string();
            std::thread::spawn(move || {
                let _ = stdin.write_all(contents.as_bytes());
            });
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(failure(&output));
        }
        Ok(parse_blame(&String::from_utf8_lossy(&output.stdout)))
    }
}

pub fn parse_blame(porcelain: &str) -> Vec<BlameLine> {
    use std::collections::HashMap;
    let mut details: HashMap<String, (String, i64, String)> = HashMap::new();
    let mut lines = Vec::new();
    let mut current: Option<(String, usize)> = None;
    for line in porcelain.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            let _ = text;
            if let Some((commit, number)) = current.take() {
                let (author, time, summary) = details.get(&commit).cloned().unwrap_or_default();
                lines.push(BlameLine {
                    line: number,
                    uncommitted: commit.chars().all(|c| c == '0'),
                    commit,
                    author,
                    time,
                    summary,
                });
            }
            continue;
        }
        let mut words = line.split(' ');
        let first = words.next().unwrap_or_default();
        if first.len() == 40 && first.chars().all(|c| c.is_ascii_hexdigit()) {
            let _original = words.next();
            let number = words.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            details.entry(first.to_string()).or_default();
            current = Some((first.to_string(), number));
            continue;
        }
        if let Some((commit, _)) = &current {
            let entry = details.entry(commit.clone()).or_default();
            if let Some(author) = line.strip_prefix("author ") {
                entry.0 = author.to_string();
            } else if let Some(time) = line.strip_prefix("author-time ") {
                entry.1 = time.parse().unwrap_or(0);
            } else if let Some(summary) = line.strip_prefix("summary ") {
                entry.2 = summary.to_string();
            }
        }
    }
    lines
}

// ----- hunks -----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@` line.
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    /// Raw patch bytes of the file header (`diff --git`, `index`, `---`, `+++`).
    pub file_header: Vec<u8>,
    /// Raw bytes of the hunk, header included, exactly as Git produced them (tabs, CRLF
    /// and `\ No newline at end of file` markers preserved).
    pub patch: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HunkError {
    /// Binary files, conflicts, mode-only changes: hunks are not available.
    Unavailable(String),
    /// The diff changed since it was shown; refresh before trying again.
    Stale,
    Git(String),
}

impl std::fmt::Display for HunkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(formatter, "Hunks are not available: {reason}"),
            Self::Stale => write!(formatter, "The diff changed since it was shown; refreshed"),
            Self::Git(error) => write!(formatter, "{error}"),
        }
    }
}

/// Split raw `git diff` output for one file into hunks.
pub fn parse_hunks(raw: &[u8]) -> Result<Vec<Hunk>, HunkError> {
    let mut header = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    for line in raw.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
            return Err(HunkError::Unavailable("binary file".into()));
        }
        if line.starts_with(b"diff --cc ") || line.starts_with(b"diff --combined ") {
            return Err(HunkError::Unavailable(
                "the file has merge conflicts".into(),
            ));
        }
        if line.starts_with(b"@@ ") {
            let text = String::from_utf8_lossy(line).trim_end().to_string();
            let (old_start, new_start) = starts(&text).unwrap_or((0, 0));
            hunks.push(Hunk {
                header: text,
                old_start,
                new_start,
                file_header: header.clone(),
                patch: line.to_vec(),
            });
        } else if let Some(hunk) = hunks.last_mut() {
            hunk.patch.extend_from_slice(line);
        } else {
            header.extend_from_slice(line);
        }
    }
    Ok(hunks)
}

fn starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split(' ').skip(1);
    let start = |part: Option<&str>, sign: char| -> Option<u32> {
        part?.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    Some((start(parts.next(), '-')?, start(parts.next(), '+')?))
}

impl Repository {
    /// Raw diff bytes for a file (unstaged: worktree vs index; staged: index vs HEAD).
    pub fn raw_diff(&self, file: &FileStatus, staged: bool) -> io::Result<Vec<u8>> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-U3"];
        if staged {
            args.push("--cached");
        }
        args.push("--");
        args.push(&file.relative);
        self.run(args)
    }

    pub fn hunks(&self, file: &FileStatus, staged: bool) -> Result<Vec<Hunk>, HunkError> {
        if file.staged == Some(crate::git::Change::Conflicted)
            || file.unstaged == Some(crate::git::Change::Conflicted)
        {
            return Err(HunkError::Unavailable(
                "the file has merge conflicts".into(),
            ));
        }
        if !staged && file.unstaged == Some(crate::git::Change::Untracked) {
            return Err(HunkError::Unavailable(
                "untracked files are staged as a whole".into(),
            ));
        }
        let raw = self
            .raw_diff(file, staged)
            .map_err(|error| HunkError::Git(error.to_string()))?;
        parse_hunks(&raw)
    }

    /// Stage an unstaged hunk, or unstage a staged one. The hunk must still be part of the
    /// current diff; otherwise nothing changes and [`HunkError::Stale`] is returned.
    pub fn apply_hunk(
        &self,
        file: &FileStatus,
        hunk: &Hunk,
        staged: bool,
    ) -> Result<(), HunkError> {
        let current = self.hunks(file, staged)?;
        if !current.iter().any(|candidate| {
            candidate.patch == hunk.patch && candidate.file_header == hunk.file_header
        }) {
            return Err(HunkError::Stale);
        }
        let mut patch = hunk.file_header.clone();
        patch.extend_from_slice(&hunk.patch);
        let mut args = vec!["apply", "--cached", "--whitespace=nowarn"];
        if staged {
            args.push("--reverse");
        }
        args.push("-");
        let mut child = self
            .command()
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| HunkError::Git(error.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&patch)
                .map_err(|error| HunkError::Git(error.to_string()))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| HunkError::Git(error.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(HunkError::Git(failure(&output).to_string()))
        }
    }

    /// Commit exactly what is staged (keeps partially staged files partial).
    pub fn commit_staged(&self, message: &str) -> io::Result<String> {
        self.write_commit(message, &[])
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(root)
            .args([
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn repository() -> Option<(tempfile::TempDir, Repository)> {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        if !Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return None;
        }
        git(root, &["init", "-q", "-b", "main"]);
        let repository = Repository::discover(root).unwrap().unwrap();
        Some((directory, repository))
    }

    #[test]
    fn branches_create_switch_and_validate() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let root = directory.path();
        fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "first"]);
        repository.create_branch("feature/x", "HEAD", true).unwrap();
        let branches = repository.branches().unwrap();
        assert!(
            branches
                .iter()
                .any(|branch| branch.name == "feature/x" && branch.current)
        );
        assert!(
            branches
                .iter()
                .any(|branch| branch.name == "main" && !branch.current)
        );
        assert!(
            repository
                .create_branch("bad..name", "HEAD", false)
                .is_err()
        );
        // Conflicting local changes are reported by Git and nothing switches.
        fs::write(root.join("a.txt"), "feature\n").unwrap();
        git(root, &["commit", "-qam", "feature change"]);
        fs::write(root.join("a.txt"), "dirty\n").unwrap();
        let error = repository.switch_branch("main").unwrap_err().to_string();
        assert!(error.contains("would be overwritten"), "{error}");
    }

    #[test]
    fn history_renames_merges_blame_and_show() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let root = directory.path();
        assert!(
            repository.log(0, 10, None).unwrap().is_empty(),
            "empty history"
        );
        fs::write(root.join("old.txt"), "one\ntwo\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "add old"]);
        git(root, &["mv", "old.txt", "new.txt"]);
        git(root, &["commit", "-qm", "rename"]);
        git(root, &["switch", "-qc", "side"]);
        fs::write(root.join("side.txt"), "s\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "side"]);
        git(root, &["switch", "-q", "main"]);
        fs::write(root.join("new.txt"), "one\nTWO\n").unwrap();
        git(root, &["commit", "-qam", "edit"]);
        git(root, &["merge", "-q", "--no-edit", "side"]);

        let history = repository.log(0, 10, None).unwrap();
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].parents.len(), 2, "merge commit");
        let page = repository.log(1, 2, None).unwrap();
        assert_eq!(page.len(), 2);
        let file_history = repository.log(0, 10, Some("new.txt")).unwrap();
        let summaries: Vec<_> = file_history
            .iter()
            .map(|commit| commit.summary.as_str())
            .collect();
        assert_eq!(
            summaries,
            ["edit", "rename", "add old"],
            "follows the rename"
        );

        let rename = file_history
            .iter()
            .find(|commit| commit.summary == "rename")
            .unwrap();
        let files = repository.changed_files(rename, 0).unwrap();
        assert_eq!(files[0].status, 'R');
        assert_eq!(files[0].original.as_deref(), Some("old.txt"));
        // The merge against its second parent shows the main-line edit.
        let merge = &history[0];
        let against_side = repository.changed_files(merge, 1).unwrap();
        assert!(against_side.iter().any(|file| file.path == "new.txt"));
        let against_main = repository.changed_files(merge, 0).unwrap();
        assert!(against_main.iter().any(|file| file.path == "side.txt"));
        let root_commit = history.last().unwrap();
        assert_eq!(
            repository.changed_files(root_commit, 0).unwrap()[0].path,
            "old.txt"
        );
        assert_eq!(
            repository.show_file(&root_commit.hash, "old.txt").unwrap(),
            b"one\ntwo\n"
        );

        let blame = repository.blame("new.txt", "one\nTWO\nunsaved\n").unwrap();
        assert_eq!(blame.len(), 3);
        assert_eq!(blame[0].summary, "add old");
        assert_eq!(blame[1].summary, "edit");
        assert!(blame[2].uncommitted);
    }

    #[test]
    fn hunks_stage_and_unstage_individually_and_detect_stale_diffs() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let root = directory.path();
        let original: String = (1..=30).map(|n| format!("line {n}\r\n")).collect();
        fs::write(root.join("f.txt"), &original).unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "base"]);
        let edited = original
            .replace("line 2\r\n", "line two\t(tab)\r\n")
            .replace("line 29\r\n", "line twenty-nine\r\n");
        fs::write(root.join("f.txt"), &edited).unwrap();
        let file = repository
            .status()
            .unwrap()
            .files
            .into_iter()
            .next()
            .unwrap();
        let hunks = repository.hunks(&file, false).unwrap();
        assert_eq!(hunks.len(), 2);
        assert!(
            hunks[0].patch.windows(2).any(|pair| pair == b"\r\n"),
            "raw CRLF preserved"
        );
        repository.apply_hunk(&file, &hunks[1], false).unwrap();

        let staged = repository.raw_diff(&file, true).unwrap();
        let staged = String::from_utf8_lossy(&staged);
        assert!(
            staged.contains("twenty-nine") && !staged.contains("line two"),
            "{staged}"
        );
        // Applying the same (now obsolete) hunk again is rejected as stale.
        assert_eq!(
            repository.apply_hunk(&file, &hunks[1], false),
            Err(HunkError::Stale)
        );

        let file = repository
            .status()
            .unwrap()
            .files
            .into_iter()
            .next()
            .unwrap();
        let staged_hunks = repository.hunks(&file, true).unwrap();
        repository
            .apply_hunk(&file, &staged_hunks[0], true)
            .unwrap();
        assert!(repository.raw_diff(&file, true).unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(root.join("f.txt")).unwrap(),
            edited,
            "worktree untouched"
        );

        // Committing only the staged part keeps the rest unstaged.
        let hunks = repository.hunks(&file, false).unwrap();
        repository.apply_hunk(&file, &hunks[0], false).unwrap();
        git(root, &["-c", "user.name=T", "config", "user.email", "t@t"]);
        git(root, &["config", "user.name", "T"]);
        repository.commit_staged("only line two").unwrap();
        let remaining =
            String::from_utf8_lossy(&repository.raw_diff(&file, false).unwrap()).into_owned();
        assert!(
            remaining.contains("twenty-nine") && !remaining.contains("line two"),
            "{remaining}"
        );

        assert!(matches!(
            parse_hunks(b"diff --git a/x b/x\nBinary files a/x and b/x differ\n"),
            Err(HunkError::Unavailable(_))
        ));
    }
}
