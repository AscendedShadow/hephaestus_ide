//! Minimal glob matching shared by search scopes and file associations.
//!
//! Supported syntax:
//! - `*` matches any run of characters except a path separator,
//! - `**` matches any run of characters including separators (`**/` may match nothing),
//! - `?` matches one character except a separator,
//! - `[abc]`, `[a-z]` and `[!abc]` match character classes,
//! - `{a,b}` matches either alternative.
//!
//! A pattern without a separator is matched against the file name only, so `*.rs`
//! matches `src/main.rs`. Patterns containing `/` are matched against the whole
//! (relative, `/`-separated) path.

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glob {
    pattern: String,
    alternatives: Vec<Vec<char>>,
    name_only: bool,
}

impl Glob {
    pub fn new(pattern: &str) -> Result<Self, String> {
        let pattern = pattern.trim().replace('\\', "/");
        if pattern.is_empty() {
            return Err("empty glob pattern".into());
        }
        let alternatives = expand_braces(&pattern)?;
        for alternative in &alternatives {
            validate_classes(alternative)?;
        }
        let trimmed = pattern.trim_start_matches("./");
        let name_only = !trimmed.trim_end_matches('/').contains('/');
        Ok(Self {
            alternatives: expand_braces(trimmed)?
                .into_iter()
                .map(|alternative| {
                    let alternative = alternative.trim_end_matches('/').to_string();
                    alternative.chars().collect()
                })
                .collect(),
            pattern,
            name_only,
        })
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Match a relative path. File-name-only patterns also match any ancestor directory
    /// name, so `node_modules` excludes everything below such a folder.
    pub fn matches(&self, relative: &Path) -> bool {
        let text = relative.to_string_lossy().replace('\\', "/");
        let text = text.trim_start_matches("./");
        if self.name_only {
            return text
                .split('/')
                .filter(|segment| !segment.is_empty())
                .any(|segment| self.matches_text(segment));
        }
        if self.matches_text(text) {
            return true;
        }
        // A directory pattern also matches everything inside the directory.
        let mut prefix = String::new();
        for segment in text.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            if prefix.len() < text.len() && self.matches_text(&prefix) {
                return true;
            }
        }
        false
    }

    /// Match only the file name of `path` (ignores directories).
    pub fn matches_name(&self, name: &str) -> bool {
        self.matches_text(name)
    }

    fn matches_text(&self, text: &str) -> bool {
        let text: Vec<char> = text.chars().collect();
        self.alternatives
            .iter()
            .any(|pattern| match_from(pattern, &text))
    }
}

fn expand_braces(pattern: &str) -> Result<Vec<String>, String> {
    let Some(open) = pattern.find('{') else {
        if pattern.contains('}') {
            return Err(format!("unbalanced '}}' in {pattern:?}"));
        }
        return Ok(vec![pattern.to_string()]);
    };
    let mut depth = 0;
    let mut close = None;
    for (ix, character) in pattern[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + ix);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close.ok_or_else(|| format!("unbalanced '{{' in {pattern:?}"))?;
    let (prefix, body, suffix) = (
        &pattern[..open],
        &pattern[open + 1..close],
        &pattern[close + 1..],
    );
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (ix, character) in body.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&body[start..ix]);
                start = ix + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    let mut expanded = Vec::new();
    for part in parts {
        for tail in expand_braces(&format!("{part}{suffix}"))? {
            expanded.push(format!("{prefix}{tail}"));
        }
    }
    Ok(expanded)
}

fn validate_classes(pattern: &str) -> Result<(), String> {
    let mut chars = pattern.chars();
    while let Some(character) = chars.next() {
        if character == '[' && !chars.by_ref().any(|c| c == ']') {
            return Err(format!("unclosed '[' in {pattern:?}"));
        }
    }
    Ok(())
}

