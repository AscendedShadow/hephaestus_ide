use std::fs;

use super::*;

#[test]
fn parses_branch_headers_and_every_kind_of_entry() {
    let output = [
        "# branch.oid 0123456789abcdef0123456789abcdef01234567",
        "# branch.head main",
        "# branch.upstream origin/main",
        "# branch.ab +2 -1",
        "1 M. N... 100644 100644 100644 aaaa bbbb src/staged.rs",
        "1 .D N... 100644 100644 000000 aaaa aaaa gone.txt",
        "1 AM N... 000000 100644 100644 0000 bbbb new file.txt",
        "2 R. N... 100644 100644 100644 aaaa aaaa R100 docs/new name.md",
        "docs/old name.md",
        "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc both.rs",
        "? untracked dir/notes.txt",
        "",
    ]
    .join("\0");
    let status = Status::parse(&output).unwrap();
    assert_eq!(
        status.branch,
        Branch {
            name: Some("main".into()),
            commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
            upstream: Some("origin/main".into()),
            ahead: 2,
            behind: 1,
        }
    );
    assert_eq!(status.branch.label(), "main ↑2 ↓1");
    let summary: Vec<_> = status
        .files
        .iter()
        .map(|file| {
            let letter = |change: Option<Change>| change.map_or(".", Change::letter);
            format!(
                "{}{} {}{}",
                letter(file.staged),
                letter(file.unstaged),
                file.relative,
                file.original
                    .as_ref()
                    .map(|original| format!(" <- {original}"))
                    .unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ".! both.rs",
            "R. docs/new name.md <- docs/old name.md",
            ".D gone.txt",
            "AM new file.txt",
            "M. src/staged.rs",
            ".U untracked dir/notes.txt",
        ]
    );
}

#[test]
fn branch_labels_before_the_first_commit_and_when_detached() {
    let status = Status::parse("# branch.oid (initial)\0# branch.head main\0").unwrap();
    assert_eq!(status.branch.commit, None);
    assert_eq!(status.branch.label(), "main");
    let status =
        Status::parse("# branch.oid 0123456789abcdef\0# branch.head (detached)\0").unwrap();
    assert_eq!(status.branch.label(), "detached at 0123456");
}

#[test]
fn rejects_malformed_status() {
    assert!(Status::parse("1 M. short").is_err());
    assert!(Status::parse("2 R. N... 100644 100644 100644 a b R100 missing-original").is_err());
    assert!(Status::parse("x what").is_err());
}

#[test]
fn clone_folder_names_follow_the_last_url_segment() {
    let cases = [
        ("https://github.com/owner/repo.git", Some("repo")),
        ("https://github.com/owner/repo/", Some("repo")),
        ("  git@github.com:owner/repo.git  ", Some("repo")),
        ("git@host:repo.git", Some("repo")),
        ("ssh://git@host:22/team/tool.git/", Some("tool")),
        ("C:\\src\\project\\.git", Some("project")),
        ("/srv/git/project.git", Some("project")),
        ("", None),
        ("   ", None),
        ("https://host/..", None),
        ("C:\\", None),
    ];
    for (url, name) in cases {
        assert_eq!(clone_folder_name(url).as_deref(), name, "{url:?}");
    }
}

#[test]
fn folders_show_their_most_important_change() {
    let root = Path::new("/repo");
    let status = Status {
        branch: Branch::default(),
        files: [
            ("src/new.rs", Change::Untracked),
            ("src/lib.rs", Change::Modified),
            ("src/deep/merge.rs", Change::Conflicted),
            ("docs/added.md", Change::Untracked),
            ("top.txt", Change::Deleted),
        ]
        .into_iter()
        .map(|(path, change)| FileStatus {
            relative: path.into(),
            original: None,
            staged: None,
            unstaged: Some(change),
        })
        .collect(),
    };
    let changes = status.tree_changes(root);
    let change = |path: &str| changes.get(&absolute(root, path)).copied();
    assert_eq!(change("src"), Some(Change::Conflicted));
    assert_eq!(change("src/deep"), Some(Change::Conflicted));
    assert_eq!(change("src/lib.rs"), Some(Change::Modified));
    assert_eq!(change("docs"), Some(Change::Untracked));
    assert_eq!(change("top.txt"), Some(Change::Deleted));
    assert!(!changes.contains_key(root));
    assert_eq!(changes.len(), 8);
}

fn render(diff: &Diff) -> Vec<String> {
    diff.lines
        .iter()
        .map(|line| {
            let number = |n: Option<u32>| n.map_or("-".into(), |n| n.to_string());
            format!(
                "{:?} {} {} {}",
                line.kind,
                number(line.old_line),
                number(line.new_line),
                line.text
            )
        })
        .collect()
}

#[test]
fn parses_unified_diffs_with_line_numbers() {
    let output = "diff --git a/f.rs b/f.rs\r\n\
                  index 1111111..2222222 100644\r\n\
                  --- a/f.rs\r\n\
                  +++ b/f.rs\r\n\
                  @@ -9,3 +9,3 @@ fn main() {\r\n \tkeep\r\n-old\r\n+new\r\n context\r\n\
                  \\ No newline at end of file\r\n";
    assert_eq!(
        render(&Diff::parse(output)),
        [
            "Hunk - - @@ -9,3 +9,3 @@ fn main() {",
            "Context 9 9     keep",
            "Removed 10 - old",
            "Added - 10 new",
            "Context 11 11 context",
            "Note - - \\ No newline at end of file",
        ]
    );
}

#[test]
fn keeps_informative_headers_and_reads_combined_diffs() {
    let output = "diff --git a/a.bin b/a.bin\n\
                  new file mode 100644\n\
                  index 0000000..1111111\n\
                  Binary files /dev/null and b/a.bin differ\n\
                  diff --cc both.rs\n\
                  index 1,2..3\n\
                  @@@ -1,1 -1,1 +1,5 @@@\n\
                  ++<<<<<<< HEAD\n \
                  +ours\n\
                  - theirs\n  \
                  same\n";
    assert_eq!(
        render(&Diff::parse(output)),
        [
            "Meta - - new file mode 100644",
            "Meta - - Binary files /dev/null and b/a.bin differ",
            "Hunk - - @@@ -1,1 -1,1 +1,5 @@@",
            "Added - - <<<<<<< HEAD",
            "Added - - ours",
            "Removed - - theirs",
            "Context - - same",
        ]
    );
}

fn repository() -> (tempfile::TempDir, Repository) {
    let directory = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let output = git(directory.path()).args(args).output().unwrap();
        assert!(output.status.success(), "{}", failure(&output));
    };
    run(&["init", "-q", "--initial-branch=main"]);
    run(&["config", "user.name", "Hephaestus Tests"]);
    run(&["config", "user.email", "tests@example.invalid"]);
    run(&["config", "commit.gpgsign", "false"]);
    run(&["config", "core.hooksPath", ".no-hooks"]);
    run(&["config", "core.autocrlf", "false"]);
    let repository = Repository::discover(directory.path()).unwrap().unwrap();
    (directory, repository)
}

