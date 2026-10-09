//! File metadata that only Foundation knows, read through `NSURL` resource values.
//!
//! `resource_values` is the one entry point; the typed helpers below it pick values out of
//! the dictionary it returns. Everything here is Foundation, not AppKit, so it is safe to
//! call from the scan threads.

use std::path::Path;
use std::time::{Duration, SystemTime};

use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{
    NSArray, NSDate, NSDictionary, NSString, NSURL, NSURLAddedToDirectoryDateKey,
    NSURLContentTypeKey, NSURLLocalizedTypeDescriptionKey, NSURLResourceKey,
};

/// The resource values for `keys` on the file at `path`, or `None` if the path cannot be
/// made into a URL or Foundation cannot read it. A key the volume does not support is
/// simply missing from the dictionary.
pub(crate) fn resource_values(
    path: &Path,
    keys: &[&NSURLResourceKey],
) -> Option<Retained<NSDictionary<NSURLResourceKey, AnyObject>>> {
    let url = NSURL::from_file_path(path)?;
    let keys = NSArray::from_slice(keys);
    url.resourceValuesForKeys_error(&keys).ok()
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
    autoreleasepool(|_| {
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
            .and_then(|value| value.downcast::<NSDate>().ok())
            .and_then(|date| system_time(date.timeIntervalSince1970()));
        (kind, date_added)
    })
}

fn kind_from(
    values: &NSDictionary<NSURLResourceKey, AnyObject>,
    content_type_key: &NSURLResourceKey,
    description_key: &NSURLResourceKey,
) -> Option<Kind> {
    let type_id = values
        .objectForKey(content_type_key)
        .and_then(|value| ut_type_identifier(&value))?;
    let description = values
        .objectForKey(description_key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .map_or_else(|| type_id.clone(), |s| s.to_string());
    Some(Kind {
        type_id,
        description,
    })
}

/// `UTType.identifier`, sent by name so that this module does not need the
/// UniformTypeIdentifiers bindings, which are optional (the `quicklook` feature).
fn ut_type_identifier(value: &AnyObject) -> Option<String> {
    let class = AnyClass::get(c"UTType")?;
    // SAFETY: every object in a resource-values dictionary is an NSObject.
    let is_ut_type: bool = unsafe { msg_send![value, isKindOfClass: class] };
    if !is_ut_type {
        return None;
    }
    // SAFETY: `value` is a UTType, whose `identifier` property is a non-null NSString.
    let identifier: Retained<NSString> = unsafe { msg_send![value, identifier] };
    Some(identifier.to_string())
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
