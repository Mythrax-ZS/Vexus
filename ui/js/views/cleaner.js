// Cleaner: every category with its explanation, a per-item preview you can exclude from, and a
// sticky bar that always shows exactly what "Clean" would do.

import { h } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { computed, effect, onCleanup, signal, untrack } from '../lib/signal.js';
import { ago, bytes, count, midEllipsis, plural } from '../lib/format.js';
import { invoke } from '../core/bridge.js';
import { S, catById, elevated, isBusy, results, scanId, scanStatus } from '../core/store.js';
import {
  clearExclusions, navigate, revealItem, saveSettings, selectDefaults, selectNone, setItemExcluded, startScan, toggleCategory,
} from '../core/actions.js';
import { askToElevate } from '../components/sidebar.js';
import { Button, Check, RiskChip, StrategyChip, Switch } from '../components/ui.js';
import { VirtualList } from '../components/virtual-list.js';
import { tooltip } from '../components/tooltip.js';
import { startCleanFlow } from './clean-flow.js';

const resultById = computed(() => Object.fromEntries(results().map((r) => [r.meta.id, r.result])));
const advanced = () => !!S.settings()?.advanced;

/** Categories shown in the list: the safe ones, or everything in advanced mode. */
const visibleCategories = computed(() => S.categories().filter((c) => advanced() || c.risk === 'safe' || c.id === S.focus()));
const hiddenCount = computed(() => S.categories().length - visibleCategories().length);

const focusId = computed(() => {
  const list = visibleCategories();
  const f = S.focus();
  if (f && list.some((c) => c.id === f)) return f;
  // Default focus: the biggest thing found, else the first category.
  const best = [...list].sort((a, b) => (resultById()[b.id]?.stats.bytes ?? 0) - (resultById()[a.id]?.stats.bytes ?? 0))[0];
  return best?.id ?? null;
});

function ageText(hours) {
  if (hours <= 0) return null;
  if (hours < 48) return `${hours} hours`;
  return `${Math.round(hours / 24)} days`;
}

// ─── left: category list ────────────────────────────────────────────────────────────

function CategoryRow(meta) {
  const res = () => resultById()[meta.id] ?? null;
  const status = () => res()?.status ?? null;
  const found = () => status() === 'found';
  const selected = () => S.selection().selected.has(meta.id);

  const sub = () => {
    const r = res();
    if (!r) return meta.summary;
    switch (r.status) {
      case 'found': {
        const bits = [plural(r.stats.items, 'item')];
        if (r.stats.keptFiles) bits.push(`${count(r.stats.keptFiles)} recent kept`);
        return bits.join(' · ');
      }
      case 'needsAdmin':
        return 'Needs administrator';
      case 'unavailable':
        return r.note ?? 'Unavailable';
      default:
        return r.stats.busyFiles ? `${plural(r.stats.busyFiles, 'file')} in use right now` : 'Nothing to clean';
    }
  };

  const activate = () => S.focus.set(meta.id);
  // Two real controls side by side (a checkbox and a button), never one inside the other, so
  // keyboard and screen-reader users get exactly two predictable stops per category.
  return h(
    'div',
    {
      class: () => `cat-row${focusId() === meta.id ? ' active' : ''}${res() && !found() ? ' dim' : ''}`,
      onClick: activate,
    },
    Check({ checked: selected, disabled: () => !found(), label: () => `Include ${meta.name}`, onChange: () => toggleCategory(meta.id) }),
    h(
      'button',
      { type: 'button', class: 'cat-hit', 'aria-current': () => (focusId() === meta.id ? 'true' : null), onClick: (e) => { e.stopPropagation(); activate(); } },
      h('span', { class: 'cat-icon' }, icon(meta.icon, { size: 18 })),
      h(
        'span',
        { class: 'cat-main' },
        h('span', { class: 'cat-name' }, h('span', { class: 'truncate' }, meta.name)),
        h('span', { class: 'cat-sub' }, () => (advanced() && meta.risk !== 'safe' ? RiskChip(meta.risk) : null), h('span', { class: 'truncate' }, sub)),
      ),
      h(
        'span',
        { class: 'cat-size num' },
        () => {
          const r = res();
          if (!r) return '';
          if (r.status === 'needsAdmin') return h('span', { class: 'chip lock' }, icon('lock'), 'Admin');
          return found() ? bytes(r.stats.bytes) : '—';
        },
      ),
    ),
  );
}

