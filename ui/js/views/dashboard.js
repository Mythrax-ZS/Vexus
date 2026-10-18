// Overview: one hero that always says what's going on, then the numbers that matter.

import { h, each } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { computed, signal, tween, untrack } from '../lib/signal.js';
import { ago, bytes, count, duration, etaText, greeting, midEllipsis, percent, plural, sizeParts } from '../lib/format.js';
import { S, catById, elevated, found, foundBytes, reduceMotion } from '../core/store.js';
import { cancelJob, navigate, restoreBatch, startScan } from '../core/actions.js';
import { askToElevate } from '../components/sidebar.js';
import { startCleanFlow } from './clean-flow.js';
import { Orb } from '../components/orb.js';
import { Button, Meter } from '../components/ui.js';
import { DataTable, RankBars, Sparkline } from '../components/viz.js';
import { tooltip } from '../components/tooltip.js';

// ─── hero ───────────────────────────────────────────────────────────────────────────

function heroState() {
  const sc = S.scan();
  const cl = S.clean();
  if (cl.status === 'running') return 'cleaning';
  if (sc.status === 'scanning') return 'scanning';
  if (cl.status === 'done' && S.lastClean()) return 'done';
  if (sc.status === 'ready') return foundBytes() > 0 ? 'ready' : 'empty';
  if (sc.status === 'error' || cl.status === 'error') return 'error';
  return 'idle';
}

function systemDisk() {
  return S.sys().disks.find((d) => d.system) ?? S.sys().disks[0] ?? null;
}

function Figure({ value }) {
  const shown = tween(value, { reduced: reduceMotion, from: 0 });
  return h(
    'div',
    { class: 'hero-figure' },
    h('span', { class: 'hero-value' }, () => sizeParts(shown()).value),
    h('span', { class: 'hero-unit' }, () => sizeParts(shown()).unit),
  );
}

