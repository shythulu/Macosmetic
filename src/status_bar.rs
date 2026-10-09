// SPDX-License-Identifier: GPL-3.0-only

//! The Finder-style status bar at the bottom of the window: how many items the current
//! location shows, what is selected and how big it is, and the space left on its volume.
//!
//! Free space is read off the UI thread and cached per location, so drawing the bar never
//! touches the disk. The cache refreshes when a location is scanned and when an operation
//! finishes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use cosmic::{Element, Task, theme};

use crate::app::Message as AppMessage;
use crate::fl;
use crate::tab::{Mode, Tab};

#[derive(Clone, Debug)]
pub enum Message {
    /// Free space measured for a location path. `None` when the volume did not report it.
    FreeSpace(PathBuf, Option<u64>),
    /// Show or hide the bar. The app persists this in its config.
    Toggle,
}

/// Free space per location path, as last measured.
#[derive(Debug, Default)]
pub struct StatusBar {
    free_space: HashMap<PathBuf, u64>,
}

impl StatusBar {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::FreeSpace(path, Some(bytes)) => {
                self.free_space.insert(path, bytes);
            }
            Message::FreeSpace(path, None) => {
                self.free_space.remove(&path);
            }
            Message::Toggle => {}
        }
    }

    pub fn free_space(&self, path: &Path) -> Option<u64> {
        self.free_space.get(path).copied()
    }
}

/// Measure the free space for `path` on a blocking thread. A location without a path, such
/// as the trash, needs no measurement.
pub fn refresh(path: Option<PathBuf>) -> Task<Message> {
    let Some(path) = path else {
        return Task::none();
    };
    cosmic::task::future(async move {
        let probe = path.clone();
        let bytes = tokio::task::spawn_blocking(move || available_space(&probe))
            .await
            .ok()
            .flatten();
        Message::FreeSpace(path, bytes)
    })
}

/// The bar for `tab`, or `None` where it does not belong (the desktop and dialogs).
pub fn view<'a>(tab: &Tab, status_bar: &StatusBar) -> Option<Element<'a, AppMessage>> {
    if !matches!(tab.mode, Mode::App) {
        return None;
    }

    let show_hidden = tab.config.show_hidden;
    let mut count = 0;
    let mut selected = 0;
    let mut selected_bytes = None;
    for item in tab.items_opt.iter().flatten() {
        if !item.shown(show_hidden) {
            continue;
        }
        count += 1;
        if item.selected {
            selected += 1;
            // Folder sizes are not counted, like Finder.
            if let Some(size) = item.metadata.file_size() {
                *selected_bytes.get_or_insert(0) += size;
            }
        }
    }

    let free = tab
        .location
        .path_opt()
        .and_then(|path| status_bar.free_space(path));

    let cosmic::cosmic_theme::Spacing {
        space_xxs, space_s, ..
    } = theme::spacing();

    let mut row = vec![
        widget::text::caption(items_text(count, selected, selected_bytes)).into(),
        widget::space::horizontal().into(),
    ];
    if let Some(free) = free {
        row.push(widget::text::caption(free_space_text(free)).into());
    }

    Some(
        widget::layer_container(
            widget::row::with_children(row)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        )
        .padding([space_xxs, space_s])
        .layer(cosmic::cosmic_theme::Layer::Primary)
        .into(),
    )
}

/// "12 items", or "3 of 12 selected, 12.4 MB" with a selection. The size is left out when
/// only folders are selected.
pub fn items_text(count: usize, selected: usize, selected_bytes: Option<u64>) -> String {
    if selected == 0 {
        return fl!("status-items", count = count);
    }
    match selected_bytes {
        Some(bytes) => fl!(
            "status-selected-size",
            selected = selected,
            count = count,
            size = format_bytes(bytes)
        ),
        None => fl!("status-selected", selected = selected, count = count),
    }
}

/// "245.33 GB available".
pub fn free_space_text(bytes: u64) -> String {
    fl!("status-available", size = format_bytes(bytes))
}

