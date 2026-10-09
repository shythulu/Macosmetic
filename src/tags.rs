// SPDX-License-Identifier: GPL-3.0-only

//! Finder tags: the model and the read path.
//!
//! Finder keeps a file's tags in the `com.apple.metadata:_kMDItemUserTags` extended attribute.
//! The value is a binary plist array of strings. Each string is `Name\nN`, where `N` is the
//! colour index 0 to 7. A string with no `\n` has no stored colour; the standard names still
//! map to their colour.
//!
//! [`read`] tries the attribute first because it carries colours. If the attribute exists but
//! cannot be parsed, it falls back to `NSURLTagNamesKey`, which gives names only. Tags are read
//! at scan time on local volumes only, so a change made in Finder shows after a reload.
//!
//! Writing lives in `tags_macos`. On other platforms [`read`] returns no tags.

use std::fs::Metadata;
use std::path::Path;

/// Finder's eight tag colours, numbered as Finder stores them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TagColour {
    #[default]
    None,
    Grey,
    Green,
    Purple,
    Blue,
    Yellow,
    Red,
    Orange,
}

impl TagColour {
    /// The colour for Finder's stored index. Out-of-range indices have no colour.
    pub fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Grey,
            2 => Self::Green,
            3 => Self::Purple,
            4 => Self::Blue,
            5 => Self::Yellow,
            6 => Self::Red,
            7 => Self::Orange,
            _ => Self::None,
        }
    }

    /// Finder's stored index for this colour, the inverse of [`Self::from_index`].
    pub fn index(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Grey => 1,
            Self::Green => 2,
            Self::Purple => 3,
            Self::Blue => 4,
            Self::Yellow => 5,
            Self::Red => 6,
            Self::Orange => 7,
        }
    }

    /// The colour Finder gives a standard tag name. Custom names have no colour.
    pub fn from_standard_name(name: &str) -> Self {
        match name {
            "Gray" | "Grey" => Self::Grey,
            "Green" => Self::Green,
            "Purple" => Self::Purple,
            "Blue" => Self::Blue,
            "Yellow" => Self::Yellow,
            "Red" => Self::Red,
            "Orange" => Self::Orange,
            _ => Self::None,
        }
    }

    /// The dot colour as sRGB, from the macOS system colours. `None` draws no dot.
    pub fn rgb(self) -> Option<(u8, u8, u8)> {
        match self {
            Self::None => None,
            Self::Grey => Some((142, 142, 147)),
            Self::Green => Some((52, 199, 89)),
            Self::Purple => Some((175, 82, 222)),
            Self::Blue => Some((0, 122, 255)),
            Self::Yellow => Some((255, 204, 0)),
            Self::Red => Some((255, 59, 48)),
            Self::Orange => Some((255, 149, 0)),
        }
    }
}

/// One Finder tag.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tag {
    pub name: String,
    pub colour: TagColour,
}

impl Tag {
    /// Parses one stored entry: `Name\nN`, or a bare name.
    pub fn from_entry(entry: &str) -> Self {
        if let Some((name, index)) = entry.rsplit_once('\n')
            && let Ok(index) = index.trim().parse::<u8>()
        {
            return Self {
                name: name.to_string(),
                colour: TagColour::from_index(index),
            };
        }
        Self::from_name(entry)
    }

    /// A tag known only by name, as `NSURLTagNamesKey` returns it.
    pub fn from_name(name: &str) -> Self {
        Self {
            name: name.to_string(),
            colour: TagColour::from_standard_name(name),
        }
    }
}

/// Up to `max` tags that have a colour, in stored order. These are the dots drawn by a name.
pub fn dot_colours(tags: &[Tag], max: usize) -> impl Iterator<Item = TagColour> + '_ {
    tags.iter()
        .map(|tag| tag.colour)
        .filter(|colour| *colour != TagColour::None)
        .take(max)
}

#[cfg(target_os = "macos")]
pub(crate) const TAGS_XATTR: &std::ffi::CStr = c"com.apple.metadata:_kMDItemUserTags";