function Hero() {
  const state = computed(heroState);

  const orbProgress = () => {
    const s = state();
    if (s === 'scanning') return S.scan().progress.fraction;
    if (s === 'cleaning') return S.clean().progress?.fraction ?? 0;
    return null;
  };

  const idleBody = () => {
    const low = () => {
      const d = systemDisk();
      return d && d.free / d.total < 0.1 ? d : null;
    };
    return [
      h('div', { class: 'hero-eyebrow' }, greeting()),
      h('h1', { class: 'hero-title' }, () => (low() ? `${low().root.replace('\\', '')} is almost full` : 'Ready for a tune-up')),
      h('p', { class: 'hero-sub' }, () =>
        low()
          ? `Only ${bytes(low().free)} free (${percent(low().free / low().total, 1)}). Run a Smart Scan to see what can be cleaned safely.`
          : 'Run a Smart Scan to see what can be cleaned safely. It only looks — nothing is removed until you say so.',
      ),
      h(
        'div',
        { class: 'hero-actions' },
        Button({ label: 'Smart Scan', icon: 'search', kind: 'primary', size: 'lg', onClick: () => startScan() }),
        () => {
          const last = S.totals().lastScan;
          return last ? h('span', { class: 'muted hero-note' }, `Last scan ${ago(last.ts)} found ${bytes(last.bytes)}`) : null;
        },
      ),
    ];
  };

  const scanningBody = () => {
    const p = () => S.scan().progress;
    return [
      h('div', { class: 'hero-eyebrow' }, 'Scanning'),
      Figure({ value: () => p().foundBytes }),
      h('p', { class: 'hero-sub' }, () => `found so far · ${count(p().entries)} items checked`),
      h('div', { class: 'hero-path mono truncate' }, () => (p().current ? midEllipsis(p().current, 72) : ' ')),
      h('div', { class: 'progress', 'data-indet': () => (p().fraction == null ? '' : null), style: { '--v': () => p().fraction ?? 0 } }, h('i')),
      h(
        'div',
        { class: 'hero-actions' },
        Button({ label: 'Cancel', onClick: cancelJob }),
        h('span', { class: 'muted hero-note' }, () => {
          const bits = [`${p().categoriesDone} of ${p().categoriesTotal} categories`];
          if (p().etaMs != null) bits.push(etaText(p().etaMs));
          return bits.join(' · ');
        }),
      ),
    ];
  };

  const readyBody = () => {
    const plan = () => S.selection().plan;
    const moreBytes = () => Math.max(0, foundBytes() - (plan()?.bytes ?? 0));
    return [
      h('div', { class: 'hero-eyebrow' }, 'Scan complete', h('span', { class: 'eyebrow-meta' }, () => ` · ${duration(S.scan().summary?.elapsedMs)}`)),
      Figure({ value: () => plan()?.bytes ?? 0 }),
      h('p', { class: 'hero-sub' }, () => {
        const pl = plan();
        if (!pl?.items) return 'Nothing is selected. Pick what to clean in the Cleaner.';
        const bits = [`${plural(pl.items, 'item')} in ${plural(pl.categories.length, 'category', 'categories')}`];
        if (pl.permanentBytes) bits.push(`${bytes(pl.permanentBytes)} removed for good`);
        if (pl.undoBytes) bits.push(`${bytes(pl.undoBytes)} kept for undo`);
        return bits.join(' · ');
      }),
      h(
        'div',
        { class: 'hero-meter' },
        Meter({ value: () => (foundBytes() ? (plan()?.bytes ?? 0) / foundBytes() : 0), label: 'Share of found space selected' }),
        h('span', { class: 'muted num' }, () => `${bytes(plan()?.bytes ?? 0)} of ${bytes(foundBytes())} found is selected`),
      ),
      h(
        'div',
        { class: 'hero-actions' },
        h(
          'button',
          { type: 'button', class: 'btn primary lg', disabled: () => !plan()?.items, onClick: startCleanFlow },
          icon('cleaner'),
          () => (plan()?.items ? `Clean ${bytes(plan().bytes)}` : 'Clean now'),
        ),
        Button({ label: 'Review details', icon: 'arrow-right', size: 'lg', onClick: () => navigate('cleaner') }),
        () => (moreBytes() > 0 ? h('span', { class: 'muted hero-note' }, `${bytes(moreBytes())} more is available to review`) : null),
      ),
    ];
  };

  const emptyBody = () => [
    h('div', { class: 'hero-eyebrow' }, 'Scan complete', h('span', { class: 'eyebrow-meta' }, () => ` · ${duration(S.scan().summary?.elapsedMs)}`)),
    h('h1', { class: 'hero-title' }, 'Already spotless'),
    h('p', { class: 'hero-sub' }, () => `Vexus checked ${count(S.scan().summary?.entries ?? 0)} items and found nothing worth cleaning. Check back after a while.`),
    h('div', { class: 'hero-actions' }, Button({ label: 'Scan again', icon: 'refresh', size: 'lg', kind: 'primary', onClick: () => startScan() })),
  ];

  const cleaningBody = () => {
    const pr = () => S.clean().progress;
    const totals = () => S.clean().totals;
    return [
      h('div', { class: 'hero-eyebrow' }, 'Cleaning'),
      Figure({ value: () => pr()?.bytesDone ?? 0 }),
      h('p', { class: 'hero-sub' }, () => `of ${bytes(totals().bytesTotal)} · ${count(pr()?.filesDone ?? 0)} of ${count(totals().filesTotal)} files`),
      h('div', { class: 'progress', style: { '--v': () => pr()?.fraction ?? 0 } }, h('i')),
      h(
        'div',
        { class: 'hero-actions' },
        Button({ label: 'Stop', onClick: cancelJob }),
        h('span', { class: 'muted hero-note' }, () => (pr()?.etaMs != null ? etaText(pr().etaMs) : 'Re-checking every file as it goes')),
      ),
    ];
  };

  const doneBody = () => {
    const lc = () => S.lastClean();
    const r = () => lc().report;
    const days = () => S.settings()?.undoDays ?? 7;
    const skipped = () => Object.values(r().skipped).reduce((n, s) => n + s.files, 0);
    return [
      h('div', { class: 'hero-eyebrow' }, 'Done', h('span', { class: 'eyebrow-meta' }, () => ` · ${duration(r().elapsedMs)}`)),
      Figure({ value: () => r().bytes }),
      h('p', { class: 'hero-sub' }, () => {
        const bits = [];
        if (r().permanentBytes) bits.push(`${bytes(r().permanentBytes)} freed`);
        if (r().undoBytes) bits.push(`${bytes(r().undoBytes)} kept for undo (${plural(days(), 'day')})`);
        if (!bits.length) bits.push('nothing was removed');
        return bits.join(' · ');
      }),
      () => {
        const { freeBefore, freeAfter } = lc();
        const d = systemDisk();
        if (freeBefore == null || freeAfter == null || !d) return null;
        const delta = freeAfter - freeBefore;
        return h('div', { class: 'hero-path' }, `Free space on ${d.root.replace('\\', '')} ${bytes(freeBefore)} → ${bytes(freeAfter)}${delta > 0 ? ` (+${bytes(delta)})` : ''}`);
      },
      () =>
        skipped() > 0
          ? h('div', { class: 'hero-note-row' }, icon('info', { size: 15 }), h('span', null, `${plural(skipped(), 'file')} were in use or had changed, so Vexus left them alone.`))
          : null,
      h(
        'div',
        { class: 'hero-actions' },
        Button({ label: 'Scan again', icon: 'refresh', kind: 'primary', size: 'lg', onClick: () => startScan() }),
        () => (r().batch ? Button({ label: 'Undo', icon: 'undo', size: 'lg', onClick: () => restoreBatch(r().batch.id) }) : null),
      ),
    ];
  };

  const errorBody = () => {
    const e = () => S.scan().error ?? S.clean().error;
    return [
      h('div', { class: 'hero-eyebrow' }, 'Something went wrong'),
      h('h1', { class: 'hero-title' }, () => e()?.title ?? 'That didn’t work'),
      h('p', { class: 'hero-sub' }, () => e()?.detail ?? ''),
      h('div', { class: 'hero-actions' }, Button({ label: 'Try again', icon: 'refresh', kind: 'primary', size: 'lg', onClick: () => startScan() })),
    ];
  };

  const bodies = { idle: idleBody, scanning: scanningBody, ready: readyBody, empty: emptyBody, cleaning: cleaningBody, done: doneBody, error: errorBody };

  return h(
    'section',
    { class: 'card hero spot rise', 'data-state': state, 'aria-label': 'Status' },
    Orb({ state, progress: orbProgress }),
    // The body is rebuilt only when the *state* changes, never on a progress tick: everything
    // inside a body that changes over time is its own reactive child.
    h('div', { class: 'hero-body' }, () => {
      const s = state();
      return untrack(() => bodies[s]());
    }),
  );
}

