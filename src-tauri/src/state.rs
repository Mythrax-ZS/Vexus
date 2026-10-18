//! Shared application state.
//!
//! Scan results and the user's selection live **here**, on the Rust side. The UI only ever holds
//! ids that this state handed out, so a compromised or buggy page has no way to ask for an
//! arbitrary path to be removed.

use std::collections::{HashMap, VecDeque};
use std::ops::Deref;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use serde::{Deserialize, Serialize};
use vexus_core::history::History;
use vexus_core::model::ScanId;
use vexus_core::paths::DataDirs;
use vexus_core::progress::CancelToken;
use vexus_core::scan::{Scan, Selection};
use vexus_core::settings::Settings;
use vexus_core::{Engine, Error};

use crate::args::Args;
use crate::logging::Logging;

/// How many recent scans stay in memory (older ones are dropped).
const KEEP_SESSIONS: usize = 2;

/// Locks a mutex, ignoring poisoning (a panicked worker must not wedge the whole app).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Scan,
    Clean,
    Restore,
    /// Freeing Recovery copies. Shares the job slot so it can never delete a batch that a clean
    /// is still writing into or a restore is still reading from.
    Purge,
}

struct Job {
    kind: JobKind,
    cancel: CancelToken,
}

pub struct Session {
    pub scan: Arc<Scan>,
    pub selection: Selection,
}

#[derive(Default)]
struct Sessions {
    map: HashMap<ScanId, Session>,
    order: VecDeque<ScanId>,
}

/// Remembered between runs so the next scan can predict its own progress and ETA.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ScanStats {
    entries: u64,
}

pub struct Inner {
    pub engine: Engine,
    pub data: DataDirs,
    pub args: Args,
    pub sandboxed: bool,
    pub elevated: bool,
    pub settings: RwLock<Settings>,
    pub history: History,
    pub logging: Logging,
    pub quitting: AtomicBool,
    job: Mutex<Option<Job>>,
    sessions: Mutex<Sessions>,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// Releases the single job slot when dropped — including on panic.
pub struct JobGuard(AppState);

impl Drop for JobGuard {
    fn drop(&mut self) {
        *lock(&self.0.job) = None;
    }
}

impl AppState {
    pub fn new(
        engine: Engine,
        data: DataDirs,
        args: Args,
        sandboxed: bool,
        elevated: bool,
        settings: Settings,
        logging: Logging,
    ) -> AppState {
        AppState(Arc::new(Inner {
            history: History::new(&data.state),
            engine,
            data,
            args,
            sandboxed,
            elevated,
            settings: RwLock::new(settings),
            logging,
            quitting: AtomicBool::new(false),
            job: Mutex::new(None),
            sessions: Mutex::new(Sessions::default()),
        }))
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_settings(&self, s: Settings) {
        *self.settings.write().unwrap_or_else(|p| p.into_inner()) = s;
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Relaxed)
    }

    /// Claims the single job slot. Only one scan/clean/restore may run at a time, so the plan
    /// the user reviewed can never change underneath a running clean.
    pub fn begin_job(&self, kind: JobKind) -> Result<(CancelToken, JobGuard), Error> {
        let mut slot = lock(&self.job);
        if slot.is_some() {
            return Err(Error::Busy);
        }
        let cancel = CancelToken::new();
        *slot = Some(Job { kind, cancel: cancel.clone() });
        Ok((cancel, JobGuard(self.clone())))
    }

    /// Cancels whatever is running (if anything). Returns what was running.
    pub fn cancel_job(&self) -> Option<JobKind> {
        let slot = lock(&self.job);
        slot.as_ref().map(|j| {
            j.cancel.cancel();
            j.kind
        })
    }

    pub fn store_scan(&self, scan: Scan) -> Arc<Scan> {
        let scan = Arc::new(scan);
        let selection = scan.default_selection();
        let mut s = lock(&self.sessions);
        s.order.push_back(scan.id);
        s.map.insert(scan.id, Session { scan: scan.clone(), selection });
        while s.order.len() > KEEP_SESSIONS {
            if let Some(old) = s.order.pop_front() {
                s.map.remove(&old);
            }
        }
        scan
    }

    pub fn with_session<R>(
        &self,
        id: ScanId,
        f: impl FnOnce(&mut Session) -> R,
    ) -> Result<R, Error> {
        let mut s = lock(&self.sessions);
        s.map.get_mut(&id).map(f).ok_or(Error::UnknownScan)
    }

    /// Takes the scan and a copy of its selection for a clean.
    pub fn session_for_clean(&self, id: ScanId) -> Result<(Arc<Scan>, Selection), Error> {
        self.with_session(id, |s| (s.scan.clone(), s.selection.clone()))
    }

    /// Forgets a scan (after it has been cleaned its plan is stale).
    pub fn drop_session(&self, id: ScanId) {
        let mut s = lock(&self.sessions);
        s.map.remove(&id);
        s.order.retain(|x| *x != id);
    }

    fn stats_path(&self) -> std::path::PathBuf {
        self.data.state.join("scan-stats.json")
    }

    pub fn last_scan_entries(&self) -> Option<u64> {
        let bytes = std::fs::read(self.stats_path()).ok()?;
        let s: ScanStats = serde_json::from_slice(&bytes).ok()?;
        (s.entries > 0).then_some(s.entries)
    }

    pub fn remember_scan_entries(&self, entries: u64) {
        if let Ok(json) = serde_json::to_vec(&ScanStats { entries }) {
            let _ = vexus_core::util::write_atomic(&self.stats_path(), &json);
        }
    }
}

/// Opens a folder in Explorer (used for "Show logs" and friends).
pub fn open_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}
