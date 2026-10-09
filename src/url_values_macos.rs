// SPDX-License-Identifier: GPL-3.0-only

//! File-system facts that macOS keeps as `NSURL` resource values.
//!
//! Finder decides much of its behaviour from these values rather than from `stat`: whether a
//! directory is a package (`NSURLIsPackageKey`), what Kind a file is (`NSURLContentTypeKey`,
//! `NSURLLocalizedTypeDescriptionKey`), when it was added to its folder
//! (`NSURLAddedToDirectoryDateKey`). Only Launch Services can answer these, because they depend
//! on which apps are installed and on extended attributes `stat` does not show.
//!
//! [`resource_values`] is the one entry point. The typed readers below it ask for the keys a
//! feature needs, in one call per file, and pick the values out of the dictionary it returns.
//!
//! Everything here is Foundation, not AppKit, so it is safe to call from any thread, including
//! the blocking workers that scan directories.

use std::path::Path;
use std::time::{Duration, SystemTime};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSArray, NSDate, NSDictionary, NSNumber, NSString, NSURL, NSURLAddedToDirectoryDateKey,
    NSURLContentTypeKey, NSURLIsPackageKey, NSURLLocalizedTypeDescriptionKey,
    NSURLResourceKey,
};
use objc2_uniform_type_identifiers::UTType;

/// The resource values for `keys` on the file at `path`, read in one call.
///
/// Returns `None` when the path cannot be made into a file URL (an interior NUL byte) or when
/// the file cannot be read. A key the volume does not support is simply missing from the
/// dictionary.
pub(crate) fn resource_values(
    path: &Path,
    keys: &[&NSURLResourceKey],
) -> Option<Retained<NSDictionary<NSURLResourceKey, AnyObject>>> {
    // Foundation autoreleases temporaries inside these calls. Scan workers have no pool of
    // their own, so without this one every lookup would leak until the thread exits.
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path)?;
        let keys = NSArray::from_slice(keys);
        match url.resourceValuesForKeys_error(&keys) {
            Ok(values) => Some(values),
            Err(err) => {
                log::debug!(
                    "failed to read resource values of {}: {}",
                    path.display(),
                    err.localizedDescription()
                );
                None
            }
        }
    })
}

/// Read one resource value of the file at `path`.
///
/// `None` when the file cannot be read or when the key has no value for this file.
pub(crate) fn resource_value(path: &Path, key: &NSURLResourceKey) -> Option<Retained<AnyObject>> {
    resource_values(path, &[key])?.objectForKey(key)
}

/// Read a boolean resource value, such as `NSURLIsPackageKey`.
pub(crate) fn resource_bool(path: &Path, key: &NSURLResourceKey) -> Option<bool> {
    let value = resource_value(path, key)?;
    value
        .downcast_ref::<NSNumber>()
        .map(|number| number.boolValue())
}

/// Whether Finder treats the directory at `path` as a single file.
///
/// A package is a directory that Finder shows and opens as one file (`.app`, `.rtfd`,
/// `.bundle`, `.photoslibrary`, `.pages` and so on). False for anything that is not a package,
/// including plain files and paths that cannot be read.
pub(crate) fn is_package(path: &Path) -> bool {
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

/// What a file is: the UTType identifier to sort on and the localised name to show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Kind {
    /// For example `com.adobe.pdf`.
    pub type_id: String,
    /// For example "PDF document", in the user's language.
    pub description: String,
}

/// The Kind and Date Added of the file at `path`, read in one call.
pub(crate) fn kind_and_date_added(path: &Path) -> (Option<Kind>, Option<SystemTime>) {
    // SAFETY: the keys are immutable statics exported by Foundation.
    let keys = unsafe {
        [
            NSURLContentTypeKey,
            NSURLLocalizedTypeDescriptionKey,
            NSURLAddedToDirectoryDateKey,
        ]
    };
    let Some(values) = resource_values(path, &keys) else {
        return (None, None);
    };
    let kind = kind_from(&values, keys[0], keys[1]);
    let date_added = values
        .objectForKey(keys[2])
        .and_then(|value| date_from(&value));
    (kind, date_added)
}

fn kind_from(
    values: &NSDictionary<NSURLResourceKey, AnyObject>,
    content_type_key: &NSURLResourceKey,
    description_key: &NSURLResourceKey,
) -> Option<Kind> {
    let type_id = values
        .objectForKey(content_type_key)
        .and_then(|value| value.downcast::<UTType>().ok())
        .map(|content_type| content_type.identifier().to_string())?;
    let description = values
        .objectForKey(description_key)
        .and_then(|value| string_from(&value))
        .unwrap_or_else(|| type_id.clone());
    Some(Kind {
        type_id,
        description,
    })
}

/// A non-empty `NSString` value as a Rust string.
fn string_from(value: &AnyObject) -> Option<String> {
    let string = value.downcast_ref::<NSString>()?.to_string();
    (!string.is_empty()).then_some(string)
}

/// An `NSDate` value as a `SystemTime`.
fn date_from(value: &AnyObject) -> Option<SystemTime> {
    let date = value.downcast_ref::<NSDate>()?;
    system_time(date.timeIntervalSince1970())
}

fn system_time(seconds_since_epoch: f64) -> Option<SystemTime> {
    if !seconds_since_epoch.is_finite() {
        return None;
    }
    if seconds_since_epoch >= 0.0 {
        SystemTime::UNIX_EPOCH.checked_add(Duration::try_from_secs_f64(seconds_since_epoch).ok()?)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(Duration::try_from_secs_f64(-seconds_since_epoch).ok()?)
    }
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

    #[test]
    fn pdf_text_and_png_get_distinct_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let mut kinds = Vec::new();
        for name in ["a.pdf", "b.txt", "c.png"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"").unwrap();
            let (kind, _) = kind_and_date_added(&path);
            let kind = kind.unwrap_or_else(|| panic!("no kind for {name}"));
            assert!(!kind.description.is_empty());
            kinds.push(kind);
        }
        assert_eq!(kinds[0].type_id, "com.adobe.pdf");
        assert_eq!(kinds[1].type_id, "public.plain-text");
        assert_eq!(kinds[2].type_id, "public.png");
        assert_ne!(kinds[0].description, kinds[1].description);
        assert_ne!(kinds[1].description, kinds[2].description);
        assert_ne!(kinds[0].description, kinds[2].description);
    }

    #[test]
    fn a_folder_has_a_folder_kind() {
        let dir = tempfile::tempdir().unwrap();
        let (kind, _) = kind_and_date_added(dir.path());
        assert_eq!(kind.unwrap().type_id, "public.folder");
    }

    #[test]
    fn a_new_file_has_a_date_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let before = SystemTime::now() - Duration::from_secs(60);
        std::fs::write(&path, b"hello").unwrap();
        let (_, added) = kind_and_date_added(&path);
        let added = added.expect("a new file has a date added");
        assert!(added >= before, "{added:?} is older than the file");
    }

    #[test]
    fn a_missing_file_has_no_values() {
        let (kind, date_added) = kind_and_date_added(Path::new("/nonexistent/macosmetic/x.pdf"));
        assert_eq!(kind, None);
        assert_eq!(date_added, None);
    }

    #[test]
    fn epoch_seconds_convert_both_ways() {
        assert_eq!(system_time(0.0), Some(SystemTime::UNIX_EPOCH));
        assert_eq!(
            system_time(-1.0),
            SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs(1))
        );
        assert_eq!(system_time(f64::NAN), None);
    }
}