#[test]
fn ignored_paths_follow_gitignore_rules_and_collapse_ignored_directories() {
    let (directory, repository) = repository();
    let root = directory.path();
    fs::write(root.join(".gitignore"), "build/\n*.log\n!keep.log\n").unwrap();
    fs::create_dir_all(root.join("build/nested")).unwrap();
    fs::write(root.join("build/nested/output.txt"), "").unwrap();
    fs::write(root.join("error log.log"), "").unwrap();
    fs::write(root.join("keep.log"), "").unwrap();
    fs::write(root.join("tracked.log"), "").unwrap();
    repository.run(["add", "-f", "tracked.log"]).unwrap();

    let ignored = repository.ignored_paths().unwrap();
    assert!(ignored.contains(&repository.root().join("build")));
    assert!(ignored.contains(&repository.root().join("error log.log")));
    assert!(!ignored.contains(&repository.root().join("keep.log")));
    assert!(!ignored.contains(&repository.root().join("tracked.log")));
}

fn summary(repository: &Repository) -> Vec<String> {
    let status = repository.status().unwrap();
    status
        .files
        .iter()
        .map(|file| {
            let letter = |change: Option<Change>| change.map_or(".", Change::letter);
            format!(
                "{}{} {}",
                letter(file.staged),
                letter(file.unstaged),
                file.relative
            )
        })
        .collect()
}

fn file<'a>(status: &'a Status, relative: &str) -> &'a FileStatus {
    status
        .files
        .iter()
        .find(|file| file.relative == relative)
        .unwrap()
}

