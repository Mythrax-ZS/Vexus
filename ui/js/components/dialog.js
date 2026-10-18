// Modal dialogs on the native <dialog> element: focus trapping, Esc to close and the top layer
// come for free. Each call returns a promise that resolves with the chosen value.

import { h } from '../lib/dom.js';
import { root } from '../lib/signal.js';
import { Button } from './ui.js';

/**
 * Opens a dialog. `render(close)` returns the body node(s); `close(value)` resolves the promise.
 * Resolves with `undefined` if dismissed (Esc / backdrop).
 */
export function openDialog({ className = '', render, dismissible = true, label }) {
  return new Promise((resolve) => {
    let result;
    const dlg = h('dialog', { class: `dlg ${className}`.trim(), 'aria-label': label });
    const dispose = root((dispose) => {
      const close = (value) => {
        result = value;
        dlg.close();
      };
      dlg.append(...[].concat(render(close)));
      return dispose;
    }, { detached: true });

    dlg.addEventListener('cancel', (e) => {
      if (!dismissible) e.preventDefault();
    });
    dlg.addEventListener('mousedown', (e) => {
      // A click on the backdrop (outside the dialog box) dismisses.
      if (dismissible && e.target === dlg) {
        const r = dlg.getBoundingClientRect();
        const inside = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
        if (!inside) dlg.close();
      }
    });
    dlg.addEventListener('close', () => {
      dispose();
      dlg.remove();
      resolve(result);
    });
    document.getElementById('overlay-root').appendChild(dlg);
    dlg.showModal();
  });
}

/**
 * A confirmation. Resolves true/false. For anything irreversible the focus starts on the safe
 * choice (Cancel), so a stray Enter can never confirm.
 */
export function confirmDialog({ title, text, body, confirmLabel = 'Continue', cancelLabel = 'Cancel', danger = false, className = '' }) {
  return openDialog({
    className,
    label: title,
    render: (close) => [
      h('div', { class: 'dlg-body' }, h('h2', { class: 'dlg-title' }, title), text ? h('p', { class: 'dlg-text' }, text) : null, body ?? null),
      h(
        'div',
        { class: 'dlg-foot' },
        Button({ label: cancelLabel, onClick: () => close(false), autofocus: danger }),
        Button({ label: confirmLabel, kind: danger ? 'danger' : 'primary', onClick: () => close(true), autofocus: !danger }),
      ),
    ],
  }).then((v) => v === true);
}
