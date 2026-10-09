//! Drag-out: an `NSDraggingSession` carrying the dragged files as URLs.
//!
//! Used for every drag this process starts whose content has a
//! `text/uri-list`. AppKit then runs the drag: other apps see file URLs, and
//! this app's own windows see the same drag through the destination path in
//! `destination.rs`. Drags without file URLs (tab reordering) keep the
//! in-process loopback in `appkit.rs`.
//!
//! The session needs the mouse event that started the drag. iced handles
//! `StartDnd` inside the winit turn for the `mouseDragged` that crossed the
//! drag threshold, so `NSApp.currentEvent` is still that event. When it is
//! not a mouse event the caller falls back to the loopback.

use super::appkit::MainThreadCell;
use super::state::Offer;
use super::{Shared, LOG};
use dnd::{DndAction, Icon};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadMarker,
    MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSBitmapImageRep, NSDeviceRGBColorSpace, NSDragOperation,
    NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEventType, NSImage, NSPasteboardItem, NSPasteboardTypeFileURL,
    NSPasteboardTypeString, NSView,
};
use objc2_foundation::{
    NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};
use std::cell::Cell;
use std::sync::{Arc, Mutex, Weak};

static SHARED: Mutex<Option<Weak<Shared>>> = Mutex::new(None);

pub(crate) struct DragSourceIvars {
    mask: Cell<NSDragOperation>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; no `Drop` impl.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MacosmeticDragSource"]
    #[ivars = DragSourceIvars]
    pub(crate) struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            self.ivars().mask.get()
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(
            &self,
            _session: &NSDraggingSession,
            _point: NSPoint,
            operation: NSDragOperation,
        ) {
            let performed = operation != NSDragOperation::None;
            log::debug!(target: LOG, "drag session ended: operation={operation:?}");
            let Some(shared) =
                SHARED.lock().unwrap().as_ref().and_then(Weak::upgrade)
            else {
                return;
            };
            let events = shared.dnd.lock().unwrap().source_ended(performed);
            shared.emit(events);
        }

        #[unsafe(method(ignoreModifierKeysForDraggingSession:))]
        fn ignore_modifier_keys(&self, _session: &NSDraggingSession) -> bool {
            false
        }
    }
);

impl DragSource {
    fn new(mtm: MainThreadMarker, mask: NSDragOperation) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DragSourceIvars {
            mask: Cell::new(mask),
        });
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// What a drag's content puts on the pasteboard.
enum Payload {
    Files(Vec<String>),
    Text(String),
}

fn payload(offer: &Offer) -> Option<Payload> {
    if let Some(bytes) = offer.data("text/uri-list") {
        let urls: Vec<String> = String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect();
        if !urls.is_empty() {
            return Some(Payload::Files(urls));
        }
    }
    for mime in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"] {
        if let Some(bytes) = offer.data(mime) {
            return Some(Payload::Text(
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
    }
    None
}

fn operation_mask(actions: DndAction) -> NSDragOperation {
    let mut mask = NSDragOperation::empty();
    if actions.contains(DndAction::Copy) {
        mask |= NSDragOperation::Copy;
    }
    if actions.contains(DndAction::Move) {
        mask |= NSDragOperation::Move;
    }
    if mask.is_empty() {
        mask = NSDragOperation::Copy;
    }
    mask
}

/// Turn iced's rendered drag icon into an `NSImage`. Pixels arrive as
/// premultiplied BGRA (`lib.rs` swaps rgba to "argb little endian").
fn image_from_icon(
    icon: &Icon,
    scale: f64,
) -> Option<(Retained<NSImage>, NSSize)> {
    let Icon::Buffer {
        data,
        width,
        height,
        ..
    } = icon
    else {
        return None;
    };
    let (w, h) = (*width as usize, *height as usize);
    if w == 0 || h == 0 || data.len() < w * h * 4 {
        return None;
    }
    // SAFETY: planes is null so AppKit allocates the buffer; the sizes match.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            w as isize,
            h as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            (w * 4) as isize,
            32,
        )
    }?;
    let dst = rep.bitmapData();
    if dst.is_null() {
        return None;
    }
    // SAFETY: AppKit allocated w*h*4 bytes at `dst`.
    let out = unsafe { std::slice::from_raw_parts_mut(dst, w * h * 4) };
    for (src, px) in data.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        px[0] = src[2];
        px[1] = src[1];
        px[2] = src[0];
        px[3] = src[3];
    }
    let size = NSSize::new(w as f64 / scale, h as f64 / scale);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    image.addRepresentation(&rep);
    Some((image, size))
}

