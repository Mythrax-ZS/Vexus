// Left navigation and the always-visible safety status.

import { h } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { S, elevated } from '../core/store.js';
import { navigate, restartElevated } from '../core/actions.js';
import { confirmDialog } from './dialog.js';
import { plural } from '../lib/format.js';

const NAV = [
  { id: 'overview', label: 'Overview', icon: 'overview', key: '1' },
  { id: 'cleaner', label: 'Cleaner', icon: 'cleaner', key: '2' },
  { id: 'recovery', label: 'Recovery', icon: 'recovery', key: '3' },
];

export async function askToElevate() {
  const ok = await confirmDialog({
    title: 'Restart as administrator?',
    text: 'Some cleanable areas — Windows temp files, update caches, logs — can only be seen by administrators. Vexus will restart with elevated rights; Windows will ask you to confirm. Nothing is cleaned until you review and approve it, as always.',
    confirmLabel: 'Restart as administrator',
  });
  if (ok) restartElevated();
}

export function Sidebar() {
  const item = (n) =>
    h(
      'button',
      {
        type: 'button',
        class: 'nav-item',
        'aria-current': () => (S.view() === n.id ? 'page' : null),
        onClick: () => navigate(n.id),
      },
      icon(n.icon),
      h('span', { class: 'label' }, n.label),
      n.id === 'recovery' ? () => (S.recovery().count > 0 ? h('span', { class: 'nav-badge' }, String(S.recovery().count)) : null) : null,
    );

  const settingsItem = h(
    'button',
    { type: 'button', class: 'nav-item', 'aria-current': () => (S.view() === 'settings' ? 'page' : null), onClick: () => navigate('settings') },
    icon('settings'),
    h('span', { class: 'label' }, 'Settings'),
  );

  const undoDays = () => S.settings()?.undoDays ?? 7;
  const policy = () => S.settings()?.deletionPolicy ?? 'smart';

  return h(
    'nav',
    { class: 'sidebar', 'aria-label': 'Main' },
    ...NAV.map(item),
    h('div', { class: 'spacer' }),
    settingsItem,
    h(
      'div',
      { class: 'sidebar-foot' },
      h(
        'div',
        { class: 'foot-row ok' },
        icon('shield-check', { size: 16 }),
        h('span', null, () => (policy() === 'alwaysPermanent' ? 'Previews before cleaning' : `Undo for ${plural(undoDays(), 'day')}`)),
      ),
      h('div', { class: 'foot-row' }, icon('wifi-off', { size: 16 }), h('span', null, 'Offline by design')),
      () =>
        elevated()
          ? h('div', { class: 'foot-row ok' }, icon('lock', { size: 16 }), h('span', null, 'Administrator mode'))
          : h(
              'div',
              { class: 'foot-row warn' },
              icon('lock', { size: 16 }),
              h('div', { class: 'foot-stack' }, h('span', null, 'Standard access'), h('button', { type: 'button', class: 'foot-link', onClick: askToElevate }, 'Unlock system areas')),
            ),
    ),
  );
}
