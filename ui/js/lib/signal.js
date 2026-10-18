// A small fine-grained reactivity runtime: signals, effects, computed values and ownership
// scopes. ~100 lines, no dependencies — the whole UI is built on it.
//
//   const count = signal(0);
//   effect(() => console.log(count()));   // runs now, and again whenever count changes
//   count.set(1);                         // effects re-run on a microtask (coalesced)
//
// Anything created while an effect/root is running is *owned* by it and disposed when it re-runs
// or is disposed, so views clean up their listeners and sub-effects automatically.

let currentEffect = null;
let currentOwner = null;
const queue = new Set();
let flushScheduled = false;

class Owner {
  constructor(parent) {
    this.parent = parent;
    this.cleanups = [];
    this.disposed = false;
  }

  add(fn) {
    if (this.disposed) fn();
    else this.cleanups.push(fn);
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    const fns = this.cleanups.splice(0).reverse();
    for (const fn of fns) {
      try {
        fn();
      } catch (e) {
        reportError(e);
      }
    }
  }
}

function schedule(effect) {
  if (effect.disposed) return;
  queue.add(effect);
  if (!flushScheduled) {
    flushScheduled = true;
    queueMicrotask(flush);
  }
}

/** Runs pending effects now (normally they run on a microtask). */
export function flush() {
  flushScheduled = false;
  let rounds = 0;
  while (queue.size) {
    const batch = [...queue];
    queue.clear();
    for (const e of batch) if (!e.disposed) e.run();
    if (++rounds > 200) {
      console.error('Reactive update loop detected; stopping.');
      queue.clear();
      break;
    }
  }
}

export function signal(initial) {
  let value = initial;
  const subs = new Set();
  const read = () => {
    if (currentEffect) {
      subs.add(currentEffect);
      currentEffect.sources.add(subs);
    }
    return value;
  };
  read.peek = () => value;
  read.set = (next) => {
    if (Object.is(next, value)) return value;
    value = next;
    for (const e of [...subs]) schedule(e);
    return value;
  };
  read.update = (fn) => read.set(fn(value));
  /** Notify subscribers after mutating a value in place (arrays, sets, maps). */
  read.touch = () => {
    for (const e of [...subs]) schedule(e);
  };
  return read;
}

export function effect(fn) {
  const parent = currentOwner;
  const e = {
    sources: new Set(),
    disposed: false,
    owner: null,
    run() {
      for (const s of e.sources) s.delete(e);
      e.sources.clear();
      e.owner?.dispose();
      e.owner = new Owner(parent);
      const prevEffect = currentEffect;
      const prevOwner = currentOwner;
      currentEffect = e;
      currentOwner = e.owner;
      try {
        fn();
      } catch (err) {
        reportError(err);
      } finally {
        currentEffect = prevEffect;
        currentOwner = prevOwner;
      }
    },
    dispose() {
      if (e.disposed) return;
      e.disposed = true;
      for (const s of e.sources) s.delete(e);
      e.sources.clear();
      queue.delete(e);
      e.owner?.dispose();
    },
  };
  parent?.add(e.dispose);
  e.run();
  return e.dispose;
}

export function computed(fn) {
  const s = signal(undefined);
  effect(() => s.set(fn()));
  const read = () => s();
  read.peek = s.peek;
  return read;
}

/** Reads signals without subscribing the current effect to them. */
export function untrack(fn) {
  const prev = currentEffect;
  currentEffect = null;
  try {
    return fn();
  } finally {
    currentEffect = prev;
  }
}

/** Registers cleanup for the current owner (effect re-run, view unmount…). */
export function onCleanup(fn) {
  currentOwner?.add(fn);
}

/** Creates an independent ownership scope. `fn` receives a `dispose` function. */
export function root(fn, { detached = false } = {}) {
  const owner = new Owner(detached ? null : currentOwner);
  const prevOwner = currentOwner;
  const prevEffect = currentEffect;
  currentOwner = owner;
  currentEffect = null;
  const dispose = () => owner.dispose();
  if (!detached) prevOwner?.add(dispose);
  try {
    return fn(dispose);
  } finally {
    currentOwner = prevOwner;
    currentEffect = prevEffect;
  }
}

export function getOwner() {
  return currentOwner;
}

/** A signal that eases toward its source for count-up style animation. */
export function tween(source, { duration = 600, ease = (t) => 1 - Math.pow(1 - t, 3), reduced = () => false, from } = {}) {
  // `from` makes the first value count up from there (e.g. 0) instead of appearing instantly.
  const out = signal(from ?? (source.peek ? source.peek() : source()));
  let raf = 0;
  effect(() => {
    const target = source();
    const from = out.peek();
    if (reduced() || from === target || typeof target !== 'number' || !Number.isFinite(from)) {
      out.set(target);
      return;
    }
    cancelAnimationFrame(raf);
    const t0 = performance.now();
    const step = (now) => {
      const t = Math.min(1, (now - t0) / duration);
      out.set(from + (target - from) * ease(t));
      if (t < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    onCleanup(() => cancelAnimationFrame(raf));
  });
  return out;
}
