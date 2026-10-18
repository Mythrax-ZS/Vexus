// Icon set: 24×24 line icons, stroke-based so they inherit `currentColor`.
// Drawn in the style (and partly from the geometry) of Lucide — ISC licensed, https://lucide.dev.
// Each icon is a list of elements: a string is a path, otherwise [tag, ...numbers].

const SVG_NS = 'http://www.w3.org/2000/svg';

const I = {
  overview: [['rect', 3, 3, 7, 9, 1.5], ['rect', 14, 3, 7, 5, 1.5], ['rect', 14, 12, 7, 9, 1.5], ['rect', 3, 16, 7, 5, 1.5]],
  cleaner: [
    'M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z',
    'M20 3v4', 'M22 5h-4', 'M4 17v2', 'M5 18H3',
  ],
  sparkle: ['M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z'],
  recovery: [['rect', 2, 3, 20, 5, 1], 'M4 8v11a2 2 0 0 0 2 2h2', 'M20 8v11a2 2 0 0 1-2 2h-2', 'm9 15 3-3 3 3', 'M12 12v9'],
  settings: ['M21 4h-7', 'M10 4H3', 'M21 12h-9', 'M8 12H3', 'M21 20h-5', 'M12 20H3', 'M14 2v4', 'M8 10v4', 'M16 18v4'],
  search: [['circle', 11, 11, 8], 'm21 21-4.3-4.3'],
  check: ['M20 6 9 17l-5-5'],
  x: ['M18 6 6 18', 'm6 6 12 12'],
  plus: ['M5 12h14', 'M12 5v14'],
  'chevron-right': ['m9 18 6-6-6-6'],
  'chevron-left': ['m15 18-6-6 6-6'],
  'chevron-down': ['m6 9 6 6 6-6'],
  'chevron-up': ['m18 15-6-6-6 6'],
  'arrow-right': ['M5 12h14', 'm12 5 7 7-7 7'],
  'arrow-up-right': ['M7 7h10v10', 'M7 17 17 7'],
  shield: ['M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z'],
  'shield-check': ['M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z', 'm9 12 2 2 4-4'],
  lock: [['rect', 3, 11, 18, 11, 2], 'M7 11V7a5 5 0 0 1 10 0v4'],
  trash: ['M3 6h18', 'M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6', 'M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2'],
  file: ['M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z', 'M14 2v4a2 2 0 0 0 2 2h4'],
  folder: ['M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z'],
  'external-link': ['M15 3h6v6', 'M10 14 21 3', 'M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6'],
  clock: [['circle', 12, 12, 10], 'M12 6v6l4 2'],
  undo: ['M9 14 4 9l5-5', 'M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11'],
  refresh: ['M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8', 'M21 3v5h-5', 'M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16', 'M8 16H3v5'],
  play: [['polygon', '6 3 20 12 6 21 6 3']],
  stop: [['rect', 5, 5, 14, 14, 2]],
  alert: ['m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3', 'M12 9v4', 'M12 17h.01'],
  info: [['circle', 12, 12, 10], 'M12 16v-4', 'M12 8h.01'],
  'circle-check': [['circle', 12, 12, 10], 'm9 12 2 2 4-4'],
  'circle-alert': [['circle', 12, 12, 10], 'M12 8v4', 'M12 16h.01'],
  eye: ['M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0', ['circle', 12, 12, 3]],
  zap: ['M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z'],
  cpu: [['rect', 4, 4, 16, 16, 2], ['rect', 9, 9, 6, 6, 1], 'M15 2v2', 'M15 20v2', 'M2 15h2', 'M2 9h2', 'M20 15h2', 'M20 9h2', 'M9 2v2', 'M9 20v2'],
  drive: ['M22 12H2', 'M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z', 'M6 16h.01', 'M10 16h.01'],
  memory: ['M6 19v-3', 'M10 19v-3', 'M14 19v-3', 'M18 19v-3', 'M8 11V9', 'M16 11V9', 'M12 11V9', 'M2 15h20', 'M2 7a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v1.1a2 2 0 0 0 0 3.837V17a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-5.1a2 2 0 0 0 0-3.837Z'],
  network: [['rect', 16, 16, 6, 6, 1], ['rect', 2, 16, 6, 6, 1], ['rect', 9, 2, 6, 6, 1], 'M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3', 'M12 12V8'],
  image: [['rect', 3, 3, 18, 18, 2], ['circle', 9, 9, 2], 'm21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21'],
  bug: ['m8 2 1.88 1.88', 'M14.12 3.88 16 2', 'M9 7.13v-1a3.003 3.003 0 1 1 6 0v1', 'M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6', 'M12 20v-9', 'M6.53 9C4.6 8.8 3 7.1 3 5', 'M6 13H2', 'M3 21c0-2.1 1.7-3.9 3.8-4', 'M20.97 5c0 2.1-1.6 3.8-3.5 4', 'M22 13h-4', 'M17.2 17c2.1.1 3.8 1.9 3.8 4'],
  scroll: ['M15 12h-5', 'M15 8h-5', 'M19 17V5a2 2 0 0 0-2-2H4', 'M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3'],
  download: ['M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4', ['polyline', '7 10 12 15 17 10'], 'M12 15V3'],
  windows: [['rect', 3, 3, 8, 8, 1.5], ['rect', 13, 3, 8, 8, 1.5], ['rect', 3, 13, 8, 8, 1.5], ['rect', 13, 13, 8, 8, 1.5]],
  temp: ['M10 2h4', 'M12 14l3-3', ['circle', 12, 14, 8]],
  history: ['M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8', 'M3 3v5h5', 'M12 7v5l4 2'],
  sun: [['circle', 12, 12, 4], 'M12 2v2', 'M12 20v2', 'm4.93 4.93 1.41 1.41', 'm17.66 17.66 1.41 1.41', 'M2 12h2', 'M20 12h2', 'm6.34 17.66-1.41 1.41', 'm19.07 4.93-1.41 1.41'],
  moon: ['M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z'],
  monitor: [['rect', 2, 3, 20, 14, 2], 'M8 21h8', 'M12 17v4'],
  palette: [['circle', 13.5, 6.5, 0.6], ['circle', 17.5, 10.5, 0.6], ['circle', 8.5, 7.5, 0.6], ['circle', 6.5, 12.5, 0.6], 'M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z'],
  command: ['M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3'],
  'wifi-off': ['M12 20h.01', 'M8.5 16.429a5 5 0 0 1 7 0', 'M5 12.859a10 10 0 0 1 5.17-2.69', 'M19 12.859a10 10 0 0 0-2.007-1.523', 'M2 8.82a15 15 0 0 1 4.177-2.643', 'M22 8.82a15 15 0 0 0-11.288-3.764', 'm2 2 20 20'],
  power: ['M12 2v10', 'M18.4 6.6a9 9 0 1 1-12.77.04'],
  shuffle: ['M2 18h1.4c1.3 0 2.5-.6 3.3-1.7l6.1-8.6c.7-1.1 2-1.7 3.3-1.7H22', 'm18 2 4 4-4 4', 'M2 6h1.9c1.5 0 2.9.9 3.6 2.2', 'M22 18h-5.9c-1.3 0-2.6-.7-3.3-1.8l-.5-.8', 'm18 14 4 4-4 4'],
  list: ['M8 6h13', 'M8 12h13', 'M8 18h13', 'M3 6h.01', 'M3 12h.01', 'M3 18h.01'],
  table: ['M12 3v18', ['rect', 3, 3, 18, 18, 2], 'M3 9h18', 'M3 15h18'],
  'bar-chart': ['M3 3v16a2 2 0 0 0 2 2h16', 'M7 16V8', 'M12 16V4', 'M17 16v-5'],
  keyboard: [['rect', 2, 4, 20, 16, 2], 'M6 8h.01', 'M10 8h.01', 'M14 8h.01', 'M18 8h.01', 'M8 12h.01', 'M12 12h.01', 'M16 12h.01', 'M7 16h10'],
  sliders: ['M4 21v-7', 'M4 10V3', 'M12 21v-9', 'M12 8V3', 'M20 21v-5', 'M20 12V3', 'M1 14h6', 'M9 8h6', 'M17 16h6'],
  more: [['circle', 12, 12, 1], ['circle', 19, 12, 1], ['circle', 5, 12, 1]],
  flame: ['M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z'],
  // Window chrome glyphs use a 10-unit grid and a thinner stroke (see WINDOW below).
};

