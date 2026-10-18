//! Cleaning: executing a scan's plan — exactly that plan, re-verified item by item.
//!
//! The scan is a *proposal*; between the scan and the clean the world may have changed. So every
//! file is re-opened without following links, checked through the handle (it still lives under
//! its rule root, still matches the rule's filter, is still old enough, and — for files listed
//! individually — still has the size and timestamp the user saw), and only then removed or moved
//! to Recovery through that same handle. Anything that no longer matches is skipped and reported.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::time::{Duration, Instant, SystemTime};

use rayon::prelude::*;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::fsops::{
    Node, OpenFailure, canonical_dir, inspect_dir, is_cross_volume, is_dir_not_empty, is_reparse,
    open_for_removal, open_node,
};
use crate::model::{DeletionPolicy, Item, ItemKind, Strategy, item_id};
use crate::progress::{CancelToken, CleanEvent, CleanProgress, Sink, eta_ms};
use crate::recovery::{Batch, BatchInfo, RecoveryStore};
use crate::recyclebin;
use crate::safety::{is_ambiguous_name, is_protected_file_name, is_strictly_within, normalize};
use crate::scan::walker::MAX_DEPTH;
use crate::scan::walker::old_enough;
use crate::scan::{
    ResolvedKind, ResolvedTarget, Scan, Selection, WORKER_STACK, default_threads,
    lower_thread_priority,
};

/// Individual failure messages kept per category (the rest are only counted).
const MAX_FAILURES: usize = 25;

#[derive(Debug, Clone)]
pub struct CleanOptions {
    pub policy: DeletionPolicy,
    /// How long Recovery keeps undoable removals.
    pub undo_days: u32,
    pub threads: Option<usize>,
}

impl Default for CleanOptions {
    fn default() -> Self {
        CleanOptions { policy: DeletionPolicy::Smart, undo_days: 7, threads: None }
    }
}

/// Why something that was in the plan was left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// Another program was using it.
    InUse,
    /// Windows denied access (usually needs administrator rights).
    AccessDenied,
    /// It changed after the scan, so what you reviewed is no longer what is there.
    Changed,
    /// Too recent for the age limit (it was modified after the scan).
    TooRecent,
    /// No longer matches the rule's file pattern.
    NotMatching,
    /// A link; Vexus never follows or removes links.
    Link,
    /// Resolved to somewhere outside the location it was scanned in.
    Outside,
    /// Covered by the safety guard.
    Protected,
    /// Nested too deeply to examine safely (see `scan::walker::MAX_DEPTH`).
    TooDeep,
    /// Failed for another reason (details are listed).
    Failed,
}

impl SkipReason {
    /// Plain-language reason, for logs and the CLI.
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::InUse => "in use by another program",
            SkipReason::AccessDenied => "access denied (may need administrator rights)",
            SkipReason::Changed => "changed since the scan",
            SkipReason::TooRecent => "newer than the age limit",
            SkipReason::NotMatching => "no longer matches the rule",
            SkipReason::Link => "a link (never followed)",
            SkipReason::Outside => "resolved outside the scanned location",
            SkipReason::Protected => "protected",
            SkipReason::TooDeep => "nested too deeply to examine safely",
            SkipReason::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipStat {
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryReport {
    pub id: &'static str,
    pub strategy: Strategy,
    pub removed_files: u64,
    pub removed_bytes: u64,
    pub removed_dirs: u64,
    pub skipped: BTreeMap<SkipReason, SkipStat>,
    pub failures: Vec<Failure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub categories: Vec<CategoryReport>,
    pub files: u64,
    pub bytes: u64,
    pub dirs: u64,
    /// Bytes now held in Recovery (restorable, not yet freed).
    pub undo_bytes: u64,
    /// Bytes deleted outright (freed now).
    pub permanent_bytes: u64,
    /// The Recovery batch holding the undoable part, if any.
    pub batch: Option<BatchInfo>,
    pub skipped: BTreeMap<SkipReason, SkipStat>,
    pub elapsed_ms: u64,
    pub cancelled: bool,
}

/// Per-thread accumulator, merged with `absorb`.
#[derive(Default)]
struct Acc {
    files: u64,
    bytes: u64,
    dirs: u64,
    skipped: BTreeMap<SkipReason, SkipStat>,
    failures: Vec<Failure>,
}

impl Acc {
    fn skip(&mut self, why: SkipReason, files: u64, bytes: u64) {
        let e = self.skipped.entry(why).or_default();
        e.files += files;
        e.bytes += bytes;
    }

    fn fail(&mut self, path: &Path, reason: String, bytes: u64) {
        tracing::debug!(path = %path.display(), "{reason}");
        self.skip(SkipReason::Failed, 1, bytes);
        if self.failures.len() < MAX_FAILURES {
            self.failures.push(Failure { path: path.display().to_string(), reason });
        }
    }

    fn absorb(mut self, o: Acc) -> Acc {
        self.files += o.files;
        self.bytes += o.bytes;
        self.dirs += o.dirs;
        for (k, v) in o.skipped {
            let e = self.skipped.entry(k).or_default();
            e.files += v.files;
            e.bytes += v.bytes;
        }
        for f in o.failures {
            if self.failures.len() < MAX_FAILURES {
                self.failures.push(f);
            }
        }
        self
    }
}

#[derive(Default)]
struct CleanLive {
    files: AtomicU64,
    bytes: AtomicU64,
}

impl CleanLive {
    fn advance(&self, files: u64, bytes: u64) {
        self.files.fetch_add(files, Relaxed);
        self.bytes.fetch_add(bytes, Relaxed);
    }
}

struct Exec<'a> {
    scan: &'a Scan,
    batch: Option<&'a Batch>,
    cancel: &'a CancelToken,
    live: &'a CleanLive,
    now: SystemTime,
    /// Per target: its root still resolves to the canonical path the scan used.
    root_ok: Vec<bool>,
}

struct Work<'a> {
    category: usize,
    strategy: Strategy,
    items: Vec<&'a Item>,
}

