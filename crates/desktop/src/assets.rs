use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component::IconNamed;

#[derive(Clone, Copy)]
pub enum AppIcon {
    Anvil,
    Bug,
    Files,
    GitBranch,
    Refresh,
}

impl AppIcon {
    const ALL: [Self; 5] = [
        Self::Anvil,
        Self::Bug,
        Self::Files,
        Self::GitBranch,
        Self::Refresh,
    ];

    fn svg(self) -> &'static str {
        match self {
            Self::Anvil => include_str!("icons/anvil.svg"),
            Self::Bug => include_str!("icons/bug.svg"),
            Self::Files => include_str!("icons/files.svg"),
            Self::GitBranch => include_str!("icons/git-branch.svg"),
            Self::Refresh => include_str!("icons/refresh-cw.svg"),
        }
    }
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Anvil => "icons/app/anvil.svg",
            Self::Bug => "icons/app/bug.svg",
            Self::Files => "icons/app/files.svg",
            Self::GitBranch => "icons/app/git-branch.svg",
            Self::Refresh => "icons/app/refresh-cw.svg",
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
