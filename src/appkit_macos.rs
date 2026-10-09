// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

//! The parts of the application only AppKit can provide.
//!
//! Two fixes are applied once at launch:
//!
//! * [`disable_autofill_heuristics`] registers a user default before the event loop starts, so
//!   AppKit never attaches its autofill heuristics to the rename, search and path-bar text
//!   fields. Left on, they make typing slower the longer the app runs on macOS 26.
//! * [`pin_srgb_color_space`] pins the window to sRGB. MacBook panels are Display P3 and
//!   libcosmic's palettes are authored in sRGB, so greys and the accent colour come out
//!   oversaturated until the window is told which space its colours are in.
//!
//! The rest is the application lifecycle a Mac app is expected to have.
//! [`hide_application`] is what Cmd+H does everywhere else, and [`watch_activation`] notices
//! the application being brought to the front — a click on the Dock icon, most of all — so that
//! a window can be put back after the last one was closed. [`route_quit_menu_item`] sends the
//! Quit menu item, and with it Cmd+Q, to the application, so that a quit waits for the copies
//! and moves still running.
//!
//! [`watch_open_documents`] receives the folders and files the system asks this application to
//! open: a folder double-clicked in the Dock, `open <dir>` in a shell, Finder's Open With, and
//! the paths an `open -a` launch carries. They reach the application through
//! [`open_documents_subscription`], or through [`take_launch_documents`] for the ones that
//! arrived while it was still starting.
//!
//! Reaching the `NSWindow` means going out through `raw_window_handle` to the `NSView` iced
//! draws into, which is what [`with_ns_window`] wraps: it hands a live `NSWindow` to a closure
//! on the main thread, or yields nothing if there is no window to hand over. AppKit calls back
//! into us synchronously, so nothing in here may touch application state.

use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use block2::RcBlock;
use cosmic::iced::futures::{self, StreamExt, channel::mpsc};
use cosmic::iced::runtime::window::raw_window_handle::RawWindowHandle;
use cosmic::iced::runtime::window::run_with_handle;
use cosmic::iced::window::Id as WindowId;
use cosmic::iced::{Subscription, Task};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSApplicationDidBecomeActiveNotification,
    NSApplicationWillFinishLaunchingNotification, NSColorSpace, NSMenu, NSView, NSWindow,
};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSArray, NSDictionary, NSNotification,
    NSNotificationCenter, NSURL, NSUserDefaults, ns_string,
};

/// Turn off AppKit's autofill heuristics. Call once, before the event loop starts; a default
/// registered afterwards would not be read by the text fields that are already alive.
pub fn disable_autofill_heuristics() {
    let key = ns_string!("NSAutoFillHeuristicControllerEnabled");
    let value: &AnyObject = ns_string!("NO");
    let defaults = NSUserDefaults::standardUserDefaults();

    // SAFETY: the dictionary is the `NSString` → `NSString` registration dictionary the method
    // documents; both are static strings that outlive the call.
    unsafe { defaults.registerDefaults(&NSDictionary::from_slices(&[key], &[value])) };

    log::info!(
        "registered NSAutoFillHeuristicControllerEnabled = NO (now {})",
        defaults.boolForKey(key)
    );
}

/// Hide the application, the way Cmd+H hides any other Mac app. Clicking the Dock icon, or
/// Cmd+Tabbing back, brings it out again; AppKit restores the windows it hid.
pub fn hide_application() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("not hiding the application: not on the main thread");
        return;
    };
    // `nil` is the sender a programmatic hide passes, as opposed to the menu item that would
    // otherwise be validated against it.
    NSApplication::sharedApplication(mtm).hide(None);
    log::info!("hid the application");
}

/// The application was brought to the front: the Dock icon was clicked, it was Cmd+Tabbed to,
/// or it was unhidden.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Activated;

/// The receiving end of the activation observer's channel, parked here until the subscription
/// starts.
static ACTIVATIONS: Mutex<Option<mpsc::UnboundedReceiver<Activated>>> = Mutex::new(None);

/// Start watching for the application being activated. Call once, on the main thread, before
/// the first [`activation_subscription`] runs; the observer lives for the rest of the process.
///
/// This observes a notification rather than implementing `applicationShouldHandleReopen:` on
/// the delegate [`watch_open_documents`] installs, because the delegate may not be ours if
/// something else claimed the slot first. The cost is that an activation is only reported when
/// the application was not already frontmost.
pub fn watch_activation() {
    let Some(_mtm) = MainThreadMarker::new() else {
        log::warn!("activation observer not installed: not on the main thread");
        return;
    };

    let (tx, rx) = mpsc::unbounded();
    *ACTIVATIONS.lock().unwrap() = Some(rx);

    let handler = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        // AppKit posts this synchronously, while the application may already be borrowed, so
        // handing the news to the subscription is all this may do.
        let _ = tx.unbounded_send(Activated);
    });

    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: the name is AppKit's own notification constant, the block only sends on a
    // channel, and a `None` queue asks for delivery on the posting thread, which is the main
    // thread for this notification.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidBecomeActiveNotification),
            None,
            None,
            &handler,
        )
    };
    // The observer is wanted for the life of the process, and dropping the token would remove
    // it, so the token is leaked rather than tracked.
    std::mem::forget(token);
    log::info!("watching for application activation");
}

