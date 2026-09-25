use std::{
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use ropey::Rope;
use tempfile::NamedTempFile;

pub const MAX_OPEN_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    path: Option<PathBuf>,
    text: Rope,
    saved_text: Rope,
    line_ending: LineEnding,
    utf8_bom: bool,
    disk_hash: Option<u64>,
}

impl Document {
    pub fn open(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        let file = File::open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("Choose a regular text file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_OPEN_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_OPEN_BYTES {
            return Err(io::Error::other("Basic editor supports files up to 8 MiB"));
        }
        let mut document = Self::decode(&bytes)?;
        document.path = Some(path);
        document.disk_hash = Some(hash_bytes(&bytes));
        Ok(document)
    }

    fn decode(bytes: &[u8]) -> io::Result<Self> {
        let utf8_bom = bytes.starts_with(b"\xef\xbb\xbf");
        let bytes = if utf8_bom { &bytes[3..] } else { bytes };
        let text = std::str::from_utf8(bytes)
            .map_err(|_| io::Error::other("Only UTF-8 text files are supported"))?;
        if text.contains('\0') {
            return Err(io::Error::other("Binary files cannot be edited"));
        }
        let crlf = text.matches("\r\n").count();
        let line_ending = if crlf > 0 && crlf == text.matches('\n').count() {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        let text = Rope::from(text.replace("\r\n", "\n").as_str());
        Ok(Self {
            saved_text: text.clone(),
            text,
            line_ending,
            utf8_bom,
            path: None,
            disk_hash: None,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn name(&self) -> String {
        self.path()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn set_text(&mut self, text: Rope) {
        self.text = text;
    }

    pub fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn save_to(mut self, path: &Path) -> io::Result<Self> {
        let path = if path.exists() {
            path.canonicalize()?
        } else {
            std::path::absolute(path)?
        };
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("Invalid file path"))?;
        if self.path.as_deref() == Some(path.as_path()) && self.has_external_changes()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "File changed on disk; reload it or use Save As to preserve both versions",
            ));
        }
        let permissions = match fs::metadata(&path) {
            Ok(metadata) => {
                if metadata.permissions().readonly() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "File is read-only",
                    ));
                }
                Some(metadata.permissions())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let mut file = NamedTempFile::new_in(parent)?;
        if let Some(permissions) = permissions {
            file.as_file().set_permissions(permissions)?;
        }
        if self.utf8_bom {
            file.write_all(b"\xef\xbb\xbf")?;
        }
        let text = self.text.to_string().replace("\r\n", "\n");
        let text = match self.line_ending {
            LineEnding::Lf => text,
            LineEnding::CrLf => text.replace('\n', "\r\n"),
        };
        file.write_all(text.as_bytes())?;
        let mut written = Vec::with_capacity(text.len() + 3);
        if self.utf8_bom {
            written.extend_from_slice(b"\xef\xbb\xbf");
        }
        written.extend_from_slice(text.as_bytes());
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|error| error.error)?;
        self.path = Some(path.canonicalize().unwrap_or(path));
        self.saved_text = self.text.clone();
        self.disk_hash = Some(hash_bytes(&written));
        Ok(self)
    }

    pub fn has_external_changes(&self) -> io::Result<bool> {
        let (Some(path), Some(expected)) = (&self.path, self.disk_hash) else {
            return Ok(false);
        };
        match fs::read(path) {
            Ok(bytes) => Ok(hash_bytes(&bytes) != expected),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error),
        }
    }

    pub fn accept_saved(&mut self, saved: Self) {
        self.path = saved.path;
        self.saved_text = saved.saved_text;
        self.disk_hash = saved.disk_hash;
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_crlf_and_bom_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unicode.rs");
        let original = "\u{feff}let greeting = \"你好 👋\";\r\n";
        fs::write(&path, original).unwrap();
        let mut document = Document::open(&path).unwrap();
        assert_eq!(document.line_ending(), LineEnding::CrLf);
        assert_eq!(document.text().to_string(), "let greeting = \"你好 👋\";\n");
        assert!(!document.is_dirty());
        document.set_text(Rope::from("// café\n"));
        let saved = document.clone().save_to(&path).unwrap();
        document.accept_saved(saved);
        assert!(!document.is_dirty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "\u{feff}// café\r\n");
    }

    #[test]
    fn undo_to_saved_text_clears_dirty_and_save_preserves_newer_edits() {
        let mut document = Document::decode(b"original\n").unwrap();
        let original = document.text().clone();
        document.set_text(Rope::from("edited\n"));
        assert!(document.is_dirty());
        document.set_text(original);
        assert!(!document.is_dirty());
        let directory = tempfile::tempdir().unwrap();
        let saved = document
            .clone()
            .save_to(&directory.path().join("file.txt"))
            .unwrap();
        document.set_text(Rope::from("newer edit\n"));
        document.accept_saved(saved);
        assert!(document.is_dirty());
        assert_eq!(document.text().to_string(), "newer edit\n");
    }

    #[test]
    fn failed_save_does_not_modify_original_or_clear_dirty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.txt");
        fs::write(&path, "original").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.set_text(Rope::from("edited"));
        assert!(
            document
                .clone()
                .save_to(&directory.path().join("missing/file.txt"))
                .is_err()
        );
        assert!(document.is_dirty());
        assert_eq!(fs::read_to_string(path).unwrap(), "original");
    }

    #[test]
    fn rejects_binary_invalid_utf8_and_oversized_files() {
        assert!(Document::decode(b"binary\0text").is_err());
        assert!(Document::decode(&[0xff, 0xfe]).is_err());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.txt");
        File::create(&path)
            .unwrap()
            .set_len(MAX_OPEN_BYTES + 1)
            .unwrap();
        assert!(Document::open(&path).is_err());
    }

    #[test]
    fn empty_files_and_no_final_newline_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        for text in ["", "no final newline", "mixed\r\nend\n"] {
            let document = Document::decode(text.as_bytes()).unwrap();
            let path = directory.path().join("file.txt");
            document.save_to(&path).unwrap();
            assert_eq!(
                fs::read_to_string(path).unwrap(),
                text.replace("\r\n", "\n")
            );
        }
    }

    #[test]
    fn pasted_crlf_is_not_double_encoded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("paste.txt");
        let mut document = Document::decode(b"windows\r\n").unwrap();
        document.set_text(Rope::from("pasted\r\ntext\n"));
        document.save_to(&path).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "pasted\r\ntext\r\n");
    }

    #[test]
    fn refuses_to_overwrite_external_edits_and_tracks_new_saved_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.txt");
        fs::write(&path, "first").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.set_text(Rope::from("ours"));
        fs::write(&path, "theirs").unwrap();
        assert!(document.has_external_changes().unwrap());
        assert_eq!(
            document.clone().save_to(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "theirs");
        let saved = document
            .save_to(&directory.path().join("ours.txt"))
            .unwrap();
        assert!(!saved.has_external_changes().unwrap());
    }
}
