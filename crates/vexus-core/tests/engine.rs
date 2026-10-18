//! End-to-end tests of the engine against a fake Windows profile: scan → preview → clean →
//! undo, plus the attacks a cleaner must survive (stale plans, locked files, junction swaps,
//! hostile `%TEMP%`, an unwritable Recovery folder).
//!
//! Nothing here touches the real machine: every path lives under a temporary sandbox root, and
//! the Recycle Bin rule (which is not path-based) is excluded from the rule set.

use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use vexus_core::Engine;
use vexus_core::clean::{CleanOptions, CleanReport, SkipReason};
use vexus_core::env::Env;
use vexus_core::fsops::canonical_dir;
use vexus_core::model::{CategoryStatus, DeletionPolicy, ItemKind, Strategy, item_id};
use vexus_core::paths::{DataDirs, DataMode};
use vexus_core::progress::{CancelToken, CleanEvent, NullSink, ScanEvent};
use vexus_core::rules;
use vexus_core::scan::{ItemQuery, ItemSort, Scan, ScanRequest, Selection};

const DAY: u64 = 86_400;
const TEMP_RULE: &str = "windows.user-temp";
const LOGS_RULE: &str = "windows.logs";
const WU_RULE: &str = "windows.update-cache";

struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
    env: Env,
    data: DataDirs,
    engine: Engine,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical_dir(dir.path()).unwrap();
        let env = Env::sandbox(&root);
        env.create_sandbox_skeleton().unwrap();
        Self::build(dir, root, env)
    }

    fn with_env(mutate: impl FnOnce(&mut Env, &Path)) -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical_dir(dir.path()).unwrap();
        let mut env = Env::sandbox(&root);
        env.create_sandbox_skeleton().unwrap();
        mutate(&mut env, &root);
        Self::build(dir, root, env)
    }

    fn build(dir: tempfile::TempDir, root: PathBuf, env: Env) -> Sandbox {
        let data = DataDirs::resolve(&DataMode::Custom(root.join(".vexus")), &env);
        data.ensure().unwrap();
        // Sandboxed environments drop the (non-path-based) Recycle Bin rule on their own.
        let engine = Engine::with_rules(env.clone(), rules::builtin(&env), &data, false);
        Sandbox { _dir: dir, root, env, data, engine }
    }

    fn temp(&self) -> PathBuf {
        self.env.temp.clone()
    }

    fn scan(&self, ids: &[&str]) -> Scan {
        let req = ScanRequest {
            categories: Some(ids.iter().map(|s| s.to_string()).collect()),
            ..ScanRequest::default()
        };
        self.engine.scan(&req, &CancelToken::new(), &NullSink).unwrap()
    }

    fn select(&self, ids: &[&str]) -> Selection {
        Selection {
            categories: ids.iter().map(|s| s.to_string()).collect(),
            ..Selection::default()
        }
    }

    fn clean(&self, scan: &Scan, sel: &Selection, policy: DeletionPolicy) -> CleanReport {
        let opts = CleanOptions { policy, undo_days: 7, threads: Some(4) };
        self.engine.clean(scan, sel, &opts, &CancelToken::new(), &NullSink).unwrap()
    }
}

/// Creates a file that looks `age_days` old: both its modification and creation time say so.
fn put(path: &Path, bytes: usize, age_days: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
    let t = SystemTime::now() - Duration::from_secs(age_days * DAY);
    set_times(path, t, t);
}

fn set_times(path: &Path, modified: SystemTime, created: SystemTime) {
    use std::os::windows::fs::FileTimesExt;
    let times = fs::FileTimes::new().set_modified(modified).set_created(created);
    File::options().write(true).open(path).unwrap().set_times(times).unwrap();
}

fn junction(link: &Path, target: &Path) -> bool {
    Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn category<'a>(scan: &'a Scan, id: &str) -> &'a vexus_core::scan::CategoryResult {
    &scan.categories[scan.category_index(id).unwrap()]
}

fn skipped(report: &CleanReport, reason: SkipReason) -> u64 {
    report.skipped.get(&reason).map_or(0, |s| s.files)
}

// ─── scanning ────────────────────────────────────────────────────────────────

#[test]
fn scan_lists_old_files_and_keeps_recent_ones() {
    let sb = Sandbox::new();
    put(&sb.temp().join("old.tmp"), 1000, 10);
    put(&sb.temp().join("new.tmp"), 500, 0);
    put(&sb.temp().join("cache").join("a.bin"), 200, 20);
    put(&sb.temp().join("cache").join("b.bin"), 300, 20);

    let scan = sb.scan(&[TEMP_RULE]);
    let cat = category(&scan, TEMP_RULE);
    assert_eq!(cat.status, CategoryStatus::Found);
    assert_eq!(cat.stats.bytes, 1500);
    assert_eq!(cat.stats.files, 3);
    assert_eq!((cat.stats.kept_files, cat.stats.kept_bytes), (1, 500));
    assert_eq!(cat.items.len(), 2, "old.tmp + the collapsed cache folder");
    assert_eq!(cat.items[0].size, 1000, "largest first");
    assert_eq!(cat.items[1].kind, ItemKind::Dir);
    assert!(sb.temp().join("old.tmp").exists(), "a scan never changes anything");
}

#[test]
fn scan_reports_empty_categories_and_missing_locations_quietly() {
    let sb = Sandbox::new();
    let scan = sb.scan(&[TEMP_RULE, WU_RULE]);
    assert_eq!(category(&scan, TEMP_RULE).status, CategoryStatus::Empty);
    assert_eq!(
        category(&scan, WU_RULE).status,
        CategoryStatus::Empty,
        "location doesn't exist here"
    );
    assert!(scan.notes.is_empty());
}