/// Deliver the observer's activations as messages. Yields nothing if [`watch_activation`] did
/// not run.
pub fn activation_subscription() -> Subscription<Activated> {
    Subscription::run(|| {
        // The receiver is taken once; a restarted subscription gets an empty stream.
        let rx = ACTIVATIONS.lock().unwrap().take();
        futures::stream::iter(rx).flatten()
    })
}

/// A Quit menu item was chosen, or its Cmd+Q pressed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuitRequested;

/// The Quit item's channel: the sending end for [`QuitTarget`], and the receiving end parked
/// until the subscription starts. It exists from first use rather than from
/// [`route_quit_menu_item`], because the subscription starts before there is a window, and so
/// before the menu can be rerouted; a receiver created later would never be picked up.
static QUIT_CHANNEL: LazyLock<(
    mpsc::UnboundedSender<QuitRequested>,
    Mutex<Option<mpsc::UnboundedReceiver<QuitRequested>>>,
)> = LazyLock::new(|| {
    let (tx, rx) = mpsc::unbounded();
    (tx, Mutex::new(Some(rx)))
});

/// Set once the Quit item has been rerouted, so the repeat calls cost nothing.
static QUIT_ROUTED: AtomicBool = AtomicBool::new(false);

define_class!(
    /// The target the Quit menu item sends its action to, in place of `NSApplication`.
    // SAFETY: `NSObject` has no subclassing requirements, and `QuitTarget` does not implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MacosmeticQuitTarget"]
    struct QuitTarget;

    impl QuitTarget {
        #[unsafe(method(requestQuit:))]
        fn request_quit(&self, _sender: Option<&AnyObject>) {
            // AppKit calls this while the application may already be borrowed, so handing the
            // request to the subscription is all this may do.
            let _ = QUIT_CHANNEL.0.unbounded_send(QuitRequested);
        }
    }
);

impl QuitTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// Point the Quit item in the application menu that winit installs at
/// [`crate::app::Message::Quit`], through [`quit_subscription`].
///
/// Left alone, the item sends `terminate:`, which ends the process where it stands; a copy or
/// move still running would be lost with it. The item keeps its Cmd+Q, so the shortcut works
/// the way it does in any Mac app: with no window open, and while a text field has focus,
/// neither of which a binding on the window can see.
///
/// Call once a window exists: winit builds the menu while the event loop is starting, which is
/// after anything `main` can do.
pub fn route_quit_menu_item() {
    if QUIT_ROUTED.load(Ordering::Relaxed) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("not rerouting Quit: not on the main thread");
        return;
    };
    QUIT_ROUTED.store(true, Ordering::Relaxed);
    let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else {
        log::warn!("no application menu, so no Quit item to reroute");
        return;
    };

    let target = QuitTarget::new(mtm);
    let routed = route_terminate(&menu, &target);
    // A menu item holds its target weakly, and the target is wanted for the life of the
    // process, so it is leaked rather than tracked.
    std::mem::forget(target);
    log::info!("routed {routed} Quit menu item(s) to the application");
}

/// Send every `terminate:` item in `menu` and its submenus to `target` instead, and report how
/// many there were.
fn route_terminate(menu: &NSMenu, target: &QuitTarget) -> usize {
    let mut routed = 0;
    for item in &menu.itemArray() {
        if let Some(submenu) = item.submenu() {
            routed += route_terminate(&submenu, target);
        }
        if item.action() == Some(sel!(terminate:)) {
            let target: &AnyObject = target;
            // SAFETY: `target` implements `requestQuit:` with the `(id sender)` signature a menu
            // item action takes, and is kept alive for the rest of the process.
            unsafe {
                item.setTarget(Some(target));
                item.setAction(Some(sel!(requestQuit:)));
            }
            routed += 1;
        }
    }
    routed
}

/// Deliver the Quit item's requests as messages. Yields nothing until [`route_quit_menu_item`]
/// has rerouted the item.
pub fn quit_subscription() -> Subscription<QuitRequested> {
    Subscription::run(|| {
        // The receiver is taken once; a restarted subscription gets an empty stream.
        let rx = QUIT_CHANNEL.1.lock().unwrap().take();
        futures::stream::iter(rx).flatten()
    })
}

