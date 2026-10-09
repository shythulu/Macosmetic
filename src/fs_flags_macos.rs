//! BSD file and mount flags on macOS.
//!
//! Three facts the file list needs that `std` does not name:
//!
//! - `UF_HIDDEN` in `st_flags` hides an entry from Finder. The OS sets it on the
//!   conventional system names at `/` (`bin`, `etc`, `usr`, ...) and on `~/Library`.
//! - `SF_DATALESS` in `st_flags` marks a file provider placeholder, such as an evicted iCloud
//!   Drive file. Reading its content makes the OS download it, so nothing here may open it.
//! - `MNT_LOCAL` in `statfs`'s `f_flags` is clear for network mounts (SMB, NFS, AFP, WebDAV).
//!
//! All checks are pure `libc`: `lstat`, `statfs` and `getfsstat`. None of them reads file
//! content or blocks on a network mount (`getfsstat` runs with `MNT_NOWAIT`).

use std::{
    collections::HashMap,
    ffi::CString,
    fs::Metadata,
    io,
    os::{macos::fs::MetadataExt as _, unix::ffi::OsStrExt as _, unix::fs::MetadataExt as _},
    path::Path,
    sync::{LazyLock, RwLock},
};

/// `UF_HIDDEN` from `<sys/stat.h>`.
pub const UF_HIDDEN: u32 = libc::UF_HIDDEN;
/// `SF_DATALESS` from `<sys/stat.h>`. The `libc` crate does not export it.
pub const SF_DATALESS: u32 = 0x4000_0000;

/// Whether `st_flags` carries the Finder hidden flag.
pub const fn flags_hidden(st_flags: u32) -> bool {
    st_flags & UF_HIDDEN != 0
}

/// Whether `st_flags` marks a dataless placeholder whose content lives elsewhere.
pub const fn flags_dataless(st_flags: u32) -> bool {
    st_flags & SF_DATALESS != 0
}

/// Whether `statfs`'s `f_flags` describe a local filesystem.
pub const fn mount_flags_local(f_flags: u32) -> bool {
    f_flags & libc::MNT_LOCAL as u32 != 0
}

/// Whether the entry this metadata describes is hidden with `UF_HIDDEN`.
pub fn is_hidden(metadata: &Metadata) -> bool {
    flags_hidden(metadata.st_flags())
}

/// Whether the entry this metadata describes is dataless.
pub fn is_dataless(metadata: &Metadata) -> bool {
    flags_dataless(metadata.st_flags())
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::ErrorKind::InvalidInput.into())
}