function CategoryList() {
  const sections = () =>
    S.groups().map((g) => {
      const metas = visibleCategories().filter((c) => c.group === g.id);
      if (!metas.length) return null;
      return h('section', { class: 'cat-group' }, h('h3', { class: 'cat-group-title' }, g.label), h('div', { class: 'cat-rows' }, metas.map(CategoryRow)));
    });
  return h(
    'div',
    { class: 'cat-list', role: 'group', 'aria-label': 'Categories' },
    sections,
    () =>
      hiddenCount() > 0
        ? h(
            'button',
            { type: 'button', class: 'cat-more', onClick: () => saveSettings({ advanced: true }) },
            icon('sliders', { size: 16 }),
            h('span', null, `Show ${plural(hiddenCount(), 'more cleaner')}`),
            h('small', { class: 'muted' }, 'Advanced: needs a closer look'),
          )
        : null,
  );
}

// ─── right: detail ──────────────────────────────────────────────────────────────────

function Explainer({ iconName, title, text }) {
  return h('div', { class: 'explainer' }, h('div', { class: 'explainer-head' }, icon(iconName, { size: 15 }), h('span', null, title)), h('p', null, text));
}

function StatPill(label, value, help) {
  const el = h('div', { class: 'stat-pill' }, h('span', { class: 'stat-pill-value num' }, value), h('span', { class: 'stat-pill-label' }, label));
  if (help) tooltip(el, { label: help });
  return el;
}

function ItemsPanel(meta) {
  const search = signal('');
  const sort = signal('sizeDesc');
  const total = signal(0);
  const excluded = signal(0);
  const inPlan = () => S.selection().selected.has(meta.id);
  const stats = () => resultById()[meta.id]?.stats;

  const list = VirtualList({
    rowHeight: 54,
    ariaLabel: `${meta.name} items`,
    total: () => total(),
    fetchPage: async (offset, limit) => {
      // The engine only holds a scan's items once the whole scan has finished.
      if (scanStatus.peek() !== 'ready' || scanId.peek() == null) return [];
      const page = await invoke('scan_items', { scanId: scanId.peek(), category: meta.id, query: { offset, limit, sort: sort.peek(), search: search.peek() || null } });
      if (offset === 0) {
        total.set(page.total);
        excluded.set(page.excluded);
        list.repaint();
      }
      return page.items;
    },
    renderRow: (it) => ItemRow(it),
  });

  function ItemRow(it) {
    const reveal = h('button', { type: 'button', class: 'btn ghost sm icon-only item-reveal', 'aria-label': `Show ${it.name} in Explorer`, onClick: () => revealItem(it.id) }, icon('external-link', { size: 15 }));
    tooltip(reveal, { label: it.kind === 'recycleBin' ? 'Open the Recycle Bin' : 'Show in Explorer' });
    const row = h(
      'div',
      { class: 'item-row', role: 'listitem' },
      Check({
        checked: !it.excluded,
        label: `Include ${it.name}`,
        onChange: (on) => {
          it.excluded = !on;
          excluded.update((n) => n + (on ? -1 : 1));
          setItemExcluded(it.id, !on);
        },
      }),
      icon(it.kind === 'dir' ? 'folder' : it.kind === 'recycleBin' ? 'trash' : 'file', { size: 18, className: 'item-icon' }),
      h(
        'div',
        { class: 'item-main' },
        h('div', { class: 'item-name truncate' }, it.name),
        h('div', { class: 'item-dir truncate' }, it.kind === 'dir' ? h('span', { class: 'item-count' }, `${plural(it.files, 'file')} · `) : null, midEllipsis(it.dir, 70)),
      ),
      h('div', { class: 'item-right' }, h('div', { class: 'item-size num' }, bytes(it.size)), h('div', { class: 'item-age muted' }, it.modified ? ago(it.modified) : '')),
      reveal,
    );
    tooltip(row, { label: it.path, value: undefined }, { delay: 700 });
    return row;
  }

  // Reload from the top whenever what we're looking at changes.
  let timer = 0;
  effect(() => {
    scanId();
    S.itemsVersion();
    const s = search();
    sort();
    untrack(() => {
      clearTimeout(timer);
      total.set(stats()?.items ?? 0);
      timer = setTimeout(() => list.reset(), s ? 160 : 0);
    });
  });
  onCleanup(() => clearTimeout(timer));

  const input = h('input', {
    class: 'input item-search',
    type: 'search',
    placeholder: 'Filter by name or folder',
    'aria-label': 'Filter items',
    spellcheck: 'false',
    onInput: (e) => search.set(e.target.value),
  });

  return h(
    'div',
    { class: () => `items-panel${inPlan() ? '' : ' off'}` },
    h(
      'div',
      { class: 'items-bar' },
      h('div', { class: 'items-title' }, 'Items', h('span', { class: 'muted num' }, () => ` ${count(total())}${search() ? ' match' : ''}`)),
      h('div', { class: 'items-tools' }, input,
        h('select', { class: 'select', 'aria-label': 'Sort items', onChange: (e) => sort.set(e.target.value) },
          h('option', { value: 'sizeDesc' }, 'Largest first'),
          h('option', { value: 'sizeAsc' }, 'Smallest first'),
          h('option', { value: 'oldest' }, 'Oldest first'),
          h('option', { value: 'newest' }, 'Newest first'),
          h('option', { value: 'nameAsc' }, 'Name A–Z'),
        ),
      ),
    ),
    () =>
      excluded() > 0
        ? h(
            'div',
            { class: 'items-note' },
            icon('info', { size: 15 }),
            h('span', null, `${plural(excluded(), 'item')} will be kept.`),
            h('button', { type: 'button', class: 'foot-link', onClick: async () => { await clearExclusions(meta.id); S.itemsVersion.update((v) => v + 1); } }, 'Include all'),
          )
        : null,
    () =>
      inPlan()
        ? null
        : h(
            'div',
            { class: 'items-note off' },
            icon('info', { size: 15 }),
            h('span', null, 'This category isn’t selected, so none of these will be touched.'),
            h('button', { type: 'button', class: 'foot-link', onClick: () => toggleCategory(meta.id) }, 'Select it'),
          ),
    list.el,
  );
}