/// Paths the system asked this application to open, in the order they were sent. Folders are
/// to be browsed; files are to be shown in their folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenDocuments(pub Vec<PathBuf>);

/// The open-documents channel: the sending end for the delegate and the event handler, and
/// the receiving end parked until it is claimed. It exists from first use, because the paths an
/// `open -a` launch carries arrive before the application has started, and are held here until
/// [`take_launch_documents`] or [`open_documents_subscription`] asks for them.
static OPEN_DOCUMENTS: LazyLock<(
    mpsc::UnboundedSender<OpenDocuments>,
    Mutex<Option<mpsc::UnboundedReceiver<OpenDocuments>>>,
)> = LazyLock::new(|| {
    let (tx, rx) = mpsc::unbounded();
    (tx, Mutex::new(Some(rx)))
});

/// The four-character codes of the open-documents Apple Event: event class `aevt`, event ID
/// `odoc`, and the `----` keyword of its direct object, the list of files.
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS_EVENT: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

/// Hand the file URLs AppKit delivered to the application. Anything that is not a file URL is
/// dropped with a log line; AppKit does not send those for folders.
fn deliver_urls(urls: impl IntoIterator<Item = Retained<NSURL>>) {
    let paths: Vec<PathBuf> = urls
        .into_iter()
        .filter_map(|url| {
            // SAFETY: `path` reads an immutable property of a URL this call owns.
            let path = url.isFileURL().then(|| url.path()).flatten();
            if path.is_none() {
                log::warn!("ignoring a non-file URL the system asked us to open");
            }
            path.map(|path| PathBuf::from(path.to_string()))
        })
        .collect();
    log::info!("asked to open {paths:?}");
    if !paths.is_empty() {
        // AppKit calls this while the application may already be borrowed, so handing the
        // paths to the channel is all this may do.
        let _ = OPEN_DOCUMENTS.0.unbounded_send(OpenDocuments(paths));
    }
}

define_class!(
    /// The application delegate. winit 0.31 does not install one, and leaves this slot to the
    /// application, so this is where `application:openURLs:` arrives.
    // SAFETY: `NSObject` has no subclassing requirements, and `AppDelegate` does not implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MacosmeticAppDelegate"]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(application:openURLs:))]
        fn application_open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            deliver_urls(urls.iter());
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    /// The handler for the `odoc` Apple Event, used only when some other object already holds
    /// the application delegate. It reads the same file list the delegate would be handed.
    // SAFETY: as for `AppDelegate`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MacosmeticOpenDocumentsHandler"]
    struct OpenDocumentsHandler;

    impl OpenDocumentsHandler {
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: &NSAppleEventDescriptor,
        ) {
            deliver_urls(direct_object_urls(event));
        }
    }
);

impl OpenDocumentsHandler {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// The file URLs in an `odoc` event's direct object, which is a list of file descriptors.
fn direct_object_urls(event: &NSAppleEventDescriptor) -> Vec<Retained<NSURL>> {
    // SAFETY: `paramDescriptorForKeyword:` takes an `AEKeyword`, a four-character code that is
    // a `u32`, and returns an owned descriptor or nil. The typed binding is behind a Core
    // Services crate this build does not carry.
    let list: Option<Retained<NSAppleEventDescriptor>> =
        unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
    let Some(list) = list else {
        log::warn!("open-documents event without a direct object");
        return Vec::new();
    };
    // Apple Event lists count from one.
    (1..=list.numberOfItems())
        .filter_map(|index| list.descriptorAtIndex(index)?.fileURLValue())
        .collect()
}

/// Start receiving the folders and files the system asks this application to open. Call once,
/// on the main thread, before the event loop starts.
///
/// The work waits for `NSApplicationWillFinishLaunchingNotification`, which is the last moment
/// before AppKit dispatches the open-documents event an `open -a` launch carries, and the
/// earliest the application object may be touched: winit asks that `NSApplication` not be
/// created before its event loop is. At that point this becomes the application delegate, so
/// `application:openURLs:` is called for every folder, now and later. If something else already
/// holds the delegate, the `odoc` Apple Event is handled directly instead.
pub fn watch_open_documents() {
    let Some(_mtm) = MainThreadMarker::new() else {
        log::warn!("open-documents observer not installed: not on the main thread");
        return;
    };
    // Create the channel now, so a launch document has somewhere to go.
    LazyLock::force(&OPEN_DOCUMENTS);

    let handler = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("not installing the open-documents handler: not on the main thread");
            return;
        };
        install_open_documents_handler(mtm);
    });

    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: the name is AppKit's own notification constant, the block only installs objects
    // on the main thread, and a `None` queue asks for delivery on the posting thread, which is
    // the main thread for this notification.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
            None,
            &handler,
        )
    };
    // One launch per process, but dropping the token would remove the observer before it has
    // fired, so it is leaked rather than tracked.
    std::mem::forget(token);
    log::info!("watching for documents to open");
}