/// A byte count in Finder's style: decimal units, whole kilobytes, one decimal for
/// megabytes, two for gigabytes and up.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1e3;
    const MB: f64 = 1e6;
    const GB: f64 = 1e9;
    const TB: f64 = 1e12;

    let b = bytes as f64;
    if bytes < 1000 {
        fl!("status-bytes", count = bytes)
    } else if b < MB {
        format!("{:.0} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else if b < TB {
        format!("{:.2} GB", b / GB)
    } else {
        format!("{:.2} TB", b / TB)
    }
}

/// Bytes available on the volume holding `path`.
///
/// On macOS this is `NSURLVolumeAvailableCapacityForImportantUsageKey`, the number Finder
/// shows. It counts purgeable space, so it is larger than `df`. Elsewhere, or when that key
/// fails, it is `statvfs`'s space available to unprivileged users.
pub fn available_space(path: &Path) -> Option<u64> {
    #[cfg(target_os = "macos")]
    if let Some(bytes) = important_usage_capacity(path) {
        return Some(bytes);
    }
    statvfs_available(path)
}

#[cfg(target_os = "macos")]
fn important_usage_capacity(path: &Path) -> Option<u64> {
    use objc2_foundation::{
        NSNumber, NSString, NSURL, NSURLVolumeAvailableCapacityForImportantUsageKey,
    };

    let path = path.to_str()?;
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let mut value = None;
        // SAFETY: the key is a valid Foundation constant, and the value it yields is an
        // NSNumber, which the downcast checks.
        unsafe {
            url.getResourceValue_forKey_error(
                &mut value,
                NSURLVolumeAvailableCapacityForImportantUsageKey,
            )
        }
        .ok()?;
        let number = value?.downcast::<NSNumber>().ok()?;
        u64::try_from(number.longLongValue()).ok()
    })
}

#[cfg(unix)]
fn statvfs_available(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is NUL-terminated and `stat` is a valid out pointer.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(not(unix))]
fn statvfs_available(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fluent wraps placeables in Unicode isolation marks; strip them to compare text.
    fn plain(s: String) -> String {
        s.replace(['\u{2068}', '\u{2069}'], "")
    }

    #[test]
    fn item_counts_use_plurals() {
        assert_eq!(plain(items_text(0, 0, None)), "0 items");
        assert_eq!(plain(items_text(1, 0, None)), "1 item");
        assert_eq!(plain(items_text(12, 0, None)), "12 items");
    }

    #[test]
    fn selection_shows_count_and_size() {
        assert_eq!(
            plain(items_text(12, 3, Some(12_400_000))),
            "3 of 12 selected, 12.4 MB"
        );
        assert_eq!(plain(items_text(1, 1, Some(1))), "1 of 1 selected, 1 byte");
    }

    #[test]
    fn selection_of_only_folders_has_no_size() {
        assert_eq!(plain(items_text(5, 2, None)), "2 of 5 selected");
    }

    #[test]
    fn sizes_follow_finder_precision() {
        assert_eq!(plain(format_bytes(0)), "0 bytes");
        assert_eq!(plain(format_bytes(1)), "1 byte");
        assert_eq!(plain(format_bytes(999)), "999 bytes");
        assert_eq!(format_bytes(1_000), "1 KB");
        assert_eq!(format_bytes(4_096), "4 KB");
        assert_eq!(format_bytes(12_400_000), "12.4 MB");
        assert_eq!(format_bytes(245_330_000_000), "245.33 GB");
        assert_eq!(format_bytes(2_500_000_000_000), "2.50 TB");
    }

    #[test]
    fn free_space_text_reads_available() {
        assert_eq!(
            plain(free_space_text(245_330_000_000)),
            "245.33 GB available"
        );
    }

    #[test]
    fn root_has_sane_free_space() {
        let bytes = available_space(Path::new("/")).expect("free space for /");
        assert!(bytes > 0);
        // No volume this runs on is over a petabyte.
        assert!(bytes < 1_000_000_000_000_000);
        let fallback = statvfs_available(Path::new("/")).expect("statvfs for /");
        // Finder's number counts purgeable space, so it is never meaningfully below df's.
        assert!(
            bytes as f64 >= fallback as f64 * 0.95,
            "{bytes} vs {fallback}"
        );
    }
}
