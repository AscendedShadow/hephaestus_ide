use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component::IconNamed;

#[derive(Clone, Copy)]
pub enum AppIcon {
    Bug,
    CloudDownload,
    Files,
    GitBranch,
    File,
    FileIgnored,
    FileJavaScript,
    FileJson,
    FileLock,
    FileMarkdown,
    FileRust,
    FileToml,
    FileTypeScript,
}

impl AppIcon {
    const ALL: [Self; 13] = [
        Self::Bug,
        Self::CloudDownload,
        Self::Files,
        Self::GitBranch,
        Self::File,
        Self::FileIgnored,
        Self::FileJavaScript,
        Self::FileJson,
        Self::FileLock,
        Self::FileMarkdown,
        Self::FileRust,
        Self::FileToml,
        Self::FileTypeScript,
    ];

    fn svg(self) -> &'static str {
        match self {
            Self::Bug => include_str!("icons/bug.svg"),
            Self::CloudDownload => include_str!("icons/cloud-download.svg"),
            Self::Files => include_str!("icons/files.svg"),
            Self::GitBranch => include_str!("icons/git-branch.svg"),
            Self::File => include_str!("icons/file.svg"),
            Self::FileIgnored => include_str!("icons/file-ignored.svg"),
            Self::FileJavaScript => include_str!("icons/file-javascript.svg"),
            Self::FileJson => include_str!("icons/file-json.svg"),
            Self::FileLock => include_str!("icons/file-lock.svg"),
            Self::FileMarkdown => include_str!("icons/file-markdown.svg"),
            Self::FileRust => include_str!("icons/file-rust.svg"),
            Self::FileToml => include_str!("icons/file-toml.svg"),
            Self::FileTypeScript => include_str!("icons/file-typescript.svg"),
        }
    }
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Bug => "icons/app/bug.svg",
            Self::CloudDownload => "icons/app/cloud-download.svg",
            Self::Files => "icons/app/files.svg",
            Self::GitBranch => "icons/app/git-branch.svg",
            Self::File => "icons/app/file.svg",
            Self::FileIgnored => "icons/app/file-ignored.svg",
            Self::FileJavaScript => "icons/app/file-javascript.svg",
            Self::FileJson => "icons/app/file-json.svg",
            Self::FileLock => "icons/app/file-lock.svg",
            Self::FileMarkdown => "icons/app/file-markdown.svg",
            Self::FileRust => "icons/app/file-rust.svg",
            Self::FileToml => "icons/app/file-toml.svg",
            Self::FileTypeScript => "icons/app/file-typescript.svg",
        }
        .into()
    }
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match AppIcon::ALL.into_iter().find(|icon| icon.path() == path) {
            Some(icon) => Ok(Some(Cow::Borrowed(icon.svg().as_bytes()))),
            None => gpui_component_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        paths.extend(
            AppIcon::ALL
                .into_iter()
                .map(IconNamed::path)
                .filter(|icon| icon.starts_with(path)),
        );
        Ok(paths)
    }
}
