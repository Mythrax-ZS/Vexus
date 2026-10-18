// Boot sequence: ask the backend for the first-paint data, build the shell, then tell Rust the UI
// is ready so the (initially hidden) window can appear without a white flash.

import { installErrorReporting } from './core/log.js';
import { h, mount } from './lib/dom.js';
import { effect, flush, untrack } from './lib/signal.js';
import { icon } from './lib/icons.js';
import { inTauri, invoke, onAppEvent, AppError } from './core/bridge.js';
import { S } from './core/store.js';
import { applyTheme } from './core/theme.js';
import { announcement, quit, refreshRecovery, startScan, startSystemPolling } from './core/actions.js';
import { handleShortcut } from './core/commands.js';
import { Titlebar } from './components/titlebar.js';
import { Sidebar } from './components/sidebar.js';
import { ToastHost } from './components/toast.js';
import { Palette } from './components/palette.js';
import { openOnboarding } from './views/onboarding.js';
import { OverviewView } from './views/dashboard.js';
import { CleanerView } from './views/cleaner.js';
import { RecoveryView } from './views/recovery.js';
import { SettingsView } from './views/settings.js';

installErrorReporting();

const VIEWS = { overview: OverviewView, cleaner: CleanerView, recovery: RecoveryView, settings: SettingsView };

function fatal(error) {
  const e = error instanceof AppError ? error : new AppError({ detail: String(error) });
  document.getElementById('app').replaceChildren(
    h(
      'div',
      { class: 'fatal' },
      icon('circle-alert', { size: 40 }),
      h('h1', null, e.title),
      h('p', null, e.detail || 'Vexus could not start.'),
      inTauri ? h('button', { type: 'button', class: 'btn primary', onClick: quit }, 'Quit') : null,
    ),
  );
  if (inTauri) invoke('app_ready').catch(() => {});
}

function mountShell() {
  const main = h('main', { class: 'main', id: 'main' });
  const shell = h('div', { class: 'shell' }, Titlebar(), Sidebar(), main);
  document.getElementById('app').replaceChildren(shell);

  // One view at a time. `mount` owns each view's effects and listeners, so switching pages
  // tears the old one down completely.
  effect(() => {
    const view = S.view();
    untrack(() => mount(main, () => VIEWS[view]()));
  });

  const overlay = document.getElementById('overlay-root');
  overlay.append(ToastHost(), Palette(), h('div', { class: 'sr-only', 'aria-live': 'polite', role: 'status' }, () => announcement()));
}

// Things a web page does by default that make no sense in an app (and a few that are unsafe).
function installAppBehaviour() {
  const blocked = new Set(['r', 'p', 's', 'u', 'g', 'f', 'o', 'j', 'l', 'd', 'h']);
  document.addEventListener('keydown', (e) => {
    if (handleShortcut(e)) {
      e.preventDefault();
      return;
    }
    const ctrl = e.ctrlKey || e.metaKey;
    if (e.key === 'F5' || e.key === 'F7' || (ctrl && blocked.has(e.key.toLowerCase()) && !e.shiftKey && !e.altKey)) e.preventDefault();
  });
  document.addEventListener('contextmenu', (e) => {
    if (!e.target.closest?.('input, textarea, .selectable')) e.preventDefault();
  });
  // Dropping a file on the window must never navigate to it.
  for (const type of ['dragover', 'drop']) window.addEventListener(type, (e) => e.preventDefault());
  window.addEventListener('wheel', (e) => e.ctrlKey && e.preventDefault(), { passive: false });

  // Pointer-following spotlight on cards (sets --mx/--my; purely cosmetic).
  let raf = 0;
  document.addEventListener('pointermove', (e) => {
    const card = e.target.closest?.('.spot');
    if (!card || raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      const r = card.getBoundingClientRect();
      card.style.setProperty('--mx', `${e.clientX - r.left}px`);
      card.style.setProperty('--my', `${e.clientY - r.top}px`);
    });
  });
}

async function boot() {
  let info;
  try {
    info = await invoke('bootstrap');
  } catch (e) {
    return fatal(e);
  }
  S.boot.set(info);
  S.settings.set(info.settings);
  S.categories.set(info.categories);
  S.groups.set(info.groups);
  S.sys.set({ disks: info.disks, memory: info.memory });
  if (info.memory) S.memHistory.set([info.memory.loadPercent]);
  S.totals.set(info.totals);
  S.recovery.set({ batches: [], bytes: info.recovery.bytes, count: info.recovery.batches, loaded: false });
  applyTheme(info.settings);

  installAppBehaviour();
  mountShell();
  flush();
  startSystemPolling();
  refreshRecovery();

  // Tray menu → "Smart Scan".
  onAppEvent('app:command', (cmd) => {
    if (cmd === 'scan') startScan();
  });

  // Two frames: the first paint has happened before the window is revealed.
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      invoke('app_ready').catch(() => {});
      if (!info.settings.onboarded) openOnboarding();
    }),
  );
}

boot();
