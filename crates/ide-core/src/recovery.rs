//! Crash recovery snapshots for unsaved buffers (IDE-015).
//!
//! Each modified buffer is stored as one JSON-like record file in a recovery folder
//! outside the project, written atomically (temporary file + rename). Records carry a
//! format version; unreadable or corrupt records are skipped (and reported) without
//! affecting the others.

use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;

pub const FORMAT_VERSION: u32 = 1;
const EXTENSION: &str = "recovery";

/// One recovered buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Stable key of the buffer within the session (file name of the record).
    pub key: String,
    /// The file the buffer belongs to; `None` for untitled buffers.
    pub path: Option<PathBuf>,
    pub name: String,
    pub text: String,
    /// Hash of the file on disk when the buffer was last loaded/saved, to detect files
    /// changed since the snapshot.
    pub disk_hash: Option<u64>,
    /// Tab order and cursor (line, UTF-16 column).
    pub order: usize,
    pub cursor: (u32, u32),
    pub language: String,
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn unescape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some(other) => result.push(other),
                None => {}
            }
        } else {
            result.push(character);
        }
    }
    result
}

impl Entry {
    /// Serialize as a simple line-oriented record: header fields, then the text.
    pub fn encode(&self) -> String {
        let mut record = format!("hephaestus-recovery {FORMAT_VERSION}\n");
        record.push_str(&format!("key {}\n", escape(&self.key)));
        if let Some(path) = &self.path {
            record.push_str(&format!("path {}\n", escape(&path.to_string_lossy())));
        }
        record.push_str(&format!("name {}\n", escape(&self.name)));
        if let Some(hash) = self.disk_hash {
            record.push_str(&format!("disk {hash}\n"));
        }
        record.push_str(&format!("order {}\n", self.order));
        record.push_str(&format!("cursor {} {}\n", self.cursor.0, self.cursor.1));
        record.push_str(&format!("language {}\n", escape(&self.language)));
        record.push_str(&format!("length {}\n", self.text.len()));
        record.push('\n');
        record.push_str(&self.text);
        record
    }

    pub fn decode(record: &str) -> Result<Self, String> {
        let (header, text) = record.split_once("\n\n").ok_or("missing record body")?;
        let mut lines = header.lines();
        let version = lines
            .next()
            .and_then(|line| line.strip_prefix("hephaestus-recovery "))
            .ok_or("not a recovery record")?;
        if version.trim() != FORMAT_VERSION.to_string() {
            return Err(format!("unsupported recovery format {version}"));
        }
        let mut entry = Entry {
            key: String::new(),
            path: None,
            name: String::new(),
            text: String::new(),
            disk_hash: None,
            order: 0,
            cursor: (0, 0),
            language: String::new(),
        };
        let mut length = None;
        for line in lines {
            let (field, value) = line.split_once(' ').unwrap_or((line, ""));
            match field {
                "key" => entry.key = unescape(value),
                "path" => entry.path = Some(PathBuf::from(unescape(value))),
                "name" => entry.name = unescape(value),
                "disk" => entry.disk_hash = value.parse().ok(),
                "order" => entry.order = value.parse().map_err(|_| "bad order")?,
                "cursor" => {
                    let (line, column) = value.split_once(' ').ok_or("bad cursor")?;
                    entry.cursor = (
                        line.parse().map_err(|_| "bad cursor")?,
                        column.parse().map_err(|_| "bad cursor")?,
                    );
                }
                "language" => entry.language = unescape(value),
                "length" => length = value.parse::<usize>().ok(),
                _ => {}
            }
        }
        if length != Some(text.len()) {
            return Err("truncated recovery record".into());
        }
        if entry.key.is_empty() {
            return Err("recovery record without a key".into());
        }
        entry.text = text.to_string();
        Ok(entry)
    }
}

fn file_name(key: &str) -> String {
    let safe: String = key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.{EXTENSION}")
}

/// Write (or replace) one record atomically.
pub fn write(directory: &Path, entry: &Entry) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let mut file = NamedTempFile::new_in(directory)?;
    file.write_all(entry.encode().as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(directory.join(file_name(&entry.key)))
        .map_err(|error| error.error)?;
    Ok(())
}

pub fn remove(directory: &Path, key: &str) {
    let _ = fs::remove_file(directory.join(file_name(key)));
}

/// Remove every record (after a clean shutdown with nothing unsaved).
pub fn clear(directory: &Path) {
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == EXTENSION)
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Read all records, ordered by tab order; corrupt records are returned as problems.
pub fn load(directory: &Path) -> (Vec<Entry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let Ok(listing) = fs::read_dir(directory) else {
        return (entries, problems);
    };
    for item in listing.flatten() {
        let path = item.path();
        if path
            .extension()
            .is_none_or(|extension| extension != EXTENSION)
        {
            continue;
        }
        match fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|record| Entry::decode(&record))
        {
            Ok(entry) => entries.push(entry),
            Err(error) => problems.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    entries.sort_by_key(|entry| entry.order);
    (entries, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, order: usize) -> Entry {
        Entry {
            key: key.into(),
            path: Some(PathBuf::from("/p/ünï code.rs")),
            name: "ünï code.rs".into(),
            text: "line\r\n\nwith \\ escapes and \n\n blank lines 😀".into(),
            disk_hash: Some(42),
            order,
            cursor: (3, 7),
            language: "rust".into(),
        }
    }

    #[test]
    fn records_round_trip_and_corrupt_ones_are_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let mut untitled = entry("b-2", 1);
        untitled.path = None;
        write(directory.path(), &entry("a-1", 2)).unwrap();
        write(directory.path(), &untitled).unwrap();
        // An interrupted write leaves a truncated record; another file is garbage.
        let truncated = entry("c-3", 0).encode();
        fs::write(
            directory.path().join("c-3.recovery"),
            &truncated[..truncated.len() - 5],
        )
        .unwrap();
        fs::write(directory.path().join("junk.recovery"), "nonsense").unwrap();
        fs::write(directory.path().join("other.txt"), "ignored").unwrap();
        let (entries, problems) = load(directory.path());
        assert_eq!(entries, vec![untitled, entry("a-1", 2)]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        remove(directory.path(), "a-1");
        assert_eq!(load(directory.path()).0.len(), 1);
        clear(directory.path());
        assert!(load(directory.path()).0.is_empty());
        assert!(
            fs::read_dir(directory.path())
                .unwrap()
                .any(|item| item.unwrap().file_name() == "other.txt")
        );
        assert!(
            Entry::decode("hephaestus-recovery 9\nkey x\n\n")
                .unwrap_err()
                .contains("unsupported")
        );
    }
}
