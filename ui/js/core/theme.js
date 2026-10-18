// Applies appearance settings to the document: theme, accent hue and motion.

import { accentHue } from '../lib/color.js';

const root = document.documentElement;
const dark = window.matchMedia?.('(prefers-color-scheme: dark)');
const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)');
let last = null;

export function resolveTheme(theme) {
  if (theme === 'light' || theme === 'dark') return theme;
  return dark?.matches === false ? 'light' : 'dark';
}

export function applyTheme(settings) {
  if (!settings) return;
  last = settings;
  root.dataset.theme = resolveTheme(settings.theme);
  root.dataset.motion = settings.reduceMotion || reduce?.matches ? 'reduced' : 'full';
  root.style.setProperty('--accent-h', String(accentHue(settings.accent)));
}

// "System" must follow the OS live.
dark?.addEventListener('change', () => last && applyTheme(last));
reduce?.addEventListener('change', () => last && applyTheme(last));
