//! The recovery store: Vexus's undo.
//!
//! Undoable removals are *moved* (a same-volume rename, so effectively instant) into
//! `<store>/<batch-id>/files/<drive letter>/<original path…>`. The layout mirrors the original
//! location, which makes every batch self-describing: restoring needs nothing but the folder
//! structure, so a crash mid-clean (or a lost manifest) can never strand data.
//!
//! Batches expire after a configurable number of days and are then purged for good.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::fsops::{OpenFailure, PinnedDir, canonical_dir, is_cross_volume, is_reparse, open_node};
use crate::progress::CancelToken;
use crate::safety::{PathGuard, is_protected_file_name, is_strictly_within, normalize};
use crate::util::{now_unix, random_u64, stamp, write_atomic};

const MANIFEST: &str = "batch.json";
const FILES: &str = "files";
const ORPHAN_TTL_DAYS: i64 = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInfo {
    pub id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub label: String,
    pub files: u64,
    pub bytes: u64,
    pub categories: Vec<String>,
    /// False while a clean is running (or if it was interrupted). Restorable either way.
    pub complete: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub restored_files: u64,
    pub restored_bytes: u64,
    /// Something already exists at the original path; the recovered copy was left in place.
    pub conflicts: u64,
    /// The original location is no longer one Vexus manages, so it was left in place.
    pub refused: u64,
    pub failed: u64,
    /// Everything was restored and the (now empty) batch was removed.
    pub batch_removed: bool,
}

#[derive(Debug, Clone)]
pub struct RecoveryStore {
    root: PathBuf,
}

