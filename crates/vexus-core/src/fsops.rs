//! Race-free file system primitives.
//!
//! Path-based `check, then delete` sequences are vulnerable to time-of-check/time-of-use races:
//! another process can swap a folder for a junction between the check and the delete, which
//! turns a harmless cleaner into an arbitrary-file-deleter (a classic privilege-escalation bug
//! when the cleaner runs elevated). Everything destructive in Vexus therefore goes through
//! [`Node`]: open the object **without following links**, verify facts through the *handle*
//! (final path, attributes, size, timestamp), and then delete or move *through that same handle*.

use std::ffi::{OsString, c_void};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX,
    FILE_DISPOSITION_INFO_EX_FLAGS, FILE_NAME_NORMALIZED, FILE_RENAME_INFO, FileDispositionInfo,
    FileDispositionInfoEx, FileRenameInfo, GETFINALPATHNAMEBYHANDLE_FLAGS,
    GetFinalPathNameByHandleW, SetFileInformationByHandle, VOLUME_NAME_DOS,
};

use crate::safety::strip_verbatim;

pub const ATTR_READONLY: u32 = 0x1;
pub const ATTR_DIRECTORY: u32 = 0x10;
pub const ATTR_REPARSE_POINT: u32 = 0x400;

const DELETE: u32 = 0x0001_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
const GENERIC_READ: u32 = 0x8000_0000;
const FILE_LIST_DIRECTORY: u32 = 0x0000_0001;
const FILE_WRITE_DATA: u32 = 0x0000_0002;
const SHARE_ALL: u32 = 0x7; // FILE_SHARE_READ | WRITE | DELETE
/// Removal opens let others read and delete but not write: a file somebody is writing to is in use.
const SHARE_READ_DELETE: u32 = 0x1 | 0x4;
/// Pinned folders let others read and write inside but not delete or rename.
const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
const FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000; // required to open directories
const FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000; // never follow a link in the final component

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PATH_NOT_FOUND: i32 = 3;
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_NOT_SAME_DEVICE: i32 = 17;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;
const ERROR_NOT_SUPPORTED: i32 = 50;
const ERROR_INVALID_PARAMETER: i32 = 87;
const ERROR_DIR_NOT_EMPTY: i32 = 145;

/// True if the metadata describes a symlink, junction or other reparse point.
pub fn is_reparse(md: &std::fs::Metadata) -> bool {
    md.file_attributes() & ATTR_REPARSE_POINT != 0
}

/// Resolves `path` (following links) to a clean drive-letter path, requiring a directory.
pub fn canonical_dir(path: &Path) -> io::Result<PathBuf> {
    let canon = std::fs::canonicalize(path)?;
    if !std::fs::metadata(&canon)?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a directory"));
    }
    Ok(strip_verbatim(&canon))
}

/// Why a node could not be opened for a destructive operation.
#[derive(Debug)]
pub enum OpenFailure {
    /// Already gone — usually fine (something else cleaned it first).
    Gone,
    /// Another process holds it open without allowing deletion.
    InUse,
    /// Needs more privilege (or the ACL forbids deletion).
    Denied,
    /// It is a symlink/junction/reparse point; Vexus never follows or removes those.
    Link,
    Other(io::Error),
}

fn classify_open(e: io::Error) -> OpenFailure {
    match e.raw_os_error() {
        Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => OpenFailure::Gone,
        Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION) => OpenFailure::InUse,
        Some(ERROR_ACCESS_DENIED) => OpenFailure::Denied,
        _ => OpenFailure::Other(e),
    }
}

/// An open, verified file-system object (file or directory).
///
/// Holding the handle pins the object: while it exists the thing we inspected is the thing we
/// act on, whatever happens to the path in the meantime.
#[derive(Debug)]
pub struct Node {
    file: File,
    /// Final path of the opened object as resolved by the OS (links already resolved in any
    /// *parent* component), without the `\\?\` prefix. Safety checks use this, never the input.
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
}

impl Node {
    /// When this object last arrived or changed, for age checks: see [`last_touched`].
    pub fn touched(&self) -> Option<SystemTime> {
        last_touched(self.modified, self.created)
    }
}

