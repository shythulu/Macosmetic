// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

//! The macOS mounter: mounted volumes in the sidebar, with eject.
//!
//! The list comes from `getmntinfo(3)`. Finder's rule for what counts as a volume is the
//! `MNT_DONTBROWSE` flag, so [`is_browsable`] keeps every mount without it and then drops the
//! few system mounts that lack the flag, such as the boot volume itself. What is left is what
//! Finder shows under Locations: external and removable disks, disk images, and network shares,
//! all of which land under `/Volumes`.
//!
//! Each volume is named the way Finder names it, through `NSURLVolumeLocalizedNameKey`, and
//! ejected the way Finder ejects it, through `NSWorkspace unmountAndEjectDeviceAtURL:error:`.
//! Both calls happen on a worker thread, so a slow network share never blocks the UI.
//!
//! Changes arrive two ways. `NSWorkspace` posts mount, unmount and rename notifications on the
//! main thread; the observer forwards each as a rescan request. In case a mount slips past the
//! notifications, the worker also rescans every [`POLL_INTERVAL`] and only reports when the
//! list differs from the one it last sent.
//!
//! Network shares browse as plain paths. [`Item::is_remote`] is `false` for them on purpose:
//! the application routes remote items through [`Mounter::network_scan`], which only gvfs
//! implements, and a mounted share is a directory the ordinary scanner can read.

use std::any::TypeId;
use std::ffi::CStr;
use std::future::pending;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use block2::RcBlock;
use cosmic::iced::futures::channel::mpsc as futures_mpsc;
use cosmic::iced::futures::{SinkExt, StreamExt};
use cosmic::iced::{Subscription, stream};
use cosmic::{Task, widget};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidMountNotification, NSWorkspaceDidRenameVolumeNotification,
    NSWorkspaceDidUnmountNotification,
};
use objc2_foundation::{
    NSNotification, NSNumber, NSString, NSURL, NSURLResourceKey, NSURLVolumeIsEjectableKey,
    NSURLVolumeIsRemovableKey, NSURLVolumeLocalizedNameKey,
};

use super::{Mounter, MounterItem, MounterItems, MounterMessage};
use crate::config::IconSizes;
use crate::tab;

/// How often the worker rescans without being asked. The acceptance bar is "appears within
/// 2 s", and the notifications normally beat this by a wide margin.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// `MNT_REMOVABLE` from `<sys/mount.h>`. The `libc` crate does not export it.
const MNT_REMOVABLE: u32 = 0x0000_0200;

/// Mount points, and everything under them, that have no `MNT_DONTBROWSE` flag but are not
/// volumes a person browses. The boot volume `/` is handled separately: Finder shows it by its
/// own rule, not from the mount table, and listing it here would duplicate the Home and
/// Filesystem favourites.
const HIDDEN_MOUNT_PREFIXES: &[&str] = &["/dev", "/private/var/vm", "/System/Volumes"];

/// Filesystems that are plumbing, never a volume, whatever their flags say.
const HIDDEN_FS_TYPES: &[&str] = &["autofs", "devfs", "nullfs"];

/// Filesystems that are network shares. `MNT_LOCAL` is the primary signal; this list catches
/// the shares a filesystem driver reports as local anyway.
const REMOTE_FS_TYPES: &[&str] = &["afpfs", "cifs", "nfs", "smbfs", "webdav"];

/// One row of the mount table, as [`is_browsable`] sees it. Built from `statfs` by
/// [`mount_table`], or by hand in tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountEntry {
    pub mount_point: PathBuf,
    pub device: String,
    pub fs_type: String,
    pub flags: u32,
}

impl MountEntry {
    fn from_statfs(stat: &libc::statfs) -> Self {
        // SAFETY: the kernel NUL-terminates all three names inside their fixed arrays.
        let (mount_point, device, fs_type) = unsafe {
            (
                CStr::from_ptr(stat.f_mntonname.as_ptr()),
                CStr::from_ptr(stat.f_mntfromname.as_ptr()),
                CStr::from_ptr(stat.f_fstypename.as_ptr()),
            )
        };
        Self {
            mount_point: PathBuf::from(mount_point.to_string_lossy().into_owned()),
            device: device.to_string_lossy().into_owned(),
            fs_type: fs_type.to_string_lossy().into_owned(),
            flags: stat.f_flags,
        }
    }