#[test]
fn a_sandbox_scan_of_everything_never_reaches_the_real_recycle_bin() {
    let sb = Sandbox::new();
    let req = ScanRequest::default(); // all categories
    let scan = sb.engine.scan(&req, &CancelToken::new(), &NullSink).unwrap();
    assert!(scan.rules.iter().all(|r| r.id != "windows.recycle-bin"));
    let asked =
        ScanRequest { categories: Some(vec!["windows.recycle-bin".into()]), ..Default::default() };
    assert_eq!(
        sb.engine.scan(&asked, &CancelToken::new(), &NullSink).unwrap_err().code(),
        "unknown-category",
        "a sandbox must not even accept the request"
    );
}

#[test]
fn unknown_categories_are_rejected() {
    let sb = Sandbox::new();
    let req =
        ScanRequest { categories: Some(vec!["nope.nothing".into()]), ..ScanRequest::default() };
    let err = sb.engine.scan(&req, &CancelToken::new(), &NullSink).unwrap_err();
    assert_eq!(err.code(), "unknown-category");
}

#[test]
fn scan_streams_ordered_events() {
    let sb = Sandbox::new();
    for i in 0..300 {
        put(&sb.temp().join(format!("d{}", i % 20)).join(format!("f{i}.tmp")), 100, 9);
    }
    let events = Mutex::new(Vec::<ScanEvent>::new());
    let sink = |e: ScanEvent| events.lock().unwrap().push(e);
    let req = ScanRequest {
        categories: Some(vec![TEMP_RULE.into(), LOGS_RULE.into()]),
        ..Default::default()
    };
    let scan = sb.engine.scan(&req, &CancelToken::new(), &sink).unwrap();
    let events = events.into_inner().unwrap();

    assert!(
        matches!(events.first(), Some(ScanEvent::Started { categories, .. }) if categories.len() == 2)
    );
    let Some(ScanEvent::Finished { summary }) = events.last() else {
        panic!("last event must be Finished")
    };
    assert_eq!(summary.scan_id, scan.id);
    assert_eq!(summary.total_bytes, 30_000);
    assert_eq!(events.iter().filter(|e| matches!(e, ScanEvent::CategoryDone { .. })).count(), 2);
    // JSON shape the UI relies on.
    let json = serde_json::to_value(events.last().unwrap()).unwrap();
    assert_eq!(json["event"], "finished");
    assert_eq!(json["summary"]["totalBytes"], 30_000);
    let started = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(started["event"], "started");
}

#[test]
fn cancelled_scans_are_flagged() {
    let sb = Sandbox::new();
    put(&sb.temp().join("a.tmp"), 10, 9);
    let cancel = CancelToken::new();
    cancel.cancel();
    let req = ScanRequest { categories: Some(vec![TEMP_RULE.into()]), ..Default::default() };
    let scan = sb.engine.scan(&req, &cancel, &NullSink).unwrap();
    assert!(scan.cancelled);
    assert!(category(&scan, TEMP_RULE).items.is_empty());
}

#[test]
fn age_override_changes_what_is_eligible() {
    let sb = Sandbox::new();
    put(&sb.temp().join("two-days.tmp"), 10, 2);
    let default = sb.scan(&[TEMP_RULE]);
    assert_eq!(category(&default, TEMP_RULE).items.len(), 0, "default gate is 3 days");
    let req = ScanRequest {
        categories: Some(vec![TEMP_RULE.into()]),
        age_override_hours: Some(24),
        ..Default::default()
    };
    let scan = sb.engine.scan(&req, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(category(&scan, TEMP_RULE).items.len(), 1);
}

#[test]
fn items_can_be_paged_searched_and_sorted() {
    let sb = Sandbox::new();
    for (name, size, age) in [("alpha.log", 300, 30), ("beta.tmp", 100, 10), ("gamma.log", 200, 20)]
    {
        put(&sb.temp().join(name), size, age);
    }
    let scan = sb.scan(&[TEMP_RULE]);
    let sel = sb.select(&[TEMP_RULE]);
    let page = scan.items(TEMP_RULE, &ItemQuery::default(), &sel).unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.bytes, 600);
    assert_eq!(
        page.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["alpha.log", "gamma.log", "beta.tmp"]
    );

    let search = ItemQuery { search: Some("LOG".into()), ..Default::default() };
    let page = scan.items(TEMP_RULE, &search, &sel).unwrap();
    assert_eq!((page.total, page.bytes), (2, 500));

    let by_name = ItemQuery { sort: ItemSort::NameAsc, limit: 2, ..Default::default() };
    let page = scan.items(TEMP_RULE, &by_name, &sel).unwrap();
    assert_eq!(
        page.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["alpha.log", "beta.tmp"]
    );

    let oldest = ItemQuery { sort: ItemSort::Oldest, limit: 1, ..Default::default() };
    assert_eq!(scan.items(TEMP_RULE, &oldest, &sel).unwrap().items[0].name, "alpha.log");
    assert!(scan.items("nope", &ItemQuery::default(), &sel).is_err());
}

// ─── planning ────────────────────────────────────────────────────────────────

