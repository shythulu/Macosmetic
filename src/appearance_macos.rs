// SPDX-License-Identifier: GPL-3.0-only

//! What "match desktop" matches on macOS: the system's light or dark appearance and its
//! accent colour.
//!
//! On COSMIC, libcosmic follows the desktop's own theme settings. A Mac has none, so
//! libcosmic's system preference is its compiled-in dark theme with COSMIC's accent, whatever
//! macOS is set to. Instead, [`Look::theme`] builds libcosmic's light or dark theme around the
//! accent colour chosen in System Settings, and [`changes`] reports when either changes,
//! including when Auto switches the appearance at dusk and dawn.
//!
//! Appearance changes are observed on the application's `effectiveAppearance`, not the
//! window's: iced gives each window an appearance of its own to match the app's theme, and a
//! window with its own appearance stops following the system. Accent changes arrive as
//! AppKit's system-colours notification.
//!
//! Adapted from super-stt (github.com/jorge-menjivar/super-stt,
//! `super-stt-app/src/core/app/macos/appearance.rs`, GPL-3.0-only), less its frosted glass
//! and density.

use std::cell::{Cell, OnceCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, LazyLock};

use block2::RcBlock;
use cosmic::cosmic_theme::ThemeBuilder;
use cosmic::cosmic_theme::palette::color_difference::Wcag21RelativeContrast;
use cosmic::cosmic_theme::palette::{Clamp, FromColor, IntoColor, Oklch, Srgb};
use cosmic::iced::Subscription;
use cosmic::iced::futures::SinkExt;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSColor,
    NSColorSpace, NSSystemColorsDidChangeNotification,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSKeyValueChangeKey, NSKeyValueObservingOptions, NSNotification,
    NSNotificationCenter, NSObject, NSObjectNSKeyValueObserverRegistration, NSObjectProtocol,
    NSOperationQueue, NSString, NSUserDefaults, ns_string,
};
use tokio::sync::watch;

/// The system's appearance, as far as the theme is concerned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    dark: bool,
    /// The accent colour as macOS draws it in this appearance; `None` if it could not be read,
    /// which leaves libcosmic's own.
    accent: Option<Srgb>,
}

impl Look {
    /// Whether this is the dark appearance.
    pub fn is_dark(self) -> bool {
        self.dark
    }

    /// libcosmic's light or dark theme around this look's accent colour.
    pub fn theme(self) -> cosmic::Theme {
        let mut builder = if self.dark {
            ThemeBuilder::dark()
        } else {
            ThemeBuilder::light()
        };
        if let Some(accent) = self.accent {
            builder = builder.accent(accent);
        }
        let mut theme = builder.build();
        if let Some(accent) = self.accent {
            // libcosmic derives accent text too, but for most macOS accents on its light
            // surfaces finds no shade that reads, and falls back to black.
            let surfaces = [
                theme.bg_color().color,
                theme.primary_container_color().color,
            ];
            theme.accent_text = Some(readable_on(accent, &surfaces, self.dark).into());
        }
        cosmic::Theme::custom(Arc::new(theme))
    }
}

/// The contrast libcosmic asks of accent text against the surfaces it sits on.
const TEXT_CONTRAST: f32 = 4.0;

/// `accent`, darkened on light surfaces or lightened on dark ones just enough to read as text
/// on all of `surfaces`, keeping its hue, the way macOS darkens its own blue for links. As
/// close to white or black as it gets if no shade reads.
fn readable_on(accent: Srgb, surfaces: &[Srgb], dark: bool) -> Srgb {
    let reads = |color: Srgb| {
        surfaces
            .iter()
            .all(|surface| surface.relative_contrast(color) >= TEXT_CONTRAST)
    };
    let step = if dark { 0.01 } else { -0.01 };
    let mut shade: Oklch = accent.into_color();
    let mut color = accent;
    while !reads(color) && (0.0..=1.0).contains(&(shade.l + step)) {
        shade.l += step;
        color = Srgb::from_color(shade).clamp();
    }
    color
}

/// The system's look as last observed, `None` until [`observe`] has run.
static SYSTEM: LazyLock<watch::Sender<Option<Look>>> = LazyLock::new(|| watch::Sender::new(None));

/// The system's look now: the last one observed, or, before the observer is installed, the
/// one read from the user defaults.
pub fn current() -> Look {
    (*SYSTEM.borrow()).unwrap_or_else(launch_look)
}

/// The system's look read without the application object, which winit has yet to create, and
/// must create itself, when the first theme is chosen. This is what lets the first frame open
/// in the right appearance instead of switching after it.
///
/// `AppleInterfaceStyle` is present, as `Dark`, only while the system is dark, Auto included.
fn launch_look() -> Look {
    let style =
        NSUserDefaults::standardUserDefaults().stringForKey(ns_string!("AppleInterfaceStyle"));
    let dark = is_dark_interface_style(style.map(|style| style.to_string()).as_deref());
    // SAFETY: AppKit's own constants.
    let name = unsafe {
        if dark {
            NSAppearanceNameDarkAqua
        } else {
            NSAppearanceNameAqua
        }
    };
    let accent = NSAppearance::appearanceNamed(name).and_then(|appearance| accent(&appearance));
    Look { dark, accent }
}