function CategoryDetail() {
  return h('div', { class: 'detail' }, () => {
    const id = focusId();
    const meta = id ? catById()[id] : null;
    if (!meta) return h('div', { class: 'empty' }, icon('cleaner', { size: 30, className: 'muted' }), h('h3', null, 'Nothing to show'));
    return untrack(() => DetailBody(meta));
  });
}

function DetailBody(meta) {
  // A computed keeps the same object between progress ticks, so the pane isn't rebuilt on each one.
  const res = computed(() => resultById()[meta.id] ?? null);
  const days = () => S.settings()?.undoDays ?? 7;
  const policy = () => S.settings()?.deletionPolicy ?? 'smart';
  const strategy = () => (meta.strategyLocked ? meta.strategy : policy() === 'alwaysUndo' ? 'undoable' : policy() === 'alwaysPermanent' ? 'permanent' : meta.strategy);
  const ageHours = () => (meta.ageAdjustable ? S.settings()?.tempMinAgeHours ?? meta.minAgeHours : meta.minAgeHours);
  const scanned = () => scanStatus() === 'ready';

  const stats = () => res()?.stats;
  const statPills = () => {
    const s = stats();
    if (!s) return [];
    const pills = [StatPill('can be removed', bytes(s.bytes), `${count(s.files)} files in ${plural(s.items, 'item')}`)];
    if (s.keptFiles) pills.push(StatPill('left alone: too recent', `${plural(s.keptFiles, 'file')} · ${bytes(s.keptBytes)}`, `Files newer than ${ageText(ageHours()) ?? 'the age limit'} are never touched.`));
    if (s.busyFiles) pills.push(StatPill('in use right now', plural(s.busyFiles, 'file'), 'Open in another program, so they’re left alone.'));
    if (s.links) pills.push(StatPill('links skipped', count(s.links), 'Shortcuts to other folders are never followed or removed.'));
    if (s.denied) pills.push(StatPill('blocked by Windows', count(s.denied), 'Windows wouldn’t let Vexus look here without administrator rights.'));
    if (s.unreadable) pills.push(StatPill('couldn’t be read', count(s.unreadable), 'Folders nested unusually deep, or that changed or failed while being read, are left alone.'));
    return pills;
  };

  return h(
    'div',
    { class: 'detail-inner rise' },
    h(
      'header',
      { class: 'detail-head' },
      h('span', { class: 'detail-icon' }, icon(meta.icon, { size: 24 })),
      h(
        'div',
        { class: 'detail-titles' },
        h('h2', null, meta.name),
        h('p', { class: 'muted' }, meta.summary),
      ),
      h('div', { class: 'detail-chips' }, RiskChip(meta.risk), () => StrategyChip(strategy(), days()), meta.needsAdmin ? h('span', { class: 'chip lock' }, icon('lock'), 'Needs admin') : null),
    ),
    h(
      'div',
      { class: 'explainers' },
      Explainer({ iconName: 'info', title: 'What is this?', text: meta.detail.what }),
      Explainer({ iconName: 'undo', title: 'If you remove it', text: meta.detail.effect }),
      Explainer({
        iconName: 'shield-check',
        title: 'How Vexus keeps it safe',
        text: () => `${meta.detail.safety}${ageHours() > 0 ? ` Files newer than ${ageText(ageHours())} are never touched.` : ''}`,
      }),
    ),
    () => {
      const r = res();
      if (!scanned() && scanStatus() !== 'scanning') {
        return h('div', { class: 'detail-scan' }, icon('search', { size: 18 }), h('span', null, 'Scan to see how much this holds and exactly which items are involved.'), Button({ label: 'Smart Scan', icon: 'search', kind: 'primary', onClick: () => startScan() }));
      }
      if (!r) return h('div', { class: 'detail-scan' }, h('span', { class: 'spinner' }), h('span', null, 'Looking…'));
      if (r.status === 'needsAdmin') {
        return h('div', { class: 'detail-scan' }, icon('lock', { size: 18 }), h('span', null, 'Windows only lets administrators look here.'), elevated() ? null : Button({ label: 'Unlock system areas', icon: 'lock', onClick: askToElevate }));
      }
      if (r.status === 'unavailable') {
        return h('div', { class: 'detail-scan' }, icon('alert', { size: 18 }), h('span', null, r.note ?? 'This location is unavailable.'));
      }
      if (r.status === 'empty') {
        return h('div', { class: 'detail-scan' }, icon('circle-check', { size: 18 }), h('span', null, r.stats.busyFiles ? `${plural(r.stats.busyFiles, 'file')} here are in use right now, so there’s nothing safe to remove.` : 'Nothing to clean here right now.'));
      }
      return [
        h('div', { class: 'stat-pills' }, statPills()),
        scanned()
          ? untrack(() => ItemsPanel(meta))
          : h('div', { class: 'detail-scan' }, h('span', { class: 'spinner' }), h('span', null, 'The list of items appears as soon as the scan finishes.')),
      ];
    },
  );
}