#[test]
fn stages_diffs_commits_and_unstages_in_a_real_repository() {
    let (directory, repository) = repository();
    let root = directory.path().canonicalize().unwrap();
    assert_eq!(repository.root(), root);
    fs::create_dir_all(root.join("src/deep")).unwrap();
    let nested = Repository::discover(&root.join("src/deep"))
        .unwrap()
        .unwrap();
    assert_eq!(nested.root(), root);
    let outside = tempfile::tempdir().unwrap();
    assert!(Repository::discover(outside.path()).unwrap().is_none());
    let created = Repository::init(outside.path()).unwrap();
    assert_eq!(created.root(), outside.path().canonicalize().unwrap());
    assert!(created.status().unwrap().files.is_empty());

    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("a[1].txt"), "literal\n").unwrap();
    fs::write(root.join("a1.txt"), "glob would match me\n").unwrap();
    assert_eq!(
        summary(&repository),
        [".U a1.txt", ".U a[1].txt", ".U src/main.rs"]
    );

    let status = repository.status().unwrap();
    assert_eq!(status.branch.label(), "main");
    let diff = repository
        .diff(file(&status, "src/main.rs"), false)
        .unwrap();
    assert_eq!(render(&diff).last().unwrap(), "Added - 1 fn main() {}");

    repository
        .stage(&[
            file(&status, "a[1].txt").clone(),
            file(&status, "src/main.rs").clone(),
        ])
        .unwrap();
    assert_eq!(
        summary(&repository),
        [".U a1.txt", "A. a[1].txt", "A. src/main.rs"]
    );

    let status = repository.status().unwrap();
    repository
        .unstage(&[file(&status, "a[1].txt").clone()])
        .unwrap();
    assert_eq!(
        summary(&repository),
        [".U a1.txt", ".U a[1].txt", "A. src/main.rs"]
    );

    let summary_line = repository.commit("Add main\n\nWith a body.\n").unwrap();
    assert!(summary_line.contains("Add main"), "{summary_line}");
    let status = repository.status().unwrap();
    assert!(status.branch.commit.is_some());
    assert_eq!(summary(&repository), [".U a1.txt", ".U a[1].txt"]);

    fs::write(root.join("src/main.rs"), "fn main() {\n    run();\n}\n").unwrap();
    let status = repository.status().unwrap();
    let main = file(&status, "src/main.rs").clone();
    assert_eq!((main.staged, main.unstaged), (None, Some(Change::Modified)));
    let diff = render(&repository.diff(&main, false).unwrap());
    assert_eq!(
        diff,
        [
            "Hunk - - @@ -1 +1,3 @@",
            "Removed 1 - fn main() {}",
            "Added - 1 fn main() {",
            "Added - 2     run();",
            "Added - 3 }",
        ]
    );
    repository.stage(&[main]).unwrap();
    let status = repository.status().unwrap();
    let main = file(&status, "src/main.rs");
    assert_eq!(repository.diff(main, true).unwrap().lines.len(), 5);
    assert!(repository.diff(main, false).unwrap().lines.is_empty());

    repository.commit("Run").unwrap();
    fs::rename(root.join("src/main.rs"), root.join("src/app.rs")).unwrap();
    let status = repository.status().unwrap();
    repository
        .stage(&[
            file(&status, "src/main.rs").clone(),
            file(&status, "src/app.rs").clone(),
        ])
        .unwrap();
    let status = repository.status().unwrap();
    let renamed = file(&status, "src/app.rs").clone();
    assert_eq!(renamed.staged, Some(Change::Renamed));
    assert_eq!(renamed.original.as_deref(), Some("src/main.rs"));
    let diff = render(&repository.diff(&renamed, true).unwrap());
    assert!(
        diff.contains(&"Meta - - rename from src/main.rs".to_string()),
        "{diff:?}"
    );
    repository.unstage(&[renamed]).unwrap();
    assert_eq!(
        summary(&repository),
        [
            ".U a1.txt",
            ".U a[1].txt",
            ".U src/app.rs",
            ".D src/main.rs"
        ]
    );

    let error = repository.commit("Nothing staged").unwrap_err();
    assert!(!error.to_string().is_empty());
}

#[test]
fn commits_and_reverts_only_selected_files() {
    let (directory, repository) = repository();
    let root = directory.path();
    fs::write(root.join("keep.txt"), "one\n").unwrap();
    fs::write(root.join("commit.txt"), "one\n").unwrap();
    let status = repository.status().unwrap();
    repository.stage(&status.files).unwrap();
    repository.commit("Initial").unwrap();

    fs::write(root.join("keep.txt"), "staged\n").unwrap();
    fs::write(root.join("commit.txt"), "selected\n").unwrap();
    fs::write(root.join("remove.txt"), "untracked\n").unwrap();
    let status = repository.status().unwrap();
    repository
        .stage(&[file(&status, "keep.txt").clone()])
        .unwrap();
    let status = repository.status().unwrap();
    let selected = file(&status, "commit.txt").clone();
    repository.stage(std::slice::from_ref(&selected)).unwrap();
    repository
        .commit_files(std::slice::from_ref(&selected), "Selected")
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("commit.txt")).unwrap(),
        "selected\n"
    );
    assert_eq!(summary(&repository), ["M. keep.txt", ".U remove.txt"]);

    fs::write(root.join("commit.txt"), "discard me\n").unwrap();
    let status = repository.status().unwrap();
    repository
        .revert(&[
            file(&status, "commit.txt").clone(),
            file(&status, "remove.txt").clone(),
        ])
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("commit.txt")).unwrap(),
        "selected\n"
    );
    assert!(!root.join("remove.txt").exists());
    assert_eq!(summary(&repository), ["M. keep.txt"]);
}

