//! [`PathGuard`]: the default-deny gatekeeper every scan and every delete goes through.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

use super::path::{is_drive_root, is_network, normalize, strictly_within, within_or_equal};
use crate::env::Env;

/// Why a path is off-limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protection {
    /// Could not be normalised (`..`, device path, alternate data stream, …).
    Unresolvable,
    DriveRoot,
    Network,
    /// A well-known folder that may never be removed itself.
    Folder(&'static str),
    /// Inside a hard-denied tree.
    Denied(&'static str),
    /// Not inside any root a rule has declared and the guard has validated.
    NotPermitted,
    /// A file Windows needs regardless of where it sits (page file, registry hives, …).
    SystemFile,
}

impl fmt::Display for Protection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protection::Unresolvable => f.write_str("its location is ambiguous"),
            Protection::DriveRoot => f.write_str("it is a drive root"),
            Protection::Network => f.write_str("it is on the network"),
            Protection::Folder(what) => write!(f, "it is {what}"),
            Protection::Denied(what) => write!(f, "it belongs to {what}"),
            Protection::NotPermitted => f.write_str("no cleaning rule covers it"),
            Protection::SystemFile => f.write_str("Windows needs this file"),
        }
    }
}

/// Why a rule root was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRejection {
    Unresolvable,
    DriveRoot,
    Network,
    Protected(&'static str),
    /// The root would contain protected locations (e.g. `%TEMP%` set to the user profile).
    TooBroad(&'static str),
    /// A root taken from a user-controllable variable that does not look like a temp folder.
    NotTempLike,
}

impl fmt::Display for RootRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootRejection::Unresolvable => f.write_str("the path is ambiguous"),
            RootRejection::DriveRoot => f.write_str("it points at an entire drive"),
            RootRejection::Network => f.write_str("it is a network location"),
            RootRejection::Protected(what) => write!(f, "it is {what}"),
            RootRejection::TooBroad(what) => write!(f, "it would include {what}"),
            RootRejection::NotTempLike => f.write_str("it does not look like a temporary folder"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PathGuard {
    /// Folders that can never be removed themselves (children may still be rule targets).
    exact: HashMap<String, &'static str>,
    /// Trees that are never touched, no matter what.
    deny: Vec<(String, &'static str)>,
    /// Validated rule roots: the only places deletion may happen (strict descendants only).
    permit_dirs: Vec<String>,
    /// Individually permitted files (e.g. `MEMORY.DMP`).
    permit_files: HashSet<String>,
}

impl PathGuard {
    /// Builds the base guard (no permitted roots yet) for `env`. `extra_deny` is for Vexus's own
    /// data directories, wherever they happen to live.
    pub fn new(env: &Env, extra_deny: &[PathBuf]) -> Self {
        let mut g = PathGuard::default();

        if let Some(users) = env.user_profile.parent() {
            g.add_exact(users, "the Users folder");
        }
        g.add_exact(&env.user_profile, "your user profile");
        g.add_exact(&env.user_profile.join("AppData"), "the AppData folder");
        g.add_exact(&env.local_app_data, "the AppData\\Local folder");
        g.add_exact(&env.app_data, "the AppData\\Roaming folder");
        g.add_exact(&env.program_data, "the ProgramData folder");
        g.add_exact(&env.system_root, "the Windows folder");

        for sub in [
            "System32",
            "SysWOW64",
            "WinSxS",
            "servicing",
            "Installer",
            "Boot",
            "Fonts",
            "assembly",
            "Microsoft.NET",
            "SystemApps",
            "SystemResources",
            "INF",
            "Setup",
        ] {
            g.add_deny(&env.system_root.join(sub), "Windows system files");
        }
        g.add_deny(&env.program_files, "installed programs");
        if let Some(x86) = &env.program_files_x86 {
            g.add_deny(x86, "installed programs");
        }
        for sub in
            ["Recovery", "System Volume Information", "$Recycle.Bin", "Boot", "EFI", "$WinREAgent"]
        {
            g.add_deny(&env.system_drive.join(sub), "system recovery data");
        }
        for (path, label) in [
            (&env.desktop, "your Desktop"),
            (&env.documents, "your Documents"),
            (&env.downloads, "your Downloads"),
            (&env.music, "your Music"),
            (&env.pictures, "your Pictures"),
            (&env.videos, "your Videos"),
        ] {
            g.add_deny(path, label);
        }
        for od in &env.one_drive {
            g.add_deny(od, "your OneDrive");
        }
        for p in extra_deny {
            g.add_deny(p, "Vexus's own data");
        }
        g
    }

    /// Marks `root` — a folder some rule cleans the *inside* of — so that no walk can treat it as
    /// an item of its own, whichever other root it happens to sit inside. Its contents stay
    /// reachable through its own rule. Call this once every root has been permitted.
    pub fn shield_root(&mut self, root: &Path) {
        self.add_exact(root, "a folder that Vexus cleans the inside of");
    }

    fn add_exact(&mut self, path: &Path, label: &'static str) {
        for n in Self::forms(path) {
            self.exact.insert(n, label);
        }
    }

    fn add_deny(&mut self, path: &Path, label: &'static str) {
        for n in Self::forms(path) {
            if !self.deny.iter().any(|(d, _)| *d == n) {
                self.deny.push((n, label));
            }
        }
    }

    /// The normalised path plus, when it differs, its canonical (link-resolved) form — so a
    /// profile that is really a junction to another drive is protected under both names.
    fn forms(path: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(n) = normalize(path) {
            out.push(n);
        }
        if let Ok(c) = std::fs::canonicalize(path)
            && let Some(n) = normalize(&super::path::strip_verbatim(&c))
            && !out.contains(&n)
        {
            out.push(n);
        }
        out
    }

    /// Checks that `root` is an acceptable place to run a rule in. Does not register it.
    pub fn validate_root(&self, root: &Path) -> Result<String, RootRejection> {
        let n = normalize(root).ok_or(RootRejection::Unresolvable)?;
        if is_network(&n) {
            return Err(RootRejection::Network);
        }
        if is_drive_root(&n) {
            return Err(RootRejection::DriveRoot);
        }
        if let Some(label) = self.exact.get(&n) {
            return Err(RootRejection::Protected(label));
        }
        if let Some((_, label)) = self.deny.iter().find(|(d, _)| within_or_equal(&n, d)) {
            return Err(RootRejection::Protected(label));
        }
        // A root that contains protected locations is too broad to be a safe cleaning root.
        if let Some((_, label)) = self.exact.iter().find(|(e, _)| strictly_within(e, &n)) {
            return Err(RootRejection::TooBroad(label));
        }
        if let Some((_, label)) = self.deny.iter().find(|(d, _)| within_or_equal(d, &n)) {
            return Err(RootRejection::TooBroad(label));
        }
        Ok(n)
    }

    /// Validates `root` and, if acceptable, permits deletion strictly inside it.
    pub fn permit_root(&mut self, root: &Path) -> Result<(), RootRejection> {
        let n = self.validate_root(root)?;
        if !self.permit_dirs.contains(&n) {
            self.permit_dirs.push(n);
        }
        Ok(())
    }

    /// Permits exactly one file (never its folder).
    pub fn permit_file(&mut self, file: &Path) -> Result<(), RootRejection> {
        let n = normalize(file).ok_or(RootRejection::Unresolvable)?;
        if is_network(&n) {
            return Err(RootRejection::Network);
        }
        if is_drive_root(&n) {
            return Err(RootRejection::DriveRoot);
        }
        if let Some(label) = self.exact.get(&n) {
            return Err(RootRejection::Protected(label));
        }
        if let Some((_, label)) = self.deny.iter().find(|(d, _)| within_or_equal(&n, d)) {
            return Err(RootRejection::Protected(label));
        }
        self.permit_files.insert(n);
        Ok(())
    }

    /// `None` means the path may be touched; `Some(why)` means it must not be.
    pub fn is_protected(&self, path: &Path) -> Option<Protection> {
        let Some(n) = normalize(path) else { return Some(Protection::Unresolvable) };
        if is_network(&n) {
            return Some(Protection::Network);
        }
        if is_drive_root(&n) {
            return Some(Protection::DriveRoot);
        }
        if let Some(label) = self.exact.get(&n) {
            return Some(Protection::Folder(label));
        }
        if let Some((_, label)) = self.deny.iter().find(|(d, _)| within_or_equal(&n, d)) {
            return Some(Protection::Denied(label));
        }
        if self.permit_files.contains(&n) || self.permit_dirs.iter().any(|p| strictly_within(&n, p))
        {
            return None;
        }
        Some(Protection::NotPermitted)
    }
}

/// Names that Win32 cannot address unambiguously: it silently drops trailing dots and spaces, so
/// `keep.tmp.` and `keep.tmp` are the same file to most tools, and anything that checked one
/// could act on the other. No real application creates these (they need the `\\?\` form), so
/// they are never touched. Names that are not valid UTF-8 are in this class too: we cannot judge
/// them.
pub fn is_ambiguous_name(name: &OsStr) -> bool {
    name.to_str().is_none_or(|n| n.ends_with('.') || n.ends_with(' '))
}

/// Files Windows needs wherever they appear (page file, registry hives, boot files), plus the
/// ambiguous names of [`is_ambiguous_name`].
pub fn is_protected_file_name(name: &OsStr) -> bool {
    if is_ambiguous_name(name) {
        return true;
    }
    let Some(name) = name.to_str() else { return true };
    const EXACT: [&str; 6] =
        ["pagefile.sys", "hiberfil.sys", "swapfile.sys", "bootmgr", "bootnxt", "desktop.ini"];
    if EXACT.iter().any(|p| name.eq_ignore_ascii_case(p)) {
        return true;
    }
    ["ntuser.dat", "usrclass.dat"].iter().any(|p| {
        name.len() >= p.len() && name.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes())
    })
}

/// Sanity check for roots taken from user-controllable variables such as `%TEMP%`: something
/// (a user, an installer, malware) may have repointed it at a folder full of real work.
/// One of the last two path components must look like a temp folder (`Temp`, `tmp`, `Temp\1`).
pub fn looks_like_temp(path: &Path) -> bool {
    fn tempish(name: &str) -> bool {
        let s = name.to_ascii_lowercase();
        let s = s.trim_end_matches(|c: char| c.is_ascii_digit());
        matches!(s, "temp" | "tmp" | "temporary")
            || ["_temp", "-temp", "_tmp", "-tmp", ".tmp", ".temp"].iter().any(|x| s.ends_with(x))
    }
    path.components().rev().take(2).any(|c| tempish(&c.as_os_str().to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env::sandbox(Path::new(r"C:\Sandbox"))
    }

    fn guard() -> PathGuard {
        PathGuard::new(&env(), &[PathBuf::from(r"C:\Sandbox\Users\vexus\AppData\Local\Vexus")])
    }

    fn p(s: &str) -> &Path {
        Path::new(s)
    }

    #[test]
    fn default_deny_blocks_everything_not_permitted() {
        let g = guard();
        assert_eq!(
            g.is_protected(p(r"C:\Sandbox\Users\vexus\AppData\Local\Temp\a.tmp")),
            Some(Protection::NotPermitted)
        );
        assert_eq!(g.is_protected(p(r"C:\Random\thing.txt")), Some(Protection::NotPermitted));
    }

    #[test]
    fn permitted_root_allows_strict_descendants_only() {
        let mut g = guard();
        let temp = env().temp;
        g.permit_root(&temp).unwrap();
        assert_eq!(g.is_protected(&temp.join("a.tmp")), None);
        assert_eq!(g.is_protected(&temp.join("sub").join("b.tmp")), None);
        assert!(g.is_protected(&temp).is_some(), "the root itself is never an item");
        let sibling = PathBuf::from(format!("{}2", temp.display()));
        assert!(g.is_protected(&sibling.join("x")).is_some(), "shared prefix is not containment");
    }

    #[test]
    fn hard_denied_trees_win_over_everything() {
        let mut g = guard();
        let e = env();
        g.permit_root(&e.temp).unwrap();
        for denied in [
            e.system_root.join("System32").join("drivers").join("x.sys"),
            e.system_root.join("WinSxS").join("x"),
            e.system_root.join("Installer").join("x.msi"),
            e.program_files.join("App").join("app.exe"),
            e.documents.join("thesis.docx"),
            e.desktop.join("a.txt"),
            e.downloads.join("setup.exe"),
            e.pictures.join("holiday.jpg"),
            PathBuf::from(r"C:\Sandbox\Users\vexus\AppData\Local\Vexus\recovery\x"),
            PathBuf::from(r"C:\Sandbox\$Recycle.Bin\S-1\x"),
        ] {
            assert!(matches!(g.is_protected(&denied), Some(Protection::Denied(_))), "{denied:?}");
        }
    }

    #[test]
    fn well_known_folders_cannot_be_removed_themselves() {
        let g = guard();
        let e = env();
        for folder in
            [&e.system_root, &e.user_profile, &e.local_app_data, &e.app_data, &e.program_data]
        {
            assert!(matches!(g.is_protected(folder), Some(Protection::Folder(_))), "{folder:?}");
        }
        assert_eq!(g.is_protected(p(r"C:\")), Some(Protection::DriveRoot));
        assert_eq!(g.is_protected(p(r"D:\")), Some(Protection::DriveRoot));
    }

    #[test]
    fn path_tricks_do_not_bypass_the_denylist() {
        let g = guard();
        for sneaky in [
            r"C:\Sandbox\Windows.\System32\x.dll",
            r"C:\Sandbox\Windows \System32\x.dll",
            r"c:\SANDBOX\windows\SYSTEM32\x.dll",
            r"\\?\C:\Sandbox\Windows\System32\x.dll",
            r"C:/Sandbox/Windows/System32/x.dll",
        ] {
            assert!(matches!(g.is_protected(p(sneaky)), Some(Protection::Denied(_))), "{sneaky}");
        }
        for ambiguous in [
            r"C:\Sandbox\Windows\Temp\..\System32\x.dll",
            r"C:\Sandbox\Windows\Temp\x.dll:evil",
            r"\\.\C:",
            r"relative\x",
        ] {
            assert_eq!(g.is_protected(p(ambiguous)), Some(Protection::Unresolvable), "{ambiguous}");
        }
        assert_eq!(g.is_protected(p(r"\\server\share\x")), Some(Protection::Network));
    }

    #[test]
    fn hostile_roots_are_rejected() {
        let g = guard();
        let e = env();
        let cases: Vec<(PathBuf, &str)> = vec![
            (PathBuf::from(r"C:\"), "drive root"),
            (PathBuf::from(r"D:\"), "other drive root"),
            (e.system_root.clone(), "windows folder"),
            (e.user_profile.clone(), "profile (TEMP=%USERPROFILE%)"),
            (e.local_app_data.clone(), "local appdata"),
            (e.user_profile.join("AppData"), "appdata (contains Local/Roaming)"),
            (e.documents.clone(), "documents (TEMP=Documents)"),
            (e.documents.join("Work"), "inside documents"),
            (e.program_files.join("App"), "program files"),
            (e.system_root.join("System32").join("config"), "registry hives"),
            (PathBuf::from(r"\\srv\share\temp"), "network"),
            (PathBuf::from(r"temp"), "relative"),
            (PathBuf::from(r"C:\Sandbox\Users"), "contains every profile"),
            (PathBuf::from(r"C:\Sandbox"), "sandbox root is a drive-like root of denied trees"),
        ];
        for (root, why) in cases {
            assert!(g.validate_root(&root).is_err(), "should reject {root:?} ({why})");
        }
        assert!(g.validate_root(&e.temp).is_ok());
        assert!(g.validate_root(&e.system_root.join("Temp")).is_ok());
        assert!(g.validate_root(&e.system_root.join("Prefetch")).is_ok());
    }

    #[test]
    fn rejected_roots_are_never_permitted() {
        let mut g = guard();
        let e = env();
        assert!(g.permit_root(&e.user_profile).is_err());
        assert!(g.permit_root(p(r"C:\")).is_err());
        assert_eq!(
            g.is_protected(&e.user_profile.join("Notes.txt")),
            Some(Protection::NotPermitted)
        );
    }

    #[test]
    fn single_file_permits_do_not_open_the_folder() {
        let mut g = guard();
        let e = env();
        let dump = e.system_root.join("MEMORY.DMP");
        g.permit_file(&dump).unwrap();
        assert_eq!(g.is_protected(&dump), None);
        assert!(g.is_protected(&e.system_root.join("notepad.exe")).is_some());
        assert!(g.is_protected(&e.system_root).is_some());
        assert!(g.permit_file(&e.system_root.join("System32").join("ntdll.dll")).is_err());
    }

    #[test]
    fn protected_file_names() {
        for n in [
            "pagefile.sys",
            "HIBERFIL.SYS",
            "NTUSER.DAT",
            "ntuser.dat.LOG1",
            "UsrClass.dat",
            "desktop.ini",
        ] {
            assert!(is_protected_file_name(OsStr::new(n)), "{n}");
        }
        for n in ["notes.txt", "thumbcache_256.db", "ntuser.txt"] {
            assert!(!is_protected_file_name(OsStr::new(n)), "{n}");
        }
    }

    #[test]
    fn temp_sanity_check() {
        for ok in [
            r"C:\Users\Bob\AppData\Local\Temp",
            r"C:\Temp",
            r"D:\tmp",
            r"C:\Users\Bob\AppData\Local\Temp\2",
            r"E:\scratch_temp",
        ] {
            assert!(looks_like_temp(p(ok)), "{ok}");
        }
        for bad in [r"C:\Users\Bob\Documents", r"D:\Work", r"D:\Templates", r"C:\Users\Bob"] {
            assert!(!looks_like_temp(p(bad)), "{bad}");
        }
    }
}
