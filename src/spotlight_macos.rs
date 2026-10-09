// SPDX-License-Identifier: GPL-3.0-only

//! Recents from Spotlight, the way Finder builds its Recents.
//!
//! Finder's Recents is a saved Spotlight search
//! (`Finder.app/Contents/Resources/MyLibraries/myDocuments.cannedSearch`). Its query keeps
//! items that have a `kMDItemLastUsedDate` and whose content type is a document, an Office
//! file or an archive. Folders and app bundles drop out because they are none of those.
//! Launch Services sets `kMDItemLastUsedDate` whenever any app opens a file, so this lists
//! files opened anywhere, not only in this app.
//!
//! The query runs through the `MDQuery` C API in one synchronous pass. The paths and dates
//! come back together, so there is no per-file `mdls` call. Callers run it off the UI
//! thread (`Location::scan` is called from `spawn_blocking`).

use std::{
    ffi::c_void,
    path::{Component, Path, PathBuf},
    ptr::NonNull,
    time::SystemTime,
};

use objc2_core_foundation::{CFArray, CFDate, CFIndex, CFRetained, CFString, CFType};

/// How far back Recents looks, in days.
pub const RECENT_DAYS: u32 = 30;
/// The most items Recents lists.
pub const MAX_RECENTS: usize = 500;

/// Finder's Recents query, limited to the last `days` days.
pub fn query_string(days: u32) -> String {
    format!(
        "kMDItemLastUsedDate >= $time.today(-{days}) && \
         ((kMDItemContentTypeTree = public.content) || \
         (kMDItemContentTypeTree = \"com.microsoft.*\"cdw) || \
         (kMDItemContentTypeTree = public.archive))"
    )
}

/// Package extensions whose contents are not user files. A Spotlight hit inside one of
/// these is an implementation detail of an app or library, so Recents skips it.
const PACKAGE_EXTENSIONS: &[&str] = &[
    "app",
    "appex",
    "bundle",
    "framework",
    "photoslibrary",
    "musiclibrary",
    "tvlibrary",
];

/// Whether a Spotlight result belongs in Recents.
///
/// Rules, in order:
/// 1. It is under `home`. The query is scoped to home, but this guards against symlinks.
/// 2. No path component below `home` is hidden (starts with `.`).
/// 3. It is not under `~/Library`, except iCloud Drive (`~/Library/Mobile Documents`),
///    which Finder's Recents also lists.
/// 4. No folder above it is a package such as an `.app` bundle.
pub fn is_listable(path: &Path, home: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(home) else {
        return false;
    };
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    // A path with a non-UTF-8 component, or home itself.
    if parts.is_empty() || parts.len() != rel.components().count() {
        return false;
    }
    if parts.iter().any(|p| p.starts_with('.')) {
        return false;
    }
    if parts[0] == "Library" && parts.get(1) != Some(&"Mobile Documents") {
        return false;
    }
    let ancestors = &parts[..parts.len() - 1];
    !ancestors.iter().any(|p| {
        Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                PACKAGE_EXTENSIONS
                    .iter()
                    .any(|pkg| e.eq_ignore_ascii_case(pkg))
            })
    })
}

/// Filter raw results, newest first, capped at `max`.
pub fn select_recents(
    mut entries: Vec<(PathBuf, SystemTime)>,
    home: &Path,
    max: usize,
) -> Vec<(PathBuf, SystemTime)> {
    entries.retain(|(path, _)| is_listable(path, home));
    // Stable sort, so ties keep Spotlight's order.
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.dedup_by(|a, b| a.0 == b.0);
    entries.truncate(max);
    entries
}

/// Files used in the last [`RECENT_DAYS`] days under the home folder, newest first.
///
/// Returns an empty list if the home folder is unknown or Spotlight fails, for example
/// when indexing is off for the home volume.
pub fn recent_files() -> Vec<(PathBuf, SystemTime)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let start = std::time::Instant::now();
    let raw = match run_query(&query_string(RECENT_DAYS), &home) {
        Some(raw) => raw,
        None => {
            log::warn!("Spotlight Recents query failed");
            return Vec::new();
        }
    };
    let raw_len = raw.len();
    let recents = select_recents(raw, &home, MAX_RECENTS);
    log::debug!(
        "Spotlight Recents: {} hits, {} listed, {:?}",
        raw_len,
        recents.len(),
        start.elapsed()
    );
    recents
}

// MDQuery lives in CoreServices' Metadata framework. objc2 has no bindings for it, so the
// handful of functions used here are declared directly. CF types cross as raw pointers.
type MDQueryRef = *mut c_void;
type CFTypeRef = *const c_void;

