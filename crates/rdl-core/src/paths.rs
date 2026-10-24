//! Where rusty-downloader keeps its data.
//!
//! Installed builds use `%APPDATA%\rusty-downloader`. A `portable.flag` file next to the
//! executable switches to `<exe dir>\data`, so the whole app can live on a USB stick.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const APP_DIR_NAME: &str = "rusty-downloader";
pub const PORTABLE_FLAG: &str = "portable.flag";
pub const DB_FILE_NAME: &str = "rusty-downloader.db";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub portable: bool,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub plugins_dir: PathBuf,
    pub db_file: PathBuf,
}

impl AppPaths {
    /// Resolve paths for the running executable.
    pub fn resolve() -> Result<Self> {
        let exe = std::env::current_exe()?;
        let exe_dir = exe.parent().ok_or(Error::NoDataDir)?;
        Self::resolve_with(exe_dir, dirs::data_dir())
    }

    /// Resolution logic with its inputs injected, for tests.
    pub fn resolve_with(exe_dir: &Path, roaming_app_data: Option<PathBuf>) -> Result<Self> {
        let portable = exe_dir.join(PORTABLE_FLAG).is_file();
        let data_dir = if portable {
            exe_dir.join("data")
        } else {
            roaming_app_data.ok_or(Error::NoDataDir)?.join(APP_DIR_NAME)
        };
        Ok(Self::in_dir(data_dir, portable))
    }

    /// Lay out the standard sub-folders under `data_dir`.
    pub fn in_dir(data_dir: PathBuf, portable: bool) -> Self {
        Self {
            portable,
            logs_dir: data_dir.join("logs"),
            plugins_dir: data_dir.join("plugins"),
            db_file: data_dir.join(DB_FILE_NAME),
            data_dir,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [&self.data_dir, &self.logs_dir, &self.plugins_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_mode_uses_roaming_app_data() {
        let exe_dir = tempfile::tempdir().unwrap();
        let roaming = tempfile::tempdir().unwrap();

        let paths = AppPaths::resolve_with(exe_dir.path(), Some(roaming.path().into())).unwrap();

        assert!(!paths.portable);
        assert_eq!(paths.data_dir, roaming.path().join(APP_DIR_NAME));
        assert_eq!(paths.db_file, paths.data_dir.join(DB_FILE_NAME));
    }

    #[test]
    fn portable_flag_keeps_data_next_to_exe() {
        let exe_dir = tempfile::tempdir().unwrap();
        std::fs::write(exe_dir.path().join(PORTABLE_FLAG), "").unwrap();

        let paths = AppPaths::resolve_with(exe_dir.path(), None).unwrap();

        assert!(paths.portable);
        assert_eq!(paths.data_dir, exe_dir.path().join("data"));
    }

    #[test]
    fn missing_app_data_is_an_error_when_installed() {
        let exe_dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            AppPaths::resolve_with(exe_dir.path(), None),
            Err(Error::NoDataDir)
        ));
    }

    #[test]
    fn ensure_dirs_creates_layout() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths::in_dir(root.path().join("d"), false);

        paths.ensure_dirs().unwrap();

        assert!(paths.logs_dir.is_dir());
        assert!(paths.plugins_dir.is_dir());
    }
}
