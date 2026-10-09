// SPDX-License-Identifier: GPL-3.0-only

//! Finder's naming for Duplicate (Cmd+D) and New Folder with Selection.
//!
//! The rules below were checked against Finder on macOS 26 by duplicating sample files through
//! AppleScript:
//!
//! | Original              | Finder's duplicate        |
//! |-----------------------|---------------------------|
//! | `a.txt`               | `a copy.txt`              |
//! | `a.txt`, again        | `a copy 2.txt`            |
//! | `a copy.txt`          | `a copy 2.txt`            |
//! | `y copy 7.txt`        | `y copy 8.txt`            |
//! | `archive.tar.gz`      | `archive copy.tar.gz`     |
//! | `report.final.pdf`    | `report.final copy.pdf`   |
//! | `k.tar.zst`           | `k.tar.zst copy`          |
//! | `h.weirdext`          | `h.weirdext copy`         |
//! | `.bashrc`             | `.bashrc copy`            |
//! | `Folder`              | `Folder copy`             |
//! | `Thing.app` (folder)  | `Thing copy.app`          |
//!
//! So Finder peels extensions off the end for as long as the system recognises each one as a
//! file type, and puts " copy" in front of what it peeled. `tar.gz` stays whole because both
//! parts are known types; `final.pdf` splits because "final" is not one. An unknown last
//! extension, such as `zst`, means the name has no extension at all.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::fl;

/// Split `name` into the part " copy" goes after and the extension it goes before.
///
/// The extension keeps its leading dot and may hold several parts, such as `.tar.gz`. A leading
/// dot never starts an extension, so `.bashrc` is all stem.
pub fn split_extension(name: &str, is_known_ext: impl Fn(&str) -> bool) -> (&str, &str) {
    let mut end = name.len();
    while let Some(dot) = name[..end].rfind('.') {
        let ext = &name[dot + 1..end];
        if dot == 0 || ext.is_empty() || !is_known_ext(ext) {
            break;
        }
        end = dot;
    }
    name.split_at(end)
}

/// The first free Finder-style duplicate name for `name`.
///
/// `word` is the localised "copy". `taken` reports whether a candidate name already exists.
pub fn duplicate_name(
    name: &str,
    word: &str,
    is_known_ext: impl Fn(&str) -> bool,
    taken: impl Fn(&str) -> bool,
) -> String {
    let (stem, ext) = split_extension(name, is_known_ext);
    let (base, mut n) = strip_copy_suffix(stem, word);
    loop {
        let candidate = if n == 1 {
            format!("{base} {word}{ext}")
        } else {
            format!("{base} {word} {n}{ext}")
        };
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Undo an earlier duplicate's suffix, so that duplicating `a copy` gives `a copy 2`, not
/// `a copy copy`. Returns the base name and the first number to try; 1 means a bare " copy".
fn strip_copy_suffix<'a>(stem: &'a str, word: &str) -> (&'a str, u64) {
    let suffix = format!(" {word}");
    if let Some(base) = stem.strip_suffix(&suffix)
        && !base.is_empty()
    {
        return (base, 2);
    }
    if let Some((head, number)) = stem.rsplit_once(' ')
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
        && let Ok(number) = number.parse::<u64>()
        && let Some(base) = head.strip_suffix(&suffix)
        && !base.is_empty()
    {
        return (base, number.saturating_add(1));
    }
    (stem, 1)
}

/// The path Duplicate copies `path` to: next to it, with Finder's name.
///
/// `reserved` holds paths already handed out in the same batch, which do not exist on disk yet.
/// Returns `None` for a path with no parent or a name that is not UTF-8.
pub fn duplicate_path(path: &Path, reserved: &HashSet<PathBuf>) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?.to_str()?;
    let word = fl!("duplicate-name-suffix");
    let new_name = duplicate_name(name, &word, is_known_extension, |candidate| {
        let candidate = parent.join(candidate);
        reserved.contains(&candidate) || candidate.symlink_metadata().is_ok()
    });
    Some(parent.join(new_name))
}

/// The path for New Folder with Selection's folder in `parent`: Finder's "New Folder With
/// Items", then "New Folder With Items 2" and so on.
pub fn new_folder_with_items_path(parent: &Path) -> PathBuf {
    let name = fl!("new-folder-with-items");
    let mut path = parent.join(&name);
    let mut n = 2u64;
    while path.symlink_metadata().is_ok() {
        path = parent.join(format!("{name} {n}"));
        n += 1;
    }
    path
}

/// Whether macOS recognises `ext` as a file type, the test Finder applies.
///
/// A type is recognised when Uniform Type Identifiers resolve it to a declared type, as data
/// (`txt`) or as a package (`rtfd`). Unknown extensions resolve to a dynamic `dyn.` type.
#[cfg(all(target_os = "macos", feature = "quicklook"))]
pub fn is_known_extension(ext: &str) -> bool {
    use objc2_foundation::NSString;
    use objc2_uniform_type_identifiers::{UTType, UTTypeDirectory};

    let ext = NSString::from_str(ext);
    let declared = |ty: Option<objc2::rc::Retained<UTType>>| ty.is_some_and(|ty| !ty.isDynamic());
    // SAFETY: `UTTypeDirectory` is an immutable framework constant.
    let directory = unsafe { UTTypeDirectory };
    declared(UTType::typeWithFilenameExtension(&ext))
        || declared(UTType::typeWithFilenameExtension_conformingToType(
            &ext, directory,
        ))
}

