// The command registry behind Ctrl+K and the global keyboard shortcuts.

import { ACCENT_PRESETS, hueToHex } from '../lib/color.js';
import { askToElevate } from '../components/sidebar.js';
import { confirmDialog } from '../components/dialog.js';
import {
  cancelJob, navigate, openFolder, quit, restoreBatch, saveSettings, selectDefaults, selectNone, startScan,
} from './actions.js';
import { S, elevated, isBusy } from './store.js';
import { startCleanFlow } from '../views/clean-flow.js';

/** Commands are re-evaluated on every palette open, so `enabled` always reflects live state. */
export function getCommands() {
  const scan = S.scan.peek();
  const plan = S.selection.peek().plan;
  const busy = isBusy.peek();
  const lastBatch = S.recovery.peek().batches[0];
  const settings = S.settings.peek();

  const list = [
    { id: 'go-overview', title: 'Go to Overview', icon: 'overview', shortcut: 'Ctrl+1', keywords: 'home dashboard', run: () => navigate('overview') },
    { id: 'go-cleaner', title: 'Go to Cleaner', icon: 'cleaner', shortcut: 'Ctrl+2', keywords: 'clean categories review', run: () => navigate('cleaner') },
    { id: 'go-recovery', title: 'Go to Recovery', icon: 'recovery', shortcut: 'Ctrl+3', keywords: 'undo restore backup', run: () => navigate('recovery') },
    { id: 'go-settings', title: 'Open Settings', icon: 'settings', shortcut: 'Ctrl+,', keywords: 'preferences options', run: () => navigate('settings') },
    { id: 'scan', title: scan.status === 'ready' ? 'Scan again' : 'Smart Scan', hint: 'Find what can be cleaned. Changes nothing.', icon: 'search', shortcut: 'Ctrl+R', keywords: 'scan find analyze refresh', enabled: !busy, run: () => startScan() },
    { id: 'clean', title: 'Clean now', hint: plan?.bytes ? 'Review and clean the current selection' : 'Run a scan first', icon: 'cleaner', shortcut: 'Ctrl+Enter', keywords: 'remove delete free', enabled: scan.status === 'ready' && !!plan?.items && !busy, run: () => startCleanFlow() },
    { id: 'cancel', title: 'Cancel current job', icon: 'x', keywords: 'stop abort', enabled: busy, run: () => cancelJob() },
    { id: 'undo-last', title: 'Undo the last clean', hint: lastBatch ? `Restore ${lastBatch.files} files` : 'Nothing to undo', icon: 'undo', keywords: 'restore recover rollback', enabled: !!lastBatch && !lastBatch.busy, run: () => restoreBatch(lastBatch.id) },
    { id: 'select-defaults', title: 'Select recommended categories', icon: 'check', keywords: 'defaults smart', enabled: scan.status === 'ready', run: () => { navigate('cleaner'); selectDefaults(); } },
    { id: 'select-none', title: 'Deselect everything', icon: 'x', keywords: 'clear none', enabled: scan.status === 'ready', run: () => selectNone() },
    { id: 'advanced', title: settings?.advanced ? 'Hide advanced options' : 'Show advanced options', hint: 'Risk levels, every category and per-item review', icon: 'sliders', keywords: 'expert details', run: () => saveSettings({ advanced: !settings?.advanced }) },
    { id: 'theme-dark', title: 'Theme: Dark', icon: 'moon', keywords: 'appearance', run: () => saveSettings({ theme: 'dark' }) },
    { id: 'theme-light', title: 'Theme: Light', icon: 'sun', keywords: 'appearance', run: () => saveSettings({ theme: 'light' }) },
    { id: 'theme-system', title: 'Theme: Match system', icon: 'monitor', keywords: 'appearance auto', run: () => saveSettings({ theme: 'system' }) },
    ...ACCENT_PRESETS.map((p) => ({ id: `accent-${p.id}`, title: `Accent: ${p.name}`, icon: 'palette', keywords: 'color colour theme', run: () => saveSettings({ accent: hueToHex(p.hue) }) })),
    { id: 'open-logs', title: 'Open log folder', hint: 'Everything Vexus does is recorded locally', icon: 'scroll', keywords: 'diagnostics', run: () => openFolder('logs') },
    { id: 'open-recovery', title: 'Open recovery folder', icon: 'folder', run: () => openFolder('recovery') },
    { id: 'open-data', title: 'Open data folder', icon: 'folder', run: () => openFolder('data') },
    { id: 'elevate', title: 'Restart as administrator', hint: 'Unlocks Windows temp, update caches and logs', icon: 'lock', keywords: 'admin uac elevate', enabled: !elevated.peek(), run: () => askToElevate() },
    { id: 'quit', title: 'Quit Vexus', icon: 'power', run: async () => { if (!isBusy.peek() || (await confirmDialog({ title: 'Quit while a job is running?', text: 'The current job will be cancelled.', confirmLabel: 'Quit', danger: true }))) quit(); } },
  ];
  return list.filter((c) => c.enabled !== false);
}

/** Global shortcuts. Returns true if the event was handled. */
export function handleShortcut(e) {
  const ctrl = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  const find = (id) => getCommands().find((c) => c.id === id);
  const run = (id) => {
    const c = find(id);
    if (c) c.run();
    return true;
  };
  if (ctrl && key === 'k') {
    S.paletteOpen.update((v) => !v);
    return true;
  }
  if (e.key === 'F5' || (ctrl && key === 'r')) return run('scan') || true;
  if (ctrl && key === '1') return run('go-overview');
  if (ctrl && key === '2') return run('go-cleaner');
  if (ctrl && key === '3') return run('go-recovery');
  if (ctrl && key === ',') return run('go-settings');
  if (ctrl && e.key === 'Enter') return run('clean');
  return false;
}