const WINDOW = {
  minimize: ['M1 5h8'],
  maximize: [['rect', 1.5, 1.5, 7, 7, 1.2]],
  restore: ['M3 3V2.2C3 1.55 3.55 1 4.2 1h3.6C8.45 1 9 1.55 9 2.2v3.6C9 6.45 8.45 7 7.8 7H7', ['rect', 1, 3, 6, 6, 1.2]],
  close: ['M1.5 1.5l7 7', 'M8.5 1.5l-7 7'],
};

function make(tag, attrs) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const k of Object.keys(attrs)) el.setAttribute(k, attrs[k]);
  return el;
}

function build(spec) {
  if (typeof spec === 'string') return make('path', { d: spec });
  const [tag, ...n] = spec;
  switch (tag) {
    case 'rect': return make('rect', { x: n[0], y: n[1], width: n[2], height: n[3], ...(n[4] ? { rx: n[4] } : {}) });
    case 'circle': return make('circle', { cx: n[0], cy: n[1], r: n[2] });
    case 'polygon': return make('polygon', { points: n[0] });
    case 'polyline': return make('polyline', { points: n[0] });
    default: return make(tag, {});
  }
}

/** Creates an `<svg class="icon">` for `name`. Unknown names render an empty box, not a crash. */
export function icon(name, { size, className = '', strokeWidth } = {}) {
  const isWin = name.startsWith('win-');
  const spec = isWin ? WINDOW[name.slice(4)] : I[name];
  const svg = make('svg', {
    viewBox: isWin ? '0 0 10 10' : '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': strokeWidth ?? (isWin ? 1.1 : 1.75),
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
    'aria-hidden': 'true',
    focusable: 'false',
  });
  svg.setAttribute('class', `icon ${className}`.trim());
  if (size) {
    svg.setAttribute('width', size);
    svg.setAttribute('height', size);
  }
  for (const part of spec || []) svg.appendChild(build(part));
  return svg;
}

export const iconNames = Object.keys(I);

/** The Vexus mark: a gradient "V" with a spark. Gradient stops are styled from CSS. */
export function logoMark({ className = 'brand-mark' } = {}) {
  const svg = make('svg', { viewBox: '0 0 32 32', 'aria-hidden': 'true', focusable: 'false' });
  svg.setAttribute('class', className);
  const defs = make('defs', {});
  const grad = make('linearGradient', { id: `lg-${Math.random().toString(36).slice(2, 7)}`, x1: '5', y1: '6', x2: '27', y2: '27', gradientUnits: 'userSpaceOnUse' });
  const a = make('stop', { offset: '0' });
  a.setAttribute('class', 'logo-stop-a');
  const b = make('stop', { offset: '1' });
  b.setAttribute('class', 'logo-stop-b');
  grad.append(a, b);
  defs.appendChild(grad);
  const v = make('path', { d: 'M8.5 9.5 16 23.5l7.5-14', fill: 'none', stroke: `url(#${grad.id})`, 'stroke-width': '4.2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' });
  const dot = make('circle', { cx: '25.2', cy: '6.4', r: '1.9' });
  dot.setAttribute('class', 'logo-spark');
  svg.append(defs, v, dot);
  return svg;
}