/// `st_flags` of `path` itself, not following a final symlink.
pub fn lstat_flags(path: &Path) -> io::Result<u32> {
    let c_path = c_path(path)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `c_path` is a valid C string and `stat` is a writable `struct stat`.
    if unsafe { libc::lstat(c_path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: lstat returned 0, so it filled `stat`.
    Ok(unsafe { stat.assume_init() }.st_flags)
}

/// Whether `path` is dataless. An entry that cannot be stat'ed counts as not dataless.
pub fn path_is_dataless(path: &Path) -> bool {
    lstat_flags(path).is_ok_and(flags_dataless)
}

/// Whether the filesystem holding `path` is local, by `statfs`.
///
/// The file list uses [`metadata_is_local`], which needs no path. This one is for callers
/// that have only a path.
#[allow(dead_code)]
pub fn is_local_fs(path: &Path) -> io::Result<bool> {
    let c_path = c_path(path)?;
    let mut buf = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `c_path` is a valid C string and `buf` is a writable `struct statfs`.
    if unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statfs returned 0, so it filled `buf`.
    Ok(mount_flags_local(unsafe { buf.assume_init() }.f_flags))
}

/// Every mounted filesystem's `f_fsid.val[0]`, which macOS sets to the mount's `st_dev`,
/// mapped to whether it is local.
fn mounted_filesystems() -> HashMap<i32, bool> {
    // SAFETY: a null buffer asks only for the mount count.
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if count <= 0 {
        log::warn!("getfsstat failed: {}", io::Error::last_os_error());
        return HashMap::new();
    }
    // Room for a few mounts appearing between the two calls.
    let capacity = count as usize + 8;
    let mut buf: Vec<libc::statfs> = Vec::with_capacity(capacity);
    let Ok(bytes) = libc::c_int::try_from(capacity * size_of::<libc::statfs>()) else {
        return HashMap::new();
    };
    // SAFETY: `buf` has room for `capacity` entries and `bytes` says exactly that.
    let filled = unsafe { libc::getfsstat(buf.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    if filled < 0 {
        log::warn!("getfsstat failed: {}", io::Error::last_os_error());
        return HashMap::new();
    }
    // SAFETY: getfsstat initialised the first `filled` entries, which is at most `capacity`.
    unsafe { buf.set_len((filled as usize).min(capacity)) };
    buf.iter()
        .map(|fs| {
            // SAFETY: `fsid_t` is `struct { int32_t val[2]; }`. The `libc` crate keeps the
            // field private, so it is read as the array it is.
            let fsid = unsafe { std::mem::transmute::<libc::fsid_t, [i32; 2]>(fs.f_fsid) };
            (fsid[0], mount_flags_local(fs.f_flags))
        })
        .collect()
}

/// Whether the filesystem holding the entry this metadata describes is local.
///
/// Answers from a per-mount cache keyed by device. A device the cache has not seen, such as
/// a share mounted after launch, refreshes the cache once. A device still unknown after that
/// is treated as local and remembered, so it does not rescan per item.
pub fn metadata_is_local(metadata: &Metadata) -> bool {
    static MOUNTS: LazyLock<RwLock<HashMap<i32, bool>>> =
        LazyLock::new(|| RwLock::new(mounted_filesystems()));

    // `st_dev` is an `i32` `dev_t` on macOS; std widens it to `u64`.
    let dev = metadata.dev() as i32;
    if let Some(local) = MOUNTS.read().unwrap().get(&dev) {
        return *local;
    }
    let mut mounts = MOUNTS.write().unwrap();
    if let Some(local) = mounts.get(&dev) {
        return *local;
    }
    *mounts = mounted_filesystems();
    *mounts.entry(dev).or_insert(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn chflags(path: &Path, flags: u32) {
        let c_path = c_path(path).unwrap();
        // SAFETY: `c_path` is a valid C string.
        let ret = unsafe { libc::chflags(c_path.as_ptr(), flags) };
        assert_eq!(ret, 0, "chflags: {}", io::Error::last_os_error());
    }

    #[test]
    fn flag_predicates() {
        assert!(flags_hidden(UF_HIDDEN));
        assert!(flags_hidden(UF_HIDDEN | SF_DATALESS));
        assert!(!flags_hidden(0));
        assert!(!flags_hidden(SF_DATALESS));

        assert!(flags_dataless(SF_DATALESS));
        assert!(flags_dataless(SF_DATALESS | UF_HIDDEN));
        assert!(!flags_dataless(0));
        assert!(!flags_dataless(UF_HIDDEN));

        assert!(mount_flags_local(libc::MNT_LOCAL as u32));
        assert!(!mount_flags_local(0));
    }

    #[test]
    fn uf_hidden_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain");
        fs::write(&path, b"x").unwrap();
        assert!(!is_hidden(&fs::metadata(&path).unwrap()));

        chflags(&path, UF_HIDDEN);
        assert!(is_hidden(&fs::metadata(&path).unwrap()));
        assert!(flags_hidden(lstat_flags(&path).unwrap()));
        assert!(!is_dataless(&fs::metadata(&path).unwrap()));
        assert!(!path_is_dataless(&path));
    }

    #[test]
    fn root_is_local() {
        assert!(is_local_fs(Path::new("/")).unwrap());
        assert!(metadata_is_local(&fs::metadata("/").unwrap()));
        let dir = tempfile::tempdir().unwrap();
        assert!(metadata_is_local(&fs::metadata(dir.path()).unwrap()));
    }

    /// `metadata_is_local` relies on `f_fsid.val[0]` matching `st_dev`.
    #[test]
    fn fsid_matches_st_dev() {
        let dev = fs::metadata("/").unwrap().dev() as i32;
        assert!(mounted_filesystems().contains_key(&dev));
    }

    #[test]
    fn conventional_root_names_are_flagged() {
        for name in ["bin", "etc", "usr", "var", "tmp", "sbin", "private"] {
            let path = Path::new("/").join(name);
            if let Ok(flags) = lstat_flags(&path) {
                assert!(flags_hidden(flags), "/{name} is not UF_HIDDEN");
            }
        }
        for name in ["Applications", "Library", "System", "Users"] {
            let flags = lstat_flags(&Path::new("/").join(name)).unwrap();
            assert!(!flags_hidden(flags), "/{name} is UF_HIDDEN");
        }
    }
}
