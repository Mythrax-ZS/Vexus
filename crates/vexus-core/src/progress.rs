//! Cancellation and the event streams that scans and cleans report through.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;

use crate::clean::CleanReport;
use crate::error::UserError;
use crate::model::{CategoryStats, CategoryStatus, ScanId};

/// Cooperative cancellation shared between the front end and the worker threads.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Lock-free counters that worker threads bump and a reporter thread samples.
#[derive(Debug, Default)]
pub struct Live {
    pub entries: AtomicU64,
    pub bytes_seen: AtomicU64,
    pub found_files: AtomicU64,
    pub found_bytes: AtomicU64,
    ticks: AtomicU64,
    current: Mutex<String>,
}

impl Live {
    /// Records the directory being read. Throttled so it costs almost nothing per call.
    pub fn note_dir(&self, dir: &std::path::Path) {
        if self.ticks.fetch_add(1, Ordering::Relaxed).is_multiple_of(48)
            && let Ok(mut cur) = self.current.try_lock()
        {
            *cur = dir.display().to_string();
        }
    }

    pub fn current(&self) -> String {
        self.current.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorySummary {
    pub id: &'static str,
    pub status: CategoryStatus,
    pub stats: CategoryStats,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub scan_id: ScanId,
    /// Directory entries examined so far.
    pub entries: u64,
    pub found_files: u64,
    pub found_bytes: u64,
    pub categories_done: u32,
    pub categories_total: u32,
    /// Folder currently being read (sampled, so it hops rather than crawls).
    pub current: String,
    pub elapsed_ms: u64,
    /// 0..1 when a previous scan makes the total predictable.
    pub fraction: Option<f32>,
    pub eta_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub scan_id: ScanId,
    pub elapsed_ms: u64,
    pub entries: u64,
    pub total_files: u64,
    pub total_bytes: u64,
    pub total_items: u64,
    pub cancelled: bool,
    pub elevated: bool,
    pub categories: Vec<CategorySummary>,
    /// Plain-language notes about locations that were refused or skipped.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ScanEvent {
    Started { scan_id: ScanId, categories: Vec<&'static str> },
    Progress(ScanProgress),
    CategoryDone { scan_id: ScanId, category: CategorySummary },
    Finished { summary: ScanSummary },
    Failed { scan_id: ScanId, error: UserError },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanProgress {
    pub files_done: u64,
    pub bytes_done: u64,
    pub files_total: u64,
    pub bytes_total: u64,
    pub category: Option<&'static str>,
    pub categories_done: u32,
    pub categories_total: u32,
    pub elapsed_ms: u64,
    pub fraction: f32,
    pub eta_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CleanEvent {
    Started { files_total: u64, bytes_total: u64, categories: Vec<&'static str> },
    Progress(CleanProgress),
    CategoryDone { id: &'static str, removed_files: u64, removed_bytes: u64 },
    Finished { report: Box<CleanReport> },
    Failed { error: UserError },
}

/// Sink for events. Closures that are `Fn(Event) + Sync` can be passed directly.
pub trait Sink<E>: Sync {
    fn emit(&self, event: E);
}

impl<E, F: Fn(E) + Sync> Sink<E> for F {
    fn emit(&self, event: E) {
        self(event);
    }
}

/// A sink that discards everything (tests, quick checks).
#[derive(Debug, Clone, Copy)]
pub struct NullSink;

impl<E> Sink<E> for NullSink {
    fn emit(&self, _event: E) {}
}

/// Estimates remaining time from a completed fraction, once there is enough signal to trust.
pub fn eta_ms(elapsed_ms: u64, fraction: f32) -> Option<u64> {
    if elapsed_ms < 600 || !(0.04..0.995).contains(&fraction) {
        return None;
    }
    let remaining = (elapsed_ms as f64) * (1.0 - f64::from(fraction)) / f64::from(fraction);
    Some(remaining as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_needs_signal_and_is_proportional() {
        assert_eq!(eta_ms(100, 0.5), None, "too early to trust");
        assert_eq!(eta_ms(5_000, 0.01), None, "fraction too small to extrapolate");
        assert_eq!(eta_ms(5_000, 1.0), None);
        assert_eq!(eta_ms(4_000, 0.5), Some(4_000));
        assert_eq!(eta_ms(2_000, 0.25), Some(6_000));
    }

    #[test]
    fn cancel_is_shared_between_clones() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn closures_are_sinks() {
        let n = std::sync::atomic::AtomicU32::new(0);
        let f = |_: u8| {
            n.fetch_add(1, Ordering::Relaxed);
        };
        Sink::emit(&f, 1u8);
        Sink::emit(&f, 2u8);
        assert_eq!(n.load(Ordering::Relaxed), 2);
    }
}
