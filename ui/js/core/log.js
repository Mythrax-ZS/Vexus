// Forwards front-end problems to the same local log file as the backend, so a bug report
// contains the whole story. Nothing leaves the machine.

import { inTauri } from './bridge.js';

const T = window.__TAURI__;
let installed = false;

export function reportToLog(level, message) {
  if (!inTauri) return;
  try {
    T.core.invoke('log_frontend', { level, message: String(message).slice(0, 4000) }).catch(() => {});
  } catch {
    /* logging must never throw */
  }
}

export function installErrorReporting() {
  if (installed) return;
  installed = true;

  window.addEventListener('error', (e) => {
    reportToLog('error', `${e.message} at ${e.filename}:${e.lineno}:${e.colno}`);
  });
  window.addEventListener('unhandledrejection', (e) => {
    const r = e.reason;
    reportToLog('error', `Unhandled promise rejection: ${r?.stack || r?.message || r}`);
  });
  // The page's CSP is strict on purpose; if anything ever trips it, say so.
  document.addEventListener('securitypolicyviolation', (e) => {
    reportToLog('error', `CSP violation: ${e.violatedDirective} blocked ${e.blockedURI || 'inline'} (${e.sourceFile || 'page'}:${e.lineNumber})`);
  });
  for (const level of ['warn', 'error']) {
    const original = console[level].bind(console);
    console[level] = (...args) => {
      original(...args);
      reportToLog(level, args.map((a) => (a instanceof Error ? a.stack : typeof a === 'object' ? safeJson(a) : String(a))).join(' '));
    };
  }
}

function safeJson(v) {
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}
