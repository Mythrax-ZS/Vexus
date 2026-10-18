// First-run welcome: three short, honest screens about how Vexus keeps you safe.

import { h } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { signal } from '../lib/signal.js';
import { saveSettings } from '../core/actions.js';
import { S } from '../core/store.js';
import { openDialog } from '../components/dialog.js';
import { Button } from '../components/ui.js';
import { AccentPicker } from './settings.js';
import { Orb } from '../components/orb.js';

const STEPS = [
  {
    icon: 'eye',
    title: 'You see everything first',
    text: 'A scan only looks. Before anything is removed you can open any category, read why it’s safe, and exclude individual files. Vexus then re-checks each file right before touching it.',
  },
  {
    icon: 'undo',
    title: 'Undo for what can’t be rebuilt',
    text: () =>
      `Caches that Windows rebuilds on its own are removed straight away. Everything else — logs, crash dumps, history — is moved to Recovery for ${S.settings()?.undoDays ?? 7} days first, so you can always bring it back.`,
  },
  {
    icon: 'wifi-off',
    title: 'Private, offline, transparent',
    text: 'No telemetry, no account and no network access. Everything Vexus does is written to a local log you can read. Pick an accent to make it yours.',
    accent: true,
  },
];

export function openOnboarding() {
  const step = signal(0);
  const last = STEPS.length - 1;

  return openDialog({
    className: 'welcome',
    label: 'Welcome to Vexus',
    dismissible: true,
    render: (close) => {
      const finish = () => {
        saveSettings({ onboarded: true });
        close(true);
      };
      const body = () => {
        const s = STEPS[step()];
        return h(
          'div',
          { class: 'welcome-step rise' },
          h('div', { class: 'welcome-icon' }, icon(s.icon, { size: 30 })),
          h('h2', { class: 'dlg-title' }, s.title),
          h('p', { class: 'dlg-text' }, typeof s.text === 'function' ? s.text : s.text),
          s.accent ? h('div', { class: 'welcome-accent' }, AccentPicker()) : null,
        );
      };
      return [
        h('div', { class: 'welcome-hero' }, Orb({ state: () => 'ready', size: 'sm' }), h('div', { class: 'welcome-brand' }, 'Welcome to Vexus')),
        h('div', { class: 'dlg-body' }, body),
        h(
          'div',
          { class: 'dlg-foot' },
          Button({ label: 'Skip', kind: 'ghost', onClick: finish }),
          h('div', { class: 'dots', role: 'presentation' }, STEPS.map((_, i) => h('i', { class: () => (i === step() ? 'on' : '') }))),
          h('span', { class: 'grow' }),
          () => (step() > 0 ? Button({ label: 'Back', onClick: () => step.update((n) => n - 1) }) : null),
          () =>
            step() < last
              ? Button({ label: 'Next', icon: 'arrow-right', kind: 'primary', onClick: () => step.update((n) => n + 1) })
              : Button({ label: 'Get started', icon: 'check', kind: 'primary', onClick: finish }),
        ),
      ];
    },
  }).then((v) => {
    // Closing with Esc counts as "skip": never nag twice.
    if (!S.settings.peek()?.onboarded) saveSettings({ onboarded: true });
    return v;
  });
}
