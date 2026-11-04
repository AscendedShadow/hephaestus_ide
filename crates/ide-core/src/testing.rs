//! Test discovery and results (IDE-018): a provider-neutral model and a Cargo provider.
//!
//! Identity: a test is identified by its package, target (kind + name) and full test
//! path, so equally named tests in different packages or targets stay distinct.
//! Discovery builds the test binaries (`cargo test --no-run --message-format=json`) and
//! lists each binary's tests with `--list`; running invokes the exact binary with
//! `--exact` filters, parsing libtest's output as it streams.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Target {
    pub package: String,
    /// `lib`, `bin`, `test` (integration test), `example`, `bench`.
    pub kind: String,
    pub name: String,
}

impl Target {
    pub fn label(&self) -> String {
        match self.kind.as_str() {
            "lib" => format!("{} (lib)", self.package),
            kind => format!("{} ({kind} {})", self.package, self.name),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TestId {
    pub target: Target,
    /// `module::submodule::test_name`.
    pub path: String,
}

impl TestId {
    pub fn name(&self) -> &str {
        self.path.rsplit("::").next().unwrap_or(&self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestState {
    Unknown,
    Queued,
    Running,
    Passed,
    Failed,
    Skipped,
}

impl TestState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResult {
    pub state: TestState,
    /// Captured output of a failed test.
    pub output: String,
    /// Where it failed (from a panic message), relative paths resolved.
    pub failure: Option<(PathBuf, usize, usize)>,
    pub duration: Option<Duration>,
}

impl Default for TestResult {
    fn default() -> Self {
        Self {
            state: TestState::Unknown,
            output: String::new(),
            failure: None,
            duration: None,
        }
    }
}

/// A built test binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Executable {
    pub target: Target,
    pub path: PathBuf,
    /// The package root (for resolving relative source paths).
    pub root: PathBuf,
    /// Source file of the target (`src/lib.rs`, `tests/it.rs`).
    pub source: PathBuf,
}

/// Parse `cargo test --no-run --message-format=json` output into test executables.
pub fn parse_artifacts(json_lines: &str) -> Vec<Executable> {
    let mut executables = Vec::new();
    for line in json_lines.lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if message["reason"] != "compiler-artifact" || message["profile"]["test"] != true {
            continue;
        }
        let Some(path) = message["executable"].as_str() else {
            continue;
        };
        let manifest = message["manifest_path"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_default();
        let package = message["package_id"]
            .as_str()
            .map(package_name)
            .unwrap_or_default();
        let kind = message["target"]["kind"][0]
            .as_str()
            .unwrap_or("lib")
            .to_string();
        let kind = if kind == "proc-macro" || kind.ends_with("lib") {
            "lib".to_string()
        } else {
            kind
        };
        executables.push(Executable {
            target: Target {
                package,
                kind,
                name: message["target"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            },
            path: PathBuf::from(path),
            root: manifest.parent().map(Path::to_path_buf).unwrap_or_default(),
            source: message["target"]["src_path"]
                .as_str()
                .map(PathBuf::from)
                .unwrap_or_default(),
        });
    }
    executables
}

/// Package name from a package id (`path+file:///…/crate#name@0.1.0`, or the older
/// `name 0.1.0 (path+file:///…)` form).
fn package_name(id: &str) -> String {
    if let Some((_, fragment)) = id.rsplit_once('#') {
        let fragment = fragment.split('@').next().unwrap_or(fragment);
        // `#0.1.0` (no name) means the directory name is the package name.
        if fragment.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return id
                .split('#')
                .next()
                .and_then(|url| url.rsplit('/').next())
                .unwrap_or(fragment)
                .to_string();
        }
        return fragment.to_string();
    }
    id.split(' ').next().unwrap_or(id).to_string()
}

/// Parse `<binary> --list --format terse` output.
pub fn parse_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .map(str::to_string)
        .collect()
}

/// Build test binaries and list their tests. Blocking; run it in the background.
pub fn discover(workspace: &Path) -> Result<Vec<(Executable, Vec<String>)>, String> {
    let output = Command::new("cargo")
        .current_dir(workspace)
        .args([
            "test",
            "--workspace",
            "--all-targets",
            "--no-run",
            "--message-format=json",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cargo: {error}"))?;
    let executables = parse_artifacts(&String::from_utf8_lossy(&output.stdout));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .filter(|line| line.starts_with("error"))
            .take(3)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("Building tests failed: {detail}"));
    }
    let mut found = Vec::new();
    for executable in executables {
        let listed = Command::new(&executable.path)
            .current_dir(&executable.root)
            .args(["--list", "--format", "terse"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("{}: {error}", executable.path.display()))?;
        found.push((
            executable,
            parse_list(&String::from_utf8_lossy(&listed.stdout)),
        ));
    }
    Ok(found)
}

/// Locate a test function in its target's sources (`fn name(`), as `(file, line)`.
pub fn locate(root: &Path, source: &Path, id: &TestId) -> Option<(PathBuf, usize)> {
    let pattern = format!("fn {}(", id.name());
    let mut candidates = vec![source.to_path_buf()];
    for directory in ["src", "tests"] {
        let mut pending = vec![root.join(directory)];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    candidates.push(path);
                }
            }
        }
    }
    // Prefer a file whose name matches a module in the test path.
    let modules: Vec<&str> = id.path.split("::").collect();
    candidates.sort_by_key(|path| {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        !modules.contains(&stem.as_str())
    });
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(line) = text.lines().position(|line| line.contains(&pattern)) {
            return Some((path, line + 1));
        }
    }
    None
}

/// Incremental parser for libtest's human output.
#[derive(Default)]
pub struct LibtestParser {
    partial: String,
    capture: Option<(String, String)>,
    /// Results as they complete: (test path, state).
    pub results: Vec<(String, TestState)>,
    /// Captured failure output per test path.
    pub outputs: HashMap<String, String>,
}

impl LibtestParser {
    pub fn feed(&mut self, chunk: &str) {
        self.partial.push_str(chunk);
        while let Some(end) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=end).collect();
            self.line(crate::problems::strip_ansi(line.trim_end_matches(['\n', '\r'])).as_str());
        }
    }

    fn line(&mut self, line: &str) {
        if let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|rest| rest.strip_suffix(" stdout ----"))
        {
            self.finish_capture();
            self.capture = Some((name.to_string(), String::new()));
            return;
        }
        if line == "failures:" || line.starts_with("test result:") {
            self.finish_capture();
        }
        if let Some((_, text)) = &mut self.capture {
            text.push_str(line);
            text.push('\n');
            return;
        }
        let Some(rest) = line.strip_prefix("test ") else {
            return;
        };
        let Some((name, outcome)) = rest.rsplit_once(" ... ") else {
            return;
        };
        let state = match outcome.trim() {
            "ok" => TestState::Passed,
            "FAILED" => TestState::Failed,
            outcome if outcome.starts_with("ignored") => TestState::Skipped,
            _ => return,
        };
        self.results.push((name.to_string(), state));
    }

    fn finish_capture(&mut self) {
        if let Some((name, text)) = self.capture.take() {
            self.outputs.insert(name, text);
        }
    }

    pub fn finish(&mut self) {
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            self.line(&line);
        }
        self.finish_capture();
    }
}