/// An open batch that a clean is writing into.
#[derive(Debug)]
pub struct Batch {
    dir: PathBuf,
    files_root: PathBuf,
    info: Mutex<BatchInfo>,
    files: AtomicU64,
    bytes: AtomicU64,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

impl RecoveryStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        RecoveryStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Opens a new batch. Nothing is created on disk for an empty clean until a file is moved.
    pub fn begin(&self, label: &str, ttl_days: u32, categories: Vec<String>) -> io::Result<Batch> {
        let now = now_unix();
        fs::create_dir_all(&self.root)?;
        // `create_dir` (not `_all`) so a folder that already exists is never reused: two batches
        // sharing one would let the empty one's cleanup delete the other's data.
        let (id, dir) = {
            let mut attempt = 0;
            loop {
                let id = format!("{}-{:08x}", stamp(now), random_u64() & 0xffff_ffff);
                let dir = self.root.join(&id);
                match fs::create_dir(&dir) {
                    Ok(()) => break (id, dir),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 32 => {
                        attempt += 1;
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        let files_root = dir.join(FILES);
        fs::create_dir(&files_root)?;
        let info = BatchInfo {
            id,
            created_at: now,
            expires_at: now + i64::from(ttl_days.max(1)) * 86_400,
            label: label.to_string(),
            files: 0,
            bytes: 0,
            categories,
            complete: false,
        };
        write_manifest(&dir, &info)?;
        Ok(Batch {
            dir,
            files_root,
            info: Mutex::new(info),
            files: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        })
    }

    /// All batches, newest first. Damaged or half-written batch folders are still listed so the
    /// data inside can be restored.
    pub fn list(&self) -> Vec<BatchInfo> {
        let Ok(rd) = fs::read_dir(&self.root) else { return Vec::new() };
        let mut out: Vec<BatchInfo> = rd
            .flatten()
            .filter(|e| e.metadata().is_ok_and(|m| m.is_dir() && !is_reparse(&m)))
            .filter_map(|e| {
                let id = e.file_name().to_string_lossy().into_owned();
                valid_id(&id).then(|| {
                    read_manifest(&e.path()).unwrap_or_else(|| recover_info(&e.path(), &id))
                })
            })
            .collect();
        out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        out
    }

    pub fn total_bytes(&self) -> u64 {
        self.list().iter().map(|b| b.bytes).sum()
    }

    /// Permanently deletes one batch. Returns the bytes released.
    pub fn purge(&self, id: &str) -> Result<u64> {
        let dir = self.batch_dir(id)?;
        let bytes = read_manifest(&dir).map_or(0, |i| i.bytes);
        fs::remove_dir_all(&dir).map_err(|e| Error::io("Couldn't free the recovery copy", e))?;
        tracing::info!(batch = id, bytes, "recovery batch purged");
        Ok(bytes)
    }

    /// Purges every batch whose time is up. Returns the ids removed.
    pub fn purge_expired(&self, now: i64) -> Vec<String> {
        self.list()
            .into_iter()
            .filter(|b| b.complete && b.expires_at <= now)
            .filter(|b| self.purge(&b.id).is_ok())
            .map(|b| b.id)
            .collect()
    }

    /// Moves a batch's files back to where they came from.
    ///
    /// Restoration is as guarded as cleaning: every destination must still be inside a location
    /// the current rules manage (`guard`), so a tampered recovery folder cannot be used to write
    /// files somewhere sensitive. Existing files are never overwritten.
    pub fn restore(
        &self,
        id: &str,
        guard: &PathGuard,
        cancel: &CancelToken,
    ) -> Result<RestoreReport> {
        let dir = self.batch_dir(id)?;
        let files_root = dir.join(FILES);
        // Where the stored copies really are. Each one is checked against this through its own
        // handle, so a link planted anywhere above it cannot make us move something else.
        let files_real = canonical_dir(&files_root).unwrap_or_else(|_| files_root.clone());
        let mut report = RestoreReport::default();
        let mut stored = Vec::new();
        collect_files(&files_root, &mut stored);

        // Destination folders are pinned for the whole restore: from the moment one is verified
        // it cannot be removed, renamed or swapped for a link, so every rename below lands in
        // the folder that was checked.
        let mut pins = Pins::default();
        for file in stored {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let Some(original) = original_path(&files_root, &file) else {
                report.refused += 1;
                continue;
            };
            let name_protected = original.file_name().is_none_or(is_protected_file_name);
            if name_protected || guard.is_protected(&original).is_some() {
                report.refused += 1;
                continue;
            }
            let Some(parent) = original.parent() else {
                report.refused += 1;
                continue;
            };
            match pins.ensure(parent, guard) {
                Ok(()) => {}
                Err(PinFail::Refused) => {
                    report.refused += 1;
                    continue;
                }
                Err(PinFail::Failed) => {
                    report.failed += 1;
                    continue;
                }
            }
            match restore_one(&file, &original, &files_real) {
                Ok(size) => {
                    report.restored_files += 1;
                    report.restored_bytes += size;
                }
                Err(RestoreFail::Conflict) => report.conflicts += 1,
                Err(RestoreFail::Refused) => report.refused += 1,
                Err(RestoreFail::Other(e)) => {
                    tracing::warn!(file = %original.display(), "restore failed: {e}");
                    report.failed += 1;
                }
            }
        }
        drop(pins);

        prune_empty_dirs(&files_root);
        let mut leftover = Vec::new();
        collect_files(&files_root, &mut leftover);
        if leftover.is_empty() {
            let _ = fs::remove_dir_all(&dir);
            report.batch_removed = true;
        } else if let Some(mut info) = read_manifest(&dir) {
            info.files = leftover.len() as u64;
            info.bytes =
                leftover.iter().filter_map(|f| fs::metadata(f).ok()).map(|m| m.len()).sum();
            let _ = write_manifest(&dir, &info);
        }
        tracing::info!(
            batch = id,
            restored = report.restored_files,
            conflicts = report.conflicts,
            "restore finished"
        );
        Ok(report)
    }

    fn batch_dir(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(Error::UnknownBatch(id.to_string()));
        }
        let dir = self.root.join(id);
        // A batch folder that is a link could lead anywhere: it is not one of ours.
        match fs::symlink_metadata(&dir) {
            Ok(md) if md.is_dir() && !is_reparse(&md) => Ok(dir),
            _ => Err(Error::UnknownBatch(id.to_string())),
        }
    }
}

/// Why a destination folder could not be made ready.
enum PinFail {
    /// A link is in the way, or it is not a place the rules manage.
    Refused,
    Failed,
}

/// Destination folders held open (pinned) while one restore runs.
#[derive(Default)]
struct Pins {
    held: HashMap<String, PinnedDir>,
}

impl Pins {
    /// Makes sure `dir` exists as a real folder — not reached through a link — and is pinned.
    ///
    /// Missing folders are created one level at a time from the nearest existing ancestor, each
    /// pinned and verified before the next is created, so nothing is ever created behind a link.
    fn ensure(&mut self, dir: &Path, guard: &PathGuard) -> std::result::Result<(), PinFail> {
        let key = normalize(dir).ok_or(PinFail::Refused)?;
        if self.held.contains_key(&key) {
            return Ok(());
        }

        let mut missing: Vec<&Path> = Vec::new();
        let mut existing = dir;
        let base = loop {
            match PinnedDir::open(existing) {
                Ok(pin) => break pin,
                Err(OpenFailure::Gone) => {
                    missing.push(existing);
                    existing = existing.parent().ok_or(PinFail::Refused)?;
                }
                Err(OpenFailure::Link) => return Err(PinFail::Refused),
                Err(_) => return Err(PinFail::Failed),
            }
        };
        // The folder must be exactly where the stored path says. Anything else means a link
        // somewhere on the way, and the files are not ours to put there.
        if normalize(base.path()) != normalize(existing) {
            return Err(PinFail::Refused);
        }
        self.held.insert(normalize(existing).ok_or(PinFail::Refused)?, base);

        for next in missing.into_iter().rev() {
            // Never create anything above the folders the rules manage.
            if guard.is_protected(&next.join("_")).is_some() {
                return Err(PinFail::Refused);
            }
            match fs::create_dir(next) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {} // verified just below
                Err(_) => return Err(PinFail::Failed),
            }
            let pin = PinnedDir::open(next).map_err(|e| match e {
                OpenFailure::Link => PinFail::Refused,
                _ => PinFail::Failed,
            })?;
            if normalize(pin.path()) != normalize(next) {
                return Err(PinFail::Refused);
            }
            self.held.insert(normalize(next).ok_or(PinFail::Refused)?, pin);
        }
        Ok(())
    }
}

impl Batch {
    pub fn id(&self) -> String {
        self.info.lock().map(|i| i.id.clone()).unwrap_or_default()
    }

    /// Where `original` is stored inside this batch, mirroring its path. `None` if the path
    /// cannot be mirrored safely (relative, `..`, device path…).
    pub fn destination(&self, original: &Path) -> Option<PathBuf> {
        let mut dest = self.files_root.clone();
        let mut saw_prefix = false;
        for c in original.components() {
            match c {
                Component::Prefix(p) => {
                    match p.kind() {
                        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                            dest.push(char::from(d).to_ascii_uppercase().to_string());
                        }
                        _ => return None,
                    }
                    saw_prefix = true;
                }
                Component::RootDir => {}
                Component::Normal(n) => dest.push(n),
                Component::ParentDir | Component::CurDir => return None,
            }
        }
        saw_prefix.then_some(dest)
    }

    /// Counts a successfully moved file.
    pub fn record(&self, bytes: u64) {
        self.files.fetch_add(1, Relaxed);
        self.bytes.fetch_add(bytes, Relaxed);
    }

    /// Seals the batch. An empty batch is deleted and `None` is returned.
    pub fn finish(self) -> io::Result<Option<BatchInfo>> {
        let files = self.files.load(Relaxed);
        if files == 0 {
            let _ = fs::remove_dir_all(&self.dir);
            return Ok(None);
        }
        let mut info = self.info.into_inner().unwrap_or_else(|p| p.into_inner());
        info.files = files;
        info.bytes = self.bytes.load(Relaxed);
        info.complete = true;
        write_manifest(&self.dir, &info)?;
        Ok(Some(info))
    }
}

enum RestoreFail {
    Conflict,
    /// Not something a restore may do (the stored copy is not really inside the batch).
    Refused,
    Other(io::Error),
}

fn restore_one(
    stored: &Path,
    original: &Path,
    batch_files: &Path,
) -> std::result::Result<u64, RestoreFail> {
    let node = match open_node(stored, true) {
        Ok(n) => n,
        Err(OpenFailure::Gone) => return Err(RestoreFail::Other(io::ErrorKind::NotFound.into())),
        Err(OpenFailure::Link) => return Err(RestoreFail::Refused),
        Err(OpenFailure::Other(e)) => return Err(RestoreFail::Other(e)),
        Err(other) => return Err(RestoreFail::Other(io::Error::other(format!("{other:?}")))),
    };
    // Judge the object we actually opened, not the path we were given.
    if node.is_dir || !is_strictly_within(&node.path, batch_files) {
        return Err(RestoreFail::Refused);
    }
    let size = node.size;
    match node.rename_to(original) {
        Ok(()) => Ok(size),
        Err(e) if matches!(e.raw_os_error(), Some(80 | 183)) => Err(RestoreFail::Conflict),
        Err(e) if is_cross_volume(&e) => {
            node.copy_to(original).map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    RestoreFail::Conflict
                } else {
                    RestoreFail::Other(e)
                }
            })?;
            node.delete().map_err(RestoreFail::Other)?;
            Ok(size)
        }
        Err(e) => Err(RestoreFail::Other(e)),
    }
}

