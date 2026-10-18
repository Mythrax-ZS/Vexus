//! The directory walker: parallel, link-safe, cancellable.
//!
//! The walk produces a *plan*, never touches anything, and keeps the plan compact: a folder
//! whose entire content is eligible collapses into a single [`Item`], so a cache folder with
//! 80,000 files is one row, not 80,000.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, SystemTime};

use rayon::prelude::*;

use crate::fsops::{OpenFailure, is_reparse, open_node, touched_of};
use crate::model::{CategoryStats, Item};
use crate::progress::{CancelToken, Live};
use crate::rules::NameFilter;
use crate::safety::{PathGuard, is_ambiguous_name, is_protected_file_name};

pub(crate) struct WalkCtx<'a> {
    pub guard: &'a PathGuard,
    pub names: NameFilter,
    pub min_age: Duration,
    pub now: SystemTime,
    pub recurse: bool,
    pub prune_dirs: bool,
    pub probe_locks: bool,
    pub cancel: &'a CancelToken,
    pub live: &'a Live,
    /// Index of the resolved target these items belong to.
    pub target: usize,
}

/// What a subtree contributes to its parent.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    /// Every entry in the subtree qualifies, so the parent may collapse it into one item.
    pub full: bool,
    /// Eligible bytes / files beneath (whether or not collapsed).
    pub size: u64,
    pub files: u64,
    pub newest: Option<SystemTime>,
    /// Items to emit for this subtree. Empty when the subtree is `full` and prunable, because
    /// the parent will emit one item for the whole thing.
    pub items: Vec<Item>,
    pub stats: CategoryStats,
}

pub(crate) fn old_enough(modified: Option<SystemTime>, now: SystemTime, min_age: Duration) -> bool {
    if min_age.is_zero() {
        return true;
    }
    // Unknown times and times in the future (clock skew) both count as "too recent".
    modified.is_some_and(|m| now.duration_since(m).is_ok_and(|age| age >= min_age))
}

fn newer(a: Option<SystemTime>, b: Option<SystemTime>) -> Option<SystemTime> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// How many folder levels below a rule root the engine will walk.
///
/// Real folder trees (even `node_modules`) stay far below this, but NTFS allows ~16,000 levels,
/// and a recursive walker that follows them overflows its stack — a hostile or merely broken
/// tree in `%TEMP%` must never be able to crash a cleaner. Anything deeper is left alone and
/// reported, and its parent is never collapsed into a single removable item.
pub(crate) const MAX_DEPTH: usize = 128;