    /// The share is on another machine.
    pub fn is_remote(&self) -> bool {
        self.flags & (libc::MNT_LOCAL as u32) == 0 || REMOTE_FS_TYPES.contains(&&*self.fs_type)
    }
}

/// Finder's rule for the Locations list: no `MNT_DONTBROWSE`, and not one of the system mounts
/// that lack the flag.
pub fn is_browsable(entry: &MountEntry) -> bool {
    if entry.flags & (libc::MNT_DONTBROWSE as u32) != 0 {
        return false;
    }
    if HIDDEN_FS_TYPES.contains(&&*entry.fs_type) {
        return false;
    }
    if entry.mount_point == Path::new("/") {
        return false;
    }
    !HIDDEN_MOUNT_PREFIXES
        .iter()
        .any(|hidden| entry.mount_point.starts_with(hidden))
}

/// Every mount the kernel knows about, in mount order.
pub fn mount_table() -> Vec<MountEntry> {
    let mut table: *mut libc::statfs = std::ptr::null_mut();
    // SAFETY: `getmntinfo` fills a buffer it owns and returns its length; the buffer stays
    // valid until the next `getmntinfo` call on this thread, and it is read before then.
    let count = unsafe { libc::getmntinfo(&mut table, libc::MNT_NOWAIT) };
    if count <= 0 || table.is_null() {
        log::warn!("getmntinfo failed: {}", std::io::Error::last_os_error());
        return Vec::new();
    }
    // SAFETY: the kernel wrote `count` entries starting at `table`.
    let entries = unsafe { std::slice::from_raw_parts(table, count as usize) };
    entries.iter().map(MountEntry::from_statfs).collect()
}

/// The mounts Finder would list, in mount order.
pub fn browsable_mounts() -> Vec<MountEntry> {
    mount_table().into_iter().filter(is_browsable).collect()
}

/// What a volume is, as far as choosing an icon goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// A share on another machine.
    Network,
    /// Media that comes out of a drive, and disk images.
    Removable,
    /// A disk that can be ejected but whose media stays put: an external SSD.
    External,
    /// A second internal disk, or a volume on the boot disk.
    Internal,
}

impl Kind {
    fn icon_name(self) -> &'static str {
        match self {
            Self::Network => "network-server",
            Self::Removable => "drive-removable-media",
            Self::External => "drive-harddisk-usb",
            Self::Internal => "drive-harddisk-system",
        }
    }
}

/// A mounted volume.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    name: String,
    path: PathBuf,
    kind: Kind,
}

impl Item {
    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub const fn is_mounted(&self) -> bool {
        // Only mounted volumes are listed; there is no "known but unmounted" state here.
        true
    }

    /// Always `false`: see the module docs for why shares browse as paths.
    pub const fn is_remote(&self) -> bool {
        false
    }

    pub const fn kind(&self) -> Kind {
        self.kind
    }

    pub fn uri(&self) -> String {
        url::Url::from_directory_path(&self.path)
            .map(String::from)
            .unwrap_or_default()
    }

    pub fn icon(&self, symbolic: bool) -> Option<widget::icon::Handle> {
        let name = self.kind.icon_name();
        let name = if symbolic {
            format!("{name}-symbolic")
        } else {
            name.to_string()
        };
        Some(widget::icon::from_name(name).size(16).handle())
    }

