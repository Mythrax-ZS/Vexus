//! Restarting Vexus with administrator rights — only ever on explicit user request.
//!
//! Vexus runs as a normal user (`asInvoker`) and never silently elevates. A few categories
//! (Windows temp, update caches, prefetch…) live where only administrators can look; the UI
//! shows them as locked and offers this restart. The new instance waits for the old one to
//! exit so the single-instance lock is free.

use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// Quotes one argument for a Windows command line.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Launches a new, elevated copy of this executable. Returns `Err` if the user declines the
/// UAC prompt or Windows refuses. The caller exits only on `Ok`.
pub fn relaunch_as_admin(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut all: Vec<String> = args.to_vec();
    all.push("--wait-pid".to_string());
    all.push(std::process::id().to_string());
    let params = all.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");

    let exe_w = HSTRING::from(exe.as_os_str());
    let params_w = HSTRING::from(params);
    // SAFETY: all string arguments are valid NUL-terminated wide strings that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(exe_w.as_ptr()),
            PCWSTR(params_w.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success with a value greater than 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err("Windows didn't allow the restart (the administrator prompt was cancelled)."
            .to_string())
    }
}

/// Blocks until process `pid` exits, or `timeout` passes.
pub fn wait_for_exit(pid: u32, timeout: Duration) {
    // SAFETY: opens a synchronize-only handle, waits on it and closes it.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return };
        let r = WaitForSingleObject(handle, timeout.as_millis().min(u128::from(u32::MAX)) as u32);
        let _ = CloseHandle(handle);
        if r != WAIT_OBJECT_0 {
            tracing::warn!(pid, "previous instance did not exit in time");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quoting_follows_windows_rules() {
        assert_eq!(quote("--verbose"), "--verbose");
        assert_eq!(quote(r"C:\demo dir"), r#""C:\demo dir""#);
        assert_eq!(quote(r"C:\demo dir\"), r#""C:\demo dir\\""#);
        assert_eq!(quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote(""), r#""""#);
    }
}
