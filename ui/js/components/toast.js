// Toast stack. State lives in S.toasts; this only renders it.

import { h, each } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { S } from '../core/store.js';
import { dismissToast } from '../core/actions.js';
import { Button } from './ui.js';

const ICONS = { good: 'circle-check', info: 'info', warn: 'alert', bad: 'circle-alert' };

export function ToastHost() {
  return h(
    'div',
    { class: 'toasts', 'aria-live': 'polite' },
    each(
      () => S.toasts(),
      (t) => t.id,
      (item) => {
        const t = item.peek();
        return h(
          'div',
          { class: () => `toast ${item().kind}${item().leaving ? ' leaving' : ''}`, role: 'status' },
          icon(ICONS[t.kind] ?? 'info', { className: 'lg' }),
          h(
            'div',
            null,
            h('div', { class: 'toast-title' }, t.title),
            t.text ? h('div', { class: 'toast-text' }, t.text) : null,
            t.actions.length
              ? h(
                  'div',
                  { class: 'toast-actions' },
                  t.actions.map((a) =>
                    Button({
                      label: a.label,
                      size: 'sm',
                      kind: a.primary === false ? '' : 'primary',
                      onClick: () => {
                        dismissToast(t.id);
                        a.run();
                      },
                    }),
                  ),
                )
              : null,
          ),
          Button({ label: 'Dismiss', icon: 'x', iconOnly: true, size: 'sm', kind: 'ghost', onClick: () => dismissToast(t.id) }),
        );
      },
    ),
  );
}
