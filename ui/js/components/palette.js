// Ctrl+K command palette.

import { h, each } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { computed, effect, signal } from '../lib/signal.js';
import { getCommands } from '../core/commands.js';
import { S } from '../core/store.js';

/** Higher is better; 0 means no match. Prefix > word start > substring > subsequence. */
export function score(command, query) {
  const q = query.trim().toLowerCase();
  if (!q) return 1;
  const title = command.title.toLowerCase();
  const hay = `${title} ${(command.keywords ?? '').toLowerCase()}`;
  if (title.startsWith(q)) return 100;
  if (title.split(/\s+/).some((w) => w.startsWith(q))) return 80;
  if (title.includes(q)) return 65;
  if (hay.includes(q)) return 45;
  // subsequence: every character of the query appears in order
  let i = 0;
  for (const ch of title) if (ch === q[i]) i++;
  return i === q.length ? 20 : 0;
}

export function Palette() {
  const query = signal('');
  const active = signal(0);
  const version = signal(0);
  const results = computed(() => {
    version();
    const q = query();
    return getCommands()
      .map((c) => [c, score(c, q)])
      .filter(([, s]) => s > 0)
      .sort((a, b) => b[1] - a[1])
      .map(([c]) => c);
  });

  const input = h('input', {
    class: 'palette-input',
    type: 'text',
    placeholder: 'Type a command…',
    spellcheck: 'false',
    autocomplete: 'off',
    role: 'combobox',
    'aria-expanded': 'true',
    'aria-controls': 'palette-list',
    'aria-label': 'Command',
    onInput: (e) => {
      query.set(e.target.value);
      active.set(0);
    },
  });

  const run = (cmd) => {
    S.paletteOpen.set(false);
    queueMicrotask(() => cmd.run());
  };

  const list = h(
    'ul',
    { class: 'palette-list', id: 'palette-list', role: 'listbox' },
    each(
      results,
      (c) => c.id,
      (item, index) =>
        h(
          'li',
          {
            role: 'option',
            class: () => `palette-row${active() === index() ? ' active' : ''}`,
            'aria-selected': () => String(active() === index()),
            onPointermove: () => active.set(index()),
            onClick: () => run(item()),
          },
          () => icon(item().icon ?? 'command', { className: 'palette-icon' }),
          h('span', { class: 'palette-text' }, h('span', { class: 'palette-title' }, () => item().title), h('small', { class: 'muted truncate' }, () => item().hint ?? '')),
          () => (item().shortcut ? h('span', { class: 'palette-keys' }, item().shortcut.split('+').map((k) => h('kbd', null, k))) : null),
        ),
    ),
  );

  const empty = h('div', { class: 'palette-empty' }, () => (results().length ? null : `No command matches “${query()}”`));

  const dlg = h(
    'dialog',
    {
      class: 'dlg palette',
      'aria-label': 'Command palette',
      onClose: () => S.paletteOpen.set(false),
      onKeydown: (e) => {
        const n = results.peek().length;
        if (e.key === 'ArrowDown' && n) {
          e.preventDefault();
          active.set((active.peek() + 1) % n);
        } else if (e.key === 'ArrowUp' && n) {
          e.preventDefault();
          active.set((active.peek() - 1 + n) % n);
        } else if (e.key === 'Enter') {
          e.preventDefault();
          const cmd = results.peek()[active.peek()];
          if (cmd) run(cmd);
        }
      },
      onMousedown: (e) => {
        if (e.target === dlg) dlg.close();
      },
    },
    h('div', { class: 'palette-head' }, icon('search', { className: 'palette-icon' }), input, h('kbd', null, 'Esc')),
    list,
    empty,
  );

  effect(() => {
    const open = S.paletteOpen();
    if (open && !dlg.open) {
      query.set('');
      active.set(0);
      input.value = '';
      version.update((v) => v + 1);
      dlg.showModal();
      input.focus();
    } else if (!open && dlg.open) {
      dlg.close();
    }
  });

  // Keep the active row in view while arrowing through a long list.
  effect(() => {
    const i = active();
    results();
    queueMicrotask(() => list.children[i]?.scrollIntoView?.({ block: 'nearest' }));
  });

  return dlg;
}
