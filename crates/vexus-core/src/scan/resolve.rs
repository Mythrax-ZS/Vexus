//! Turns rule definitions into concrete, guard-approved targets for one scan.
//!
//! This is where hostile or broken environments are neutralised: a `%TEMP%` pointing at a
//! drive root, at the user profile or at a work folder makes that target *inert* (with a note
//! for the user) instead of dangerous.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use super::{ResolvedKind, ResolvedTarget};
use crate::fsops::{canonical_dir, is_reparse};
use crate::rules::{RootSanity, Rule, Target};
use crate::safety::{PathGuard, looks_like_temp, normalize, strip_verbatim};
use crate::util::hours;

pub(crate) struct Resolved {
    pub targets: Vec<ResolvedTarget>,
    pub guard: PathGuard,
    /// Per category: a note explaining refused or unreadable locations.
    pub category_notes: Vec<Option<String>>,
    /// Per category: how many declared locations were refused.
    pub refused: Vec<u32>,
    /// User-visible notes about the scan as a whole.
    pub notes: Vec<String>,
}

pub(crate) fn resolve(
    rules: &[&Rule],
    base: PathGuard,
    age_override_hours: Option<u32>,
) -> Resolved {
    let mut guard = base;
    let mut targets = Vec::new();
    let mut category_notes: Vec<Option<String>> = vec![None; rules.len()];
    let mut refused = vec![0u32; rules.len()];
    let mut notes = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (category, rule) in rules.iter().enumerate() {
        let meta = &rule.meta;
        let hours_gate = if meta.age_adjustable {
            age_override_hours.unwrap_or(meta.min_age_hours)
        } else {
            meta.min_age_hours
        };
        let min_age: Duration = hours(hours_gate);

        let mut refuse = |why: String, category_notes: &mut Vec<Option<String>>| {
            tracing::warn!(rule = meta.id, "{why}");
            refused[category] += 1;
            let line = format!("Skipped part of “{}”: {why}", meta.name);
            notes.push(line);
            category_notes[category].get_or_insert(why);
        };

        for target in &rule.targets {
            match target {
                Target::Tree(t) => {
                    if t.sanity == RootSanity::LooksLikeTemp && !looks_like_temp(&t.root) {
                        refuse(
                            format!(
                                "{} doesn't look like a temporary folder, so Vexus left it alone",
                                strip_verbatim(&t.root).display()
                            ),
                            &mut category_notes,
                        );
                        continue;
                    }
                    // A fixed system location that has turned into a link (anyone who may write
                    // there can do that to an empty folder) would quietly point the rule at an
                    // entirely different folder. Redirected *ancestors* are fine: only the
                    // folder itself must be the real thing.
                    if t.sanity == RootSanity::None
                        && std::fs::symlink_metadata(&t.root).is_ok_and(|m| is_reparse(&m))
                    {
                        refuse(
                            format!(
                                "{} is a link to another folder, so Vexus left it alone",
                                strip_verbatim(&t.root).display()
                            ),
                            &mut category_notes,
                        );
                        continue;
                    }
                    let root = match canonical_dir(&t.root) {
                        Ok(r) => r,
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                            ) =>
                        {
                            continue; // not present on this PC: nothing to do
                        }
                        Err(e) => {
                            tracing::debug!(rule = meta.id, root = %t.root.display(), "cannot open root: {e}");
                            // Likely an ACL block; the walker will count it as denied.
                            if let Some(n) = normalize(&t.root)
                                && seen.insert(n)
                            {
                                targets.push(ResolvedTarget {
                                    category,
                                    min_age,
                                    kind: ResolvedKind::Blocked { root: t.root.clone() },
                                });
                            }
                            continue;
                        }
                    };
                    if t.sanity == RootSanity::LooksLikeTemp && !looks_like_temp(&root) {
                        refuse(
                            format!("{} doesn't look like a temporary folder", root.display()),
                            &mut category_notes,
                        );
                        continue;
                    }
                    if let Err(why) = guard.permit_root(&root) {
                        refuse(
                            format!("{} was refused because {why}", root.display()),
                            &mut category_notes,
                        );
                        continue;
                    }
                    if !seen.insert(normalize(&root).unwrap_or_default()) {
                        continue; // already covered by another target (e.g. TEMP == LOCALAPPDATA\Temp)
                    }
                    targets.push(ResolvedTarget {
                        category,
                        min_age,
                        kind: ResolvedKind::Tree {
                            root,
                            recurse: t.recurse,
                            names: t.names,
                            prune_dirs: t.prune_dirs,
                            probe_locks: t.probe_locks,
                        },
                    });
                }
                Target::File(path) => {
                    let Some(name) = path.file_name() else { continue };
                    let Some(parent) = path.parent() else { continue };
                    let Ok(parent) = canonical_dir(parent) else { continue };
                    let file: PathBuf = parent.join(name);
                    // Permit the exact path even while the file is absent: restoring a recovered
                    // copy later must still be allowed to put it back.
                    if let Err(why) = guard.permit_file(&file) {
                        refuse(
                            format!("{} was refused because {why}", file.display()),
                            &mut category_notes,
                        );
                        continue;
                    }
                    match std::fs::symlink_metadata(&file) {
                        Ok(md) if md.is_file() && !is_reparse(&md) => {}
                        _ => continue, // nothing to scan right now
                    }
                    targets.push(ResolvedTarget {
                        category,
                        min_age,
                        kind: ResolvedKind::File { path: file },
                    });
                }
                Target::RecycleBin => {
                    targets.push(ResolvedTarget {
                        category,
                        min_age,
                        kind: ResolvedKind::RecycleBin,
                    });
                }
            }
        }
    }

    // One cleaned root can sit inside another (a remote-desktop session's TEMP is `…\Temp\2`,
    // inside `…\Temp`). Each root is walked by its own target, so every *other* walk must treat
    // it as an opaque folder: nothing is then counted twice, and a root is never removed as an
    // item of the root around it.
    let roots: Vec<PathBuf> = targets
        .iter()
        .filter_map(|t| match &t.kind {
            ResolvedKind::Tree { root, .. } => Some(root.clone()),
            _ => None,
        })
        .collect();
    for root in &roots {
        guard.shield_root(root);
    }

    Resolved { targets, guard, category_notes, refused, notes }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::env::Env;
    use crate::rules::{Rule, builtin};

    fn sandbox() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical_dir(dir.path()).unwrap();
        let env = Env::sandbox(&root);
        env.create_sandbox_skeleton().unwrap();
        (dir, env)
    }

    fn resolve_all(env: &Env) -> (Vec<Rule>, Resolved) {
        let rules = builtin(env);
        let refs: Vec<&Rule> = rules.iter().collect();
        let resolved = resolve(&refs, PathGuard::new(env, &[]), None);
        (rules, resolved)
    }

    fn roots(r: &Resolved) -> Vec<PathBuf> {
        r.targets
            .iter()
            .filter_map(|t| match &t.kind {
                ResolvedKind::Tree { root, .. } => Some(root.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn missing_locations_are_silently_absent() {
        let (_d, env) = sandbox();
        let (_, r) = resolve_all(&env);
        assert_eq!(roots(&r), vec![env.temp.clone()], "only the existing temp folder resolves");
        assert!(r.notes.is_empty(), "{:?}", r.notes);
    }

    #[test]
    fn existing_locations_resolve_and_are_permitted() {
        let (_d, env) = sandbox();
        fs::create_dir_all(env.system_root.join("Temp")).unwrap();
        fs::create_dir_all(env.system_root.join("Prefetch")).unwrap();
        let (_, r) = resolve_all(&env);
        let roots = roots(&r);
        assert!(roots.contains(&env.system_root.join("Temp")));
        assert!(roots.contains(&env.system_root.join("Prefetch")));
        assert!(r.guard.is_protected(&env.system_root.join("Temp").join("x.tmp")).is_none());
        assert!(r.guard.is_protected(&env.system_root.join("notepad.exe")).is_some());
    }

    #[test]
    fn temp_pointing_at_documents_becomes_inert() {
        let (_d, mut env) = sandbox();
        env.temp = env.documents.clone();
        let (_, r) = resolve_all(&env);
        assert!(roots(&r).iter().all(|p| *p != env.documents));
        assert!(r.guard.is_protected(&env.documents.join("thesis.docx")).is_some());
        assert!(r.notes.iter().any(|n| n.contains("temporary")), "{:?}", r.notes);
    }

    #[test]
    fn temp_pointing_at_the_profile_or_a_drive_becomes_inert() {
        for hostile in [r"C:\", r"D:\"] {
            let (_d, mut env) = sandbox();
            env.temp = PathBuf::from(hostile);
            let (_, r) = resolve_all(&env);
            assert!(roots(&r).iter().all(|p| p != Path::new(hostile)), "{hostile}");
        }
        let (_d, mut env) = sandbox();
        env.temp = env.user_profile.clone();
        let (_, r) = resolve_all(&env);
        assert!(roots(&r).iter().all(|p| *p != env.user_profile));
        assert!(r.guard.is_protected(&env.user_profile.join("anything")).is_some());
    }

    #[test]
    fn work_folder_named_oddly_is_not_accepted_as_temp() {
        let (_d, mut env) = sandbox();
        env.temp = env.user_profile.join("Work");
        fs::create_dir_all(&env.temp).unwrap();
        let (_, r) = resolve_all(&env);
        assert!(roots(&r).iter().all(|p| *p != env.temp));
    }

    #[test]
    fn duplicate_roots_are_scanned_once() {
        let (_d, env) = sandbox();
        // The sandbox already has %TEMP% == %LOCALAPPDATA%\Temp.
        let (_, r) = resolve_all(&env);
        assert_eq!(roots(&r).iter().filter(|p| **p == env.temp).count(), 1);
    }

    #[test]
    fn single_file_targets_resolve_only_if_the_file_exists() {
        let (_d, env) = sandbox();
        let (_, r) = resolve_all(&env);
        assert!(!r.targets.iter().any(|t| matches!(t.kind, ResolvedKind::File { .. })));
        assert!(
            r.guard.is_protected(&env.system_root.join("MEMORY.DMP")).is_none(),
            "the exact path stays permitted while absent, so a recovered copy can be restored"
        );
        fs::write(env.system_root.join("MEMORY.DMP"), b"dump").unwrap();
        let (_, r) = resolve_all(&env);
        assert!(r.targets.iter().any(
            |t| matches!(&t.kind, ResolvedKind::File { path } if path.ends_with("MEMORY.DMP"))
        ));
        assert!(r.guard.is_protected(&env.system_root.join("MEMORY.DMP")).is_none());
        assert!(r.guard.is_protected(&env.system_root.join("other.dmp")).is_some());
    }

    #[test]
    fn age_override_applies_only_to_adjustable_rules() {
        let (_d, env) = sandbox();
        fs::create_dir_all(env.system_root.join("SoftwareDistribution").join("Download")).unwrap();
        let rules = builtin(&env);
        let refs: Vec<&Rule> = rules.iter().collect();
        let r = resolve(&refs, PathGuard::new(&env, &[]), Some(1));
        let temp = r
            .targets
            .iter()
            .find(|t| matches!(&t.kind, ResolvedKind::Tree { root, .. } if *root == env.temp))
            .unwrap();
        assert_eq!(temp.min_age, hours(1));
        let wu = r.targets.iter().find(|t| matches!(&t.kind, ResolvedKind::Tree { root, .. } if root.ends_with("Download"))).unwrap();
        assert_eq!(wu.min_age, hours(48), "fixed-age rules ignore the override");
    }
}