#[test]
fn plan_reflects_exclusions_and_strategy() {
    let sb = Sandbox::new();
    put(&sb.temp().join("a.tmp"), 1000, 9);
    put(&sb.temp().join("b.tmp"), 400, 9);
    put(&sb.env.system_root.join("Logs").join("CBS").join("old.log"), 250, 30);
    let scan = sb.scan(&[TEMP_RULE, LOGS_RULE]);
    let mut sel = sb.select(&[TEMP_RULE, LOGS_RULE]);

    let plan = scan.plan(&sel, DeletionPolicy::Smart);
    assert_eq!((plan.items, plan.bytes), (3, 1650));
    assert_eq!(plan.permanent_bytes, 1400, "temp is regenerable → deleted outright");
    assert_eq!(plan.undo_bytes, 250, "logs are not → recoverable");
    assert!(plan.irreversible.is_empty());

    // Exclude the big temp file: it leaves the plan entirely.
    let c = scan.category_index(TEMP_RULE).unwrap();
    let big = scan.categories[c].items.iter().position(|i| i.size == 1000).unwrap();
    sel.excluded.insert(item_id(c, big));
    let plan = scan.plan(&sel, DeletionPolicy::Smart);
    assert_eq!((plan.items, plan.bytes), (2, 650));
    let temp_plan = plan.categories.iter().find(|p| p.id == TEMP_RULE).unwrap();
    assert_eq!(temp_plan.excluded_items, 1);

    let all_undo = scan.plan(&sel, DeletionPolicy::AlwaysUndo);
    assert_eq!((all_undo.undo_bytes, all_undo.permanent_bytes), (650, 0));
    let all_perm = scan.plan(&sel, DeletionPolicy::AlwaysPermanent);
    assert_eq!((all_perm.undo_bytes, all_perm.permanent_bytes), (0, 650));

    // Deselecting a category removes it.
    sel.categories.remove(LOGS_RULE);
    assert_eq!(scan.plan(&sel, DeletionPolicy::Smart).bytes, 400);
}

#[test]
fn default_selection_only_includes_preselected_non_empty_categories() {
    let sb = Sandbox::new();
    put(&sb.temp().join("a.tmp"), 10, 9);
    put(&sb.env.system_root.join("Prefetch").join("APP.EXE-1.pf"), 10, 9);
    let scan = sb.scan(&[TEMP_RULE, "windows.prefetch", LOGS_RULE]);
    let sel = scan.default_selection();
    assert!(sel.categories.contains(TEMP_RULE));
    assert!(!sel.categories.contains("windows.prefetch"), "found but not default-on");
    assert!(!sel.categories.contains(LOGS_RULE), "default-on but empty");
}

// ─── cleaning ────────────────────────────────────────────────────────────────

#[test]
fn clean_removes_exactly_the_selection() {
    let sb = Sandbox::new();
    put(&sb.temp().join("remove-me.tmp"), 1000, 9);
    put(&sb.temp().join("keep-me.tmp"), 700, 9);
    put(&sb.temp().join("too-new.tmp"), 50, 0);
    put(&sb.temp().join("cache").join("x.bin"), 400, 9);
    put(&sb.temp().join("cache").join("sub").join("y.bin"), 100, 9);

    let scan = sb.scan(&[TEMP_RULE]);
    let mut sel = sb.select(&[TEMP_RULE]);
    let c = scan.category_index(TEMP_RULE).unwrap();
    let keep =
        scan.categories[c].items.iter().position(|i| i.path.ends_with("keep-me.tmp")).unwrap();
    sel.excluded.insert(item_id(c, keep));

    let report = sb.clean(&scan, &sel, DeletionPolicy::Smart);
    assert_eq!((report.files, report.bytes), (3, 1500));
    assert_eq!(report.permanent_bytes, 1500);
    assert_eq!(report.undo_bytes, 0);
    assert!(report.batch.is_none(), "nothing undoable → no Recovery batch");
    assert!(!report.cancelled);

    assert!(!sb.temp().join("remove-me.tmp").exists());
    assert!(!sb.temp().join("cache").exists(), "a fully-cleaned folder is removed too");
    assert!(sb.temp().join("keep-me.tmp").exists(), "excluded items are never touched");
    assert!(sb.temp().join("too-new.tmp").exists(), "recent files are never touched");
    assert!(sb.temp().exists(), "the root itself is never removed");
}

#[test]
fn unselected_categories_are_untouched() {
    let sb = Sandbox::new();
    put(&sb.temp().join("a.tmp"), 10, 9);
    put(&sb.env.system_root.join("Logs").join("CBS").join("old.log"), 10, 30);
    let scan = sb.scan(&[TEMP_RULE, LOGS_RULE]);
    sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert!(!sb.temp().join("a.tmp").exists());
    assert!(sb.env.system_root.join("Logs").join("CBS").join("old.log").exists());
}

#[test]
fn undoable_clean_moves_to_recovery_and_restore_brings_everything_back() {
    let sb = Sandbox::new();
    let log = sb.env.system_root.join("Logs").join("CBS").join("old.log");
    put(&log, 2048, 30);
    put(&sb.env.system_root.join("Logs").join("CBS").join("fresh.log"), 10, 0);
    let before = fs::metadata(&log).unwrap().modified().unwrap();

    let scan = sb.scan(&[LOGS_RULE]);
    let report = sb.clean(&scan, &sb.select(&[LOGS_RULE]), DeletionPolicy::Smart);
    assert_eq!(
        (report.files, report.bytes, report.undo_bytes, report.permanent_bytes),
        (1, 2048, 2048, 0)
    );
    let batch = report.batch.clone().expect("undoable removals create a Recovery batch");
    assert!(!log.exists());
    assert!(sb.env.system_root.join("Logs").join("CBS").join("fresh.log").exists());

    let listed = sb.engine.recovery().list();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].files, listed[0].bytes), (1, 2048));
    assert!(listed[0].expires_at > listed[0].created_at + 6 * DAY as i64);

    let restored = sb.engine.restore(&batch.id, &CancelToken::new()).unwrap();
    assert_eq!(
        (restored.restored_files, restored.conflicts, restored.refused, restored.failed),
        (1, 0, 0, 0)
    );
    assert!(restored.batch_removed);
    assert_eq!(fs::read(&log).unwrap().len(), 2048);
    assert_eq!(
        fs::metadata(&log).unwrap().modified().unwrap(),
        before,
        "timestamps survive a round trip"
    );
    assert!(sb.engine.recovery().list().is_empty());
}

