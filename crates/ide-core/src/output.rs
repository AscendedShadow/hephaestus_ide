use std::path::{Path, PathBuf};

use crate::search::Match;

pub fn locations(lines: &[String], directory: &Path) -> Vec<Match> {
    lines
        .iter()
        .filter_map(|line| parse_line(line, directory))
        .collect()
}

fn parse_line(line: &str, directory: &Path) -> Option<Match> {
    let line = line.trim().trim_start_matches("-->").trim();
    // Compiler messages may append ": error: ..." after the location.
    let location = line.split(": ").next()?;
    let (path_line, column) = location.rsplit_once(':')?;
    let column: usize = column.parse().ok()?;
    let (path, row) = path_line.rsplit_once(':')?;
    let row: usize = row.parse().ok()?;
    if path.is_empty() || row == 0 || column == 0 {
        return None;
    }
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        directory.join(path)
    };
    Some(Match {
        path: path.canonicalize().unwrap_or(path),
        line: row,
        column,
        preview: line.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_and_clang_locations() {
        let root = Path::new("/project");
        let hits = locations(
            &[
                " --> src/main.rs:12:5".into(),
                "src/lib.c:4:8: error: missing".into(),
                "warning: plain text".into(),
            ],
            root,
        );
        assert_eq!((hits[0].line, hits[0].column), (12, 5));
        assert_eq!((hits[1].line, hits[1].column), (4, 8));
        assert_eq!(hits.len(), 2);
    }
}