/// Whether `ext` names a known file type. Without Uniform Type Identifiers this asks the MIME
/// database, which agrees with macOS on common types.
#[cfg(not(all(target_os = "macos", feature = "quicklook")))]
pub fn is_known_extension(ext: &str) -> bool {
    mime_guess::from_ext(ext).first().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed type table, so the tests do not depend on what this machine has installed.
    fn known(ext: &str) -> bool {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "txt" | "pdf" | "gz" | "tar" | "bz2" | "app" | "jpg"
        )
    }

    fn dup(name: &str, existing: &[&str]) -> String {
        duplicate_name(name, "copy", known, |c| existing.contains(&c))
    }

    #[test]
    fn single_extension() {
        assert_eq!(dup("a.txt", &[]), "a copy.txt");
        assert_eq!(dup("notes.TXT", &[]), "notes copy.TXT");
    }

    #[test]
    fn numbering_continues_past_taken_names() {
        assert_eq!(dup("a.txt", &["a copy.txt"]), "a copy 2.txt");
        assert_eq!(
            dup("a.txt", &["a copy.txt", "a copy 2.txt"]),
            "a copy 3.txt"
        );
    }

    #[test]
    fn no_extension_and_folders() {
        assert_eq!(dup("noext", &[]), "noext copy");
        assert_eq!(dup("Folder", &["Folder copy"]), "Folder copy 2");
    }

    #[test]
    fn dotfiles_keep_their_leading_dot() {
        assert_eq!(dup(".bashrc", &[]), ".bashrc copy");
        assert_eq!(dup(".notes.txt", &[]), ".notes copy.txt");
    }

    #[test]
    fn compound_extensions_stay_whole_when_every_part_is_known() {
        assert_eq!(dup("archive.tar.gz", &[]), "archive copy.tar.gz");
        assert_eq!(dup("j.tar.bz2", &[]), "j copy.tar.bz2");
        assert_eq!(dup("report.final.pdf", &[]), "report.final copy.pdf");
        assert_eq!(dup("v1.2.3.txt", &[]), "v1.2.3 copy.txt");
    }

    #[test]
    fn unknown_last_extension_means_no_extension() {
        assert_eq!(dup("k.tar.zst", &[]), "k.tar.zst copy");
        assert_eq!(dup("h.weirdext", &[]), "h.weirdext copy");
        assert_eq!(dup("s.", &[]), "s. copy");
    }

    #[test]
    fn duplicating_a_duplicate_bumps_its_number() {
        assert_eq!(dup("a copy.txt", &[]), "a copy 2.txt");
        assert_eq!(dup("x copy", &[]), "x copy 2");
        assert_eq!(dup("y copy 7.txt", &[]), "y copy 8.txt");
        // "copy" with no space before it is part of the name.
        assert_eq!(dup("z.copy.txt", &[]), "z.copy copy.txt");
        // A name that is only the word has nothing to strip.
        assert_eq!(dup("copy.txt", &[]), "copy copy.txt");
    }

    #[test]
    fn split_extension_cases() {
        assert_eq!(split_extension("Thing.app", known), ("Thing", ".app"));
        assert_eq!(split_extension("a.tar.gz", known), ("a", ".tar.gz"));
        assert_eq!(split_extension(".gz", known), (".gz", ""));
        assert_eq!(split_extension("gz.gz", known), ("gz", ".gz"));
    }

    #[cfg(all(target_os = "macos", feature = "quicklook"))]
    #[test]
    fn system_types_match_finder() {
        assert!(is_known_extension("txt"));
        assert!(is_known_extension("tar"));
        assert!(is_known_extension("rtfd"));
        assert!(!is_known_extension("weirdext"));
        assert!(!is_known_extension("final"));
    }

    #[test]
    fn duplicate_path_skips_existing_and_reserved_names() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        std::fs::write(dir.path().join("a copy.txt"), b"x").unwrap();
        let mut reserved = HashSet::new();
        reserved.insert(dir.path().join("a copy 2.txt"));
        assert_eq!(
            duplicate_path(&file, &reserved),
            Some(dir.path().join("a copy 3.txt"))
        );
    }

    #[test]
    fn new_folder_with_items_numbers_from_two() {
        let dir = tempfile::tempdir().unwrap();
        let first = new_folder_with_items_path(dir.path());
        assert_eq!(first, dir.path().join("New Folder With Items"));
        std::fs::create_dir(&first).unwrap();
        assert_eq!(
            new_folder_with_items_path(dir.path()),
            dir.path().join("New Folder With Items 2")
        );
    }
}