    pub fn path(&self) -> Option<PathBuf> {
        Some(self.path.clone())
    }
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn resource_value(url: &NSURL, key: &NSURLResourceKey) -> Option<Retained<AnyObject>> {
    let mut value: Option<Retained<AnyObject>> = None;
    // SAFETY: `value` is a valid out-pointer for the call's duration and the key is one of
    // Foundation's own resource key constants.
    match unsafe { url.getResourceValue_forKey_error(&mut value, key) } {
        Ok(()) => value,
        Err(err) => {
            log::debug!("no {key} for {url:?}: {}", err.localizedDescription());
            None
        }
    }
}

fn resource_bool(url: &NSURL, key: &NSURLResourceKey) -> bool {
    resource_value(url, key)
        .and_then(|value| value.downcast::<NSNumber>().ok())
        .is_some_and(|number| number.boolValue())
}

/// Finder's name for the volume, or the mount point's last component.
fn volume_name(url: &NSURL, mount_point: &Path) -> String {
    resource_value(url, unsafe { NSURLVolumeLocalizedNameKey })
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            mount_point
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| mount_point.to_string_lossy().into_owned())
}

fn item(entry: &MountEntry) -> Item {
    let url = file_url(&entry.mount_point);
    let kind = if entry.is_remote() {
        Kind::Network
    } else if entry.flags & MNT_REMOVABLE != 0
        || resource_bool(&url, unsafe { NSURLVolumeIsRemovableKey })
    {
        Kind::Removable
    } else if resource_bool(&url, unsafe { NSURLVolumeIsEjectableKey }) {
        Kind::External
    } else {
        Kind::Internal
    };
    Item {
        name: volume_name(&url, &entry.mount_point),
        path: entry.mount_point.clone(),
        kind,
    }
}

/// The current volume list.
pub fn volumes() -> Vec<Item> {
    browsable_mounts().iter().map(item).collect()
}

fn mounter_items(volumes: &[Item]) -> MounterItems {
    volumes.iter().cloned().map(MounterItem::Macos).collect()
}

/// The current volume list, as the sidebar wants it.
#[cfg(test)]
pub fn items() -> MounterItems {
    mounter_items(&volumes())
}

/// Unmount the volume at `path`, ejecting its disk if the disk can be ejected. Blocks; call
/// from the worker thread.
fn eject(path: &Path) -> Result<(), String> {
    let workspace = NSWorkspace::sharedWorkspace();
    workspace
        .unmountAndEjectDeviceAtURL_error(&file_url(path))
        .map_err(|err| err.localizedDescription().to_string())
}

enum Cmd {
    /// A subscription started and wants the current list, then every change.
    Subscribe(futures_mpsc::UnboundedSender<MounterMessage>),
    /// Something changed, or might have.
    Rescan,
    Eject(Item),
}

/// A `Send` handle on the command channel, for the notification block.
#[derive(Clone)]
struct CommandSender(mpsc::Sender<Cmd>);

impl CommandSender {
    fn send(&self, cmd: Cmd) {
        if self.0.send(cmd).is_err() {
            log::warn!("macOS mounter worker is gone");
        }
    }
}

pub struct Macos {
    command_tx: CommandSender,
    /// The last list the worker sent, for [`Mounter::items`].
    current: Arc<Mutex<MounterItems>>,
}

impl Macos {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let command_tx = CommandSender(command_tx);
        let current = Arc::new(Mutex::new(MounterItems::new()));

        let worker_current = current.clone();
        std::thread::Builder::new()
            .name("macos-mounter".into())
            .spawn(move || worker(command_rx, worker_current))
            .expect("failed to spawn macos mounter thread");

        watch_workspace(command_tx.clone());

        Self {
            command_tx,
            current,
        }
    }
}

/// Ask `NSWorkspace` to say when a volume mounts, unmounts or is renamed. Each notification
/// becomes a rescan. The observer lives for the rest of the process.
fn watch_workspace(command_tx: CommandSender) {
    let handler = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        // Posted on the main thread, possibly while the application is borrowed, so this only
        // hands the news to the worker.
        command_tx.send(Cmd::Rescan);
    });
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    for name in [
        unsafe { NSWorkspaceDidMountNotification },
        unsafe { NSWorkspaceDidUnmountNotification },
        unsafe { NSWorkspaceDidRenameVolumeNotification },
    ] {
        // SAFETY: the names are AppKit's own constants, the block only sends on a channel, and
        // a `None` queue delivers on the posting thread.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &handler)
        };
        // Dropping the token would remove the observer, which is wanted for the whole process.
        std::mem::forget(token);
    }
    log::info!("watching NSWorkspace for volume changes");
}

