use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

pub const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;

const TAB_WIDTH: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl Change {
    fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            b'A' => Self::Added,
            b'M' => Self::Modified,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'T' => Self::TypeChanged,
            _ => return None,
        })
    }

    pub fn letter(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::TypeChanged => "T",
            Self::Untracked => "U",
            Self::Conflicted => "!",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Conflicted => 3,
            Self::Added | Self::Untracked => 1,
            _ => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    pub relative: String,
    pub original: Option<String>,
    pub staged: Option<Change>,
    pub unstaged: Option<Change>,
}

impl FileStatus {
    pub fn change(&self) -> Change {
        self.unstaged.or(self.staged).unwrap_or(Change::Modified)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branch {
    pub name: Option<String>,
    pub commit: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

impl Branch {
    pub fn label(&self) -> String {
        let mut label = match (&self.name, &self.commit) {
            (Some(name), _) => name.clone(),
            (None, Some(commit)) => format!("detached at {}", short(commit)),
            (None, None) => "detached".into(),
        };
        if self.ahead > 0 {
            label.push_str(&format!(" ↑{}", self.ahead));
        }
        if self.behind > 0 {
            label.push_str(&format!(" ↓{}", self.behind));
        }
        label
    }
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: Branch,
    pub files: Vec<FileStatus>,
}

impl Status {
    pub fn parse(output: &str) -> io::Result<Self> {
        let mut status = Self::default();
        let mut fields = output.split('\0').filter(|field| !field.is_empty());
        while let Some(field) = fields.next() {
            let malformed = || io::Error::other(format!("Unexpected git status line: {field}"));
            let (kind, rest) = field.split_once(' ').ok_or_else(malformed)?;
            match kind {
                "#" => status.branch.parse_header(rest),
                "1" => {
                    let parts: Vec<_> = rest.splitn(8, ' ').collect();
                    let [xy, _, _, _, _, _, _, path] = parts[..] else {
                        return Err(malformed());
                    };
                    status
                        .files
                        .extend(tracked(xy, path, None).ok_or_else(malformed)?);
                }
                "2" => {
                    let parts: Vec<_> = rest.splitn(9, ' ').collect();
                    let [xy, _, _, _, _, _, _, _, path] = parts[..] else {
                        return Err(malformed());
                    };
                    let original = fields.next().ok_or_else(malformed)?;
                    status
                        .files
                        .extend(tracked(xy, path, Some(original)).ok_or_else(malformed)?);
                }
                "u" => {
                    let path = rest.splitn(10, ' ').nth(9).ok_or_else(malformed)?;
                    status.files.push(FileStatus {
                        relative: path.into(),
                        original: None,
                        staged: None,
                        unstaged: Some(Change::Conflicted),
                    });
                }
                "?" => status.files.push(FileStatus {
                    relative: rest.into(),
                    original: None,
                    staged: None,
                    unstaged: Some(Change::Untracked),
                }),
                "!" => {}
                _ => return Err(malformed()),
            }
        }
        status.files.sort_by(|a, b| a.relative.cmp(&b.relative));
        Ok(status)
    }

    pub fn tree_changes(&self, root: &Path) -> HashMap<PathBuf, Change> {
        let mut changes = HashMap::new();
        for file in &self.files {
            let change = file.change();
            let mut path = absolute(root, &file.relative);
            changes.insert(path.clone(), change);
            while path.pop() && path.starts_with(root) && path != root {
                let folder = changes.entry(path.clone()).or_insert(change);
                if change.rank() > folder.rank() {
                    *folder = change;
                }
            }
        }
        changes
    }
}

impl Branch {
    fn parse_header(&mut self, header: &str) {
        let Some((key, value)) = header.split_once(' ') else {
            return;
        };
        match key {
            "branch.oid" => self.commit = (value != "(initial)").then(|| value.into()),
            "branch.head" => self.name = (value != "(detached)").then(|| value.into()),
            "branch.upstream" => self.upstream = Some(value.into()),
            "branch.ab" => {
                for count in value.split(' ') {
                    if let Some(ahead) = count.strip_prefix('+') {
                        self.ahead = ahead.parse().unwrap_or(0);
                    } else if let Some(behind) = count.strip_prefix('-') {
                        self.behind = behind.parse().unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }
}

fn tracked(xy: &str, path: &str, original: Option<&str>) -> Option<Option<FileStatus>> {
    let &[x, y] = xy.as_bytes() else {
        return None;
    };
    let (staged, unstaged) = (Change::from_code(x), Change::from_code(y));
    Some(
        (staged.is_some() || unstaged.is_some()).then(|| FileStatus {
            relative: path.into(),
            original: original.map(Into::into),
            staged,
            unstaged,
        }),
    )
}

pub fn clone_folder_name(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(['/', '\\']);
    let url = url.strip_suffix(".git").unwrap_or(url);
    let name = url
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\', ':'])
        .next()?;
    (!matches!(name, "" | "." | "..")).then(|| name.to_string())
}

pub fn absolute(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Meta,
    Hunk,
    Context,
    Added,
    Removed,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<DiffLine>,
}

impl Diff {
    pub fn parse(output: &str) -> Self {
        let mut lines = Vec::new();
        let mut columns = 0;
        let (mut old, mut new) = (0, 0);
        for line in output.lines() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.starts_with("diff ") {
                columns = 0;
            }
            if line.starts_with("@@") {
                columns = line.bytes().take_while(|&byte| byte == b'@').count() - 1;
                if columns == 1 {
                    (old, new) = hunk_starts(line).unwrap_or((0, 0));
                }
                lines.push(diff_line(LineKind::Hunk, line, None, None));
                continue;
            }
            if columns == 0 {
                let skip = ["diff ", "index ", "--- ", "+++ "];
                if !skip.iter().any(|prefix| line.starts_with(prefix)) {
                    lines.push(diff_line(LineKind::Meta, line, None, None));
                }
                continue;
            }
            if line.starts_with('\\') {
                lines.push(diff_line(LineKind::Note, line, None, None));
                continue;
            }
            let prefix = line.get(..columns).unwrap_or(line);
            let text = line.get(columns..).unwrap_or("");
            let kind = if prefix.contains('+') {
                LineKind::Added
            } else if prefix.contains('-') {
                LineKind::Removed
            } else {
                LineKind::Context
            };
            let numbered = columns == 1;
            let (old_line, new_line) = match kind {
                LineKind::Added => (None, numbered.then_some(new)),
                LineKind::Removed => (numbered.then_some(old), None),
                _ => (numbered.then_some(old), numbered.then_some(new)),
            };
            if kind != LineKind::Added {
                old += 1;
            }
            if kind != LineKind::Removed {
                new += 1;
            }
            lines.push(diff_line(kind, text, old_line, new_line));
        }
        Self { lines }
    }
}

fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut ranges = header.split(' ').skip(1);
    let start = |range: &str, sign| {
        let range = range.strip_prefix(sign)?;
        range.split(',').next()?.parse().ok()
    };
    Some((start(ranges.next()?, '-')?, start(ranges.next()?, '+')?))
}

fn diff_line(kind: LineKind, text: &str, old_line: Option<u32>, new_line: Option<u32>) -> DiffLine {
    let mut expanded = String::with_capacity(text.len());
    let mut column = 0;
    for ch in text.chars() {
        if ch == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(ch);
            column += 1;
        }
    }
    DiffLine {
        kind,
        text: expanded,
        old_line,
        new_line,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    root: PathBuf,
}

impl Repository {
    pub fn discover(directory: &Path) -> io::Result<Option<Self>> {
        let output = git(directory)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(not_installed)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return if stderr.contains("not a git repository") {
                Ok(None)
            } else {
                Err(failure(&output))
            };
        }
        let root = String::from_utf8_lossy(&output.stdout);
        let root = PathBuf::from(root.trim_end_matches(['\r', '\n']));
        Ok(Some(Self {
            root: root.canonicalize().unwrap_or(root),
        }))
    }

    pub fn init(directory: &Path) -> io::Result<Self> {
        let output = git(directory).arg("init").output().map_err(not_installed)?;
        if !output.status.success() {
            return Err(failure(&output));
        }
        Self::discover(directory)?
            .ok_or_else(|| io::Error::other("git init did not create a repository"))
    }

    pub fn clone_remote(url: &str, parent: &Path) -> io::Result<Self> {
        let url = url.trim();
        let name = clone_folder_name(url)
            .ok_or_else(|| io::Error::other("Enter a repository URL or path"))?;
        let destination = parent.join(name);
        let output = git(parent)
            .args(["clone", "--quiet", "--", url])
            .arg(&destination)
            .output()
            .map_err(not_installed)?;
        if !output.status.success() {
            return Err(failure(&output));
        }
        Self::discover(&destination)?
            .ok_or_else(|| io::Error::other("git clone did not create a repository"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn status(&self) -> io::Result<Status> {
        let output = self.run([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        Status::parse(&String::from_utf8_lossy(&output))
    }

    pub fn ignored_paths(&self) -> io::Result<HashSet<PathBuf>> {
        let output = self.run([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ])?;
        Ok(output
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                absolute(
                    &self.root,
                    String::from_utf8_lossy(path).trim_end_matches('/'),
                )
            })
            .collect())
    }

    pub fn diff(&self, file: &FileStatus, staged: bool) -> io::Result<Diff> {
        let mut command = self.command();
        let untracked = !staged && file.unstaged == Some(Change::Untracked);
        if untracked {
            command.args([
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                "--",
                "/dev/null",
            ]);
            command.arg(&file.relative);
        } else {
            command.args(["diff", "--no-color", "--no-ext-diff", "-M"]);
            if staged {
                command.arg("--cached");
            }
            command.arg("--");
            command.args(file.original.iter().filter(|_| staged));
            command.arg(&file.relative);
        }
        let output = command.output().map_err(not_installed)?;
        let expected = output.status.success() || (untracked && output.status.code() == Some(1));
        if !expected {
            return Err(failure(&output));
        }
        let truncated = output.stdout.len() > MAX_DIFF_BYTES;
        let text = &output.stdout[..output.stdout.len().min(MAX_DIFF_BYTES)];
        let mut diff = Diff::parse(&String::from_utf8_lossy(text));
        if truncated {
            diff.lines.pop();
            diff.lines.push(diff_line(
                LineKind::Note,
                "Diff too large to show in full",
                None,
                None,
            ));
        }
        Ok(diff)
    }

    pub fn stage(&self, files: &[FileStatus]) -> io::Result<()> {
        let paths = files.iter().map(|file| file.relative.as_str());
        self.run(["add", "-A", "--"].into_iter().chain(paths))
            .map(drop)
    }

    pub fn unstage(&self, files: &[FileStatus]) -> io::Result<()> {
        let paths = files.iter().flat_map(|file| {
            file.original
                .as_deref()
                .into_iter()
                .chain([file.relative.as_str()])
        });
        self.run(["reset", "-q", "--"].into_iter().chain(paths))
            .map(drop)
    }

    pub fn revert(&self, files: &[FileStatus]) -> io::Result<()> {
        let mut tracked = Vec::new();
        for file in files {
            if file.unstaged == Some(Change::Untracked) {
                let path = absolute(&self.root, &file.relative);
                match std::fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path)?,
                    Ok(_) => std::fs::remove_file(path)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            } else {
                tracked.push(file.relative.as_str());
            }
        }
        if tracked.is_empty() {
            Ok(())
        } else {
            self.run(["restore", "--worktree", "--"].into_iter().chain(tracked))
                .map(drop)
        }
    }

    pub fn commit(&self, message: &str) -> io::Result<String> {
        self.write_commit(message, &[])
    }

    pub fn commit_files(&self, files: &[FileStatus], message: &str) -> io::Result<String> {
        let paths: Vec<_> = files
            .iter()
            .flat_map(|file| {
                file.original
                    .as_deref()
                    .into_iter()
                    .chain([file.relative.as_str()])
            })
            .collect();
        self.write_commit(message, &paths)
    }

    fn write_commit(&self, message: &str, paths: &[&str]) -> io::Result<String> {
        let mut command = self.command();
        command.args(["commit", "--file=-"]);
        if !paths.is_empty() {
            command.arg("--").args(paths);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(not_installed)?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let written = stdin.write_all(message.as_bytes());
        drop(stdin);
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(failure(&output));
        }
        written?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().next().unwrap_or_default().trim().to_string())
    }

    pub fn fetch(&self) -> io::Result<String> {
        self.default_remote()?;
        self.run(["fetch", "--all", "--prune", "--quiet"])?;
        Ok("Fetched from all remotes".into())
    }

    pub fn pull(&self) -> io::Result<String> {
        self.default_remote()?;
        let before = self.head()?;
        self.run(["pull", "--no-edit", "--quiet"])?;
        let after = self.head()?;
        Ok(match (before, after) {
            (Some(before), Some(after)) if before == after => "Already up to date".into(),
            (Some(before), Some(_)) => {
                let range = format!("{before}..HEAD");
                let count = String::from_utf8_lossy(&self.run(["rev-list", "--count", &range])?)
                    .trim()
                    .parse()
                    .unwrap_or(0);
                format!("Pulled {}", commits(count))
            }
            _ => "Pulled".into(),
        })
    }

    pub fn push(&self) -> io::Result<String> {
        let branch = self.status()?.branch;
        let name = branch
            .name
            .ok_or_else(|| io::Error::other("Check out a branch before pushing"))?;
        let (args, summary): (Vec<String>, _) = match branch.upstream {
            Some(upstream) if branch.ahead == 0 => (
                vec!["push".into()],
                format!("{upstream} is already up to date"),
            ),
            Some(upstream) => (
                vec!["push".into()],
                format!("Pushed {} to {upstream}", commits(branch.ahead)),
            ),
            None => {
                let remote = self.default_remote()?;
                let summary = format!("Pushed {name} to {remote}");
                (
                    vec!["push".into(), "--set-upstream".into(), remote, name],
                    summary,
                )
            }
        };
        let output = self.output(&args)?;
        if output.status.success() {
            Ok(summary)
        } else if String::from_utf8_lossy(&output.stderr).contains("[rejected]") {
            Err(io::Error::other(
                "The remote has commits you don't have yet — pull first",
            ))
        } else {
            Err(failure(&output))
        }
    }

    fn default_remote(&self) -> io::Result<String> {
        let output = self.run(["remote"])?;
        let remotes = String::from_utf8_lossy(&output);
        let remotes: Vec<_> = remotes
            .lines()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .collect();
        remotes
            .iter()
            .find(|&&remote| remote == "origin")
            .or(remotes.first())
            .map(|remote| remote.to_string())
            .ok_or_else(|| io::Error::other("This repository has no remotes"))
    }

    fn head(&self) -> io::Result<Option<String>> {
        let output = self.output(["rev-parse", "--verify", "--quiet", "HEAD"])?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
    }

    fn command(&self) -> Command {
        git(&self.root)
    }

    fn output<I, S>(&self, args: I) -> io::Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command().args(args).output().map_err(not_installed)
    }

    fn run<I, S>(&self, args: I) -> io::Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output(args)?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(failure(&output))
        }
    }
}

fn commits(count: u32) -> String {
    match count {
        1 => "1 commit".into(),
        count => format!("{count} commits"),
    }
}

fn git(directory: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(directory)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LITERAL_PATHSPECS", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn not_installed(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Git is not installed or not on PATH",
        )
    } else {
        error
    }
}

fn failure(output: &Output) -> io::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = || {
        [stderr.trim(), stdout.trim()]
            .into_iter()
            .flat_map(str::lines)
    };
    let message = lines()
        .find_map(|line| {
            line.strip_prefix("fatal: ")
                .or_else(|| line.strip_prefix("error: "))
        })
        .or_else(|| lines().find(|line| !line.is_empty() && !line.starts_with("hint:")))
        .unwrap_or("git failed")
        .to_string();
    io::Error::other(message)
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