/// `kMDQuerySynchronous`: `MDQueryExecute` returns after the initial gathering.
const K_MD_QUERY_SYNCHRONOUS: usize = 1;

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn MDQueryCreate(
        allocator: CFTypeRef,
        query_string: CFTypeRef,
        value_list_attrs: CFTypeRef,
        sorting_attrs: CFTypeRef,
    ) -> MDQueryRef;
    fn MDQuerySetSearchScope(query: MDQueryRef, scope_directories: CFTypeRef, scope_options: u32);
    fn MDQueryExecute(query: MDQueryRef, option_flags: usize) -> u8;
    fn MDQueryGetResultCount(query: MDQueryRef) -> CFIndex;
    fn MDQueryGetResultAtIndex(query: MDQueryRef, idx: CFIndex) -> CFTypeRef;
    fn MDItemCopyAttribute(item: CFTypeRef, name: CFTypeRef) -> CFTypeRef;
    fn MDQueryGetAttributeValueOfResultAtIndex(
        query: MDQueryRef,
        name: CFTypeRef,
        idx: CFIndex,
    ) -> CFTypeRef;
}

fn cf_ptr<T: ?Sized>(value: &CFRetained<T>) -> CFTypeRef
where
    T: objc2_core_foundation::Type,
{
    let r: &T = value;
    (r as *const T).cast()
}

/// Run a Spotlight query scoped to `scope`, returning each result's path and last-used date.
/// Results without either value are skipped. `None` means the query could not run.
fn run_query(query: &str, scope: &Path) -> Option<Vec<(PathBuf, SystemTime)>> {
    let query_cf = CFString::from_str(query);
    let path_attr = CFString::from_static_str("kMDItemPath");
    let date_attr = CFString::from_static_str("kMDItemLastUsedDate");
    // Only the date goes in the value list. `kMDItemPath` comes back null from the value
    // list, so it is read from each result's MDItem instead.
    let value_attrs = CFArray::from_retained_objects(&[date_attr.clone()]);
    let scope_cf = CFString::from_str(scope.to_str()?);
    let scopes = CFArray::from_retained_objects(&[scope_cf]);

    // SAFETY: Every argument is a live CF object of the type MDQueryCreate expects. A null
    // allocator means the default allocator, and a null sorting list means unsorted.
    let raw = unsafe {
        MDQueryCreate(
            std::ptr::null(),
            cf_ptr(&query_cf),
            cf_ptr(&value_attrs),
            std::ptr::null(),
        )
    };
    // SAFETY: MDQueryCreate follows the Create rule, so we own this +1 reference. The
    // wrapper releases it on every return path below.
    let query_obj: CFRetained<CFType> = unsafe { CFRetained::from_raw(NonNull::new(raw.cast())?) };
    let q = raw;

    // SAFETY: `q` is alive (owned by `query_obj`) and `scopes` is a CFArray of CFStrings.
    let executed = unsafe {
        MDQuerySetSearchScope(q, cf_ptr(&scopes), 0);
        MDQueryExecute(q, K_MD_QUERY_SYNCHRONOUS)
    };
    if executed == 0 {
        return None;
    }

    // SAFETY: `q` is alive and has finished its synchronous gather.
    let count = unsafe { MDQueryGetResultCount(q) };
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for idx in 0..count {
        // SAFETY: `idx` is in range and the date is in the query's value list. The item and
        // the date follow the Get rule, so the date is retained to outlive the query. The
        // path follows the Copy rule, so it is adopted without an extra retain.
        let (path, date) = unsafe {
            let item = MDQueryGetResultAtIndex(q, idx);
            let path = MDItemCopyAttribute(item, cf_ptr(&path_attr));
            let date = MDQueryGetAttributeValueOfResultAtIndex(q, cf_ptr(&date_attr), idx);
            (adopt_cf(path), retain_cf(date))
        };
        let Some(path) = path.and_then(|v| v.downcast::<CFString>().ok()) else {
            continue;
        };
        let Some(date) = date
            .and_then(|v| v.downcast::<CFDate>().ok())
            .and_then(|d| d.to_system_time())
        else {
            continue;
        };
        out.push((PathBuf::from(path.to_string()), date));
    }
    drop(query_obj);
    Some(out)
}

/// Take ownership of a +1 CF value, or `None` for null.
///
/// # Safety
/// `ptr` must be null or a CF object the caller owns a reference to.
unsafe fn adopt_cf(ptr: CFTypeRef) -> Option<CFRetained<CFType>> {
    let ptr = NonNull::new(ptr.cast_mut().cast::<CFType>())?;
    // SAFETY: The caller hands over its reference.
    Some(unsafe { CFRetained::from_raw(ptr) })
}