pub(crate) fn walk_dir(
    ctx: &WalkCtx<'_>,
    dir: &Path,
    own_modified: Option<SystemTime>,
    is_root: bool,
    depth: usize,
) -> Outcome {
    let mut out = Outcome { full: true, ..Outcome::default() };
    if ctx.cancel.is_cancelled() {
        out.full = false;
        return out;
    }
    if depth > MAX_DEPTH {
        tracing::warn!(depth, dir = %dir.display(), "folder tree is nested too deeply; leaving it alone");
        out.full = false;
        out.stats.unreadable += 1;
        return out;
    }
    ctx.live.note_dir(dir);

    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            out.full = false;
            if e.kind() == io::ErrorKind::PermissionDenied {
                out.stats.denied += 1;
            } else if e.kind() != io::ErrorKind::NotFound {
                out.stats.unreadable += 1;
            }
            return out;
        }
    };

    let mut had_entries = false;
    let mut file_items: Vec<Item> = Vec::new();
    let mut subdirs: Vec<(PathBuf, Option<SystemTime>)> = Vec::new();

    for entry in read {
        let Ok(entry) = entry else {
            out.full = false;
            out.stats.unreadable += 1;
            continue;
        };
        had_entries = true;
        ctx.live.entries.fetch_add(1, Relaxed);
        let Ok(md) = entry.metadata() else {
            out.full = false;
            out.stats.unreadable += 1;
            continue;
        };
        let path = entry.path();

        if is_reparse(&md) {
            // Never follow, never remove: a link may lead anywhere.
            out.stats.links += 1;
            out.full = false;
            continue;
        }

        if md.is_dir() {
            if !ctx.recurse
                || is_ambiguous_name(&entry.file_name())
                || ctx.guard.is_protected(&path).is_some()
            {
                out.full = false;
                continue;
            }
            subdirs.push((path, md.modified().ok()));
            continue;
        }

        let size = md.len();
        ctx.live.bytes_seen.fetch_add(size, Relaxed);
        let name = entry.file_name();
        if is_protected_file_name(&name) || !ctx.names.matches(&name.to_string_lossy()) {
            out.full = false;
            continue;
        }
        // The newer of modified and created: a copied or extracted file keeps its old
        // modification time but is new to this PC.
        let modified = touched_of(&md);
        if !old_enough(modified, ctx.now, ctx.min_age) {
            out.stats.kept_files += 1;
            out.stats.kept_bytes += size;
            out.full = false;
            continue;
        }
        if ctx.probe_locks {
            match open_node(&path, false) {
                Ok(_) => {}
                Err(OpenFailure::InUse) => {
                    out.stats.busy_files += 1;
                    out.stats.busy_bytes += size;
                    out.full = false;
                    continue;
                }
                Err(OpenFailure::Gone) => continue,
                Err(OpenFailure::Denied) => {
                    out.stats.denied += 1;
                    out.full = false;
                    continue;
                }
                Err(OpenFailure::Link) => {
                    out.stats.links += 1;
                    out.full = false;
                    continue;
                }
                Err(OpenFailure::Other(_)) => {
                    out.stats.unreadable += 1;
                    out.full = false;
                    continue;
                }
            }
        }
        out.size += size;
        out.files += 1;
        out.newest = newer(out.newest, modified);
        ctx.live.found_files.fetch_add(1, Relaxed);
        ctx.live.found_bytes.fetch_add(size, Relaxed);
        file_items.push(Item::file(path, size, modified, ctx.target));
    }

    // Recurse into subdirectories in parallel; rayon balances nested work across the pool.
    let children: Vec<(PathBuf, Option<SystemTime>, Outcome)> = subdirs
        .into_par_iter()
        .map(|(p, m)| {
            let o = walk_dir(ctx, &p, m, false, depth + 1);
            (p, m, o)
        })
        .collect();

    let mut dir_items: Vec<Item> = Vec::new();
    let mut partial: Vec<Item> = Vec::new();
    for (path, modified, mut sub) in children {
        out.size += sub.size;
        out.files += sub.files;
        out.newest = newer(out.newest, sub.newest);
        out.stats.merge(&sub.stats);
        if sub.full && ctx.prune_dirs {
            dir_items.push(Item::dir(
                path,
                sub.size,
                sub.files,
                sub.newest.or(modified),
                ctx.target,
            ));
        } else {
            out.full = false;
            partial.append(&mut sub.items);
        }
    }

    if !had_entries && !is_root {
        // An empty folder is only removable if it has been empty and untouched for a while —
        // it may be a freshly created work folder an app is about to fill.
        out.full = ctx.prune_dirs && old_enough(own_modified, ctx.now, ctx.min_age);
    }

    if out.full && ctx.prune_dirs && !is_root {
        // The parent collapses this whole subtree into a single item.
        return out;
    }
    out.items.extend(file_items);
    out.items.extend(dir_items);
    out.items.extend(partial);
    out
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::windows::fs::{FileTimesExt, OpenOptionsExt};
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;
    use crate::env::Env;

    const DAY: u64 = 86_400;

    struct Fixture {
        _dir: TempDir,
        root: PathBuf,
        guard: PathGuard,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fsops::canonical_dir(dir.path()).unwrap();
        let env = Env::sandbox(Path::new(r"C:\Nowhere"));
        let mut guard = PathGuard::new(&env, &[]);
        guard.permit_root(&root).unwrap();
        Fixture { _dir: dir, root, guard }
    }

    fn write(path: &Path, bytes: usize, age_days: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_days * DAY);
        let times = fs::FileTimes::new().set_modified(t).set_created(t);
        File::options().write(true).open(path).unwrap().set_times(times).unwrap();
    }

    fn age_dir(path: &Path, age_days: u64) {
        let t = SystemTime::now() - Duration::from_secs(age_days * DAY);
        let dir = fs::OpenOptions::new()
            .write(true)
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS: required to open a directory
            .open(path)
            .unwrap();
        dir.set_modified(t).unwrap();
    }

    fn walk(fx: &Fixture, min_age_days: u64, prune: bool) -> Outcome {
        let cancel = CancelToken::new();
        let live = Live::default();
        let ctx = WalkCtx {
            guard: &fx.guard,
            names: NameFilter::Any,
            min_age: Duration::from_secs(min_age_days * DAY),
            now: SystemTime::now(),
            recurse: true,
            prune_dirs: prune,
            probe_locks: false,
            cancel: &cancel,
            live: &live,
            target: 0,
        };
        walk_dir(&ctx, &fx.root, None, true, 0)
    }

    fn names(out: &Outcome) -> Vec<String> {
        let mut v: Vec<String> = out
            .items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn old_files_are_listed_and_recent_ones_are_kept() {
        let fx = fixture();
        write(&fx.root.join("old.tmp"), 100, 10);
        write(&fx.root.join("new.tmp"), 50, 0);
        let out = walk(&fx, 3, true);
        assert_eq!(names(&out), ["old.tmp"]);
        assert_eq!((out.files, out.size), (1, 100));
        assert_eq!((out.stats.kept_files, out.stats.kept_bytes), (1, 50));
    }

    #[test]
    fn fully_eligible_folders_collapse_into_one_item() {
        let fx = fixture();
        for i in 0..5 {
            write(&fx.root.join("cache").join(format!("f{i}.bin")), 10, 30);
        }
        write(&fx.root.join("cache").join("deep").join("g.bin"), 20, 30);
        let out = walk(&fx, 3, true);
        assert_eq!(out.items.len(), 1);
        let item = &out.items[0];
        assert_eq!(item.kind, crate::model::ItemKind::Dir);
        assert_eq!((item.files, item.size), (6, 70));
    }

    #[test]
    fn a_recent_file_prevents_collapsing_and_survivors_are_itemised() {
        let fx = fixture();
        write(&fx.root.join("mixed").join("old1.bin"), 10, 30);
        write(&fx.root.join("mixed").join("old2.bin"), 10, 30);
        write(&fx.root.join("mixed").join("fresh.bin"), 10, 0);
        write(&fx.root.join("mixed").join("sub").join("old3.bin"), 10, 30);
        let out = walk(&fx, 3, true);
        // `sub` is fully old so it collapses; the two old files stay individual; `mixed` itself
        // cannot be removed because `fresh.bin` stays.
        assert_eq!(names(&out), ["old1.bin", "old2.bin", "sub"]);
        assert_eq!(out.files, 3);
        assert_eq!(out.stats.kept_files, 1);
    }

    #[test]
    fn without_pruning_only_files_are_items() {
        let fx = fixture();
        write(&fx.root.join("keep").join("a.lnk"), 10, 30);
        write(&fx.root.join("keep").join("b.lnk"), 10, 30);
        let out = walk(&fx, 0, false);
        assert_eq!(names(&out), ["a.lnk", "b.lnk"]);
        assert!(out.items.iter().all(|i| i.kind == crate::model::ItemKind::File));
    }

    #[test]
    fn empty_folders_obey_the_age_gate() {
        let fx = fixture();
        fs::create_dir_all(fx.root.join("fresh-empty")).unwrap();
        fs::create_dir_all(fx.root.join("stale-empty")).unwrap();
        age_dir(&fx.root.join("stale-empty"), 30);
        let out = walk(&fx, 3, true);
        assert_eq!(names(&out), ["stale-empty"]);
    }

    #[test]
    fn name_filters_and_protected_names_are_respected() {
        let fx = fixture();
        write(&fx.root.join("a.pf"), 1, 9);
        write(&fx.root.join("b.txt"), 1, 9);
        write(&fx.root.join("pagefile.sys"), 1, 9);
        write(&fx.root.join("NTUSER.DAT"), 1, 9);
        let cancel = CancelToken::new();
        let live = Live::default();
        let ctx = WalkCtx {
            guard: &fx.guard,
            names: NameFilter::Patterns(&["*.pf", "*.sys", "ntuser.dat*"]),
            min_age: Duration::ZERO,
            now: SystemTime::now(),
            recurse: false,
            prune_dirs: false,
            probe_locks: false,
            cancel: &cancel,
            live: &live,
            target: 0,
        };
        let out = walk_dir(&ctx, &fx.root, None, true, 0);
        assert_eq!(names(&out), ["a.pf"], "protected names must never qualify, even if matched");
    }

    #[test]
    fn links_are_skipped_not_followed() {
        let fx = fixture();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("precious.txt"), 10, 99);
        let link = fx.root.join("link");
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("skipping: cannot create junctions here");
            return;
        }
        write(&fx.root.join("real.tmp"), 5, 9);
        let out = walk(&fx, 0, true);
        assert_eq!(names(&out), ["real.tmp"], "the junction and its target must not be listed");
        assert_eq!(out.stats.links, 1);
        assert!(outside.path().join("precious.txt").exists());
    }

    #[test]
    fn locked_files_are_reported_as_busy_when_probing() {
        let fx = fixture();
        write(&fx.root.join("free.db"), 10, 9);
        write(&fx.root.join("locked.db"), 20, 9);
        let _held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(fx.root.join("locked.db"))
            .unwrap();
        let cancel = CancelToken::new();
        let live = Live::default();
        let ctx = WalkCtx {
            guard: &fx.guard,
            names: NameFilter::Any,
            min_age: Duration::ZERO,
            now: SystemTime::now(),
            recurse: false,
            prune_dirs: false,
            probe_locks: true,
            cancel: &cancel,
            live: &live,
            target: 0,
        };
        let out = walk_dir(&ctx, &fx.root, None, true, 0);
        assert_eq!(names(&out), ["free.db"]);
        assert_eq!((out.stats.busy_files, out.stats.busy_bytes), (1, 20));
    }

    #[test]
    fn cancellation_stops_the_walk() {
        let fx = fixture();
        write(&fx.root.join("a").join("x.bin"), 1, 9);
        let cancel = CancelToken::new();
        cancel.cancel();
        let live = Live::default();
        let ctx = WalkCtx {
            guard: &fx.guard,
            names: NameFilter::Any,
            min_age: Duration::ZERO,
            now: SystemTime::now(),
            recurse: true,
            prune_dirs: true,
            probe_locks: false,
            cancel: &cancel,
            live: &live,
            target: 0,
        };
        let out = walk_dir(&ctx, &fx.root, None, true, 0);
        assert!(out.items.is_empty());
        assert_eq!(live.entries.load(Relaxed), 0);
    }

    #[test]
    fn future_timestamps_count_as_recent() {
        assert!(!old_enough(
            Some(SystemTime::now() + Duration::from_secs(3600)),
            SystemTime::now(),
            Duration::from_secs(60)
        ));
        assert!(!old_enough(None, SystemTime::now(), Duration::from_secs(60)));
        assert!(old_enough(None, SystemTime::now(), Duration::ZERO));
    }
}
