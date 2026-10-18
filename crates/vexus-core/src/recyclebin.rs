//! The Windows Recycle Bin, via the shell. It is a per-drive, per-user structure with its own
//! metadata, so it is queried and emptied through the supported API rather than file by file.

use std::io;
use std::mem::size_of;

use windows::Win32::UI::Shell::{
    SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHQUERYRBINFO,
    SHQueryRecycleBinW,
};
use windows::core::PCWSTR;

use crate::fsops::win_to_io;

/// `(total bytes, item count)` across all drives.
pub fn query() -> io::Result<(u64, u64)> {
    let mut info =
        SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    // SAFETY: `info` is a valid, initialised struct with `cbSize` set as the API requires.
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }.map_err(win_to_io)?;
    Ok((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
}

/// Permanently empties the Recycle Bin on every drive. **Irreversible.**
pub fn empty() -> io::Result<()> {
    // SAFETY: plain call with a null window and null path (= all drives).
    let result = unsafe {
        SHEmptyRecycleBinW(
            None,
            PCWSTR::null(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )
    };
    match result {
        Ok(()) => Ok(()),
        // Some Windows builds report an already-empty bin as E_UNEXPECTED; that's success here.
        Err(e) if e.code().0 as u32 == 0x8000_FFFF => Ok(()),
        Err(e) => Err(win_to_io(e)),
    }
}
