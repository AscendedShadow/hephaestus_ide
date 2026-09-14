//! Workspace identity. Filesystem discovery and watching will be added here.

use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Workspace {
    root: Option<PathBuf>,
}

impl Workspace {
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn display_name(&self) -> &str {
        self.root()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("No project open")
    }
}