/// The newer of a file's modification and creation times.
///
/// Copying or extracting a file keeps its old modification time but stamps a new creation time,
/// and a file like that is new to this PC: "never touch anything newer than N days" has to hold
/// for it. An unknown modification time stays unknown (and so counts as recent).
pub fn last_touched(
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
) -> Option<SystemTime> {
    match (modified, created) {
        (Some(m), Some(c)) => Some(m.max(c)),
        (m, _) => m,
    }
}

/// [`last_touched`] for directory-listing metadata.
pub fn touched_of(md: &std::fs::Metadata) -> Option<SystemTime> {
    last_touched(md.modified().ok(), md.created().ok())
}

/// Opens `path` for deletion/move without following a link in the final component.
/// Pass `read = true` when the content may need to be copied (cross-volume recovery).
///
/// This is the *permissive* open, for looking at things: it shares everything, so it can never
/// disturb another program. Use [`open_for_removal`] when something is about to be removed.
pub fn open_node(path: &Path, read: bool) -> Result<Node, OpenFailure> {
    open_with(path, read, SHARE_ALL)
}

/// Opens `path` to remove or move it, which is stricter than looking at it:
///
/// * a file another program has open **for writing** is refused ([`OpenFailure::InUse`]) even
///   if that program allows deletion, because removing it would cut the writer off from its
///   own file (typical for loggers);
/// * with `read = true` (the file is about to be *moved* into Recovery, and Windows lets a
///   running program be renamed) a file that is open in a way that forbids writers — a loaded
///   program or library — is refused as well.
///
/// The handle is held only for the moment of the operation.
pub fn open_for_removal(path: &Path, read: bool) -> Result<Node, OpenFailure> {
    if read {
        // Must happen before the real open: that one denies writers and would trip over itself.
        let probe = OpenOptions::new()
            .access_mode(FILE_WRITE_DATA)
            .share_mode(SHARE_ALL)
            .custom_flags(FLAG_BACKUP_SEMANTICS | FLAG_OPEN_REPARSE_POINT)
            .open(to_verbatim(path));
        // Opened (nobody forbids writers) or cannot tell (no write permission, already gone…):
        // the real open below reports whatever matters.
        if let Err(e) = probe
            && matches!(e.raw_os_error(), Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION))
        {
            return Err(OpenFailure::InUse);
        }
    }
    open_with(path, read, SHARE_READ_DELETE)
}

fn open_with(path: &Path, read: bool, share: u32) -> Result<Node, OpenFailure> {
    let access = DELETE | FILE_READ_ATTRIBUTES | if read { GENERIC_READ } else { 0 };
    // Verbatim: Win32 would otherwise silently drop trailing dots and spaces, and "a.txt." would
    // open "a.txt" — a different file from the one that was checked.
    let file = OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags(FLAG_BACKUP_SEMANTICS | FLAG_OPEN_REPARSE_POINT)
        .open(to_verbatim(path))
        .map_err(classify_open)?;
    let md = file.metadata().map_err(OpenFailure::Other)?;
    let attrs = md.file_attributes();
    if attrs & ATTR_REPARSE_POINT != 0 {
        return Err(OpenFailure::Link);
    }
    let path = final_path(&file).map_err(OpenFailure::Other)?;
    Ok(Node {
        file,
        path,
        is_dir: attrs & ATTR_DIRECTORY != 0,
        size: if attrs & ATTR_DIRECTORY != 0 { 0 } else { md.len() },
        modified: md.modified().ok(),
        created: md.created().ok(),
    })
}

fn open_dir_handle(path: &Path, access: u32, share: u32) -> Result<(File, PathBuf), OpenFailure> {
    let file = OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags(FLAG_BACKUP_SEMANTICS | FLAG_OPEN_REPARSE_POINT)
        .open(to_verbatim(path))
        .map_err(classify_open)?;
    let attrs = file.metadata().map_err(OpenFailure::Other)?.file_attributes();
    if attrs & ATTR_REPARSE_POINT != 0 {
        return Err(OpenFailure::Link);
    }
    if attrs & ATTR_DIRECTORY == 0 {
        return Err(OpenFailure::Other(io::Error::new(
            io::ErrorKind::NotADirectory,
            "not a folder",
        )));
    }
    let real = final_path(&file).map_err(OpenFailure::Other)?;
    Ok((file, real))
}

