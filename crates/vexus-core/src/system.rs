//! Read-only facts about the machine: disks, memory and whether we are elevated.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

const DRIVE_FIXED: u32 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    /// `C:\`
    pub root: String,
    pub label: String,
    pub total: u64,
    pub free: u64,
    /// The drive Windows is installed on.
    pub system: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInfo {
    pub total: u64,
    pub available: u64,
    pub load_percent: u32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Fixed local disks with their capacity and free space.
pub fn disks(system_drive: &Path) -> Vec<DiskInfo> {
    let system = system_drive.to_string_lossy().to_ascii_uppercase();
    // SAFETY: plain query with no arguments.
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u8 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", char::from(b'A' + i));
        let w = wide(&root);
        // SAFETY: `w` is NUL-terminated and outlives the calls.
        if unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) } != DRIVE_FIXED {
            continue;
        }
        let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
        // SAFETY: out-pointers reference live locals.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(w.as_ptr()),
                Some(&mut free),
                Some(&mut total),
                Some(&mut total_free),
            )
        };
        if ok.is_err() || total == 0 {
            continue;
        }
        let mut label = [0u16; 64];
        // SAFETY: `label` is a valid buffer; unused out-parameters are None.
        let have_label = unsafe {
            GetVolumeInformationW(PCWSTR(w.as_ptr()), Some(&mut label), None, None, None, None)
        }
        .is_ok();
        let label = if have_label {
            let end = label.iter().position(|&c| c == 0).unwrap_or(label.len());
            String::from_utf16_lossy(&label[..end])
        } else {
            String::new()
        };
        out.push(DiskInfo {
            system: system.starts_with(&root.to_ascii_uppercase()[..2]),
            root,
            label,
            total,
            free,
        });
    }
    out
}

pub fn memory() -> Option<MemoryInfo> {
    let mut m =
        MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `m` is initialised with the required `dwLength`.
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some(MemoryInfo {
        total: m.ullTotalPhys,
        available: m.ullAvailPhys,
        load_percent: m.dwMemoryLoad,
    })
}

/// True if this process is running with an elevated (administrator) token.
pub fn is_elevated() -> bool {
    // SAFETY: standard token query; the handle is closed on every path that opened it.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast::<c_void>()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disks_report_sane_numbers_and_exactly_one_system_drive() {
        let env = crate::env::Env::detect();
        let disks = disks(&env.system_drive);
        assert!(!disks.is_empty());
        for d in &disks {
            assert!(d.total > 0 && d.free <= d.total, "{d:?}");
            assert!(d.root.ends_with(":\\"));
        }
        assert_eq!(disks.iter().filter(|d| d.system).count(), 1);
    }

    #[test]
    fn memory_is_plausible() {
        let m = memory().unwrap();
        assert!(m.total > 0 && m.available <= m.total && m.load_percent <= 100);
    }

    #[test]
    fn elevation_query_does_not_panic() {
        let _ = is_elevated();
    }
}
