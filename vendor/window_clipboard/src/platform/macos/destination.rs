//! Drops from other apps: `NSDraggingDestination` on winit's content view.
//!
//! winit registers the *window* for `NSFilenamesPboardType` and answers the
//! dragging messages in its `WindowDelegate`, which iced turns into
//! position-less `FileHovered`/`FileDropped` events nobody uses. AppKit lets a
//! registered view under the pointer win over the window, so this module gives
//! the content view the methods instead. The view's class is swapped at
//! runtime for a subclass of whatever class winit gave it (`WinitView`), with
//! the five dragging methods added. Nothing in winit is overridden; the
//! subclass only adds selectors `WinitView` never had.
//!
//! One `Shared` is reachable from the methods through a process-wide `Weak`,
//! because Objective-C method implementations get no closure context. iced
//! keeps a single clipboard for the process, so this is not a restriction.

use super::appkit::{modifiers, MainThreadCell};
use super::state::{uri_list, Offer, SurfaceKey};
use super::{Shared, LOG};
use dnd::DndAction;

use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, ProtocolObject, Sel};
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{
    NSDragOperation, NSDraggingInfo, NSEvent, NSPasteboard, NSPasteboardTypeFileURL, NSView,
};
use objc2_foundation::{NSArray, NSObjectProtocol, NSURL};
use std::ffi::CStr;
use std::sync::{Arc, Mutex, OnceLock, Weak};

const CLASS_NAME: &CStr = c"MacosmeticDndView";

static SHARED: Mutex<Option<Weak<Shared>>> = Mutex::new(None);
static CLASS: OnceLock<MainThreadCell<&'static AnyClass>> = OnceLock::new();

/// Make a registered view accept file drags from other apps.
pub(crate) fn install(shared: &Arc<Shared>, mtm: MainThreadMarker, key: SurfaceKey) {
    *SHARED.lock().unwrap() = Some(Arc::downgrade(shared));

    // SAFETY: keys are live `NSView` pointers (see `appkit::sample_at`).
    let view: &NSView = unsafe { &*(key as *const NSView) };
    let current = view.class();
    if current.name() == CLASS_NAME {
        return;
    }
    let class = CLASS.get_or_init(|| MainThreadCell::new(build_class(current), mtm));
    let class: &'static AnyClass = class.get(mtm);
    if class.superclass().map(|s| s.name()) != Some(current.name()) {
        log::warn!(
            target: LOG,
            "view class {:?} differs from the one the DnD subclass was built on; drops from other apps disabled for it",
            current.name()
        );
        return;
    }
    let object: &AnyObject = view;
    // SAFETY: the subclass adds no instance variables and only new selectors,
    // so the object's layout and every existing method are unchanged.
    unsafe { AnyObject::set_class(object, class) };
    // SAFETY: reading an AppKit constant.
    let file_url = unsafe { NSPasteboardTypeFileURL };
    view.registerForDraggedTypes(&NSArray::from_slice(&[file_url]));
    let is_content_view = view
        .window()
        .and_then(|w| w.contentView())
        .is_some_and(|cv| std::ptr::eq(&*cv as *const NSView, view as *const NSView));
    log::debug!(
        target: LOG,
        "view {key:#x} now accepts file drags from other apps: class={:?} responds={} types={} content_view={is_content_view}",
        view.class().name(),
        view.respondsToSelector(sel!(draggingEntered:)),
        view.registeredDraggedTypes().len(),
    );
}

