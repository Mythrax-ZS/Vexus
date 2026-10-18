//! Where Vexus keeps its own data (logs, settings, history, the Recovery store).

use std::io;
use std::path::{Path, PathBuf};

use crate::env::Env;

/// File whose presence next to the executable switches Vexus to portable mode.
pub const PORTABLE_MARKER: &str = "vexus.portable";

#[derive(Debug, Clone)]
pub enum DataMode {
    /// `%LOCALAPPDATA%\Vexus`.
    Installed,
    /// `<folder containing vexus.exe>\data` — nothing is written anywhere else.
    Portable { exe_dir: PathBuf },
    /// An explicit folder (sandbox runs, tests).
    Custom(PathBuf),
}

impl DataMode {
    /// Portable if the marker file sits next to the executable, otherwise installed.
    pub fn detect(exe_dir: Option<&Path>) -> DataMode {
        match exe_dir {
            Some(dir) if dir.join(PORTABLE_MARKER).is_file() => {
                DataMode::Portable { exe_dir: dir.to_path_buf() }
            }
            _ => DataMode::Installed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataDirs {
    pub root: PathBuf,
    pub logs: PathBuf,
    /// Settings, history and other small state files.
    pub state: PathBuf,
    pub recovery: PathBuf,
    /// WebView2 profile (kept inside the data folder in portable mode).
    pub webview: PathBuf,
    pub portable: bool,
}

impl DataDirs {
    pub fn resolve(mode: &DataMode, env: &Env) -> DataDirs {
        let (root, portable) = match mode {
            DataMode::Installed => (env.local_app_data.join("Vexus"), false),
            DataMode::Portable { exe_dir } => (exe_dir.join("data"), true),
            DataMode::Custom(p) => (p.clone(), false),
        };
        DataDirs {
            logs: root.join("logs"),
            state: root.join("state"),
            recovery: root.join("recovery"),
            webview: root.join("webview"),
            root,
            portable,
        }
    }

    pub fn ensure(&self) -> io::Result<()> {
        for d in [&self.logs, &self.state, &self.recovery] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_data_lives_under_local_app_data() {
        let env = Env::sandbox(Path::new(r"C:\Sandbox"));
        let d = DataDirs::resolve(&DataMode::Installed, &env);
        assert!(d.root.starts_with(&env.local_app_data));
        assert!(!d.portable);
        assert!(d.recovery.starts_with(&d.root) && d.logs.starts_with(&d.root));
    }

    #[test]
    fn portable_mode_is_detected_by_marker_and_stays_beside_the_exe() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(DataMode::detect(Some(dir.path())), DataMode::Installed));
        std::fs::write(dir.path().join(PORTABLE_MARKER), b"").unwrap();
        let mode = DataMode::detect(Some(dir.path()));
        assert!(matches!(mode, DataMode::Portable { .. }));
        let env = Env::detect();
        let d = DataDirs::resolve(&mode, &env);
        assert!(d.portable);
        assert!(d.root.starts_with(dir.path()));
        assert!(d.webview.starts_with(dir.path()));
    }
}
