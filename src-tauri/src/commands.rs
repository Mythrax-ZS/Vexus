//! IPC commands: the complete list of things the UI can ask Rust to do.
//!
//! Note what is *not* here: nothing accepts a file path to delete. Cleaning is expressed purely
//! as "run the plan for scan N", where the plan was built — and is held — by this process.

use std::os::windows::process::CommandExt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use vexus_core::clean::{CleanOptions, CleanReport};
use vexus_core::error::{Error, UserError};
use vexus_core::history::{HistoryEntry, HistoryKind, Totals};
use vexus_core::model::{Group, ItemId, ScanId, split_item_id};
use vexus_core::progress::{CleanEvent, ScanEvent};
use vexus_core::recovery::{BatchInfo, RestoreReport};
use vexus_core::rules::RuleMeta;
use vexus_core::scan::{ItemPage, ItemQuery, PlanSummary, ScanRequest};
use vexus_core::settings::Settings;
use vexus_core::system::{self, DiskInfo, MemoryInfo};

use crate::state::{AppState, JobKind, open_in_explorer};
use crate::{elevation, osinfo, tray, window};

type CmdResult<T> = Result<T, UserError>;

fn user(e: Error) -> UserError {
    e.to_user()
}

fn custom(code: &'static str, title: &str, detail: &str) -> UserError {
    UserError { code, title: title.to_string(), detail: detail.to_string() }
}

// ─── start-up & system ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    id: Group,
    label: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySummary {
    batches: usize,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    version: &'static str,
    os: String,
    sandboxed: bool,
    portable: bool,
    elevated: bool,
    settings: Settings,
    categories: Vec<RuleMeta>,
    groups: Vec<GroupInfo>,
    data_dir: String,
    log_dir: String,
    recovery: RecoverySummary,
    totals: Totals,
    disks: Vec<DiskInfo>,
    memory: Option<MemoryInfo>,
    last_scan_entries: Option<u64>,
}

