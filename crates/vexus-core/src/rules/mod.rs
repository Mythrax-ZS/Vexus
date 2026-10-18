//! Cleaning rules: the *only* source of things Vexus may remove.
//!
//! A rule declares where to look, what qualifies, how old it must be, how risky it is and
//! whether removal is undoable. Rules are plain data resolved against an [`Env`](crate::env::Env),
//! so they can be unit-tested against a fake profile.

mod builtin;

use std::path::PathBuf;

use serde::Serialize;

use crate::model::{DeletionPolicy, Group, Risk, Strategy};
use crate::util::wildcard_match;

pub use builtin::builtin;

/// Which files inside a directory qualify.
#[derive(Debug, Clone, Copy)]
pub enum NameFilter {
    Any,
    /// Case-insensitive wildcard patterns (`*` and `?`), matched against the file name.
    Patterns(&'static [&'static str]),
}

impl NameFilter {
    pub fn matches(&self, name: &str) -> bool {
        match self {
            NameFilter::Any => true,
            NameFilter::Patterns(patterns) => patterns.iter().any(|p| wildcard_match(p, name)),
        }
    }
}

/// Extra scrutiny for roots that come from variables a user (or malware) can repoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSanity {
    None,
    /// The root must look like a temp folder (`%TEMP%` could have been set to `D:\Work`).
    LooksLikeTemp,
}

/// A directory tree to examine. Items are strict descendants of `root`; `root` itself is never
/// removed.
#[derive(Debug, Clone)]
pub struct TreeTarget {
    pub root: PathBuf,
    pub recurse: bool,
    pub names: NameFilter,
    /// Fold fully-eligible folders into a single item and remove the folder too. When false the
    /// folder structure is preserved and only files are removed.
    pub prune_dirs: bool,
    /// Try to open each candidate for deletion at scan time, so files that are in use are
    /// reported honestly instead of being promised and then skipped. Costly; for small sets.
    pub probe_locks: bool,
    pub sanity: RootSanity,
}

impl TreeTarget {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        TreeTarget {
            root: root.into(),
            recurse: true,
            names: NameFilter::Any,
            prune_dirs: true,
            probe_locks: false,
            sanity: RootSanity::None,
        }
    }

    pub fn flat(mut self) -> Self {
        self.recurse = false;
        self
    }

    pub fn names(mut self, patterns: &'static [&'static str]) -> Self {
        self.names = NameFilter::Patterns(patterns);
        self
    }

    pub fn keep_folders(mut self) -> Self {
        self.prune_dirs = false;
        self
    }

    pub fn probe_locks(mut self) -> Self {
        self.probe_locks = true;
        self
    }

    pub fn temp_like(mut self) -> Self {
        self.sanity = RootSanity::LooksLikeTemp;
        self
    }
}

#[derive(Debug, Clone)]
pub enum Target {
    Tree(TreeTarget),
    /// One specific file (never its folder).
    File(PathBuf),
    /// The Windows Recycle Bin on all drives, emptied through the shell.
    RecycleBin,
}

/// The three plain-language answers shown for every category.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDetail {
    /// What is this?
    pub what: &'static str,
    /// What happens if I remove it?
    pub effect: &'static str,
    /// How does Vexus keep that safe?
    pub safety: &'static str,
}

/// Everything about a rule that the UI needs (the targets stay inside the engine).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleMeta {
    pub id: &'static str,
    pub group: Group,
    pub name: &'static str,
    pub summary: &'static str,
    pub detail: RuleDetail,
    pub icon: &'static str,
    pub risk: Risk,
    pub strategy: Strategy,
    /// The user's deletion policy cannot change `strategy` (e.g. the Recycle Bin can only ever
    /// be emptied for good).
    pub strategy_locked: bool,
    /// Pre-selected for a one-click clean.
    pub default_selected: bool,
    /// All of its locations need administrator rights.
    pub needs_admin: bool,
    /// Files younger than this are never touched (0 = no limit).
    pub min_age_hours: u32,
    /// The user may change `min_age_hours` in settings.
    pub age_adjustable: bool,
}

impl RuleMeta {
    /// What will actually happen to this category's data under the user's deletion policy.
    pub fn effective_strategy(&self, policy: DeletionPolicy) -> Strategy {
        if self.strategy_locked { self.strategy } else { policy.resolve(self.strategy) }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub meta: RuleMeta,
    pub targets: Vec<Target>,
}