/// `files/C/Users/Bob/x.tmp` → `C:\Users\Bob\x.tmp`. Rejects anything that isn't a plain
/// `<single drive letter>/<normal components>` shape.
fn original_path(files_root: &Path, stored: &Path) -> Option<PathBuf> {
    let rel = stored.strip_prefix(files_root).ok()?;
    let mut comps = rel.components();
    let Component::Normal(drive) = comps.next()? else { return None };
    let drive = drive.to_str()?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let mut out = PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase()));
    let mut any = false;
    for c in comps {
        let Component::Normal(n) = c else { return None };
        out.push(n);
        any = true;
    }
    any.then_some(out)
}

/// A recovery batch mirrors original paths (drive + the walker's depth cap + the rule root), so
/// nothing legitimate is deeper than this; anything that is must have been tampered with and is
/// ignored rather than recursed into.
const MAX_STORE_DEPTH: usize = 192;

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    collect_files_at(dir, out, 0);
}

fn collect_files_at(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_STORE_DEPTH {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if is_reparse(&md) {
            continue; // we never store links, so never follow one found here
        }
        if md.is_dir() {
            collect_files_at(&e.path(), out, depth + 1);
        } else {
            out.push(e.path());
        }
    }
}

/// Removes empty directories bottom-up (never the root itself).
fn prune_empty_dirs(dir: &Path) {
    prune_empty_dirs_at(dir, 0);
}

