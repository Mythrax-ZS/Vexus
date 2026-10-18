//! The built-in rule set. Adding a cleaner means adding an entry here — nothing else in the
//! engine needs to change, and the safety guard validates every root it declares.

use std::path::{Path, PathBuf};

use super::{Rule, RuleDetail, RuleMeta, Target, TreeTarget};
use crate::env::Env;
use crate::model::{Group, Risk, Strategy};
use crate::safety::normalize;

/// All built-in rules, in display order, resolved against `env`.
pub fn builtin(env: &Env) -> Vec<Rule> {
    let local = &env.local_app_data;
    let windows = &env.system_root;
    let wer = |base: &Path| -> Vec<Target> {
        ["ReportArchive", "ReportQueue", "Temp"]
            .iter()
            .map(|d| tree(base.join("Microsoft").join("Windows").join("WER").join(d)))
            .collect()
    };

    // %TEMP% is user-controllable, so it gets the temp-name sanity check; if it differs from
    // %LOCALAPPDATA%\Temp both are covered.
    let mut user_temp = vec![Target::Tree(TreeTarget::new(&env.temp).temp_like())];
    let default_temp = local.join("Temp");
    if normalize(&default_temp) != normalize(&env.temp) {
        user_temp.push(Target::Tree(TreeTarget::new(default_temp).temp_like()));
    }

    let mut error_reports = vec![tree(local.join("CrashDumps"))];
    error_reports.extend(wer(local));
    error_reports.extend(wer(&env.program_data));
    error_reports.push(tree(windows.join("Minidump")));
    error_reports.push(tree(windows.join("LiveKernelReports")));
    error_reports.push(Target::File(windows.join("MEMORY.DMP")));

    let mut rules = vec![
        Rule {
            meta: RuleMeta {
                id: "windows.user-temp",
                group: Group::System,
                name: "Temporary files",
                summary: "Scratch files left behind by apps and installers",
                detail: RuleDetail {
                    what: "Apps, installers and Windows drop working files into your Temp folder. They are meant to be disposable, but many are never cleaned up.",
                    effect: "Nothing you use directly. Apps recreate anything they still need.",
                    safety: "Only files that haven't changed for a few days are included (you can adjust this). Anything in use is skipped — never forced.",
                },
                icon: "temp",
                risk: Risk::Safe,
                strategy: Strategy::Permanent,
                strategy_locked: false,
                default_selected: true,
                needs_admin: false,
                min_age_hours: 72,
                age_adjustable: true,
            },
            targets: user_temp,
        },
        Rule {
            meta: RuleMeta {
                id: "windows.system-temp",
                group: Group::System,
                name: "Windows temporary files",
                summary: "System-wide scratch space used by Windows and installers",
                detail: RuleDetail {
                    what: "The shared Temp folder inside the Windows directory, used by Windows services, installers and updates.",
                    effect: "Nothing you use directly. Services recreate anything they still need.",
                    safety: "Needs administrator rights. Recent files and files in use are left alone.",
                },
                icon: "windows",
                risk: Risk::Safe,
                strategy: Strategy::Permanent,
                strategy_locked: false,
                default_selected: true,
                needs_admin: true,
                min_age_hours: 72,
                age_adjustable: true,
            },
            targets: vec![tree(windows.join("Temp"))],
        },
        Rule {
            meta: RuleMeta {
                id: "windows.update-cache",
                group: Group::System,
                name: "Windows Update downloads",
                summary: "Update packages Windows has already downloaded",
                detail: RuleDetail {
                    what: "Windows keeps the update files it downloads in SoftwareDistribution\\Download. Once an update is installed, its download is no longer needed.",
                    effect: "If an update hasn't finished installing, Windows simply downloads it again.",
                    safety: "Needs administrator rights. Files from the last two days are kept so a pending install isn't disturbed.",
                },
                icon: "download",
                risk: Risk::Safe,
                strategy: Strategy::Permanent,
                strategy_locked: false,
                default_selected: true,
                needs_admin: true,
                min_age_hours: 48,
                age_adjustable: false,
            },
            targets: vec![tree(windows.join("SoftwareDistribution").join("Download"))],
        },
        Rule {
            meta: RuleMeta {
                id: "windows.delivery-optimization",
                group: Group::System,
                name: "Delivery Optimization cache",
                summary: "Update files Windows keeps to share with other PCs",
                detail: RuleDetail {
                    what: "Delivery Optimization stores recently downloaded Windows and Store updates so it can hand pieces to other PCs on your network.",
                    effect: "Windows re-downloads what it needs next time; nothing is lost.",
                    safety: "Needs administrator rights. Content that is in use is skipped.",
                },
                icon: "network",
                risk: Risk::Safe,
                strategy: Strategy::Permanent,
                strategy_locked: false,
                default_selected: true,
                needs_admin: true,
                min_age_hours: 24,
                age_adjustable: false,
            },
            targets: vec![
                tree(
                    windows
                        .join("ServiceProfiles")
                        .join("NetworkService")
                        .join("AppData")
                        .join("Local")
                        .join("Microsoft")
                        .join("Windows")
                        .join("DeliveryOptimization")
                        .join("Cache"),
                ),
                tree(
                    env.program_data
                        .join("Microsoft")
                        .join("Windows")
                        .join("DeliveryOptimization")
                        .join("Cache"),
                ),
            ],
        },
        Rule {
            meta: RuleMeta {
                id: "windows.thumbnails",
                group: Group::System,
                name: "Thumbnail & icon cache",
                summary: "Picture previews and icons Explorer has stored",
                detail: RuleDetail {
                    what: "Explorer stores small previews of your pictures, videos and documents, plus icons, so folders open faster.",
                    effect: "Explorer rebuilds them as you browse; folders with many pictures load a little slower once.",
                    safety: "Explorer usually keeps these open. Vexus skips whatever is in use and never closes Explorer for you.",
                },
                icon: "image",
                risk: Risk::Safe,
                strategy: Strategy::Permanent,
                strategy_locked: false,
                default_selected: true,
                needs_admin: false,
                min_age_hours: 0,
                age_adjustable: false,
            },
            targets: vec![Target::Tree(
                TreeTarget::new(local.join("Microsoft").join("Windows").join("Explorer"))
                    .flat()
                    .names(&["thumbcache_*.db", "iconcache_*.db"])
                    .keep_folders()
                    .probe_locks(),
            )],
        },
        Rule {
            meta: RuleMeta {
                id: "windows.prefetch",
                group: Group::System,
                name: "Prefetch data",
                summary: "Launch hints Windows keeps for faster app startup",
                detail: RuleDetail {
                    what: "Windows records which files each app loads at startup and preloads them next time.",
                    effect: "Windows relearns your habits within a few launches; the first start of each app may be slightly slower. Rarely worth cleaning.",
                    safety: "Needs administrator rights. Off by default.",
                },
                icon: "zap",
                risk: Risk::Review,
                strategy: Strategy::Permanent,
                strategy_locked: false,
                default_selected: false,
                needs_admin: true,
                min_age_hours: 24,
                age_adjustable: false,
            },
            targets: vec![Target::Tree(
                TreeTarget::new(windows.join("Prefetch")).flat().names(&["*.pf"]).keep_folders(),
            )],
        },
        Rule {
            meta: RuleMeta {
                id: "windows.error-reports",
                group: Group::System,
                name: "Crash dumps & error reports",
                summary: "Memory dumps and reports from app and system crashes",
                detail: RuleDetail {
                    what: "When an app or Windows crashes, it can save a memory dump and a report. Full memory dumps can be several gigabytes.",
                    effect: "You lose the ability to diagnose or share those specific crashes. Windows keeps working normally.",
                    safety: "Moved to Recovery first, so you can bring them back. Some locations need administrator rights.",
                },
                icon: "bug",
                risk: Risk::Review,
                strategy: Strategy::Undoable,
                strategy_locked: false,
                default_selected: false,
                needs_admin: false,
                min_age_hours: 24,
                age_adjustable: false,
            },
            targets: error_reports,
        },
        Rule {
            meta: RuleMeta {
                id: "windows.logs",
                group: Group::System,
                name: "Windows log files",
                summary: "Old servicing, setup and update logs",
                detail: RuleDetail {
                    what: "Windows writes logs while installing updates and features. Old ones are only useful to someone troubleshooting.",
                    effect: "Past installation details are no longer available for troubleshooting.",
                    safety: "Needs administrator rights. Only logs older than a week are included, and they go to Recovery first.",
                },
                icon: "scroll",
                risk: Risk::Safe,
                strategy: Strategy::Undoable,
                strategy_locked: false,
                default_selected: true,
                needs_admin: true,
                min_age_hours: 168,
                age_adjustable: false,
            },
            targets: vec![
                Target::Tree(
                    TreeTarget::new(windows.join("Logs").join("CBS"))
                        .flat()
                        .names(&["*.log", "*.cab"])
                        .keep_folders(),
                ),
                Target::Tree(
                    TreeTarget::new(windows.join("Logs").join("DISM"))
                        .flat()
                        .names(&["*.log"])
                        .keep_folders(),
                ),
                Target::Tree(
                    TreeTarget::new(windows.join("Logs").join("WindowsUpdate"))
                        .flat()
                        .names(&["*.etl"])
                        .keep_folders(),
                ),
                Target::Tree(
                    TreeTarget::new(windows.join("Debug")).flat().names(&["*.log"]).keep_folders(),
                ),
            ],
        },
        Rule {
            meta: RuleMeta {
                id: "privacy.recent-files",
                group: Group::Privacy,
                name: "Recent files & jump lists",
                summary: "Windows' memory of the files and apps you opened",
                detail: RuleDetail {
                    what: "Windows remembers recently opened files and shows them in Quick Access, Start and taskbar jump lists.",
                    effect: "Those lists start empty. Your files are untouched.",
                    safety: "Only the shortcut records are removed, never the files they point to. Moved to Recovery first.",
                },
                icon: "history",
                risk: Risk::Review,
                strategy: Strategy::Undoable,
                strategy_locked: false,
                default_selected: false,
                needs_admin: false,
                min_age_hours: 0,
                age_adjustable: false,
            },
            targets: vec![Target::Tree(
                TreeTarget::new(env.app_data.join("Microsoft").join("Windows").join("Recent"))
                    .keep_folders(),
            )],
        },
        Rule {
            meta: RuleMeta {
                id: "apps.shader-cache",
                group: Group::Apps,
                name: "Graphics shader caches",
                summary: "Compiled GPU shaders for games and apps",
                detail: RuleDetail {
                    what: "Graphics drivers keep compiled shader programs so games and apps load faster.",
                    effect: "Games may stutter briefly the first time you play afterwards while shaders rebuild.",
                    safety: "Regenerates automatically. Off by default because of that one-time slowdown.",
                },
                icon: "cpu",
                risk: Risk::Safe,
                strategy: Strategy::Permanent,
                strategy_locked: false,
                default_selected: false,
                needs_admin: false,
                min_age_hours: 0,
                age_adjustable: false,
            },
            targets: vec![
                tree(local.join("D3DSCache")),
                tree(local.join("NVIDIA").join("DXCache")),
                tree(local.join("NVIDIA").join("GLCache")),
                tree(local.join("AMD").join("DxCache")),
                tree(local.join("AMD").join("GLCache")),
                tree(local.join("AMD").join("VkCache")),
            ],
        },
        Rule {
            meta: RuleMeta {
                id: "windows.recycle-bin",
                group: Group::System,
                name: "Recycle Bin",
                summary: "Files you deleted but haven't permanently removed",
                detail: RuleDetail {
                    what: "Everything you've deleted and could still restore, across all your drives.",
                    effect: "You can no longer restore those files. This cannot be undone.",
                    safety: "Off by default. Vexus asks for explicit confirmation and cannot offer Undo for this one.",
                },
                icon: "trash",
                risk: Risk::Caution,
                strategy: Strategy::Permanent,
                strategy_locked: true,
                default_selected: false,
                needs_admin: false,
                min_age_hours: 0,
                age_adjustable: false,
            },
            targets: vec![Target::RecycleBin],
        },
    ];

    // Some targets are real, machine-wide state that a fake profile cannot stand in for. A
    // sandbox run must never be able to reach them.
    if env.sandboxed {
        rules.retain(|r| !r.targets.iter().any(|t| matches!(t, Target::RecycleBin)));
    }
    rules
}

