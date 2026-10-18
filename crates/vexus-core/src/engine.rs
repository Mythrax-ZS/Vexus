//! [`Engine`]: the one object front ends talk to.

use std::path::PathBuf;

use crate::clean::{self, CleanOptions, CleanReport};
use crate::env::Env;
use crate::error::{Error, Result};
use crate::paths::DataDirs;
use crate::progress::{CancelToken, CleanEvent, ScanEvent, Sink};
use crate::recovery::{RecoveryStore, RestoreReport};
use crate::rules::{self, Rule, RuleMeta};
use crate::safety::PathGuard;
use crate::scan::{self, Scan, ScanRequest, Selection};

pub struct Engine {
    env: Env,
    rules: Vec<Rule>,
    /// Vexus's own data, which no rule may ever touch.
    deny: Vec<PathBuf>,
    recovery: RecoveryStore,
    elevated: bool,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("rules", &self.rules.len())
            .field("elevated", &self.elevated)
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub fn new(env: Env, data: &DataDirs, elevated: bool) -> Engine {
        let rules = rules::builtin(&env);
        Engine::with_rules(env, rules, data, elevated)
    }

    /// For tests and custom rule sets.
    pub fn with_rules(env: Env, rules: Vec<Rule>, data: &DataDirs, elevated: bool) -> Engine {
        Engine {
            deny: vec![data.root.clone()],
            recovery: RecoveryStore::new(&data.recovery),
            env,
            rules,
            elevated,
        }
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    pub fn elevated(&self) -> bool {
        self.elevated
    }

    pub fn recovery(&self) -> &RecoveryStore {
        &self.recovery
    }

    /// Metadata of every rule, in display order.
    pub fn rules(&self) -> Vec<RuleMeta> {
        self.rules.iter().map(|r| r.meta.clone()).collect()
    }

    fn base_guard(&self) -> PathGuard {
        PathGuard::new(&self.env, &self.deny)
    }

    /// Scans the requested categories (all by default). Read-only.
    pub fn scan(
        &self,
        req: &ScanRequest,
        cancel: &CancelToken,
        sink: &dyn Sink<ScanEvent>,
    ) -> Result<Scan> {
        let selected: Vec<&Rule> = match &req.categories {
            None => self.rules.iter().collect(),
            Some(ids) => {
                for id in ids {
                    if !self.rules.iter().any(|r| r.meta.id == id) {
                        return Err(Error::UnknownCategory(id.clone()));
                    }
                }
                self.rules.iter().filter(|r| ids.iter().any(|i| i == r.meta.id)).collect()
            }
        };
        scan::run(&selected, self.base_guard(), req, self.elevated, cancel, sink)
    }

    /// Executes `selection` from `scan`, re-verifying every item.
    pub fn clean(
        &self,
        scan: &Scan,
        selection: &Selection,
        opts: &CleanOptions,
        cancel: &CancelToken,
        sink: &dyn Sink<CleanEvent>,
    ) -> Result<CleanReport> {
        clean::run(scan, selection, opts, &self.recovery, cancel, sink)
    }

    /// Restores a Recovery batch, but only to places the current rules still manage.
    pub fn restore(&self, batch_id: &str, cancel: &CancelToken) -> Result<RestoreReport> {
        let all: Vec<&Rule> = self.rules.iter().collect();
        let guard = scan::guard_for(&all, self.base_guard());
        self.recovery.restore(batch_id, &guard, cancel)
    }
}
