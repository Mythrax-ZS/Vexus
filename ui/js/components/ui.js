// Small, stateless building blocks shared by every view.

import { h } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { tooltip } from './tooltip.js';

/** Custom checkbox. `checked` may be a value or a function (reactive). */
export function Check({ checked, disabled, onChange, label, className = '' }) {
  const input = h('input', {
    type: 'checkbox',
    checked,
    disabled,
    'aria-label': label,
    onChange: (e) => onChange?.(e.target.checked, e),
    onClick: (e) => e.stopPropagation(),
  });
  return h('label', { class: `check ${className}`.trim(), onClick: (e) => e.stopPropagation() }, input, h('span', { class: 'box' }, icon('check', { strokeWidth: 3.2 })));
}

export function Switch({ checked, onChange, label, disabled }) {
  return h(
    'span',
    { class: 'switch' },
    h('input', {
      type: 'checkbox',
      role: 'switch',
      checked,
      disabled,
      'aria-label': label,
      onChange: (e) => onChange?.(e.target.checked),
    }),
    h('span', { class: 'track' }),
    h('span', { class: 'thumb' }),
  );
}

const RISK = {
  safe: { label: 'Safe', icon: 'shield-check', help: 'Rebuilt automatically. No loss of data or settings.' },
  review: { label: 'Review', icon: 'eye', help: 'Harmless to Windows, but you may lose history or convenience. Worth a look first.' },
  caution: { label: 'Caution', icon: 'alert', help: 'Contains things that can’t be rebuilt. Check before removing.' },
};

export function riskLabel(risk) {
  return RISK[risk]?.label ?? risk;
}

/** A risk badge. It always carries an icon and a word, never colour alone. */
export function RiskChip(risk) {
  const r = RISK[risk] ?? RISK.review;
  const el = h('span', { class: `chip ${risk}` }, icon(r.icon), r.label);
  tooltip(el, { label: r.help });
  return el;
}

/** How removal behaves: permanent, or undoable for N days. */
export function StrategyChip(strategy, days) {
  const undo = strategy === 'undoable';
  const el = h(
    'span',
    { class: `chip ${undo ? 'accent' : ''}`.trim() },
    icon(undo ? 'undo' : 'trash'),
    undo ? `Undo for ${days} days` : 'Permanent',
  );
  tooltip(el, {
    label: undo
      ? `Removed items are kept in Recovery for ${days} days, so you can bring them back.`
      : 'Rebuilt automatically by Windows or the app, so nothing is kept. Removal is immediate.',
  });
  return el;
}

/** A horizontal meter. `severity`: undefined | 'warn' | 'serious' | 'critical'. */
export function Meter({ value, severity, thick = false, label }) {
  return h(
    'div',
    {
      class: `meter${thick ? ' thick' : ''}`,
      role: 'meter',
      'aria-valuemin': 0,
      'aria-valuemax': 100,
      'aria-valuenow': () => Math.round(value() * 100),
      'aria-label': label,
      'data-sev': severity ? () => severity() ?? '' : null,
      style: { '--v': () => Math.min(1, Math.max(0, value())) },
    },
    h('i'),
  );
}

export function Button({ label, icon: iconName, onClick, kind = '', size = '', disabled, title, iconOnly = false, autofocus }) {
  const children = [];
  if (iconName) children.push(typeof iconName === 'string' ? icon(iconName) : iconName);
  if (label != null && !iconOnly) children.push(label);
  return h(
    'button',
    {
      type: 'button',
      class: ['btn', kind, size, iconOnly ? 'icon-only' : ''].filter(Boolean).join(' '),
      disabled,
      'aria-label': iconOnly ? (typeof label === 'string' ? label : title) : null,
      title,
      autofocus,
      onClick,
    },
    ...children,
  );
}