#[test]
fn clones_into_a_new_folder_named_after_the_repository() {
    let (source, repository) = repository();
    fs::write(source.path().join("README.md"), "hello\n").unwrap();
    let status = repository.status().unwrap();
    repository
        .stage(&[file(&status, "README.md").clone()])
        .unwrap();
    repository.commit("Initial").unwrap();

    let parent = tempfile::tempdir().unwrap();
    let url = source.path().to_string_lossy().into_owned();
    let name = clone_folder_name(&url).unwrap();
    let cloned = Repository::clone_remote(&format!(" {url} "), parent.path()).unwrap();
    let destination = parent.path().join(&name).canonicalize().unwrap();
    assert_eq!(cloned.root(), destination);
    let readme = fs::read_to_string(destination.join("README.md")).unwrap();
    assert_eq!(readme.trim_end(), "hello");
    assert!(cloned.status().unwrap().files.is_empty());

    let error = Repository::clone_remote(&url, parent.path()).unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
    let missing = source.path().join("nowhere").join("void");
    let error = Repository::clone_remote(&missing.to_string_lossy(), parent.path()).unwrap_err();
    assert!(!error.to_string().starts_with("Cloning"), "{error}");
    assert!(!parent.path().join("void").exists());
    assert!(Repository::clone_remote("  ", parent.path()).is_err());
}

fn commit_file(repository: &Repository, relative: &str, text: &str, message: &str) {
    fs::write(repository.root().join(relative), text).unwrap();
    let status = repository.status().unwrap();
    repository
        .stage(&[file(&status, relative).clone()])
        .unwrap();
    repository.commit(message).unwrap();
}

fn configure(root: &Path) {
    for args in [
        ["config", "user.name", "Hephaestus Tests"],
        ["config", "user.email", "tests@example.invalid"],
        ["config", "commit.gpgsign", "false"],
        ["config", "core.hooksPath", ".no-hooks"],
        ["config", "pull.rebase", "false"],
    ] {
        let output = git(root).args(args).output().unwrap();
        assert!(output.status.success(), "{}", failure(&output));
    }
}

#[test]
fn fetches_pulls_and_pushes_through_a_remote() {
    let (local, repository) = repository();
    configure(local.path());
    commit_file(&repository, "README.md", "one\n", "One");
    for error in [repository.fetch(), repository.pull(), repository.push()] {
        assert_eq!(
            error.unwrap_err().to_string(),
            "This repository has no remotes"
        );
    }

    let remote = tempfile::tempdir().unwrap();
    let output = git(remote.path())
        .args(["init", "-q", "--bare", "--initial-branch=main"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", failure(&output));
    let url = remote.path().to_string_lossy().into_owned();
    repository.run(["remote", "add", "origin", &url]).unwrap();
    assert_eq!(repository.push().unwrap(), "Pushed main to origin");
    let branch = repository.status().unwrap().branch;
    assert_eq!(branch.upstream.as_deref(), Some("origin/main"));
    assert_eq!(
        repository.push().unwrap(),
        "origin/main is already up to date"
    );

    let parent = tempfile::tempdir().unwrap();
    let other = Repository::clone_remote(&url, parent.path()).unwrap();
    configure(other.root());
    commit_file(&other, "README.md", "two\n", "Two");
    commit_file(&other, "notes.txt", "notes\n", "Notes");
    assert_eq!(other.push().unwrap(), "Pushed 2 commits to origin/main");

    assert_eq!(repository.fetch().unwrap(), "Fetched from all remotes");
    let branch = repository.status().unwrap().branch;
    assert_eq!((branch.ahead, branch.behind), (0, 2));
    assert_eq!(repository.pull().unwrap(), "Pulled 2 commits");
    let readme = fs::read_to_string(repository.root().join("README.md")).unwrap();
    assert_eq!(readme, "two\n");
    assert_eq!(repository.pull().unwrap(), "Already up to date");

    commit_file(&other, "notes.txt", "remote\n", "Remote");
    other.push().unwrap();
    commit_file(&repository, "README.md", "local\n", "Local");
    let error = repository.push().unwrap_err();
    assert_eq!(
        error.to_string(),
        "The remote has commits you don't have yet — pull first"
    );
    assert_eq!(repository.pull().unwrap(), "Pulled 2 commits");
    assert_eq!(
        repository.push().unwrap(),
        "Pushed 2 commits to origin/main"
    );

    repository.run(["checkout", "-q", "--detach"]).unwrap();
    let error = repository.push().unwrap_err();
    assert_eq!(error.to_string(), "Check out a branch before pushing");
}
