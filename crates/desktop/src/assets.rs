use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component::IconNamed;

#[derive(Clone, Copy)]
pub enum AppIcon {
    Bug,
    CloudDownload,
    Files,
    GitBranch,
}

impl AppIcon {
    const ALL: [Self; 4] = [Self::Bug, Self::CloudDownload, Self::Files, Self::GitBranch];

    fn svg(self) -> &'static str {
        match self {
            Self::Bug => include_str!("icons/bug.svg"),
            Self::CloudDownload => include_str!("icons/cloud-download.svg"),
            Self::Files => include_str!("icons/files.svg"),
            Self::GitBranch => include_str!("icons/git-branch.svg"),
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