fn tree(root: PathBuf) -> Target {
    Target::Tree(TreeTarget::new(root))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::safety::{PathGuard, is_within};

    fn rules() -> (Env, Vec<Rule>) {
        let mut env = Env::sandbox(Path::new(r"C:\Sandbox"));
        // Evaluate the full rule set; the sandbox filter has its own test below.
        env.sandboxed = false;
        let rules = builtin(&env);
        (env, rules)
    }

    #[test]
    fn sandboxes_can_never_reach_real_system_state() {
        let sandboxed = Env::sandbox(Path::new(r"C:\Sandbox"));
        let rules = builtin(&sandboxed);
        assert!(!rules.is_empty());
        for r in &rules {
            assert!(
                !r.targets.iter().any(|t| matches!(t, Target::RecycleBin)),
                "{} would touch the real Recycle Bin from a sandbox",
                r.meta.id
            );
        }
        assert!(rules.iter().all(|r| r.meta.id != "windows.recycle-bin"));
        let mut real = sandboxed;
        real.sandboxed = false;
        assert!(builtin(&real).iter().any(|r| r.meta.id == "windows.recycle-bin"));
    }

    #[test]
    fn ids_are_unique_and_well_formed() {
        let (_, rules) = rules();
        let mut seen = HashSet::new();
        for r in &rules {
            assert!(seen.insert(r.meta.id), "duplicate id {}", r.meta.id);
            assert!(r.meta.id.contains('.'), "ids are group.name: {}", r.meta.id);
            assert!(!r.targets.is_empty(), "{} has no targets", r.meta.id);
        }
    }

    #[test]
    fn every_rule_explains_itself() {
        let (_, rules) = rules();
        for r in &rules {
            let m = &r.meta;
            for (what, text) in [
                ("name", m.name),
                ("summary", m.summary),
                ("what", m.detail.what),
                ("effect", m.detail.effect),
                ("safety", m.detail.safety),
            ] {
                assert!(text.trim().len() > 10, "{} has a missing or stub {what}", m.id);
            }
            assert!(!m.icon.trim().is_empty(), "{} has no icon", m.id);
            assert!(!m.summary.ends_with('.'), "{}: summaries are fragments", m.id);
        }
    }

    #[test]
    fn risky_categories_are_never_preselected() {
        let (_, rules) = rules();
        for r in &rules {
            if r.meta.risk != Risk::Safe {
                assert!(!r.meta.default_selected, "{} is risky but preselected", r.meta.id);
            }
        }
    }

    #[test]
    fn irreversible_categories_are_flagged_cautious() {
        let (_, rules) = rules();
        let bin = rules.iter().find(|r| r.meta.id == "windows.recycle-bin").unwrap();
        assert_eq!(bin.meta.risk, Risk::Caution);
        assert_eq!(bin.meta.strategy, Strategy::Permanent);
        assert!(!bin.meta.default_selected);
    }

    #[test]
    fn non_regenerable_data_is_undoable() {
        let (_, rules) = rules();
        for id in ["windows.error-reports", "windows.logs", "privacy.recent-files"] {
            let r = rules.iter().find(|r| r.meta.id == id).unwrap();
            assert_eq!(r.meta.strategy, Strategy::Undoable, "{id}");
        }
    }

    #[test]
    fn every_declared_root_passes_the_guard_and_none_overlap() {
        let (env, rules) = rules();
        let guard = PathGuard::new(&env, &[]);
        let mut roots: Vec<PathBuf> = Vec::new();
        for r in &rules {
            for t in &r.targets {
                match t {
                    Target::Tree(t) => {
                        guard.validate_root(&t.root).unwrap_or_else(|e| {
                            panic!("{}: {:?} rejected: {e}", r.meta.id, t.root)
                        });
                        roots.push(t.root.clone());
                    }
                    Target::File(f) => {
                        let mut g = guard.clone();
                        g.permit_file(f)
                            .unwrap_or_else(|e| panic!("{}: {f:?} rejected: {e}", r.meta.id));
                    }
                    Target::RecycleBin => {}
                }
            }
        }
        for (i, a) in roots.iter().enumerate() {
            for b in &roots[i + 1..] {
                assert!(!is_within(a, b) && !is_within(b, a), "overlapping roots {a:?} / {b:?}");
            }
        }
    }

    #[test]
    fn no_rule_reaches_into_protected_trees() {
        let (env, rules) = rules();
        let forbidden = [
            env.system_root.join("System32"),
            env.system_root.join("WinSxS"),
            env.system_root.join("Installer"),
            env.documents.clone(),
            env.desktop.clone(),
            env.downloads.clone(),
            env.program_files.clone(),
        ];
        for r in &rules {
            for t in &r.targets {
                let path = match t {
                    Target::Tree(t) => &t.root,
                    Target::File(f) => f,
                    Target::RecycleBin => continue,
                };
                for f in &forbidden {
                    assert!(!is_within(path, f), "{} targets {path:?} inside {f:?}", r.meta.id);
                }
            }
        }
    }

    #[test]
    fn extra_temp_root_is_added_only_when_it_differs() {
        let mut env = Env::sandbox(Path::new(r"C:\Sandbox"));
        let same = builtin(&env);
        assert_eq!(same[0].targets.len(), 1);
        env.temp = PathBuf::from(r"D:\Temp");
        let different = builtin(&env);
        assert_eq!(different[0].targets.len(), 2);
    }
}
