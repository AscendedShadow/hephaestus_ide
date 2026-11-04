//! Problems from build output (IDE-020).
//!
//! [`OutputParser`] consumes raw process output incrementally: chunk boundaries may
//! split lines or UTF-8 sequences, and ANSI escape sequences are stripped. Recognized
//! formats:
//!
//! - rustc / cargo: `error[E0425]: message` followed by `  --> path:line:col`,
//! - GCC / Clang / Go / TypeScript (`tsc --pretty false`): `path:line:col: severity: message`,
//! - MSVC: `path(line,col): error C2065: message`,
//! - Windows drive-letter paths in all of the above.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "info",
            Self::Hint => "hint",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "error" | "fatal error" | "fatal" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "note" | "info" | "information" => Some(Self::Information),
            "help" | "hint" => Some(Self::Hint),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub path: PathBuf,
    /// One-based line and column.
    pub line: usize,
    pub column: usize,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    pub source: String,
}

/// Remove ANSI escape sequences (CSI and OSC) and carriage returns.
pub fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\u{1b}' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{7}'
                            || (next == '\u{1b}' && chars.next_if_eq(&'\\').is_some())
                        {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            },
            '\r' => {}
            other => result.push(other),
        }
    }
    result
}

pub struct OutputParser {
    directory: PathBuf,
    source: String,
    bytes: Vec<u8>,
    line: String,
    /// A rustc header waiting for its `-->` location line.
    pending: Option<(Severity, Option<String>, String)>,
}

impl OutputParser {
    pub fn new(directory: &Path, source: impl Into<String>) -> Self {
        Self {
            directory: directory.to_path_buf(),
            source: source.into(),
            bytes: Vec::new(),
            line: String::new(),
            pending: None,
        }
    }

    /// Feed raw output; returns problems completed by this chunk.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Problem> {
        self.bytes.extend_from_slice(chunk);
        // Decode the longest valid UTF-8 prefix; keep a split sequence for later.
        let valid = match std::str::from_utf8(&self.bytes) {
            Ok(text) => text.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(error) => error.valid_up_to() + error.error_len().unwrap_or(1),
        };
        let decoded = String::from_utf8_lossy(&self.bytes[..valid]).into_owned();
        self.bytes.drain(..valid);
        let mut problems = Vec::new();
        for character in decoded.chars() {
            if character == '\n' {
                let line = std::mem::take(&mut self.line);
                problems.extend(self.line(&strip_ansi(&line)));
            } else {
                self.line.push(character);
            }
        }
        problems
    }

    /// Flush a final unterminated line.
    pub fn finish(&mut self) -> Vec<Problem> {
        let line = std::mem::take(&mut self.line);
        let mut problems = self.line(&strip_ansi(&line));
        self.pending = None;
        problems.retain(|problem| !problem.message.is_empty());
        problems
    }

    fn resolve(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path.trim());
        let path = if is_absolute(&path) {
            path
        } else {
            self.directory.join(path)
        };
        path.canonicalize().unwrap_or(path)
    }

    fn line(&mut self, line: &str) -> Vec<Problem> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        // rustc location line.
        if let Some(location) = trimmed
            .strip_prefix("--> ")
            .or_else(|| trimmed.strip_prefix("::: "))
        {
            if let Some((severity, code, message)) = self.pending.take()
                && let Some((path, line, column)) = split_location(location)
            {
                return vec![Problem {
                    path: self.resolve(path),
                    line,
                    column,
                    severity,
                    code,
                    message,
                    source: self.source.clone(),
                }];
            }
            return Vec::new();
        }
        // rustc header: `error[E0425]: message` / `warning: message`.
        if let Some((head, message)) = trimmed.split_once(": ") {
            let (kind, code) = match head.split_once('[') {
                Some((kind, code)) => (kind, code.strip_suffix(']').map(str::to_string)),
                None => (head, None),
            };
            if let Some(severity) = Severity::parse(kind)
                && !kind.contains([':', '/', '\\', '('])
            {
                // Summary lines such as "warning: 3 warnings emitted" have no location.
                self.pending = Some((severity, code, message.trim().to_string()));
                return Vec::new();
            }
        }
        if let Some(problem) = self.msvc(trimmed).or_else(|| self.gcc(trimmed)) {
            self.pending = None;
            return vec![problem];
        }
        Vec::new()
    }

    /// `path:line:col: severity: message` (column optional).
    fn gcc(&self, line: &str) -> Option<Problem> {
        let (location, rest) = split_after_location(line)?;
        let (path, row, column) = split_location(location)?;
        let (severity, message) = match rest.split_once(':') {
            Some((kind, message)) if Severity::parse(kind).is_some() => {
                (Severity::parse(kind)?, message.trim().to_string())
            }
            _ => return None,
        };
        let (code, message) = trailing_code(&message);
        Some(Problem {
            path: self.resolve(path),
            line: row,
            column,
            severity,
            code,
            message,
            source: self.source.clone(),
        })
    }

    /// `path(line,col): error C2065: message`.
    fn msvc(&self, line: &str) -> Option<Problem> {
        let open = line.find('(')?;
        let close = open + line[open..].find("):")?;
        let (row, column) = match line[open + 1..close].split_once(',') {
            Some((row, column)) => (row.trim().parse().ok()?, column.trim().parse().ok()?),
            None => (line[open + 1..close].trim().parse().ok()?, 1),
        };
        let rest = line[close + 2..].trim();
        let (kind, message) = rest.split_once(':')?;
        let mut words = kind.split_whitespace();
        let severity = Severity::parse(words.next()?)?;
        let code = words.next().map(str::to_string);
        Some(Problem {
            path: self.resolve(&line[..open]),
            line: row,
            column,
            severity,
            code,
            message: message.trim().to_string(),
            source: self.source.clone(),
        })
    }
}