pub(crate) fn run(
    scan: &Scan,
    sel: &Selection,
    opts: &CleanOptions,
    recovery: &RecoveryStore,
    cancel: &CancelToken,
    sink: &dyn Sink<CleanEvent>,
) -> Result<CleanReport> {
    let clock = Instant::now();

    let mut work: Vec<Work<'_>> = Vec::new();
    for (c, (rule, cat)) in scan.rules.iter().zip(&scan.categories).enumerate() {
        if !sel.categories.contains(rule.id) {
            continue;
        }
        let items: Vec<&Item> = cat
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| !sel.excluded.contains(&item_id(c, *i)))
            .map(|(_, it)| it)
            .collect();
        if !items.is_empty() {
            work.push(Work { category: c, strategy: rule.effective_strategy(opts.policy), items });
        }
    }

    let files_total: u64 = work.iter().flat_map(|w| &w.items).map(|i| i.files).sum();
    let bytes_total: u64 = work.iter().flat_map(|w| &w.items).map(|i| i.size).sum();
    let ids: Vec<&'static str> = work.iter().map(|w| scan.rules[w.category].id).collect();
    sink.emit(CleanEvent::Started { files_total, bytes_total, categories: ids.clone() });

    // Undo must be possible *before* anything is touched: if the Recovery folder can't be
    // prepared we stop here rather than quietly deleting what was promised to be recoverable.
    let undoable: Vec<(&'static str, &'static str)> = work
        .iter()
        .filter(|w| w.strategy == Strategy::Undoable)
        .map(|w| (scan.rules[w.category].id, scan.rules[w.category].name))
        .collect();
    let batch = if undoable.is_empty() {
        None
    } else {
        // The batch describes only what actually went to Recovery, not everything cleaned.
        let label = match undoable.as_slice() {
            [(_, only)] => (*only).to_string(),
            [(_, a), (_, b)] => format!("{a} and {b}"),
            many => format!("{} categories", many.len()),
        };
        let cats = undoable.iter().map(|(id, _)| (*id).to_string()).collect();
        Some(recovery.begin(&label, opts.undo_days, cats).map_err(|e| {
            Error::io("Couldn't prepare the Recovery folder, so nothing was removed", e)
        })?)
    };

    let root_ok: Vec<bool> = scan
        .targets
        .iter()
        .map(|t| match &t.kind {
            ResolvedKind::Tree { root, .. } => {
                canonical_dir(root).is_ok_and(|c| normalize(&c) == normalize(root))
            }
            _ => true,
        })
        .collect();

    let live = CleanLive::default();
    let finished = AtomicBool::new(false);
    let cats_done = AtomicU32::new(0);
    let ex =
        Exec { scan, batch: batch.as_ref(), cancel, live: &live, now: SystemTime::now(), root_ok };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads.unwrap_or_else(default_threads).clamp(1, 16))
        .thread_name(|i| format!("vexus-clean-{i}"))
        .stack_size(WORKER_STACK)
        .start_handler(|_| lower_thread_priority())
        .build()
        .map_err(|e| Error::Invalid(format!("could not start worker threads: {e}")))?;

    let total_cats = work.len() as u32;
    let mut reports: Vec<CategoryReport> = Vec::new();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            loop {
                for _ in 0..6 {
                    if finished.load(Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(12));
                }
                let (files_done, bytes_done) = (live.files.load(Relaxed), live.bytes.load(Relaxed));
                let fraction = if bytes_total > 0 {
                    bytes_done as f32 / bytes_total as f32
                } else if files_total > 0 {
                    files_done as f32 / files_total as f32
                } else {
                    0.0
                }
                .min(0.995);
                let elapsed = clock.elapsed().as_millis() as u64;
                sink.emit(CleanEvent::Progress(CleanProgress {
                    files_done,
                    bytes_done,
                    files_total,
                    bytes_total,
                    category: None,
                    categories_done: cats_done.load(Relaxed),
                    categories_total: total_cats,
                    elapsed_ms: elapsed,
                    fraction,
                    eta_ms: eta_ms(elapsed, fraction),
                }));
            }
        });

        for w in &work {
            if cancel.is_cancelled() {
                break;
            }
            let acc = pool.install(|| {
                w.items
                    .par_iter()
                    .fold(Acc::default, |mut a, item| {
                        clean_item(&ex, w.strategy, item, &mut a);
                        a
                    })
                    .reduce(Acc::default, Acc::absorb)
            });
            let id = scan.rules[w.category].id;
            sink.emit(CleanEvent::CategoryDone {
                id,
                removed_files: acc.files,
                removed_bytes: acc.bytes,
            });
            cats_done.fetch_add(1, Relaxed);
            reports.push(CategoryReport {
                id,
                strategy: w.strategy,
                removed_files: acc.files,
                removed_bytes: acc.bytes,
                removed_dirs: acc.dirs,
                skipped: acc.skipped,
                failures: acc.failures,
            });
        }
        finished.store(true, Relaxed);
    });

    let batch_info = match batch {
        Some(b) => b.finish().map_err(|e| Error::io("Couldn't finalise the Recovery copy", e))?,
        None => None,
    };

    let mut skipped: BTreeMap<SkipReason, SkipStat> = BTreeMap::new();
    for r in &reports {
        for (k, v) in &r.skipped {
            let e = skipped.entry(*k).or_default();
            e.files += v.files;
            e.bytes += v.bytes;
        }
    }
    let (mut undo_bytes, mut permanent_bytes) = (0u64, 0u64);
    for r in &reports {
        match r.strategy {
            Strategy::Undoable => undo_bytes += r.removed_bytes,
            Strategy::Permanent => permanent_bytes += r.removed_bytes,
        }
    }
    let report = CleanReport {
        files: reports.iter().map(|r| r.removed_files).sum(),
        bytes: reports.iter().map(|r| r.removed_bytes).sum(),
        dirs: reports.iter().map(|r| r.removed_dirs).sum(),
        undo_bytes,
        permanent_bytes,
        batch: batch_info,
        skipped,
        categories: reports,
        elapsed_ms: clock.elapsed().as_millis() as u64,
        cancelled: cancel.is_cancelled(),
    };
    tracing::info!(
        files = report.files,
        bytes = report.bytes,
        undo_bytes = report.undo_bytes,
        elapsed_ms = report.elapsed_ms,
        cancelled = report.cancelled,
        "clean finished"
    );
    sink.emit(CleanEvent::Finished { report: Box::new(report.clone()) });
    Ok(report)
}