/// Everything the UI needs for its first paint, in one round trip.
#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Bootstrap {
    let categories = state.engine.rules();
    let mut groups: Vec<GroupInfo> = Vec::new();
    for g in [Group::System, Group::Privacy, Group::Apps, Group::Browsers] {
        if categories.iter().any(|c| c.group == g) {
            groups.push(GroupInfo { id: g, label: g.label() });
        }
    }
    let batches = state.engine.recovery().list();
    Bootstrap {
        version: env!("CARGO_PKG_VERSION"),
        os: osinfo::describe(),
        sandboxed: state.sandboxed,
        portable: state.data.portable,
        elevated: state.elevated,
        settings: state.settings(),
        categories,
        groups,
        data_dir: state.data.root.display().to_string(),
        log_dir: state.data.logs.display().to_string(),
        recovery: RecoverySummary {
            batches: batches.len(),
            bytes: batches.iter().map(|b| b.bytes).sum(),
        },
        totals: state.history.totals(),
        disks: system::disks(&state.engine.env().system_drive),
        memory: system::memory(),
        last_scan_entries: state.last_scan_entries(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    totals: Totals,
    recovery: RecoverySummary,
}

/// The few numbers that change after a scan, clean or restore (kept separate from `bootstrap`
/// so refreshing them is cheap).
#[tauri::command]
pub fn app_summary(state: State<'_, AppState>) -> Summary {
    let batches = state.engine.recovery().list();
    Summary {
        totals: state.history.totals(),
        recovery: RecoverySummary {
            batches: batches.len(),
            bytes: batches.iter().map(|b| b.bytes).sum(),
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    disks: Vec<DiskInfo>,
    memory: Option<MemoryInfo>,
}

#[tauri::command]
pub fn system_overview(state: State<'_, AppState>) -> Overview {
    Overview { disks: system::disks(&state.engine.env().system_drive), memory: system::memory() }
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, settings: Settings) -> CmdResult<Settings> {
    let mut next = settings;
    next.sanitize();
    let before = state.settings();
    if before.verbose_logging != next.verbose_logging {
        state.logging.set_verbose(next.verbose_logging);
    }
    next.save(&state.data.state).map_err(|e| user(Error::io("Couldn't save your settings", e)))?;
    state.set_settings(next.clone());
    Ok(next)
}

// ─── scanning ────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ScanRequestDto {
    /// Category ids; omitted means every category.
    categories: Option<Vec<String>>,
}

#[tauri::command]
pub fn scan_start(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ScanRequestDto,
    on_event: Channel<ScanEvent>,
) -> CmdResult<()> {
    let (cancel, guard) = state.begin_job(JobKind::Scan).map_err(user)?;
    let state = state.inner().clone();
    std::thread::Builder::new()
        .name("vexus-scan".into())
        .spawn(move || {
            let _guard = guard; // releases the job slot however this thread ends
            let settings = state.settings();
            let req = ScanRequest {
                categories: request.categories,
                age_override_hours: Some(settings.temp_min_age_hours),
                expected_entries: state.last_scan_entries(),
                threads: None,
            };
            // `Finished` is withheld until the session is stored, so the UI can never ask for
            // items of a scan that isn't registered yet.
            let forward = on_event.clone();
            let sink = move |e: ScanEvent| {
                if !matches!(e, ScanEvent::Finished { .. }) {
                    let _ = forward.send(e);
                }
            };
            match state.engine.scan(&req, &cancel, &sink) {
                Ok(scan) => {
                    if !scan.cancelled {
                        state.remember_scan_entries(scan.entries);
                    }
                    let summary = scan.summary();
                    let scan = state.store_scan(scan);
                    if !scan.cancelled {
                        let mut h = HistoryEntry::new(
                            HistoryKind::Scan,
                            summary.total_files,
                            summary.total_bytes,
                        );
                        h.categories =
                            summary.categories.iter().map(|c| c.id.to_string()).collect();
                        state.history.append(&h);
                    }
                    tray::refresh(&app, &state, Some(&scan));
                    let _ = on_event.send(ScanEvent::Finished { summary });
                }
                Err(e) => {
                    tracing::error!("scan failed: {e}");
                    let _ = on_event.send(ScanEvent::Failed { scan_id: 0, error: e.to_user() });
                }
            }
        })
        .map_err(|e| user(Error::io("Couldn't start the scan", e)))?;
    Ok(())
}

/// Cancels the running scan, clean or restore.
#[tauri::command]
pub fn job_cancel(state: State<'_, AppState>) -> bool {
    state.cancel_job().is_some()
}

#[tauri::command]
pub fn scan_items(
    state: State<'_, AppState>,
    scan_id: ScanId,
    category: String,
    query: ItemQuery,
) -> CmdResult<ItemPage> {
    state
        .with_session(scan_id, |s| s.scan.items(&category, &query, &s.selection))
        .map_err(user)?
        .map_err(user)
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SelectionPatch {
    select: Vec<String>,
    deselect: Vec<String>,
    /// Remove these items from the plan.
    exclude: Vec<ItemId>,
    /// Put these items back into the plan.
    include: Vec<ItemId>,
    /// Re-apply the recommended selection.
    select_defaults: bool,
    /// Forget all item exclusions in one category.
    clear_exclusions_in: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionState {
    selected: Vec<String>,
    plan: PlanSummary,
}

#[tauri::command]
pub fn selection_apply(
    state: State<'_, AppState>,
    scan_id: ScanId,
    patch: SelectionPatch,
) -> CmdResult<SelectionState> {
    let policy = state.settings().deletion_policy;
    state
        .with_session(scan_id, |s| {
            if patch.select_defaults {
                s.selection = s.scan.default_selection();
            }
            for id in patch.select {
                if s.scan.category_index(&id).is_some() {
                    s.selection.categories.insert(id);
                }
            }
            for id in &patch.deselect {
                s.selection.categories.remove(id);
            }
            for id in patch.exclude {
                s.selection.excluded.insert(id);
            }
            for id in &patch.include {
                s.selection.excluded.remove(id);
            }
            if let Some(cat) = &patch.clear_exclusions_in
                && let Some(ci) = s.scan.category_index(cat)
            {
                s.selection.excluded.retain(|id| split_item_id(*id).0 != ci);
            }
            SelectionState {
                selected: s.selection.categories.iter().cloned().collect(),
                plan: s.scan.plan(&s.selection, policy),
            }
        })
        .map_err(user)
}

// ─── cleaning & recovery ─────────────────────────────────────────────────────────────

#[tauri::command]
pub fn clean_start(
    app: AppHandle,
    state: State<'_, AppState>,
    scan_id: ScanId,
    acknowledge_irreversible: bool,
    on_event: Channel<CleanEvent>,
) -> CmdResult<()> {
    let (scan, selection) = state.session_for_clean(scan_id).map_err(user)?;
    let settings = state.settings();

    // Defence in depth: the UI asks for explicit confirmation before irreversible removals,
    // and the backend refuses to proceed if that acknowledgement is missing.
    let plan = scan.plan(&selection, settings.deletion_policy);
    if plan.items == 0 {
        return Err(custom("empty-plan", "Nothing selected", "Choose something to clean first."));
    }
    if !plan.irreversible.is_empty() && !acknowledge_irreversible {
        return Err(custom(
            "needs-confirmation",
            "Please confirm",
            "Some of the selected items can't be undone. Confirm that you want to remove them.",
        ));
    }

    let (cancel, guard) = state.begin_job(JobKind::Clean).map_err(user)?;
    let state = state.inner().clone();
    std::thread::Builder::new()
        .name("vexus-clean".into())
        .spawn(move || {
            let _guard = guard;
            let opts = CleanOptions {
                policy: settings.deletion_policy,
                undo_days: settings.undo_days,
                threads: None,
            };
            let forward = on_event.clone();
            let sink = move |e: CleanEvent| {
                if !matches!(e, CleanEvent::Finished { .. }) {
                    let _ = forward.send(e);
                }
            };
            match state.engine.clean(&scan, &selection, &opts, &cancel, &sink) {
                Ok(report) => {
                    record_clean(&state, &selection.categories, &report);
                    state.drop_session(scan_id); // the plan is spent; a fresh scan is required
                    tray::refresh(&app, &state, None);
                    let _ = on_event.send(CleanEvent::Finished { report: Box::new(report) });
                }
                Err(e) => {
                    tracing::error!("clean failed: {e}");
                    let _ = on_event.send(CleanEvent::Failed { error: e.to_user() });
                }
            }
        })
        .map_err(|e| user(Error::io("Couldn't start cleaning", e)))?;
    Ok(())
}

fn record_clean(
    state: &AppState,
    categories: &std::collections::BTreeSet<String>,
    report: &CleanReport,
) {
    let mut h = HistoryEntry::new(HistoryKind::Clean, report.files, report.bytes);
    h.undo_bytes = report.undo_bytes;
    h.categories = categories.iter().cloned().collect();
    h.batch = report.batch.as_ref().map(|b| b.id.clone());
    state.history.append(&h);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryList {
    batches: Vec<BatchInfo>,
    bytes: u64,
}

#[tauri::command]
pub fn recovery_list(state: State<'_, AppState>) -> RecoveryList {
    let batches = state.engine.recovery().list();
    RecoveryList { bytes: batches.iter().map(|b| b.bytes).sum(), batches }
}

#[tauri::command]
pub async fn recovery_restore(state: State<'_, AppState>, id: String) -> CmdResult<RestoreReport> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (cancel, _guard) = state.begin_job(JobKind::Restore).map_err(user)?;
        let report = state.engine.restore(&id, &cancel).map_err(user)?;
        let mut h =
            HistoryEntry::new(HistoryKind::Restore, report.restored_files, report.restored_bytes);
        h.batch = Some(id);
        state.history.append(&h);
        Ok(report)
    })
    .await
    .map_err(|e| custom("internal", "Something went wrong", &e.to_string()))?
}

#[tauri::command]
pub async fn recovery_purge(state: State<'_, AppState>, id: String) -> CmdResult<u64> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (_, _guard) = state.begin_job(JobKind::Purge).map_err(user)?;
        let bytes = state.engine.recovery().purge(&id).map_err(user)?;
        let mut h = HistoryEntry::new(HistoryKind::Purge, 0, bytes);
        h.batch = Some(id);
        state.history.append(&h);
        Ok(bytes)
    })
    .await
    .map_err(|e| custom("internal", "Something went wrong", &e.to_string()))?
}

// ─── shell integration ───────────────────────────────────────────────────────────────

/// Shows an item from a scan in Explorer. The target comes from the scan session, never from
/// the page.
#[tauri::command]
pub fn reveal_item(state: State<'_, AppState>, scan_id: ScanId, item_id: ItemId) -> CmdResult<()> {
    let found = state
        .with_session(scan_id, |s| {
            let (c, i) = split_item_id(item_id);
            s.scan
                .categories
                .get(c)
                .and_then(|cat| cat.items.get(i))
                .map(|it| (it.kind, it.path.clone()))
        })
        .map_err(user)?;
    let Some((kind, path)) = found else {
        return Err(custom(
            "unknown-item",
            "Item not found",
            "That item is no longer part of the scan.",
        ));
    };
    let mut cmd = std::process::Command::new("explorer.exe");
    if kind == vexus_core::model::ItemKind::RecycleBin {
        cmd.arg("shell:RecycleBinFolder");
    } else {
        cmd.raw_arg(format!("/select,\"{}\"", path.display()));
    }
    let _ = cmd.spawn();
    Ok(())
}

/// Opens one of Vexus's own folders. Only a fixed set of names is accepted.
#[tauri::command]
pub fn open_folder(state: State<'_, AppState>, which: String) -> CmdResult<()> {
    let dir = match which.as_str() {
        "logs" => &state.data.logs,
        "data" => &state.data.root,
        "recovery" => &state.data.recovery,
        _ => {
            return Err(custom(
                "invalid",
                "Unknown folder",
                "That folder isn't one Vexus manages.",
            ));
        }
    };
    let _ = std::fs::create_dir_all(dir);
    open_in_explorer(dir);
    Ok(())
}

#[tauri::command]
pub fn restart_elevated(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if state.elevated {
        return Err("Vexus is already running as administrator.".into());
    }
    if state.cancel_job().is_some() {
        return Err("Wait for the current scan or clean to finish first.".into());
    }
    elevation::relaunch_as_admin(&state.args.relaunch_args())?;
    state.quitting.store(true, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        app.exit(0);
    });
    Ok(())
}

#[tauri::command]
pub fn app_ready(app: AppHandle) {
    window::show_main(&app);
}

#[tauri::command]
pub fn quit_app(app: AppHandle, state: State<'_, AppState>) {
    state.quitting.store(true, std::sync::atomic::Ordering::Relaxed);
    state.cancel_job();
    app.exit(0);
}

/// Lets the page's errors land in the same log file as everything else.
#[tauri::command]
pub fn log_frontend(level: String, message: String) {
    let message: String = message.chars().take(4000).collect();
    match level.as_str() {
        "error" => tracing::error!(target: "ui", "{message}"),
        "warn" => tracing::warn!(target: "ui", "{message}"),
        _ => tracing::info!(target: "ui", "{message}"),
    }
}
