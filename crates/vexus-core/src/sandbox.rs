//! A realistic fake Windows profile for trying Vexus without risk.
//!
//! `vexus-cli sandbox create <dir>` fills `<dir>` with the kinds of junk the built-in rules look
//! for — plus some files that must *never* be touched — and `vexus --sandbox <dir>` (or
//! `vexus-cli --sandbox <dir>`) then runs the real engine against that folder instead of the
//! machine. Sizes are logical (`set_len`), so the folder costs little to create.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::env::Env;

const DAY: u64 = 86_400;
const MB: u64 = 1024 * 1024;

/// Tiny deterministic generator so every sandbox looks the same (handy for screenshots/tests).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn between(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SandboxSummary {
    pub files: u64,
    pub bytes: u64,
    /// Files that Vexus must leave alone (documents, system files, recent files).
    pub protected_files: u64,
}

struct Builder {
    rng: Rng,
    summary: SandboxSummary,
}

impl Builder {
    fn file(&mut self, path: PathBuf, size: u64, age_days: u64, protected: bool) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut f = File::create(&path)?;
        if size <= 32 * 1024 {
            let mut buf = vec![0u8; size as usize];
            for b in &mut buf {
                *b = (self.rng.next() & 0xff) as u8;
            }
            f.write_all(&buf)?;
        } else {
            f.set_len(size)?;
        }
        let when =
            SystemTime::now() - Duration::from_secs(age_days * DAY + self.rng.between(0, DAY / 2));
        // Both times: the age gate looks at the newer of the two, and a sample file created a
        // moment ago would otherwise always look brand new.
        use std::os::windows::fs::FileTimesExt;
        f.set_times(fs::FileTimes::new().set_modified(when).set_created(when))?;
        self.summary.files += 1;
        self.summary.bytes += size;
        if protected {
            self.summary.protected_files += 1;
        }
        Ok(())
    }

    fn many(
        &mut self,
        dir: &Path,
        prefix: &str,
        ext: &str,
        count: u64,
        size: (u64, u64),
        age: (u64, u64),
    ) -> io::Result<()> {
        for i in 0..count {
            let (s, a) = (self.rng.between(size.0, size.1), self.rng.between(age.0, age.1));
            self.file(dir.join(format!("{prefix}{i:03}.{ext}")), s, a, false)?;
        }
        Ok(())
    }
}

