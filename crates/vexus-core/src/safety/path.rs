//! Windows path normalisation used for every safety comparison.

use std::path::{Path, PathBuf};

/// Normalises a path for *comparison*: lower-case, `\` separators, no `\\?\` prefix, no trailing
/// separator, `.` segments dropped, and the per-component trailing dots/spaces that Win32
/// silently strips removed (so `C:\Windows.\System32` cannot dodge a `c:\windows\system32` rule).
///
/// Returns `None` for anything that cannot be reasoned about safely: relative paths, `..`
/// segments, device paths (`\\.\`), volume-GUID paths, alternate data streams and wildcard
/// characters. Callers must treat `None` as "protected".
///
/// Network paths normalise to a string starting with `\\`.
pub fn normalize(path: &Path) -> Option<String> {
    let raw = path.to_str()?.replace('/', "\\");
    let (network, rest) = if let Some(r) = raw.strip_prefix(r"\\?\UNC\") {
        (true, r.to_string())
    } else if let Some(r) = raw.strip_prefix(r"\\?\") {
        (false, r.to_string())
    } else if raw.starts_with(r"\\.\") {
        return None;
    } else if let Some(r) = raw.strip_prefix(r"\\") {
        (true, r.to_string())
    } else {
        (false, raw)
    };

    let mut out = String::with_capacity(rest.len() + 2);
    if network {
        out.push_str(r"\\");
    }
    let mut first = true;
    for seg in rest.split('\\') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return None;
        }
        if first && !network {
            // Local paths must start with an exact drive designator ("c:"), checked *before*
            // any trimming so drive-relative forms such as `C:.\x` or `C:foo` are rejected.
            let b = seg.as_bytes();
            if !(b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
                return None;
            }
            out.push_str(seg);
            first = false;
            continue;
        }
        let seg = seg.trim_end_matches(['.', ' ']);
        if seg.is_empty() || seg.contains(['<', '>', '"', '|', '?', '*', ':']) {
            return None; // empty after trimming, wildcard, or alternate data stream
        }
        if !first {
            out.push('\\');
        }
        out.push_str(seg);
        first = false;
    }
    if out.is_empty() || out == r"\\" {
        return None;
    }
    Some(out.to_lowercase())
}

/// `child` is `parent` or lies below it (component-boundary aware: `\temp2` is not in `\temp`).
pub fn is_within(child: &Path, parent: &Path) -> bool {
    match (normalize(child), normalize(parent)) {
        (Some(c), Some(p)) => within_or_equal(&c, &p),
        _ => false,
    }
}

/// `child` lies strictly below `parent` (never equal to it).
pub fn is_strictly_within(child: &Path, parent: &Path) -> bool {
    match (normalize(child), normalize(parent)) {
        (Some(c), Some(p)) => strictly_within(&c, &p),
        _ => false,
    }
}

pub(super) fn within_or_equal(child: &str, parent: &str) -> bool {
    child == parent || strictly_within(child, parent)
}

pub(super) fn strictly_within(child: &str, parent: &str) -> bool {
    child.len() > parent.len()
        && child.as_bytes()[parent.len()] == b'\\'
        && child.starts_with(parent)
}

pub(super) fn is_drive_root(norm: &str) -> bool {
    let b = norm.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

pub(super) fn is_network(norm: &str) -> bool {
    norm.starts_with(r"\\")
}

/// Removes a `\\?\` (or `\\?\UNC\`) prefix from a canonical path so it can be displayed and
/// compared with ordinary paths. Device and volume-GUID forms are returned unchanged.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else { return path.to_path_buf() };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Option<String> {
        normalize(Path::new(s))
    }

    #[test]
    fn normalizes_case_slashes_and_trailing_separators() {
        assert_eq!(n(r"C:\Windows\System32\").as_deref(), Some(r"c:\windows\system32"));
        assert_eq!(n("C:/Windows//System32").as_deref(), Some(r"c:\windows\system32"));
        assert_eq!(n(r"c:\").as_deref(), Some("c:"));
        assert_eq!(n(r"C:\a\.\b").as_deref(), Some(r"c:\a\b"));
    }

    #[test]
    fn strips_verbatim_prefixes() {
        assert_eq!(n(r"\\?\C:\Users\Bob").as_deref(), Some(r"c:\users\bob"));
        assert_eq!(n(r"\\?\UNC\srv\share\x").as_deref(), Some(r"\\srv\share\x"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\C:\Users\Bob")), PathBuf::from(r"C:\Users\Bob"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\UNC\srv\share")), PathBuf::from(r"\\srv\share"));
        let guid = Path::new(r"\\?\Volume{1234}\x");
        assert_eq!(strip_verbatim(guid), guid.to_path_buf());
    }

    #[test]
    fn rejects_anything_ambiguous() {
        assert_eq!(n(r"C:\a\..\b"), None, "parent segments");
        assert_eq!(n(r"relative\path"), None, "relative");
        assert_eq!(n(r"\\.\PhysicalDrive0"), None, "device path");
        assert_eq!(n(r"\\?\Volume{1234}\x"), None, "volume guid");
        assert_eq!(n(r"C:\a\file.txt:stream"), None, "alternate data stream");
        assert_eq!(n(r"C:\a\*.tmp"), None, "wildcard");
        assert_eq!(n(r"C:.\a"), None, "drive-relative");
        assert_eq!(n(r"C:foo\a"), None, "drive-relative");
        assert_eq!(n(""), None);
    }

    #[test]
    fn trailing_dots_and_spaces_cannot_dodge_comparisons() {
        assert_eq!(n(r"C:\Windows.\System32 ").as_deref(), Some(r"c:\windows\system32"));
        assert_eq!(n(r"C:\Windows...\x").as_deref(), Some(r"c:\windows\x"));
        assert_eq!(n(r"C:\ . \x"), None, "a component of only dots/spaces is meaningless");
    }

    #[test]
    fn within_respects_component_boundaries() {
        let p = Path::new;
        assert!(is_within(p(r"C:\Temp\a\b"), p(r"C:\Temp")));
        assert!(is_within(p(r"c:\temp"), p(r"C:\TEMP")));
        assert!(!is_within(p(r"C:\Temp2\a"), p(r"C:\Temp")), "sibling with shared prefix");
        assert!(is_within(p(r"C:\Windows"), p(r"C:\")));
        assert!(!is_within(p(r"D:\x"), p(r"C:\")));
        assert!(!is_within(p(r"C:\a\..\b"), p(r"C:\")), "unresolvable never matches");
    }
}