/// Resolves the real location of a folder without following a link in its final component,
/// without asking for delete rights (a folder we may empty but not remove is still inspectable).
///
/// Used to re-verify a planned folder right before it is listed: a junction swapped in after the
/// scan is refused with [`OpenFailure::Link`] instead of being walked. The handle is released
/// immediately; use [`PinnedDir`] to keep a folder from changing.
pub fn inspect_dir(path: &Path) -> Result<PathBuf, OpenFailure> {
    open_dir_handle(path, FILE_READ_ATTRIBUTES, SHARE_ALL).map(|(_, real)| real)
}

/// A folder that is held open so it cannot be removed, renamed or swapped for a link while the
/// value lives. The handle is opened without `FILE_SHARE_DELETE`, which also stops its
/// ancestors from being renamed (the reason a shell cannot rename a folder it is "in").
///
/// Anything written *into* a pinned folder by path lands in the folder that was verified, which
/// is what makes restoring files race-free.
#[derive(Debug)]
pub struct PinnedDir {
    _file: File,
    path: PathBuf,
}

impl PinnedDir {
    /// Pins `path`. Refuses links ([`OpenFailure::Link`]) and anything that is not a folder.
    pub fn open(path: &Path) -> Result<PinnedDir, OpenFailure> {
        // NTFS ignores the share mode of handles that only ask for attribute access, so the
        // handle must hold an access right the share mode governs. Listing rights are enough
        // (and, unlike DELETE, we are allowed them on folders such as C:\Windows): with them
        // "nobody else may delete or rename this" is enforced.
        let (file, path) =
            open_dir_handle(path, FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES, SHARE_READ_WRITE)?;
        Ok(PinnedDir { _file: file, path })
    }

    /// Where the pinned folder really is (final path, no `\\?\` prefix).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Node {
    /// Deletes the object through the open handle (POSIX semantics where the file system allows,
    /// so the name disappears immediately even if other handles linger). Read-only files are
    /// handled. Fails with `ERROR_DIR_NOT_EMPTY` for non-empty directories, by design.
    pub fn delete(self) -> io::Result<()> {
        let handle = HANDLE(self.file.as_raw_handle());
        let ex = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
                FILE_DISPOSITION_FLAG_DELETE.0
                    | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                    | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
            ),
        };
        // SAFETY: `handle` stays valid for the call (owned by `self.file`); `ex` is a properly
        // initialised FILE_DISPOSITION_INFO_EX and the size passed matches it exactly.
        let result = unsafe {
            SetFileInformationByHandle(
                handle,
                FileDispositionInfoEx,
                (&raw const ex).cast::<c_void>(),
                size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        };
        match result.map_err(win_to_io) {
            Ok(()) => Ok(()),
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED)
                ) =>
            {
                // File systems without POSIX-delete support (FAT/exFAT): classic disposition.
                let legacy = FILE_DISPOSITION_INFO { DeleteFile: true };
                // SAFETY: as above, with the legacy struct.
                unsafe {
                    SetFileInformationByHandle(
                        handle,
                        FileDispositionInfo,
                        (&raw const legacy).cast::<c_void>(),
                        size_of::<FILE_DISPOSITION_INFO>() as u32,
                    )
                }
                .map_err(win_to_io)
            }
            Err(e) => Err(e),
        }
    }

    /// Moves the object to `dest` through the open handle. Never overwrites: if `dest` exists
    /// the call fails. Same-volume only; a cross-volume attempt fails with
    /// [`is_cross_volume`] true, and the caller falls back to [`Node::copy_to`] + [`Node::delete`].
    pub fn rename_to(&self, dest: &Path) -> io::Result<()> {
        let wide: Vec<u16> = to_verbatim(dest).as_os_str().encode_wide().collect();
        let name_off = offset_of!(FILE_RENAME_INFO, FileName);
        let total = name_off + wide.len() * 2;
        // u64 backing store guarantees the alignment FILE_RENAME_INFO needs.
        let mut buf = vec![0u64; total.div_ceil(8) + 1];
        let base = buf.as_mut_ptr().cast::<u8>();
        // SAFETY: `buf` is large enough for the header plus `wide.len()` UTF-16 units and is
        // 8-byte aligned. We initialise every field we rely on before the call, the name bytes
        // are copied within the allocation, and the handle outlives the call.
        unsafe {
            let info = base.cast::<FILE_RENAME_INFO>();
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = HANDLE::default();
            (*info).FileNameLength = (wide.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                wide.as_ptr(),
                base.add(name_off).cast::<u16>(),
                wide.len(),
            );
            SetFileInformationByHandle(
                HANDLE(self.file.as_raw_handle()),
                FileRenameInfo,
                info.cast::<c_void>(),
                total as u32,
            )
        }
        .map_err(win_to_io)
    }

    /// Copies the content to a *new* file at `dest` (fails if it exists), verifies that every
    /// byte arrived, and preserves the modification and creation times. Requires the node to
    /// have been opened with `read = true`.
    ///
    /// On any failure after the file was created it removes its own partial output — and only
    /// that: a `dest` that already existed belongs to somebody else and is never touched.
    pub fn copy_to(&self, dest: &Path) -> io::Result<u64> {
        use std::os::windows::fs::FileTimesExt;
        let mut out = OpenOptions::new().write(true).create_new(true).open(dest)?;
        let copied = (|| {
            let n = io::copy(&mut &self.file, &mut out)?;
            if n != self.size {
                return Err(io::Error::other(format!(
                    "copied {n} of {} bytes; the file changed while it was being copied",
                    self.size
                )));
            }
            let mut times = std::fs::FileTimes::new();
            if let Some(m) = self.modified {
                times = times.set_modified(m);
            }
            if let Some(c) = self.created {
                times = times.set_created(c);
            }
            let _ = out.set_times(times);
            out.sync_all()?;
            Ok(n)
        })();
        if copied.is_err() {
            drop(out);
            let _ = std::fs::remove_file(dest);
        }
        copied
    }
}