fn prune_empty_dirs_at(dir: &Path, depth: usize) {
    if depth > MAX_STORE_DEPTH {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if e.metadata().is_ok_and(|m| m.is_dir() && !is_reparse(&m)) {
            prune_empty_dirs_at(&e.path(), depth + 1);
            let _ = fs::remove_dir(e.path());
        }
    }
}

fn write_manifest(dir: &Path, info: &BatchInfo) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(info).map_err(io::Error::other)?;
    write_atomic(&dir.join(MANIFEST), &json)
}

fn read_manifest(dir: &Path) -> Option<BatchInfo> {
    let bytes = fs::read(dir.join(MANIFEST)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Rebuilds minimal info for a batch whose manifest is missing or unreadable.
///
/// Such a batch is never `complete`, so it is never purged automatically: not being able to read
/// a file (a crash mid-write, an antivirus holding it) says nothing about how long the user was
/// promised to be able to undo. It stays until someone removes it on purpose.
fn recover_info(dir: &Path, id: &str) -> BatchInfo {
    let mut files = Vec::new();
    collect_files(&dir.join(FILES), &mut files);
    let bytes = files.iter().filter_map(|f| fs::metadata(f).ok()).map(|m| m.len()).sum();
    let created = fs::metadata(dir)
        .and_then(|m| m.created())
        .map(crate::util::unix_secs)
        .unwrap_or_else(|_| now_unix());
    BatchInfo {
        id: id.to_string(),
        created_at: created,
        expires_at: created + ORPHAN_TTL_DAYS * 86_400,
        label: "Recovered batch".to_string(),
        files: files.len() as u64,
        bytes,
        categories: Vec::new(),
        complete: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Env;

    fn store() -> (tempfile::TempDir, RecoveryStore) {
        let d = tempfile::tempdir().unwrap();
        let s = RecoveryStore::new(d.path().join("recovery"));
        (d, s)
    }

    #[test]
    fn destination_mirrors_the_original_path() {
        let (_d, s) = store();
        let b = s.begin("test", 7, vec![]).unwrap();
        let dest = b.destination(Path::new(r"C:\Users\Bob\AppData\Local\Temp\a.tmp")).unwrap();
        assert!(
            dest.ends_with(Path::new(r"files\C\Users\Bob\AppData\Local\Temp\a.tmp")),
            "{dest:?}"
        );
        assert_eq!(b.destination(Path::new(r"relative\a.tmp")), None);
        assert_eq!(b.destination(Path::new(r"C:\a\..\b")), None);
        assert_eq!(b.destination(Path::new(r"\\server\share\a")), None);
    }

    #[test]
    fn original_path_round_trips_and_rejects_oddities() {
        let root = Path::new(r"X:\store\b1\files");
        let stored = root.join("C").join("Users").join("Bob").join("a.tmp");
        assert_eq!(original_path(root, &stored), Some(PathBuf::from(r"C:\Users\Bob\a.tmp")));
        assert_eq!(
            original_path(root, &root.join("CC").join("a")),
            None,
            "drive must be one letter"
        );
        assert_eq!(original_path(root, &root.join("1").join("a")), None);
        assert_eq!(original_path(root, &root.join("C")), None, "a bare drive is not a file");
    }

    #[test]
    fn empty_batches_leave_nothing_behind() {
        let (_d, s) = store();
        let b = s.begin("noop", 7, vec![]).unwrap();
        assert!(b.finish().unwrap().is_none());
        assert!(s.list().is_empty());
    }

    #[test]
    fn ids_cannot_escape_the_store() {
        let (_d, s) = store();
        for bad in ["..", "../x", "a/b", r"a\b", "", "C:"] {
            assert!(matches!(s.purge(bad), Err(Error::UnknownBatch(_))), "{bad:?}");
        }
    }

    /// Moves a file into a batch the way the cleaner does.
    fn stash(b: &Batch, original: &Path) {
        let node = open_node(original, false).unwrap();
        let dest = b.destination(&node.path).unwrap();
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let size = node.size;
        node.rename_to(&dest).unwrap();
        b.record(size);
    }

    fn permissive_guard(root: &Path) -> PathGuard {
        let env = Env::sandbox(Path::new(r"C:\Nowhere"));
        let mut g = PathGuard::new(&env, &[]);
        g.permit_root(root).unwrap();
        g
    }

    #[test]
    fn stash_then_restore_round_trips() {
        let (d, s) = store();
        let work = canonical_dir(d.path()).unwrap().join("work");
        fs::create_dir_all(work.join("sub")).unwrap();
        fs::write(work.join("a.txt"), b"alpha").unwrap();
        fs::write(work.join("sub").join("b.txt"), b"bravo!").unwrap();

        let batch = s.begin("Temporary files", 7, vec!["windows.user-temp".into()]).unwrap();
        stash(&batch, &work.join("a.txt"));
        stash(&batch, &work.join("sub").join("b.txt"));
        let info = batch.finish().unwrap().unwrap();
        assert_eq!((info.files, info.bytes), (2, 11));
        assert!(info.complete);
        assert!(!work.join("a.txt").exists() && !work.join("sub").join("b.txt").exists());

        let listed = s.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, info.id);
        assert_eq!(s.total_bytes(), 11);

        let guard = permissive_guard(&work);
        let rep = s.restore(&info.id, &guard, &CancelToken::new()).unwrap();
        assert_eq!(
            (rep.restored_files, rep.restored_bytes, rep.conflicts, rep.refused),
            (2, 11, 0, 0)
        );
        assert!(rep.batch_removed);
        assert_eq!(fs::read(work.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(fs::read(work.join("sub").join("b.txt")).unwrap(), b"bravo!");
        assert!(s.list().is_empty());
    }

    #[test]
    fn restore_never_overwrites_and_keeps_the_batch() {
        let (d, s) = store();
        let work = canonical_dir(d.path()).unwrap().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("a.txt"), b"original").unwrap();
        let batch = s.begin("t", 7, vec![]).unwrap();
        stash(&batch, &work.join("a.txt"));
        let info = batch.finish().unwrap().unwrap();
        fs::write(work.join("a.txt"), b"new file created since").unwrap();

        let rep = s.restore(&info.id, &permissive_guard(&work), &CancelToken::new()).unwrap();
        assert_eq!((rep.restored_files, rep.conflicts), (0, 1));
        assert!(!rep.batch_removed, "the unrestored copy must stay recoverable");
        assert_eq!(fs::read(work.join("a.txt")).unwrap(), b"new file created since");
        assert_eq!(s.list()[0].files, 1);
    }

    #[test]
    fn restore_refuses_destinations_the_guard_does_not_manage() {
        let (d, s) = store();
        let work = canonical_dir(d.path()).unwrap().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("a.txt"), b"x").unwrap();
        let batch = s.begin("t", 7, vec![]).unwrap();
        stash(&batch, &work.join("a.txt"));
        let info = batch.finish().unwrap().unwrap();

        // A guard that no longer manages `work` (e.g. a tampered or stale batch).
        let env = Env::sandbox(Path::new(r"C:\Nowhere"));
        let guard = PathGuard::new(&env, &[]);
        let rep = s.restore(&info.id, &guard, &CancelToken::new()).unwrap();
        assert_eq!((rep.restored_files, rep.refused), (0, 1));
        assert!(!work.join("a.txt").exists());
        assert_eq!(s.list()[0].files, 1);
    }

    #[test]
    fn tampered_batches_cannot_write_into_protected_places() {
        let (d, s) = store();
        let batch = s.begin("evil", 7, vec![]).unwrap();
        let env = Env::sandbox(&canonical_dir(d.path()).unwrap().join("fake"));
        let evil = batch.destination(&env.system_root.join("System32").join("evil.dll")).unwrap();
        fs::create_dir_all(evil.parent().unwrap()).unwrap();
        fs::write(&evil, b"MZ").unwrap();
        batch.record(2);
        let info = batch.finish().unwrap().unwrap();
        let rep = s.restore(&info.id, &PathGuard::new(&env, &[]), &CancelToken::new()).unwrap();
        assert_eq!((rep.restored_files, rep.refused), (0, 1));
        assert!(!env.system_root.join("System32").join("evil.dll").exists());
    }

    #[test]
    fn expiry_purges_only_completed_old_batches() {
        let (d, s) = store();
        let work = canonical_dir(d.path()).unwrap().join("work");
        fs::create_dir_all(&work).unwrap();
        for n in ["a", "b"] {
            fs::write(work.join(n), n).unwrap();
        }
        let b1 = s.begin("one", 1, vec![]).unwrap();
        stash(&b1, &work.join("a"));
        let i1 = b1.finish().unwrap().unwrap();
        let b2 = s.begin("two", 30, vec![]).unwrap();
        stash(&b2, &work.join("b"));
        let i2 = b2.finish().unwrap().unwrap();

        assert!(s.purge_expired(now_unix()).is_empty());
        let removed = s.purge_expired(now_unix() + 2 * 86_400);
        assert_eq!(removed, vec![i1.id.clone()]);
        let left = s.list();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, i2.id);
        assert_eq!(s.purge(&i2.id).unwrap(), 1);
        assert!(s.list().is_empty());
    }

    #[test]
    fn interrupted_batches_are_still_listed_and_restorable() {
        let (d, s) = store();
        let work = canonical_dir(d.path()).unwrap().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("a.txt"), b"data").unwrap();
        let batch = s.begin("crashy", 7, vec![]).unwrap();
        stash(&batch, &work.join("a.txt"));
        let id = batch.id();
        drop(batch); // process "crashes": never finished, manifest says incomplete

        let listed = s.list();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].complete);
        assert!(
            s.purge_expired(now_unix() + 365 * 86_400).is_empty(),
            "incomplete batches are never auto-purged"
        );

        // Even with the manifest gone the data is recoverable from the layout alone.
        fs::remove_file(s.root().join(&id).join(MANIFEST)).unwrap();
        assert_eq!(s.list()[0].files, 1);
        let rep = s.restore(&id, &permissive_guard(&work), &CancelToken::new()).unwrap();
        assert_eq!(rep.restored_files, 1);
        assert_eq!(fs::read(work.join("a.txt")).unwrap(), b"data");
    }

    fn junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn restore_creates_nothing_behind_a_link_in_the_path() {
        let (d, s) = store();
        let base = canonical_dir(d.path()).unwrap();
        let work = base.join("work");
        fs::create_dir_all(work.join("x").join("y")).unwrap();
        fs::write(work.join("x").join("y").join("a.txt"), b"data").unwrap();
        let batch = s.begin("t", 7, vec![]).unwrap();
        stash(&batch, &work.join("x").join("y").join("a.txt"));
        let info = batch.finish().unwrap().unwrap();

        // The folder the file came from is replaced by a junction to somewhere else.
        let victim = base.join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::remove_dir_all(work.join("x")).unwrap();
        if !junction(&work.join("x"), &victim) {
            eprintln!("skipping: cannot create junctions here");
            return;
        }

        let rep = s.restore(&info.id, &permissive_guard(&work), &CancelToken::new()).unwrap();
        assert_eq!(rep.restored_files, 0, "{rep:?}");
        assert!(rep.refused + rep.failed >= 1, "{rep:?}");
        assert!(
            fs::read_dir(&victim).unwrap().next().is_none(),
            "nothing may be created behind the link"
        );
        assert_eq!(s.list()[0].files, 1, "the recovered copy stays in Recovery");
    }

    #[test]
    fn a_batch_folder_that_is_a_link_is_never_restored_from() {
        let (d, s) = store();
        let base = canonical_dir(d.path()).unwrap();
        let work = base.join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("a.txt"), b"data").unwrap();
        let batch = s.begin("t", 7, vec![]).unwrap();
        stash(&batch, &work.join("a.txt"));
        let info = batch.finish().unwrap().unwrap();

        // The batch folder is swapped for a junction to a copy of it elsewhere.
        let elsewhere = base.join("elsewhere");
        fs::rename(s.root().join(&info.id), &elsewhere).unwrap();
        if !junction(&s.root().join(&info.id), &elsewhere) {
            eprintln!("skipping: cannot create junctions here");
            return;
        }

        let r = s.restore(&info.id, &permissive_guard(&work), &CancelToken::new());
        assert!(matches!(r, Err(Error::UnknownBatch(_))), "{r:?}");
        assert!(!work.join("a.txt").exists());
        assert!(s.list().is_empty(), "a linked batch is not listed either");
    }

    #[test]
    fn an_unreadable_manifest_never_leads_to_an_automatic_purge() {
        let (d, s) = store();
        let work = canonical_dir(d.path()).unwrap().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("a.txt"), b"data").unwrap();
        let batch = s.begin("t", 30, vec![]).unwrap();
        stash(&batch, &work.join("a.txt"));
        let info = batch.finish().unwrap().unwrap();

        // A crash mid-write, or an antivirus holding the file, leaves the manifest unreadable.
        fs::write(s.root().join(&info.id).join(MANIFEST), b"").unwrap();

        let listed = s.list();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].complete, "a batch we cannot vouch for is not 'complete'");
        assert!(
            s.purge_expired(now_unix() + 8 * 86_400).is_empty(),
            "the 30-day promise must not shrink to 7 because a file could not be read"
        );
        let rep = s.restore(&info.id, &permissive_guard(&work), &CancelToken::new()).unwrap();
        assert_eq!(rep.restored_files, 1);
    }

    #[test]
    fn batch_ids_never_collide_even_within_one_second() {
        let (_d, s) = store();
        let mut ids = std::collections::HashSet::new();
        let mut open = Vec::new();
        for _ in 0..400 {
            let b = s.begin("t", 7, vec![]).unwrap();
            assert!(ids.insert(b.id()), "two batches shared one folder");
            open.push(b);
        }
    }

    #[test]
    fn a_failed_copy_leaves_no_partial_file_behind() {
        let (d, _s) = store();
        let src = d.path().join("src.bin");
        fs::write(&src, vec![7u8; 4096]).unwrap();
        let dest = d.path().join("dest.bin");
        // Opened without read access, so the copy fails after the destination was created.
        let node = open_node(&src, false).unwrap();
        assert!(node.copy_to(&dest).is_err());
        assert!(!dest.exists(), "a half-written copy must not be left to look like a backup");

        // And a destination that already exists is somebody else's: never touched.
        fs::write(&dest, b"theirs").unwrap();
        let node = open_node(&src, true).unwrap();
        assert!(node.copy_to(&dest).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"theirs");
    }
}