#[test]
fn a_recovery_batch_names_only_what_actually_went_to_recovery() {
    let sb = Sandbox::new();
    put(&sb.temp().join("junk.tmp"), 100, 9); // permanent
    put(&sb.env.system_root.join("Logs").join("CBS").join("old.log"), 100, 30); // undoable
    let scan = sb.scan(&[TEMP_RULE, LOGS_RULE]);
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE, LOGS_RULE]), DeletionPolicy::Smart);
    let batch = report.batch.expect("the logs are undoable");
    assert_eq!(
        batch.categories,
        [LOGS_RULE],
        "the temp files were deleted outright, not recovered"
    );
    assert_eq!(batch.label, "Windows log files");
    let listed = sb.engine.recovery().list();
    assert_eq!(listed[0].categories, [LOGS_RULE]);
}

#[test]
fn always_undo_policy_makes_even_temp_files_recoverable() {
    let sb = Sandbox::new();
    put(&sb.temp().join("cache").join("a.bin"), 300, 9);
    put(&sb.temp().join("b.tmp"), 200, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysUndo);
    assert_eq!((report.undo_bytes, report.permanent_bytes), (500, 0));
    assert_eq!(report.categories[0].strategy, Strategy::Undoable);
    let batch = report.batch.unwrap();
    assert_eq!(batch.files, 2);

    let restored = sb.engine.restore(&batch.id, &CancelToken::new()).unwrap();
    assert_eq!(restored.restored_files, 2);
    assert_eq!(fs::read(sb.temp().join("cache").join("a.bin")).unwrap().len(), 300);
    assert_eq!(fs::read(sb.temp().join("b.tmp")).unwrap().len(), 200);
}

