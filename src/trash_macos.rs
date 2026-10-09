//! Trash and Put Back on macOS.
//!
//! `trash::delete` asks Finder over AppleScript, one `osascript` process per file, and a
//! bundled build has no Automation permission to do it. This module calls
//! `NSFileManager trashItemAtURL:resultingItemURL:error:` instead.
//!
//! macOS keeps no record of where a trashed item came from that a non-Finder app can read.
//! So every trash writes a journal entry: original path to trashed path. Undo, Restore and
//! the Trash view all read that journal. Items trashed by Finder are not in it.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use objc2_foundation::{NSFileManager, NSString, NSURL};
use serde::{Deserialize, Serialize};

/// The journal file name inside the app's data directory.
const JOURNAL_FILE: &str = "trash-journal.json";

/// The oldest entries go once the journal holds this many, so it cannot grow without bound
/// when their trashed files can't be checked.
const MAX_ENTRIES: usize = 10_000;

/// Serialises read-modify-write of the journal file within this process.
static JOURNAL_LOCK: Mutex<()> = Mutex::new(());

/// One trashed item: where it was, and where the Trash put it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub original: PathBuf,
    pub trashed: PathBuf,
    /// Seconds since the Unix epoch.
    pub time_deleted: i64,
}

impl JournalEntry {
    /// The entry as the `trash` crate's item type, which the Trash view and Restore use.
    /// The id is the trashed path.
    pub fn to_trash_item(&self) -> trash::TrashItem {
        trash::TrashItem {
            id: self.trashed.clone().into_os_string(),
            name: self
                .original
                .file_name()
                .map_or_else(OsString::new, ToOwned::to_owned),
            original_parent: self
                .original
                .parent()
                .map_or_else(PathBuf::new, ToOwned::to_owned),
            time_deleted: self.time_deleted,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default)]
    pub entries: Vec<JournalEntry>,
}

impl Journal {
    /// Reads the journal. A missing file is an empty journal. A corrupt one is logged and
    /// treated as empty, because a lost Put Back record must not block trashing.
    pub fn load(path: &Path) -> Self {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                log::warn!("ignoring corrupt trash journal {}: {err}", path.display());
                Self::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                log::warn!("failed to read trash journal {}: {err}", path.display());
                Self::default()
            }
        }
    }

    /// Writes the journal through a temp file and a rename, so a crash leaves the old one.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)
    }

    /// Drops entries whose trashed file is gone: emptied, or put back by Finder.
    pub fn prune(&mut self) {
        self.entries.retain(|entry| trashed_exists(&entry.trashed));
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..excess);
        }
    }

    /// The newest entry for each original path, in the order given. Paths with no entry
    /// are skipped.
    pub fn latest_for(&self, originals: &[PathBuf]) -> Vec<JournalEntry> {
        originals
            .iter()
            .filter_map(|original| {
                self.entries
                    .iter()
                    .rev()
                    .find(|entry| &entry.original == original)
                    .cloned()
            })
            .collect()
    }
}

/// Whether a trashed item still exists. Only a definite "not found" counts as gone: without
/// Full Disk Access, macOS may refuse to stat inside `~/.Trash`, and that must not drop the
/// record.
fn trashed_exists(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(err) => err.kind() != io::ErrorKind::NotFound,
    }
}

/// `~/Library/Application Support/<app id>/trash-journal.json`.
pub fn journal_path() -> Option<PathBuf> {
    use cosmic::Application;
    Some(
        dirs::data_dir()?
            .join(crate::app::App::APP_ID)
            .join(JOURNAL_FILE),
    )
}

/// Loads and prunes the journal, applies `f`, and saves it if `f` changed it.
fn with_journal<R>(path: &Path, f: impl FnOnce(&mut Journal) -> R) -> io::Result<R> {
    let _guard = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut journal = Journal::load(path);
    let before = journal.clone();
    journal.prune();
    let result = f(&mut journal);
    if journal != before {
        journal.save(path)?;
    }
    Ok(result)
}

/// The journal's current entries, pruned.
pub fn entries(journal: &Path) -> Vec<JournalEntry> {
    with_journal(journal, |j| j.entries.clone()).unwrap_or_else(|err| {
        log::warn!("failed to update trash journal: {err}");
        Vec::new()
    })
}

/// Appends entries to the journal.
pub fn record(journal: &Path, new: &[JournalEntry]) -> io::Result<()> {
    if new.is_empty() {
        return Ok(());
    }
    with_journal(journal, |j| j.entries.extend_from_slice(new))
}

/// Removes the entries for these trashed paths.
pub fn forget(journal: &Path, trashed: &[PathBuf]) -> io::Result<()> {
    with_journal(journal, |j| {
        j.entries.retain(|entry| !trashed.contains(&entry.trashed))
    })
}

