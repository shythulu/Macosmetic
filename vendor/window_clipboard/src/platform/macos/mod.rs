//! macOS backend: the general pasteboard for text, plus drag and drop.
//!
//! Upstream left every `DndProvider` method empty here. This module is the
//! Macosmetic replacement. Layout:
//!
//! | File | Role |
//! |---|---|
//! | `state.rs` | pure state machine: destination rectangles, hit testing, the event sequence iced's widgets expect |
//! | `appkit.rs` | AppKit glue: pointer monitor for in-process drags, view geometry, `NSDraggingDestination` methods for drops from other apps |
//!
//! Every `DndProvider` call arrives on the main thread (iced's event loop is
//! the main thread on macOS). The shared state sits behind a `Mutex` because
//! the trait takes `&self` and the AppKit callbacks need the same state.

use crate::ClipboardProvider;
use crate::dnd::DndProvider;
use dnd::{DndAction, DndDestinationRectangle, DndEvent, DndSurface, Icon, Sender};
use mime::{AllowedMimeTypes, AsMimeTypes};
use objc2::MainThreadMarker;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};
use std::{
    borrow::Cow,
    error::Error,
    sync::{Arc, Mutex},
};

mod appkit;
mod destination;
pub mod state;

use state::{Dnd, Offer, SurfaceKey};

const LOG: &str = "window_clipboard::macos";

pub struct Clipboard {
    pasteboard: clipboard_macos::Clipboard,
    shared: Arc<Shared>,
}

/// State shared between the `Clipboard` and the AppKit callbacks.
pub(crate) struct Shared {
    pub(crate) dnd: Mutex<Dnd<DndSurface>>,
    sender: Mutex<Option<Box<dyn Sender<DndSurface> + Send + Sync + 'static>>>,
    pub(crate) hooks: appkit::Hooks,
}

impl Shared {
    /// Hand events to iced. Each one wakes the event loop.
    pub(crate) fn emit(&self, events: Vec<DndEvent<DndSurface>>) {
        if events.is_empty() {
            return;
        }
        let sender = self.sender.lock().unwrap();
        let Some(tx) = sender.as_ref() else {
            log::warn!(
                target: LOG,
                "dropping {} DnD events: iced never called init_dnd on this platform",
                events.len()
            );
            return;
        };
        for event in events {
            log::trace!(target: LOG, "emit {event:?}");
            if tx.send(event).is_err() {
                log::warn!(target: LOG, "DnD event channel closed");
                return;
            }
        }
    }
}

pub fn connect<W: HasDisplayHandle + ?Sized>(
    _window: &W,
) -> Result<Clipboard, Box<dyn Error>> {
    Ok(Clipboard {
        pasteboard: clipboard_macos::Clipboard::new()?,
        shared: Arc::new(Shared {
            dnd: Mutex::new(Dnd::default()),
            sender: Mutex::new(None),
            hooks: appkit::Hooks::default(),
        }),
    })
}

/// The `NSView` pointer behind a surface, which keys the destination registry.
pub(crate) fn surface_key(surface: &DndSurface) -> Option<SurfaceKey> {
    match surface.0.window_handle().ok()?.as_raw() {
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as usize),
        other => {
            log::warn!(target: LOG, "not an AppKit window handle: {other:?}");
            None
        }
    }
}

impl DndProvider for Clipboard {
    fn init_dnd(
        &self,
        tx: Box<dyn Sender<DndSurface> + Send + Sync + 'static>,
    ) {
        log::debug!(target: LOG, "init_dnd: sender installed");
        *self.shared.sender.lock().unwrap() = Some(tx);
    }

    fn start_dnd<D: AsMimeTypes + Send + 'static>(
        &self,
        internal: bool,
        source_surface: DndSurface,
        icon_surface: Option<Icon>,
        content: D,
        actions: DndAction,
    ) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::error!(target: LOG, "start_dnd off the main thread; ignored");
            return;
        };
        let mimes = content.available();
        log::debug!(
            target: LOG,
            "start_dnd internal={internal} actions={actions:?} mimes={mimes:?} icon={}",
            icon_surface.is_some()
        );
        let source_key = surface_key(&source_surface);
        let events = self.shared.dnd.lock().unwrap().begin(
            Offer::Internal(Box::new(content)),
            actions,
            true,
        );
        self.shared.emit(events);
        appkit::begin_internal_drag(&self.shared, mtm, source_key, icon_surface);
    }

    fn end_dnd(&self) {
        log::debug!(target: LOG, "end_dnd");
        self.shared.dnd.lock().unwrap().end();
        if let Some(mtm) = MainThreadMarker::new() {
            self.shared.hooks.remove_monitor(mtm);
        }
    }

    fn register_dnd_destination(
        &self,
        surface: DndSurface,
        rectangles: Vec<DndDestinationRectangle>,
    ) {
        let Some(key) = surface_key(&surface) else {
            return;
        };
        log::trace!(
            target: LOG,
            "register_dnd_destination view={key:#x} rects={}",
            rectangles.len()
        );
        let first_time = !rectangles.is_empty()
            && !self.shared.dnd.lock().unwrap().surface_keys().any(|k| k == key);
        self.shared
            .dnd
            .lock()
            .unwrap()
            .register(key, surface, rectangles);
        if first_time {
            if let Some(mtm) = MainThreadMarker::new() {
                appkit::install_destination(&self.shared, mtm, key);
            }
        }
    }

    fn set_action(&self, action: DndAction) {
        log::debug!(target: LOG, "set_action {action:?}");
        let (_outcome, events) = self.shared.dnd.lock().unwrap().set_action(action);
        self.shared.emit(events);
    }

    fn peek_offer<D: AllowedMimeTypes + 'static>(
        &self,
        mime_type: Option<Cow<'static, str>>,
    ) -> std::io::Result<D> {
        let wanted: Option<String> = match mime_type {
            Some(m) => Some(m.into_owned()),
            None => {
                // Pick the first type the caller accepts that the offer has.
                let offered = self.shared.dnd.lock().unwrap().offer_mime_types();
                D::allowed()
                    .iter()
                    .find(|m| offered.iter().any(|o| o == *m))
                    .cloned()
            }
        };
        let peeked = self.shared.dnd.lock().unwrap().peek(wanted.as_deref());
        let Some((data, mime)) = peeked else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no DnD offer in progress, or the mime type is not offered",
            ));
        };
        D::try_from((data, mime)).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "DnD offer data could not be converted",
            )
        })
    }
}

impl ClipboardProvider for Clipboard {
    fn read(&self) -> Result<String, Box<dyn Error>> {
        self.pasteboard.read()
    }

    fn write(&mut self, contents: String) -> Result<(), Box<dyn Error>> {
        self.pasteboard.write(contents)
    }
}