fn install_open_documents_handler(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    if app.delegate().is_none() {
        let delegate = AppDelegate::new(mtm);
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The application holds its delegate weakly, and the delegate is wanted for the life of
        // the process, so it is leaked rather than tracked.
        std::mem::forget(delegate);
        log::info!("installed the application delegate for open-documents requests");
        return;
    }

    log::warn!("another object is the application delegate; handling odoc events directly");
    let handler = OpenDocumentsHandler::new(mtm);
    let target: &AnyObject = &handler;
    // SAFETY: `handler` implements `handleOpenDocuments:withReplyEvent:` with the two-descriptor
    // signature an Apple Event handler takes, is kept alive for the rest of the process, and the
    // class and ID are the four-character codes the method documents.
    unsafe {
        let _: () = msg_send![
            &*NSAppleEventManager::sharedAppleEventManager(),
            setEventHandler: target,
            andSelector: sel!(handleOpenDocuments:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: OPEN_DOCUMENTS_EVENT,
        ];
    }
    std::mem::forget(handler);
}

/// The documents that arrived before the application existed: the folder an `open -a` launch
/// or a Dock folder click started this process with. Call once, from the application's `init`,
/// before [`open_documents_subscription`] runs; it empties the channel of what is there and
/// leaves it for the subscription.
pub fn take_launch_documents() -> Vec<PathBuf> {
    let mut guard = OPEN_DOCUMENTS.1.lock().unwrap();
    let Some(rx) = guard.as_mut() else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    while let Ok(OpenDocuments(more)) = rx.try_recv() {
        paths.extend(more);
    }
    paths
}

/// Deliver the system's open-documents requests as messages. Yields nothing if
/// [`watch_open_documents`] did not run.
pub fn open_documents_subscription() -> Subscription<OpenDocuments> {
    Subscription::run(|| {
        // The receiver is taken once; a restarted subscription gets an empty stream.
        let rx = OPEN_DOCUMENTS.1.lock().unwrap().take();
        futures::stream::iter(rx).flatten()
    })
}

/// Set once the window's colour space has been pinned, so the repeat calls that come with
/// every resize cost nothing.
static PINNED_SRGB: AtomicBool = AtomicBool::new(false);

/// Pin `window_id`'s colour space to sRGB. Safe to call whenever the window reports a size:
/// the work happens on the first call that finds a window, and every later call is a no-op.
///
/// A task queued before the window exists is dropped by iced, which is why this is driven from
/// a window event rather than from `init`.
pub fn pin_srgb_color_space<M: Send + 'static>(window_id: WindowId) -> Task<M> {
    if PINNED_SRGB.load(Ordering::Relaxed) {
        return Task::none();
    }

    with_ns_window(window_id, |window, _mtm| {
        let was = color_space_name(window);
        window.setColorSpace(Some(&NSColorSpace::sRGBColorSpace()));
        PINNED_SRGB.store(true, Ordering::Relaxed);
        log::info!(
            "window colour space {was} is now {}",
            color_space_name(window)
        );
    })
    .discard()
}

fn color_space_name(window: &NSWindow) -> String {
    window
        .colorSpace()
        .and_then(|color_space| color_space.localizedName())
        .map_or_else(|| "unnamed".to_string(), |name| name.to_string())
}

/// Run `f` against the `NSWindow` behind an iced window, on the main thread.
///
/// Yields nothing when the window has already closed, when AppKit has not put the view in a
/// window yet, or when the callback somehow runs off the main thread. Do not touch application
/// state from `f`: iced holds the app borrowed while the task runs.
pub fn with_ns_window<T: Send + 'static>(
    window_id: WindowId,
    f: impl FnOnce(&NSWindow, MainThreadMarker) -> T + Send + 'static,
) -> Task<Option<T>> {
    run_with_handle(window_id, move |handle| {
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            log::warn!("no AppKit window handle for window {window_id:?}");
            return None;
        };
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("skipping AppKit window call for {window_id:?}: not on the main thread");
            return None;
        };

        // SAFETY: the handle borrows the view for the duration of this call, and the main
        // thread marker proves the thread AppKit requires for an `NSView`.
        let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let Some(window) = view.window() else {
            log::warn!("AppKit view for window {window_id:?} has no window");
            return None;
        };
        Some(f(&window, mtm))
    })
}
