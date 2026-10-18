// Everything the UI *does*: starting scans and cleans, changing the selection, undoing,
// saving settings. Views call these; they never talk to the backend directly.

import { flush, signal } from '../lib/signal.js';
import { bytes, plural } from '../lib/format.js';
import { AppError, channel, invoke } from './bridge.js';
import { S, idleClean, idleScan, isBusy, reduceMotion } from './store.js';
import { applyTheme } from './theme.js';
import { reportToLog } from './log.js';

// ─── announcements & toasts ─────────────────────────────────────────────────────────

/** Text for the screen-reader live region. */
export const announcement = signal('');
export function announce(text) {
  announcement.set('');
  queueMicrotask(() => announcement.set(text));
}

let toastSeq = 0;

/** Shows a toast. `actions` is a list of `{ label, run, primary }`. Returns its id. */
export function toast({ kind = 'info', title, text = '', actions = [], timeout = 6500, batchId = null }) {
  const id = ++toastSeq;
  S.toasts.update((list) => [...list.slice(-3), { id, kind, title, text, actions, batchId, leaving: false }]);
  if (timeout) setTimeout(() => dismissToast(id), timeout);
  return id;
}

export function dismissToast(id) {
  if (!S.toasts.peek().some((t) => t.id === id)) return;
  S.toasts.update((list) => list.map((t) => (t.id === id ? { ...t, leaving: true } : t)));
  setTimeout(() => S.toasts.update((list) => list.filter((t) => t.id !== id)), 220);
}

export function handleError(e, context = '') {
  const err = e instanceof AppError ? e : new AppError({ detail: e?.message ?? String(e) });
  reportToLog('error', `${context ? context + ': ' : ''}${err.code}: ${err.title} — ${err.detail}`);
  toast({ kind: 'bad', title: err.title, text: err.detail, timeout: 9000 });
}

// ─── navigation ─────────────────────────────────────────────────────────────────────

export function navigate(view, { focus } = {}) {
  if (focus) S.focus.set(focus);
  if (S.view.peek() === view) return;
  const apply = () => {
    S.view.set(view);
    flush();
  };
  if (document.startViewTransition && !reduceMotion.peek()) document.startViewTransition(apply);
  else apply();
}

// ─── system numbers ─────────────────────────────────────────────────────────────────

export async function refreshSystem() {
  try {
    const o = await invoke('system_overview');
    S.sys.set(o);
    if (o.memory) S.memHistory.update((h) => [...h.slice(-59), o.memory.loadPercent]);
  } catch {
    /* transient; the next tick will try again */
  }
}

export function startSystemPolling() {
  const tick = () => {
    if (!document.hidden) refreshSystem();
  };
  setInterval(tick, 2500);
  document.addEventListener('visibilitychange', tick);
  refreshSystem();
}

export async function refreshSummary() {
  try {
    const s = await invoke('app_summary');
    S.totals.set(s.totals);
    S.recovery.update((r) => ({ ...r, bytes: s.recovery.bytes, count: s.recovery.batches }));
  } catch {
    /* non-critical */
  }
}

// ─── scanning ───────────────────────────────────────────────────────────────────────

export async function startScan(categories = null) {
  if (isBusy.peek()) return;
  S.scan.set({ ...idleScan(), status: 'scanning', startedAt: performance.now() });
  S.clean.set(idleClean());
  S.lastClean.set(null);
  S.selection.set({ selected: new Set(), plan: null });
  announce('Scanning your PC.');
  try {
    await invoke('scan_start', { request: { categories }, onEvent: channel(onScanEvent) });
  } catch (e) {
    S.scan.set({ ...idleScan(), status: 'error', error: e });
    handleError(e, 'scan');
  }
}

function onScanEvent(ev) {
  switch (ev.event) {
    case 'started':
      S.scan.update((s) => ({ ...s, scanId: ev.scanId }));
      break;
    case 'progress':
      S.scan.update((s) => (s.status === 'scanning' ? { ...s, progress: ev } : s));
      break;
    case 'categoryDone':
      S.scan.update((s) => ({ ...s, categories: { ...s.categories, [ev.category.id]: ev.category } }));
      break;
    case 'finished':
      onScanFinished(ev.summary);
      break;
    case 'failed':
      S.scan.set({ ...idleScan(), status: 'error', error: ev.error });
      handleError(ev.error, 'scan');
      break;
  }
}

