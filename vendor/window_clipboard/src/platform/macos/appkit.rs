//! AppKit glue for the macOS DnD backend.
//!
//! In-process drags: iced never hands pointer events to the clipboard, and
//! winit keeps delivering them to its view, so a local `NSEvent` monitor
//! watches `leftMouseDragged`, `leftMouseUp`, `flagsChanged` and Escape while a
//! drag is in flight. Each sample is mapped to the registered view under the
//! pointer and fed to the state machine. The monitor passes every event on
//! untouched except Escape, which it swallows after cancelling.
//!
//! Main thread only. `Hooks` keeps AppKit handles behind `MainThreadBound`.

use super::state::{Modifiers, SurfaceKey};
use super::{Shared, LOG};
use block2::RcBlock;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSView, NSWindow,
};
use objc2_foundation::NSPoint;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, Weak};

const ESCAPE_KEY_CODE: u16 = 53;

/// A value that is only ever touched on the main thread.
///
/// `Retained<AnyObject>` is neither `Send` nor `Sync`. The backend keeps it in
/// state that is shared through an `Arc`, so this wrapper asserts the main
/// thread rule instead; every accessor takes a `MainThreadMarker`.
pub(crate) struct MainThreadCell<T>(T);

// SAFETY: construction and access both require a `MainThreadMarker`, so the
// inner value never moves off the main thread.
unsafe impl<T> Send for MainThreadCell<T> {}
unsafe impl<T> Sync for MainThreadCell<T> {}

impl<T> MainThreadCell<T> {
    pub(crate) fn new(value: T, _mtm: MainThreadMarker) -> Self {
        Self(value)
    }

    pub(crate) fn into_inner(self, _mtm: MainThreadMarker) -> T {
        self.0
    }

    pub(crate) fn get(&self, _mtm: MainThreadMarker) -> &T {
        &self.0
    }
}

/// AppKit handles owned by the backend.
#[derive(Default)]
pub(crate) struct Hooks {
    monitor: Mutex<Option<MainThreadCell<Retained<AnyObject>>>>,
    /// The `NSDraggingSource` of the session in flight, kept alive for it.
    pub(crate) drag_source:
        Mutex<Option<MainThreadCell<Retained<super::source::DragSource>>>>,
}

impl Hooks {
    pub(crate) fn remove_monitor(&self, mtm: MainThreadMarker) {
        if let Some(monitor) = self.monitor.lock().unwrap().take() {
            let monitor = monitor.into_inner(mtm);
            // SAFETY: `monitor` came from addLocalMonitorForEventsMatchingMask.
            unsafe { NSEvent::removeMonitor(&monitor) };
            log::trace!(target: LOG, "pointer monitor removed");
        }
    }
}

pub(crate) fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers {
        option: flags.contains(NSEventModifierFlags::Option),
        command: flags.contains(NSEventModifierFlags::Command),
    }
}

/// Which registered view is under a screen point, and where in it.
///
/// Uses AppKit's own front-to-back window order, so a window from another app
/// on top yields `None`. Coordinates come out in the view's flipped space,
/// which is the top-left logical space iced lays out in (`WinitView` returns
/// `true` from `isFlipped`).
pub(crate) fn sample_at(
    mtm: MainThreadMarker,
    screen_point: NSPoint,
    keys: &[SurfaceKey],
) -> Option<(SurfaceKey, f64, f64)> {
    let front = NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(
        screen_point,
        0,
        mtm,
    );
    for &key in keys {
        // SAFETY: a key is a live `NSView` pointer. iced registers an empty
        // rectangle list for a window before it closes, which removes the key.
        let view: &NSView = unsafe { &*(key as *const NSView) };
        let Some(window) = view.window() else {
            continue;
        };
        if window.windowNumber() != front {
            continue;
        }
        let in_window = window.convertPointFromScreen(screen_point);
        let p = view.convertPoint_fromView(in_window, None);
        let b = view.bounds();
        let inside = p.x >= b.origin.x
            && p.x <= b.origin.x + b.size.width
            && p.y >= b.origin.y
            && p.y <= b.origin.y + b.size.height;
        if inside {
            return Some((key, p.x, p.y));
        }
    }
    None
}

fn track(shared: &Shared, mtm: MainThreadMarker, mods: Modifiers) {
    let keys: Vec<SurfaceKey> =
        shared.dnd.lock().unwrap().surface_keys().collect();
    let at = sample_at(mtm, NSEvent::mouseLocation(), &keys);
    let events = shared.dnd.lock().unwrap().pointer(at, mods);
    shared.emit(events);
}

/// Start the loopback for a drag this process began.
pub(crate) fn begin_internal_drag(shared: &Arc<Shared>, mtm: MainThreadMarker) {
    install_monitor(shared, mtm);
    track(shared, mtm, modifiers(NSEvent::modifierFlags_class()));
}

fn install_monitor(shared: &Arc<Shared>, mtm: MainThreadMarker) {
    shared.hooks.remove_monitor(mtm);
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let handler =
        RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            let pass_through = event.as_ptr();
            let Some(shared) = weak.upgrade() else {
                return pass_through;
            };
            let Some(mtm) = MainThreadMarker::new() else {
                return pass_through;
            };
            // SAFETY: AppKit hands the monitor a valid event for the call.
            let event_ref: &NSEvent = unsafe { event.as_ref() };
            let mods = modifiers(event_ref.modifierFlags());
            match event_ref.r#type() {
                NSEventType::LeftMouseDragged | NSEventType::FlagsChanged => {
                    track(&shared, mtm, mods);
                }
                NSEventType::LeftMouseUp => {
                    track(&shared, mtm, mods);
                    let (outcome, events) =
                        shared.dnd.lock().unwrap().release();
                    log::debug!(target: LOG, "loopback drop: {outcome:?}");
                    shared.emit(events);
                    shared.hooks.remove_monitor(mtm);
                }
                NSEventType::KeyDown
                    if event_ref.keyCode() == ESCAPE_KEY_CODE =>
                {
                    let events = shared.dnd.lock().unwrap().cancel();
                    log::debug!(target: LOG, "loopback cancelled with Escape");
                    shared.emit(events);
                    shared.hooks.remove_monitor(mtm);
                    return std::ptr::null_mut();
                }
                _ => {}
            }
            pass_through
        });
    let mask = NSEventMask::LeftMouseDragged
        | NSEventMask::LeftMouseUp
        | NSEventMask::FlagsChanged
        | NSEventMask::KeyDown;
    // SAFETY: the block returns the event it was given, or null for Escape.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler)
    };
    match monitor {
        Some(monitor) => {
            *shared.hooks.monitor.lock().unwrap() =
                Some(MainThreadCell::new(monitor, mtm));
            log::trace!(target: LOG, "pointer monitor installed");
        }
        None => {
            log::error!(target: LOG, "addLocalMonitorForEventsMatchingMask returned nil")
        }
    }
}

/// Make a view accept drags from other apps.
pub(crate) fn install_destination(
    shared: &Arc<Shared>,
    mtm: MainThreadMarker,
    key: SurfaceKey,
) {
    super::destination::install(shared, mtm, key);
}
