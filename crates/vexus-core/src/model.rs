//! Plain data types shared by the scanner, cleaner and front ends.
//!
//! Everything that crosses the IPC boundary derives `Serialize` with camelCase names so the UI
//! can consume it directly.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::util::unix_secs;

/// Identifies one scan session. Kept below 2^53 so JavaScript can hold it exactly.
pub type ScanId = u64;

/// Identifies one item inside a scan: `(category index << 32) | index within category`.
pub type ItemId = u64;

pub fn item_id(category: usize, index: usize) -> ItemId {
    ((category as u64) << 32) | (index as u64 & 0xFFFF_FFFF)
}

pub fn split_item_id(id: ItemId) -> (usize, usize) {
    ((id >> 32) as usize, (id & 0xFFFF_FFFF) as usize)
}

/// How much care a category deserves. Drives defaults, colours and the confirmation wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Risk {
    /// Rebuilt automatically; no loss of data, settings or convenience worth mentioning.
    Safe,
    /// Harmless to the system but you may lose history, convenience or troubleshooting detail.
    Review,
    /// Contains things that cannot be rebuilt or that you may still want.
    Caution,
}

/// What happens to removed data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Strategy {
    /// Deleted outright. Used for data that regenerates by design.
    Permanent,
    /// Moved into the recovery store first, so it can be restored for a while.
    Undoable,
}

/// User-configurable override of each rule's own strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeletionPolicy {
    /// Each category decides: regenerable caches are deleted, everything else is undoable.
    #[default]
    Smart,
    /// Everything is moved to recovery first (space is freed when the copy expires).
    AlwaysUndo,
    /// Everything is deleted immediately.
    AlwaysPermanent,
}

impl DeletionPolicy {
    pub fn resolve(self, rule: Strategy) -> Strategy {
        match self {
            DeletionPolicy::Smart => rule,
            DeletionPolicy::AlwaysUndo => Strategy::Undoable,
            DeletionPolicy::AlwaysPermanent => Strategy::Permanent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    System,
    Privacy,
    Browsers,
    Apps,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::System => "Windows",
            Group::Privacy => "Privacy",
            Group::Browsers => "Browsers",
            Group::Apps => "Apps & games",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    File,
    /// A whole directory whose entire contents are eligible.
    Dir,
    /// The Windows Recycle Bin (emptied through the shell, not file by file).
    RecycleBin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CategoryStatus {
    /// Something removable was found.
    Found,
    /// Looked, nothing to remove.
    Empty,
    /// Windows refused access; elevation would reveal more (or everything).
    NeedsAdmin,
    /// The location was refused by the safety guard or could not be examined.
    Unavailable,
}

/// Numbers describing one category after a scan. Mergeable across targets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryStats {
    /// Entries in the plan (a collapsed folder counts as one).
    pub items: u64,
    /// Files that would be removed.
    pub files: u64,
    /// Bytes that would be removed.
    pub bytes: u64,
    /// Files left alone because they are newer than the age limit.
    pub kept_files: u64,
    pub kept_bytes: u64,
    /// Files in use by another program at scan time (left alone).
    pub busy_files: u64,
    pub busy_bytes: u64,
    /// Links (symlinks/junctions) that were skipped rather than followed.
    pub links: u64,
    /// Locations Windows would not let us list or open.
    pub denied: u64,
    /// Locations that failed for other reasons.
    pub unreadable: u64,
}

impl CategoryStats {
    pub fn merge(&mut self, o: &CategoryStats) {
        self.items += o.items;
        self.files += o.files;
        self.bytes += o.bytes;
        self.kept_files += o.kept_files;
        self.kept_bytes += o.kept_bytes;
        self.busy_files += o.busy_files;
        self.busy_bytes += o.busy_bytes;
        self.links += o.links;
        self.denied += o.denied;
        self.unreadable += o.unreadable;
    }
}

/// One entry of a cleaning plan.
#[derive(Debug, Clone)]
pub struct Item {
    pub path: PathBuf,
    pub kind: ItemKind,
    /// Total bytes beneath (logical size).
    pub size: u64,
    /// Files beneath (1 for a file).
    pub files: u64,
    /// For a file: its modification time at scan. For a folder: its newest file.
    pub modified: Option<SystemTime>,
    /// Index into the scan's resolved targets (which root it came from).
    pub target: usize,
}

impl Item {
    pub fn file(path: PathBuf, size: u64, modified: Option<SystemTime>, target: usize) -> Self {
        Item { path, kind: ItemKind::File, size, files: 1, modified, target }
    }

    pub fn dir(
        path: PathBuf,
        size: u64,
        files: u64,
        modified: Option<SystemTime>,
        target: usize,
    ) -> Self {
        Item { path, kind: ItemKind::Dir, size, files, modified, target }
    }
}

/// An [`Item`] as shown in the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub id: ItemId,
    pub name: String,
    /// Parent folder, for display.
    pub dir: String,
    pub path: String,
    pub kind: ItemKind,
    pub size: u64,
    pub files: u64,
    pub modified: Option<i64>,
    pub excluded: bool,
}

impl ItemView {
    pub fn new(id: ItemId, item: &Item, excluded: bool) -> Self {
        let name = match item.kind {
            ItemKind::RecycleBin => "Recycle Bin".to_string(),
            _ => item.path.file_name().map_or_else(
                || item.path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
        };
        let dir = match item.kind {
            ItemKind::RecycleBin => "All drives".to_string(),
            _ => item.path.parent().map(display_path).unwrap_or_default(),
        };
        ItemView {
            id,
            name,
            dir,
            path: display_path(&item.path),
            kind: item.kind,
            size: item.size,
            files: item.files,
            modified: item.modified.map(unix_secs),
            excluded,
        }
    }
}

fn display_path(p: &Path) -> String {
    p.display().to_string()
}
