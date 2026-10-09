//! Pure DnD state machine for the macOS backend.
//!
//! It owns the registered destination rectangles and the offer in flight, and
//! turns pointer samples, drops and cancels into the `DndEvent` sequence the
//! iced widgets were written against. The sequence mirrors the Wayland backend
//! in `smithay-clipboard/src/dnd/state.rs`, which is the only backend libcosmic
//! was ever tested on. Nothing in here touches AppKit, so it unit-tests on any
//! platform.
//!
//! Drivers (the in-process pointer monitor and the `NSDraggingDestination`
//! methods) call the transitions and forward the returned events to the sender.

use dnd::{DndAction, DndDestinationRectangle, DndEvent, OfferEvent, SourceEvent};
use mime::AsMimeTypes;

/// Identity of a window surface. In production it is the `NSView` pointer.
pub type SurfaceKey = usize;

/// Keyboard modifiers that pick the drop action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Option held: copy.
    pub option: bool,
    /// Command held: move.
    pub command: bool,
}

/// Where the dragged data comes from.
pub enum Offer {
    /// A drag this process started. The data is the widget's own content.
    Internal(Box<dyn AsMimeTypes + Send>),
    /// A drag from another app. Data snapshotted from the dragging pasteboard,
    /// in order of preference.
    External(Vec<(String, Vec<u8>)>),
}

impl Offer {
    pub fn mime_types(&self) -> Vec<String> {
        match self {
            Offer::Internal(content) => content.available().to_vec(),
            Offer::External(items) => items.iter().map(|(m, _)| m.clone()).collect(),
        }
    }

    pub fn data(&self, mime: &str) -> Option<Vec<u8>> {
        match self {
            Offer::Internal(content) => {
                content.as_bytes(mime).map(|bytes| bytes.into_owned())
            }
            Offer::External(items) => items
                .iter()
                .find(|(m, _)| m == mime)
                .map(|(_, data)| data.clone()),
        }
    }

    fn has(&self, mime: &str) -> bool {
        match self {
            Offer::Internal(content) => {
                content.available().iter().any(|m| m == mime)
            }
            Offer::External(items) => items.iter().any(|(m, _)| m == mime),
        }
    }
}

/// What a drop did, so the AppKit destination can answer `performDragOperation:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropOutcome {
    /// Data was delivered to a destination with this action.
    Accepted(DndAction),
    /// The destination asked for a choice; `set_action` finishes the drop.
    AwaitingAction,
    /// No destination under the pointer, or no usable mime type.
    Rejected,
}

struct Registered<S> {
    key: SurfaceKey,
    surface: S,
    rects: Vec<DndDestinationRectangle>,
}

struct Active {
    offer: Offer,
    source_actions: DndAction,
    /// This process is the drag source, so it also receives `SourceEvent`s.
    internal: bool,
    surface: Option<SurfaceKey>,
    dest: Option<DndDestinationRectangle>,
    selected_mime: Option<String>,
    selected_action: DndAction,
    modifiers: Modifiers,
    awaiting_action: bool,
}

/// Destination registry plus the offer in flight.
pub struct Dnd<S> {
    destinations: Vec<Registered<S>>,
    active: Option<Active>,
}

impl<S> Default for Dnd<S> {
    fn default() -> Self {
        Self {
            destinations: Vec::new(),
            active: None,
        }
    }
}

impl<S: Clone> Dnd<S> {
    /// Replace the rectangles for a surface. An empty list removes it.
    pub fn register(
        &mut self,
        key: SurfaceKey,
        surface: S,
        rects: Vec<DndDestinationRectangle>,
    ) {
        self.destinations.retain(|r| r.key != key);
        if !rects.is_empty() {
            self.destinations.push(Registered {
                key,
                surface,
                rects,
            });
        }
    }