/// The trash items for the newest journal entry of each original path. This is what the
/// Undo button on the trash toast restores.
pub fn items_for_originals(journal: &Path, originals: &[PathBuf]) -> Vec<trash::TrashItem> {
    with_journal(journal, |j| j.latest_for(originals))
        .unwrap_or_else(|err| {
            log::warn!("failed to update trash journal: {err}");
            Vec::new()
        })
        .iter()
        .map(JournalEntry::to_trash_item)
        .collect()
}

/// Moves one item to the Trash and returns where it landed.
pub fn trash_one(path: &Path) -> io::Result<PathBuf> {
    let path_str = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path is not UTF-8: {}", path.display()),
        )
    })?;

    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path_str));
        let mut resulting: Option<objc2::rc::Retained<NSURL>> = None;
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
            .map_err(|err| io::Error::other(err.localizedDescription().to_string()))?;
        resulting
            .and_then(|url| url.path())
            .map(|p| PathBuf::from(p.to_string()))
            .ok_or_else(|| io::Error::other("the Trash returned no location for the item"))
    })
}

/// Trashes each path in order, calling `before_each(index)` first so the caller can report
/// progress and stop. Every item that made it to the Trash is journaled, even when a later
/// one fails or the caller stops.
pub fn trash_paths<E>(
    journal: &Path,
    paths: &[PathBuf],
    mut before_each: impl FnMut(usize) -> Result<(), E>,
    io_err: impl Fn(io::Error) -> E,
) -> Result<Vec<JournalEntry>, E> {
    let mut done = Vec::with_capacity(paths.len());
    let mut failure = None;
    for (i, path) in paths.iter().enumerate() {
        if let Err(err) = before_each(i) {
            failure = Some(err);
            break;
        }
        match trash_one(path) {
            Ok(trashed) => done.push(JournalEntry {
                original: path.clone(),
                trashed,
                time_deleted: now(),
            }),
            Err(err) => {
                failure = Some(io_err(io::Error::new(
                    err.kind(),
                    format!("{}: {err}", path.display()),
                )));
                break;
            }
        }
    }

    if let Err(err) = record(journal, &done) {
        log::warn!("failed to record trashed items in the journal: {err}");
    }

    match failure {
        Some(err) => Err(err),
        None => Ok(done),
    }
}

/// A free path for `path`: itself if nothing is there, else "name 2.ext", "name 3.ext" and
/// so on, as Finder names copies. Folders keep any dot in their name.
pub fn unique_path(path: &Path) -> PathBuf {
    if fs::symlink_metadata(path).is_err() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let is_dir = path.is_dir();
    let (stem, ext) = match (path.file_stem(), path.extension()) {
        (Some(stem), Some(ext)) if !is_dir => (stem.to_os_string(), Some(ext.to_os_string())),
        _ => (
            path.file_name()
                .map_or_else(OsString::new, ToOwned::to_owned),
            None,
        ),
    };
    (2u64..)
        .map(|n| {
            let mut name = stem.clone();
            name.push(format!(" {n}"));
            if let Some(ext) = &ext {
                name.push(".");
                name.push(ext);
            }
            parent.join(name)
        })
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .expect("an unbounded range always finds a free name")
}

/// Moves a trashed item back to `original`, creating missing parent folders. If something
/// already sits at `original`, the item comes back as "name 2". Returns where it went.
pub fn restore_one(trashed: &Path, original: &Path) -> io::Result<PathBuf> {
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent)?;
    }
    let target = unique_path(original);
    fs::rename(trashed, &target)?;
    Ok(target)
}

/// Restores trash items and forgets their journal entries. The item id is the trashed path.
/// Stops at the first failure; items restored before it stay restored and forgotten.
pub fn restore_items<E>(
    journal: &Path,
    items: &[trash::TrashItem],
    mut before_each: impl FnMut(usize) -> Result<(), E>,
    io_err: impl Fn(io::Error) -> E,
) -> Result<Vec<PathBuf>, E> {
    let mut restored = Vec::with_capacity(items.len());
    let mut forgotten = Vec::with_capacity(items.len());
    let mut failure = None;
    for (i, item) in items.iter().enumerate() {
        if let Err(err) = before_each(i) {
            failure = Some(err);
            break;
        }
        let trashed = PathBuf::from(&item.id);
        match restore_one(&trashed, &item.original_path()) {
            Ok(target) => {
                restored.push(target);
                forgotten.push(trashed);
            }
            Err(err) => {
                failure = Some(io_err(io::Error::new(
                    err.kind(),
                    format!("{}: {err}", item.original_path().display()),
                )));
                break;
            }
        }
    }

    if let Err(err) = forget(journal, &forgotten) {
        log::warn!("failed to remove restored items from the journal: {err}");
    }

    match failure {
        Some(err) => Err(err),
        None => Ok(restored),
    }
}

