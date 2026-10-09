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

use crate::tab::{Item, ItemMetadata};
use crate::tags::{TAGS_XATTR, Tag, TagColour, Xattr, read_xattr};

/// Finder's seven standard tags, in Finder's menu order.
pub const STANDARD_TAGS: [(&str, TagColour); 7] = [
    ("Red", TagColour::Red),
    ("Orange", TagColour::Orange),
    ("Yellow", TagColour::Yellow),
    ("Green", TagColour::Green),
    ("Blue", TagColour::Blue),
    ("Purple", TagColour::Purple),
    ("Gray", TagColour::Grey),
];

/// One entry of the Tags submenu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuTag {
    pub name: String,
    pub colour: TagColour,
    /// Every selected item has this tag.
    pub checked: bool,
}

/// What a click on a menu tag does to the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Add,
    Remove,
}

impl MenuTag {
    /// If every selected item has the tag, the click removes it from all. Otherwise it adds
    /// it to the items that lack it.
    pub fn toggle(&self) -> Toggle {
        if self.checked {
            Toggle::Remove
        } else {
            Toggle::Add
        }
    }
}

/// The Tags submenu for a selection, one tag list per selected item: the seven standard
/// tags, then every other tag on the selection in first-seen order. A non-standard tag
/// takes the first colour stored for it on the selection.
pub fn menu_tags(selection: &[&[Tag]]) -> Vec<MenuTag> {
    let all_have = |name: &str| {
        !selection.is_empty()
            && selection
                .iter()
                .all(|tags| tags.iter().any(|tag| tag.name == name))
    };
    let mut menu: Vec<MenuTag> = STANDARD_TAGS
        .iter()
        .map(|(name, colour)| MenuTag {
            name: (*name).to_string(),
            colour: *colour,
            checked: all_have(name),
        })
        .collect();
    for tag in selection.iter().flat_map(|tags| tags.iter()) {
        match menu.iter_mut().find(|entry| entry.name == tag.name) {
            Some(entry) => {
                if entry.colour == TagColour::None {
                    entry.colour = tag.colour;
                }
            }
            None => menu.push(MenuTag {
                name: tag.name.clone(),
                colour: tag.colour,
                checked: all_have(&tag.name),
            }),
        }
    }
    menu
}

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

/// Whether an item can carry tags here: a real path, not a trash entry.
fn taggable(item: &Item) -> bool {
    item.path_opt().is_some() && !matches!(item.metadata, ItemMetadata::Trash { .. })
}

/// The tag lists of the selected items, or `None` if nothing is selected or any selected
/// item cannot carry tags. The menu and the click handler both build the list from this,
/// so a menu index means the same tag in both.
pub fn selected_tag_sets(items: &[Item]) -> Option<Vec<&[Tag]>> {
    let mut sets = Vec::new();
    for item in items.iter().filter(|item| item.selected) {
        if !taggable(item) {
            return None;
        }
        sets.push(item.tags.as_slice());
    }
    (!sets.is_empty()).then_some(sets)
}

/// Applies Tags submenu entry `index` to the selected items, then re-reads their tags from
/// disk so the dots match what was written. Failures are logged per file; the other files
/// still change.
pub fn toggle_selected(items: &mut [Item], index: usize) {
    let Some(tag) =
        selected_tag_sets(items).and_then(|sets| menu_tags(&sets).into_iter().nth(index))
    else {
        return;
    };
    let toggle = tag.toggle();
    for item in items.iter_mut().filter(|item| item.selected) {
        let Some(path) = item.path_opt().cloned() else {
            continue;
        };
        if let Err(err) = apply(&path, &tag.name, tag.colour, toggle) {
            log::warn!(
                "failed to {toggle:?} tag {:?} on {}: {err}",
                tag.name,
                path.display()
            );
        }
        if let Ok(metadata) = std::fs::metadata(&path) {
            item.tags = crate::tags::read(&path, &metadata);
        }
    }
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
    fn menu_lists_standard_tags_then_others() {
        let a = tags(&["Work\n6", "Red\n6"]);
        let b = tags(&["Red\n6", "Home\n4", "Work\n6"]);
        let menu = menu_tags(&[&a, &b]);
        let names: Vec<_> = menu.iter().map(|tag| tag.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Red", "Orange", "Yellow", "Green", "Blue", "Purple", "Gray", "Work", "Home"
            ]
        );
        let checked: Vec<_> = menu
            .iter()
            .filter(|tag| tag.checked)
            .map(|tag| tag.name.as_str())
            .collect();
        assert_eq!(checked, ["Red", "Work"]);
        assert_eq!(menu[7].colour, TagColour::Red);
        assert_eq!(menu[8].colour, TagColour::Blue);
    }

    #[test]
    fn toggle_removes_only_when_all_have_it() {
        let red = tags(&["Red\n6"]);
        let none = tags(&[]);
        // All have Red: remove from all.
        assert_eq!(menu_tags(&[&red, &red])[0].toggle(), Toggle::Remove);
        // Mixed: add to all.
        assert_eq!(menu_tags(&[&red, &none])[0].toggle(), Toggle::Add);
        // None have it: add.
        assert_eq!(menu_tags(&[&none, &none])[0].toggle(), Toggle::Add);
        // Empty selection checks nothing.
        assert!(menu_tags(&[]).iter().all(|tag| !tag.checked));
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