fn match_from(pattern: &[char], text: &[char]) -> bool {
    let Some((&first, rest)) = pattern.split_first() else {
        return text.is_empty();
    };
    match first {
        '*' if rest.first() == Some(&'*') => {
            let mut rest = &rest[1..];
            // `**/` may match zero directories.
            if rest.first() == Some(&'/') {
                rest = &rest[1..];
                if match_from(rest, text) {
                    return true;
                }
            }
            (0..=text.len()).any(|skip| match_from(rest, &text[skip..]))
        }
        '*' => {
            for skip in 0..=text.len() {
                if match_from(rest, &text[skip..]) {
                    return true;
                }
                if text.get(skip) == Some(&'/') {
                    break;
                }
            }
            false
        }
        '?' => text
            .split_first()
            .is_some_and(|(&c, tail)| c != '/' && match_from(rest, tail)),
        '[' => {
            let Some(close) = rest.iter().position(|&c| c == ']') else {
                return false;
            };
            let (class, rest) = (&rest[..close], &rest[close + 1..]);
            let Some((&c, tail)) = text.split_first() else {
                return false;
            };
            let (negated, class) = match class.split_first() {
                Some((&('!' | '^'), class)) => (true, class),
                _ => (false, class),
            };
            let mut found = false;
            let mut ix = 0;
            while ix < class.len() {
                if ix + 2 < class.len() && class[ix + 1] == '-' {
                    found |= class[ix] <= c && c <= class[ix + 2];
                    ix += 3;
                } else {
                    found |= class[ix] == c;
                    ix += 1;
                }
            }
            found != negated && c != '/' && match_from(rest, tail)
        }
        literal => text
            .split_first()
            .is_some_and(|(&c, tail)| c == literal && match_from(rest, tail)),
    }
}

/// Parse a comma-separated list of glob patterns (as typed in a search field).
pub fn parse_list(list: &str) -> Result<Vec<Glob>, String> {
    let mut globs = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (ix, character) in list.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                if !list[start..ix].trim().is_empty() {
                    globs.push(Glob::new(&list[start..ix])?);
                }
                start = ix + 1;
            }
            _ => {}
        }
    }
    if !list[start..].trim().is_empty() {
        globs.push(Glob::new(&list[start..])?);
    }
    Ok(globs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(pattern: &str) -> Glob {
        Glob::new(pattern).unwrap()
    }

    #[test]
    fn name_patterns_match_any_directory_level() {
        assert!(glob("*.rs").matches(Path::new("src/main.rs")));
        assert!(!glob("*.rs").matches(Path::new("src/main.rsx")));
        assert!(glob("node_modules").matches(Path::new("web/node_modules/a/index.js")));
        assert!(glob("Dockerfile*").matches(Path::new("Dockerfile.dev")));
        assert!(glob("*.{ts,tsx}").matches(Path::new("a/b.tsx")));
        assert!(glob("?.c").matches(Path::new("x/a.c")));
        assert!(glob("[a-c]x.txt").matches(Path::new("bx.txt")));
        assert!(!glob("[!a-c]x.txt").matches(Path::new("bx.txt")));
        assert!(glob("ünï*.md").matches(Path::new("docs/ünïcode.md")));
    }

    #[test]
    fn path_patterns_anchor_to_the_relative_path() {
        assert!(glob("src/**/*.rs").matches(Path::new("src/main.rs")));
        assert!(glob("src/**/*.rs").matches(Path::new("src/a/b/c.rs")));
        assert!(!glob("src/*.rs").matches(Path::new("src/a/c.rs")));
        assert!(glob("**/tests/**").matches(Path::new("crate/tests/it.rs")));
        assert!(glob("docs/").matches(Path::new("docs/guide/intro.md")));
        assert!(glob("./target/debug").matches(Path::new("target/debug/app")));
        assert!(!glob("src/*.rs").matches(Path::new("other/src/main.rs")));
    }

    #[test]
    fn lists_and_errors() {
        let globs = parse_list("*.rs, *.{md,txt} ,").unwrap();
        assert_eq!(globs.len(), 2);
        assert!(globs[1].matches(Path::new("README.txt")));
        assert!(Glob::new("{a,b").is_err());
        assert!(Glob::new("[abc").is_err());
        assert!(Glob::new(" ").is_err());
    }
}
