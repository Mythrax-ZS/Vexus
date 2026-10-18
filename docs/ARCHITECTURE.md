# Architecture

Vexus is a native Windows app built from three layers with strict boundaries. The engine knows nothing about windows or web pages; the shell knows nothing about what makes a file safe to delete; the UI knows nothing about the file system at all.

```
┌───────────────────────────────  ui/  ───────────────────────────────┐
│ HTML + CSS + ES modules, no bundler, no npm. Signals runtime, views, │
│ components. Holds ids and numbers — never paths to delete.           │
└──────────────┬───────────────────────────────────────▲──────────────┘
   invoke(cmd) │ typed JSON                streaming   │ Channel<Event>
┌──────────────▼────────────────  src-tauri/  ─────────┴──────────────┐
│ Tauri 2 shell: IPC commands, scan sessions + selection state, tray,  │
│ window geometry, settings persistence, logging, elevation relaunch.  │
└──────────────┬──────────────────────────────────────────────────────┘
               │ Rust API
┌──────────────▼──────────────  crates/vexus-core  ───────────────────┐
│ rules → resolve + guard → scan → plan → clean → recovery            │
│ No UI, no Tauri. Also drives crates/vexus-cli.                      │
└─────────────────────────────────────────────────────────────────────┘
```

## Repository layout

```
Vexus/
├─ Cargo.toml                    workspace (core, cli, app)
├─ crates/
│  ├─ vexus-core/                the engine
│  │  ├─ src/
│  │  │  ├─ engine.rs            facade used by front ends
│  │  │  ├─ rules/               declarative cleaning rules (builtin.rs = the catalogue)
│  │  │  ├─ safety/              path normalisation + default-deny guard
│  │  │  ├─ fsops.rs             handle-based open / verify / delete / move primitives, pinned folders
│  │  │  ├─ scan/                resolve.rs (roots), walker.rs (parallel walk), mod.rs (plan, queries)
│  │  │  ├─ clean/               re-verify and execute a plan
│  │  │  ├─ recovery.rs          the undo store
│  │  │  ├─ recyclebin.rs        Recycle Bin via the shell API
│  │  │  ├─ settings.rs history.rs paths.rs system.rs env.rs sandbox.rs progress.rs model.rs error.rs util.rs
│  │  └─ tests/engine.rs         end-to-end tests against a fake profile
│  └─ vexus-cli/                 headless front end (scan, clean, recovery, sandbox)
├─ src-tauri/                    the desktop shell (binary: vexus.exe)
│  ├─ src/                       main, commands, state, tray, window, logging, elevation, args
│  ├─ capabilities/main.json     what the window may ask Tauri to do
│  └─ tauri.conf.json            CSP, bundling (NSIS + MSI), identifiers
├─ ui/
│  ├─ index.html
│  ├─ css/                       tokens → base → shell → components → views/cleaner/pages
│  └─ js/
│     ├─ lib/                    signal.js dom.js format.js icons.js color.js
│     ├─ core/                   bridge store actions theme commands log
│     ├─ components/             titlebar sidebar toast tooltip dialog palette viz orb virtual-list ui
│     └─ views/                  dashboard cleaner recovery settings onboarding clean-flow
├─ scripts/                      icon generator, portable packager, DevTools-protocol test harness
└─ docs/
```

## The engine (`vexus-core`)

The pipeline is linear, and each stage is independently safe:

1. **Rules** (`rules/`) are plain data: where to look (`Tree` / single `File` / Recycle Bin targets), which files qualify, the age gate, risk (`Safe` / `Review` / `Caution`), strategy (`Permanent` / `Undoable`), whether it needs admin, and the three plain-language explanations shown in the UI. Adding a cleaner means adding an entry to `builtin.rs`; nothing else changes.
2. **Resolve** (`scan/resolve.rs`) turns rules into concrete targets for *this* machine: canonicalises roots, applies sanity checks, refuses fixed locations that have become links, registers each root with the **guard**, dedupes, shields every root from the walks of the others, and records why anything was refused.
3. **Scan** (`scan/`) walks the roots in parallel (rayon, 16 MiB worker stacks, 128 levels deep at most), never following links, and builds a compact plan: a folder whose entire content qualifies collapses into one item, so an 80,000-file cache is one row. Progress is lock-free counters sampled by a reporter thread at ~75 ms.
4. **Plan** (`Scan::plan`, `Selection`) computes, for the user's current selection and policy, exactly what a clean would do and how much goes where.
5. **Clean** (`clean/`) re-verifies every node through a handle and removes or moves it (see `docs/SAFETY.md`).
6. **Recovery** (`recovery.rs`) is the undo store, with expiry.

