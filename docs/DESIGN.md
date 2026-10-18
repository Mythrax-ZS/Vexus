# Design

Vexus should feel calm, premium and trustworthy: charcoal surfaces, one accent, generous spacing, and nothing on screen that does not earn its place. Safety and impact are the visual hierarchy — how much, how risky, can I undo it — before anything decorative.

## Principles

* **Say what will happen.** Every control that changes something is next to a plain sentence about the consequence. The action bar always shows the exact total and how much is permanent vs. recoverable.
* **Show, don't scare.** Risk is a small chip with an icon *and* a word, never colour alone, and never red unless something cannot be undone.
* **One hero per view.** The Overview leads with a single large figure whose meaning changes with state (idle, scanning, ready, cleaning, done, empty, error).
* **Motion explains.** Count-ups show that numbers are live; the ring spins until progress is known and then fills; bars grow from their baseline; lists rise in. All of it collapses to nothing under *Reduce motion* or the Windows setting.
* **Keyboard first.** Everything is reachable and operable without a mouse; the command palette (`Ctrl+K`) exposes every action.

## Tokens (`ui/css/tokens.css`)

| Role | Dark | Light |
|---|---|---|
| Window / sidebar / page | `#0c0e12` / `#101318` / `#13161c` | `#e6e9f0` / `#ecf0f5` / `#f3f5f9` |
| Card surface | `#1a1d24` | `#ffffff` |
| Raised / hover | `#20242c` / `#292e38` | `#f6f8fb` / `#eceff4` |
| Text (primary → faint) | `#edf0f5` `#aab1bf` `#7e8695` `#565d6b` | `#12161f` `#475064` `#687184` `#9aa2b2` |
| Hairlines | white at 7% / 12% | ink at 9% / 15% |

Surfaces are soft charcoal with a slight blue cast, never pure black. Borders are hairlines and inner highlights rather than strokes.

### Accent: one hue, derived

The accent is a single hue (`--accent-h`, default 235° — cyan-blue). Lightness and chroma come from the theme in OKLCH, so *any* hue a user picks stays legible:

```
dark   accent  oklch(0.80 0.125 h)     light  accent  oklch(0.56 0.16 h)
       marks   oklch(0.66 0.15  h)            marks   oklch(0.56 0.16 h)
       violet highlight: hue + 55°            violet highlight: hue + 55°
```

The primary button is a gradient from a slightly cyan stop (h − 18°) through the accent to a violet stop (h + 50°) — the "subtle purple" of the brief. Settings offers six presets and a hue slider; a stored hex colour is reduced to its hue when applied.

### Type, space, shape

* **Typeface:** Segoe UI Variable (Display for headings, Text for body) — native, crisp at every size, zero bytes shipped. Monospace (Cascadia) only for paths.
* **Scale:** 11 / 12 / 13 / 14 / 16 / 20 / 26 px, plus a 64 px hero figure with proportional digits (tabular figures are reserved for columns that must align).
* **Space:** 4 · 8 · 12 · 16 · 20 · 24 · 32 · 40 · 56. **Radii:** 4 · 6 · 10 · 14 · 20.
* **Motion:** 120 / 200 / 380 ms with a spring-ish ease-out; view changes use the View Transitions API.

## Data visualisation

The chart rules follow a validated method: choose the form by the data's job, assign colour by the job it does, validate the palette, then apply thin-mark specs.

| Data | Form | Colour |
|---|---|---|
| Reclaimable space by category (compare magnitude, nominal categories) | **Ranked horizontal bars**, label above a full-width bar, value at the tip | One series colour (`--viz-1`) for every bar. Never a ramp or a rainbow on nominal categories. |
| Free vs used space, memory (a single ratio against a limit) | **Meter** with a same-ramp track | Accent → warning → serious → critical, always with an icon + label ("Low space", "Almost full") |
| Memory over time | **Sparkline**: 2 px line, 9% wash, ≥ 8 px end dot with a 2 px surface ring, crosshair tooltip | One series colour |
| The one number a view leads with | **Hero figure** (≥ 48 px, same sans) | Text colour |

Mark specs: bars ≤ 24 px (ours are 12 px), square at the baseline with a 4 px rounded data end, hairline solid baseline, no gridline noise. Every chart has hover **and** focus tooltips (value leads, label follows), a hit area larger than the mark, and a **table view** twin; labels are always set with `textContent`.

### Why there is no donut or stacked rainbow in v1

Part-to-whole across eleven nominal categories would need eleven distinguishable hues, which no colour-vision-safe palette can supply. The palette check (OKLab ΔE, protan/deutan simulation, lightness band, chroma floor, contrast) shows the reference categorical set passes for adjacent pairs on both of our card surfaces (dark `#1a1d24`: worst adjacent CVD ΔE 8.4, normal-vision 19.3; light `#ffffff`: 9.1 / 19.6, with three slots below 3:1 contrast that therefore require visible labels), but **only its first three slots survive the stricter "any two can touch" test**, and no 4-slot subset does. Ranked bars answer the actual question — *how much is each one* — more accurately than slices, with one hue and no ambiguity. Categorical colour will return with the disk analyzer, using the validated slots in fixed order.

### Status colours

`good #0ca30c · warning #fab219 · serious #ec835a · critical #d03b3b`. Reserved for state, never used as a series colour, and always paired with an icon and a word. Chip text stays in ink; only the icon takes the colour, which keeps contrast independent of the status hue.

## Components

| Piece | Notes |
|---|---|
| Hero + orb | The orb's ring spins while the fraction is unknown, fills when known, and rests as a full ring when done. |
| Category rows | Checkbox · icon tile · name + sub-line (items, recent files kept, risk chip in advanced mode) · size. Locked categories show an "Admin" chip instead of a size. |
| Detail pane | Chips (risk, strategy), then three plain answers — *What is this? / If you remove it / How Vexus keeps it safe* — then what was found and what was deliberately left alone, then the virtualized item list with exclusions. |
| Action bar | Sticky; total selected, permanent vs. undoable split, a warning chip when the selection includes review or irreversible items, and the primary action labelled with the exact size. |
| Confirmation | Names every category with its strategy; irreversible categories add an explicit "I understand" gate and start focus on *Cancel*. |
| Toasts | Actions are real (Undo); a toast for a restored batch is dismissed so it can never lie. |

## Accessibility

* Semantic landmarks, real buttons, native `<dialog>` (focus trap, `Esc`), `aria-live` announcements for scans and cleans, `aria-current` navigation, labelled controls.
* Visible `:focus-visible` rings; all hover content is available on focus.
* Text contrast ≥ 4.5:1 on both themes; status is never colour-only.
* *Reduce motion* setting plus `prefers-reduced-motion`; light, dark and system themes; arbitrary accent hue without losing legibility.
* Strict CSP, no remote fonts, no network.
