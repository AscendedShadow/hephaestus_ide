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

/// A text encoding the editor can read and write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// A single- or multi-byte legacy encoding, by its WHATWG name.
    Legacy(&'static str),
}

/// Legacy encodings offered when automatic (UTF-8/UTF-16) decoding does not apply.
pub const LEGACY_ENCODINGS: &[&str] = &[
    "windows-1252",
    "ISO-8859-2",
    "ISO-8859-15",
    "windows-1250",
    "windows-1251",
    "KOI8-R",
    "Shift_JIS",
    "EUC-JP",
    "GBK",
    "Big5",
    "EUC-KR",
];

impl Encoding {
    pub fn all() -> Vec<Encoding> {
        let mut all = vec![Self::Utf8, Self::Utf8Bom, Self::Utf16Le, Self::Utf16Be];
        all.extend(LEGACY_ENCODINGS.iter().map(|name| Self::Legacy(name)));
        all
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Legacy(name) => name,
        }
    }

    fn legacy(name: &str) -> io::Result<&'static encoding_rs::Encoding> {
        encoding_rs::Encoding::for_label(name.as_bytes())
            .ok_or_else(|| io::Error::other(format!("Unknown encoding {name}")))
    }

    /// Decode bytes with this encoding (a matching BOM is skipped).
    pub fn decode(self, bytes: &[u8]) -> io::Result<String> {
        let invalid = || io::Error::other(format!("The file is not valid {}", self.label()));
        match self {
            Self::Utf8 | Self::Utf8Bom => {
                let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
                std::str::from_utf8(bytes)
                    .map(str::to_string)
                    .map_err(|_| invalid())
            }
            Self::Utf16Le | Self::Utf16Be => {
                let (bom, swap): (&[u8], bool) = match self {
                    Self::Utf16Le => (b"\xff\xfe", false),
                    _ => (b"\xfe\xff", true),
                };
                let bytes = bytes.strip_prefix(bom).unwrap_or(bytes);
                if bytes.len() % 2 != 0 {
                    return Err(invalid());
                }
                let units = bytes.as_chunks::<2>().0.iter().map(|pair| {
                    if swap {
                        u16::from_be_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_le_bytes([pair[0], pair[1]])
                    }
                });
                char::decode_utf16(units)
                    .collect::<Result<String, _>>()
                    .map_err(|_| invalid())
            }
            Self::Legacy(name) => Self::legacy(name)?
                .decode_without_bom_handling_and_without_replacement(bytes)
                .map(|text| text.into_owned())
                .ok_or_else(invalid),
        }
    }

    /// Encode text, failing if a character cannot be represented.
    pub fn encode(self, text: &str) -> io::Result<Vec<u8>> {
        Ok(match self {
            Self::Utf8 => text.as_bytes().to_vec(),
            Self::Utf8Bom => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
            Self::Utf16Le => [0xff, 0xfe]
                .into_iter()
                .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
                .collect(),
            Self::Utf16Be => [0xfe, 0xff]
                .into_iter()
                .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
                .collect(),
            Self::Legacy(name) => {
                let encoding = Self::legacy(name)?;
                let (bytes, _, unmappable) = encoding.encode(text);
                if unmappable {
                    let character = text
                        .chars()
                        .find(|character| encoding.encode(&character.to_string()).2)
                        .unwrap_or('?');
                    return Err(io::Error::other(format!(
                        "{character:?} cannot be saved as {name}; choose another encoding"
                    )));
                }
                bytes.into_owned()
            }
        })
    }

    /// Detect the encoding of file bytes: a BOM, or UTF-8.
    pub fn detect(bytes: &[u8]) -> io::Result<Self> {
        if bytes.starts_with(b"\xef\xbb\xbf") {
            Ok(Self::Utf8Bom)
        } else if bytes.starts_with(b"\xff\xfe") {
            Ok(Self::Utf16Le)
        } else if bytes.starts_with(b"\xfe\xff") {
            Ok(Self::Utf16Be)
        } else if std::str::from_utf8(bytes).is_ok() {
            Ok(Self::Utf8)
        } else {
            Err(io::Error::other(
                "Not valid UTF-8 — use Reopen with Encoding to choose the file's encoding",
            ))
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    path: Option<PathBuf>,
    text: Rope,
    saved_text: Rope,
    line_ending: LineEnding,
    saved_line_ending: LineEnding,
    encoding: Encoding,
    saved_encoding: Encoding,
    disk_hash: Option<u64>,
}

impl Document {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::open_with(path, None)
    }

    /// Open with an explicit encoding (`None` detects a BOM or UTF-8).
    pub fn open_with(path: &Path, encoding: Option<Encoding>) -> io::Result<Self> {
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
        let mut document = Self::decode_with(&bytes, encoding)?;
        document.path = Some(path);
        document.disk_hash = Some(hash_bytes(&bytes));
        Ok(document)
    }

    /// An unsaved, clean document holding `text` (for generated read-only views).
    pub fn from_text(text: &str) -> Self {
        let text = Rope::from(text);
        Self {
            saved_text: text.clone(),
            text,
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn decode(bytes: &[u8]) -> io::Result<Self> {
        Self::decode_with(bytes, None)
    }

    fn decode_with(bytes: &[u8], encoding: Option<Encoding>) -> io::Result<Self> {
        let encoding = match encoding {
            Some(encoding) => encoding,
            None => Encoding::detect(bytes)?,
        };
        let text = encoding.decode(bytes)?;
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
            saved_line_ending: line_ending,
            encoding,
            saved_encoding: encoding,
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

    /// Modified text, or a pending encoding/line-ending conversion.
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved_text
            || self.encoding != self.saved_encoding
            || self.line_ending != self.saved_line_ending
    }

    /// Hash of the file content last read or written (for recovery conflict checks).
    pub fn disk_hash(&self) -> Option<u64> {
        self.disk_hash
    }

    /// Hash of the file's current content on disk.
    pub fn current_disk_hash(path: &Path) -> Option<u64> {
        fs::read(path).ok().map(|bytes| hash_bytes(&bytes))
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Convert the encoding used when saving (a pending change until saved).
    pub fn set_encoding(&mut self, encoding: Encoding) {
        self.encoding = encoding;
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Point the document at a new path after its file was moved or renamed on disk. The
    /// recorded disk content is kept, so external-change detection keeps working.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    /// Change the line ending used when saving.
    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.line_ending = line_ending;
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
        let text = self.text.to_string().replace("\r\n", "\n");
        let text = match self.line_ending {
            LineEnding::Lf => text,
            LineEnding::CrLf => text.replace('\n', "\r\n"),
        };
        // Encode before touching the file so an unrepresentable character leaves the
        // disk file untouched.
        let written = self.encoding.encode(&text)?;
        let mut file = NamedTempFile::new_in(parent)?;
        if let Some(permissions) = permissions {
            file.as_file().set_permissions(permissions)?;
        }
        file.write_all(&written)?;
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|error| error.error)?;
        self.path = Some(path.canonicalize().unwrap_or(path));
        self.saved_text = self.text.clone();
        self.saved_encoding = self.encoding;
        self.saved_line_ending = self.line_ending;
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
        self.saved_encoding = saved.saved_encoding;
        self.saved_line_ending = saved.saved_line_ending;
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
        assert!(Document::decode(&[0xc3, 0x28]).is_err());
        assert!(
            Document::decode(&[0xff, 0xfe]).is_ok(),
            "a UTF-16 BOM alone is an empty file"
        );
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
    fn utf16_and_legacy_encodings_round_trip_and_conversions_are_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("utf16.txt");
        let utf16: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain("héllo 😀\r\nline".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        fs::write(&path, &utf16).unwrap();
        let mut document = Document::open(&path).unwrap();
        assert_eq!(document.encoding(), Encoding::Utf16Le);
        assert_eq!(document.line_ending(), LineEnding::CrLf);
        assert_eq!(document.text().to_string(), "héllo 😀\nline");
        let saved = document.clone().save_to(&path).unwrap();
        document.accept_saved(saved);
        assert_eq!(fs::read(&path).unwrap(), utf16, "preserved by default");

        // A conversion with unchanged text is a pending change until saved.
        document.set_encoding(Encoding::Utf8);
        document.set_line_ending(LineEnding::Lf);
        assert!(document.is_dirty());
        let saved = document.clone().save_to(&path).unwrap();
        document.accept_saved(saved);
        assert!(!document.is_dirty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "héllo 😀\nline");

        // Unrepresentable characters fail without touching the file.
        document.set_encoding(Encoding::Legacy("windows-1252"));
        let error = document.clone().save_to(&path).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot be saved as windows-1252"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "héllo 😀\nline");

        let latin = directory.path().join("latin.txt");
        fs::write(&latin, b"caf\xe9\n").unwrap();
        assert!(
            Document::open(&latin)
                .unwrap_err()
                .to_string()
                .contains("Reopen with Encoding")
        );
        let legacy = Document::open_with(&latin, Some(Encoding::Legacy("windows-1252"))).unwrap();
        assert_eq!(legacy.text().to_string(), "café\n");
        let saved = legacy.save_to(&latin).unwrap();
        assert!(!saved.has_external_changes().unwrap());
        assert_eq!(fs::read(&latin).unwrap(), b"caf\xe9\n");
        assert!(Encoding::Utf16Be.decode(&[0xfe, 0xff, 0xd8]).is_err());
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