#[test]
fn files_changed_after_the_scan_are_not_deleted() {
    let sb = Sandbox::new();
    let file = sb.temp().join("report.tmp");
    put(&file, 1000, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    // The user (or an app) appends to it after reviewing the list.
    OpenOptions::new().write(true).open(&file).unwrap().set_len(1500).unwrap();

    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert_eq!(report.files, 0);
    assert!(file.exists());
    assert_eq!(skipped(&report, SkipReason::Changed) + skipped(&report, SkipReason::TooRecent), 1);
}

#[test]
fn files_touched_after_the_scan_are_not_deleted() {
    let sb = Sandbox::new();
    let file = sb.temp().join("busy.tmp");
    put(&file, 100, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    File::options().write(true).open(&file).unwrap().set_modified(SystemTime::now()).unwrap();
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert_eq!(report.files, 0);
    assert_eq!(skipped(&report, SkipReason::TooRecent), 1);
    assert!(file.exists());
}

#[test]
fn files_inside_a_folder_item_are_rechecked_individually() {
    let sb = Sandbox::new();
    put(&sb.temp().join("proj").join("old.bin"), 100, 9);
    put(&sb.temp().join("proj").join("later.bin"), 100, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    assert_eq!(category(&scan, TEMP_RULE).items.len(), 1, "collapsed into one folder item");
    // Between scan and clean an app writes into the folder.
    put(&sb.temp().join("proj").join("brand-new.bin"), 100, 0);
    File::options()
        .write(true)
        .open(sb.temp().join("proj").join("later.bin"))
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();

    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert_eq!(report.files, 1, "only old.bin is still eligible");
    assert!(!sb.temp().join("proj").join("old.bin").exists());
    assert!(sb.temp().join("proj").join("brand-new.bin").exists());
    assert!(sb.temp().join("proj").join("later.bin").exists());
    assert!(sb.temp().join("proj").exists(), "a folder that still has files stays");
}

#[test]
fn locked_files_are_skipped_never_forced() {
    let sb = Sandbox::new();
    let locked = sb.temp().join("locked.db");
    put(&locked, 100, 9);
    put(&sb.temp().join("free.tmp"), 100, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    let _held = OpenOptions::new().read(true).share_mode(1).open(&locked).unwrap();

    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert_eq!(report.files, 1);
    assert_eq!(skipped(&report, SkipReason::InUse), 1);
    assert!(locked.exists());
    assert!(!sb.temp().join("free.tmp").exists());
}

#[test]
fn read_only_files_are_removed() {
    let sb = Sandbox::new();
    let f = sb.temp().join("ro.tmp");
    put(&f, 10, 9);
    let mut p = fs::metadata(&f).unwrap().permissions();
    p.set_readonly(true);
    fs::set_permissions(&f, p).unwrap();
    let scan = sb.scan(&[TEMP_RULE]);
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert_eq!(report.files, 1);
    assert!(!f.exists());
}

#[test]
fn protected_file_names_survive_even_in_temp() {
    let sb = Sandbox::new();
    put(&sb.temp().join("desktop.ini"), 10, 99);
    put(&sb.temp().join("pagefile.sys"), 10, 99);
    put(&sb.temp().join("junk.tmp"), 10, 99);
    let scan = sb.scan(&[TEMP_RULE]);
    sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert!(sb.temp().join("desktop.ini").exists());
    assert!(sb.temp().join("pagefile.sys").exists());
    assert!(!sb.temp().join("junk.tmp").exists());
}

#[test]
fn a_junction_swapped_in_after_the_scan_cannot_redirect_the_clean() {
    let sb = Sandbox::new();
    let victim_dir = sb.root.join("Users").join("vexus").join("Documents").join("precious");
    put(&victim_dir.join("thesis.docx"), 5000, 400);
    put(&victim_dir.join("sub").join("notes.txt"), 100, 400);

    put(&sb.temp().join("cache").join("a.bin"), 100, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    assert_eq!(category(&scan, TEMP_RULE).items[0].kind, ItemKind::Dir);

    // Attacker/accident: replace the scanned folder with a junction to the user's documents.
    fs::remove_dir_all(sb.temp().join("cache")).unwrap();
    if !junction(&sb.temp().join("cache"), &victim_dir) {
        eprintln!("skipping: cannot create junctions here");
        return;
    }

    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysPermanent);
    assert_eq!(report.files, 0, "nothing may be removed through the junction");
    assert!(victim_dir.join("thesis.docx").exists(), "user documents must survive");
    assert!(victim_dir.join("sub").join("notes.txt").exists());
    assert_eq!(
        skipped(&report, SkipReason::Link),
        1,
        "the swapped folder is refused as a link: {report:?}"
    );
    assert_eq!(
        skipped(&report, SkipReason::Outside),
        0,
        "the documents tree must not even be walked, let alone reported on: {report:?}"
    );
    // And with the Undoable strategy nothing is *moved* out of the documents either.
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysUndo);
    assert_eq!(report.files, 0);
    assert!(victim_dir.join("thesis.docx").exists());
}

#[test]
fn junctions_inside_temp_are_never_followed_during_scan_or_clean() {
    let sb = Sandbox::new();
    let outside = sb.root.join("Users").join("vexus").join("Documents").join("keep");
    put(&outside.join("important.docx"), 999, 500);
    put(&sb.temp().join("real.tmp"), 10, 9);
    if !junction(&sb.temp().join("shortcut"), &outside) {
        eprintln!("skipping: cannot create junctions here");
        return;
    }
    let scan = sb.scan(&[TEMP_RULE]);
    let cat = category(&scan, TEMP_RULE);
    assert_eq!(cat.stats.bytes, 10, "the junction's target must not be counted");
    assert_eq!(cat.stats.links, 1);
    sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert!(outside.join("important.docx").exists());
    assert!(!sb.temp().join("real.tmp").exists());
}

#[test]
fn a_hostile_temp_variable_never_reaches_user_files() {
    for case in ["documents", "profile", "drive"] {
        let sb = Sandbox::with_env(|env, root| {
            env.temp = match case {
                "documents" => env.documents.clone(),
                "profile" => env.user_profile.clone(),
                _ => root.ancestors().last().unwrap().to_path_buf(),
            };
        });
        let precious = sb.env.documents.join("thesis.docx");
        put(&precious, 4096, 900);
        put(&sb.env.user_profile.join("old-notes.txt"), 10, 900);
        let scan = sb.scan(&[TEMP_RULE]);
        let cat = category(&scan, TEMP_RULE);
        assert!(cat.items.is_empty(), "{case}: nothing may be offered for cleaning");
        assert_ne!(cat.status, CategoryStatus::Found, "{case}");
        sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysPermanent);
        assert!(precious.exists(), "{case}: documents must survive");
        assert!(sb.env.user_profile.join("old-notes.txt").exists(), "{case}");
    }
}

#[test]
fn clean_aborts_before_touching_anything_if_recovery_cannot_be_prepared() {
    let sb = Sandbox::new();
    let log = sb.env.system_root.join("Logs").join("CBS").join("old.log");
    put(&log, 100, 30);
    let scan = sb.scan(&[LOGS_RULE]);

    // Put a *file* where the Recovery folder must be created.
    fs::remove_dir_all(&sb.data.recovery).unwrap();
    fs::write(&sb.data.recovery, b"not a directory").unwrap();

    let opts = CleanOptions::default();
    let err = sb
        .engine
        .clean(&scan, &sb.select(&[LOGS_RULE]), &opts, &CancelToken::new(), &NullSink)
        .unwrap_err();
    assert!(err.to_string().contains("nothing was removed"), "{err}");
    assert!(log.exists(), "undoable data must never be deleted when undo is impossible");
}

#[test]
fn cancelling_a_clean_stops_it_and_seals_what_was_done() {
    let sb = Sandbox::new();
    for i in 0..50 {
        put(&sb.temp().join(format!("f{i}.tmp")), 10, 9);
    }
    let scan = sb.scan(&[TEMP_RULE]);
    let cancel = CancelToken::new();
    cancel.cancel();
    let opts = CleanOptions::default();
    let report =
        sb.engine.clean(&scan, &sb.select(&[TEMP_RULE]), &opts, &cancel, &NullSink).unwrap();
    assert!(report.cancelled);
    assert_eq!(report.files, 0);
    assert_eq!(fs::read_dir(sb.temp()).unwrap().count(), 50);
}

#[test]
fn clean_events_have_the_shape_the_ui_expects() {
    let sb = Sandbox::new();
    for i in 0..40 {
        put(&sb.temp().join(format!("f{i}.tmp")), 100, 9);
    }
    let scan = sb.scan(&[TEMP_RULE]);
    let events = Mutex::new(Vec::<CleanEvent>::new());
    let sink = |e: CleanEvent| events.lock().unwrap().push(e);
    let opts = CleanOptions::default();
    sb.engine.clean(&scan, &sb.select(&[TEMP_RULE]), &opts, &CancelToken::new(), &sink).unwrap();
    let events = events.into_inner().unwrap();
    assert!(matches!(&events[0], CleanEvent::Started { files_total: 40, bytes_total: 4000, .. }));
    let last = serde_json::to_value(events.last().unwrap()).unwrap();
    assert_eq!(last["event"], "finished");
    assert_eq!(last["report"]["files"], 40);
    assert_eq!(last["report"]["permanentBytes"], 4000);
    assert!(last["report"]["skipped"].is_object());
}

#[test]
fn single_file_targets_work_and_are_undoable() {
    let sb = Sandbox::new();
    let dump = sb.env.system_root.join("MEMORY.DMP");
    put(&dump, 8192, 30);
    put(&sb.env.system_root.join("notepad.exe"), 100, 900);
    let scan = sb.scan(&["windows.error-reports"]);
    let cat = category(&scan, "windows.error-reports");
    assert_eq!(cat.items.len(), 1);
    let report = sb.clean(&scan, &sb.select(&["windows.error-reports"]), DeletionPolicy::Smart);
    assert_eq!((report.files, report.undo_bytes), (1, 8192));
    assert!(!dump.exists());
    assert!(
        sb.env.system_root.join("notepad.exe").exists(),
        "siblings of a single-file target are safe"
    );
    sb.engine.restore(&report.batch.unwrap().id, &CancelToken::new()).unwrap();
    assert!(dump.exists());
}

#[test]
fn system_files_outside_every_rule_are_unreachable() {
    let sb = Sandbox::new();
    // Plant old "precious" files everywhere a bug could reach.
    let planted = [
        sb.env.system_root.join("System32").join("kernel32.dll"),
        sb.env.system_root.join("Installer").join("setup.msi"),
        sb.env.program_files.join("App").join("app.exe"),
        sb.env.downloads.join("installer.exe"),
        sb.env.app_data.join("settings.ini"),
        sb.env.local_app_data.join("Programs").join("tool.exe"),
        sb.env.user_profile.join("NTUSER.DAT"),
    ];
    for p in &planted {
        put(p, 64, 999);
    }
    put(&sb.temp().join("junk.tmp"), 10, 99);
    let all: Vec<&str> = vec![
        TEMP_RULE,
        LOGS_RULE,
        WU_RULE,
        "windows.system-temp",
        "windows.prefetch",
        "windows.error-reports",
        "windows.thumbnails",
        "privacy.recent-files",
        "apps.shader-cache",
        "windows.delivery-optimization",
    ];
    let scan = sb.scan(&all);
    let sel =
        Selection { categories: all.iter().map(|s| s.to_string()).collect(), ..Default::default() };
    sb.clean(&scan, &sel, DeletionPolicy::AlwaysPermanent);
    for p in &planted {
        assert!(p.exists(), "{} must never be touched", p.display());
    }
    assert!(!sb.temp().join("junk.tmp").exists());
}

#[test]
fn thousands_of_files_scan_and_clean_correctly() {
    let sb = Sandbox::new();
    let mut expected = 0u64;
    for d in 0..40 {
        for f in 0..150 {
            put(&sb.temp().join(format!("dir{d}")).join(format!("f{f}.bin")), 64, 9);
            expected += 64;
        }
    }
    let scan = sb.scan(&[TEMP_RULE]);
    let cat = category(&scan, TEMP_RULE);
    assert_eq!(cat.stats.files, 6000);
    assert_eq!(cat.stats.bytes, expected);
    assert_eq!(cat.items.len(), 40, "each directory collapses into a single item");
    assert!(scan.entries >= 6040);
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert_eq!((report.files, report.bytes), (6000, expected));
    assert_eq!(report.dirs, 40);
    assert_eq!(fs::read_dir(sb.temp()).unwrap().count(), 0);
}

// ─── hostile folder shapes ───────────────────────────────────────────────────

/// Creates `root/d/d/d/…` (`depth` levels) and returns the deepest folder.
fn make_deep(root: &Path, depth: usize) -> PathBuf {
    let mut p = root.to_path_buf();
    for _ in 0..depth {
        p.push("d");
    }
    fs::create_dir_all(&p).unwrap();
    p
}

/// Removes a deep chain bottom-up without recursion (std's `remove_dir_all` is recursive).
fn remove_deep(mut deepest: PathBuf, root: &Path) {
    let _ = fs::remove_dir_all(&deepest); // only files live in the deepest folder
    while deepest != root && deepest.starts_with(root) {
        let _ = fs::remove_dir(&deepest);
        deepest.pop();
    }
}

#[test]
fn a_pathologically_deep_folder_tree_cannot_crash_the_scan() {
    // NTFS allows ~16,000 levels. A recursive walker without a depth limit overflows its stack
    // on a tree like this — which is exactly what happened to the first version of Vexus when
    // such a tree turned up in a real %TEMP%.
    let sb = Sandbox::new();
    put(&sb.temp().join("ordinary.tmp"), 100, 9);
    let bottom = make_deep(&sb.temp().join("hostile"), 2_000);
    put(&bottom.join("buried.tmp"), 50, 99);
    put(&sb.temp().join("hostile").join("shallow.tmp"), 10, 99);

    let scan = sb.scan(&[TEMP_RULE]);
    let cat = category(&scan, TEMP_RULE);
    assert_eq!(cat.status, CategoryStatus::Found);
    assert!(
        cat.stats.unreadable >= 1,
        "the too-deep subtree must be reported, not silently ignored"
    );
    let names: Vec<String> = cat
        .items
        .iter()
        .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.contains(&"ordinary.tmp".to_string()) && names.contains(&"shallow.tmp".to_string()),
        "{names:?}"
    );
    assert!(
        !names.contains(&"hostile".to_string()),
        "a tree we could not fully examine must never collapse into one removable item"
    );
    assert!(!names.contains(&"buried.tmp".to_string()));

    // Cleaning that plan removes only what was examined and leaves the deep tree alone.
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::Smart);
    assert_eq!(report.files, 2);
    assert!(bottom.join("buried.tmp").exists());
    remove_deep(bottom.clone(), &sb.temp());
}

