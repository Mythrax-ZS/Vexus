// Number, size and time formatting. Sizes use binary multiples labelled KB/MB/GB, which is what
// Windows Explorer shows, so numbers match what people see elsewhere on their PC.

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
const nf = new Intl.NumberFormat('en-US');

/** Splits a byte count into a display value and unit: 1536 → { value: "1.5", unit: "KB" }. */
export function sizeParts(bytes) {
  let n = Math.max(0, Number(bytes) || 0);
  let u = 0;
  while (n >= 1024 && u < UNITS.length - 1) {
    n /= 1024;
    u++;
  }
  let value;
  if (u === 0) value = String(Math.round(n));
  else if (n >= 100) value = String(Math.round(n));
  else if (n >= 10) value = n.toFixed(1);
  else value = n.toFixed(2);
  // Trim a trailing zero fraction ("12.0" → "12") but keep "1.50"-style precision otherwise.
  if (value.includes('.')) value = value.replace(/\.0+$/, '').replace(/(\.\d)0$/, '$1');
  return { value, unit: UNITS[u] };
}

export function bytes(n) {
  const { value, unit } = sizeParts(n);
  return `${value} ${unit}`;
}

export function count(n) {
  return nf.format(Math.round(Number(n) || 0));
}

export function plural(n, one, many = one + 's') {
  return `${count(n)} ${Math.round(n) === 1 ? one : many}`;
}

export function percent(fraction, digits = 0) {
  return `${(fraction * 100).toFixed(digits)}%`;
}

/** "just now", "5 min ago", "yesterday", "12 Mar". Input is Unix seconds. */
export function ago(unixSecs, now = Date.now() / 1000) {
  if (!unixSecs) return '';
  const d = Math.max(0, now - unixSecs);
  if (d < 45) return 'just now';
  if (d < 90) return '1 min ago';
  if (d < 3600) return `${Math.round(d / 60)} min ago`;
  if (d < 5400) return '1 hour ago';
  if (d < 86400) return `${Math.round(d / 3600)} hours ago`;
  if (d < 172800) return 'yesterday';
  if (d < 86400 * 30) return `${Math.round(d / 86400)} days ago`;
  return new Date(unixSecs * 1000).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
}

/** "in 5 days", "tomorrow", "in 3 hours" for a future Unix time. */
export function until(unixSecs, now = Date.now() / 1000) {
  const d = unixSecs - now;
  if (d <= 0) return 'now';
  if (d < 3600) return `in ${Math.max(1, Math.round(d / 60))} min`;
  if (d < 86400) return `in ${Math.round(d / 3600)} hours`;
  if (d < 172800) return 'tomorrow';
  return `in ${Math.round(d / 86400)} days`;
}

export function duration(ms) {
  if (ms == null) return '';
  if (ms < 950) return `${Math.max(0.1, ms / 1000).toFixed(1)} s`;
  const s = ms / 1000;
  if (s < 10) return `${s.toFixed(1)} s`;
  if (s < 60) return `${Math.round(s)} s`;
  const m = Math.floor(s / 60);
  return `${m} min ${Math.round(s - m * 60)} s`;
}

export function etaText(ms) {
  if (ms == null) return '';
  const s = Math.max(1, Math.round(ms / 1000));
  if (s < 60) return `about ${s} s left`;
  return `about ${Math.round(s / 60)} min left`;
}

export function dateTime(unixSecs) {
  return new Date(unixSecs * 1000).toLocaleString('en-GB', {
    day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit',
  });
}

/** Shortens a long path in the middle, keeping the drive and the file name visible. */
export function midEllipsis(text, max = 64) {
  if (text.length <= max) return text;
  const keep = max - 1;
  const head = Math.ceil(keep * 0.4);
  return text.slice(0, head) + '…' + text.slice(text.length - (keep - head));
}

export function greeting(date = new Date()) {
  const h = date.getHours();
  if (h < 5) return 'Still up?';
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
}
