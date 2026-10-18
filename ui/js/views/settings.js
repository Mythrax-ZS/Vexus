// Settings: appearance, cleaning behaviour, app behaviour and honest information about what
// Vexus is and does.

import { h } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { ACCENT_PRESETS, accentHue, hueToHex } from '../lib/color.js';
import { S, elevated } from '../core/store.js';
import { openFolder, previewSettings, saveSettings } from '../core/actions.js';
import { askToElevate } from '../components/sidebar.js';
import { Button, Switch } from '../components/ui.js';

const cfg = () => S.settings() ?? {};

function Row({ title, desc, control }) {
  return h('div', { class: 'setting-row' }, h('div', { class: 'setting-text' }, h('div', { class: 'setting-title' }, title), desc ? h('div', { class: 'setting-desc' }, desc) : null), h('div', { class: 'setting-control' }, control));
}

function Group({ title, iconName, children, id }) {
  return h('section', { class: 'card settings-group rise', id }, h('div', { class: 'card-title' }, icon(iconName, { size: 16 }), h('span', null, title)), h('div', { class: 'setting-rows' }, children));
}

function Select({ value, options, onChange, label }) {
  return h(
    'select',
    { class: 'select', 'aria-label': label, onChange: (e) => onChange(e.target.value) },
    options.map(([v, text]) => h('option', { value: String(v), selected: () => String(value()) === String(v) }, text)),
  );
}

function Segmented({ value, options, onChange, label }) {
  return h(
    'div',
    { class: 'seg', role: 'group', 'aria-label': label },
    options.map(([v, text, iconName]) =>
      h('button', { type: 'button', 'aria-pressed': () => String(value() === v), onClick: () => onChange(v) }, iconName ? icon(iconName, { size: 14 }) : null, text),
    ),
  );
}

export function AccentPicker() {
  const hue = () => accentHue(cfg().accent);
  return h(
    'div',
    { class: 'accent-picker' },
    h(
      'div',
      { class: 'swatches', role: 'group', 'aria-label': 'Accent colour' },
      ACCENT_PRESETS.map((p) =>
        h('button', {
          type: 'button',
          class: () => `swatch${Math.abs(hue() - p.hue) <= 2 ? ' on' : ''}`,
          'aria-label': p.name,
          'aria-pressed': () => String(Math.abs(hue() - p.hue) <= 2),
          style: { '--sw': hueToHex(p.hue) },
          onClick: () => saveSettings({ accent: hueToHex(p.hue) }),
        }),
      ),
    ),
    h('input', {
      class: 'hue-slider',
      type: 'range',
      min: '0',
      max: '359',
      'aria-label': 'Accent hue',
      value: () => String(hue()),
      onInput: (e) => previewSettings({ accent: hueToHex(Number(e.target.value)) }),
      onChange: (e) => saveSettings({ accent: hueToHex(Number(e.target.value)) }),
    }),
  );
}

const POLICY_HELP = {
  smart: 'Caches Windows rebuilds on its own are removed immediately. Anything else — logs, crash dumps, history — goes to Recovery first.',
  alwaysUndo: 'Everything goes to Recovery first. Safest, but space is only freed when the undo window ends or you free it by hand.',
  alwaysPermanent: 'Everything is deleted right away, with no recovery copy. Fastest way to free space, and the least forgiving.',
};