fn build_class(superclass: &AnyClass) -> &'static AnyClass {
    let mut builder = ClassBuilder::new(CLASS_NAME, superclass)
        .expect("MacosmeticDndView already registered");
    // SAFETY: each signature matches the NSDraggingDestination declaration.
    unsafe {
        builder.add_method(
            sel!(draggingEntered:),
            dragging_entered as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
        builder.add_method(
            sel!(draggingUpdated:),
            dragging_updated as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
        builder.add_method(
            sel!(draggingExited:),
            dragging_exited as unsafe extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(prepareForDragOperation:),
            prepare_for_drag_operation as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
        builder.add_method(
            sel!(performDragOperation:),
            perform_drag_operation as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
        builder.add_method(
            sel!(wantsPeriodicDraggingUpdates),
            wants_periodic_dragging_updates as unsafe extern "C-unwind" fn(_, _) -> _,
        );
    }
    builder.register()
}

fn shared() -> Option<Arc<Shared>> {
    SHARED.lock().unwrap().as_ref().and_then(Weak::upgrade)
}

/// The dragging pasteboard as mime-typed items. File URLs become one
/// `text/uri-list` body; anything else is ignored.
fn read_pasteboard(pasteboard: &NSPasteboard) -> Vec<(String, Vec<u8>)> {
    let mut urls = Vec::new();
    // SAFETY: reading an AppKit constant.
    let file_url_type = unsafe { NSPasteboardTypeFileURL };
    if let Some(items) = pasteboard.pasteboardItems() {
        for item in items.iter() {
            let Some(text) = item.stringForType(file_url_type) else {
                continue;
            };
            // Finder hands out file-reference URLs (`file:///.file/id=...`).
            // Resolve them to paths so `Url::to_file_path` works downstream.
            let resolved = NSURL::URLWithString(&text)
                .and_then(|u| u.filePathURL())
                .and_then(|u| u.absoluteString())
                .map(|s| s.to_string())
                .unwrap_or_else(|| text.to_string());
            urls.push(resolved);
        }
    }
    if urls.is_empty() {
        return Vec::new();
    }
    log::trace!(target: LOG, "pasteboard offers {} file URLs", urls.len());
    vec![("text/uri-list".to_string(), uri_list(urls))]
}

/// Actions the source allows, in `dnd` terms.
fn source_actions(mask: NSDragOperation) -> DndAction {
    let mut actions = DndAction::empty();
    if mask.intersects(NSDragOperation::Copy | NSDragOperation::Generic) {
        actions |= DndAction::Copy;
    }
    if mask.contains(NSDragOperation::Move) {
        actions |= DndAction::Move;
    }
    if actions.is_empty() {
        actions = DndAction::Copy;
    }
    actions
}

fn operation_for(action: DndAction) -> NSDragOperation {
    if action.contains(DndAction::Move) {
        NSDragOperation::Move
    } else if action.contains(DndAction::Copy) {
        NSDragOperation::Copy
    } else {
        NSDragOperation::None
    }
}

fn location_in(view: &NSView, info: &ProtocolObject<dyn NSDraggingInfo>) -> (f64, f64) {
    let p = view.convertPoint_fromView(info.draggingLocation(), None);
    (p.x, p.y)
}

fn track(shared: &Shared, view: &NSView, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
    let key = view as *const NSView as SurfaceKey;
    let (x, y) = location_in(view, info);
    let mods = modifiers(NSEvent::modifierFlags_class());
    let (events, action) = {
        let mut dnd = shared.dnd.lock().unwrap();
        let events = dnd.pointer(Some((key, x, y)), mods);
        (events, dnd.selected_action())
    };
    shared.emit(events);
    operation_for(action)
}

unsafe extern "C-unwind" fn dragging_entered(
    this: &NSView,
    _sel: Sel,
    info: &ProtocolObject<dyn NSDraggingInfo>,
) -> NSDragOperation {
    log::trace!(target: LOG, "draggingEntered:");
    let Some(shared) = shared() else {
        return NSDragOperation::None;
    };
    if shared.dnd.lock().unwrap().is_internal() {
        // An in-process drag is tracked by the pointer monitor already.
        return NSDragOperation::None;
    }
    let items = read_pasteboard(&info.draggingPasteboard());
    if items.is_empty() {
        return NSDragOperation::None;
    }
    let actions = source_actions(info.draggingSourceOperationMask());
    log::debug!(target: LOG, "external drag entered, source actions {actions:?}");
    let events = shared
        .dnd
        .lock()
        .unwrap()
        .begin(Offer::External(items), actions, false);
    shared.emit(events);
    track(&shared, this, info)
}

unsafe extern "C-unwind" fn dragging_updated(
    this: &NSView,
    _sel: Sel,
    info: &ProtocolObject<dyn NSDraggingInfo>,
) -> NSDragOperation {
    let Some(shared) = shared() else {
        return NSDragOperation::None;
    };
    if shared.dnd.lock().unwrap().is_internal() {
        return NSDragOperation::None;
    }
    track(&shared, this, info)
}

unsafe extern "C-unwind" fn dragging_exited(
    _this: &NSView,
    _sel: Sel,
    _info: Option<&ProtocolObject<dyn NSDraggingInfo>>,
) {
    let Some(shared) = shared() else {
        return;
    };
    log::debug!(target: LOG, "external drag exited");
    let events = shared.dnd.lock().unwrap().external_exit();
    shared.emit(events);
}

unsafe extern "C-unwind" fn prepare_for_drag_operation(
    _this: &NSView,
    _sel: Sel,
    _info: &ProtocolObject<dyn NSDraggingInfo>,
) -> Bool {
    Bool::YES
}

unsafe extern "C-unwind" fn perform_drag_operation(
    this: &NSView,
    _sel: Sel,
    info: &ProtocolObject<dyn NSDraggingInfo>,
) -> Bool {
    let Some(shared) = shared() else {
        return Bool::NO;
    };
    if shared.dnd.lock().unwrap().is_internal() {
        return Bool::NO;
    }
    let items = read_pasteboard(&info.draggingPasteboard());
    shared.dnd.lock().unwrap().refresh_external(items);
    track(&shared, this, info);
    let (outcome, events) = shared.dnd.lock().unwrap().release();
    log::debug!(target: LOG, "external drop: {outcome:?}");
    shared.emit(events);
    Bool::new(!matches!(outcome, super::state::DropOutcome::Rejected))
}

unsafe extern "C-unwind" fn wants_periodic_dragging_updates(_this: &NSView, _sel: Sel) -> Bool {
    Bool::NO
}

