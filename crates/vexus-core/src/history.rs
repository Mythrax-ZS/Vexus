//! An append-only record of what Vexus has done, for transparency and the dashboard.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util::now_unix;

const FILE: &str = "history.jsonl";
/// Rewrite the file once it has this many lines, keeping the newest half.
const MAX_LINES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryKind {
    Scan,
    Clean,
    Restore,
    Purge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub ts: i64,
    pub kind: HistoryKind,
    pub files: u64,
    pub bytes: u64,
    /// Part of `bytes` that went to Recovery rather than being deleted.
    pub undo_bytes: u64,
    pub categories: Vec<String>,
    pub batch: Option<String>,
}

impl HistoryEntry {
    pub fn new(kind: HistoryKind, files: u64, bytes: u64) -> Self {
        HistoryEntry {
            ts: now_unix(),
            kind,
            files,
            bytes,
            undo_bytes: 0,
            categories: Vec::new(),
            batch: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub cleans: u64,
    pub files: u64,
    pub bytes: u64,
    pub last_clean: Option<HistoryEntry>,
    pub last_scan: Option<HistoryEntry>,
}

#[derive(Debug, Clone)]
pub struct History {
    path: PathBuf,
}

impl History {
    pub fn new(state_dir: &Path) -> Self {
        History { path: state_dir.join(FILE) }
    }

    pub fn append(&self, entry: &HistoryEntry) {
        let Ok(mut line) = serde_json::to_string(entry) else { return };
        line.push('\n');
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = written {
            tracing::warn!("couldn't record history: {e}");
        }
        self.compact_if_needed();
    }

    /// Newest first.
    pub fn recent(&self, n: usize) -> Vec<HistoryEntry> {
        let mut all = self.all();
        all.reverse();
        all.truncate(n);
        all
    }

    pub fn totals(&self) -> Totals {
        let mut t = Totals::default();
        for e in self.all() {
            match e.kind {
                HistoryKind::Clean => {
                    t.cleans += 1;
                    t.files += e.files;
                    t.bytes += e.bytes;
                    t.last_clean = Some(e);
                }
                HistoryKind::Scan => t.last_scan = Some(e),
                _ => {}
            }
        }
        t
    }

    fn all(&self) -> Vec<HistoryEntry> {
        fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn compact_if_needed(&self) {
        let Ok(text) = fs::read_to_string(&self.path) else { return };
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() < MAX_LINES {
            return;
        }
        let keep = lines[lines.len() - MAX_LINES / 2..].join("\n") + "\n";
        let _ = crate::util::write_atomic(&self.path, keep.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_reads_back_newest_first_and_totals() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path());
        assert!(h.recent(5).is_empty());
        h.append(&HistoryEntry::new(HistoryKind::Scan, 10, 1000));
        let mut clean = HistoryEntry::new(HistoryKind::Clean, 4, 400);
        clean.undo_bytes = 100;
        h.append(&clean);
        h.append(&HistoryEntry::new(HistoryKind::Clean, 6, 600));
        let recent = h.recent(2);
        assert_eq!(recent[0].files, 6);
        assert_eq!(recent[1].kind, HistoryKind::Clean);
        let t = h.totals();
        assert_eq!((t.cleans, t.files, t.bytes), (2, 10, 1000));
        assert_eq!(t.last_scan.unwrap().bytes, 1000);
    }

    #[test]
    fn corrupt_lines_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE), "garbage\n{\"ts\":1}\n").unwrap();
        let h = History::new(dir.path());
        h.append(&HistoryEntry::new(HistoryKind::Clean, 1, 1));
        assert_eq!(h.recent(10).len(), 1);
    }

    #[test]
    fn the_file_is_compacted_instead_of_growing_forever() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path());
        for i in 0..(MAX_LINES + 5) {
            h.append(&HistoryEntry::new(HistoryKind::Clean, i as u64, 1));
        }
        let all = h.recent(usize::MAX);
        assert!(all.len() <= MAX_LINES, "{}", all.len());
        assert_eq!(all[0].files, (MAX_LINES + 4) as u64, "newest entry survives compaction");
    }
}
