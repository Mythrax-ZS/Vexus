// Application state. Plain signals, grouped by concern; actions.js is the only writer for
// anything that involves the backend.

import { computed, signal } from '../lib/signal.js';

export const VIEWS = ['overview', 'cleaner', 'recovery', 'settings'];

export const idleScan = () => ({
  status: 'idle', // idle | scanning | ready | cancelled | error
  scanId: null,
  progress: { entries: 0, foundFiles: 0, foundBytes: 0, categoriesDone: 0, categoriesTotal: 0, current: '', elapsedMs: 0, fraction: null, etaMs: null },
  categories: {}, // id → CategorySummary, filled as categories finish
  summary: null,
  error: null,
  startedAt: 0,
});

export const idleClean = () => ({
  status: 'idle', // idle | running | done | error
  progress: null,
  totals: { filesTotal: 0, bytesTotal: 0 },
  report: null,
  error: null,
  startedAt: 0,
});

export const S = {
  /** Static facts from the backend (version, os, paths, …). */
  boot: signal(null),
  settings: signal(null),
  view: signal('overview'),
  categories: signal([]),
  groups: signal([]),

  /** Live system numbers for the dashboard. */
  sys: signal({ disks: [], memory: null }),
  memHistory: signal([]),

  scan: signal(idleScan()),
  selection: signal({ selected: new Set(), plan: null }),
  clean: signal(idleClean()),
  /** Details of the most recent clean, kept for the dashboard's "done" state. */
  lastClean: signal(null),

  recovery: signal({ batches: [], bytes: 0, loaded: false }),
  totals: signal({ cleans: 0, files: 0, bytes: 0, lastClean: null, lastScan: null }),

  toasts: signal([]),
  /** Category highlighted in the Cleaner's detail pane. */
  focus: signal(null),
  paletteOpen: signal(false),
  /** Bumps when the user asks the Cleaner to refresh its item list. */
  itemsVersion: signal(0),
};

// ─── derived state ──────────────────────────────────────────────────────────────────

export const catById = computed(() => Object.fromEntries(S.categories().map((c) => [c.id, c])));

export const isBusy = computed(() => S.scan().status === 'scanning' || S.clean().status === 'running');

// `S.scan` is replaced on every progress tick, so anything that only cares about one field reads
// it through a computed: those only notify when the value itself changes.
export const scanStatus = computed(() => S.scan().status);
export const scanId = computed(() => S.scan().scanId);

export const elevated = computed(() => !!S.boot()?.elevated);
export const sandboxed = computed(() => !!S.boot()?.sandboxed);

/** Per-category result: summary stats merged with the static rule metadata. */
export const results = computed(() => {
  const scan = S.scan();
  const done = scan.summary ? Object.fromEntries(scan.summary.categories.map((c) => [c.id, c])) : scan.categories;
  return S.categories().map((meta) => ({ meta, result: done[meta.id] ?? null }));
});

/** Everything found, sorted by size (largest first). */
export const found = computed(() =>
  results()
    .filter((r) => r.result && r.result.stats.bytes > 0)
    .sort((a, b) => b.result.stats.bytes - a.result.stats.bytes),
);

export const foundBytes = computed(() => found().reduce((n, r) => n + r.result.stats.bytes, 0));

export const reduceMotion = computed(
  () => !!S.settings()?.reduceMotion || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches,
);
