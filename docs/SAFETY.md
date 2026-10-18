# Safety model

Vexus deletes files, so the question that matters most is not *how much can it clean* but *what can it never touch, and what happens when something goes wrong*. This document is the contract. Every claim below is enforced in code and covered by a test; the file paths point at where.

## Principles

1. **Nothing is removed without being shown first.** A scan is read-only. The plan you review is exactly what a clean may execute — nothing more.
2. **Default deny.** A path is touchable only if it lies *strictly inside* a directory root that a built-in rule declared and the guard validated. Everything else is protected by default.
3. **Never trust the world between "scan" and "clean".** Every file is re-opened, re-verified and only then removed.
4. **Reversible unless regenerable.** Anything Windows or an app does not rebuild by itself goes to a Recovery store first.
5. **Fail closed.** If undo cannot be prepared, nothing is removed. If a path is ambiguous, it is protected. If a location is unreadable, it is skipped and reported.
6. **No surprises from privilege.** Vexus runs as a normal user and never elevates on its own.
7. **Offline and auditable.** No network code; every action is logged locally.

## The layers

| # | Layer | What it guarantees | Where |
|---|---|---|---|
| 1 | **Rules, not paths** | The UI cannot ask for a path to be deleted. IPC accepts only category ids and opaque item ids that the backend itself produced; the plan and the user's selection live in the Rust process. | `src-tauri/src/commands.rs`, `state.rs` |
| 2 | **Path guard** | Default-deny allow-list of validated rule roots, plus a hard deny-list (drive roots, Windows core folders, Program Files, Documents/Desktop/Downloads/Pictures/Music/Videos, OneDrive, profile roots, registry hives, page file, Vexus's own data). Deny always beats permit. Paths are normalised before comparison (case, `\\?\` prefixes, trailing dots/spaces, `..`, alternate data streams and device paths are rejected). | `crates/vexus-core/src/safety/` |
| 3 | **Root validation** | Each rule root is resolved to its canonical (link-free) path and checked: not a drive root, not a protected folder, not an ancestor of one, not a network path. Roots taken from user-controllable variables (`%TEMP%`) must also *look like* a temp folder. A poisoned `%TEMP%` (pointing at `C:\`, the profile, Documents or a work folder) makes the rule inert and tells the user why. A fixed system location (such as the error-report queues, which every user may write to) that has been turned into a link is refused outright, and roots nested inside other roots — a remote-desktop session's `TEMP` is `…\Temp\2` — are shielded so nothing is counted twice and no live root is ever removed as an item of another. | `scan/resolve.rs`, `safety/guard.rs` |
| 4 | **Conservative eligibility** | Name filters, an age gate measured from the **newer of a file's modified and created times** (so a copied or extracted file with an old timestamp still counts as new; temp files default to 3 days; times in the future or unknown count as "too recent"), protected file names (and names ending in a dot or space, which Win32 cannot address unambiguously), and **links are never followed or removed**. Files in use are skipped, never forced; Vexus never kills processes or stops services. Folder trees nested deeper than 128 levels are left alone and reported as "couldn't be read" (see *Known limits*). | `scan/walker.rs` |
| 5 | **Preview and exclusion** | Per-category selection and per-item exclusion, enforced by the backend, with the plan summary recomputed after every change. | `scan/mod.rs` (`Selection`, `plan`) |
| 6 | **Handle-verified execution** | Each node is opened *without following links*, then verified **through the handle** (its final resolved path is still strictly inside its root and not protected; it still matches the filter and age gate; files listed individually still have the size and timestamp you saw; a file found inside a listed folder must not have arrived or changed after the scan began) and removed or moved **through that same handle**. Removal opens are stricter than the scan's look-around: a file another program has open *for writing* is refused even if that program allows deletion (loggers do), and a file about to be moved into Recovery is refused if it is loaded as a program or library (Windows lets a running program be renamed). A directory swapped for a junction between scan and clean cannot redirect the operation, because the final path is read from the object actually opened; and each planned folder is inspected through a handle *before* it is listed, so a swapped-in junction is refused outright instead of being walked. | `fsops.rs`, `clean/mod.rs` |
| 7 | **Undo** | Anything not regenerable by design is *moved* (a same-volume rename, so effectively instant) into Recovery, mirroring its original path so each batch is self-describing, then restored on demand or freed when its window ends. Restore never overwrites an existing file and only writes into places the current rules still manage. It is as careful as a clean: each destination folder is *pinned* (held open so it cannot be removed, renamed or swapped for a link) and verified to be exactly where the stored path says before anything is written into it; missing folders are created one level at a time from the nearest verified ancestor, never behind a link; and each stored copy must really lie inside its batch. A batch whose manifest is unreadable is never freed automatically. | `recovery.rs`, `fsops.rs` (`PinnedDir`) |
| 8 | **Least privilege** | `asInvoker` manifest. Admin-only categories show as locked until you explicitly choose *Restart as administrator*. | `src-tauri/src/elevation.rs` |
| 9 | **Offline, auditable** | No HTTP client or TLS crate (`reqwest`, `hyper`, `ureq`, `curl`, `rustls`, `native-tls`) is in the dependency tree (check with `cargo tree`), and the page's CSP allows only the app's own IPC. Scans, cleans, restores and purges are logged locally (verbose mode records every path). | `tauri.conf.json`, `src-tauri/src/logging.rs` |

## Deletion strategies

Each rule declares how its data is removed:

* **Permanent** — for data that regenerates by design (temp files, thumbnail and shader caches, update download caches). Deleted immediately; the UI labels each row "Permanent" and explains why that is acceptable.
* **Undoable** — for everything else (logs, crash dumps, jump lists…). Moved to Recovery first.

The user can override with a policy: *Smart* (default, as above), *Always keep an undo copy*, or *Delete immediately*. The Recycle Bin is the one exception that no policy can soften: emptying it is irreversible, so it is off by default, requires an explicit "I understand" confirmation in the UI, **and** the backend refuses to start such a clean without that acknowledgement.

## What Vexus never does

* Remove or move anything outside a validated rule root, or the root itself.
* Follow or delete a symlink, junction or other reparse point (this includes OneDrive placeholders).
* Touch `System32`, `WinSxS`, `Installer`, Program Files, your personal folders, registry hives, the page file or its own data.
* Delete a file that changed after you reviewed it, appeared in a listed folder after the scan, is newer than the age gate, or is open for writing in another program.
* Fall back to permanent deletion if Recovery cannot be prepared.
* Open a network connection, or run an installer, script or any other program.

## Known limits (stated honestly)

* **Sizes are logical** (bytes in files), not clusters on disk, so "freed space" can differ slightly from what Explorer reports. The done screen also shows the measured change in free space.
* **Locked files stay.** Windows Explorer or a browser may hold files open; those are reported as "in use" and left alone. Vexus does not stop services (for example the update service) to reach them.
* **"In use" is best effort.** A program that has a file open only for *reading* and allows deletion looks the same as a bystander, so such a file can still be removed; the reader keeps its open handle (Windows semantics) but cannot reopen the file. Files open for writing, and — when going to Recovery — files loaded as programs or libraries, are refused.
* **Age is a timestamp judgement.** The newer of the modified and created times is used. NTFS's separate *change time* (which also moves on renames) is not consulted, so an old file moved into a cleaned folder is as old as it looks.
* **Elevation is all-or-nothing today.** *Restart as administrator* relaunches the whole app elevated. A split-privilege helper — a small elevated process that re-derives its own plan from the rules and never accepts paths from the UI — is on the roadmap.
* **Very deep folder trees are skipped.** Scanning, cleaning and the Recovery store never recurse more than 128 levels (192 inside the store, which adds its own prefix). NTFS allows trees about 16,000 levels deep, which no real application produces and which would overflow a recursive walker's stack. Such a tree is left untouched and counted under "couldn't be read". Worker threads also run on 16 MiB stacks, far more than 128 levels need.
* **Listing is not atomic.** A folder item is re-walked at clean time and every file inside is re-verified individually, but a file created a microsecond after the directory listing is simply left for the next scan.
* **Cross-volume recovery** falls back to copy-then-delete, which is slower than a rename. The copy is verified byte-for-byte by length before the original is removed, and carries the file's content and its modified and created times — not its permissions or alternate data streams. The default Recovery store lives on the system drive, so the common case is a rename.
* **The Recovery store is not permission-hardened.** It inherits the permissions of the folder it lives in (your profile by default). In a portable copy on a shared drive, or while elevated, someone else who can write there could tamper with stored copies. Restore re-verifies every destination and that every copy lies inside its batch, but cannot make tampered *content* trustworthy.
* **Some categories need administrator rights** (`C:\Windows\Temp`, Windows Update downloads, Delivery Optimization, Prefetch, parts of the log set). They are shown as locked rather than hidden.

## Sandbox mode

`vexus.exe --sandbox <folder>` and `vexus-cli --sandbox <folder>` run the real engine against a fake Windows profile (filled with sample junk *and* files that must never be touched). A sandbox rule set excludes the Recycle Bin rule, because that talks to the real shell and a sandbox must not be able to reach real system state. An explicit `--sandbox` that is malformed fails with an error instead of silently running against your PC.

## How this is verified

* **Unit tests** for path normalisation, the guard (including hostile roots and bypass attempts), filters, the walker, settings, history and recovery.
* **End-to-end engine tests** (`crates/vexus-core/tests/engine.rs`) against a fake profile: stale plans (files modified or touched after the scan), locked files, read-only files, protected names, a **junction swapped in after the scan**, junctions inside the temp folder, a hostile `%TEMP%` (Documents, profile, drive root), an unwritable Recovery folder, cancellation, undo round-trips, tampered recovery batches, a **2,000-level-deep folder tree** present at scan time and one that appears *after* the scan, and "a bug-reaching clean of every category leaves planted system files untouched".
* **Mutation checks:** the junction-swap test was confirmed to *fail* when the containment check is removed from the cleaner, again when the folder pre-check is removed, and the deep-tree tests crash with a stack overflow when the depth cap is removed — i.e. each test genuinely guards its property.
* **A real-machine finding.** The first scan of a real `%TEMP%` crashed the app: leftover test folders in it were thousands of levels deep and the recursive walker overflowed its stack. The depth cap, the larger worker stacks and the regression tests above came out of that; the same scan now completes (about 97,000 entries in roughly half a second) and reports the deep trees instead of entering them.
* **An independent review.** A separate reviewer with no stake in the implementation read the whole deletion path and reproduced nine problems against it (a restore race, a fixed location turned into a link, unbounded recursion, folder items not tied to the preview, files that were being written to, nested roots, an unreadable manifest shortening its own undo window, look-alike names, and an unverified folder listing). Each was reproduced with a test that failed before the fix; the scan- and clean-side fixes were also mutation-checked (their tests fail again when the fix is removed). Two honest caveats: the restore race is closed *by construction* — a pinned folder cannot be swapped, which is tested on the primitive rather than by racing two threads — and the batch-id collision test is probabilistic against the old code, deterministic against the new. The reviewer also found that the per-file handle checks held up against junction swaps at every level.
* **Lints:** `cargo clippy --workspace --all-targets` is clean; unsafe blocks carry `SAFETY:` justifications.

## Reporting a problem

If Vexus ever offers, removes or moves something it should not have, that is a security-relevant bug. Please open an issue with the log from the folder shown in *Settings → Verbose log → Open logs* (it contains the paths involved) and, if possible, the rule category that was selected.