/// Whether an `AppleInterfaceStyle` value names the dark appearance. macOS writes `Dark`
/// while the dark appearance is on, Auto included, and removes the key for light.
fn is_dark_interface_style(style: Option<&str>) -> bool {
    style.is_some_and(|style| style.eq_ignore_ascii_case("dark"))
}

/// The system's look from the application object, which follows it.
fn system_look(mtm: MainThreadMarker) -> Option<Look> {
    let appearance = NSApplication::sharedApplication(mtm).effectiveAppearance();
    // SAFETY: AppKit's own constants.
    let (light, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    let name =
        appearance.bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[light, dark]))?;
    Some(Look {
        dark: *name == *dark,
        accent: accent(&appearance),
    })
}

/// The accent colour as drawn in `appearance`, in sRGB: macOS gives each appearance its own
/// shade of it, and a Display P3 panel would otherwise hand back P3 components.
fn accent(appearance: &NSAppearance) -> Option<Srgb> {
    let accent = Cell::new(None);
    appearance.performAsCurrentDrawingAppearance(&RcBlock::new(|| {
        let color =
            NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace());
        #[allow(clippy::cast_possible_truncation)]
        accent.set(color.map(|color| {
            Srgb::new(
                color.redComponent() as f32,
                color.greenComponent() as f32,
                color.blueComponent() as f32,
            )
        }));
    }));
    accent.get()
}

/// Record the system's look, waking [`changes`] if it is new.
fn publish(mtm: MainThreadMarker) {
    let look = system_look(mtm);
    SYSTEM.send_if_modified(|last| {
        if *last == look {
            return false;
        }
        log::info!("system look: {look:?}");
        *last = look;
        true
    });
}

define_class!(
    /// Observes the application's `effectiveAppearance`.
    #[unsafe(super(NSObject))]
    #[name = "CosmicFilesAppearanceObserver"]
    #[thread_kind = MainThreadOnly]
    struct AppearanceObserver;

    unsafe impl NSObjectProtocol for AppearanceObserver {}

    impl AppearanceObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut c_void,
        ) {
            publish(self.mtm());
        }
    }
);

thread_local! {
    /// The registered observer. It observes the application, which lives as long as the
    /// process, so it is never unregistered or freed.
    static OBSERVER: OnceCell<Retained<AppearanceObserver>> = const { OnceCell::new() };
}

/// Start following the system's look. Safe to call repeatedly; only the first call does
/// anything.
///
/// Must run on the main thread, after winit has set up the application object, so that the
/// observer lands on the application winit drives.
pub fn observe() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("not following the system appearance: not on the main thread");
        return;
    };
    OBSERVER.with(|registered| {
        if registered.get().is_some() {
            return;
        }
        let observer = mtm.alloc::<AppearanceObserver>().set_ivars(());
        // SAFETY: `init` is `NSObject`'s initializer, which the class keeps.
        let observer: Retained<AppearanceObserver> = unsafe { msg_send![super(observer), init] };
        // SAFETY: the observer implements `observeValueForKeyPath:…` and is kept for the life
        // of the process, so it outlives the registration. The context is unused, so null.
        unsafe {
            NSApplication::sharedApplication(mtm).addObserver_forKeyPath_options_context(
                &observer,
                ns_string!("effectiveAppearance"),
                // `Initial` reports the current look straight away, which corrects the launch
                // look should it have read differently.
                NSKeyValueObservingOptions::Initial | NSKeyValueObservingOptions::New,
                std::ptr::null_mut(),
            );
        }
        // KVO does not retain its observers, and a thread-local is dropped at exit, so keep an
        // extra reference that is never released.
        std::mem::forget(observer.clone());
        let _ = registered.set(observer);

        let changed = RcBlock::new(|_: NonNull<NSNotification>| {
            if let Some(mtm) = MainThreadMarker::new() {
                publish(mtm);
            }
        });
        // SAFETY: AppKit's own notification constant; the block runs on the main queue, where
        // its AppKit calls must. The token is leaked: the observer is wanted for the life of
        // the process, and dropping the token would remove it.
        let token = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSSystemColorsDidChangeNotification),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &changed,
            )
        };
        std::mem::forget(token);
        log::info!("following the system appearance and accent colour");
    });
}

/// Each new system look. Yields nothing until [`observe`] has run.
pub fn changes() -> Subscription<Look> {
    Subscription::run(|| {
        cosmic::iced::stream::channel(1, async |mut output| {
            let mut system = SYSTEM.subscribe();
            // `subscribe` marks the current look as seen. Deliver it anyway, in case
            // `observe` published before this stream started.
            system.mark_changed();
            // The sender is a static, so it is never dropped and this never ends.
            while system.changed().await.is_ok() {
                let Some(look) = *system.borrow_and_update() else {
                    continue;
                };
                if output.send(look).await.is_err() {
                    break;
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::is_dark_interface_style;

    #[test]
    fn interface_style_names_dark() {
        assert!(is_dark_interface_style(Some("Dark")));
        assert!(is_dark_interface_style(Some("dark")));
        assert!(!is_dark_interface_style(None));
        assert!(!is_dark_interface_style(Some("Light")));
        assert!(!is_dark_interface_style(Some("")));
    }
}