/// The `panicked at path:line:col` location in a failure output.
pub fn failure_location(output: &str, root: &Path) -> Option<(PathBuf, usize, usize)> {
    let at = output.find("panicked at ")?;
    let rest = &output[at + "panicked at ".len()..];
    let location = rest.split([':', '\n']).take(3).collect::<Vec<_>>();
    let [path, line, column] = location[..] else {
        return None;
    };
    let path = Path::new(path.trim());
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    Some((
        path,
        line.trim().parse().ok()?,
        column.trim().trim_end_matches(':').parse().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifacts_lists_and_results_parse() {
        let json = r#"{"reason":"compiler-artifact","package_id":"path+file:///w/crates/core#ide-core@0.1.0","manifest_path":"/w/crates/core/Cargo.toml","target":{"kind":["lib"],"name":"ide_core","src_path":"/w/crates/core/src/lib.rs"},"profile":{"test":true},"executable":"/w/target/debug/deps/ide_core-1"}
{"reason":"compiler-artifact","package_id":"app 0.1.0 (path+file:///w/app)","manifest_path":"/w/app/Cargo.toml","target":{"kind":["test"],"name":"it","src_path":"/w/app/tests/it.rs"},"profile":{"test":true},"executable":"/w/target/debug/deps/it-2"}
{"reason":"compiler-artifact","package_id":"x","target":{"kind":["lib"],"name":"x"},"profile":{"test":false},"executable":null}
{"reason":"build-finished","success":true}"#;
        let executables = parse_artifacts(json);
        assert_eq!(executables.len(), 2);
        assert_eq!(
            executables[0].target,
            Target {
                package: "ide-core".into(),
                kind: "lib".into(),
                name: "ide_core".into()
            }
        );
        assert_eq!(executables[0].root, PathBuf::from("/w/crates/core"));
        assert_eq!(executables[1].target.package, "app");
        assert_eq!(executables[1].target.label(), "app (test it)");
        assert_eq!(
            package_name("path+file:///w/crates/terminal#0.1.0"),
            "terminal"
        );

        assert_eq!(
            parse_list("tests::a: test\nb::c: test\n2 tests, 0 benchmarks\nbench_x: benchmark\n"),
            ["tests::a", "b::c"]
        );

        let mut parser = LibtestParser::default();
        let output = "running 3 tests\ntest tests::a ... ok\ntest tests::b ... FAILED\ntest tests::c ... ignored, slow\n\nfailures:\n\n---- tests::b stdout ----\n\nthread 'tests::b' panicked at src/lib.rs:12:9:\nassertion failed\n\n\nfailures:\n    tests::b\n\ntest result: FAILED. 1 passed; 1 failed; 1 ignored\n";
        // Feed in awkward chunks.
        for chunk in output.as_bytes().chunks(7) {
            parser.feed(std::str::from_utf8(chunk).unwrap());
        }
        parser.finish();
        assert_eq!(
            parser.results,
            vec![
                ("tests::a".to_string(), TestState::Passed),
                ("tests::b".to_string(), TestState::Failed),
                ("tests::c".to_string(), TestState::Skipped)
            ]
        );
        let failure = &parser.outputs["tests::b"];
        assert!(failure.contains("assertion failed"));
        assert_eq!(
            failure_location(failure, Path::new("/w/core")),
            Some((PathBuf::from("/w/core/src/lib.rs"), 12, 9))
        );
    }

    #[test]
    fn locates_test_functions_preferring_matching_modules() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "mod parser;\n#[test]\nfn works() {}\n",
        )
        .unwrap();
        std::fs::write(root.join("src/parser.rs"), "\n\n#[test]\nfn works() {}\n").unwrap();
        let id = |path: &str| TestId {
            target: Target {
                package: "p".into(),
                kind: "lib".into(),
                name: "p".into(),
            },
            path: path.into(),
        };
        assert_eq!(
            locate(root, &root.join("src/lib.rs"), &id("parser::works")),
            Some((root.join("src/parser.rs"), 4))
        );
        assert_eq!(
            locate(root, &root.join("src/lib.rs"), &id("works")),
            Some((root.join("src/lib.rs"), 3))
        );
        assert_eq!(locate(root, &root.join("src/lib.rs"), &id("missing")), None);
    }
}