/// Parses the tags attribute. `None` if it is not a plist array of strings.
#[cfg(target_os = "macos")]
pub fn parse_xattr(bytes: &[u8]) -> Option<Vec<Tag>> {
    let value = plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?;
    value
        .as_array()?
        .iter()
        .map(|entry| entry.as_string().map(Tag::from_entry))
        .collect()
}

/// What the tags attribute read found.
#[cfg(target_os = "macos")]
pub(crate) enum Xattr {
    Absent,
    Present(Vec<u8>),
    Failed,
}

#[cfg(target_os = "macos")]
pub(crate) fn read_xattr(path: &Path) -> Xattr {
    use std::os::unix::ffi::OsStrExt;

    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return Xattr::Failed;
    };
    // A second pass covers the attribute growing between the size query and the read.
    for _ in 0..2 {
        // SAFETY: a null buffer with size 0 asks for the value's size.
        let size = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                TAGS_XATTR.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        if size < 0 {
            return match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOATTR) => Xattr::Absent,
                _ => Xattr::Failed,
            };
        }
        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` is valid for `buf.len()` bytes.
        let read = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                TAGS_XATTR.as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                0,
            )
        };
        if read >= 0 {
            buf.truncate(read as usize);
            return Xattr::Present(buf);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ERANGE) {
            return Xattr::Failed;
        }
    }
    Xattr::Failed
}

/// Whether `statfs` flags describe a local volume. Tags are not read on anything else.
#[cfg(target_os = "macos")]
pub fn is_local_flags(f_flags: u32) -> bool {
    f_flags & libc::MNT_LOCAL as u32 != 0
}

/// Whether `path` is on a local volume. Cached per device, because `statfs` on a network
/// mount can cost a round trip and a folder's entries share one device.
#[cfg(target_os = "macos")]
fn is_local_volume(path: &Path, metadata: &Metadata) -> bool {
    use std::collections::HashMap;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::sync::{LazyLock, Mutex};

    static LOCAL: LazyLock<Mutex<HashMap<u64, bool>>> = LazyLock::new(Default::default);

    let dev = metadata.dev();
    if let Some(local) = LOCAL.lock().unwrap().get(&dev) {
        return *local;
    }
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `stat` is a valid out-pointer for one `statfs`.
    let local = if unsafe { libc::statfs(c_path.as_ptr(), stat.as_mut_ptr()) } == 0 {
        // SAFETY: `statfs` returned 0, so it filled `stat`.
        is_local_flags(unsafe { stat.assume_init() }.f_flags)
    } else {
        false
    };
    LOCAL.lock().unwrap().insert(dev, local);
    local
}

/// The Finder tags on `path`. Empty on network volumes and when the file has none.
#[cfg(target_os = "macos")]
pub fn read(path: &Path, metadata: &Metadata) -> Vec<Tag> {
    if !is_local_volume(path, metadata) {
        return Vec::new();
    }
    let names_only = || {
        crate::url_values_macos::tag_names(path)
            .map(|names| names.iter().map(|name| Tag::from_name(name)).collect())
            .unwrap_or_default()
    };
    match read_xattr(path) {
        Xattr::Absent => Vec::new(),
        Xattr::Present(bytes) => parse_xattr(&bytes).unwrap_or_else(names_only),
        Xattr::Failed => names_only(),
    }
}

/// Finder tags exist only on macOS.
#[cfg(not(target_os = "macos"))]
pub fn read(_path: &Path, _metadata: &Metadata) -> Vec<Tag> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_index_table() {
        let expected = [
            TagColour::None,
            TagColour::Grey,
            TagColour::Green,
            TagColour::Purple,
            TagColour::Blue,
            TagColour::Yellow,
            TagColour::Red,
            TagColour::Orange,
        ];
        for (index, colour) in expected.into_iter().enumerate() {
            assert_eq!(TagColour::from_index(index as u8), colour);
            assert_eq!(colour.index(), index as u8);
        }
        assert_eq!(TagColour::from_index(8), TagColour::None);
        assert_eq!(TagColour::None.rgb(), None);
        assert!(expected[1..].iter().all(|colour| colour.rgb().is_some()));
    }

    #[test]
    fn standard_names_map_to_colours() {
        assert_eq!(TagColour::from_standard_name("Red"), TagColour::Red);
        assert_eq!(TagColour::from_standard_name("Gray"), TagColour::Grey);
        assert_eq!(TagColour::from_standard_name("Orange"), TagColour::Orange);
        assert_eq!(TagColour::from_standard_name("Work"), TagColour::None);
    }

    #[test]
    fn entry_parsing() {
        let tag = Tag::from_entry("Red\n6");
        assert_eq!(tag.name, "Red");
        assert_eq!(tag.colour, TagColour::Red);
        // A custom tag carries its own index.
        assert_eq!(Tag::from_entry("Work\n4").colour, TagColour::Blue);
        // A bare standard name still gets its colour; a bare custom name gets none.
        assert_eq!(Tag::from_entry("Green").colour, TagColour::Green);
        assert_eq!(Tag::from_entry("Work").colour, TagColour::None);
        // A name with a newline but no index keeps the whole string.
        assert_eq!(Tag::from_entry("a\nb").name, "a\nb");
    }

    #[test]
    fn dots_skip_colourless_and_cap() {
        let tags: Vec<Tag> = ["Work", "Red\n6", "Blue\n4", "Green\n2", "Orange\n7"]
            .into_iter()
            .map(Tag::from_entry)
            .collect();
        let dots: Vec<_> = dot_colours(&tags, 3).collect();
        assert_eq!(
            dots,
            [TagColour::Red, TagColour::Blue, TagColour::Green].to_vec()
        );
    }

    #[cfg(target_os = "macos")]
    fn binary_plist(entries: &[&str]) -> Vec<u8> {
        let value = plist::Value::Array(
            entries
                .iter()
                .map(|entry| plist::Value::String(entry.to_string()))
                .collect(),
        );
        let mut bytes = Vec::new();
        value.to_writer_binary(&mut bytes).unwrap();
        bytes
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_rejects_non_array() {
        let mut bytes = Vec::new();
        plist::Value::String("Red\n6".into())
            .to_writer_binary(&mut bytes)
            .unwrap();
        assert_eq!(parse_xattr(&bytes), None);
        assert_eq!(parse_xattr(b"not a plist"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_tags_from_temp_file_xattr() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tagged.txt");
        std::fs::write(&path, b"x").unwrap();
        let payload = binary_plist(&["Red\n6", "Project X\n4", "Plain"]);
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: valid C strings and a buffer valid for its length.
        let rc = unsafe {
            libc::setxattr(
                c_path.as_ptr(),
                TAGS_XATTR.as_ptr(),
                payload.as_ptr().cast(),
                payload.len(),
                0,
                0,
            )
        };
        assert_eq!(
            rc,
            0,
            "setxattr failed: {}",
            std::io::Error::last_os_error()
        );

        let metadata = std::fs::metadata(&path).unwrap();
        let tags = read(&path, &metadata);
        assert_eq!(
            tags,
            vec![
                Tag {
                    name: "Red".into(),
                    colour: TagColour::Red
                },
                Tag {
                    name: "Project X".into(),
                    colour: TagColour::Blue
                },
                Tag {
                    name: "Plain".into(),
                    colour: TagColour::None
                },
            ]
        );

        // Foundation reads the same attribute, so the names-only path agrees.
        assert_eq!(
            crate::url_values_macos::tag_names(&path).unwrap(),
            vec!["Red", "Project X", "Plain"]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn untagged_file_has_no_tags() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.txt");
        std::fs::write(&path, b"x").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(read(&path, &metadata).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn remote_volumes_are_skipped() {
        assert!(is_local_flags(libc::MNT_LOCAL as u32));
        assert!(is_local_flags(
            libc::MNT_LOCAL as u32 | libc::MNT_RDONLY as u32
        ));
        // An SMB or NFS mount has no MNT_LOCAL.
        assert!(!is_local_flags(0));
        assert!(!is_local_flags(libc::MNT_RDONLY as u32));
        // The temp dir is on the boot volume.
        let dir = tempfile::tempdir().unwrap();
        let metadata = std::fs::metadata(dir.path()).unwrap();
        assert!(is_local_volume(dir.path(), &metadata));
    }
}
