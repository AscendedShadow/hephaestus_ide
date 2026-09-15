use std::{
    collections::HashMap,
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

    pub fn commit(&self, message: &str) -> io::Result<String> {
        let mut child = self
            .command()
            .args(["commit", "--file=-"])
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

    fn command(&self) -> Command {
        git(&self.root)
    }

    fn run<I, S>(&self, args: I) -> io::Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.command().args(args).output().map_err(not_installed)?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(failure(&output))
        }
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
    let message = [stderr.trim(), stdout.trim()]
        .into_iter()
        .flat_map(str::lines)
        .map(|line| {
            line.trim_start_matches("fatal: ")
                .trim_start_matches("error: ")
        })
        .find(|line| !line.is_empty() && !line.starts_with("hint:"))
        .unwrap_or("git failed")
        .to_string();
    io::Error::other(message)
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