export function SettingsView() {
  const boot = () => S.boot() ?? {};
  const mode = () => (boot().sandboxed ? 'Sandbox (fake profile)' : boot().portable ? 'Portable' : 'Installed');

  return h(
    'div',
    { class: 'page', role: 'region', 'aria-label': 'Settings' },
    h(
      'div',
      { class: 'page-inner narrow' },
      h('div', { class: 'page-head' }, h('div', null, h('h1', { class: 'page-title' }, 'Settings'), h('p', { class: 'page-sub' }, 'Changes apply immediately.'))),

      Group({
        title: 'Appearance',
        iconName: 'palette',
        children: [
          Row({
            title: 'Theme',
            control: Segmented({
              label: 'Theme',
              value: () => cfg().theme,
              onChange: (v) => saveSettings({ theme: v }),
              options: [['dark', 'Dark', 'moon'], ['light', 'Light', 'sun'], ['system', 'System', 'monitor']],
            }),
          }),
          Row({ title: 'Accent colour', desc: 'Pick a preset or slide to any hue. Vexus keeps text readable whatever you choose.', control: AccentPicker() }),
          Row({ title: 'Reduce motion', desc: 'Turns off animations and count-up effects. Follows your Windows setting by default.', control: Switch({ checked: () => !!cfg().reduceMotion, label: 'Reduce motion', onChange: (v) => saveSettings({ reduceMotion: v }) }) }),
        ],
      }),

      Group({
        title: 'Cleaning',
        iconName: 'cleaner',
        children: [
          Row({
            title: 'When removing files',
            desc: () => POLICY_HELP[cfg().deletionPolicy ?? 'smart'],
            control: Select({
              label: 'Deletion policy',
              value: () => cfg().deletionPolicy ?? 'smart',
              onChange: (v) => saveSettings({ deletionPolicy: v }),
              options: [['smart', 'Smart (recommended)'], ['alwaysUndo', 'Always keep an undo copy'], ['alwaysPermanent', 'Delete immediately']],
            }),
          }),
          Row({
            title: 'Undo window',
            desc: 'How long Recovery keeps removed items before freeing the space.',
            control: Select({ label: 'Undo window', value: () => cfg().undoDays ?? 7, onChange: (v) => saveSettings({ undoDays: Number(v) }), options: [[1, '1 day'], [3, '3 days'], [7, '7 days'], [14, '14 days'], [30, '30 days']] }),
          }),
          Row({
            title: 'Leave recent temporary files alone',
            desc: 'Temporary files younger than this are never touched — an app may still be using them.',
            control: Select({ label: 'Temporary file age limit', value: () => cfg().tempMinAgeHours ?? 72, onChange: (v) => saveSettings({ tempMinAgeHours: Number(v) }), options: [[24, '1 day'], [72, '3 days (recommended)'], [168, '7 days'], [720, '30 days']] }),
          }),
          Row({ title: 'Advanced mode', desc: 'Show every cleaner with its risk level, including ones that need a closer look.', control: Switch({ checked: () => !!cfg().advanced, label: 'Advanced mode', onChange: (v) => saveSettings({ advanced: v }) }) }),
        ],
      }),

      Group({
        title: 'App',
        iconName: 'sliders',
        children: [
          Row({ title: 'Keep running in the tray', desc: 'Closing the window hides Vexus instead of quitting. Use the tray icon to bring it back.', control: Switch({ checked: () => !!cfg().closeToTray, label: 'Keep running in the tray', onChange: (v) => saveSettings({ closeToTray: v }) }) }),
          Row({
            title: 'Verbose log',
            desc: 'Also records every file path Vexus touches. Logs stay on this PC.',
            control: h('div', { class: 'row-controls' }, Switch({ checked: () => !!cfg().verboseLogging, label: 'Verbose log', onChange: (v) => saveSettings({ verboseLogging: v }) }), Button({ label: 'Open logs', icon: 'scroll', size: 'sm', onClick: () => openFolder('logs') })),
          }),
          Row({
            title: 'Administrator access',
            desc: () => (elevated() ? 'Vexus is running as administrator, so system-wide areas are included.' : 'Windows temp files, update caches and logs can only be seen by administrators. Vexus never elevates on its own.'),
            control: () => (elevated() ? h('span', { class: 'chip safe' }, icon('shield-check'), 'Active') : Button({ label: 'Restart as administrator', icon: 'lock', size: 'sm', onClick: askToElevate })),
          }),
        ],
      }),

      Group({
        title: 'Privacy & transparency',
        iconName: 'shield-check',
        children: [
          h(
            'div',
            { class: 'privacy-note' },
            h('p', null, h('strong', null, 'Vexus works entirely offline.'), ' It contains no network code, collects no telemetry and has no account. Its pages can only talk to the app itself.'),
            h('p', null, 'Every scan and clean is recorded in a local log you can read, and anything not rebuilt automatically goes to Recovery before it’s removed.'),
          ),
          Row({ title: 'Data folder', desc: () => boot().dataDir ?? '', control: Button({ label: 'Open', icon: 'folder', size: 'sm', onClick: () => openFolder('data') }) }),
        ],
      }),

      Group({
        title: 'About',
        iconName: 'info',
        children: [
          h(
            'dl',
            { class: 'about selectable' },
            h('dt', null, 'Version'), h('dd', null, () => boot().version ?? ''),
            h('dt', null, 'Windows'), h('dd', null, () => boot().os ?? ''),
            h('dt', null, 'Mode'), h('dd', null, mode),
            h('dt', null, 'Engine'), h('dd', null, 'Rust · Tauri 2 · WebView2'),
          ),
          h('p', { class: 'muted fine' }, 'Icons are based on Lucide (ISC licence).'),
        ],
      }),
    ),
  );
}