#[test]
fn a_deep_tree_that_appears_after_the_scan_is_not_recursed_into_by_clean() {
    let sb = Sandbox::new();
    put(&sb.temp().join("cache").join("a.bin"), 100, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    assert_eq!(
        category(&scan, TEMP_RULE).items[0].kind,
        ItemKind::Dir,
        "the cache folder collapses into one item"
    );

    // Between the scan and the clean, something builds a hostile chain inside that folder.
    let bottom = make_deep(&sb.temp().join("cache").join("sub"), 2_000);
    put(&bottom.join("buried.tmp"), 50, 99);

    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysPermanent);
    assert!(!sb.temp().join("cache").join("a.bin").exists(), "the listed file is still removed");
    assert!(
        bottom.join("buried.tmp").exists(),
        "the cleaner must not wander into an unexamined deep tree"
    );
    assert!(report.skipped.contains_key(&SkipReason::TooDeep), "{:?}", report.skipped);
    remove_deep(bottom, &sb.temp().join("cache"));
}

// ─── findings of the independent safety review ───────────────────────────────

const ERROR_REPORTS_RULE: &str = "windows.error-reports";

fn days_ago(n: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(n * DAY)
}

#[test]
fn a_file_that_just_arrived_is_recent_whatever_its_modified_time_says() {
    // Extracting an archive or copying a file keeps the old modification time but gives the
    // file a brand-new creation time. To the user it is a new file: "newer than 3 days is never
    // touched" has to hold for it.
    let sb = Sandbox::new();
    let arrived = sb.temp().join("extracted.dll");
    put(&arrived, 100, 0);
    set_times(&arrived, days_ago(400), SystemTime::now());
    put(&sb.temp().join("really-old.tmp"), 50, 40);

    let scan = sb.scan(&[TEMP_RULE]);
    let cat = category(&scan, TEMP_RULE);
    assert_eq!(cat.items.len(), 1, "only the genuinely old file is listed: {:?}", cat.items);
    assert_eq!((cat.stats.kept_files, cat.stats.kept_bytes), (1, 100));
    sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysPermanent);
    assert!(arrived.exists());
    assert!(!sb.temp().join("really-old.tmp").exists());
}