/// Retain a borrowed CF value, or `None` for null.
///
/// # Safety
/// `ptr` must be null or point to a live CF object.
unsafe fn retain_cf(ptr: CFTypeRef) -> Option<CFRetained<CFType>> {
    let ptr = NonNull::new(ptr.cast_mut().cast::<CFType>())?;
    // SAFETY: The caller guarantees a live CF object.
    Some(unsafe { CFRetained::retain(ptr) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn home() -> PathBuf {
        PathBuf::from("/Users/someone")
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn query_matches_finder_recents() {
        let q = query_string(30);
        assert!(q.starts_with("kMDItemLastUsedDate >= $time.today(-30) && "));
        assert!(q.contains("kMDItemContentTypeTree = public.content"));
        assert!(q.contains("kMDItemContentTypeTree = public.archive"));
        assert!(q.contains(r#"kMDItemContentTypeTree = "com.microsoft.*"cdw"#));
    }

    #[test]
    fn listable_rules() {
        let h = home();
        let cases = [
            ("/Users/someone/Desktop/a.png", true),
            ("/Users/someone/Downloads/sub/b.pdf", true),
            (
                "/Users/someone/Library/Mobile Documents/com~apple~CloudDocs/c.jpg",
                true,
            ),
            ("/Users/someone/Library/Caches/x.json", false),
            ("/Users/someone/Library", false),
            ("/Users/someone/.config/app/settings.toml", false),
            ("/Users/someone/dev/repo/.git/HEAD", false),
            ("/Users/someone/Desktop/.hidden.txt", false),
            (
                "/Users/someone/Applications/Foo.app/Contents/Info.plist",
                false,
            ),
            (
                "/Users/someone/Pictures/Photos Library.photoslibrary/db.sqlite",
                false,
            ),
            ("/Users/someone/dev/Thing.APP/readme.txt", false),
            // A package itself is fine; only its contents are skipped.
            ("/Users/someone/Downloads/archive.app.zip", true),
            ("/Users/someone", false),
            ("/Users/other/Desktop/a.png", false),
            ("/tmp/a.png", false),
        ];
        for (path, want) in cases {
            assert_eq!(is_listable(Path::new(path), &h), want, "{path}");
        }
    }

    #[test]
    fn select_filters_sorts_and_caps() {
        let h = home();
        let entries = vec![
            (PathBuf::from("/Users/someone/Desktop/old.txt"), at(100)),
            (PathBuf::from("/Users/someone/Library/Caches/c"), at(999)),
            (PathBuf::from("/Users/someone/Desktop/new.txt"), at(300)),
            (PathBuf::from("/Users/someone/Desktop/mid.txt"), at(200)),
            (PathBuf::from("/Users/someone/Desktop/new.txt"), at(300)),
        ];
        let got: Vec<_> = select_recents(entries.clone(), &h, 10)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(
            got,
            [
                PathBuf::from("/Users/someone/Desktop/new.txt"),
                PathBuf::from("/Users/someone/Desktop/mid.txt"),
                PathBuf::from("/Users/someone/Desktop/old.txt"),
            ]
        );
        assert_eq!(select_recents(entries, &h, 2).len(), 2);
    }

    /// Live: the real query runs against this Mac's index and returns only listable
    /// paths under home, newest first. An empty result passes, because a fresh account or
    /// one with Spotlight off has nothing to list.
    #[test]
    fn live_query_returns_home_paths() {
        let home = dirs::home_dir().expect("home dir");
        let start = std::time::Instant::now();
        let recents = recent_files();
        let elapsed = start.elapsed();
        eprintln!(
            "Spotlight Recents: {} items in {:?}",
            recents.len(),
            elapsed
        );
        if let Some((newest, _)) = recents.first() {
            eprintln!("newest: {}", newest.display());
        }
        assert!(recents.len() <= MAX_RECENTS);
        for (path, _) in &recents {
            assert!(path.starts_with(&home), "{}", path.display());
            assert!(is_listable(path, &home), "{}", path.display());
        }
        assert!(recents.windows(2).all(|w| w[0].1 >= w[1].1));
    }

    /// Live: MDQuery returns a path and a date for every hit the `mdfind` CLI finds with
    /// the same query. This catches a result that silently drops values, which would
    /// otherwise look like an empty Recents.
    #[test]
    fn live_query_matches_mdfind() {
        let home = dirs::home_dir().expect("home dir");
        let query = query_string(RECENT_DAYS);
        let ours = run_query(&query, &home).expect("MDQuery runs");
        let output = std::process::Command::new("mdfind")
            .arg("-0")
            .arg("-onlyin")
            .arg(&home)
            .arg(&query)
            .output()
            .expect("mdfind runs");
        let mut theirs: Vec<PathBuf> = output
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
            .collect();
        let mut ours: Vec<PathBuf> = ours.into_iter().map(|(p, _)| p).collect();
        ours.sort();
        theirs.sort();
        // The index can change between the two queries, so allow a little drift.
        let missing = theirs
            .iter()
            .filter(|p| ours.binary_search(p).is_err())
            .count();
        eprintln!(
            "MDQuery {} hits, mdfind {} hits, {missing} missing",
            ours.len(),
            theirs.len()
        );
        assert!(
            missing <= 2,
            "{missing} of {} mdfind hits missing",
            theirs.len()
        );
    }
}
