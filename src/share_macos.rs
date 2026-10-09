// SPDX-License-Identifier: GPL-3.0-only

//! The system share sheet and AirDrop, for the selected files and folders.
//!
//! [`show_picker`] opens `NSSharingServicePicker`, the menu Finder shows under Share: AirDrop,
//! Messages, Mail, Notes and whatever share extensions are installed. [`send_via_airdrop`] skips
//! the menu and goes straight to the AirDrop panel.
//!
//! Both need the main thread, and the picker needs an `NSView` to hang from, so both go through
//! [`crate::appkit_macos::with_ns_window`]. The picker is anchored at the mouse pointer, which
//! is still over the context menu entry that was just clicked. AppKit reports that point in
//! window coordinates with the origin at the bottom left, and the content view converts it to
//! its own coordinates, so no flip by hand is needed.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use cosmic::iced::Task;
use cosmic::iced::window::Id as WindowId;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSSharingService, NSSharingServiceNameSendViaAirDrop, NSSharingServicePicker};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSURL};

thread_local! {
    /// The picker on screen. AppKit does not keep it alive while its menu is open, so the last
    /// one shown is held here until the next replaces it. Only touched on the main thread.
    static PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
}

/// A file URL for `path`. Folders get a directory URL, with the trailing slash, so the share
/// services treat them as folders. A relative path has no URL: AppKit would resolve it against
/// the working directory, which is not where the user's file is.
pub fn file_url(path: &Path) -> Option<Retained<NSURL>> {
    if !path.is_absolute() {
        None
    } else if path.is_dir() {
        NSURL::from_directory_path(path)
    } else {
        NSURL::from_file_path(path)
    }
}

/// The share items for `paths`: one file URL each, in order. A path that cannot be made into a
/// URL (a relative path, or one with a NUL byte) is skipped and logged.
pub fn file_urls(paths: &[PathBuf]) -> Retained<NSArray<NSURL>> {
    let urls: Vec<Retained<NSURL>> = paths
        .iter()
        .filter_map(|path| {
            let url = file_url(path);
            if url.is_none() {
                log::warn!("cannot share {}: no file URL for it", path.display());
            }
            url
        })
        .collect();
    NSArray::from_retained_slice(&urls)
}

/// Show the share menu for `paths`, at the pointer in `window_id`.
pub fn show_picker<M: Send + 'static>(window_id: WindowId, paths: Vec<PathBuf>) -> Task<M> {
    if paths.is_empty() {
        return Task::none();
    }
    crate::appkit_macos::with_ns_window(window_id, move |window, _mtm| {
        let Some(view) = window.contentView() else {
            log::warn!("no content view to show the share menu from");
            return;
        };
        let items = file_urls(&paths);
        if items.is_empty() {
            return;
        }

        // A zero-size rect at the pointer, clamped into the view in case the pointer has left
        // the window since the click.
        let bounds = view.bounds();
        let pointer = view.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
        let anchor = NSRect::new(clamp_into(pointer, bounds), NSSize::new(0.0, 0.0));

        // SAFETY: `items` is an array of `NSURL`, which conforms to `NSPasteboardWriting` as
        // the initializer requires.
        let picker = unsafe {
            NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), as_items(&items))
        };
        picker.showRelativeToRect_ofView_preferredEdge(anchor, &view, NSRectEdge::MinY);
        PICKER.with(|slot| *slot.borrow_mut() = Some(picker));
    })
    .discard()
}

/// Hand `paths` straight to AirDrop, without the share menu.
pub fn send_via_airdrop<M: Send + 'static>(window_id: WindowId, paths: Vec<PathBuf>) -> Task<M> {
    if paths.is_empty() {
        return Task::none();
    }
    crate::appkit_macos::with_ns_window(window_id, move |_window, _mtm| {
        // SAFETY: the name is the AppKit constant, alive for the process.
        let name = unsafe { NSSharingServiceNameSendViaAirDrop };
        let Some(service) = NSSharingService::sharingServiceNamed(name) else {
            log::warn!("AirDrop is not available");
            return;
        };
        let items = file_urls(&paths);
        // SAFETY: `items` is an array of `NSURL`, which conforms to `NSPasteboardWriting` as
        // both methods require.
        unsafe {
            if service.canPerformWithItems(Some(as_items(&items))) {
                service.performWithItems(as_items(&items));
            } else {
                log::warn!("AirDrop cannot send these {} items", items.len());
            }
        }
    })
    .discard()
}

/// The URLs as the untyped array the sharing APIs take.
fn as_items(urls: &NSArray<NSURL>) -> &NSArray {
    // SAFETY: every `NSURL` is an `AnyObject`; the array is only read through the result.
    unsafe { urls.cast_unchecked::<AnyObject>() }
}

fn clamp_into(point: NSPoint, rect: NSRect) -> NSPoint {
    NSPoint::new(
        point
            .x
            .clamp(rect.origin.x, rect.origin.x + rect.size.width),
        point
            .y
            .clamp(rect.origin.y, rect.origin.y + rect.size.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_keep_order_and_mark_folders() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Report final.pdf");
        std::fs::write(&file, b"x").unwrap();
        let folder = dir.path().join("Photos");
        std::fs::create_dir(&folder).unwrap();

        let urls = file_urls(&[file.clone(), folder.clone()]);

        assert_eq!(urls.len(), 2);
        let first = urls.objectAtIndex(0);
        let second = urls.objectAtIndex(1);
        assert!(first.isFileURL());
        assert_eq!(first.to_file_path().as_deref(), Some(file.as_path()));
        assert!(!first.hasDirectoryPath());
        assert_eq!(second.to_file_path().as_deref(), Some(folder.as_path()));
        assert!(second.hasDirectoryPath());
        // Spaces are escaped in the URL, not passed through.
        assert!(
            first
                .absoluteString()
                .unwrap()
                .to_string()
                .ends_with("/Report%20final.pdf")
        );
    }

    #[test]
    fn file_urls_skip_relative_paths() {
        let urls = file_urls(&[PathBuf::from("relative/file.txt")]);
        assert!(urls.is_empty());
    }

    #[test]
    fn anchor_is_clamped_into_the_view() {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 50.0));
        let p = clamp_into(NSPoint::new(-5.0, 80.0), rect);
        assert_eq!((p.x, p.y), (0.0, 50.0));
        let p = clamp_into(NSPoint::new(30.0, 20.0), rect);
        assert_eq!((p.x, p.y), (30.0, 20.0));
    }
}