#[test]
fn a_file_copied_into_a_listed_folder_after_the_scan_is_not_removed() {
    // The preview showed a folder holding one file. A file copied into it afterwards (old
    // modification time, new creation time) was never shown and must stay.
    let sb = Sandbox::new();
    put(&sb.temp().join("cache").join("a.bin"), 100, 9);
    let scan = sb.scan(&[TEMP_RULE]);
    assert_eq!(category(&scan, TEMP_RULE).items[0].kind, ItemKind::Dir);

    let late = sb.temp().join("cache").join("late.bin");
    put(&late, 10, 0);
    set_times(&late, days_ago(400), SystemTime::now());

    sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysPermanent);
    assert!(!sb.temp().join("cache").join("a.bin").exists(), "what the preview showed is removed");
    assert!(late.exists(), "what it did not show stays");
}

#[test]
fn files_created_in_a_listed_folder_after_the_scan_stay_even_without_an_age_gate() {
    let sb = Sandbox::new();
    put(&sb.temp().join("cache").join("a.bin"), 100, 9);
    let req = ScanRequest {
        categories: Some(vec![TEMP_RULE.to_string()]),
        age_override_hours: Some(0),
        ..ScanRequest::default()
    };
    let scan = sb.engine.scan(&req, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(category(&scan, TEMP_RULE).items[0].kind, ItemKind::Dir);

    std::thread::sleep(Duration::from_millis(30));
    put(&sb.temp().join("cache").join("late.bin"), 10, 0);

    sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysPermanent);
    assert!(!sb.temp().join("cache").join("a.bin").exists());
    assert!(sb.temp().join("cache").join("late.bin").exists(), "created after the preview");
}

#[test]
fn a_file_another_program_is_writing_to_is_never_removed() {
    // Loggers routinely open a file for writing while allowing others to delete it. Removing
    // such a file would silently cut the writer off from its own log.
    for (policy, label) in
        [(DeletionPolicy::AlwaysPermanent, "permanent"), (DeletionPolicy::AlwaysUndo, "undo")]
    {
        let sb = Sandbox::new();
        let path = sb.temp().join("held.tmp");
        put(&path, 100, 40);
        let writer = OpenOptions::new().write(true).share_mode(7).open(&path).unwrap();

        let scan = sb.scan(&[TEMP_RULE]);
        let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), policy);
        assert!(path.exists(), "{label}: a file being written to must survive");
        assert_eq!(report.files, 0, "{label}: {report:?}");
        assert_eq!(skipped(&report, SkipReason::InUse), 1, "{label}: {report:?}");
        drop(writer);
    }
}

