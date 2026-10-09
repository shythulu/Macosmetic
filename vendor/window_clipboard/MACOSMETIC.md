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
| `src/lib.rs` | macOS platform module path is now `platform/macos/mod.rs`. |
| `src/platform/macos/mod.rs` | the old stub, now a real `DndProvider`: sender storage, destination registry, `start_dnd`, `set_action`, `peek_offer`. |
| `src/platform/macos/state.rs` | new. Pure state machine with unit tests. Mirrors the event sequence of `smithay-clipboard/src/dnd/state.rs`. |
| `src/platform/macos/appkit.rs` | new. `NSEvent` local monitor that drives in-process drags; view geometry; `NSDraggingDestination` methods for drops from other apps. |

Everything else is byte-for-byte upstream. Linux, Windows and the other
platform files are untouched.

## Requirement on libcosmic

`iced/winit/src/clipboard.rs` in libcosmic calls `init_dnd` only under
`#[cfg(wayland_platform)]`. Without that call this backend has no channel to
iced and logs "iced never called init_dnd on this platform". The
`shythulu/libcosmic` branch `macosmetic` needs that `cfg` line removed (one
line, `Clipboard::connect`). See `docs/` notes in the Macosmetic repo and
`/Users/shylo/dev/macosmetic-research/09-dnd-spike.md`.

## Tests

```
cd vendor/window_clipboard
CARGO_TARGET_DIR=<shared target> cargo test -p window_clipboard --lib
```

## Updating

Diff this directory against the upstream tag, re-apply the table above, and
keep the lock's `window_clipboard` entries pointing at the path.