fn worker(command_rx: mpsc::Receiver<Cmd>, current: Arc<Mutex<MounterItems>>) {
    let mut subscribers: Vec<futures_mpsc::UnboundedSender<MounterMessage>> = Vec::new();
    let mut last: Option<Vec<Item>> = None;

    let broadcast = |subscribers: &mut Vec<futures_mpsc::UnboundedSender<MounterMessage>>,
                     message: MounterMessage| {
        subscribers.retain(|tx| tx.unbounded_send(message.clone()).is_ok());
    };

    loop {
        let cmd = match command_rx.recv_timeout(POLL_INTERVAL) {
            Ok(cmd) => Some(cmd),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };

        match cmd {
            Some(Cmd::Subscribe(tx)) => {
                if last.is_none() {
                    let volumes = volumes();
                    *current.lock().unwrap() = mounter_items(&volumes);
                    last = Some(volumes);
                }
                let items = current.lock().unwrap().clone();
                log::info!(
                    "new volume subscriber, sending {:?}",
                    items.iter().map(MounterItem::name).collect::<Vec<_>>()
                );
                if tx.unbounded_send(MounterMessage::Items(items)).is_ok() {
                    subscribers.push(tx);
                }
                continue;
            }
            Some(Cmd::Eject(item)) => {
                log::info!("ejecting {:?} at {:?}", item.name, item.path);
                if let Err(error) = eject(&item.path) {
                    log::warn!("failed to eject {:?}: {error}", item.name);
                    broadcast(
                        &mut subscribers,
                        MounterMessage::MountResult(MounterItem::Macos(item), Err(error)),
                    );
                }
                // Fall through to the rescan: the notification may already have been handled.
            }
            Some(Cmd::Rescan) | None => {}
        }

        let volumes = volumes();
        if last.as_ref() == Some(&volumes) {
            continue;
        }
        log::info!(
            "volumes: {:?}",
            volumes.iter().map(Item::name).collect::<Vec<_>>()
        );
        let items = mounter_items(&volumes);
        *current.lock().unwrap() = items.clone();
        last = Some(volumes);
        broadcast(&mut subscribers, MounterMessage::Items(items));
    }
}

impl Mounter for Macos {
    fn items(&self, _sizes: IconSizes) -> Option<MounterItems> {
        Some(self.current.lock().unwrap().clone())
    }

    fn mount(&self, _item: MounterItem) -> Task<()> {
        // Every listed volume is already mounted; a click opens it through its `Location`.
        Task::none()
    }

    fn network_drive(&self, _uri: String) -> Task<bool> {
        // Connect to Server is NetFS work, tracked separately.
        Task::none()
    }

    fn network_scan(
        &self,
        _uri: &str,
        _sizes: IconSizes,
    ) -> Option<Result<Vec<tab::Item>, String>> {
        None
    }

    fn dir_info(&self, _uri: &str) -> Option<(String, String, Option<PathBuf>)> {
        None
    }

    fn unmount(&self, item: MounterItem) -> Task<()> {
        let MounterItem::Macos(item) = item else {
            return Task::none();
        };
        let command_tx = self.command_tx.clone();
        Task::future(async move {
            command_tx.send(Cmd::Eject(item));
        })
    }