#[test]
fn a_running_program_is_never_moved_into_recovery() {
    // Windows lets a running executable be *renamed* (only deleting it is refused), so an
    // undo-style clean would pull the file out from under its own process.
    let sb = Sandbox::new();
    let exe = sb.temp().join("tool.exe");
    fs::create_dir_all(sb.temp()).unwrap();
    if fs::copy(r"C:\Windows\System32\PING.EXE", &exe).is_err() {
        eprintln!("skipping: PING.EXE is not available");
        return;
    }
    set_times(&exe, days_ago(40), days_ago(40));
    let spawned = Command::new(&exe)
        .args(["-n", "30", "127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        eprintln!("skipping: cannot start a test program here");
        return;
    };
    std::thread::sleep(Duration::from_millis(500));
    if child.try_wait().unwrap().is_some() {
        eprintln!("skipping: the test program exited immediately");
        return;
    }

    let scan = sb.scan(&[TEMP_RULE]);
    let report = sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysUndo);
    let survived = exe.exists();
    let _ = child.kill();
    let _ = child.wait();
    assert!(survived, "the running program was moved away: {report:?}");
    assert_eq!(skipped(&report, SkipReason::InUse), 1, "{report:?}");
}

#[test]
fn a_name_ending_in_a_dot_cannot_stand_in_for_another_file() {
    // Win32 silently drops trailing dots and spaces, so "keep.tmp." and "keep.tmp" are one file
    // to most tools. Only the verbatim form can even create the former.
    let sb = Sandbox::new();
    let keep = sb.temp().join("keep.tmp");
    put(&keep, 10, 40);
    let dotted = PathBuf::from(format!(r"\\?\{}\keep.tmp.", sb.temp().display()));
    fs::write(&dotted, vec![b'x'; 10]).unwrap();
    // Same size and times, so nothing but the name tells the two apart.
    let same = days_ago(40);
    set_times(&keep, same, same);
    set_times(&dotted, same, same);
    put(&sb.temp().join("other.tmp"), 5, 40);

    let scan = sb.scan(&[TEMP_RULE]);
    let c = scan.category_index(TEMP_RULE).unwrap();
    assert!(
        category(&scan, TEMP_RULE).items.iter().all(|i| !i.path.to_string_lossy().ends_with('.')),
        "a name Win32 cannot address unambiguously must not even be offered"
    );
    let keep_idx = category(&scan, TEMP_RULE)
        .items
        .iter()
        .position(|i| i.path.file_name().is_some_and(|n| n == "keep.tmp"))
        .expect("keep.tmp is listed");
    let mut sel = sb.select(&[TEMP_RULE]);
    sel.excluded.insert(item_id(c, keep_idx));

    sb.clean(&scan, &sel, DeletionPolicy::AlwaysPermanent);
    assert!(keep.exists(), "the user excluded keep.tmp; a look-alike name must not remove it");
    assert!(!sb.temp().join("other.tmp").exists());
    let _ = fs::remove_file(&dotted);
}

#[test]
fn a_session_temp_folder_inside_the_temp_folder_is_neither_counted_twice_nor_removed() {
    // Remote-desktop sessions set TEMP to ...\Temp\2, which puts the live temp folder inside
    // the other temp root.
    let sb = Sandbox::with_env(|env, _| env.temp = env.local_app_data.join("Temp").join("2"));
    let session = sb.env.temp.clone();
    let outer = sb.env.local_app_data.join("Temp");
    put(&session.join("a.tmp"), 200, 20);
    put(&session.join("sub").join("c.tmp"), 40, 20);
    put(&outer.join("b.tmp"), 100, 20);

    let scan = sb.scan(&[TEMP_RULE]);
    let cat = category(&scan, TEMP_RULE);
    assert_eq!(cat.stats.bytes, 340, "nothing may be counted twice: {:?}", cat.stats);
    assert_eq!(cat.stats.files, 3);
    sb.clean(&scan, &sb.select(&[TEMP_RULE]), DeletionPolicy::AlwaysPermanent);
    assert!(session.is_dir(), "the live TEMP folder itself must never be removed");
    assert!(!session.join("a.tmp").exists() && !session.join("sub").join("c.tmp").exists());
    assert!(!outer.join("b.tmp").exists());
}

#[test]
fn a_fixed_location_that_was_turned_into_a_link_is_refused() {
    // Some cleaned folders (the error-report queues) are writable by every user. One of them
    // turned into a junction to somebody else's documents must not be treated as junk.
    let sb = Sandbox::new();
    let precious = sb.root.join("Users").join("other").join("Documents").join("precious");
    put(&precious.join("thesis.docx"), 5000, 400);
    let queue =
        sb.env.program_data.join("Microsoft").join("Windows").join("WER").join("ReportQueue");
    fs::create_dir_all(queue.parent().unwrap()).unwrap();
    if !junction(&queue, &precious) {
        eprintln!("skipping: cannot create junctions here");
        return;
    }

    let scan = sb.scan(&[ERROR_REPORTS_RULE]);
    let cat = category(&scan, ERROR_REPORTS_RULE);
    assert!(cat.items.is_empty(), "files behind the link were listed: {:?}", cat.items);
    assert!(cat.note.as_deref().is_some_and(|n| n.contains("link")), "{:?}", cat.note);
    sb.clean(&scan, &sb.select(&[ERROR_REPORTS_RULE]), DeletionPolicy::AlwaysPermanent);
    assert!(precious.join("thesis.docx").exists());
}