fn clean_item(ex: &Exec<'_>, strategy: Strategy, item: &Item, acc: &mut Acc) {
    if ex.cancel.is_cancelled() {
        return;
    }
    let target = &ex.scan.targets[item.target];
    if !ex.root_ok[item.target] {
        acc.skip(SkipReason::Outside, item.files, item.size);
        ex.live.advance(item.files, item.size);
        return;
    }
    match item.kind {
        ItemKind::RecycleBin => match recyclebin::empty() {
            Ok(()) => {
                acc.files += item.files;
                acc.bytes += item.size;
                ex.live.advance(item.files, item.size);
            }
            Err(e) => {
                acc.fail(&item.path, format!("Couldn't empty the Recycle Bin: {e}"), item.size);
                ex.live.advance(item.files, item.size);
            }
        },
        ItemKind::File => {
            clean_file(ex, strategy, target, &item.path, Some((item.size, item.modified)), acc)
        }
        ItemKind::Dir => {
            clean_dir(ex, strategy, target, &item.path, acc, 0);
        }
    }
}

/// Is `path` (an already-resolved final path) allowed to be touched for this target?
fn check_node(
    ex: &Exec<'_>,
    t: &ResolvedTarget,
    node: &Node,
) -> std::result::Result<(), SkipReason> {
    check_resolved(ex, t, &node.path, node.is_dir)
}