fn is_absolute(path: &Path) -> bool {
    let text = path.to_string_lossy();
    path.is_absolute()
        || (text.len() > 2
            && text.as_bytes()[1] == b':'
            && text.as_bytes()[0].is_ascii_alphabetic())
}

/// Split `path:line:col` (or `path:line`), allowing a Windows drive letter.
fn split_location(location: &str) -> Option<(&str, usize, usize)> {
    let location = location.trim();
    let drive = location.len() > 2
        && location.as_bytes()[1] == b':'
        && location.as_bytes()[0].is_ascii_alphabetic();
    let body_start = if drive { 2 } else { 0 };
    let mut parts = location[body_start..].rsplitn(3, ':');
    let last = parts.next()?;
    let middle = parts.next()?;
    match parts.next() {
        Some(path) => {
            let line = middle.parse().ok()?;
            let column = last.parse().ok()?;
            let path = &location[..body_start + path.len()];
            (!path.is_empty() && line > 0).then_some((path, line, column))
        }
        None => {
            let line = last.parse().ok()?;
            let path = &location[..body_start + middle.len()];
            (!path.is_empty() && line > 0).then_some((path, line, 1))
        }
    }
}

/// For `path:line:col: rest`, return (`path:line:col`, `rest`).
fn split_after_location(line: &str) -> Option<(&str, &str)> {
    let mut search = 0;
    while let Some(ix) = line[search..].find(": ") {
        let at = search + ix;
        if split_location(&line[..at]).is_some() {
            return Some((&line[..at], line[at + 2..].trim()));
        }
        search = at + 2;
    }
    None
}

/// GCC appends `[-Wflag]`; TypeScript prefixes `TS1234:`.
fn trailing_code(message: &str) -> (Option<String>, String) {
    if let Some(start) = message.rfind(" [")
        && message.ends_with(']')
    {
        return (
            Some(message[start + 2..message.len() - 1].to_string()),
            message[..start].to_string(),
        );
    }
    if let Some((code, rest)) = message.split_once(": ")
        && code.starts_with("TS")
        && code[2..].chars().all(|c| c.is_ascii_digit())
    {
        return (Some(code.to_string()), rest.to_string());
    }
    (None, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&[u8]]) -> Vec<Problem> {
        let mut parser = OutputParser::new(Path::new("/w"), "build");
        let mut problems = Vec::new();
        for chunk in chunks {
            problems.extend(parser.feed(chunk));
        }
        problems.extend(parser.finish());
        problems
    }

    #[test]
    fn rustc_output_across_chunks_with_ansi_codes() {
        let output = "\u{1b}[0m\u{1b}[1m\u{1b}[38;5;9merror[E0425]\u{1b}[0m: cannot find value `x` in this scope\r\n  \u{1b}[38;5;12m--> \u{1b}[0msrc/main.rs:3:5\n   |\nwarning: unused variable: `ü`\n --> src/lib.rs:10:9\nwarning: 2 warnings emitted\n";
        let bytes = output.as_bytes();
        // Split inside the escape sequence and inside the multi-byte `ü`.
        let split_escape = output.find("[38;5;12m").unwrap() + 3;
        let split_unicode = output.find('ü').unwrap() + 1;
        let problems = parse(&[
            &bytes[..split_escape],
            &bytes[split_escape..split_unicode],
            &bytes[split_unicode..],
        ]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(problems[0].path, PathBuf::from("/w/src/main.rs"));
        assert_eq!((problems[0].line, problems[0].column), (3, 5));
        assert_eq!(problems[0].severity, Severity::Error);
        assert_eq!(problems[0].code.as_deref(), Some("E0425"));
        assert_eq!(problems[0].message, "cannot find value `x` in this scope");
        assert_eq!(problems[1].message, "unused variable: `ü`");
        assert_eq!(problems[1].severity, Severity::Warning);
    }

    #[test]
    fn gcc_msvc_and_windows_paths() {
        let problems = parse(&[
            b"src/a.c:4:8: error: 'x' undeclared [-Werror]\n",
            b"C:\\proj\\b.c:7:2: warning: unused\n",
            b"C:\\proj\\c.cpp(12,5): error C2065: 'y': undeclared identifier\n",
            b"main.go:9: note: something\n",
            b"src/app.ts:1:7: error: TS2322: Type mismatch\n",
            b"plain text: not a location\n",
        ]);
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert_eq!(problems[0].code.as_deref(), Some("-Werror"));
        assert_eq!(problems[0].message, "'x' undeclared");
        assert_eq!(problems[1].path, PathBuf::from("C:\\proj\\b.c"));
        assert_eq!((problems[1].line, problems[1].column), (7, 2));
        assert_eq!(problems[2].path, PathBuf::from("C:\\proj\\c.cpp"));
        assert_eq!(problems[2].code.as_deref(), Some("C2065"));
        assert_eq!((problems[2].line, problems[2].column), (12, 5));
        assert_eq!(problems[3].severity, Severity::Information);
        assert_eq!(problems[3].column, 1);
        assert_eq!(problems[4].code.as_deref(), Some("TS2322"));
    }
}
