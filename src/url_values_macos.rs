// SPDX-License-Identifier: GPL-3.0-only

//! File-system facts that macOS keeps as `NSURL` resource values.
//!
//! Finder decides much of its behaviour from these values rather than from `stat`. The first
//! one used here is `NSURLIsPackageKey`: a package is a directory that Finder shows and opens as
//! a single file (`.app`, `.rtfd`, `.bundle`, `.photoslibrary`, `.pages` and so on). Whether a
//! directory is a package depends on its extension being claimed by an installed app, or on the
//! directory's bundle bit, so only Launch Services can answer it.
//!
//! Everything here is Foundation, not AppKit, so it is safe to call from any thread, including
//! the blocking workers that scan directories.

use std::path::Path;

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{NSNumber, NSURL, NSURLIsPackageKey, NSURLResourceKey};

/// Read one resource value of the file at `path`.
///
/// Returns `None` when the path cannot be made into a file URL (an interior NUL byte), when the
/// file cannot be read, or when the key has no value for this file.
pub fn resource_value(path: &Path, key: &NSURLResourceKey) -> Option<Retained<AnyObject>> {
    // Foundation autoreleases temporaries inside these calls. Scan workers have no pool of
    // their own, so without this one every lookup would leak until the thread exits.
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path)?;
        let mut value: Option<Retained<AnyObject>> = None;
        // SAFETY: `value` is an `Option<Retained<AnyObject>>`, which matches any object type the
        // key may produce; callers check the concrete type before using it.
        match unsafe { url.getResourceValue_forKey_error(&mut value, key) } {
            Ok(()) => value,
            Err(err) => {
                log::debug!(
                    "failed to read resource value of {}: {}",
                    path.display(),
                    err.localizedDescription()
                );
                None
            }
        }
    })
}

/// Read a boolean resource value, such as `NSURLIsPackageKey`.
pub fn resource_bool(path: &Path, key: &NSURLResourceKey) -> Option<bool> {
    let value = resource_value(path, key)?;
    value
        .downcast_ref::<NSNumber>()
        .map(|number| number.boolValue())
}

/// Whether Finder treats the directory at `path` as a single file.
///
/// False for anything that is not a package, including plain files and paths that cannot be
/// read.
pub fn is_package(path: &Path) -> bool {
    // NSURL answers for a symlink itself, never its target, and /Applications/Safari.app is
    // a symlink into the Safari cryptex. Finder follows the link, so this does too.
    let resolved;
    let path = if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_symlink()) {
        match std::fs::canonicalize(path) {
            Ok(target) => {
                resolved = target;
                resolved.as_path()
            }
            Err(_) => return false,
        }
    } else {
        path
    };
    // SAFETY: `NSURLIsPackageKey` is an immutable constant string exported by Foundation.
    let key = unsafe { NSURLIsPackageKey };
    resource_bool(path, key).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_bundle_is_a_package() {
        assert!(is_package(Path::new("/System/Applications/Calculator.app")));
    }

    #[test]
    fn a_symlink_to_an_app_is_a_package() {
        let dir = tempfile::tempdir().expect("temp dir should be creatable");
        let link = dir.path().join("Calculator.app");
        std::os::unix::fs::symlink("/System/Applications/Calculator.app", &link)
            .expect("symlink should be creatable");
        assert!(is_package(&link));
    }

    #[test]
    fn a_plain_directory_is_not_a_package() {
        let dir = tempfile::tempdir().expect("temp dir should be creatable");
        assert!(!is_package(dir.path()));
    }

    #[test]
    fn a_plain_file_is_not_a_package() {
        let dir = tempfile::tempdir().expect("temp dir should be creatable");
        let file = dir.path().join("note.txt");
        std::fs::write(&file, "hello").expect("file should be writable");
        assert!(!is_package(&file));
    }

    #[test]
    fn a_missing_path_is_not_a_package() {
        assert!(!is_package(Path::new("/nonexistent/Missing.app")));
    }

    /// Launch Services claims `.rtfd` for TextEdit, so an empty directory with that extension
    /// is already a package; no bundle bit or contents are needed.
    #[test]
    fn a_directory_with_a_document_package_extension_is_a_package() {
        let dir = tempfile::tempdir().expect("temp dir should be creatable");
        let rtfd = dir.path().join("x.rtfd");
        std::fs::create_dir(&rtfd).expect("rtfd dir should be creatable");
        assert!(is_package(&rtfd));
    }
}