/// Populates `env` (which should be `Env::sandbox(..)`) with sample data.
pub fn populate(env: &Env) -> io::Result<SandboxSummary> {
    env.create_sandbox_skeleton()?;
    let mut b = Builder { rng: Rng(0x5eed_cafe), summary: SandboxSummary::default() };
    let temp = env.temp.clone();

    // ── Temporary files: folders from typical apps, old and recent mixed ──────────────
    for (name, files, size, age) in [
        ("chrome_BITS_4312", 14, (200_000, 9 * MB), (6, 40)),
        ("npm-8832-a1b2c3d4", 60, (4_000, 600_000), (9, 70)),
        ("pip-unpack-9fd2", 40, (10_000, 2 * MB), (12, 90)),
        ("MicrosoftEdgeDownloads", 6, (MB, 38 * MB), (20, 120)),
        ("vscode-inno-updater-1", 3, (20 * MB, 52 * MB), (30, 60)),
        ("Adobe", 24, (50_000, 3 * MB), (15, 100)),
        ("NVIDIA Corporation", 12, (300_000, 6 * MB), (25, 80)),
        ("dotnet-installer", 9, (MB, 11 * MB), (40, 160)),
    ] {
        b.many(&temp.join(name), "tmp", "bin", files, size, age)?;
    }
    b.many(&temp, "~DF", "tmp", 70, (1_000, 900_000), (4, 60))?;
    b.many(&temp, "setup_log_", "txt", 18, (2_000, 80_000), (8, 50))?;
    // Recent files and an app's in-use folder: these must be kept.
    b.many(&temp.join("Low"), "session", "dat", 10, (5_000, 400_000), (0, 1))?;
    b.many(&temp, "~WRS", "tmp", 8, (10_000, 90_000), (0, 2))?;
    b.file(temp.join("desktop.ini"), 174, 900, true)?;

    // ── Windows-wide locations ───────────────────────────────────────────────────────
    let win = &env.system_root;
    b.many(&win.join("Temp"), "MpCmdRun-", "log", 20, (5_000, 400_000), (5, 80))?;
    b.many(
        &win.join("SoftwareDistribution").join("Download"),
        "pkg",
        "cab",
        4,
        (60 * MB, 140 * MB),
        (6, 45),
    )?;
    b.many(
        &env.program_data
            .join("Microsoft")
            .join("Windows")
            .join("DeliveryOptimization")
            .join("Cache"),
        "chunk",
        "bin",
        6,
        (8 * MB, 30 * MB),
        (3, 30),
    )?;
    b.many(&win.join("Prefetch"), "APP-", "pf", 90, (4_000, 160_000), (2, 200))?;
    b.many(&win.join("Logs").join("CBS"), "CbsPersist_", "log", 7, (MB, 12 * MB), (10, 120))?;
    b.many(&win.join("Logs").join("DISM"), "dism-", "log", 3, (50_000, 700_000), (14, 90))?;
    b.file(win.join("MEMORY.DMP"), 256 * MB, 21, false)?;
    b.many(&win.join("Minidump"), "0412", "dmp", 4, (400_000, 2 * MB), (7, 70))?;

    // ── Per-user locations ──────────────────────────────────────────────────────────
    let local = &env.local_app_data;
    let explorer = local.join("Microsoft").join("Windows").join("Explorer");
    for n in ["thumbcache_256.db", "thumbcache_96.db", "thumbcache_idx.db", "iconcache_256.db"] {
        let size = b.rng.between(MB, 22 * MB);
        b.file(explorer.join(n), size, 3, false)?;
    }
    b.many(&local.join("CrashDumps"), "app-", "dmp", 3, (28 * MB, 140 * MB), (8, 40))?;
    b.many(
        &local.join("D3DSCache").join("a1b2c3"),
        "shader",
        "bin",
        25,
        (200_000, 3 * MB),
        (3, 90),
    )?;
    b.many(&local.join("NVIDIA").join("DXCache"), "cache", "bin", 16, (MB, 7 * MB), (4, 120))?;
    b.many(
        &env.app_data.join("Microsoft").join("Windows").join("Recent"),
        "doc-",
        "lnk",
        160,
        (900, 2_400),
        (0, 300),
    )?;
    b.many(
        &env.app_data
            .join("Microsoft")
            .join("Windows")
            .join("Recent")
            .join("AutomaticDestinations"),
        "jump-",
        "automaticDestinations-ms",
        24,
        (8_000, 90_000),
        (0, 200),
    )?;

    // ── Things that must never be touched ────────────────────────────────────────────
    for (path, size) in [
        (env.documents.join("Thesis final v7.docx"), 2 * MB),
        (env.documents.join("Taxes 2025").join("return.pdf"), 3 * MB),
        (env.desktop.join("notes.txt"), 4_000),
        (env.downloads.join("installer-setup.exe"), 90 * MB),
        (env.pictures.join("holiday").join("IMG_0001.jpg"), 5 * MB),
        (win.join("System32").join("kernel32.dll"), 700_000),
        (win.join("Installer").join("1a2b3c.msi"), 12 * MB),
        (env.program_files.join("Example App").join("app.exe"), 30 * MB),
        (env.user_profile.join("NTUSER.DAT"), 2 * MB),
        (local.join("Programs").join("Tool").join("tool.exe"), 8 * MB),
    ] {
        b.file(path, size, 1_000, true)?;
    }
    Ok(b.summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsops::canonical_dir;

    #[test]
    fn populates_a_complete_profile_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical_dir(dir.path()).unwrap();
        let env = Env::sandbox(&root);
        let s = populate(&env).unwrap();
        assert!(s.files > 600, "{s:?}");
        assert!(s.protected_files >= 10);
        assert!(env.documents.join("Thesis final v7.docx").exists());
        assert!(env.system_root.join("MEMORY.DMP").exists());

        let dir2 = tempfile::tempdir().unwrap();
        let s2 = populate(&Env::sandbox(&canonical_dir(dir2.path()).unwrap())).unwrap();
        assert_eq!((s.files, s.bytes), (s2.files, s2.bytes), "must be deterministic");
    }
}
