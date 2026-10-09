// SPDX-License-Identifier: GPL-3.0-only

//! Get Info fields only macOS can supply: the localized Kind, the Date Added and an app's version.
//!
//! Each value is one `NSURL` resource-value or `NSBundle` read for a single item. They are read
//! when the details pane first shows an item, never during a directory scan. Foundation's URL
//! resource values are safe to read from any thread.

use std::path::Path;
use std::time::{Duration, SystemTime};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSBundle, NSDate, NSString, NSURL, NSURLAddedToDirectoryDateKey,
    NSURLLocalizedTypeDescriptionKey, NSURLResourceKey,
};

fn file_url(path: &Path) -> Option<Retained<NSURL>> {
    let path = path.to_str()?;
    Some(NSURL::fileURLWithPath(&NSString::from_str(path)))
}

fn resource_value(path: &Path, key: &NSURLResourceKey) -> Option<Retained<AnyObject>> {
    let url = file_url(path)?;
    let mut value = None;
    // SAFETY: `value` is an untyped object slot, and every caller downcasts it before use.
    unsafe { url.getResourceValue_forKey_error(&mut value, key) }.ok()?;
    value
}

/// Finder's Kind, such as "PDF document" or "Folder", in the system language.
pub fn localized_kind(path: &Path) -> Option<String> {
    // SAFETY: the key is a constant Foundation exports.
    let key = unsafe { NSURLLocalizedTypeDescriptionKey };
    let kind = resource_value(path, key)?.downcast::<NSString>().ok()?;
    Some(kind.to_string()).filter(|kind| !kind.is_empty())
}

/// When the item was moved or copied into its current folder.
pub fn date_added(path: &Path) -> Option<SystemTime> {
    // SAFETY: the key is a constant Foundation exports.
    let key = unsafe { NSURLAddedToDirectoryDateKey };
    let date = resource_value(path, key)?.downcast::<NSDate>().ok()?;
    let secs = date.timeIntervalSince1970();
    if secs >= 0.0 {
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs_f64(secs))
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs_f64(-secs))
    }
}

/// The marketing version (`CFBundleShortVersionString`) of an application bundle.
pub fn app_version(path: &Path) -> Option<String> {
    if path.extension().is_none_or(|ext| ext != "app") || !path.is_dir() {
        return None;
    }
    let url = file_url(path)?;
    let bundle = NSBundle::bundleWithURL(&url)?;
    let version = bundle
        .objectForInfoDictionaryKey(&NSString::from_str("CFBundleShortVersionString"))?
        .downcast::<NSString>()
        .ok()?;
    Some(version.to_string()).filter(|version| !version.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cosmic-files-get-info-{name}-{}",
            fastrand::u64(..)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn kind_of_a_pdf_names_pdf() {
        let dir = temp_dir("kind");
        let path = dir.join("sample.pdf");
        std::fs::write(&path, b"%PDF-1.4\n%%EOF\n").unwrap();
        let kind = localized_kind(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        let kind = kind.expect("a PDF has a localized kind");
        assert!(kind.contains("PDF"), "unexpected kind {kind:?}");
    }

    #[test]
    fn a_new_file_has_a_date_added() {
        let dir = temp_dir("added");
        let path = dir.join("new.txt");
        let before = SystemTime::now() - Duration::from_secs(60);
        std::fs::write(&path, b"hello").unwrap();
        let added = date_added(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        let added = added.expect("a new file has a date added");
        assert!(added >= before, "{added:?} is older than the file");
    }

    #[test]
    fn app_version_reads_the_bundle_info_plist() {
        let dir = temp_dir("version");
        let app = dir.join("Sample.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>org.example.get-info-test</string>
<key>CFBundleShortVersionString</key><string>1.2.3</string>
</dict></plist>
"#,
        )
        .unwrap();
        let version = app_version(&app);
        let not_an_app = app_version(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(version.as_deref(), Some("1.2.3"));
        assert_eq!(not_an_app, None);
    }
}
