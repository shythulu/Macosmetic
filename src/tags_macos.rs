// SPDX-License-Identifier: GPL-3.0-only

//! Finder tags, write path.
//!
//! # Why the extended attribute and not `NSURLTagNamesKey`
//!
//! `NSURL setResourceValue:forKey:` with `NSURLTagNamesKey` takes names only. Foundation
//! rebuilds the stored list from those names, so a custom tag such as `Work\n6` (a red
//! "Work") loses its colour, and any entry this app did not touch can be rewritten. The
//! tags xattr is the store Finder itself reads, so this module edits that instead:
//!
//! 1. Read `com.apple.metadata:_kMDItemUserTags` (a binary plist array).
//! 2. Add or remove the one toggled entry. Every other entry is kept as the same value, in
//!    the same order, including entries that are not strings.
//! 3. Write the full array back as a binary plist. When the last tag goes, the attribute is
//!    removed, which is how an untagged file looks.
//!
//! Nothing else is written. Finder and `NSURLTagNamesKey` read the attribute directly, and
//! Spotlight re-imports the file when the attribute changes, so `mdls -name kMDItemUserTags`
//! follows on indexed volumes. The legacy Finder label bits in `com.apple.FinderInfo` are
//! not updated; Finder on current macOS draws tags from the attribute.
//!
//! An attribute that exists but does not parse is never overwritten: the write fails and
//! the file keeps what it had.

use std::io;
use std::path::Path;

use crate::tags::{TAGS_XATTR, Tag, TagColour, Toggle, Xattr, read_xattr};

/// The tag name of one stored entry, parsed the way the read path parses it.
fn entry_name(entry: &plist::Value) -> Option<String> {
    entry.as_string().map(|entry| Tag::from_entry(entry).name)
}

/// Appends `Name\nN` unless an entry with `name` exists. Returns whether it appended.
pub fn add_entry(entries: &mut Vec<plist::Value>, name: &str, colour: TagColour) -> bool {
    if entries
        .iter()
        .any(|entry| entry_name(entry).as_deref() == Some(name))
    {
        return false;
    }
    entries.push(plist::Value::String(format!("{name}\n{}", colour.index())));
    true
}

/// Drops every entry named `name`. Returns whether any went.
pub fn remove_entry(entries: &mut Vec<plist::Value>, name: &str) -> bool {
    let before = entries.len();
    entries.retain(|entry| entry_name(entry).as_deref() != Some(name));
    entries.len() != before
}

/// Adds or removes one tag on `path`, keeping every other stored entry. Returns whether the
/// file changed.
pub fn apply(path: &Path, name: &str, colour: TagColour, toggle: Toggle) -> io::Result<bool> {
    let mut entries = match read_xattr(path) {
        Xattr::Absent => Vec::new(),
        Xattr::Present(bytes) => plist::Value::from_reader(io::Cursor::new(bytes))
            .ok()
            .and_then(plist::Value::into_array)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the tags attribute is not a plist array; leaving it alone",
                )
            })?,
        Xattr::Failed => return Err(io::Error::other("could not read the tags attribute")),
    };
    let changed = match toggle {
        Toggle::Add => add_entry(&mut entries, name, colour),
        Toggle::Remove => remove_entry(&mut entries, name),
    };
    if changed {
        write_entries(path, entries)?;
    }
    Ok(changed)
}