// ─── bottom: what Clean will do ─────────────────────────────────────────────────────

function ActionBar() {
  const plan = () => S.selection().plan;
  const ready = () => S.scan().status === 'ready';
  return h(
    'footer',
    { class: 'actionbar' },
    h(
      'div',
      { class: 'ab-summary' },
      h('div', { class: 'ab-figure' }, () => (ready() ? bytes(plan()?.bytes ?? 0) : '—'), h('span', { class: 'ab-unit' }, ' selected')),
      h('div', { class: 'ab-detail muted' }, () => {
        if (!ready()) return S.scan().status === 'scanning' ? 'Scanning…' : 'Run a scan to build a plan.';
        const p = plan();
        if (!p?.items) return 'Nothing selected.';
        const bits = [`${plural(p.items, 'item')} in ${plural(p.categories.length, 'category', 'categories')}`];
        if (p.permanentBytes) bits.push(`${bytes(p.permanentBytes)} removed for good`);
        if (p.undoBytes) bits.push(`${bytes(p.undoBytes)} kept for undo`);
        return bits.join(' · ');
      }),
    ),
    () => (plan()?.highestRisk && plan().highestRisk !== 'safe' ? h('span', { class: `chip ${plan().highestRisk}` }, icon(plan().highestRisk === 'caution' ? 'alert' : 'eye'), plan().highestRisk === 'caution' ? 'Includes things that can’t be undone' : 'Includes items to review') : null),
    h('div', { class: 'ab-spacer' }),
    () =>
      ready()
        ? h(
            'button',
            { type: 'button', class: 'btn primary lg', disabled: () => !plan()?.items || isBusy(), onClick: startCleanFlow },
            icon('cleaner'),
            () => (plan()?.items ? `Clean ${bytes(plan().bytes)}` : 'Clean now'),
          )
        : Button({ label: S.scan().status === 'scanning' ? 'Scanning…' : 'Smart Scan', icon: 'search', kind: 'primary', size: 'lg', disabled: () => isBusy(), onClick: () => startScan() }),
  );
}

export function CleanerView() {
  return h(
    'div',
    { class: 'page flush', role: 'region', 'aria-label': 'Cleaner' },
    h(
      'div',
      { class: 'cleaner-head' },
      h('div', null, h('h1', { class: 'page-title' }, 'Cleaner'), h('p', { class: 'page-sub' }, () => (S.scan().status === 'ready' ? 'Review the plan, open any category, and exclude what you want to keep.' : 'Every cleaner explained. Scan to see sizes and exact items.'))),
      h(
        'div',
        { class: 'cleaner-tools' },
        h('label', { class: 'switch-row' }, h('span', null, 'Advanced'), Switch({ checked: advanced, label: 'Show advanced cleaners', onChange: (v) => saveSettings({ advanced: v }) })),
        () => (S.scan().status === 'ready' ? h('div', { class: 'seg', role: 'group', 'aria-label': 'Selection' }, h('button', { type: 'button', onClick: selectDefaults }, 'Recommended'), h('button', { type: 'button', onClick: selectNone }, 'None')) : null),
        Button({ label: () => (S.scan().status === 'ready' ? 'Scan again' : 'Smart Scan'), icon: 'refresh', disabled: () => isBusy(), onClick: () => startScan() }),
      ),
    ),
    h('div', { class: 'cleaner-body' }, CategoryList(), CategoryDetail()),
    ActionBar(),
  );
}