// ─── cards ──────────────────────────────────────────────────────────────────────────

function FoundCard() {
  const view = signal('chart');
  const rows = computed(() =>
    found().map(({ meta, result }) => ({
      id: meta.id,
      label: meta.name,
      sub: `${plural(result.stats.items, 'item')}`,
      items: result.stats.items,
      value: result.stats.bytes,
      detail: [
        ['Items', count(result.stats.items)],
        ['Files', count(result.stats.files)],
        ...(result.stats.keptFiles ? [['Left alone (recent)', `${count(result.stats.keptFiles)} · ${bytes(result.stats.keptBytes)}`]] : []),
        ...(result.stats.busyFiles ? [['In use', count(result.stats.busyFiles)]] : []),
      ],
    })),
  );
  const locked = computed(() => results_locked());
  const scanned = () => S.scan().status === 'ready';

  // Rows appear as each category finishes, so results are progressive while a scan runs.
  const body = () => {
    if (!rows().length) {
      const status = S.scan().status;
      if (status === 'ready') {
        return h('div', { class: 'empty compact' }, icon('circle-check', { size: 28, className: 'muted' }), h('h3', null, 'Nothing to clean'), h('p', null, 'Every category came back empty.'));
      }
      const busy = status === 'scanning';
      return h(
        'div',
        { class: 'empty compact' },
        icon(busy ? 'search' : 'bar-chart', { size: 28, className: 'muted' }),
        h('h3', null, busy ? 'Looking…' : 'Nothing scanned yet'),
        h('p', null, busy ? 'Results appear here as each category finishes.' : 'After a scan, every category is ranked here with exactly how much it holds.'),
      );
    }
    return view() === 'table'
      ? DataTable({
          columns: [
            { label: 'Category', cell: (r) => r.label },
            { label: 'Items', align: 'right', cell: (r) => count(r.items) },
            { label: 'Size', align: 'right', cell: (r) => bytes(r.value) },
          ],
          rows,
        })
      : RankBars({ rows, ariaLabel: 'Reclaimable space by category', onSelect: (r) => navigate('cleaner', { focus: r.id }) });
  };

  return h(
    'section',
    { class: 'card found-card rise', style: { '--i': 1 } },
    h(
      'div',
      { class: 'card-title' },
      icon('bar-chart', { size: 16 }),
      h('span', null, 'Where it comes from'),
      h('span', { class: 'spacer' }),
      () =>
        rows().length
          ? h(
              'div',
              { class: 'seg', role: 'group', 'aria-label': 'View' },
              h('button', { type: 'button', 'aria-pressed': () => String(view() === 'chart'), onClick: () => view.set('chart') }, icon('bar-chart', { size: 14 }), 'Chart'),
              h('button', { type: 'button', 'aria-pressed': () => String(view() === 'table'), onClick: () => view.set('table') }, icon('table', { size: 14 }), 'Table'),
            )
          : null,
    ),
    h('div', { class: () => `found-body${S.scan().status === 'scanning' ? ' stale' : ''}` }, body),
    () =>
      scanned() && !elevated() && locked() > 0
        ? h(
            'div',
            { class: 'found-note' },
            icon('lock', { size: 15 }),
            h('span', null, `${plural(locked(), 'area')} can only be scanned as administrator. `, h('button', { type: 'button', class: 'foot-link', onClick: askToElevate }, 'Unlock')),
          )
        : null,
  );
}