fn write_entries(path: &Path, entries: Vec<plist::Value>) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    if entries.is_empty() {
        // SAFETY: both arguments are valid C strings.
        let rc = unsafe { libc::removexattr(c_path.as_ptr(), TAGS_XATTR.as_ptr(), 0) };
        if rc != 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::ENOATTR) {
                return Err(err);
            }
        }
        return Ok(());
    }
    let mut bytes = Vec::new();
    plist::Value::Array(entries)
        .to_writer_binary(&mut bytes)
        .map_err(io::Error::other)?;
    // SAFETY: valid C strings and a buffer valid for its length.
    let rc = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            TAGS_XATTR.as_ptr(),
            bytes.as_ptr().cast(),
            bytes.len(),
            0,
            0,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::menu_tags;

    fn tags(entries: &[&str]) -> Vec<Tag> {
        entries.iter().copied().map(Tag::from_entry).collect()
    }

    fn set_raw(path: &Path, entries: &[&str]) {
        let entries = entries
            .iter()
            .map(|entry| plist::Value::String(entry.to_string()))
            .collect();
        write_entries(path, entries).unwrap();
    }

    fn raw_entries(path: &Path) -> Option<Vec<String>> {
        match read_xattr(path) {
            Xattr::Absent => None,
            Xattr::Present(bytes) => Some(
                plist::Value::from_reader(io::Cursor::new(bytes))
                    .unwrap()
                    .into_array()
                    .unwrap()
                    .into_iter()
                    .map(|value| value.into_string().unwrap())
                    .collect(),
            ),
            Xattr::Failed => panic!("read failed"),
        }
    }

    fn temp_file(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn entry_edits_keep_other_entries() {
        let mut entries = vec![
            plist::Value::String("Work\n6".into()),
            plist::Value::Integer(7.into()),
        ];
        assert!(add_entry(&mut entries, "Red", TagColour::Red));
        assert!(!add_entry(&mut entries, "Red", TagColour::Red));
        assert!(!add_entry(&mut entries, "Work", TagColour::Blue));
        assert_eq!(entries[0], plist::Value::String("Work\n6".into()));
        assert_eq!(entries[1], plist::Value::Integer(7.into()));
        assert_eq!(entries[2], plist::Value::String("Red\n6".into()));
        assert!(remove_entry(&mut entries, "Red"));
        assert!(!remove_entry(&mut entries, "Red"));
        assert_eq!(entries.len(), 2);
        // A bare name counts as the tag.
        let mut bare = vec![plist::Value::String("Red".into())];
        assert!(remove_entry(&mut bare, "Red"));
    }

    #[test]
    fn add_red_keeps_custom_tag_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_file(&dir, "a.txt");
        set_raw(&path, &["Work\n6"]);

        assert!(apply(&path, "Red", TagColour::Red, Toggle::Add).unwrap());
        assert_eq!(raw_entries(&path).unwrap(), ["Work\n6", "Red\n6"]);
        // Adding again changes nothing.
        assert!(!apply(&path, "Red", TagColour::Red, Toggle::Add).unwrap());

        // The read path and Foundation see both names.
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(
            crate::tags::read(&path, &metadata),
            tags(&["Work\n6", "Red\n6"])
        );
        assert_eq!(
            crate::url_values_macos::tag_names(&path).unwrap(),
            ["Work", "Red"]
        );
    }

    #[test]
    fn remove_red_leaves_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_file(&dir, "a.txt");
        set_raw(&path, &["Blue\n4", "Red\n6", "Work\n6"]);

        assert!(apply(&path, "Red", TagColour::Red, Toggle::Remove).unwrap());
        assert_eq!(raw_entries(&path).unwrap(), ["Blue\n4", "Work\n6"]);
        assert_eq!(
            crate::url_values_macos::tag_names(&path).unwrap(),
            ["Blue", "Work"]
        );
        // Removing a tag the file lacks changes nothing.
        assert!(!apply(&path, "Red", TagColour::Red, Toggle::Remove).unwrap());
    }

    #[test]
    fn removing_last_tag_removes_attribute() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_file(&dir, "a.txt");
        assert!(apply(&path, "Green", TagColour::Green, Toggle::Add).unwrap());
        assert_eq!(raw_entries(&path).unwrap(), ["Green\n2"]);
        assert!(apply(&path, "Green", TagColour::Green, Toggle::Remove).unwrap());
        assert_eq!(raw_entries(&path), None);
        // Removing from an untagged file is a no-op, not an error.
        assert!(!apply(&path, "Green", TagColour::Green, Toggle::Remove).unwrap());
    }

    #[test]
    fn unparseable_attribute_is_left_alone() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let path = temp_file(&dir, "a.txt");
        let junk = b"not a plist";
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: valid C strings and a buffer valid for its length.
        let rc = unsafe {
            libc::setxattr(
                c_path.as_ptr(),
                TAGS_XATTR.as_ptr(),
                junk.as_ptr().cast(),
                junk.len(),
                0,
                0,
            )
        };
        assert_eq!(rc, 0);
        assert!(apply(&path, "Red", TagColour::Red, Toggle::Add).is_err());
        assert!(matches!(read_xattr(&path), Xattr::Present(bytes) if bytes == junk));
    }

    #[test]
    fn multi_select_toggle_on_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = temp_file(&dir, "a.txt");
        let b = temp_file(&dir, "b.txt");
        set_raw(&a, &["Red\n6", "Work\n6"]);
        set_raw(&b, &["Home\n4"]);
        let read = |path: &Path| crate::tags::read(path, &std::fs::metadata(path).unwrap());

        // Mixed selection: Red is unchecked, so the click adds it where missing.
        let (ta, tb) = (read(&a), read(&b));
        let red = menu_tags(&[&ta, &tb]).remove(0);
        assert_eq!(red.toggle(), Toggle::Add);
        for path in [&a, &b] {
            apply(path, &red.name, red.colour, red.toggle()).unwrap();
        }
        assert_eq!(raw_entries(&a).unwrap(), ["Red\n6", "Work\n6"]);
        assert_eq!(raw_entries(&b).unwrap(), ["Home\n4", "Red\n6"]);

        // Now both have it: the click removes it from both.
        let (ta, tb) = (read(&a), read(&b));
        let red = menu_tags(&[&ta, &tb]).remove(0);
        assert_eq!(red.toggle(), Toggle::Remove);
        for path in [&a, &b] {
            apply(path, &red.name, red.colour, red.toggle()).unwrap();
        }
        assert_eq!(raw_entries(&a).unwrap(), ["Work\n6"]);
        assert_eq!(raw_entries(&b).unwrap(), ["Home\n4"]);
    }
}
