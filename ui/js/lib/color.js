// Accent colour handling. The user can pick any colour, but only its *hue* is used: lightness and
// chroma come from the theme tokens, so a chosen accent can never be unreadable on the surface.

const clamp01 = (v) => Math.min(1, Math.max(0, v));

function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}
function linearToSrgb(c) {
  const v = clamp01(c);
  return v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055;
}

export function isHex(s) {
  return typeof s === 'string' && /^#[0-9a-fA-F]{6}$/.test(s);
}

/** `#rrggbb` → OKLCH `{ l, c, h }` (h in degrees). */
export function hexToOklch(hex) {
  const n = parseInt(hex.slice(1), 16);
  const r = srgbToLinear(((n >> 16) & 255) / 255);
  const g = srgbToLinear(((n >> 8) & 255) / 255);
  const b = srgbToLinear((n & 255) / 255);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return { l: L, c: Math.hypot(A, B), h: ((Math.atan2(B, A) * 180) / Math.PI + 360) % 360 };
}

/** OKLCH → `#rrggbb` (gamut-clamped). */
export function oklchToHex(l, c, h) {
  const a = c * Math.cos((h * Math.PI) / 180);
  const b = c * Math.sin((h * Math.PI) / 180);
  const l_ = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m_ = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s_ = (l - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const r = 4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_;
  const g = -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_;
  const bl = -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_;
  const to = (v) => Math.round(linearToSrgb(v) * 255).toString(16).padStart(2, '0');
  return `#${to(r)}${to(g)}${to(bl)}`;
}

/** The hue (0–360) to use for an accent hex. Near-greys fall back to the brand hue. */
export function accentHue(hex, fallback = 235) {
  if (!isHex(hex)) return fallback;
  const { c, h } = hexToOklch(hex);
  return c < 0.02 ? fallback : Math.round(h);
}

/** The swatch colour shown for a hue in the picker (same L/C the dark theme applies). */
export function hueToHex(h) {
  return oklchToHex(0.8, 0.125, h);
}

export const ACCENT_PRESETS = [
  { id: 'cyan', name: 'Cyan', hue: 222 },
  { id: 'azure', name: 'Azure', hue: 250 },
  { id: 'violet', name: 'Violet', hue: 292 },
  { id: 'rose', name: 'Rose', hue: 355 },
  { id: 'amber', name: 'Amber', hue: 72 },
  { id: 'mint', name: 'Mint', hue: 165 },
];
