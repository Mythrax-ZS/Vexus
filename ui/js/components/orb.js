// The status orb: the Vexus mark inside a ring whose motion tells you what Vexus is doing.

import { h } from '../lib/dom.js';
import { logoMark } from '../lib/icons.js';

/** `state`: () => 'idle' | 'scanning' | 'ready' | 'cleaning' | 'done' | 'empty' | 'error'.
 *  `progress`: optional () => 0..1 (when known, the ring fills instead of spinning). */
export function Orb({ state, progress, size = 'lg' }) {
  return h(
    'div',
    {
      class: `orb ${size}`,
      'data-state': state,
      // When the work has a known fraction the ring fills; otherwise it spins.
      'data-det': () => (progress && progress() != null ? '' : null),
      style: { '--p': () => (progress ? progress() ?? 0 : 0) },
      'aria-hidden': 'true',
    },
    h('div', { class: 'orb-glow' }),
    h('div', { class: 'orb-ring' }),
    h('div', { class: 'orb-core' }, logoMark({ className: 'orb-mark' })),
  );
}