function results_locked() {
  return (S.scan().summary?.categories ?? []).filter((c) => c.status === 'needsAdmin').length;
}

function severity(freeFraction) {
  if (freeFraction < 0.05) return { sev: 'critical', label: 'Almost full', icon: 'circle-alert' };
  if (freeFraction < 0.1) return { sev: 'serious', label: 'Low space', icon: 'alert' };
  if (freeFraction < 0.15) return { sev: 'warn', label: 'Getting full', icon: 'alert' };
  return null;
}

function DiskRow(d) {
  const used = () => 1 - d().free / d().total;
  const sev = () => severity(d().free / d().total);
  const meter = Meter({ value: used, severity: () => sev()?.sev, label: () => `Used space on ${d().root}`, thick: true });
  tooltip(meter, () => ({
    value: `${bytes(d().free)} free`,
    label: `${d().root.replace('\\', '')}${d().label ? ` · ${d().label}` : ''}`,
    rows: [['Used', bytes(d().total - d().free)], ['Capacity', bytes(d().total)], ['Free', percent(d().free / d().total, 1)]],
  }));
  return h(
    'div',
    { class: () => `disk-row${d().system ? ' system' : ''}` },
    h(
      'div',
      { class: 'disk-head' },
      icon('drive', { size: 16, className: 'muted' }),
      h('span', { class: 'disk-name' }, () => d().root.replace('\\', ''), () => (d().label ? h('span', { class: 'muted' }, ` ${d().label}`) : null)),
      () => (sev() ? h('span', { class: `status-flag ${sev().sev}` }, icon(sev().icon, { size: 13 }), sev().label) : null),
      h('span', { class: 'spacer' }),
      h('span', { class: 'disk-free num' }, () => `${bytes(d().free)} free`, h('span', { class: 'muted' }, () => ` of ${bytes(d().total)}`)),
    ),
    meter,
  );
}