    pub fn surface_keys(&self) -> impl Iterator<Item = SurfaceKey> + '_ {
        self.destinations.iter().map(|r| r.key)
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn is_internal(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.internal)
    }

    pub fn awaiting_action(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.awaiting_action)
    }

    /// The action the destination under the pointer would get. Empty when
    /// there is no destination.
    pub fn selected_action(&self) -> DndAction {
        self.active
            .as_ref()
            .map(|a| a.selected_action)
            .unwrap_or(DndAction::empty())
    }

    pub fn offer_mime_types(&self) -> Vec<String> {
        self.active
            .as_ref()
            .map(|a| a.offer.mime_types())
            .unwrap_or_default()
    }

    /// Start tracking an offer. A still-active offer is cancelled first.
    pub fn begin(
        &mut self,
        offer: Offer,
        source_actions: DndAction,
        internal: bool,
    ) -> Vec<DndEvent<S>> {
        let mut events = self.cancel();
        self.active = Some(Active {
            offer,
            source_actions,
            internal,
            surface: None,
            dest: None,
            selected_mime: None,
            selected_action: DndAction::empty(),
            modifiers: Modifiers::default(),
            awaiting_action: false,
        });
        events.clear();
        events
    }

    /// Replace the data of an external offer, for example right before the
    /// drop when the pasteboard is read again.
    pub fn refresh_external(&mut self, items: Vec<(String, Vec<u8>)>) {
        if let Some(active) = self.active.as_mut() {
            if !active.internal {
                active.offer = Offer::External(items);
            }
        }
    }

    /// Feed a pointer sample. `at` is `None` when the pointer is over none of
    /// the registered surfaces.
    pub fn pointer(
        &mut self,
        at: Option<(SurfaceKey, f64, f64)>,
        modifiers: Modifiers,
    ) -> Vec<DndEvent<S>> {
        let mut events = Vec::new();
        let Some(active) = self.active.as_mut() else {
            return events;
        };
        let modifiers_changed = active.modifiers != modifiers;
        active.modifiers = modifiers;

        let Some((key, x, y)) = at else {
            if active.surface.take().is_some() {
                active.dest = None;
                active.selected_mime = None;
                active.selected_action = DndAction::empty();
                events.push(DndEvent::Offer(None, OfferEvent::Leave));
            }
            return events;
        };

        let Some(registered) = self.destinations.iter().find(|r| r.key == key)
        else {
            if active.surface.take().is_some() {
                active.dest = None;
                active.selected_mime = None;
                active.selected_action = DndAction::empty();
                events.push(DndEvent::Offer(None, OfferEvent::Leave));
            }
            return events;
        };

        let entering = active.surface != Some(key);
        if entering {
            if active.surface.is_some() {
                // Surfaces changed under the pointer without an outside sample.
                active.dest = None;
                active.selected_mime = None;
                active.selected_action = DndAction::empty();
                events.push(DndEvent::Offer(None, OfferEvent::Leave));
            }
            active.surface = Some(key);
        }

        Self::update_dest(active, registered, x, y, &mut events);

        if entering {
            // The Wayland backend sends a bare surface Enter after the
            // destination Enter. Widgets ignore it unless they accept any mime.
            events.push(DndEvent::Offer(
                active.dest.as_ref().map(|d| d.id),
                OfferEvent::Enter {
                    x,
                    y,
                    mime_types: Vec::new(),
                    surface: registered.surface.clone(),
                },
            ));
        } else {
            if modifiers_changed {
                if let Some(dest) = active.dest.as_ref() {
                    let action =
                        select_action(active.source_actions, dest, modifiers);
                    if action != active.selected_action {
                        active.selected_action = action;
                        events.push(DndEvent::Offer(
                            Some(dest.id),
                            OfferEvent::SelectedAction(action),
                        ));
                        if active.internal {
                            events.push(DndEvent::Source(SourceEvent::Action(
                                action,
                            )));
                        }
                    }
                }
            }
            events.push(DndEvent::Offer(
                active.dest.as_ref().map(|d| d.id),
                OfferEvent::Motion { x, y },
            ));
        }
        events
    }

    fn update_dest(
        active: &mut Active,
        registered: &Registered<S>,
        x: f64,
        y: f64,
        events: &mut Vec<DndEvent<S>>,
    ) {
        let old_id = active.dest.as_ref().map(|d| d.id);
        let hit = registered.rects.iter().find(|r| {
            let mime_ok = r.mime_types.is_empty()
                || r.mime_types.iter().any(|m| active.offer.has(m));
            let action_ok = r.actions.is_all()
                || active.source_actions.intersects(r.actions);
            contains(&r.rectangle, x, y) && mime_ok && action_ok
        });

        let Some(dest) = hit else {
            if let Some(old) = old_id {
                events.push(DndEvent::Offer(
                    Some(old),
                    OfferEvent::LeaveDestination,
                ));
                active.dest = None;
                active.selected_mime = None;
                active.selected_action = DndAction::empty();
                if active.internal {
                    events.push(DndEvent::Source(SourceEvent::Mime(None)));
                }
            }
            return;
        };

        if old_id == Some(dest.id) {
            return;
        }
        if let Some(old) = old_id {
            events.push(DndEvent::Offer(
                Some(old),
                OfferEvent::LeaveDestination,
            ));
        }
        events.push(DndEvent::Offer(
            Some(dest.id),
            OfferEvent::Enter {
                x,
                y,
                mime_types: dest
                    .mime_types
                    .iter()
                    .map(|m| m.to_string())
                    .collect(),
                surface: registered.surface.clone(),
            },
        ));
        // Like the Wayland backend: a destination that lists no mime types
        // never selects one, so it gets Enter/Motion/Drop but no Data.
        let mime = dest
            .mime_types
            .iter()
            .find(|m| active.offer.has(m))
            .map(|m| m.to_string());
        let action = select_action(active.source_actions, dest, active.modifiers);
        events.push(DndEvent::Offer(
            Some(dest.id),
            OfferEvent::SelectedAction(action),
        ));
        if active.internal {
            events.push(DndEvent::Source(SourceEvent::Mime(mime.clone())));
            events.push(DndEvent::Source(SourceEvent::Action(action)));
        }
        active.dest = Some(dest.clone());
        active.selected_mime = mime;
        active.selected_action = action;
    }

    /// The pointer was released.
    pub fn release(&mut self) -> (DropOutcome, Vec<DndEvent<S>>) {
        let mut events = Vec::new();
        let Some(active) = self.active.as_mut() else {
            return (DropOutcome::Rejected, events);
        };
        let id = active.dest.as_ref().map(|d| d.id);
        events.push(DndEvent::Offer(id, OfferEvent::Drop));

        let usable = id.is_some()
            && !active.selected_action.is_empty()
            && active.selected_mime.is_some();
        if !usable {
            if active.internal {
                events.push(DndEvent::Source(SourceEvent::Cancelled));
            }
            self.active = None;
            return (DropOutcome::Rejected, events);
        }

        if active.selected_action == DndAction::Ask {
            active.awaiting_action = true;
            events.push(DndEvent::Offer(
                id,
                OfferEvent::SelectedAction(DndAction::Ask),
            ));
            return (DropOutcome::AwaitingAction, events);
        }

        if active.internal {
            events.push(DndEvent::Source(SourceEvent::Dropped));
        }
        let outcome = self.deliver(&mut events);
        (outcome, events)
    }

    /// The app chose an action after an `Ask` drop.
    pub fn set_action(&mut self, action: DndAction) -> (DropOutcome, Vec<DndEvent<S>>) {
        let mut events = Vec::new();
        let Some(active) = self.active.as_mut() else {
            return (DropOutcome::Rejected, events);
        };
        if !active.awaiting_action {
            return (DropOutcome::Rejected, events);
        }
        active.awaiting_action = false;
        active.selected_action = action;
        if active.internal {
            events.push(DndEvent::Source(SourceEvent::Dropped));
        }
        let outcome = self.deliver(&mut events);
        (outcome, events)
    }

    fn deliver(&mut self, events: &mut Vec<DndEvent<S>>) -> DropOutcome {
        let Some(active) = self.active.take() else {
            return DropOutcome::Rejected;
        };
        let id = active.dest.as_ref().map(|d| d.id);
        let data = active
            .selected_mime
            .as_deref()
            .and_then(|m| active.offer.data(m).map(|d| (d, m.to_string())));
        match data {
            Some((data, mime_type)) => {
                events.push(DndEvent::Offer(
                    id,
                    OfferEvent::Data { data, mime_type },
                ));
                if active.internal {
                    events.push(DndEvent::Source(SourceEvent::Finished));
                }
                DropOutcome::Accepted(active.selected_action)
            }
            None => {
                if active.internal {
                    events.push(DndEvent::Source(SourceEvent::Cancelled));
                }
                DropOutcome::Rejected
            }
        }
    }

    /// Abort the offer in flight. Destinations get `Leave`; an internal source
    /// gets `Cancelled`.
    pub fn cancel(&mut self) -> Vec<DndEvent<S>> {
        let mut events = Vec::new();
        let Some(active) = self.active.take() else {
            return events;
        };
        if active.surface.is_some() {
            events.push(DndEvent::Offer(None, OfferEvent::Leave));
        }
        if active.internal {
            events.push(DndEvent::Source(SourceEvent::Cancelled));
        }
        events
    }

    /// An external drag left the window. AppKit starts a fresh offer when it
    /// comes back, so the offer is dropped here.
    pub fn external_exit(&mut self) -> Vec<DndEvent<S>> {
        if self.is_internal() {
            return Vec::new();
        }
        self.cancel()
    }

    /// An outgoing AppKit drag session ended.
    pub fn source_ended(&mut self, performed: bool) -> Vec<DndEvent<S>> {
        let mut events = Vec::new();
        if self.active.take().is_none() {
            return events;
        }
        if performed {
            events.push(DndEvent::Source(SourceEvent::Dropped));
            events.push(DndEvent::Source(SourceEvent::Finished));
        } else {
            events.push(DndEvent::Source(SourceEvent::Cancelled));
        }
        events
    }

    /// Forget the offer without telling anyone (iced's `end_dnd`).
    pub fn end(&mut self) {
        self.active = None;
    }

    /// Read offer data without finishing the drop.
    pub fn peek(&self, mime: Option<&str>) -> Option<(Vec<u8>, String)> {
        let active = self.active.as_ref()?;
        let mime = mime
            .map(str::to_string)
            .or_else(|| active.selected_mime.clone())
            .or_else(|| active.offer.mime_types().first().cloned())?;
        active.offer.data(&mime).map(|d| (d, mime))
    }
}

