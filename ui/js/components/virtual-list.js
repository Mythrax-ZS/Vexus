// A windowed list for results that can run to hundreds of thousands of rows: only the rows in
// view exist in the DOM, and data arrives from the backend a page at a time.

import { h } from '../lib/dom.js';
import { onCleanup } from '../lib/signal.js';

export function VirtualList({ rowHeight = 52, pageSize = 80, overscan = 6, total, fetchPage, renderRow, ariaLabel }) {
  const spacer = h('div', { class: 'vlist-spacer' });
  const el = h('div', { class: 'vlist', role: 'list', 'aria-label': ariaLabel, tabindex: '-1' }, spacer);
  el.style.setProperty('--rh', `${rowHeight}px`);

  const pages = new Map(); // page index → items | 'loading'
  const rows = new Map(); // row index → node
  let generation = 0;
  let queued = false;

  const ensurePage = (p) => {
    if (pages.has(p)) return;
    pages.set(p, 'loading');
    const g = generation;
    fetchPage(p * pageSize, pageSize)
      .then((items) => {
        if (g !== generation) return;
        pages.set(p, items);
        paint();
      })
      .catch((e) => {
        // Never swallow this: a silent failure here looks like an empty list.
        console.error(`VirtualList page ${p} failed:`, e?.stack || e?.message || e);
        if (g === generation) pages.delete(p);
      });
  };

  function paint() {
    queued = false;
    const t = total();
    spacer.style.height = `${t * rowHeight}px`;
    const top = el.scrollTop;
    const first = Math.max(0, Math.floor(top / rowHeight) - overscan);
    const last = Math.min(t - 1, Math.ceil((top + el.clientHeight) / rowHeight) + overscan);

    for (const [i, node] of rows) {
      if (i < first || i > last) {
        node.remove();
        rows.delete(i);
      }
    }
    for (let i = first; i <= last; i++) {
      const p = Math.floor(i / pageSize);
      ensurePage(p);
      const page = pages.get(p);
      const item = Array.isArray(page) ? page[i % pageSize] : null;
      let node = rows.get(i);
      if (item && (!node || node.dataset.placeholder)) {
        node?.remove();
        node = renderRow(item, i);
        node.classList.add('vlist-row');
        node.style.setProperty('--y', `${i * rowHeight}px`);
        rows.set(i, node);
        spacer.appendChild(node);
      } else if (!item && !node) {
        node = h('div', { class: 'vlist-row placeholder', 'data-placeholder': '1' });
        node.style.setProperty('--y', `${i * rowHeight}px`);
        rows.set(i, node);
        spacer.appendChild(node);
      }
    }
  }

  const schedule = () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(paint);
  };
  el.addEventListener('scroll', schedule, { passive: true });
  const ro = new ResizeObserver(schedule);
  ro.observe(el);
  onCleanup(() => ro.disconnect());

  return {
    el,
    /** Drop everything (the query or scan changed) and start from the top. */
    reset() {
      generation++;
      pages.clear();
      for (const n of rows.values()) n.remove();
      rows.clear();
      el.scrollTop = 0;
      paint();
    },
    /** Re-read the total and repaint (e.g. after a count change). */
    repaint: schedule,
  };
}