/// The scope checks themselves, on a path the OS resolved from an open handle.
fn check_resolved(
    ex: &Exec<'_>,
    t: &ResolvedTarget,
    resolved: &Path,
    is_dir: bool,
) -> std::result::Result<(), SkipReason> {
    let file_name = resolved.file_name().unwrap_or_default();
    match &t.kind {
        ResolvedKind::Tree { root, names, .. } => {
            if !is_strictly_within(resolved, root) {
                return Err(SkipReason::Outside);
            }
            if !is_dir {
                if is_protected_file_name(file_name) {
                    return Err(SkipReason::Protected);
                }
                if !names.matches(&file_name.to_string_lossy()) {
                    return Err(SkipReason::NotMatching);
                }
            }
        }
        ResolvedKind::File { path } => {
            if normalize(resolved) != normalize(path) {
                return Err(SkipReason::Outside);
            }
        }
        _ => return Err(SkipReason::Outside),
    }
    if ex.scan.guard.is_protected(resolved).is_some() {
        return Err(SkipReason::Protected);
    }
    Ok(())
}

fn clean_file(
    ex: &Exec<'_>,
    strategy: Strategy,
    t: &ResolvedTarget,
    path: &Path,
    snapshot: Option<(u64, Option<SystemTime>)>,
    acc: &mut Acc,
) {
    if ex.cancel.is_cancelled() {
        return;
    }
    let hint = snapshot.map_or(0, |s| s.0);
    let node = match open_for_removal(path, strategy == Strategy::Undoable) {
        Ok(n) => n,
        Err(OpenFailure::Gone) => {
            ex.live.advance(1, hint);
            return;
        }
        Err(OpenFailure::InUse) => {
            acc.skip(SkipReason::InUse, 1, hint);
            ex.live.advance(1, hint);
            return;
        }
        Err(OpenFailure::Denied) => {
            acc.skip(SkipReason::AccessDenied, 1, hint);
            ex.live.advance(1, hint);
            return;
        }
        Err(OpenFailure::Link) => {
            acc.skip(SkipReason::Link, 1, hint);
            ex.live.advance(1, hint);
            return;
        }
        Err(OpenFailure::Other(e)) => {
            acc.fail(path, e.to_string(), hint);
            ex.live.advance(1, hint);
            return;
        }
    };
    let size = node.size;
    let verdict = if node.is_dir {
        Err(SkipReason::Changed) // a folder now stands where a file was planned
    } else {
        check_node(ex, t, &node).and_then(|()| {
            let touched = node.touched();
            if !old_enough(touched, ex.now, t.min_age) {
                Err(SkipReason::TooRecent)
            } else if let Some((size, listed_touched)) = snapshot {
                // Listed on its own: it must still be the file the preview showed.
                if size != node.size || listed_touched != touched {
                    Err(SkipReason::Changed)
                } else {
                    Ok(())
                }
            } else if touched.is_none_or(|x| x > ex.scan.started_at) {
                // Found by re-walking a listed folder: if it arrived or changed after the scan
                // began, the preview never showed it, so it stays.
                Err(SkipReason::Changed)
            } else {
                Ok(())
            }
        })
    };
    match verdict {
        Ok(()) => apply(ex, strategy, node, acc),
        Err(reason) => acc.skip(reason, 1, size),
    }
    ex.live.advance(1, size);
}

/// Performs the removal through the verified handle.
fn apply(ex: &Exec<'_>, strategy: Strategy, node: Node, acc: &mut Acc) {
    let size = node.size;
    let path = node.path.clone();
    let batch = match (strategy, ex.batch) {
        (Strategy::Undoable, Some(b)) => Some(b),
        (Strategy::Undoable, None) => {
            // Cannot happen (the batch is created up front), but never fall back to deleting.
            acc.fail(&path, "no Recovery batch available".into(), size);
            return;
        }
        _ => None,
    };

    let outcome: std::io::Result<()> = match batch {
        None => node.delete(),
        Some(batch) => {
            let Some(dest) = batch.destination(&path) else {
                acc.skip(SkipReason::Protected, 1, size);
                return;
            };
            if let Some(parent) = dest.parent()
                && let Err(e) = fs::create_dir_all(parent)
            {
                acc.fail(&path, format!("Couldn't prepare Recovery: {e}"), size);
                return;
            }
            let moved = match node.rename_to(&dest) {
                Err(e) if is_cross_volume(&e) => match node.copy_to(&dest) {
                    Ok(_) => node.delete().inspect_err(|_| {
                        let _ = fs::remove_file(&dest); // keep exactly one copy
                    }),
                    // `copy_to` removed its own partial output, and never touches a `dest`
                    // that already existed: it is not ours to delete.
                    Err(e) => Err(e),
                },
                other => other,
            };
            if moved.is_ok() {
                batch.record(size);
            }
            moved
        }
    };

    match outcome {
        Ok(()) => {
            acc.files += 1;
            acc.bytes += size;
        }
        Err(e) => match e.raw_os_error() {
            Some(5) => acc.skip(SkipReason::AccessDenied, 1, size),
            Some(32 | 33) => acc.skip(SkipReason::InUse, 1, size),
            _ => acc.fail(&path, e.to_string(), size),
        },
    }
}

