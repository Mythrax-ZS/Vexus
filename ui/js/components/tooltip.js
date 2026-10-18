// A single shared tooltip. Content is built with textContent only (labels may be file names).
// It enhances, never gates: everything a tooltip shows is also available without it.

import { h } from '../lib/dom.js';
import { onCleanup } from '../lib/signal.js';

let el = null;
let hideTimer = 0;

function host() {
  if (!el) {
    el = h('div', { class: 'tip', role: 'tooltip' });
    document.getElementById('overlay-root').appendChild(el);
  }
  return el;
}

function fill(tip, content) {
  tip.replaceChildren();
  if (typeof content === 'string') {
    tip.textContent = content;
    return;
  }
  if (content instanceof Node) {
    tip.appendChild(content);
    return;
  }
  if (content.value != null) tip.appendChild(h('span', { class: 'tip-value' }, content.value));
  if (content.label != null) tip.appendChild(h('span', { class: 'tip-label' }, content.label));
  if (content.rows?.length) {
    const dl = h('dl', { class: 'tip-rows' });
    for (const [k, v] of content.rows) dl.append(h('dt', null, k), h('dd', null, v));
    tip.appendChild(dl);
  }
}

function place(tip, x, y, anchor) {
  const pad = 10;
  const { width, height } = tip.getBoundingClientRect();
  let left = x + 14;
  let top = y - height - 14;
  if (anchor) {
    left = anchor.left + anchor.width / 2 - width / 2;
    top = anchor.top - height - 10;
    if (top < pad) top = anchor.bottom + 10;
  }
  if (top < pad) top = y + 20;
  left = Math.max(pad, Math.min(left, window.innerWidth - width - pad));
  top = Math.max(pad, Math.min(top, window.innerHeight - height - pad));
  tip.style.left = `${left}px`;
  tip.style.top = `${top}px`;
}

export function hideTooltip() {
  clearTimeout(hideTimer);
  el?.classList.remove('show');
}

/**
 * Attaches a tooltip to `target`. `content` is a string, a node, `{ value, label, rows }`,
 * or a function returning any of those (evaluated on show, so it can be live).
 */
export function tooltip(target, content, { delay = 280, live = false } = {}) {
  let timer = 0;
  let pointer = { x: 0, y: 0 };

  const show = (anchor) => {
    const c = typeof content === 'function' ? content(pointer) : content;
    if (c == null || c === '') return;
    const tip = host();
    fill(tip, c);
    place(tip, pointer.x, pointer.y, anchor);
    tip.classList.add('show');
  };
  const onEnter = (e) => {
    pointer = { x: e.clientX, y: e.clientY };
    clearTimeout(timer);
    timer = setTimeout(() => show(null), delay);
  };
  const onMove = (e) => {
    pointer = { x: e.clientX, y: e.clientY };
    if (!el?.classList.contains('show')) return;
    // `live` tooltips (sparklines) re-read their content as the pointer moves between samples.
    if (live) show(null);
    else place(el, pointer.x, pointer.y, null);
  };
  const onLeave = () => {
    clearTimeout(timer);
    hideTooltip();
  };
  const onFocus = (e) => {
    if (e.target.matches?.(':focus-visible')) show(target.getBoundingClientRect());
  };

  target.addEventListener('pointerenter', onEnter);
  target.addEventListener('pointermove', onMove);
  target.addEventListener('pointerleave', onLeave);
  target.addEventListener('pointerdown', onLeave);
  target.addEventListener('focusin', onFocus);
  target.addEventListener('focusout', onLeave);
  onCleanup(() => {
    clearTimeout(timer);
    target.removeEventListener('pointerenter', onEnter);
    target.removeEventListener('pointermove', onMove);
    target.removeEventListener('pointerleave', onLeave);
    target.removeEventListener('pointerdown', onLeave);
    target.removeEventListener('focusin', onFocus);
    target.removeEventListener('focusout', onLeave);
    hideTooltip();
  });
  return target;
}
