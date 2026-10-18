// The "Clean now" confirmation: say exactly what will happen, then act on an explicit yes.

import { h } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { signal } from '../lib/signal.js';
import { bytes, plural } from '../lib/format.js';
import { S, catById } from '../core/store.js';
import { startClean } from '../core/actions.js';
import { openDialog } from '../components/dialog.js';
import { Button, Check, StrategyChip } from '../components/ui.js';

const MAX_ROWS = 6;

export async function startCleanFlow() {
  const plan = S.selection.peek().plan;
  if (!plan?.items) return;
  const days = S.settings.peek()?.undoDays ?? 7;
  const cats = catById.peek();
  const understood = signal(false);
  const irreversible = plan.irreversible.map((id) => cats[id]?.name ?? id);
  const needsAck = irreversible.length > 0;

  const rows = [...plan.categories].sort((a, b) => b.bytes - a.bytes);

  const confirmed = await openDialog({
    className: 'clean-confirm',
    label: 'Confirm cleaning',
    render: (close) => [
      h(
        'div',
        { class: 'dlg-body' },
        h('h2', { class: 'dlg-title' }, `Clean ${bytes(plan.bytes)}?`),
        h('p', { class: 'dlg-text' }, `${plural(plan.items, 'item')} in ${plural(plan.categories.length, 'category', 'categories')}. Vexus re-checks every file first and skips anything that changed or is in use.`),
        h(
          'ul',
          { class: 'confirm-list' },
          rows.slice(0, MAX_ROWS).map((p) =>
            h(
              'li',
              null,
              h('span', { class: 'confirm-name truncate' }, cats[p.id]?.name ?? p.id),
              StrategyChip(p.strategy, days),
              h('span', { class: 'confirm-size num' }, bytes(p.bytes)),
            ),
          ),
          rows.length > MAX_ROWS
            ? h('li', { class: 'muted' }, h('span', { class: 'confirm-name' }, `and ${plural(rows.length - MAX_ROWS, 'more category', 'more categories')}`), h('span'), h('span', { class: 'confirm-size num' }, bytes(rows.slice(MAX_ROWS).reduce((n, p) => n + p.bytes, 0))))
            : null,
        ),
        h(
          'div',
          { class: 'confirm-facts' },
          plan.permanentBytes
            ? h('p', null, icon('trash', { size: 16 }), h('span', null, h('strong', null, bytes(plan.permanentBytes)), ' is rebuilt automatically, so it is removed right away.'))
            : null,
          plan.undoBytes
            ? h('p', null, icon('undo', { size: 16 }), h('span', null, h('strong', null, bytes(plan.undoBytes)), ` goes to Recovery for ${plural(days, 'day')}. You can restore it any time.`))
            : null,
        ),
        needsAck
          ? h(
              'div',
              { class: 'confirm-warn', role: 'alert' },
              icon('alert', { size: 18 }),
              h(
                'div',
                null,
                h('strong', null, 'This can’t be undone'),
                h('p', null, `${irreversible.join(', ')} will be removed for good.`),
                h('label', { class: 'confirm-ack' }, Check({ checked: () => understood(), onChange: (v) => understood.set(v), label: 'I understand' }), h('span', null, 'I understand')),
              ),
            )
          : null,
      ),
      h(
        'div',
        { class: 'dlg-foot' },
        Button({ label: 'Cancel', onClick: () => close(false), autofocus: needsAck }),
        h(
          'button',
          {
            type: 'button',
            class: 'btn primary lg',
            autofocus: !needsAck,
            disabled: () => needsAck && !understood(),
            onClick: () => close(true),
          },
          icon('cleaner'),
          `Clean ${bytes(plan.bytes)}`,
        ),
      ),
    ],
  });

  if (confirmed) startClean({ acknowledged: needsAck });
}
