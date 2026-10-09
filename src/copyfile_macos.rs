// SPDX-License-Identifier: GPL-3.0-only

//! macOS `copyfile(3)` wrappers used by the recursive copy in `operation/recursive.rs`.
//!
//! A plain read/write copy drops everything that is not file data: extended attributes
//! (Finder tags, comments, quarantine, resource forks, FinderInfo), ACLs and BSD flags.
//! These helpers carry that metadata over and let same-volume copies become APFS clones.

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[cfg(test)]
thread_local! {
    /// Tests set this to force the streamed fallback on a volume that could clone.
    pub(crate) static DISABLE_CLONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
}

/// Tries to copy `from` to `to` as a clone (copy-on-write, no data is read or written).
///
/// `COPYFILE_CLONE_FORCE` implies `COPYFILE_EXCL`, so this never replaces an existing `to`,
/// matching the `create_new` open of the streamed copy. A clone carries the data, xattrs,
/// ACLs, mode, flags and timestamps of `from`.
///
/// Returns `Ok(true)` when the clone was made, `Ok(false)` when cloning is not possible here
/// (another volume, a filesystem without clones) and the caller should stream instead. An
/// existing `to` is returned as an `AlreadyExists` error.
pub fn try_clone(from: &Path, to: &Path) -> io::Result<bool> {
    #[cfg(test)]
    if DISABLE_CLONE.with(|disable| disable.get()) {
        return Ok(false);
    }

    let from_c = c_path(from)?;
    let to_c = c_path(to)?;
    // SAFETY: both paths are valid NUL-terminated strings; a null state is allowed.
    let ret = unsafe {
        libc::copyfile(
            from_c.as_ptr(),
            to_c.as_ptr(),
            std::ptr::null_mut(),
            libc::COPYFILE_CLONE_FORCE
                | libc::COPYFILE_METADATA
                | libc::COPYFILE_DATA
                | libc::COPYFILE_NOFOLLOW,
        )
    };
    if ret == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EEXIST) => Err(err),
        // Anything else (ENOTSUP, EXDEV, EINVAL...) means "no clone here". The streamed copy
        // reports real failures such as an unreadable source with its own context.
        _ => {
            tracing::debug!(?err, "clone of {} not possible, streaming", from.display());
            Ok(false)
        }
    }
}

/// Copies xattrs, ACLs, mode, flags and timestamps from one open file to another.
pub fn copy_metadata_fd(from: RawFd, to: RawFd) -> io::Result<()> {
    // SAFETY: both descriptors are open for the duration of the call; a null state is allowed.
    let ret = unsafe { libc::fcopyfile(from, to, std::ptr::null_mut(), libc::COPYFILE_METADATA) };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Copies xattrs, ACLs, mode, flags and timestamps from one path to another, without
/// following symlinks. Used for directories once their contents are in place.
pub fn copy_metadata_path(from: &Path, to: &Path) -> io::Result<()> {
    let from_c = c_path(from)?;
    let to_c = c_path(to)?;
    // SAFETY: both paths are valid NUL-terminated strings; a null state is allowed.
    let ret = unsafe {
        libc::copyfile(
            from_c.as_ptr(),
            to_c.as_ptr(),
            std::ptr::null_mut(),
            libc::COPYFILE_METADATA | libc::COPYFILE_NOFOLLOW,
        )
    };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
