// Chart marks. Built to the project's data-viz rules: thin marks (≤ 24 px), a square baseline
// with a 4 px rounded data end, 2 px lines, ≥ 8 px end dots with a 2 px surface ring, one hue
// per series, direct values, a hover/focus tooltip and a table-view twin for every chart.

import { h, each } from '../lib/dom.js';
import { effect } from '../lib/signal.js';
import { bytes, count, percent } from '../lib/format.js';
import { tooltip } from './tooltip.js';

/**
 * Ranked horizontal bars — "compare magnitude" with one series colour.
 * rows(): [{ id, label, sub?, value, detail? }]  (detail: [[label, value], …] for the tooltip)
 */
export function RankBars({ rows, onSelect, format = bytes, ariaLabel }) {
  const max = () => Math.max(1, ...rows().map((r) => r.value));
  const total = () => rows().reduce((n, r) => n + r.value, 0);

  return h(
    'ul',
    { class: 'rank', 'aria-label': ariaLabel },
    each(
      rows,
      (r) => r.id,
      (item, index) => {
        const btn = h(
          'button',
          { type: 'button', class: 'rank-row', onClick: () => onSelect?.(item()), 'aria-label': () => `${item().label}, ${format(item().value)}` },
          h('span', { class: 'rank-label' }, h('span', { class: 'truncate' }, () => item().label), h('small', { class: 'muted' }, () => item().sub ?? '')),
          h('span', { class: 'rank-track' }, h('span', { class: 'rank-bar', style: { '--w': () => (item().value / max()) * 100, '--i': () => index() } })),
          h('span', { class: 'rank-value num' }, () => format(item().value)),
        );
        tooltip(btn, () => {
          const r = item();
          return {
            value: format(r.value),
            label: r.label,
            rows: [['Share of total', percent(r.value / Math.max(1, total()), r.value / Math.max(1, total()) < 0.1 ? 1 : 0)], ...(r.detail ?? [])],
          };
        });
        return h('li', null, btn);
      },
    ),
  );
}

/** The accessible twin of a chart: the same numbers, as a table. */
export function DataTable({ columns, rows }) {
  return h(
    'table',
    { class: 'data-table' },
    h('thead', null, h('tr', null, columns.map((c) => h('th', { class: c.align === 'right' ? 'r' : '' }, c.label)))),
    h(
      'tbody',
      null,
      each(
        rows,
        (r) => r.id,
        (item) => h('tr', null, columns.map((c) => h('td', { class: c.align === 'right' ? 'r num' : '' }, () => c.cell(item())))),
      ),
    ),
  );
}

/**
 * A 2 px sparkline with a faint area wash, an end dot and a crosshair tooltip.
 * values(): number[] in 0..max.
 */
export function Sparkline({ values, max = 100, format = (v) => `${Math.round(v)}%`, label = 'Value', since = 'ago' }) {
  const W = 100;
  const H = 32;
  const pad = 3;
  const area = h('path', { class: 'spark-area' });
  const line = h('path', { class: 'spark-line', 'vector-effect': 'non-scaling-stroke' });
  const cross = h('div', { class: 'spark-cross' });
  const dot = h('i', { class: 'spark-dot' });
  const svg = h('svg', { class: 'spark-svg', viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: 'none', 'aria-hidden': 'true' }, area, line);
  const el = h('div', { class: 'spark', role: 'img', 'aria-label': () => `${label}: ${values().length ? format(values().at(-1)) : 'no data yet'}` }, svg, cross, dot);

  const point = (v, i, n) => [n <= 1 ? W : (i / (n - 1)) * W, H - pad - (Math.min(max, Math.max(0, v)) / max) * (H - pad * 2)];

  effect(() => {
    const v = values();
    const n = v.length;
    if (n < 2) {
      line.setAttribute('d', '');
      area.setAttribute('d', '');
      dot.style.opacity = '0';
      return;
    }
    const pts = v.map((val, i) => point(val, i, n));
    const d = pts.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(2)} ${y.toFixed(2)}`).join(' ');
    line.setAttribute('d', d);
    area.setAttribute('d', `${d} L${W} ${H} L0 ${H} Z`);
    const [lx, ly] = pts[n - 1];
    dot.style.opacity = '1';
    dot.style.setProperty('--x', `${(lx / W) * 100}%`);
    dot.style.setProperty('--y', `${(ly / H) * 100}%`);
  });

  let hover = -1;
  tooltip(
    el,
    (p) => {
      const v = values();
      if (v.length < 2) return null;
      const r = el.getBoundingClientRect();
      hover = Math.max(0, Math.min(v.length - 1, Math.round(((p.x - r.left) / r.width) * (v.length - 1))));
      cross.style.opacity = '1';
      cross.style.setProperty('--x', `${(hover / (v.length - 1)) * 100}%`);
      const secs = Math.round((v.length - 1 - hover) * 2.5);
      return { value: format(v[hover]), label: secs === 0 ? 'now' : `${secs} s ${since}` };
    },
    { delay: 0, live: true },
  );
  el.addEventListener('pointerleave', () => (cross.style.opacity = '0'));
  return el;
}

/** Compact stat tile: label, value, optional note. */
export function Stat({ label, value, note }) {
  return h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, label), h('div', { class: 'stat-value' }, value), note ? h('div', { class: 'stat-note' }, note) : null);
}

export { count };
