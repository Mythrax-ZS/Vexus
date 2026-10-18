**Vexus** is a Windows cleaner you can trust, because it shows its work. It finds what apps and Windows leave behind, explains every item, lets you exclude anything, re-checks each file at the last moment, and moves anything that isn't regenerable into a **Recovery** store you can restore from. Offline, no telemetry.

## Downloads

| File | What it is |
|---|---|
| `Vexus_*_x64-setup.exe` | Installer for the current user (no administrator rights needed) |
| `Vexus_*_x64_en-US.msi` | MSI package, for managed deployment |
| `Vexus-*-portable-x64.zip` | Portable build: unzip anywhere; settings, logs and Recovery stay in a `data` folder beside the program |
| `SHA256SUMS.txt` | SHA-256 checksums of the three files above |

Needs 64-bit Windows 10 or 11 with the WebView2 runtime (part of Windows 11; the installers download it if it is missing).

**These builds are not code-signed yet**, so Windows SmartScreen may warn the first time you run one: choose *More info → Run anyway*. To check what you downloaded, compare its SHA-256 with `SHA256SUMS.txt` (`Get-FileHash <file>` in PowerShell).

## Good to know

* This is the **first slice**: the engine, the safety model, the cleaner (temp files, Windows Update and Delivery Optimization caches, thumbnails, Prefetch, crash dumps, logs, recent files, shader caches, Recycle Bin), Recovery, the desktop app and a command-line tool. Browser cleaning, the disk analyzer, the startup manager and the optimizer are on the [roadmap](https://github.com/Mythrax-ZS/Vexus#roadmap) and are **not** in this build.
* A scan never changes anything. To try a clean without touching your PC, run `vexus.exe --sandbox <folder>` against a fake profile.
* How Vexus decides what it may touch, and the honest limits of that: [docs/SAFETY.md](https://github.com/Mythrax-ZS/Vexus/blob/main/docs/SAFETY.md).