async function onScanFinished(summary) {
  const cancelled = summary.cancelled;
  S.scan.update((s) => ({
    ...s,
    status: cancelled ? 'cancelled' : 'ready',
    scanId: summary.scanId,
    summary,
    progress: { ...s.progress, entries: summary.entries, elapsedMs: summary.elapsedMs, fraction: 1, etaMs: null },
  }));
  if (cancelled) {
    announce('Scan cancelled.');
    toast({ kind: 'info', title: 'Scan cancelled', text: 'Nothing was changed.' });
    return;
  }
  await applySelection({});
  refreshSummary();
  const sel = S.selection.peek().plan;
  announce(sel?.bytes ? `Scan complete. ${bytes(sel.bytes)} can be cleaned.` : 'Scan complete. Nothing to clean.');
}

export async function cancelJob() {
  try {
    await invoke('job_cancel');
  } catch (e) {
    handleError(e);
  }
}

// ─── selection ──────────────────────────────────────────────────────────────────────

let selectionChain = Promise.resolve();

/** Sends a selection change to the backend (serialised, so rapid clicks stay in order). */
export function applySelection(patch = {}) {
  selectionChain = selectionChain.then(async () => {
    const id = S.scan.peek().scanId;
    if (!id || S.scan.peek().status !== 'ready') return null;
    try {
      const r = await invoke('selection_apply', { scanId: id, patch });
      S.selection.set({ selected: new Set(r.selected), plan: r.plan });
      return r;
    } catch (e) {
      handleError(e, 'selection');
      return null;
    }
  });
  return selectionChain;
}

export function toggleCategory(id) {
  const sel = new Set(S.selection.peek().selected);
  const on = !sel.has(id);
  on ? sel.add(id) : sel.delete(id);
  S.selection.update((s) => ({ ...s, selected: sel })); // optimistic; the response brings the plan
  return applySelection(on ? { select: [id] } : { deselect: [id] });
}

export function selectDefaults() {
  return applySelection({ selectDefaults: true });
}

export function selectNone() {
  const all = [...S.selection.peek().selected];
  S.selection.update((s) => ({ ...s, selected: new Set() }));
  return applySelection({ deselect: all });
}

export function setItemExcluded(itemId, excluded) {
  return applySelection(excluded ? { exclude: [itemId] } : { include: [itemId] });
}

export function clearExclusions(categoryId) {
  return applySelection({ clearExclusionsIn: categoryId });
}

// ─── cleaning ───────────────────────────────────────────────────────────────────────

export async function startClean({ acknowledged = false } = {}) {
  const scan = S.scan.peek();
  if (scan.status !== 'ready' || isBusy.peek()) return;
  const sysDisk = S.sys.peek().disks.find((d) => d.system);
  S.clean.set({ ...idleClean(), status: 'running', startedAt: performance.now(), freeBefore: sysDisk?.free ?? null });
  announce('Cleaning.');
  try {
    await invoke('clean_start', {
      scanId: scan.scanId,
      acknowledgeIrreversible: acknowledged,
      onEvent: channel(onCleanEvent),
    });
  } catch (e) {
    S.clean.set({ ...idleClean(), status: 'error', error: e });
    handleError(e, 'clean');
  }
}

function onCleanEvent(ev) {
  switch (ev.event) {
    case 'started':
      S.clean.update((c) => ({ ...c, totals: { filesTotal: ev.filesTotal, bytesTotal: ev.bytesTotal } }));
      break;
    case 'progress':
      S.clean.update((c) => (c.status === 'running' ? { ...c, progress: ev } : c));
      break;
    case 'finished':
      onCleanFinished(ev.report);
      break;
    case 'failed':
      S.clean.set({ ...idleClean(), status: 'error', error: ev.error });
      handleError(ev.error, 'clean');
      break;
  }
}

