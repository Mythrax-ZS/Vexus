//! Scanning: read-only discovery of what *could* be cleaned.
//!
//! A scan never modifies anything. It resolves each rule against the real environment (with the
//! safety guard vetting every root), walks the roots in parallel, and produces a [`Scan`]: the
//! plan that the preview shows and that a later clean executes — nothing more, nothing less.

mod resolve;
pub(crate) mod walker;

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::time::{Duration, Instant, SystemTime};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
};

use crate::error::{Error, Result};
use crate::fsops::is_reparse;
use crate::model::{
    CategoryStats, CategoryStatus, DeletionPolicy, Item, ItemId, ItemKind, ItemView, Risk, ScanId,
    Strategy, item_id,
};
use crate::progress::{
    CancelToken, CategorySummary, Live, ScanEvent, ScanProgress, ScanSummary, Sink, eta_ms,
};
use crate::recyclebin;
use crate::rules::{NameFilter, Rule, RuleMeta};
use crate::safety::PathGuard;
use crate::util::random_u64;
use walker::{WalkCtx, old_enough, walk_dir};

#[derive(Debug, Clone, Default)]
pub struct ScanRequest {
    /// Rule ids to scan; `None` scans every rule.
    pub categories: Option<Vec<String>>,
    /// Overrides the age gate of rules that allow it.
    pub age_override_hours: Option<u32>,
    /// Entry count of a previous scan, used to predict progress and ETA.
    pub expected_entries: Option<u64>,
    pub threads: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedTarget {
    pub category: usize,
    pub min_age: Duration,
    pub kind: ResolvedKind,
}

#[derive(Debug, Clone)]
pub(crate) enum ResolvedKind {
    Tree {
        /// Canonical (link-resolved) root, validated and permitted by the guard.
        root: PathBuf,
        recurse: bool,
        names: NameFilter,
        prune_dirs: bool,
        probe_locks: bool,
    },
    File {
        path: PathBuf,
    },
    RecycleBin,
    /// Exists but cannot be opened at all (access control). Counted as "denied".
    Blocked {
        #[allow(dead_code)]
        root: PathBuf,
    },
}

#[derive(Debug)]
pub struct CategoryResult {
    pub status: CategoryStatus,
    pub stats: CategoryStats,
    /// Sorted by size, largest first.
    pub items: Vec<Item>,
    pub note: Option<String>,
}

/// The outcome of one scan, kept in memory so the preview can page through it and a clean can
/// execute exactly this plan.
#[derive(Debug)]
pub struct Scan {
    pub id: ScanId,
    pub started_at: SystemTime,
    pub elapsed: Duration,
    pub entries: u64,
    pub cancelled: bool,
    pub elevated: bool,
    /// Rule metadata, parallel to `categories`.
    pub rules: Vec<RuleMeta>,
    pub categories: Vec<CategoryResult>,
    pub notes: Vec<String>,
    pub(crate) targets: Vec<ResolvedTarget>,
    pub(crate) guard: Arc<PathGuard>,
}

/// What the user has chosen to clean: whole categories, minus individually excluded items.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub categories: BTreeSet<String>,
    pub excluded: HashSet<ItemId>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemSort {
    #[default]
    SizeDesc,
    SizeAsc,
    NameAsc,
    Oldest,
    Newest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ItemQuery {
    pub offset: usize,
    pub limit: usize,
    pub sort: ItemSort,
    pub search: Option<String>,
}

impl Default for ItemQuery {
    fn default() -> Self {
        ItemQuery { offset: 0, limit: 100, sort: ItemSort::SizeDesc, search: None }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPage {
    /// Items matching the search (before paging).
    pub total: usize,
    /// Their combined size.
    pub bytes: u64,
    /// How many of the matching items are currently excluded.
    pub excluded: usize,
    pub items: Vec<ItemView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanCategory {
    pub id: &'static str,
    pub items: u64,
    pub files: u64,
    pub bytes: u64,
    pub excluded_items: u64,
    pub strategy: Strategy,
    pub risk: Risk,
}

/// What a clean would do right now, computed from the scan and the current selection.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub categories: Vec<PlanCategory>,
    pub items: u64,
    pub files: u64,
    pub bytes: u64,
    /// Bytes that will be moved to Recovery (restorable).
    pub undo_bytes: u64,
    /// Bytes that will be deleted outright.
    pub permanent_bytes: u64,
    /// Selected categories that cannot be undone and are not regenerable by design.
    pub irreversible: Vec<&'static str>,
    pub highest_risk: Option<Risk>,
}

impl Scan {
    pub fn category_index(&self, id: &str) -> Option<usize> {
        self.rules.iter().position(|r| r.id == id)
    }

    pub fn category_summary(&self, idx: usize) -> CategorySummary {
        let c = &self.categories[idx];
        CategorySummary {
            id: self.rules[idx].id,
            status: c.status,
            stats: c.stats.clone(),
            note: c.note.clone(),
        }
    }

    pub fn summary(&self) -> ScanSummary {
        ScanSummary {
            scan_id: self.id,
            elapsed_ms: self.elapsed.as_millis() as u64,
            entries: self.entries,
            total_files: self.categories.iter().map(|c| c.stats.files).sum(),
            total_bytes: self.categories.iter().map(|c| c.stats.bytes).sum(),
            total_items: self.categories.iter().map(|c| c.stats.items).sum(),
            cancelled: self.cancelled,
            elevated: self.elevated,
            categories: (0..self.categories.len()).map(|i| self.category_summary(i)).collect(),
            notes: self.notes.clone(),
        }
    }

    /// The pre-selected categories for a one-click clean: safe, default-on and non-empty.
    pub fn default_selection(&self) -> Selection {
        Selection {
            categories: self
                .rules
                .iter()
                .zip(&self.categories)
                .filter(|(r, c)| r.default_selected && c.status == CategoryStatus::Found)
                .map(|(r, _)| r.id.to_string())
                .collect(),
            excluded: HashSet::new(),
        }
    }

    pub fn items(&self, id: &str, q: &ItemQuery, sel: &Selection) -> Result<ItemPage> {
        let c = self.category_index(id).ok_or_else(|| Error::UnknownCategory(id.to_string()))?;
        let items = &self.categories[c].items;
        let needle = q.search.as_deref().map(str::to_lowercase).filter(|s| !s.is_empty());
        let mut idx: Vec<usize> = (0..items.len())
            .filter(|&i| {
                needle
                    .as_ref()
                    .is_none_or(|n| items[i].path.to_string_lossy().to_lowercase().contains(n))
            })
            .collect();
        match q.sort {
            ItemSort::SizeDesc => {} // stored in this order
            ItemSort::SizeAsc => idx.reverse(),
            ItemSort::NameAsc => idx.sort_by_cached_key(|&i| {
                items[i].path.file_name().map(|n| n.to_string_lossy().to_lowercase())
            }),
            ItemSort::Oldest => idx.sort_by_key(|&i| items[i].modified),
            ItemSort::Newest => idx.sort_by_key(|&i| std::cmp::Reverse(items[i].modified)),
        }
        let total = idx.len();
        let bytes = idx.iter().map(|&i| items[i].size).sum();
        let excluded = idx.iter().filter(|&&i| sel.excluded.contains(&item_id(c, i))).count();
        let page = idx
            .iter()
            .skip(q.offset)
            .take(q.limit.clamp(1, 500))
            .map(|&i| {
                let id = item_id(c, i);
                ItemView::new(id, &items[i], sel.excluded.contains(&id))
            })
            .collect();
        Ok(ItemPage { total, bytes, excluded, items: page })
    }

    pub fn plan(&self, sel: &Selection, policy: DeletionPolicy) -> PlanSummary {
        let mut plan = PlanSummary {
            categories: Vec::new(),
            items: 0,
            files: 0,
            bytes: 0,
            undo_bytes: 0,
            permanent_bytes: 0,
            irreversible: Vec::new(),
            highest_risk: None,
        };
        for (c, (rule, cat)) in self.rules.iter().zip(&self.categories).enumerate() {
            if !sel.categories.contains(rule.id) || cat.items.is_empty() {
                continue;
            }
            let strategy = rule.effective_strategy(policy);
            let (mut items, mut files, mut bytes, mut excluded) = (0u64, 0u64, 0u64, 0u64);
            for (i, item) in cat.items.iter().enumerate() {
                if sel.excluded.contains(&item_id(c, i)) {
                    excluded += 1;
                } else {
                    items += 1;
                    files += item.files;
                    bytes += item.size;
                }
            }
            if items == 0 {
                continue;
            }
            plan.items += items;
            plan.files += files;
            plan.bytes += bytes;
            match strategy {
                Strategy::Undoable => plan.undo_bytes += bytes,
                Strategy::Permanent => plan.permanent_bytes += bytes,
            }
            if strategy == Strategy::Permanent && rule.risk != Risk::Safe {
                plan.irreversible.push(rule.id);
            }
            plan.highest_risk = plan.highest_risk.max(Some(rule.risk));
            plan.categories.push(PlanCategory {
                id: rule.id,
                items,
                files,
                bytes,
                excluded_items: excluded,
                strategy,
                risk: rule.risk,
            });
        }
        plan
    }
}

/// A guard that permits every currently-existing, validated root of `rules` — used to vet
/// restore destinations against the same boundaries a scan would use.
pub(crate) fn guard_for(rules: &[&Rule], base: PathGuard) -> PathGuard {
    resolve::resolve(rules, base, None).guard
}

/// Stack for scan/clean worker threads. Virtual address space only — pages are committed as
/// used — and a generous margin on top of the folder-depth cap ([`walker::MAX_DEPTH`]), whose
/// recursion (including rayon's per-level frames) is the deepest the engine goes.
pub(crate) const WORKER_STACK: usize = 16 * 1024 * 1024;

pub(crate) fn lower_thread_priority() {
    // SAFETY: adjusts only the calling thread through its pseudo-handle; failure is harmless.
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

pub(crate) fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8)
}

struct Shared<'a> {
    guard: &'a PathGuard,
    cancel: &'a CancelToken,
    live: &'a Live,
    now: SystemTime,
}

fn scan_target(idx: usize, t: &ResolvedTarget, sh: &Shared<'_>) -> (Vec<Item>, CategoryStats) {
    match &t.kind {
        ResolvedKind::Tree { root, recurse, names, prune_dirs, probe_locks } => {
            let ctx = WalkCtx {
                guard: sh.guard,
                names: *names,
                min_age: t.min_age,
                now: sh.now,
                recurse: *recurse,
                prune_dirs: *prune_dirs,
                probe_locks: *probe_locks,
                cancel: sh.cancel,
                live: sh.live,
                target: idx,
            };
            let out = walk_dir(&ctx, root, None, true, 0);
            let mut stats = out.stats;
            stats.files += out.files;
            stats.bytes += out.size;
            stats.items += out.items.len() as u64;
            (out.items, stats)
        }
        ResolvedKind::File { path } => {
            let mut stats = CategoryStats::default();
            let Ok(md) = fs::symlink_metadata(path) else { return (Vec::new(), stats) };
            if !md.is_file() || is_reparse(&md) {
                return (Vec::new(), stats);
            }
            sh.live.entries.fetch_add(1, Relaxed);
            let modified = crate::fsops::touched_of(&md);
            if !old_enough(modified, sh.now, t.min_age) {
                stats.kept_files = 1;
                stats.kept_bytes = md.len();
                return (Vec::new(), stats);
            }
            stats.items = 1;
            stats.files = 1;
            stats.bytes = md.len();
            sh.live.found_files.fetch_add(1, Relaxed);
            sh.live.found_bytes.fetch_add(md.len(), Relaxed);
            (vec![Item::file(path.clone(), md.len(), modified, idx)], stats)
        }
        ResolvedKind::RecycleBin => {
            let mut stats = CategoryStats::default();
            match recyclebin::query() {
                Ok((bytes, count)) if count > 0 => {
                    stats.items = 1;
                    stats.files = count;
                    stats.bytes = bytes;
                    sh.live.found_files.fetch_add(count, Relaxed);
                    sh.live.found_bytes.fetch_add(bytes, Relaxed);
                    let item = Item {
                        path: PathBuf::from("Recycle Bin"),
                        kind: ItemKind::RecycleBin,
                        size: bytes,
                        files: count,
                        modified: None,
                        target: idx,
                    };
                    (vec![item], stats)
                }
                Ok(_) => (Vec::new(), stats),
                Err(e) => {
                    tracing::warn!("could not query the Recycle Bin: {e}");
                    stats.unreadable = 1;
                    (Vec::new(), stats)
                }
            }
        }
        ResolvedKind::Blocked { .. } => {
            (Vec::new(), CategoryStats { denied: 1, ..CategoryStats::default() })
        }
    }
}

fn derive_status(
    stats: &CategoryStats,
    found: bool,
    had_targets: bool,
    refused: u32,
    elevated: bool,
) -> CategoryStatus {
    if found {
        CategoryStatus::Found
    } else if stats.denied > 0 {
        if elevated { CategoryStatus::Unavailable } else { CategoryStatus::NeedsAdmin }
    } else if !had_targets && refused > 0 {
        CategoryStatus::Unavailable
    } else {
        CategoryStatus::Empty
    }
}

/// Runs a scan over `rules`. Blocks the calling thread; results stream through `sink`.
pub(crate) fn run(
    rules: &[&Rule],
    base_guard: PathGuard,
    req: &ScanRequest,
    elevated: bool,
    cancel: &CancelToken,
    sink: &dyn Sink<ScanEvent>,
) -> Result<Scan> {
    let clock = Instant::now();
    let started_at = SystemTime::now();
    let scan_id: ScanId = random_u64() & ((1u64 << 48) - 1);

    let resolved = resolve::resolve(rules, base_guard, req.age_override_hours);
    let guard = Arc::new(resolved.guard);
    let n = rules.len();
    sink.emit(ScanEvent::Started {
        scan_id,
        categories: rules.iter().map(|r| r.meta.id).collect(),
    });

    let live = Live::default();
    let done = AtomicU32::new(0);
    let finished = AtomicBool::new(false);
    let threads = req.threads.unwrap_or_else(default_threads).clamp(1, 16);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("vexus-scan-{i}"))
        .stack_size(WORKER_STACK)
        .start_handler(|_| lower_thread_priority())
        .build()
        .map_err(|e| Error::Invalid(format!("could not start worker threads: {e}")))?;
    let shared = Shared { guard: &guard, cancel, live: &live, now: started_at };

    let results: Vec<CategoryResult> = std::thread::scope(|scope| {
        scope.spawn(|| {
            // Reporter: samples the lock-free counters on a fixed cadence so the UI gets a
            // smooth stream without the workers ever blocking on IPC.
            loop {
                for _ in 0..6 {
                    if finished.load(Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(12));
                }
                let entries = live.entries.load(Relaxed);
                let elapsed = clock.elapsed().as_millis() as u64;
                let fraction = req
                    .expected_entries
                    .filter(|e| *e > 0)
                    .map(|e| (entries as f32 / e as f32).min(0.985));
                sink.emit(ScanEvent::Progress(ScanProgress {
                    scan_id,
                    entries,
                    found_files: live.found_files.load(Relaxed),
                    found_bytes: live.found_bytes.load(Relaxed),
                    categories_done: done.load(Relaxed),
                    categories_total: n as u32,
                    current: live.current(),
                    elapsed_ms: elapsed,
                    fraction,
                    eta_ms: fraction.and_then(|f| eta_ms(elapsed, f)),
                }));
            }
        });

        let results = pool.install(|| {
            (0..n)
                .into_par_iter()
                .map(|c| {
                    let mut items: Vec<Item> = Vec::new();
                    let mut stats = CategoryStats::default();
                    let mut had_targets = false;
                    for (idx, t) in
                        resolved.targets.iter().enumerate().filter(|(_, t)| t.category == c)
                    {
                        had_targets = true;
                        if cancel.is_cancelled() {
                            break;
                        }
                        let (i, s) = scan_target(idx, t, &shared);
                        items.extend(i);
                        stats.merge(&s);
                    }
                    items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
                    stats.items = items.len() as u64;
                    let status = derive_status(
                        &stats,
                        !items.is_empty(),
                        had_targets,
                        resolved.refused[c],
                        elevated,
                    );
                    let note = resolved.category_notes[c].clone().or(match status {
                        CategoryStatus::NeedsAdmin => {
                            Some("Windows needs administrator rights to look here.".to_string())
                        }
                        CategoryStatus::Unavailable if stats.denied > 0 => Some(
                            "Windows blocked access even with administrator rights.".to_string(),
                        ),
                        _ => None,
                    });
                    let result = CategoryResult { status, stats, items, note };
                    done.fetch_add(1, Relaxed);
                    sink.emit(ScanEvent::CategoryDone {
                        scan_id,
                        category: CategorySummary {
                            id: rules[c].meta.id,
                            status: result.status,
                            stats: result.stats.clone(),
                            note: result.note.clone(),
                        },
                    });
                    result
                })
                .collect::<Vec<_>>()
        });
        finished.store(true, Relaxed);
        results
    });

    let scan = Scan {
        id: scan_id,
        started_at,
        elapsed: clock.elapsed(),
        entries: live.entries.load(Relaxed),
        cancelled: cancel.is_cancelled(),
        elevated,
        rules: rules.iter().map(|r| r.meta.clone()).collect(),
        categories: results,
        notes: resolved.notes,
        targets: resolved.targets,
        guard,
    };
    tracing::info!(
        scan = scan_id,
        elapsed_ms = scan.elapsed.as_millis() as u64,
        entries = scan.entries,
        bytes = scan.categories.iter().map(|c| c.stats.bytes).sum::<u64>(),
        cancelled = scan.cancelled,
        "scan finished"
    );
    sink.emit(ScanEvent::Finished { summary: scan.summary() });
    Ok(scan)
}
