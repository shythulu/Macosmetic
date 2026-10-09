# Vendored `cosmic-freedesktop-icons`

Source: https://github.com/pop-os/freedesktop-icons, commit `ab4c57b8e416c6af9297cb04d101889896fd9a92`
("fix: wrong icons selected on size mismatch"), the revision `Cargo.lock` pinned before
the vendoring. Licence: MIT, see `LICENSE`.

The repo's `Cargo.toml` points both this app's and libcosmic's dependency at this copy
through `[patch.'https://github.com/pop-os/freedesktop-icons']`.

## Why

The crate reads the installed themes once per process into `THEMES`, a `LazyLock`. A
theme the app installs into `~/.local/share/icons` was invisible to the lookup until a
restart.

## What changed

| File | Change |
|---|---|
| `src/theme/mod.rs` | `THEMES` is a `LazyLock<RwLock<ThemeMap>>`. New `themes()` returns the read guard. New `pub fn reload_themes()` rescans the base paths under the write lock and clears the lookup cache. |
| `src/lib.rs` | Readers take the guard once per lookup and pass the map down to `search_theme_inherits` and `search_inherited_theme`, so no lookup takes the lock twice. `reload_themes` is re-exported. |
| `src/theme/parse.rs` | The `local_tests` test reads through `themes()`. |

Nothing else differs from upstream. `BASE_PATHS` is still a `LazyLock`: it only lists
directories that exist at first use, and `launch_macos::prepare` creates
`~/.local/share/icons` before that.

## Updating

Copy the new upstream revision over this directory, reapply the two changes above, and
update the commit hash here.
