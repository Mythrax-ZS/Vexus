//! A human-readable Windows version for the About/Settings screens.

use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

/// e.g. `Windows 11 Pro 24H2 (build 26100)`.
pub fn describe() -> String {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let Ok(key) = key else { return "Windows".to_string() };
    let build: u32 = key
        .get_value::<String, _>("CurrentBuildNumber")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(0);
    // `ProductName` still says "Windows 10" on Windows 11; the build number is authoritative.
    let name = if build >= 22_000 { "Windows 11" } else { "Windows 10" };
    let edition = key.get_value::<String, _>("EditionID").unwrap_or_default();
    let version = key.get_value::<String, _>("DisplayVersion").unwrap_or_default();
    let mut s = name.to_string();
    for part in [edition, version] {
        if !part.is_empty() {
            s.push(' ');
            s.push_str(&part);
        }
    }
    if build > 0 {
        s.push_str(&format!(" (build {build})"));
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn describes_this_machine() {
        let d = super::describe();
        assert!(d.starts_with("Windows"), "{d}");
    }
}
