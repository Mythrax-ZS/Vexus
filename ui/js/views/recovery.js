// Recovery: the undo. Everything Vexus removed that wasn't rebuilt automatically waits here.

import { h, each } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { computed, onCleanup } from '../lib/signal.js';
import { bytes, dateTime, ago, plural, until } from '../lib/format.js';
import { S, catById } from '../core/store.js';
import { openFolder, purgeBatch, refreshRecovery, restoreBatch } from '../core/actions.js';
import { confirmDialog } from '../components/dialog.js';
import { Button } from '../components/ui.js';
import { navigate } from '../core/actions.js';

function BatchCard(item) {
  const b = () => item();
  const names = () => b().categories.map((id) => catById()[id]?.name ?? id);
  const busy = () => b().busy;

  async function free() {
    const ok = await confirmDialog({
      title: `Free ${bytes(b().bytes)} now?`,
      text: 'The recovery copy will be deleted for good. After this you can’t restore these files.',
      confirmLabel: 'Free it now',
      danger: true,
    });
    if (ok) purgeBatch(b().id);
  }

  return h(
    'article',
    { class: () => `card batch spot${busy() ? ' busy' : ''}` },
    h('div', { class: 'batch-icon' }, icon('recovery', { size: 20 })),
    h(
      'div',
      { class: 'batch-main' },
      h('div', { class: 'batch-title' }, () => b().label, () => (b().complete ? null : h('span', { class: 'chip review' }, icon('alert'), 'Interrupted'))),
      h('div', { class: 'batch-meta muted' }, () => `${dateTime(b().createdAt)} · ${ago(b().createdAt)}`),
      h('div', { class: 'batch-cats' }, () => names().slice(0, 4).map((n) => h('span', { class: 'chip' }, n)), () => (names().length > 4 ? h('span', { class: 'chip plain' }, `+${names().length - 4}`) : null)),
    ),
    h(
      'div',
      { class: 'batch-stats' },
      h('div', { class: 'batch-size num' }, () => bytes(b().bytes)),
      h('div', { class: 'muted' }, () => `${plural(b().files, 'file')} · ${b().complete ? `frees itself ${until(b().expiresAt)}` : 'kept until you decide'}`),
    ),
    h(
      'div',
      { class: 'batch-actions' },
      () =>
        busy()
          ? h('span', { class: 'busy-note' }, h('span', { class: 'spinner' }), busy() === 'restoring' ? 'Restoring…' : 'Freeing…')
          : [
              Button({ label: 'Restore', icon: 'undo', kind: 'primary', onClick: () => restoreBatch(b().id) }),
              Button({ label: 'Free now', icon: 'trash', kind: 'ghost', onClick: free }),
            ],
    ),
  );
}

export function RecoveryView() {
  refreshRecovery();
  const batches = computed(() => S.recovery().batches);
  const total = computed(() => S.recovery().bytes);
  const next = computed(() => {
    const open = batches().filter((b) => b.complete);
    return open.length ? Math.min(...open.map((b) => b.expiresAt)) : null;
  });
  const days = () => S.settings()?.undoDays ?? 7;

  const refreshTimer = setInterval(() => refreshRecovery(), 30000);
  onCleanup(() => clearInterval(refreshTimer));

  // Only the *presence* of batches switches layouts; the cards themselves are a keyed list, so
  // the 30-second background refresh never rebuilds them.
  const has = computed(() => batches().length > 0);

  const summary = () =>
    has()
      ? h(
          'div',
          { class: 'recovery-summary rise' },
          h('div', null, h('div', { class: 'rs-value num' }, () => bytes(total())), h('div', { class: 'rs-label' }, 'held for undo')),
          h('div', null, h('div', { class: 'rs-value num' }, () => String(batches().length)), h('div', { class: 'rs-label' }, () => (batches().length === 1 ? 'batch' : 'batches'))),
          h('div', null, h('div', { class: 'rs-value' }, () => (next() ? until(next()) : '—')), h('div', { class: 'rs-label' }, 'next one frees itself')),
        )
      : null;

  return h(
    'div',
    { class: 'page', role: 'region', 'aria-label': 'Recovery' },
    h(
      'div',
      { class: 'page-inner' },
      h(
        'div',
        { class: 'page-head' },
        h('div', null, h('h1', { class: 'page-title' }, 'Recovery'), h('p', { class: 'page-sub' }, `Anything Vexus removes that isn’t rebuilt automatically waits here for ${plural(days(), 'day')}, so you can change your mind.`)),
        Button({ label: 'Open folder', icon: 'folder', onClick: () => openFolder('recovery') }),
      ),
      summary,
      () =>
        has()
          ? h('div', { class: 'batch-list' }, each(batches, (b) => b.id, (item) => BatchCard(item)))
          : h(
              'div',
              { class: 'card empty-card' },
              h('div', { class: 'empty' }, icon('shield-check', { size: 34, className: 'muted' }), h('h3', null, 'Nothing waiting here'), h('p', null, 'When Vexus removes something it can’t rebuild — old logs, crash dumps, history lists — a copy is kept here, so cleaning is never a one-way door.'), Button({ label: 'Go to Cleaner', icon: 'cleaner', onClick: () => navigate('cleaner') })),
            ),
    ),
  );
}