function StorageCard() {
  return h(
    'section',
    { class: 'card rise', style: { '--i': 2 } },
    h('div', { class: 'card-title' }, icon('drive', { size: 16 }), h('span', null, 'Storage')),
    h(
      'div',
      { class: 'disk-list' },
      each(
        () => [...S.sys().disks].sort((a, b) => Number(b.system) - Number(a.system)),
        (d) => d.root,
        (item) => DiskRow(item),
      ),
    ),
  );
}

function MemoryCard() {
  const mem = () => S.sys().memory;
  const load = () => mem()?.loadPercent ?? 0;
  const sev = () => (load() >= 90 ? { sev: 'critical', label: 'Very high', icon: 'circle-alert' } : load() >= 80 ? { sev: 'warn', label: 'High', icon: 'alert' } : null);
  return h(
    'section',
    { class: 'card rise', style: { '--i': 3 } },
    h('div', { class: 'card-title' }, icon('memory', { size: 16 }), h('span', null, 'Memory')),
    h(
      'div',
      { class: 'mem' },
      h(
        'div',
        { class: 'mem-main' },
        h('div', { class: 'stat-value big' }, () => `${load()}%`, h('span', { class: 'stat-unit' }, ' in use')),
        h('div', { class: 'muted num' }, () => (mem() ? `${bytes(mem().total - mem().available)} of ${bytes(mem().total)}` : '')),
        () => (sev() ? h('span', { class: `status-flag ${sev().sev}` }, icon(sev().icon, { size: 13 }), `${sev().label} memory use`) : null),
      ),
      Sparkline({ values: () => S.memHistory(), max: 100, label: 'Memory in use', format: (v) => `${Math.round(v)}% in use` }),
    ),
  );
}

function SafetyCard() {
  const fact = (iconName, title, value, note, onClick) =>
    h(
      onClick ? 'button' : 'div',
      { class: `fact${onClick ? ' fact-link' : ''}`, type: onClick ? 'button' : null, onClick },
      h('div', { class: 'fact-icon' }, icon(iconName)),
      h('div', null, h('div', { class: 'fact-title' }, title), h('div', { class: 'fact-value' }, value), h('div', { class: 'fact-note' }, note)),
    );
  return h(
    'section',
    { class: 'card safety rise', style: { '--i': 4 } },
    h(
      'div',
      { class: 'facts' },
      fact('undo', 'Undo window', () => plural(S.settings()?.undoDays ?? 7, 'day'), 'Anything Vexus can’t rebuild waits in Recovery this long.'),
      fact('recovery', 'In Recovery now', () => (S.recovery().bytes ? bytes(S.recovery().bytes) : 'Nothing'), () => (S.recovery().count ? `${plural(S.recovery().count, 'batch', 'batches')} · open Recovery` : 'Cleaned items you can restore appear here.'), () => navigate('recovery')),
      fact('wifi-off', 'Private by design', 'Offline', 'No network access, no telemetry. Logs stay on this PC.'),
    ),
  );
}

export function OverviewView() {
  return h(
    'div',
    { class: 'page', role: 'region', 'aria-label': 'Overview' },
    h('div', { class: 'page-inner' }, Hero(), h('div', { class: 'dash-grid' }, FoundCard(), h('div', { class: 'dash-col' }, StorageCard(), MemoryCard())), SafetyCard()),
  );
}