/// The journaled items for the Trash view, with sizes where the trashed item can be read.
pub fn list(journal: &Path) -> Vec<(trash::TrashItem, trash::TrashItemMetadata)> {
    entries(journal)
        .iter()
        .map(|entry| {
            let size = match fs::symlink_metadata(&entry.trashed) {
                Ok(m) if m.is_dir() => trash::TrashItemSize::Entries(
                    fs::read_dir(&entry.trashed).map_or(0, Iterator::count),
                ),
                Ok(m) => trash::TrashItemSize::Bytes(m.len()),
                // Unreadable without Full Disk Access. Guess the kind from the original name.
                Err(_) if entry.original.extension().is_none() => trash::TrashItemSize::Entries(0),
                Err(_) => trash::TrashItemSize::Bytes(0),
            };
            (entry.to_trash_item(), trash::TrashItemMetadata { size })
        })
        .collect()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("macosmetic-trash-test-")
            .tempdir()
            .unwrap()
    }

    fn entry(original: &str, trashed: &str, time: i64) -> JournalEntry {
        JournalEntry {
            original: original.into(),
            trashed: trashed.into(),
            time_deleted: time,
        }
    }

    #[test]
    fn journal_round_trips_through_disk() {
        let dir = temp_dir();
        let path = dir.path().join("nested").join(JOURNAL_FILE);
        let journal = Journal {
            entries: vec![
                entry("/a/one.txt", "/t/one.txt", 1),
                entry("/a/two", "/t/two", 2),
            ],
        };
        journal.save(&path).unwrap();
        assert_eq!(Journal::load(&path), journal);
    }

    #[test]
    fn missing_or_corrupt_journal_loads_empty() {
        let dir = temp_dir();
        let path = dir.path().join(JOURNAL_FILE);
        assert_eq!(Journal::load(&path), Journal::default());
        fs::write(&path, b"{not json").unwrap();
        assert_eq!(Journal::load(&path), Journal::default());
    }

    #[test]
    fn prune_drops_entries_whose_trashed_file_is_gone() {
        let dir = temp_dir();
        let kept = dir.path().join("kept");
        fs::write(&kept, b"x").unwrap();
        let mut journal = Journal {
            entries: vec![
                entry("/a/kept", kept.to_str().unwrap(), 1),
                entry("/a/gone", dir.path().join("gone").to_str().unwrap(), 2),
            ],
        };
        journal.prune();
        assert_eq!(journal.entries.len(), 1);
        assert_eq!(journal.entries[0].trashed, kept);
    }

    #[test]
    fn latest_for_picks_the_newest_entry_per_original() {
        let journal = Journal {
            entries: vec![
                entry("/a/f.txt", "/t/f.txt", 1),
                entry("/a/g.txt", "/t/g.txt", 2),
                entry("/a/f.txt", "/t/f 2.txt", 3),
            ],
        };
        let found = journal.latest_for(&["/a/f.txt".into(), "/a/missing".into()]);
        assert_eq!(found, vec![entry("/a/f.txt", "/t/f 2.txt", 3)]);
    }

    #[test]
    fn trash_item_carries_original_location() {
        let item = entry("/a/b/c.txt", "/t/c.txt", 7).to_trash_item();
        assert_eq!(item.original_path(), PathBuf::from("/a/b/c.txt"));
        assert_eq!(PathBuf::from(&item.id), PathBuf::from("/t/c.txt"));
        assert_eq!(item.time_deleted, 7);
    }

    #[test]
    fn unique_path_names_conflicts_like_finder() {
        let dir = temp_dir();
        let file = dir.path().join("report.txt");
        assert_eq!(unique_path(&file), file);
        fs::write(&file, b"x").unwrap();
        assert_eq!(unique_path(&file), dir.path().join("report 2.txt"));
        fs::write(dir.path().join("report 2.txt"), b"x").unwrap();
        assert_eq!(unique_path(&file), dir.path().join("report 3.txt"));

        let folder = dir.path().join("photos.2024");
        fs::create_dir(&folder).unwrap();
        assert_eq!(unique_path(&folder), dir.path().join("photos.2024 2"));

        let bare = dir.path().join("Makefile");
        fs::write(&bare, b"x").unwrap();
        assert_eq!(unique_path(&bare), dir.path().join("Makefile 2"));
    }

    #[test]
    fn restore_moves_back_and_creates_parents() {
        let dir = temp_dir();
        let trashed = dir.path().join("trashed.txt");
        fs::write(&trashed, b"payload").unwrap();
        let original = dir.path().join("gone").join("deeper").join("note.txt");

        let target = restore_one(&trashed, &original).unwrap();
        assert_eq!(target, original);
        assert_eq!(fs::read(&original).unwrap(), b"payload");
        assert!(!trashed.exists());
    }

    #[test]
    fn restore_onto_an_existing_name_picks_name_2() {
        let dir = temp_dir();
        let original = dir.path().join("note.txt");
        fs::write(&original, b"new").unwrap();
        let trashed = dir.path().join("trashed.txt");
        fs::write(&trashed, b"old").unwrap();

        let target = restore_one(&trashed, &original).unwrap();
        assert_eq!(target, dir.path().join("note 2.txt"));
        assert_eq!(fs::read(&original).unwrap(), b"new");
        assert_eq!(fs::read(&target).unwrap(), b"old");
    }

    #[test]
    fn restore_items_forgets_restored_entries() {
        let dir = temp_dir();
        let journal = dir.path().join(JOURNAL_FILE);
        let trashed = dir.path().join("trashed.txt");
        fs::write(&trashed, b"x").unwrap();
        let original = dir.path().join("back.txt");
        let e = JournalEntry {
            original: original.clone(),
            trashed: trashed.clone(),
            time_deleted: 1,
        };
        record(&journal, &[e]).unwrap();

        let items = items_for_originals(&journal, &[original.clone()]);
        assert_eq!(items.len(), 1);
        let restored = restore_items(&journal, &items, |_| Ok::<_, io::Error>(()), |e| e).unwrap();
        assert_eq!(restored, vec![original.clone()]);
        assert!(original.exists());
        assert!(Journal::load(&journal).entries.is_empty());
    }

    /// Uses the real Trash. The file is uniquely named and comes back out before the test
    /// ends, so the user's Trash is left as it was.
    #[test]
    fn trashing_a_temp_file_lands_in_the_trash_and_comes_back() {
        let dir = temp_dir();
        let journal = dir.path().join(JOURNAL_FILE);
        let name = format!(
            "macosmetic-trash-test-{}-{}.txt",
            std::process::id(),
            fastrand::u64(..)
        );
        let original = dir.path().join(&name);
        fs::write(&original, b"round trip").unwrap();

        let done = trash_paths(&journal, &[original.clone()], |_| Ok(()), |e| e).unwrap();
        assert_eq!(done.len(), 1);
        let trashed = done[0].trashed.clone();
        assert!(!original.exists(), "the original should be gone");
        assert!(
            trashed.components().any(|c| {
                let c = c.as_os_str();
                c == ".Trash" || c == ".Trashes"
            }),
            "landed outside a Trash folder: {}",
            trashed.display()
        );
        assert_eq!(Journal::load(&journal).entries, done);

        let items = items_for_originals(&journal, &[original.clone()]);
        let restored = restore_items(&journal, &items, |_| Ok::<_, io::Error>(()), |e| e);
        if restored.is_err() {
            // Leave nothing behind in the user's Trash even if the restore under test broke.
            let _ = fs::rename(&trashed, &original);
        }
        assert_eq!(restored.unwrap(), vec![original.clone()]);
        assert_eq!(fs::read(&original).unwrap(), b"round trip");
        assert!(Journal::load(&journal).entries.is_empty());
    }

    /// The spec's acceptance count: 200 files in one pass, one progress call each. All come
    /// back out before the test ends.
    #[test]
    fn trashing_200_files_is_one_pass_and_all_come_back() {
        const COUNT: usize = 200;
        let dir = temp_dir();
        let journal = dir.path().join(JOURNAL_FILE);
        let tag = format!("{}-{}", std::process::id(), fastrand::u64(..));
        let originals: Vec<PathBuf> = (0..COUNT)
            .map(|i| {
                let path = dir
                    .path()
                    .join(format!("macosmetic-trash-test-{tag}-{i}.txt"));
                fs::write(&path, i.to_string()).unwrap();
                path
            })
            .collect();

        let mut progress_calls = 0;
        let done = trash_paths(
            &journal,
            &originals,
            |_| {
                progress_calls += 1;
                Ok(())
            },
            |e| e,
        );
        let put_back_everything = || {
            for entry in Journal::load(&journal).entries {
                let _ = fs::rename(&entry.trashed, &entry.original);
            }
        };
        let done = done.unwrap_or_else(|err| {
            put_back_everything();
            panic!("trashing failed: {err}");
        });
        assert_eq!(done.len(), COUNT);
        assert_eq!(progress_calls, COUNT);
        assert_eq!(Journal::load(&journal).entries, done);

        let items = items_for_originals(&journal, &originals);
        let restored = restore_items(&journal, &items, |_| Ok::<_, io::Error>(()), |e| e)
            .unwrap_or_else(|err| {
                put_back_everything();
                panic!("restoring failed: {err}");
            });
        assert_eq!(restored, originals);
        for (i, entry) in done.iter().enumerate() {
            assert!(!entry.trashed.exists());
            assert_eq!(fs::read_to_string(&originals[i]).unwrap(), i.to_string());
        }
        assert!(Journal::load(&journal).entries.is_empty());
    }
}
