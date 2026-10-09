// SPDX-License-Identifier: GPL-3.0-only

//! File resource values read through `NSURL`.
//!
//! Foundation's resource-value API is thread safe, so these helpers can run on the scan
//! worker. Each call builds a fresh `NSURL`, so nothing is cached between scans.

use std::path::Path;

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSString, NSURL, NSURLResourceKey, NSURLTagNamesKey};

/// Reads one resource value. `None` if the path is not UTF-8, the key is unset, or Foundation
/// reports an error.
fn resource_value(path: &Path, key: &NSURLResourceKey) -> Option<Retained<AnyObject>> {
    let path = NSString::from_str(path.to_str()?);
    let url = NSURL::fileURLWithPath(&path);
    let mut value = None;
    // SAFETY: `value` is a valid out-pointer and `key` is a Foundation resource key.
    unsafe { url.getResourceValue_forKey_error(&mut value, key) }.ok()?;
    value
}

/// The Finder tag names on `path`, in the order Finder stores them. Names only: this key does
/// not carry the tag colours.
pub fn tag_names(path: &Path) -> Option<Vec<String>> {
    autoreleasepool(|_| {
        // SAFETY: `NSURLTagNamesKey` is an immutable Foundation constant.
        let value = resource_value(path, unsafe { NSURLTagNamesKey })?;
        let array = value.downcast::<NSArray>().ok()?;
        Some(
            array
                .iter()
                .filter_map(|name| name.downcast::<NSString>().ok())
                .map(|name| name.to_string())
                .collect(),
        )
    })
}
