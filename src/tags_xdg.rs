// SPDX-License-Identifier: GPL-3.0-only

//! Tags on Linux, in the `user.xdg.tags` extended attribute.
//!
//! This is the freedesktop attribute KDE's Dolphin and Baloo read and write: a comma-separated
//! list of tag names. It stores no colours, so a tag's colour comes from its name, the same
//! way Finder colours its standard tags. "Red" is red; "Work" has no colour.
//!
//! [`parse`] and [`edited`] are pure so the format rules are tested on every platform. The
//! attribute calls are Linux-only.

use crate::tags::{Tag, Toggle};

/// The tags in one attribute value, in stored order. Empty names are skipped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse(value: &[u8]) -> Vec<Tag> {
    names(value).map(Tag::from_name).collect()
}

fn names(value: &[u8]) -> impl Iterator<Item = &str> {
    std::str::from_utf8(value)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// The attribute value after adding or removing `name`, or `None` when nothing changes.
/// An empty string means the last tag went and the attribute should be removed.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn edited(value: &[u8], name: &str, toggle: Toggle) -> Option<String> {
    let mut list: Vec<&str> = names(value).collect();
    let present = list.contains(&name);
    match toggle {
        Toggle::Add if present => return None,
        Toggle::Add => list.push(name),
        Toggle::Remove if !present => return None,
        Toggle::Remove => list.retain(|entry| *entry != name),
    }
    Some(list.join(","))
}

#[cfg(target_os = "linux")]
const XATTR: &std::ffi::CStr = c"user.xdg.tags";

#[cfg(target_os = "linux")]
fn c_path(path: &std::path::Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains a NUL byte")
    })
}

/// The raw attribute value; empty when the file has none or the filesystem has no xattrs.
#[cfg(target_os = "linux")]
fn read_raw(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let c_path = c_path(path)?;
    // A second pass covers the attribute growing between the size query and the read.
    for _ in 0..2 {
        // SAFETY: a null buffer with size 0 asks for the value's size.
        let size =
            unsafe { libc::getxattr(c_path.as_ptr(), XATTR.as_ptr(), std::ptr::null_mut(), 0) };
        if size < 0 {
            let err = std::io::Error::last_os_error();
            return match err.raw_os_error() {
                Some(libc::ENODATA | libc::ENOTSUP) => Ok(Vec::new()),
                _ => Err(err),
            };
        }
        let mut buffer = vec![0u8; size as usize];
        // SAFETY: the buffer is valid for its length.
        let read = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                XATTR.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if read >= 0 {
            buffer.truncate(read as usize);
            return Ok(buffer);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ERANGE) {
            return Err(std::io::Error::last_os_error());
        }
    }
    Err(std::io::Error::other(
        "the tags attribute kept changing size",
    ))
}

/// The tags on `path`. Empty when the file has none or they cannot be read.
#[cfg(target_os = "linux")]
pub fn read(path: &std::path::Path) -> Vec<Tag> {
    read_raw(path)
        .map(|value| parse(&value))
        .unwrap_or_default()
}

/// Adds or removes one tag on `path`, keeping every other name. Returns whether it changed.
/// Removing the last tag removes the attribute.
#[cfg(target_os = "linux")]
pub fn apply(path: &std::path::Path, name: &str, toggle: Toggle) -> std::io::Result<bool> {
    let Some(value) = edited(&read_raw(path)?, name, toggle) else {
        return Ok(false);
    };
    let c_path = c_path(path)?;
    // SAFETY: valid C strings, and a buffer valid for its length.
    let rc = unsafe {
        if value.is_empty() {
            libc::removexattr(c_path.as_ptr(), XATTR.as_ptr())
        } else {
            libc::setxattr(
                c_path.as_ptr(),
                XATTR.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        }
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::TagColour;

    #[test]
    fn standard_names_take_their_colour() {
        let tags = parse(b"Red,Work");
        assert_eq!(tags.len(), 2);
        assert_eq!(
            (tags[0].name.as_str(), tags[0].colour),
            ("Red", TagColour::Red)
        );
        assert_eq!(
            (tags[1].name.as_str(), tags[1].colour),
            ("Work", TagColour::None)
        );
    }

    #[test]
    fn blank_entries_and_padding_are_dropped() {
        let names: Vec<_> = parse(b" Red, ,Blue,")
            .into_iter()
            .map(|tag| tag.name)
            .collect();
        assert_eq!(names, ["Red", "Blue"]);
    }

    #[test]
    fn adding_appends_after_existing_tags() {
        assert_eq!(
            edited(b"Work", "Red", Toggle::Add).as_deref(),
            Some("Work,Red")
        );
        assert_eq!(edited(b"", "Red", Toggle::Add).as_deref(), Some("Red"));
    }

    #[test]
    fn adding_a_present_tag_changes_nothing() {
        assert_eq!(edited(b"Work,Red", "Red", Toggle::Add), None);
    }

    #[test]
    fn removing_keeps_the_others_in_order() {
        assert_eq!(
            edited(b"Work,Red,Home", "Red", Toggle::Remove).as_deref(),
            Some("Work,Home")
        );
    }

    #[test]
    fn removing_the_last_tag_empties_the_value() {
        assert_eq!(edited(b"Red", "Red", Toggle::Remove).as_deref(), Some(""));
    }

    #[test]
    fn removing_an_absent_tag_changes_nothing() {
        assert_eq!(edited(b"Work", "Red", Toggle::Remove), None);
    }
}