/// Clears a folder item: re-walks it, re-verifying every file, then removes the folders that
/// ended up empty. Files and subfolders are processed in parallel.
fn clean_dir(
    ex: &Exec<'_>,
    strategy: Strategy,
    t: &ResolvedTarget,
    dir: &Path,
    acc: &mut Acc,
    depth: usize,
) {
    if ex.cancel.is_cancelled() {
        return;
    }
    if depth > MAX_DEPTH {
        // A tree this deep appeared after the scan (the scan never plans one). Do not recurse
        // into it: that is how a hostile folder would crash the cleaner.
        acc.skip(SkipReason::TooDeep, 0, 0);
        return;
    }
    // Verify the folder itself before listing it. Files are re-verified one by one anyway, but a
    // junction swapped in after the scan must not even get *walked*: that would enumerate (and
    // report on) a tree the scan never examined.
    match inspect_dir(dir) {
        Ok(resolved) => {
            if let Err(reason) = check_resolved(ex, t, &resolved, true) {
                acc.skip(reason, 1, 0);
                return;
            }
        }
        Err(OpenFailure::Gone) => return,
        Err(OpenFailure::Link) => {
            acc.skip(SkipReason::Link, 1, 0);
            return;
        }
        // Listing it will report these properly if they are real problems.
        Err(OpenFailure::InUse | OpenFailure::Denied) => {}
        Err(OpenFailure::Other(e)) => {
            acc.fail(dir, e.to_string(), 0);
            return;
        }
    }
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            acc.skip(SkipReason::AccessDenied, 0, 0);
            return;
        }
        Err(e) => {
            acc.fail(dir, e.to_string(), 0);
            return;
        }
    };
    let mut had_entries = false;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in rd.flatten() {
        had_entries = true;
        let Ok(md) = entry.metadata() else { continue };
        if is_reparse(&md) {
            acc.skip(SkipReason::Link, 1, 0);
            continue;
        }
        if md.is_dir() {
            if is_ambiguous_name(&entry.file_name()) {
                acc.skip(SkipReason::Protected, 1, 0);
            } else {
                dirs.push(entry.path());
            }
        } else {
            files.push(entry.path());
        }
    }

    let from_files = files
        .par_iter()
        .fold(Acc::default, |mut a, p| {
            clean_file(ex, strategy, t, p, None, &mut a);
            a
        })
        .reduce(Acc::default, Acc::absorb);
    let from_dirs = dirs
        .par_iter()
        .fold(Acc::default, |mut a, d| {
            clean_dir(ex, strategy, t, d, &mut a, depth + 1);
            a
        })
        .reduce(Acc::default, Acc::absorb);
    let merged = std::mem::take(acc).absorb(from_files).absorb(from_dirs);
    *acc = merged;

    remove_if_empty(ex, t, dir, !had_entries, acc);
}

/// Removes `dir` if nothing is left in it. An folder that was *already* empty must also pass
/// the age gate (it may be a fresh work folder); one we just emptied need not.
fn remove_if_empty(ex: &Exec<'_>, t: &ResolvedTarget, dir: &Path, was_empty: bool, acc: &mut Acc) {
    let Ok(node) = open_node(dir, false) else { return };
    if !node.is_dir {
        return;
    }
    match check_node(ex, t, &node) {
        Ok(()) => {}
        Err(_) => return,
    }
    if was_empty && !old_enough(node.modified, ex.now, t.min_age) {
        return;
    }
    match node.delete() {
        Ok(()) => acc.dirs += 1,
        Err(e) if is_dir_not_empty(&e) => {} // some files were skipped; the folder stays
        Err(e) => tracing::debug!(dir = %dir.display(), "couldn't remove folder: {e}"),
    }
}