    fn subscription(&self) -> Subscription<MounterMessage> {
        struct Wrapper(CommandSender);
        impl Hash for Wrapper {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                TypeId::of::<Self>().hash(state);
            }
        }
        Subscription::run_with(Wrapper(self.command_tx.clone()), |Wrapper(command_tx)| {
            let command_tx = command_tx.clone();
            stream::channel(
                1,
                move |mut output: futures_mpsc::Sender<MounterMessage>| async move {
                    let (tx, mut rx) = futures_mpsc::unbounded();
                    command_tx.send(Cmd::Subscribe(tx));
                    while let Some(message) = rx.next().await {
                        if output.send(message).await.is_err() {
                            break;
                        }
                    }
                    pending().await
                },
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(mount_point: &str, fs_type: &str, flags: u32) -> MountEntry {
        MountEntry {
            mount_point: PathBuf::from(mount_point),
            device: String::from("/dev/disk9s1"),
            fs_type: fs_type.to_string(),
            flags,
        }
    }

    const LOCAL: u32 = libc::MNT_LOCAL as u32;
    const DONTBROWSE: u32 = libc::MNT_DONTBROWSE as u32;

    #[test]
    fn boot_volume_and_system_volumes_are_hidden() {
        assert!(!is_browsable(&entry(
            "/",
            "apfs",
            LOCAL | libc::MNT_RDONLY as u32
        )));
        assert!(!is_browsable(&entry("/System/Volumes/Data", "apfs", LOCAL)));
        assert!(!is_browsable(&entry(
            "/System/Volumes/Data",
            "apfs",
            LOCAL | DONTBROWSE
        )));
        assert!(!is_browsable(&entry(
            "/System/Volumes/Preboot",
            "apfs",
            LOCAL | DONTBROWSE
        )));
        assert!(!is_browsable(&entry("/private/var/vm", "apfs", LOCAL)));
        assert!(!is_browsable(&entry("/dev", "devfs", LOCAL)));
        assert!(!is_browsable(&entry(
            "/System/Volumes/Data/home",
            "autofs",
            0
        )));
    }

    #[test]
    fn dontbrowse_hides_a_volumes_mount() {
        assert!(!is_browsable(&entry(
            "/Volumes/Hidden",
            "apfs",
            LOCAL | DONTBROWSE
        )));
    }

    #[test]
    fn external_disks_images_and_shares_are_listed() {
        assert!(is_browsable(&entry(
            "/Volumes/USB",
            "msdos",
            LOCAL | MNT_REMOVABLE
        )));
        assert!(is_browsable(&entry(
            "/Volumes/MacosmeticTest",
            "apfs",
            LOCAL
        )));
        assert!(is_browsable(&entry("/Volumes/share", "smbfs", 0)));
        assert!(is_browsable(&entry("/Volumes/nas", "nfs", 0)));
        assert!(is_browsable(&entry("/Volumes/dav", "webdav", 0)));
    }

    #[test]
    fn a_second_volume_on_the_boot_disk_is_listed() {
        // A user-created APFS volume mounts under /Volumes without MNT_DONTBROWSE and Finder
        // lists it; nothing about its path says "system".
        assert!(is_browsable(&entry("/Volumes/Projects", "apfs", LOCAL)));
    }

    #[test]
    fn remote_is_by_flag_or_filesystem() {
        assert!(entry("/Volumes/share", "smbfs", 0).is_remote());
        assert!(entry("/Volumes/share", "smbfs", LOCAL).is_remote());
        assert!(entry("/Volumes/odd", "fuse", 0).is_remote());
        assert!(!entry("/Volumes/USB", "msdos", LOCAL).is_remote());
    }

    #[test]
    fn live_mount_table_hides_the_system_volumes() {
        let mounts = browsable_mounts();
        for entry in &mounts {
            assert!(
                !entry.mount_point.starts_with("/System/Volumes"),
                "listed {entry:?}"
            );
            assert_ne!(entry.mount_point, Path::new("/"), "listed the boot volume");
        }
        // The real table always holds the system mounts, so the filter must have removed some.
        assert!(mount_table().len() > mounts.len());
    }

    #[test]
    fn live_items_are_named_and_pathed() {
        for item in items() {
            let MounterItem::Macos(item) = item else {
                panic!("not a macOS item: {item:?}");
            };
            assert!(!item.name().is_empty());
            assert!(item.path().is_some_and(|p| p.is_absolute()));
            assert!(item.uri().starts_with("file:///"));
        }
    }
}
