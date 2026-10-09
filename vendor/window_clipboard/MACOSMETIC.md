# Vendored `window_clipboard`

Upstream: `https://github.com/pop-os/window_clipboard.git`, tag `sctk-0.20`,
commit `f68595ee0e62fbd6589f4709b5aaa5c3c7ea5f6c`. That is the revision
`Cargo.lock` pinned before vendoring. The root `Cargo.toml` points the three
crates libcosmic pulls from that repo (`window_clipboard`, `dnd`, `mime`) at
this directory through `[patch.'https://github.com/pop-os/window_clipboard']`.

## Why

Upstream's macOS backend (`src/platform/macos.rs`) implemented every
`DndProvider` method as an empty stub and `peek_offer` returned an error. iced
routes all drag and drop through that trait, so on macOS no drag inside the
app, no drop from Finder and no drag to another app ever reached a widget.

## Changes against upstream

| Path | Change |
|---|---|
| `Cargo.toml` | macOS dependencies: `log`, `objc2`, `objc2-app-kit`, `objc2-foundation`, `block2` (versions already in the app's lock). Dev-dependencies and `examples/` removed; they pulled winit 0.29 for nothing. |
| `src/lib.rs` | macOS platform module path is now `platform/macos/mod.rs`; the pure state tests also run on other platforms. |
| `src/platform/macos/mod.rs` | the old stub, now a real `DndProvider`: sender storage, destination registry, `start_dnd`, `set_action`, `peek_offer`. |
| `src/platform/macos/state.rs` | new. Pure state machine with unit tests. Mirrors the event sequence of `smithay-clipboard/src/dnd/state.rs`. |
| `src/platform/macos/appkit.rs` | new. `NSEvent` local monitor that drives in-process drags without file URLs (tab reordering); view geometry. |
| `src/platform/macos/destination.rs` | new. Runtime subclass of winit's content view with the `NSDraggingDestination` methods, so file drags from any app, this one included, arrive as a `text/uri-list` offer. |
| `src/platform/macos/source.rs` | new. `NSDraggingSession` for drags that carry file URLs, with iced's rendered icon as the drag image, so other apps can take the files. |
| `libcosmic-init-dnd.patch` | the one-line libcosmic change this backend needs (below). |

Everything else is byte-for-byte upstream. Linux, Windows and the other
platform files are untouched.

## How a drag flows

| Drag | Path |
|---|---|
| Files, started here | `source.rs` starts an AppKit session. Our own windows receive it through `destination.rs` like any other app's drag. Session end produces the source events. |
| Anything without file URLs, started here | `appkit.rs` pointer monitor feeds `state.rs` directly; data comes from the widget's content. |
| Files from another app | `destination.rs`; data is a `text/uri-list` built from the pasteboard's file URLs. Option copies, Command moves, default follows the destination's preferred action. |

## Requirement on libcosmic

The `shythulu/iced` branch `macosmetic` initializes the event sender on macOS
as well as Wayland. `shythulu/libcosmic` points its iced submodule at that fork,
and the app's lockfile pins the updated libcosmic revision. This connects the
backend to iced; without it the backend logs "iced never called init_dnd on
this platform". `libcosmic-init-dnd.patch` records the original proposed fix.

## Tests

```
cd vendor/window_clipboard
CARGO_TARGET_DIR=<shared target> cargo test -p window_clipboard --lib
```

## Updating

Diff this directory against the upstream tag, re-apply the table above, and
keep the lock's `window_clipboard` entries pointing at the path.