/// Start an AppKit drag session for `offer`. Returns `false` when AppKit
/// cannot run it, in which case the caller uses the loopback.
pub(crate) fn begin_session(
    shared: &Arc<Shared>,
    mtm: MainThreadMarker,
    view_key: usize,
    offer: &Offer,
    icon: Option<&Icon>,
    actions: DndAction,
) -> bool {
    let Some(payload) = payload(offer) else {
        log::debug!(target: LOG, "drag has no file URLs or text; using the loopback");
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(event) = app.currentEvent() else {
        return false;
    };
    if !matches!(
        event.r#type(),
        NSEventType::LeftMouseDragged | NSEventType::LeftMouseDown
    ) {
        log::debug!(
            target: LOG,
            "current event is {:?}, not a mouse drag; using the loopback",
            event.r#type()
        );
        return false;
    }
    // SAFETY: a key is a live `NSView` pointer (see `appkit::sample_at`).
    let view: &NSView = unsafe { &*(view_key as *const NSView) };
    let Some(window) = view.window() else {
        return false;
    };
    let scale = window.backingScaleFactor();
    let at = view.convertPoint_fromView(event.locationInWindow(), None);

    // SAFETY: reading AppKit constants.
    let (file_url_type, string_type) =
        unsafe { (NSPasteboardTypeFileURL, NSPasteboardTypeString) };
    let writers: Vec<Retained<NSPasteboardItem>> = match &payload {
        Payload::Files(urls) => urls
            .iter()
            .filter_map(|url| {
                let item = NSPasteboardItem::new();
                let ns = NSString::from_str(url);
                // Finder wants a path URL, not whatever encoding we were given.
                let normalised = NSURL::URLWithString(&ns)
                    .and_then(|u| u.absoluteString())
                    .unwrap_or(ns);
                item.setString_forType(&normalised, file_url_type)
                    .then_some(item)
            })
            .collect(),
        Payload::Text(text) => {
            let item = NSPasteboardItem::new();
            item.setString_forType(&NSString::from_str(text), string_type);
            vec![item]
        }
    };
    if writers.is_empty() {
        return false;
    }

    let image = icon.and_then(|i| image_from_icon(i, scale));
    let items: Vec<Retained<NSDraggingItem>> = writers
        .iter()
        .enumerate()
        .map(|(i, writer)| {
            let writer: &ProtocolObject<
                dyn objc2_app_kit::NSPasteboardWriting,
            > = ProtocolObject::from_ref(&**writer);
            // SAFETY: `writer` conforms to NSPasteboardWriting.
            let item = NSDraggingItem::initWithPasteboardWriter(
                NSDraggingItem::alloc(),
                writer,
            );
            let (contents, size): (Option<&AnyObject>, NSSize) =
                match image.as_ref() {
                    // Only the first item carries the picture; the rest ride underneath it.
                    Some((img, size)) if i == 0 => (Some(img), *size),
                    Some((_, size)) => (None, *size),
                    None => (None, NSSize::new(1.0, 1.0)),
                };
            let frame = NSRect::new(
                NSPoint::new(at.x - size.width / 2.0, at.y - size.height / 2.0),
                size,
            );
            // SAFETY: contents is an NSImage or nil.
            unsafe { item.setDraggingFrame_contents(frame, contents) };
            item
        })
        .collect();

    *SHARED.lock().unwrap() = Some(Arc::downgrade(shared));
    let source = DragSource::new(mtm, operation_mask(actions));
    let events = shared.dnd.lock().unwrap().begin_source_session();
    shared.emit(events);
    let session = view.beginDraggingSessionWithItems_event_source(
        &NSArray::from_retained_slice(&items),
        &event,
        ProtocolObject::from_ref(&*source),
    );
    session.setAnimatesToStartingPositionsOnCancelOrFail(true);
    *shared.hooks.drag_source.lock().unwrap() =
        Some(MainThreadCell::new(source, mtm));
    log::debug!(
        target: LOG,
        "drag session started with {} item(s), image={}",
        items.len(),
        image.is_some()
    );
    true
}
