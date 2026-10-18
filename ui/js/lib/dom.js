// DOM construction helpers on top of the signal runtime.
//
//   h('button', { class: 'btn', onClick: save }, 'Save')
//   h('span', { class: () => (on() ? 'on' : 'off') }, () => `${count()} items`)
//   h('ul', null, each(items, (it) => it.id, (item) => h('li', null, () => item().name)))
//
// Function-valued props and children are *reactive*: they re-run when the signals they read
// change, updating only that attribute/text node. Static strings are always set via textContent
// or attributes — never innerHTML — so untrusted names (file paths!) cannot inject markup.

import { effect, onCleanup, root, signal, getOwner } from './signal.js';

const SVG_NS = 'http://www.w3.org/2000/svg';
const SVG_TAGS = new Set(['svg', 'path', 'circle', 'rect', 'line', 'polyline', 'polygon', 'g', 'defs', 'linearGradient', 'stop', 'ellipse', 'text', 'clipPath', 'mask']);
const BOOLEAN_ATTRS = new Set(['disabled', 'hidden', 'checked', 'selected', 'readonly', 'required', 'open', 'autofocus', 'multiple']);

export function h(tag, props, ...children) {
  const el = SVG_TAGS.has(tag) ? document.createElementNS(SVG_NS, tag) : document.createElement(tag);
  if (props) for (const key of Object.keys(props)) applyProp(el, key, props[key]);
  appendChildren(el, children);
  return el;
}

function applyProp(el, key, value) {
  if (value == null || value === false) {
    if (key === 'ref' || key.startsWith('on')) return;
  }
  if (key === 'ref') {
    if (typeof value === 'function') value(el);
    return;
  }
  if (key.length > 2 && key.startsWith('on') && key[2] === key[2].toUpperCase()) {
    const [handler, opts] = Array.isArray(value) ? value : [value, undefined];
    if (typeof handler === 'function') {
      const type = key.slice(2).toLowerCase();
      el.addEventListener(type, handler, opts);
      onCleanup(() => el.removeEventListener(type, handler, opts));
    }
    return;
  }
  if (key === 'style' && value && typeof value === 'object') {
    // Per-property reactivity: `style: { '--v': () => progress() }` updates only that property.
    for (const name of Object.keys(value)) {
      const v = value[name];
      if (typeof v === 'function') effect(() => setStyle(el, name, v()));
      else setStyle(el, name, v);
    }
    return;
  }
  if (typeof value === 'function') {
    effect(() => setProp(el, key, value()));
  } else {
    setProp(el, key, value);
  }
}

function setStyle(el, name, v) {
  const prop = toKebab(name);
  if (v == null || v === false) el.style.removeProperty(prop);
  else el.style.setProperty(prop, String(v));
}

function setProp(el, key, value) {
  if (key === 'class' || key === 'className') {
    const cls = value == null || value === false ? '' : String(value);
    if (el instanceof SVGElement) el.setAttribute('class', cls);
    else el.className = cls;
    return;
  }
  if (key === 'style') {
    // CSS-in-JS via the CSSOM only: the page's CSP forbids style *attributes*. (Object values
    // are handled per property in applyProp; this branch serves a reactive whole-style prop.)
    if (typeof value === 'string') {
      el.style.cssText = value;
    } else if (value && typeof value === 'object') {
      for (const name of Object.keys(value)) setStyle(el, name, value[name]);
    }
    return;
  }
  if (key === 'value' || key === 'checked' || key === 'indeterminate') {
    if (el[key] !== value) el[key] = value;
    return;
  }
  if (BOOLEAN_ATTRS.has(key)) {
    if (value) el.setAttribute(key, '');
    else el.removeAttribute(key);
    if (key in el && !(el instanceof SVGElement)) el[key] = !!value;
    return;
  }
  if (value == null || value === false) el.removeAttribute(key);
  else el.setAttribute(key, value === true ? '' : String(value));
}

