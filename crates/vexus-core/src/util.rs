//! Small dependency-free helpers shared across the engine.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch (negative for times before it).
pub fn unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Current time as Unix seconds.
pub fn now_unix() -> i64 {
    unix_secs(SystemTime::now())
}

/// A random `u64` without pulling in an RNG crate (std seeds `RandomState` from the OS).
pub fn random_u64() -> u64 {
    let mut h = RandomState::new().build_hasher();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    h.write_u128(nanos);
    h.finish()
}

/// Binary-prefix size formatting for logs and the CLI (the UI formats its own).
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

/// Case-insensitive wildcard match supporting `*` and `?`.
///
/// Operates on bytes with ASCII case folding and never allocates, because it runs once per file
/// in the hot scan loop. Patterns are ASCII by construction; `?` matches exactly one byte.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi].eq_ignore_ascii_case(&t[ti])) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// Whole-hours to [`Duration`].
pub fn hours(h: u32) -> Duration {
    Duration::from_secs(u64::from(h) * 3600)
}

/// `(year, month, day)` for a day count since 1970-01-01 (Howard Hinnant's civil-from-days).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYYMMDD-HHMMSS` (UTC) — sortable, human-readable, used in recovery batch ids.
pub fn stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Writes `bytes` to `path` atomically (temp file in the same directory, flush, rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{:x}", random_u64() & 0xffff_ffff));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_basics() {
        assert!(wildcard_match("*.pf", "NOTEPAD.EXE-1234.pf"));
        assert!(wildcard_match("thumbcache_*.db", "thumbcache_256.db"));
        assert!(wildcard_match("THUMBCACHE_*.DB", "thumbcache_idx.db"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("a*b*c", "aXXbYYc"));
        assert!(!wildcard_match("*.pf", "notepad.pfx"));
        assert!(!wildcard_match("thumbcache_*.db", "iconcache_256.db"));
        assert!(!wildcard_match("a?c", "ac"));
        assert!(!wildcard_match("", "x"));
    }

    #[test]
    fn stamp_is_utc_and_sortable() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_700_000_000), "20231114-221320");
        assert_eq!(stamp(951_782_400), "20000229-000000"); // leap day
        assert!(stamp(1_000) < stamp(2_000_000));
    }

    #[test]
    fn human_bytes_formats() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn random_ids_differ() {
        assert_ne!(random_u64(), random_u64());
    }
}
