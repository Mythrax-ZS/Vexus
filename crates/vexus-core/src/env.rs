//! The Windows environment Vexus runs in: well-known folders, resolved once.
//!
//! Everything the rules and the safety guard need to know about "where things are" lives here,
//! so tests (and the `--sandbox` mode) can swap in a fake profile without touching real data.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_LocalAppData,
    FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_ProgramData,
    FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, FOLDERID_RoamingAppData, FOLDERID_Videos,
    FOLDERID_Windows, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
};
use windows::core::GUID;

#[derive(Debug, Clone)]
pub struct Env {
    pub system_drive: PathBuf,
    pub system_root: PathBuf,
    pub program_files: PathBuf,
    pub program_files_x86: Option<PathBuf>,
    pub program_data: PathBuf,
    pub user_profile: PathBuf,
    pub local_app_data: PathBuf,
    pub app_data: PathBuf,
    /// The effective temp directory (`%TEMP%`), which users and installers can repoint — it is
    /// therefore never trusted without the guard's sanity checks.
    pub temp: PathBuf,
    pub desktop: PathBuf,
    pub documents: PathBuf,
    pub downloads: PathBuf,
    pub music: PathBuf,
    pub pictures: PathBuf,
    pub videos: PathBuf,
    /// OneDrive sync roots (cloud placeholders live here; never touched).
    pub one_drive: Vec<PathBuf>,
    /// True when this is a fake profile created for testing or demos.
    pub sandboxed: bool,
}

impl Env {
    /// Resolves the real machine's folders.
    pub fn detect() -> Self {
        // The OS answer first, like every other folder here: an environment variable is
        // something whoever launched us could have set to anything, and five rule roots hang off
        // this one.
        let system_root = known_folder(&FOLDERID_Windows)
            .or_else(|| env_dir("SystemRoot"))
            .or_else(|| env_dir("windir"))
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system_drive = drive_root(&system_root);

        let user_profile = known_folder(&FOLDERID_Profile)
            .or_else(|| env_dir("USERPROFILE"))
            .unwrap_or_else(|| system_drive.join("Users").join("Default"));
        let local_app_data = known_folder(&FOLDERID_LocalAppData)
            .or_else(|| env_dir("LOCALAPPDATA"))
            .unwrap_or_else(|| user_profile.join("AppData").join("Local"));
        let app_data = known_folder(&FOLDERID_RoamingAppData)
            .or_else(|| env_dir("APPDATA"))
            .unwrap_or_else(|| user_profile.join("AppData").join("Roaming"));
        let program_files = known_folder(&FOLDERID_ProgramFiles)
            .or_else(|| env_dir("ProgramFiles"))
            .unwrap_or_else(|| system_drive.join("Program Files"));
        let program_files_x86 = known_folder(&FOLDERID_ProgramFilesX86)
            .or_else(|| env_dir("ProgramFiles(x86)"))
            .filter(|p| *p != program_files);
        let program_data = known_folder(&FOLDERID_ProgramData)
            .or_else(|| env_dir("ProgramData"))
            .unwrap_or_else(|| system_drive.join("ProgramData"));

        let user_folder =
            |id: &GUID, name: &str| known_folder(id).unwrap_or_else(|| user_profile.join(name));
        let one_drive = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
            .iter()
            .filter_map(|v| env_dir(v))
            .fold(Vec::<PathBuf>::new(), |mut acc, p| {
                if !acc.contains(&p) {
                    acc.push(p);
                }
                acc
            });

        Env {
            system_drive,
            system_root,
            program_files,
            program_files_x86,
            program_data,
            temp: std::env::temp_dir(),
            desktop: user_folder(&FOLDERID_Desktop, "Desktop"),
            documents: user_folder(&FOLDERID_Documents, "Documents"),
            downloads: user_folder(&FOLDERID_Downloads, "Downloads"),
            music: user_folder(&FOLDERID_Music, "Music"),
            pictures: user_folder(&FOLDERID_Pictures, "Pictures"),
            videos: user_folder(&FOLDERID_Videos, "Videos"),
            user_profile,
            local_app_data,
            app_data,
            one_drive,
            sandboxed: false,
        }
    }

    /// A self-contained fake Windows layout under `root`. Used by tests, the `sandbox` CLI
    /// command and `vexus --sandbox <dir>` so cleaning can be exercised without real data.
    pub fn sandbox(root: &Path) -> Self {
        let user = root.join("Users").join("vexus");
        let local = user.join("AppData").join("Local");
        Env {
            system_drive: root.to_path_buf(),
            system_root: root.join("Windows"),
            program_files: root.join("Program Files"),
            program_files_x86: Some(root.join("Program Files (x86)")),
            program_data: root.join("ProgramData"),
            temp: local.join("Temp"),
            desktop: user.join("Desktop"),
            documents: user.join("Documents"),
            downloads: user.join("Downloads"),
            music: user.join("Music"),
            pictures: user.join("Pictures"),
            videos: user.join("Videos"),
            app_data: user.join("AppData").join("Roaming"),
            local_app_data: local,
            user_profile: user,
            one_drive: Vec::new(),
            sandboxed: true,
        }
    }

    /// Creates the directory skeleton of a sandbox so rules have something to point at.
    pub fn create_sandbox_skeleton(&self) -> std::io::Result<()> {
        for dir in [
            &self.system_root,
            &self.program_files,
            &self.program_data,
            &self.local_app_data,
            &self.app_data,
            &self.temp,
            &self.desktop,
            &self.documents,
            &self.downloads,
            &self.music,
            &self.pictures,
            &self.videos,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// An absolute directory path from an environment variable, if set and sensible.
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: `SHGetKnownFolderPath` returns a NUL-terminated wide string allocated with
    // `CoTaskMemAlloc`. We copy it out before freeing it exactly once.
    unsafe {
        let pw = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None).ok()?;
        let path = PathBuf::from(OsString::from_wide(pw.as_wide()));
        CoTaskMemFree(Some(pw.0 as *const _));
        Some(path).filter(|p| p.is_absolute())
    }
}

/// `C:\` for `C:\Windows\System32`.
pub fn drive_root(path: &Path) -> PathBuf {
    let mut root = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => root.push(c.as_os_str()),
            _ => break,
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_finds_a_sane_layout() {
        let env = Env::detect();
        assert!(env.system_root.is_absolute());
        assert!(env.local_app_data.starts_with(&env.user_profile) || env.sandboxed);
        assert!(!env.sandboxed);
    }

    #[test]
    fn sandbox_stays_inside_its_root() {
        let root = Path::new(r"C:\fake-root");
        let env = Env::sandbox(root);
        for p in [&env.system_root, &env.temp, &env.documents, &env.app_data, &env.program_files] {
            assert!(p.starts_with(root), "{p:?} escaped the sandbox");
        }
        assert!(env.sandboxed);
    }

    #[test]
    fn drive_root_extracts_prefix() {
        assert_eq!(drive_root(Path::new(r"C:\Windows\System32")), PathBuf::from(r"C:\"));
        assert_eq!(drive_root(Path::new(r"D:\")), PathBuf::from(r"D:\"));
    }
}