function toKebab(name) {
  if (name.startsWith('--') || name.includes('-')) return name;
  return name.replace(/[A-Z]/g, (m) => '-' + m.toLowerCase());
}

const EACH = Symbol('each');

/** Keyed list. `list` is a function returning an array; `render(item, index)` returns ONE node
 *  and receives `item` as a *function* returning the latest value for that key. */
export function each(list, key, render) {
  return { [EACH]: true, list, key, render };
}

/** Appends children (strings, numbers, nodes, arrays, reactive functions, `each()` regions). */
export function appendChildren(parent, children) {
  for (const child of children) appendChild(parent, child);
}

function appendChild(parent, child) {
  if (child == null || child === false || child === true) return;
  if (Array.isArray(child)) return appendChildren(parent, child);
  if (child instanceof Node) return void parent.appendChild(child);
  if (typeof child === 'function') return reactiveRegion(parent, child);
  if (typeof child === 'object' && child[EACH]) return keyedRegion(parent, child);
  parent.appendChild(document.createTextNode(String(child)));
}

function clearBetween(start, end) {
  while (start.nextSibling && start.nextSibling !== end) start.nextSibling.remove();
}

function reactiveRegion(parent, fn) {
  const start = document.createComment('');
  const end = document.createComment('');
  parent.append(start, end);
  let text = null;
  effect(() => {
    const value = fn();
    const primitive = value == null || value === false || value === true || (typeof value !== 'object' && typeof value !== 'function');
    if (primitive) {
      const s = value == null || value === false || value === true ? '' : String(value);
      if (text && text.parentNode === end.parentNode && text.nextSibling === end) {
        if (text.data !== s) text.data = s;
      } else {
        clearBetween(start, end);
        text = document.createTextNode(s);
        end.before(text);
      }
      return;
    }
    text = null;
    clearBetween(start, end);
    const frag = document.createDocumentFragment();
    appendChild(frag, value);
    end.before(frag);
  });
}

function keyedRegion(parent, spec) {
  const start = document.createComment('');
  const end = document.createComment('');
  parent.append(start, end);
  const outer = getOwner();
  let rows = new Map();

  outer?.add(() => {
    for (const row of rows.values()) row.dispose();
    rows = new Map();
  });

  effect(() => {
    const items = spec.list() || [];
    const next = new Map();
    const order = [];
    items.forEach((item, i) => {
      const k = spec.key(item, i);
      let row = rows.get(k);
      if (row) {
        row.item.set(item);
        row.index.set(i);
      } else {
        row = createRow(spec, item, i);
      }
      next.set(k, row);
      order.push(row);
    });
    for (const [k, row] of rows) {
      if (!next.has(k)) {
        row.node.remove();
        row.dispose();
      }
    }
    let cursor = start;
    for (const row of order) {
      if (cursor.nextSibling !== row.node) cursor.after(row.node);
      cursor = row.node;
    }
    rows = next;
  });
}

function createRow(spec, initial, i) {
  const item = signal(initial);
  const index = signal(i);
  let node;
  const dispose = root((dispose) => {
    node = spec.render(item, index);
    return dispose;
  }, { detached: true });
  return { node, dispose, item, index };
}

/** Renders `fn()` into `container`, replacing its content, inside an owned scope. */
export function mount(container, fn) {
  return root((dispose) => {
    container.replaceChildren();
    appendChild(container, fn());
    return () => {
      dispose();
      container.replaceChildren();
    };
  });
}

/** Conditional rendering sugar: `when(() => cond(), () => node, () => fallback)`. */
export function when(cond, yes, no) {
  return () => (cond() ? yes() : no ? no() : null);
}

/** Joins class names, skipping falsy values. */
export function cx(...parts) {
  return parts.filter(Boolean).join(' ');
}

/** Adds a global listener that is removed when the current scope is disposed. */
export function listen(target, type, handler, opts) {
  target.addEventListener(type, handler, opts);
  onCleanup(() => target.removeEventListener(type, handler, opts));
}
