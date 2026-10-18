// Custom title bar: brand, sandbox badge, command-palette entry point and window controls.

import { h } from '../lib/dom.js';
import { icon, logoMark } from '../lib/icons.js';
import { signal } from '../lib/signal.js';
import { appWindow } from '../core/bridge.js';
import { S, sandboxed } from '../core/store.js';
import { tooltip } from './tooltip.js';

export function Titlebar() {
  const win = appWindow();
  const maximized = signal(false);
  if (win) {
    const sync = () => win.isMaximized().then(maximized.set).catch(() => {});
    sync();
    win.onResized(sync).catch(() => {});
  }

  const winBtn = (name, label, action, extra = '') =>
    h('button', { type: 'button', class: `win-btn ${extra}`.trim(), 'aria-label': label, onClick: () => action?.().catch?.(() => {}) }, icon(name));

  const sandboxBadge = h('span', { class: 'badge-sandbox' }, 'Sandbox');
  tooltip(sandboxBadge, () => ({
    value: 'Sandbox mode',
    label: `Vexus is working on a fake profile, not this PC.\n${S.boot()?.dataDir ?? ''}`,
  }));

  return h(
    'header',
    { class: 'titlebar', 'data-tauri-drag-region': '' },
    h('div', { class: 'brand' }, logoMark(), h('span', null, 'Vexus'), () => (sandboxed() ? sandboxBadge : null)),
    h(
      'button',
      { type: 'button', class: 'search-pill', onClick: () => S.paletteOpen.set(true), 'aria-label': 'Search or run a command' },
      icon('search', { size: 15 }),
      h('span', { class: 'grow' }, 'Search or run a command'),
      h('kbd', null, 'Ctrl'),
      h('kbd', null, 'K'),
    ),
    win
      ? h(
          'div',
          { class: 'win-controls' },
          winBtn('win-minimize', 'Minimize', () => win.minimize()),
          h(
            'button',
            { type: 'button', class: 'win-btn', 'aria-label': () => (maximized() ? 'Restore' : 'Maximize'), onClick: () => win.toggleMaximize() },
            () => icon(maximized() ? 'win-restore' : 'win-maximize'),
          ),
          winBtn('win-close', 'Close', () => win.close(), 'close'),
        )
      : h('div'),
  );
}
