# Bundled themes

## cosmic-ext-themes

Forest Dark, HP Dev One, System76 Dark and System76 Light come from
[`cosmic-utils/cosmic-ext-themes`](https://github.com/cosmic-utils/cosmic-ext-themes),
copied unmodified on 2026-09-21. That project is GPL-3.0, the same licence as this one;
its `LICENSE` sits beside them here.

## Darkberry

The twenty `Darkberry`, `Lingonberry`, `Cloudberry`, `Crowberry` and `Blueberry` files are
[Darkberry](https://github.com/shythulu/DarkBerry)'s four flavours (Wisp, Fen, Mire,
Blackwater) in each of its five tints. They are generated, not edited by hand:
`scripts/darkberry-themes.mjs` resolves each flavour's colours through Darkberry's own
roles and writes them out, so they agree with the Darkberry ports for other apps.
Regenerate them with `node scripts/darkberry-themes.mjs [path to DarkBerry]` after a
Darkberry palette change; these were generated from Darkberry commit `9cebe55`.
Darkberry is MIT-licensed; its licence is `LICENSE-darkberry` here.

## Format

Each file is a serialized `cosmic_theme::ThemeBuilder` in RON — the format cosmic-settings
imports and exports, so these interoperate with the rest of the COSMIC ecosystem and with
anything a user installs themselves.

`scripts/macos-bundle.sh` copies this directory to
`Contents/Resources/share/themes/cosmic` inside the app bundle. The bundle's `share` is
first on `XDG_DATA_DIRS`, and `src/theme_catalog.rs` scans `themes/cosmic` under every XDG
data directory, so these are found by the same pass that finds the user's own themes — and
a user's theme of the same name takes precedence over the copy shipped here.

To add a theme, drop a `.ron` file in. The display name is the file stem, and whether it
is dark is read from its palette, so no naming convention is required. `cargo test
theme_catalog` checks that everything here still parses.

Note the cosmic-ext-themes files use `is_frosted`, a field that has since been renamed to `frosted` in
`cosmic-theme`. They still load because the replacement fields carry `#[serde(default)]`;
`vendored_themes_parse` is the regression test for that.