fn contains(r: &dnd::Rectangle, x: f64, y: f64) -> bool {
    r.x <= x && x <= r.x + r.width && r.y <= y && y <= r.y + r.height
}

/// Pick the drop action for a destination.
///
/// Option forces copy and Command forces move when the destination and the
/// source both allow it. Otherwise the destination's preferred action wins,
/// then the first action both sides share.
pub fn select_action(
    source: DndAction,
    dest: &DndDestinationRectangle,
    modifiers: Modifiers,
) -> DndAction {
    let allowed = source & dest.actions;
    if modifiers.option && allowed.contains(DndAction::Copy) {
        return DndAction::Copy;
    }
    if modifiers.command && allowed.contains(DndAction::Move) {
        return DndAction::Move;
    }
    let preferred = allowed & dest.preferred;
    first_of(preferred, &[DndAction::Move, DndAction::Copy, DndAction::Ask])
        .or_else(|| {
            first_of(allowed, &[DndAction::Copy, DndAction::Move, DndAction::Ask])
        })
        .unwrap_or(DndAction::empty())
}

fn first_of(set: DndAction, order: &[DndAction]) -> Option<DndAction> {
    order.iter().copied().find(|a| set.contains(*a))
}

/// Build a `text/uri-list` body from `file://` URLs, one per line, CRLF
/// terminated, the way `ClipboardCopy` in cosmic-files writes it.
pub fn uri_list(urls: impl IntoIterator<Item = String>) -> Vec<u8> {
    let mut out = String::new();
    for url in urls {
        out.push_str(&url);
        out.push_str("\r\n");
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    struct Files;

    impl AsMimeTypes for Files {
        fn available(&self) -> Cow<'static, [String]> {
            Cow::Owned(vec![
                "text/plain".to_string(),
                "text/uri-list".to_string(),
                "x-special/gnome-copied-files".to_string(),
            ])
        }

        fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
            match mime_type {
                "text/uri-list" => Some(Cow::Borrowed(b"file:///a\r\n")),
                "x-special/gnome-copied-files" => {
                    Some(Cow::Borrowed(b"copy\nfile:///a"))
                }
                "text/plain" => Some(Cow::Borrowed(b"/a")),
                _ => None,
            }
        }
    }

    fn rect(id: u128, x: f64, y: f64, w: f64, h: f64) -> DndDestinationRectangle {
        DndDestinationRectangle {
            id,
            rectangle: dnd::Rectangle {
                x,
                y,
                width: w,
                height: h,
            },
            mime_types: vec![
                Cow::Borrowed("x-special/gnome-copied-files"),
                Cow::Borrowed("text/uri-list"),
            ],
            actions: DndAction::Copy | DndAction::Move,
            preferred: DndAction::Move,
        }
    }

    fn internal() -> Dnd<u8> {
        let mut dnd = Dnd::default();
        dnd.register(1, 7, vec![rect(10, 0.0, 0.0, 100.0, 100.0), rect(20, 200.0, 0.0, 100.0, 100.0)]);
        let events = dnd.begin(Offer::Internal(Box::new(Files)), DndAction::Copy | DndAction::Move, true);
        assert!(events.is_empty());
        dnd
    }

    fn offer_ids(events: &[DndEvent<u8>]) -> Vec<(Option<u128>, String)> {
        events
            .iter()
            .map(|e| match e {
                DndEvent::Offer(id, ev) => (
                    *id,
                    match ev {
                        OfferEvent::Enter { mime_types, .. } => format!("Enter{mime_types:?}"),
                        OfferEvent::Motion { .. } => "Motion".into(),
                        OfferEvent::LeaveDestination => "LeaveDestination".into(),
                        OfferEvent::Leave => "Leave".into(),
                        OfferEvent::Drop => "Drop".into(),
                        OfferEvent::SelectedAction(a) => format!("SelectedAction({a:?})"),
                        OfferEvent::Data { mime_type, .. } => format!("Data({mime_type})"),
                    },
                ),
                DndEvent::Source(s) => (None, format!("Source({s:?})")),
            })
            .collect()
    }

    #[test]
    fn enter_rect_emits_enter_then_selected_action_then_surface_enter() {
        let mut dnd = internal();
        let events = dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let got = offer_ids(&events);
        assert_eq!(got[0].0, Some(10));
        assert!(got[0].1.starts_with("Enter[\"x-special/gnome-copied-files\""), "{got:?}");
        assert_eq!(got[1], (Some(10), "SelectedAction(DndAction(Move))".into()));
        assert_eq!(got.last().unwrap(), &(Some(10), "Enter[]".into()));
    }

    #[test]
    fn motion_inside_rect_is_motion_only() {
        let mut dnd = internal();
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let events = dnd.pointer(Some((1, 20.0, 20.0)), Modifiers::default());
        assert_eq!(offer_ids(&events), vec![(Some(10), "Motion".into())]);
    }

    #[test]
    fn moving_between_rects_leaves_then_enters() {
        let mut dnd = internal();
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let events = dnd.pointer(Some((1, 250.0, 10.0)), Modifiers::default());
        let got = offer_ids(&events);
        assert_eq!(got[0], (Some(10), "LeaveDestination".into()));
        assert_eq!(got[1].0, Some(20));
        assert!(got.iter().any(|(id, e)| *id == Some(20) && e == "Motion"));
    }

    #[test]
    fn leaving_all_rects_sends_leave_destination_and_untargeted_motion() {
        let mut dnd = internal();
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let events = dnd.pointer(Some((1, 150.0, 150.0)), Modifiers::default());
        let got = offer_ids(&events);
        assert_eq!(got[0], (Some(10), "LeaveDestination".into()));
        assert_eq!(got.last().unwrap(), &(None, "Motion".into()));
    }

    #[test]
    fn leaving_the_surface_sends_leave_once() {
        let mut dnd = internal();
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let events = dnd.pointer(None, Modifiers::default());
        assert_eq!(offer_ids(&events), vec![(None, "Leave".into())]);
        assert!(dnd.pointer(None, Modifiers::default()).is_empty());
        assert!(dnd.is_active(), "an internal drag survives leaving the window");
    }

    #[test]
    fn option_selects_copy_and_command_selects_move() {
        let mut dnd = internal();
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let events = dnd.pointer(Some((1, 10.0, 10.0)), Modifiers { option: true, command: false });
        assert!(offer_ids(&events).contains(&(Some(10), "SelectedAction(DndAction(Copy))".into())));
        let events = dnd.pointer(Some((1, 10.0, 10.0)), Modifiers { option: false, command: true });
        assert!(offer_ids(&events).contains(&(Some(10), "SelectedAction(DndAction(Move))".into())));
        let events = dnd.pointer(Some((1, 10.0, 10.0)), Modifiers { option: false, command: true });
        assert_eq!(offer_ids(&events), vec![(Some(10), "Motion".into())], "unchanged modifiers stay quiet");
    }

    #[test]
    fn drop_on_rect_delivers_preferred_mime_then_finishes_source() {
        let mut dnd = internal();
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let (outcome, events) = dnd.release();
        assert_eq!(outcome, DropOutcome::Accepted(DndAction::Move));
        assert_eq!(
            offer_ids(&events),
            vec![
                (Some(10), "Drop".into()),
                (None, "Source(Dropped)".into()),
                (Some(10), "Data(x-special/gnome-copied-files)".into()),
                (None, "Source(Finished)".into()),
            ]
        );
        assert!(!dnd.is_active());
    }

    #[test]
    fn drop_outside_any_rect_cancels() {
        let mut dnd = internal();
        dnd.pointer(Some((1, 150.0, 150.0)), Modifiers::default());
        let (outcome, events) = dnd.release();
        assert_eq!(outcome, DropOutcome::Rejected);
        assert_eq!(
            offer_ids(&events),
            vec![(None, "Drop".into()), (None, "Source(Cancelled)".into())]
        );
        assert!(!dnd.is_active());
    }

    #[test]
    fn ask_waits_for_set_action() {
        let mut dnd = Dnd::<u8>::default();
        let mut r = rect(10, 0.0, 0.0, 100.0, 100.0);
        r.actions = DndAction::Ask;
        r.preferred = DndAction::Ask;
        dnd.register(1, 7, vec![r]);
        dnd.begin(Offer::Internal(Box::new(Files)), DndAction::all(), true);
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let (outcome, events) = dnd.release();
        assert_eq!(outcome, DropOutcome::AwaitingAction);
        assert_eq!(offer_ids(&events).last().unwrap(), &(Some(10), "SelectedAction(DndAction(Ask))".into()));
        let (outcome, events) = dnd.set_action(DndAction::Copy);
        assert_eq!(outcome, DropOutcome::Accepted(DndAction::Copy));
        assert!(offer_ids(&events).iter().any(|(_, e)| e.starts_with("Data(")));
    }

    #[test]
    fn rect_with_foreign_mimes_is_skipped_for_the_next_match() {
        let mut dnd = Dnd::<u8>::default();
        let mut tabs = rect(5, 0.0, 0.0, 100.0, 100.0);
        tabs.mime_types = vec![Cow::Borrowed("x-cosmic-files/tab-dnd")];
        dnd.register(1, 7, vec![tabs, rect(10, 0.0, 0.0, 100.0, 100.0)]);
        dnd.begin(Offer::Internal(Box::new(Files)), DndAction::Copy | DndAction::Move, true);
        let events = dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        assert_eq!(offer_ids(&events)[0].0, Some(10));
    }

    #[test]
    fn external_offer_uses_snapshot_and_exit_drops_it() {
        let mut dnd = Dnd::<u8>::default();
        dnd.register(1, 7, vec![rect(10, 0.0, 0.0, 100.0, 100.0)]);
        dnd.begin(
            Offer::External(vec![("text/uri-list".into(), b"file:///x\r\n".to_vec())]),
            DndAction::Copy | DndAction::Move,
            false,
        );
        let events = dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        assert_eq!(offer_ids(&events)[0].0, Some(10));
        assert!(!events.iter().any(|e| matches!(e, DndEvent::Source(_))), "no source events for a foreign drag");
        assert_eq!(dnd.peek(None), Some((b"file:///x\r\n".to_vec(), "text/uri-list".into())));
        let (outcome, events) = dnd.release();
        assert_eq!(outcome, DropOutcome::Accepted(DndAction::Move));
        assert_eq!(offer_ids(&events), vec![(Some(10), "Drop".into()), (Some(10), "Data(text/uri-list)".into())]);
    }

    #[test]
    fn external_exit_sends_leave_and_forgets() {
        let mut dnd = Dnd::<u8>::default();
        dnd.register(1, 7, vec![rect(10, 0.0, 0.0, 100.0, 100.0)]);
        dnd.begin(Offer::External(vec![]), DndAction::Copy, false);
        dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        let events = dnd.external_exit();
        assert_eq!(offer_ids(&events), vec![(None, "Leave".into())]);
        assert!(!dnd.is_active());
    }

    #[test]
    fn registering_empty_removes_the_surface() {
        let mut dnd = internal();
        dnd.register(1, 7, Vec::new());
        assert_eq!(dnd.surface_keys().count(), 0);
        let events = dnd.pointer(Some((1, 10.0, 10.0)), Modifiers::default());
        assert!(events.is_empty());
    }

    #[test]
    fn uri_list_is_crlf_terminated() {
        let body = uri_list(["file:///a".to_string(), "file:///b%20c/".to_string()]);
        assert_eq!(body, b"file:///a\r\nfile:///b%20c/\r\n");
    }
}