async function onCleanFinished(report) {
  const before = S.clean.peek().freeBefore;
  S.clean.set({ ...idleClean(), status: 'done', report });
  S.scan.set(idleScan()); // the plan is spent; a new scan is needed for fresh numbers
  S.selection.set({ selected: new Set(), plan: null });
  await Promise.all([refreshRecovery(), refreshSystem(), refreshSummary()]);
  const after = S.sys.peek().disks.find((d) => d.system)?.free ?? null;
  S.lastClean.set({
    report,
    finishedAt: Date.now() / 1000,
    freeBefore: before,
    freeAfter: after,
  });
  announce(`Cleaned ${bytes(report.bytes)}.`);

  const undoable = report.batch;
  toast({
    kind: report.files ? 'good' : 'info',
    title: report.files ? `Cleaned ${bytes(report.bytes)}` : 'Nothing was removed',
    text: report.files
      ? undoable
        ? `${bytes(report.undoBytes)} is kept for undo.`
        : `${plural(report.files, 'file')} removed.`
      : 'Everything in the plan had changed or was in use.',
    actions: undoable ? [{ label: 'Undo', run: () => restoreBatch(undoable.id) }] : [],
    batchId: undoable?.id ?? null,
    timeout: undoable ? 12000 : 6500,
  });
}

/** Once a batch is restored or freed, an "Undo" toast for it would be a lie. */
function dismissToastsFor(batchId) {
  for (const t of S.toasts.peek()) if (t.batchId === batchId) dismissToast(t.id);
}

// ─── recovery ───────────────────────────────────────────────────────────────────────

export async function refreshRecovery() {
  try {
    const r = await invoke('recovery_list');
    S.recovery.set({ batches: r.batches, bytes: r.bytes, count: r.batches.length, loaded: true });
  } catch (e) {
    handleError(e, 'recovery');
  }
}

export async function restoreBatch(id) {
  if (!S.recovery.peek().batches.some((b) => b.id === id)) {
    toast({ kind: 'info', title: 'Already restored', text: 'That recovery copy is no longer there.' });
    dismissToastsFor(id);
    return;
  }
  setBatchBusy(id, 'restoring');
  try {
    const r = await invoke('recovery_restore', { id });
    dismissToastsFor(id);
    const notes = [];
    if (r.conflicts) notes.push(`${plural(r.conflicts, 'file')} already existed and were left in Recovery`);
    if (r.refused) notes.push(`${plural(r.refused, 'file')} belong to places Vexus no longer manages`);
    if (r.failed) notes.push(`${plural(r.failed, 'file')} couldn't be restored`);
    toast({
      kind: r.conflicts || r.refused || r.failed ? 'warn' : 'good',
      title: r.restoredFiles ? `Restored ${plural(r.restoredFiles, 'file')}` : 'Nothing restored',
      text: notes.join('. '),
    });
  } catch (e) {
    handleError(e, 'restore');
  } finally {
    await Promise.all([refreshRecovery(), refreshSystem(), refreshSummary()]);
  }
}

export async function purgeBatch(id) {
  setBatchBusy(id, 'freeing');
  try {
    const freed = await invoke('recovery_purge', { id });
    dismissToastsFor(id);
    toast({ kind: 'good', title: `Freed ${bytes(freed)}`, text: 'The recovery copy was removed for good.' });
  } catch (e) {
    handleError(e, 'purge');
  } finally {
    await Promise.all([refreshRecovery(), refreshSystem(), refreshSummary()]);
  }
}

function setBatchBusy(id, state) {
  S.recovery.update((r) => ({ ...r, batches: r.batches.map((b) => (b.id === id ? { ...b, busy: state } : b)) }));
}

// ─── settings & misc ────────────────────────────────────────────────────────────────

/** Applies a setting immediately (so the UI responds at once) and persists it. */
export async function saveSettings(patch) {
  const prev = S.settings.peek();
  const next = { ...prev, ...patch };
  S.settings.set(next);
  applyTheme(next);
  try {
    const saved = await invoke('settings_set', { settings: next });
    S.settings.set(saved);
    applyTheme(saved);
  } catch (e) {
    S.settings.set(prev);
    applyTheme(prev);
    handleError(e, 'settings');
  }
}

/** Live preview while dragging the accent slider, without saving. */
export function previewSettings(patch) {
  applyTheme({ ...S.settings.peek(), ...patch });
}

export function openFolder(which) {
  invoke('open_folder', { which }).catch(handleError);
}

export function revealItem(itemId) {
  const scanId = S.scan.peek().scanId;
  if (scanId) invoke('reveal_item', { scanId, itemId }).catch(handleError);
}

export async function restartElevated() {
  try {
    await invoke('restart_elevated');
  } catch (e) {
    toast({ kind: 'warn', title: 'Couldn’t restart as administrator', text: e instanceof AppError ? e.detail : String(e), timeout: 9000 });
  }
}

export function quit() {
  invoke('quit_app').catch(() => {});
}
