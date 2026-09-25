use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub folder: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub active: usize,
    pub cursors: Vec<(u32, u32)>,
    pub recent: Vec<PathBuf>,
}

pub fn path(settings: &Path) -> PathBuf {
    settings.with_file_name("session.json")
}

pub fn load(path: &Path) -> Session {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, session: &Session) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid session path"))?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut file, session)?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_missing_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        assert!(load(&path).files.is_empty());
        save(
            &path,
            &Session {
                folder: None,
                files: vec![PathBuf::from("one.rs")],
                active: 0,
                cursors: vec![(3, 2)],
                recent: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(load(&path).files, vec![PathBuf::from("one.rs")]);
    }
}