Side modules: `settings` (validated JSON, atomic writes), `history` (append-only JSONL), `system` (disks, memory, elevation), `sandbox` (fake profile generator), `paths` (installed vs portable vs custom data folders).

### Concurrency

* One job (scan / clean / restore / purge) at a time, enforced by the shell, so the plan you reviewed cannot change under a running clean and a Recovery batch can never be freed while it is being written or read.
* Workers run on a dedicated rayon pool at below-normal priority, so the UI stays smooth.
* Events never block workers: counters are sampled, not pushed.

### Data model for the UI

* `ItemId = (category index << 32) | item index`, stable within one scan.
* Sessions (`Arc<Scan>` + `Selection`) live in the shell, keep the two latest scans, and are dropped once their plan is executed.
* Item lists are paged (`scan_items`), searched and sorted in Rust; the UI virtualizes rows, so a result with hundreds of thousands of items costs the same to display as one with ten.

## The shell (`src-tauri`)

Commands are the whole API (see `commands.rs`): `bootstrap`, `app_summary`, `system_overview`, `settings_set`, `scan_start`, `job_cancel`, `scan_items`, `selection_apply`, `clean_start`, `recovery_list|restore|purge`, `reveal_item`, `open_folder`, `restart_elevated`, `app_ready`, `quit_app`, `log_frontend`.

* `scan_start` and `clean_start` return immediately and stream `ScanEvent` / `CleanEvent` over a Tauri `Channel`. The final `finished` event is withheld until the session is stored, so the UI can never ask for items of a scan that is not registered yet.
* The window is created hidden with a matching background colour and shown when the UI reports ready (with a 5-second failsafe), so there is no white flash and no invisible-app failure mode.
* Window geometry, settings, history and the Recovery store live in the app's own data folder (installed: `%LOCALAPPDATA%\Vexus`; portable: `data\` beside the exe; sandbox: `<folder>\.vexus`).
* Single instance (real runs only), system tray with a live tooltip ("12.9 GB can be cleaned safely"), optional close-to-tray.

## The UI (`ui/`)

A deliberate decision: **no framework, no bundler, no npm.** The toolchain on a typical Windows dev box is Rust and nothing else, there is no supply chain to audit but our own files, and start-up is instant.

* `lib/signal.js` — about 150 lines of fine-grained reactivity (signals, effects, ownership scopes). Effects and listeners created inside a view are disposed when the view unmounts.
* `lib/dom.js` — `h(tag, props, ...children)`; function props and children are reactive; text is always set via `textContent`, never `innerHTML`, so a hostile file name cannot inject markup. A keyed `each()` and a virtualized list keep large results cheap.
* The page runs under a **strict CSP** (`script-src 'self'; style-src 'self'; connect-src ipc:`): no inline script, no inline style attributes (all dynamic styling goes through the CSSOM), no remote anything.
* Appearance: one accent hue drives the palette in OKLCH; lightness/chroma come from the theme so any chosen accent stays legible. See `docs/DESIGN.md`.

## Packaging

* `cargo tauri build` produces an NSIS per-user installer and an MSI (`bundle` section of `tauri.conf.json`). The installer bootstraps WebView2 if it is missing.
* `scripts/package-portable.ps1` builds a release exe and zips it with a `vexus.portable` marker; in portable mode all data, including the WebView2 profile, stays beside the executable.
* Release profile: fat LTO, one codegen unit, `panic = "abort"`, stripped.

## Testing strategy

* `vexus-core`: unit tests per module + end-to-end tests that scan and clean a fake Windows profile, including the attacks listed in `SAFETY.md`.
* `vexus-app`: unit tests for argument parsing and command-line quoting.
* UI: `scripts/Vexus.Cdp.psm1` drives the *real* WebView2 window over the DevTools protocol (no Node needed) for screenshots and flow checks against a sandbox profile.
* Static: `cargo fmt --check`, `cargo clippy -D warnings`.

## Roadmap

| Slice | Scope |
|---|---|
| **1 — shipped** | Shell, scanner, cleaner, undo, dashboard, command palette, themes, tray, onboarding, CLI, installers |
| 2 | Browser cleaner (Chrome/Edge/Brave/Firefox, granular per-profile: cache, cookies, history…) and Windows activity-history cleaner |
| 3 | Analyzer: disk breakdown, large-file and duplicate finder (user-content mode of the guard, undo-only) |
| 4 | Startup manager (registry + scheduled tasks, with impact info), optimizer presets (Safe / Balanced / Aggressive), service and task recommendations |
| 5 | Secure free-space wipe, split-privilege elevated helper, code signing, auto-update (opt-in) |