/// True if `e` is the "different volume" failure from [`Node::rename_to`].
pub fn is_cross_volume(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE)
}

/// True if `e` means a directory still has content.
pub fn is_dir_not_empty(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_DIR_NOT_EMPTY)
}

fn final_path(file: &File) -> io::Result<PathBuf> {
    let handle = HANDLE(file.as_raw_handle());
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: `handle` is valid while `file` lives and `buf` is a valid writable slice.
        let n = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                &mut buf,
                GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
            )
        } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n < buf.len() {
            buf.truncate(n);
            return Ok(strip_verbatim(Path::new(&OsString::from_wide(&buf))));
        }
        buf.resize(n + 1, 0); // too small: `n` is the required size including the NUL
    }
}

/// `C:\a\b` → `\\?\C:\a\b` so deep paths beyond MAX_PATH work.
fn to_verbatim(path: &Path) -> PathBuf {
    let s = path.as_os_str().to_string_lossy();
    if s.starts_with(r"\\?\") || !path.is_absolute() {
        path.to_path_buf()
    } else if let Some(rest) = s.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{rest}"))
    } else {
        PathBuf::from(format!(r"\\?\{s}"))
    }
}

pub(crate) fn win_to_io(e: windows::core::Error) -> io::Error {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((hr & 0xFFFF) as i32)
    } else {
        io::Error::other(e)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn deletes_files_including_read_only_ones() {
        let d = tmp();
        let f = d.path().join("a.txt");
        fs::write(&f, b"hello").unwrap();
        let node = open_node(&f, false).unwrap();
        assert_eq!(node.size, 5);
        assert!(!node.is_dir);
        node.delete().unwrap();
        assert!(!f.exists());

        let ro = d.path().join("ro.txt");
        fs::write(&ro, b"x").unwrap();
        let mut perm = fs::metadata(&ro).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&ro, perm).unwrap();
        open_node(&ro, false).unwrap().delete().unwrap();
        assert!(!ro.exists(), "read-only files must be deletable");
    }

    #[test]
    fn missing_paths_report_gone() {
        let d = tmp();
        assert!(matches!(open_node(&d.path().join("nope"), false), Err(OpenFailure::Gone)));
        assert!(matches!(open_node(&d.path().join("a").join("b"), false), Err(OpenFailure::Gone)));
    }

    #[test]
    fn files_held_open_without_delete_sharing_are_in_use() {
        let d = tmp();
        let f = d.path().join("locked.dat");
        fs::write(&f, b"x").unwrap();
        let _held = OpenOptions::new().read(true).share_mode(1 /* READ only */).open(&f).unwrap();
        assert!(matches!(open_node(&f, false), Err(OpenFailure::InUse)));
        assert!(f.exists());
    }

    #[test]
    fn directories_delete_only_when_empty() {
        let d = tmp();
        let sub = d.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("x"), b"x").unwrap();
        let node = open_node(&sub, false).unwrap();
        assert!(node.is_dir);
        let err = node.delete().unwrap_err();
        assert!(is_dir_not_empty(&err), "{err:?}");
        fs::remove_file(sub.join("x")).unwrap();
        open_node(&sub, false).unwrap().delete().unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn junctions_are_never_followed_or_removed() {
        let d = tmp();
        let target = d.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("precious.txt"), b"keep me").unwrap();
        let link = d.path().join("link");
        let ok = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("skipping: could not create a junction in this environment");
            return;
        }
        assert!(matches!(open_node(&link, false), Err(OpenFailure::Link)));
        // Reaching a file *through* the junction resolves to the real location, so callers that
        // compare `Node::path` with their root can see the escape.
        let through = open_node(&link.join("precious.txt"), false).unwrap();
        assert!(through.path.starts_with(&target) || through.path.ends_with("precious.txt"));
        assert!(!through.path.starts_with(&link), "final path must be the resolved location");
        drop(through);
        assert!(target.join("precious.txt").exists());
    }

    #[test]
    fn rename_moves_without_overwriting() {
        let d = tmp();
        let src = d.path().join("src.txt");
        let dst = d.path().join("dst.txt");
        fs::write(&src, b"payload").unwrap();
        open_node(&src, false).unwrap().rename_to(&dst).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).unwrap(), b"payload");

        let other = d.path().join("other.txt");
        fs::write(&other, b"other").unwrap();
        let err = open_node(&other, false).unwrap().rename_to(&dst).unwrap_err();
        assert!(matches!(err.raw_os_error(), Some(80 | 183)), "expected exists error, got {err:?}");
        assert_eq!(fs::read(&dst).unwrap(), b"payload", "destination must be untouched");
        assert!(other.exists(), "source must be untouched on failure");
    }

    #[test]
    fn rename_supports_very_long_destinations() {
        let d = tmp();
        let src = d.path().join("src.txt");
        fs::write(&src, b"x").unwrap();
        let mut deep = d.path().to_path_buf();
        for _ in 0..12 {
            deep.push("a_fairly_long_directory_name_to_exceed_max_path");
        }
        fs::create_dir_all(&deep).unwrap();
        let dst = deep.join("moved.txt");
        open_node(&src, false).unwrap().rename_to(&dst).unwrap();
        assert!(dst.exists());
    }

    #[test]
    fn copy_preserves_content_and_mtime() {
        let d = tmp();
        let src = d.path().join("src.bin");
        fs::write(&src, vec![7u8; 10_000]).unwrap();
        let old = SystemTime::now() - std::time::Duration::from_secs(86_400 * 30);
        File::options().write(true).open(&src).unwrap().set_modified(old).unwrap();
        let node = open_node(&src, true).unwrap();
        let dst = d.path().join("copy.bin");
        assert_eq!(node.copy_to(&dst).unwrap(), 10_000);
        assert_eq!(fs::read(&dst).unwrap().len(), 10_000);
        let got = fs::metadata(&dst).unwrap().modified().unwrap();
        let diff = got.duration_since(old).unwrap_or_else(|e| e.duration());
        assert!(diff.as_secs() < 2, "mtime not preserved");
        assert!(node.copy_to(&dst).is_err(), "must not overwrite");
    }

    #[test]
    fn final_path_resolves_to_clean_drive_path() {
        let d = tmp();
        let f = d.path().join("x.txt");
        fs::write(&f, b"x").unwrap();
        let node = open_node(&f, false).unwrap();
        let s = node.path.to_string_lossy();
        assert!(!s.starts_with(r"\\?\"), "{s}");
        assert!(s.ends_with("x.txt"));
        assert!(canonical_dir(d.path()).unwrap().is_absolute());
    }

    fn junction(link: &Path, target: &Path) -> bool {
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn a_pinned_folder_cannot_be_removed_renamed_or_swapped() {
        let d = tmp();
        let outer = d.path().join("outer");
        let dir = outer.join("pinned");
        fs::create_dir_all(&dir).unwrap();
        let pin = PinnedDir::open(&dir).unwrap();
        assert!(fs::remove_dir(&dir).is_err(), "removable while pinned");
        assert!(fs::rename(&dir, outer.join("moved")).is_err(), "renamable while pinned");
        assert!(
            fs::rename(&outer, d.path().join("moved-outer")).is_err(),
            "an ancestor was renamed out from under the pin"
        );
        drop(pin);
        fs::rename(&outer, d.path().join("moved-outer")).unwrap();
    }

    #[test]
    fn pinning_refuses_links_files_and_missing_folders() {
        let d = tmp();
        let target = d.path().join("target");
        fs::create_dir(&target).unwrap();
        let f = d.path().join("f.txt");
        fs::write(&f, b"x").unwrap();
        assert!(matches!(PinnedDir::open(&f), Err(OpenFailure::Other(_))));
        assert!(matches!(PinnedDir::open(&d.path().join("missing")), Err(OpenFailure::Gone)));
        let link = d.path().join("link");
        if junction(&link, &target) {
            assert!(matches!(PinnedDir::open(&link), Err(OpenFailure::Link)));
        } else {
            eprintln!("skipping the link half: cannot create junctions here");
        }
        let pin = PinnedDir::open(&target).unwrap();
        assert_eq!(pin.path(), canonical_dir(&target).unwrap());
    }

    #[test]
    fn inspecting_a_folder_reports_its_real_location_and_refuses_links() {
        let d = tmp();
        let target = d.path().join("target");
        fs::create_dir(&target).unwrap();
        assert_eq!(inspect_dir(&target).unwrap(), canonical_dir(&target).unwrap());
        assert!(matches!(inspect_dir(&d.path().join("missing")), Err(OpenFailure::Gone)));
        let link = d.path().join("link");
        if junction(&link, &target) {
            assert!(matches!(inspect_dir(&link), Err(OpenFailure::Link)));
        }
    }

    #[test]
    fn removal_refuses_files_that_are_being_written_but_probing_does_not() {
        let d = tmp();
        let f = d.path().join("log.txt");
        fs::write(&f, b"x").unwrap();
        // Another program has it open for writing yet allows deletion (typical for loggers).
        let writer = OpenOptions::new().write(true).share_mode(7).open(&f).unwrap();
        assert!(open_node(&f, false).is_ok(), "a read-only probe must not disturb the writer");
        assert!(matches!(open_for_removal(&f, false), Err(OpenFailure::InUse)));
        assert!(matches!(open_for_removal(&f, true), Err(OpenFailure::InUse)));
        drop(writer);
        assert!(open_for_removal(&f, false).is_ok());
    }

    #[test]
    fn names_with_trailing_dots_open_exactly_that_name() {
        let d = tmp();
        let plain = d.path().join("a.txt");
        fs::write(&plain, b"plain").unwrap();
        // Only the verbatim form can create the name…
        let dotted = PathBuf::from(format!(r"\\?\{}\a.txt.", d.path().display()));
        fs::write(&dotted, b"dotted").unwrap();
        // …but directory listings hand callers the plain form, which Win32 would normalise
        // to "a.txt": opening must still reach the file that really has that name.
        let as_listed = d.path().join("a.txt.");
        let node = open_node(&as_listed, true).unwrap();
        assert!(node.path.to_string_lossy().ends_with("a.txt."), "{:?}", node.path);
        assert_eq!(node.size, 6, "opened the look-alike instead");
        drop(node);
        let _ = fs::remove_file(&dotted);
        assert!(plain.exists());
    }
}
