# Theme selection outside the COSMIC desktop

Where does "pick a theme" live in the COSMIC stack, what does libcosmic hand a standalone
app, and what would cosmic-files have to build to ship its own theme picker on macOS?

Each claim is tagged **[V]** verified against a primary source (the pinned libcosmic
source on this machine, a file on this machine, a compiled-and-run probe, or the upstream
repository's own code) or **[I]** inferred. Claims marked *"checked here"* were run or
read on this machine rather than taken from a report.

Every libcosmic path below is relative to the **pinned revision
`d9431dc3670575602385e1e2523600ba5315508c`** (`Cargo.lock:4480-4482`), unpacked at
`~/.local/share/cargo/git/checkouts/libcosmic-41009aea1d72760b/d9431dc/`. Note that
`CARGO_HOME` on this machine is `~/.local/share/cargo`, not `~/.cargo`.

`cosmic-settings`, `cosmic-settings-daemon`, `cosmic-initial-setup` and the third-party
projects in §4.5 are not vendored here; those claims were read from raw files on `master`
(fetched 2026-09-21) and are marked as such.

---

## 1. Short answer

**Theme selection is not a libcosmic feature.** libcosmic only ever *reads* a theme out of
`cosmic-config` and applies it. The selection UI lives entirely inside the
`cosmic-settings` binary, in a private module (`src/pages/desktop/appearance/`), and none
of it is exported as a library.

**But a theme catalog does exist — twice — just not in libcosmic or cosmic-settings.**
Two directory conventions are live today, and both are already populated by third parties:

| Convention | Read by | Path | Naming |
|---|---|---|---|
| First-party | `pop-os/cosmic-initial-setup` | `/usr/share/cosmic-themes/*.ron` (NixOS: `/run/current-system/sw/share/cosmic-themes/`) | display name = file stem, `-`→space, title-cased; preview = sibling `<stem>.png`; dark iff stem ends `dark` |
| Community | `cosmic-utils/tweaks` | `$XDG_DATA_HOME/themes/cosmic/`, each `$XDG_DATA_DIRS/themes/cosmic/`, `/usr/local/share/themes/cosmic/`, `/usr/share/themes/cosmic/` | display name = file stem |

Both store the same payload: **one `.ron` file holding a serialized
`cosmic_theme::ThemeBuilder`**, which is also exactly what cosmic-settings' Import/Export
round-trips. **[V]** — see §4.5.

So the accurate split is:

- **Applying a theme to our own process:** fully supported public API today
  (`cosmic::command::set_theme`, `cosmic::theme::Theme::custom`, `ThemeBuilder::build`).
  Nothing to write.
- **Enumerating themes:** no API in libcosmic and no catalog in cosmic-settings — **[V]**,
  `"cosmic-themes"` returns zero hits in either repo — but the *file format and the
  directory layout are already settled by convention* (§4.5), so we adopt rather than
  invent. The scanner itself we still write; it is about forty lines.
- **Editing a theme:** `ThemeBuilder`'s builder methods are public, but the staging /
  diff-only-write machinery around them (`theme_manager::Manager`) is cosmic-settings
  private code.

**And on macOS the existing `AppTheme` setting is currently a no-op.** All three of
Dark/Light/System resolve to `cosmic-dark`, because the `com.system76.CosmicTheme.*`
config stores are empty on a machine with no COSMIC desktop — on a real install they are
filled by a package (§4.5) — and libcosmic's fallback for an empty store is `Theme::default()` → `preferred_theme()` → `dark_default()`. **[V] ran a
probe here** — see §6.4. That bug, not the picker, is the thing worth fixing first.

---

## 2. Verified against this repo and this machine

| Finding | Status |
|---|---|
| Our `AppTheme` enum has three values and resolves them through `cosmic::theme::system_{dark,light,preference}` | **[V]** `src/config.rs:36-57` |
| We apply it with `cosmic::command::set_theme` on every config change, and at startup via `Settings::theme` | **[V]** `src/app.rs:1811`, `src/lib.rs:116` |
| `theme::system_light()` returns the **dark** theme on this Mac | **[V] probe, checked here** — see §6.4 |
| `~/Library/Application Support/cosmic/com.system76.CosmicTheme.{Light,Dark,Mode}/` exist but contain **zero files** | **[V] checked here** (`find` over that tree returns only `com.system76.CosmicFiles/v1/*`) |
| `XDG_CONFIG_HOME` is ignored on macOS; cosmic-config uses `dirs::config_dir()` = `~/Library/Application Support` | **[V] probe, checked here** |
| Writing `Theme::light_default()` into the light store makes it read back light | **[V] probe, checked here** |
| No `~/.local/share/cosmic` and no `/opt/homebrew/share/cosmic` on this machine — no system-default theme data anywhere on the search path | **[V] checked here** |
| libcosmic's D-Bus config path is `#[cfg(all(feature = "dbus-config", target_os = "linux"))]`, so macOS always uses the file watcher | **[V]** `src/core.rs:392` |
| The xdg-portal light/dark/accent subscription is compiled out on macOS (`xdg_portal` alias requires `free_unix`) | **[V]** `iced/build_helpers/src/lib.rs:16,21`; `src/app/cosmic.rs:666-669` |
| libcosmic contains **no** theme-enumeration code of any kind | **[V]** grep for `read_dir` / `.ron` / `themes` across `src/`, `cosmic-theme/src/`, `cosmic-config/src/` returns only the two `include_str!` palette files |

---

## 3. Where theme selection lives

### 3.1 libcosmic reads; it never lets the user choose

The whole of libcosmic's theme story is three functions in `src/theme/mod.rs`:

```rust
// src/theme/mod.rs:88
pub fn system_dark() -> Theme {
    let Ok(helper) = crate::cosmic_theme::Theme::dark_config() else { return Theme::dark(); };
    let t = crate::cosmic_theme::Theme::get_entry(&helper).unwrap_or_else(|(errors, theme)| { … theme });
    Theme::system(Arc::new(t))
}
// src/theme/mod.rs:103  system_light()  — same, light_config()
// src/theme/mod.rs:119
pub fn system_preference() -> Theme {
    let Ok(mode_config) = ThemeMode::config() else { return Theme::dark(); };
    let Ok(is_dark) = ThemeMode::is_dark(&mode_config) else { return Theme::dark(); };
    if is_dark { system_dark() } else { system_light() }
}
```

That is the entire "which theme am I" decision. It is a read of two cosmic-config stores.
There is no list, no name, no directory. **[V]**

The active theme is a process-global `Mutex<Theme>` — `src/theme/mod.rs:47`, `pub(crate)`,
so an app cannot poke it directly; it goes through the runtime (§5.2).

### 3.2 cosmic-settings owns the UI, and keeps it to itself

The picker is `cosmic-settings/src/pages/desktop/appearance/`, a module tree inside the
**binary** crate: `mod.rs` (the `Page`), `theme_manager.rs` (`Manager` / `ThemeCustomizer`
— the live `ThemeBuilder`/`Theme` state and every config write), `mode_and_colors.rs`,
`style.rs`, `icon_themes.rs`, `font_config.rs`, `drawer.rs`, `commands.rs`.
**[V]** upstream `master`, see §9.

Two things matter for us:

1. **It is not a theme list — in *this* app.** The page's controls are: Light/Dark buttons,
   an auto-switch toggle, an accent-colour palette, background/text/control tint pickers,
   corner roundness presets, density, fonts, frosted glass. `ContextView` in `mod.rs`
   enumerates exactly those drawers (`AccentWindowHint, ApplicationBackground,
   ContainerBackground, ControlComponent, FrostedGlass, ShadowAndCorners, CustomAccent,
   IconsAndToolkit, InterfaceText, MonospaceFont, SystemFont`). The only `read_dir` in the
   tree is `icon_themes::fetch()`, which walks `$XDG_DATA_HOME/.local/share/icons` and each
   `$XDG_DATA_DIRS/icons` looking for freedesktop `index.theme` manifests — icon themes,
   not colour themes. A search of `pop-os/cosmic-settings` for `"cosmic-themes"` returns
   zero hits, so it does not know about the catalog directory that its sibling
   `cosmic-initial-setup` scans (§4.5). **[V]**

   Worth knowing: the call site that kicks that off in `Page::on_enter()` is currently
   **commented out** upstream. Whether that is dead code, a regression or deliberate is
   *unverified* — there is no comment explaining it.

2. **None of it is a library.** `theme_manager::Manager` — the staging model, the
   diff-only-write in `build_theme()`, the `ThemeStaged::{Current,Both}` propagation that
   keeps spacing/radii/frosted-glass in sync across light and dark, the derived
   `accent_palette` bootstrap — is a private module inside a binary crate. So is the
   `commands.rs` CLI `import_theme`/`export_theme`. A third-party app gets none of it.
   **[V]**

There is no standalone COSMIC theme editor either. `pop-os/cosmic-theme-editor` was
archived on 2024-02-08 and is an unmodified GTK4/Meson/Flatpak boilerplate — its README is
still the template's — depending on `pop-os/cosmic-theme`, itself archived 2023-05-22 with
a one-line `# WIP` README. Both predate the `cosmic-theme` crate now living inside
libcosmic. **[V]**

### 3.3 cosmic-settings-daemon owns the side effects

`cosmic-settings-daemon` watches the same config IDs with `notify` and reacts:
re-reads `Theme::get_entry`, and if `CosmicTk.apply_theme_global` is set, runs
`theme.write_exports()` / `apply_exports()` (GTK3/GTK4/Qt colour files) and
`gsettings set org.gnome.desktop.interface color-scheme …`. It also owns the
sunrise/sunset `auto_switch`, computed from geolocation. **[V]** upstream `master`,
`src/theme.rs` and `src/main.rs`.

This is the part a standalone app does *not* get by writing the configs itself. Writing
`com.system76.CosmicTheme.Dark` from cosmic-files would repaint cosmic-files and any other
running libcosmic app, but would not touch GTK apps unless the daemon is running.

### 3.4 The call path, end to end

User clicks "Dark" in cosmic-settings:

1. `appearance::Message` → `theme_manager::Manager::dark_mode(true)`
   → `ThemeMode::set_is_dark(&config, true)` (a setter generated by the
   `CosmicConfigEntry` derive: `cosmic-config-derive/src/lib.rs:164`)
   → `Config::set("is_dark", true)` → `ConfigTransaction::commit()`
   → `ron::ser::to_string_pretty` → `atomicwrites::AtomicFile` at
   `$CONFIG/cosmic/com.system76.CosmicTheme.Mode/v1/is_dark`.
   (`cosmic-config/src/lib.rs:447-495`) **[V]**
2. Every running libcosmic app has a subscription on that ID:
   `src/app/cosmic.rs:651-664` → `Core::watch_config::<ThemeMode>(cosmic_theme::THEME_MODE_ID)`
   → `cosmic_config::config_subscription` → `Config::watch` (a `notify` recursive watcher
   on the user dir, `cosmic-config/src/lib.rs:332-385`), or on Linux with the daemon
   present, `cosmic_config::dbus::watcher_subscription` (`src/core.rs:392-399`). **[V]**
3. The app receives `app::Action::SystemThemeModeChange(keys, mode)`
   (`src/app/action.rs:54`), which updates `Core::system_theme_mode` and re-derives the
   active theme (`src/app/cosmic.rs:1050-1110`), and separately
   `Action::SystemThemeChange(keys, Theme)` for the colour data itself
   (`src/app/cosmic.rs:620-650, 933-1000`).
4. `THEME.lock().unwrap().set_theme(...)` — the process-global is updated, and every
   widget that calls `cosmic::theme::active()` picks it up on the next draw.
5. The app's own `Application::system_theme_update` / `system_theme_mode_update` hooks fire
   (`src/app/mod.rs:469-485`) so it can react. **We implement the first one** —
   `src/app.rs:6855-6861` maps it to `Message::SystemThemeModeChange` →
   `update_config()`.

---

## 4. The storage / IPC contract

### 4.1 Config IDs and versions

All public, all in `cosmic-theme`:

| Const | Value | Type | `VERSION` |
|---|---|---|---|
| `cosmic_theme::THEME_MODE_ID` | `com.system76.CosmicTheme.Mode` | `ThemeMode` | 1 |
| `cosmic_theme::LIGHT_THEME_ID` | `com.system76.CosmicTheme.Light` | `Theme` | 2 |
| `cosmic_theme::DARK_THEME_ID` | `com.system76.CosmicTheme.Dark` | `Theme` | 2 |
| `cosmic_theme::LIGHT_THEME_BUILDER_ID` | `com.system76.CosmicTheme.Light.Builder` | `ThemeBuilder` | 2 |
| `cosmic_theme::DARK_THEME_BUILDER_ID` | `com.system76.CosmicTheme.Dark.Builder` | `ThemeBuilder` | 2 |

`cosmic-theme/src/model/mode.rs:4,11-16`; `cosmic-theme/src/model/theme.rs:16-26,50-51,
846-848`. **[V]**

`ThemeMode` is two bools: `is_dark`, `auto_switch` (`mode.rs:11-16`).
The pair-per-mode matters: `*.Builder` is the *editable* input, `com.system76.CosmicTheme.*`
is the *built output*. cosmic-settings writes both; libcosmic only reads the output.

### 4.2 On disk

`Config::new(name, version)` resolves to:

```rust
// cosmic-config/src/lib.rs:219-247
let path = sanitize_name(name)?.join(format!("v{}", version));
#[cfg(unix)]  let system_path = xdg::BaseDirectories::with_prefix("cosmic").find_data_file(&path);
let mut user_path = get_config_dir().ok_or(Error::NoConfigDirectory)?;
user_path.push("cosmic");
user_path.push(path);
fs::create_dir_all(&user_path)?;
```

with `get_config_dir()` = `dirs::config_dir()`, plus a `HOST_XDG_CONFIG_HOME` branch for
Flatpak (`lib.rs:13-35`). **[V]**

So: **one directory per config ID per version, and one file per struct field**, with the
field name as the filename and no extension. The file contains that field's value
serialized as RON via `ron::ser::to_string_pretty` (`lib.rs:487`), written through
`atomicwrites` (`lib.rs:468-480`). Reads are `ron::from_str` (`lib.rs:421,441`).

Checked here, after writing `Theme::light_default()` into a throwaway config root:

```
…/cosmic/com.system76.CosmicTheme.Light/v2/
  accent  accent_button  accent_text  active_hint  alpha_map  background  button
  control_tint  corner_radii  destructive  …  is_dark  is_high_contrast  name
  palette  primary  secondary  shade  spacing  …  window_hint        (38 files)

$ cat name      → "cosmic-light"
$ cat is_dark   → false
$ cat accent    → (
                      base: "#00525AFF",
                      hover: "#14555CFF",
                      …
                  )
```

**[V] probe, checked here.**

Lookup order for a key is: user file → system default file → previous config version.
`Config::new` builds a `previous` chain for `version > 1` (`lib.rs:245-249`), which is why
both `…/v1/` and `…/v2/` directories exist for the theme IDs on this machine even though
only v2 is current. System defaults come from `find_data_file` over XDG data dirs on unix
(i.e. `/usr/share/cosmic/...`), or `%CommonProgramFiles%\COSMIC\...` on Windows
(`lib.rs:192-198`). **[V]**

### 4.3 cosmic-settings' own import/export: a bare file round-trip

cosmic-settings has no install path of its own. Its import/export is a portal file chooser
plus a RON round-trip of one `ThemeBuilder`:

```rust
// cosmic-settings/src/pages/desktop/appearance/mod.rs:323-341 (upstream master)
Message::StartImport => file_chooser::open::Dialog::new().modal(true)
    .filter(FileFilter::glob(FileFilter::new("ron"), "*.ron")).open_file().await,
// :345-352
Message::StartExport => {
    let is_dark = self.theme_manager.mode().is_dark;
    let name = format!("{}.ron", if is_dark { fl!("dark") } else { fl!("light") });
    file_chooser::save::Dialog::new().modal(true).file_name(name).save_file().await
}
// :383  import is literally this, with no validation:
ron::de::from_str(&s)
```

Three things to note, all **[V]** against `mod.rs` as fetched:

- **No default directory.** `file_name("dark.ron")` sets a suggested *filename* only; there
  is no `current_folder` call, so the portal opens wherever it last was. cosmic-settings
  does not point at `/usr/share/cosmic-themes/` or `themes/cosmic/`.
- **No validation and no user-visible error.** A file that fails `ron::de` produces a
  `tracing::error!` and a `Message::ImportError` that returns `Task::none()`. The source
  carries a `// TODO Error toast?` on both paths.
- Import writes the deserialized builder to the matching `*.Builder` store and
  `builder.build()` to the matching output store, then flips `ThemeMode.is_dark` to match
  `builder.palette.is_dark()`.

The buttons are behind `#[cfg(feature = "xdg-portal")]` (`mod.rs:709-710`). There is a CLI
equivalent in `commands.rs` (`import_theme`/`export_theme`).

This is why every community theme gallery's README says the same thing — e.g.
`KodeBarista/cosmic-themes`: *"Open Cosmic Settings. Navigate to Desktop > Appearance.
Import desired theme file."* The manual import is the baseline; the catalogs in §4.5 are
what the rest of the ecosystem built on top of it.

### 4.4 Change notification

Three mechanisms, in preference order:

1. **cosmic-settings-daemon over D-Bus.** Generic, not theme-specific:
   bus name `com.system76.CosmicSettingsDaemon`, interface
   `com.system76.CosmicSettingsDaemon.Config`, one object per `(id, version)` at
   `/com/system76/CosmicSettingsDaemon/Config/<id with dots as slashes>/V<version>`,
   emitting `changed(id: String, key: String)`. Consumed by
   `cosmic_config::dbus::watcher_subscription(proxy, config_id, is_state)`
   (`cosmic-config/src/dbus.rs:76-88`). **[V]** daemon side from upstream `master`.
2. **Direct file watch.** `Config::watch(F)` where `F: Fn(&Config, &[String])` builds a
   `notify::RecommendedWatcher` on the user directory, strips the prefix to recover key
   names, skips `.atomicwrite*` temp files (`cosmic-config/src/lib.rs:332-385`). Wrapped
   for iced by `cosmic_config::config_subscription(id, config_id, version)` →
   `Subscription<Update<T>>` (`cosmic-config/src/subscription.rs:22-48`). **[V]**
3. **xdg-desktop-portal `org.freedesktop.portal.Settings`.** `color-scheme`, `contrast`
   and accent colour, via `ashpd`, in `src/theme/portal.rs:15-102`, surfacing as
   `app::Action::DesktopSettings`. Linux-only (§6.2). **[V]**

`Core::watch_config` picks 1 if the daemon proxy is present *and* we are on Linux, else 2
(`src/core.rs:386-405`). **[V]**

### 4.5 How themes are delivered on a real install

Three delivery layers exist. None of them is libcosmic's.

**(a) The system-default config layer, `/usr/share/cosmic/`.** This is a deliberate,
packaged extension point, not an accident of `find_data_file`. `cosmic-settings` ships the
default themes itself:

```
# cosmic-settings/justfile:8,35
default-schema-target := usrdir / 'share' / 'cosmic'
    cd resources/default_schema && find * -type f -exec install -Dm0644 '{}' '{{default-schema-target}}/{}' \;

# cosmic-settings/debian/install:37-41
/usr/share/cosmic/com.system76.CosmicTheme.Dark
/usr/share/cosmic/com.system76.CosmicTheme.Dark.Builder
/usr/share/cosmic/com.system76.CosmicTheme.Light
/usr/share/cosmic/com.system76.CosmicTheme.Light.Builder
/usr/share/cosmic/com.system76.CosmicTheme.Mode
```

`resources/default_schema/com.system76.CosmicTheme.Light/v2/` holds 37 files — `accent`,
`background`, `is_dark`, `name`, `palette`, `spacing`, … — byte-for-byte the same
one-file-per-field layout documented in §4.2. **[V]** The `.Builder/v2/` tree holds the 20
builder inputs, and `.Mode/v1/` holds `auto_switch` and `is_dark`. The same
`default_schema` → `/usr/share/cosmic` pattern is used by `cosmic-panel` and
`cosmic-applets`. **[V]**

So **the empty `com.system76.CosmicTheme.*` directories on this Mac are empty precisely
because we do not install cosmic-settings** — that package is what fills them. §6.1's
suggestion of shipping the same tree in `Contents/Resources/share` is doing exactly what
the distro package does. **[V]** mechanism; **[I]** that it works from inside the bundle.

**(b) The first-party theme catalog, `/usr/share/cosmic-themes/`.** Read by
`pop-os/cosmic-initial-setup` (active; a cosmic-epoch component), `src/page/appearance.rs`:

```rust
// :105-108
#[cfg(feature = "nixos")]
let themes_dir_path = "/run/current-system/sw/share/cosmic-themes/";
#[cfg(not(feature = "nixos"))]
let themes_dir_path = "/usr/share/cosmic-themes/";
if let Ok(directory) = std::fs::read_dir(themes_dir_path) {
// :112-114  skip anything whose extension is not "ron"
// :133      ron::de::from_bytes::<ThemeBuilder>(&buffer[..read])
// :134-143  name    = file stem, '-' -> ' ', to_title_case()
//           is_dark = name.ends_with("dark")
//           preview = widget::image::Handle::from_path(path.with_extension("png"))
```

It seeds the list with `ThemeBuilder::dark()` / `ThemeBuilder::light()` as *"COSMIC Dark"*
and *"COSMIC Light"* (`:89-101`), sorts the discovered ones into a `BTreeSet` by name, and
on selection writes `builder.write_entry(builder_config)`, `theme.write_entry(theme_config)`
and `set("is_dark", …)` into the shared stores (`:195-210`). **[V] fetched and read here.**

Third parties already ship into it — `tiiuae/ghaf`,
`modules/common/theming/cosmic/default.nix:84,97-101`:

```nix
themesDir="$out/share/cosmic-themes"
install -m0644 ${cfg.theme.dark}  "$themesDir/ghaf-dark.ron"
install -m0644 ${cfg.theme.light} "$themesDir/ghaf-light.ron"
install -m0644 ${pkgs.ghaf-artwork}/1600px-Ghaf_logo.png "$themesDir/ghaf-dark.png"
```

and NixOS's COSMIC module links the path for every package
(`nixos/modules/services/desktop-managers/cosmic.nix:76`: `"/share/cosmic-themes"` in
`environment.pathsToLink`). **[V]**

**(c) The community catalog, `themes/cosmic/` + cosmic-themes.org.** `cosmic-utils/tweaks`
("Cosmic Tweaks", on Flathub as `dev.edfloreshz.CosmicTweaks`, active) is the de-facto
theme browser. `src/app/pages/color_schemes/storage.rs`:

```rust
// :45-48
pub fn cosmic_theme_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()?.join("themes/cosmic");
// :99-100
    let path = cosmic_theme_dir()?.join(&theme.name).with_extension("ron");
// :113-131  search path, in order:
//   dirs::data_local_dir()/themes/cosmic
//   each $XDG_DATA_DIRS entry + /themes/cosmic
//   /usr/local/share/themes/cosmic
//   /usr/share/themes/cosmic
```

and it fills that directory from a live JSON API:
`GET https://cosmic-themes.org/api/themes/?limit=…`, each row carrying the theme's RON
inline. **[V] fetched here: HTTP 200, JSON array with `id, uuid, name, ron, author, link,
downloads, created, updated`.** The site is a solo Django project
(`Fingel/cosmic-themes-org-py`); it has no install instructions of its own beyond a
"Download Theme" button, and no URL-scheme handoff into cosmic-settings.

Neither libcosmic nor cosmic-settings reads either catalog directory. **[V]** — zero hits
for `"cosmic-themes"` in `pop-os/libcosmic` and `pop-os/cosmic-settings`, and no
`themes/cosmic` either.

### 4.6 What a theme file actually contains, and how stable it is

A theme file is a serialized `cosmic_theme::ThemeBuilder` — no header, no version marker,
no name field. A real one, `Fingel/cosmic-theme-collection/gruvbox-dark.ron` (285 lines):

```ron
(
    palette: Dark((
        name: "cosmic-dark",
        blue: ( red: 0.58, green: 0.92, blue: 0.92, alpha: 1.0 ),
        …
    )),
    spacing: ( … ),  corner_radii: ( … ),
    neutral_tint: Some(( red: 0.235, green: 0.219, blue: 0.211 )),
    bg_color: Some(( … )),  accent: Some(( … )),
    is_frosted: false,
    gaps: (0, 8),  active_hint: 3,
)
```

Two compatibility facts, both **[V]** against the pinned `cosmic-theme`:

- **Colours are readable in three encodings.** `ColorRepr` is `#[serde(untagged)]` over
  `Hex(HexColor)` / `Rgba(Srgba)` / `Rgb(Srgb)` (`cosmic-theme/src/model/color.rs:9-16`),
  so the struct form above *and* the `"#00525AFF"` hex strings that cosmic-settings ships
  in `default_schema` both deserialize. Serialization always writes hex. This is a
  deliberate back-compat shim.
- **Old files still load.** `is_frosted` no longer exists on the pinned `ThemeBuilder` (it
  became `frosted: BlurStrength`); serde ignores the unknown field, and `frosted`,
  `frosted_windows`, `frosted_system_interface`, `frosted_panel`, `frosted_applets`,
  `frosted_maximized_apps` and `alpha_map` all carry `#[serde(default)]`
  (`cosmic-theme/src/model/theme.rs:902-919`). The non-defaulted fields — `palette`,
  `spacing`, `corner_radii`, `gaps`, `active_hint` and the colour `Option`s — are all
  present in the old files, so they parse.

`ThemeBuilder` is `#[version = 2]` as a *cosmic-config* entry, but that version lives in
the config directory name, not in the `.ron` file. A standalone `.ron` carries no version
at all, so a loader can only report "failed to parse", never "too old". **[V]**

---

## 5. What libcosmic gives a standalone app today

This is the section that matters. Everything here is public API on the pinned revision.

### 5.1 Reading

| API | Signature / location |
|---|---|
| `cosmic::theme::active()` | `-> Theme` — the process-global active theme. `src/theme/mod.rs:57` |
| `cosmic::theme::active_type()` | `-> ThemeType`. `src/theme/mod.rs:64` |
| `cosmic::theme::is_dark()` / `is_high_contrast()` | `-> bool`. `src/theme/mod.rs:77,84` |
| `cosmic::theme::spacing()` | `-> cosmic_theme::Spacing`. `src/theme/mod.rs:70` |
| `cosmic::theme::system_dark()` / `system_light()` / `system_preference()` | `-> Theme`. `src/theme/mod.rs:88,103,119` |
| `Core::system_theme()` / `Core::system_theme_mode()` | `-> &Theme` / `-> ThemeMode`. `src/core.rs:375,382` |
| `cosmic_theme::Theme::get_active()` | `-> Result<Self, (Vec<Error>, Self)>`. `cosmic-theme/src/model/theme.rs:760` |

### 5.2 Overriding, for this process only

```rust
// src/theme/mod.rs:237
pub fn Theme::custom(theme: Arc<cosmic_theme::Theme>) -> Theme      // ThemeType::Custom
// src/theme/mod.rs:205,213,221,229
pub fn Theme::{dark, light, dark_hc, light_hc}() -> Theme
// src/command.rs:36  (feature = "winit")
pub fn cosmic::command::set_theme<M: Send + 'static>(theme: crate::Theme)
    -> iced::Task<crate::Action<M>>
// src/app/settings.rs:58 + derive_setters
pub fn Settings::theme(self, Theme) -> Settings     // startup value
```

`set_theme` emits `app::Action::AppThemeChange`, whose handler writes the process-global
and recomputes blur/transparency (`src/app/cosmic.rs:884-931`). **[V]**

The important detail: `ThemeType::Custom` is *sticky*. `Action::SystemThemeChange` only
re-applies a system theme `if let ThemeType::System { .. } = cosmic_theme.theme_type`
(`src/app/cosmic.rs:947-960`). So an app that sets `Theme::custom(...)` is fully opted out
of desktop theme changes, which is exactly what a self-contained picker wants. **[V]**

`Theme::system(...)` keeps a `prefer_dark: Option<bool>` that only selects which config ID
the subscription watches (`src/app/cosmic.rs:622-635`); it does **not** recolour the
theme (`ThemeType::prefer_dark`, `src/theme/mod.rs:175-181`). That is the trap behind the
macOS bug in §6.4.

### 5.3 Building a theme from scratch

`cosmic_theme::ThemeBuilder` is fully public and re-exported as `cosmic::cosmic_theme`
(`src/lib.rs:129`):

```rust
// cosmic-theme/src/model/theme.rs:952-1075
ThemeBuilder::{dark, light, dark_high_contrast, light_high_contrast}() -> Self
ThemeBuilder::palette(CosmicPalette) -> Self
  .spacing(Spacing) .corner_radii(CornerRadii)
  .neutral_tint(Srgb) .text_tint(Srgb)
  .bg_color(Srgba) .primary_container_bg(Srgba)
  .accent(Srgb) .success(Srgb) .warning(Srgb) .destructive(Srgb)
  .build() -> Theme
// theme.rs:773
Theme::with_accent(&self, c: Srgba) -> Theme
Theme::{light_default, dark_default, high_contrast_light_default, high_contrast_dark_default}()
Theme::to_high_contrast(&self) -> Theme
```

`ThemeBuilder` derives `Serialize`/`Deserialize`, so RON round-tripping a third-party
theme file needs no cosmic-settings code at all — `ron::de::from_str::<ThemeBuilder>(&s)?`
then `.build()` then `cosmic::command::set_theme(Theme::custom(Arc::new(built)))`. **[V]**

### 5.4 Persisting / following

| API | Location |
|---|---|
| `cosmic_config::Config::{new, new_state, system, with_custom_path}` | `cosmic-config/src/lib.rs:185,215,253,291` |
| `ConfigGet` / `ConfigSet` / `ConfigTransaction` | `lib.rs:145,158,459` |
| `CosmicConfigEntry` + `#[derive(CosmicConfigEntry)]` → `get_entry`, `write_entry`, `update_keys`, `set_<field>` | `lib.rs:497`; `cosmic-config-derive/src/lib.rs:164,192,205` |
| `cosmic_config::config_subscription::<I, T>(id, Cow<str>, u64) -> Subscription<Update<T>>` | `cosmic-config/src/subscription.rs:22` |
| `Core::watch_config::<T>(&self, &'static str) -> Subscription<Update<T>>` | `src/core.rs:386` |
| `Application::system_theme_update` / `system_theme_mode_update` | `src/app/mod.rs:470,479` |
| `iced::system::theme() -> Task<theme::Mode>` and `theme_changes() -> Subscription<theme::Mode>` | `iced/runtime/src/system.rs:58,63`, re-exported at `iced/src/lib.rs:629-631`, reachable as `cosmic::iced::system::*` (`src/lib.rs:151`) |

### 5.5 Not provided, anywhere

- Enumerating themes. **[V]** — libcosmic has no such API, and cosmic-settings implements
  no catalog. The catalogs that exist (§4.5) live in `cosmic-initial-setup` and in
  third-party tooling, each with its own private scanner; neither is exposed as a library.
- Naming a theme beyond the `Theme::name: String` field (`cosmic-theme/src/model/theme.rs:52`),
  which is set by `ThemeBuilder::build()` and never surfaced in a picker.
- Thumbnails / previews of a theme.
- The staged, diff-only write strategy (`theme_manager::Manager::build_theme`) — private
  to cosmic-settings.
- Any "reset to system" helper other than re-calling `system_preference()`.

---

## 6. macOS reality

### 6.1 Paths

`dirs::config_dir()` on macOS is `$HOME/Library/Application Support`, and cosmic-config
uses it directly — **`XDG_CONFIG_HOME` is ignored**. Verified with a probe run under an
overridden `HOME` *and* an overridden `XDG_CONFIG_HOME`: the files landed at
`$HOME/Library/Application Support/cosmic/com.system76.CosmicTheme.Light/v2/`.
**[V] probe, checked here.**

`XDG_DATA_HOME`/`XDG_DATA_DIRS` *are* honoured, but only for the system-default lookup,
because `find_data_file` sits behind `#[cfg(unix)]` and macOS is unix
(`cosmic-config/src/lib.rs:225`). Our bundle launcher already sets `XDG_DATA_DIRS`
(`src/launch_macos.rs:100-112`), so a `.../share/cosmic/com.system76.CosmicTheme.Light/v2/`
tree shipped inside `Contents/Resources/share` **would** be picked up as the system
default. That is the cheapest possible fix for §6.4 and the cheapest possible way to ship
built-in themes. **[I]** — mechanism verified, not yet tried in the bundle.

### 6.2 What is compiled out

`build.rs` → `build_helpers::cfg_aliases_setup()` defines
`xdg_portal: { all(feature = "xdg-portal", free_unix, …) }` and
`free_unix: { all(unix, not(apple), …) }` (`iced/build_helpers/src/lib.rs:16,21`). So on
macOS:

- `crate::theme::portal::desktop_settings()` — the portal colour-scheme/contrast/accent
  subscription — is not compiled (`src/app/cosmic.rs:666-669`). **[V]**
- `app::Action::DesktopSettings` does not exist (`src/app/action.rs:77-78`). **[V]**
- `Core::watch_config`'s D-Bus branch is `#[cfg(all(feature = "dbus-config", target_os =
  "linux"))]`, so the `notify` file watcher is always used (`src/core.rs:392`). **[V]**
  Our macOS build drops `dbus-config` anyway.

The `notify` file watcher works fine on macOS (FSEvents/kqueue backend), so config-change
propagation between cosmic-files windows and any future picker is not a problem.

### 6.3 The one system signal that does work: `effectiveAppearance`

winit's AppKit backend registers a KVO observer on the window's `effectiveAppearance` and
queues `WindowEvent::ThemeChanged` (winit `71ce08c`, `winit-appkit/src/window_delegate.rs:475-508`,
`:832`, `appearance_to_theme` at `:2042`). iced's winit runner turns that into
`subscription::Event::SystemThemeChanged(theme::Mode)` (`iced/winit/src/lib.rs:1333-1344`,
and an initial read at `:917-928`), which surfaces publicly as:

```rust
// iced/runtime/src/system.rs:58,63
pub fn theme() -> Task<theme::Mode>;
pub fn theme_changes() -> Subscription<theme::Mode>;
```

Note that `iced::event::listen_with` explicitly filters this event out
(`iced/futures/src/event.rs:40,69` map it to `None`) — `system::theme_changes()` is the
only way to get it. **[V]** code read; **[I]** that it fires correctly under libcosmic's
multi-window/daemon runner here — not runtime-verified.

libcosmic itself never subscribes to it. Nothing in `src/` references
`SystemThemeChanged`. So "follow macOS Appearance" is available to us as an app, but is
not wired up by the toolkit. **[V]**

### 6.4 The bug: Light/Dark/System are all dark here

Probe compiled against the pinned `cosmic-theme`/`cosmic-config` and run on this Mac:

```
XDG_CURRENT_DESKTOP = Err(NotPresent)
Theme::default().is_dark = true   name = "cosmic-dark"
ThemeMode entry OK: ThemeMode { is_dark: true, auto_switch: false }
light_config entry OK  is_dark=true  name="cosmic-dark"
Theme::light_default().is_dark = false
```

**[V] checked here.** The chain:

1. `com.system76.CosmicTheme.Light/v2/` is empty, and there is no system-default copy on
   the XDG data path.
2. `Config::new` still **succeeds** (it `create_dir_all`s the user dir), so
   `system_light()` never takes its `Theme::light()` early-return.
3. `Theme::get_entry(&config)` starts from `Self::default()` and fills in whatever keys it
   finds (`cosmic-config-derive/src/lib.rs:192-203`). It finds none, and — note — returns
   `Ok`, not `Err`, because a missing key is not pushed as an error.
4. `impl Default for Theme` is `Self::preferred_theme()`
   (`cosmic-theme/src/model/theme.rs:149-151`), and `preferred_theme()` reads
   `XDG_CURRENT_DESKTOP`, finds nothing GNOME-ish, and returns `dark_default()`
   (`theme.rs:818-830`).
5. `AppTheme::Light` then calls `t.theme_type.prefer_dark(Some(false))`
   (`src/config.rs:51-53`), which only sets a field used to choose a subscription target —
   it does not recolour anything (`src/theme/mod.rs:175-181`).

Result: `Light`, `Dark` and `System` all render `cosmic-dark`. Writing
`Theme::light_default()` into the store makes it read back `is_dark=false,
name="cosmic-light"` — **[V] probe, checked here** — so the data model is fine; the data
is simply absent.

Two fixes, not mutually exclusive: ship the two default themes as system-default data in
the bundle (§6.1), or stop routing through `system_light()`/`system_dark()` at all and use
`cosmic::theme::Theme::{light, dark}()` (which resolve to the compiled-in `COSMIC_LIGHT` /
`COSMIC_DARK` statics, `src/theme/mod.rs:22-30`) when no system theme data exists. The
second is a two-line change in `src/config.rs` and has no packaging cost.

---

## 7. The gap

To ship an in-app theme picker we would have to write:

1. **A loader for the established theme file format.** We do not get to define this and we
   should not try: a theme is a `.ron` file holding a serialized `ThemeBuilder`, which is
   what cosmic-settings exports, what cosmic-themes.org serves, what `cosmic-initial-setup`
   reads and what Catppuccin ships. `ron::de::from_str::<ThemeBuilder>(&s)?` then `.build()`
   is the whole loader. What we add is the error reporting cosmic-settings does not have
   (§4.3) — a `.ron` that fails to parse must say so in the UI, not in a log line.

   Two conventions we inherit rather than invent (§4.5): the display name is the file stem
   (`cosmic-initial-setup` additionally does `'-' -> ' '` + title-case), and light/dark is
   inferred from a `-dark` / `-light` suffix. There is no name or version inside the file,
   so both are filename-derived by necessity, not by choice. **[V]**
2. **Discovery over the two existing directory conventions**, because no library does it
   for us. Concretely, in order: `$XDG_DATA_HOME/themes/cosmic/` and each
   `$XDG_DATA_DIRS/themes/cosmic/` (the `cosmic-utils/tweaks` convention — on macOS this is
   what our bundle launcher's `XDG_DATA_DIRS` already reaches), then
   `/usr/share/cosmic-themes/` (the `cosmic-initial-setup` convention, Linux-only in
   practice), then our own `$CONFIG/cosmic/com.system76.CosmicFiles/themes/`, with
   `ThemeBuilder::{light, dark, light_high_contrast, dark_high_contrast}()` always present
   as built-ins. Sibling `<stem>.png` previews are free if we want them later.

   Following the community path means a user who already installed themes with Cosmic
   Tweaks sees them in cosmic-files with no extra step — on Linux and, because it is
   XDG-based, on macOS too. **[I]** — the paths are verified, the end-to-end experience is
   not.
3. **The picker UI.** `widget::dropdown` for a one-line change, or a grid of swatches
   rendered from each candidate's `accent_color()` / `bg_color()` / `on_bg_color()` if we
   want previews. Both are ordinary libcosmic widgets; nothing special is needed.
4. **Persistence of the choice** in our own config (§8.2), not in
   `com.system76.CosmicTheme.*`. Writing the shared store from a file manager would be
   wrong on Linux — it would retheme every other COSMIC app.
5. **Follow-the-OS on macOS**, via `cosmic::iced::system::theme_changes()` (§6.3), since
   the portal subscription is compiled out.

What we do **not** have to write: colour derivation, the palette model, serialization,
config storage, file watching, or theme application. All public and all working.

---

## 8. Spec sketch: an in-app theme picker for cosmic-files

Deliberately conservative. The aim is to make the existing setting *work* first and leave
room for themes second.

### 8.1 Two phases

**Phase 1 — make Light/Dark/System real (no new config, no new UI).**
Change `AppTheme::theme()` in `src/config.rs:43-57` so that when the system theme store
yields nothing, it falls back to the compiled-in themes rather than `Theme::default()`:

- `Dark` → `system_dark()`, but if the loaded theme's `is_dark` is false or the store was
  empty, `cosmic::theme::Theme::dark()`.
- `Light` → same shape with `Theme::light()`.
- `System` → on macOS, seed from `iced::system::theme()` and keep following with
  `iced::system::theme_changes()`; on Linux keep `system_preference()`.

Detecting "the store was empty" needs care, because `get_entry` returns `Ok` with defaults
(§6.4). The honest check is `Config::get_local::<bool>("is_dark")` (which *does* return
`Error::NotFound`) before trusting the entry, or simply comparing the returned theme
against `Theme::default()`. This is the one piece of new logic Phase 1 needs, and it
should live behind one function with a name that says what it is, e.g.
`fn system_theme_available() -> bool`.

**Phase 2 — a theme list.** Only worth it once Phase 1 works.

### 8.2 Config keys we would own

In `com.system76.CosmicFiles` (`CONFIG_VERSION` currently 1, `src/config.rs:20`), the
existing `app_theme: AppTheme` field (`src/config.rs:202`) gains a variant rather than a
new key, so old config files keep deserializing:

```rust
pub enum AppTheme {
    Dark,
    Light,
    System,
    /// A theme file, by stem, resolved against the theme search path.
    Named(String),
}
```

`serde`'s externally-tagged enum default means `Dark`/`Light`/`System` stay
wire-compatible; `Named("solarized")` is new and unknown to older builds. An older build
reading it fails the key and falls back to `AppTheme::System` — acceptable, but worth
stating. **[I]** — not tested.

Search path, first match wins:

1. `$CONFIG/cosmic/com.system76.CosmicFiles/themes/*.ron` — ours
   (`~/Library/Application Support/cosmic/com.system76.CosmicFiles/themes` on macOS)
2. `$XDG_DATA_HOME/themes/cosmic/*.ron`, then each `$XDG_DATA_DIRS/themes/cosmic/*.ron`
   — the Cosmic Tweaks convention (§4.5c). Our bundle launcher already puts
   `Contents/Resources/share` first in `XDG_DATA_DIRS` (`src/launch_macos.rs:100-112`), so
   bundled themes drop into `Contents/Resources/share/themes/cosmic/`.
3. `/usr/share/cosmic-themes/*.ron` — the `cosmic-initial-setup` convention (§4.5b),
   Linux only
4. the four compiled-in builders

Note this is *not* a cosmic-config store: cosmic-config's one-file-per-field layout is
wrong for a list of documents. A plain directory of `.ron` files read with `ron::de` is
both the right tool and the established one.

### 8.3 UI surface

The existing settings context page, `App::settings()` at `src/app.rs:2309-2334`. Today it
is a three-item `widget::dropdown` over `self.app_themes` (`src/app.rs:2483`). Phase 2
turns that into the same dropdown with the discovered names appended after the three
built-ins, which is a change to the index mapping at `src/app.rs:2317-2331` and nothing
else. A swatch grid can come later; the dropdown is enough to ship.

No import button in the first cut. Dropping a `.ron` into the themes directory is a file
manager's native idiom, and we are a file manager.

### 8.4 Interaction with the shared COSMIC stores

- We **read** `com.system76.CosmicTheme.*` (already, via `system_preference()`), and keep
  doing so for `AppTheme::System`.
- We **never write** them. A `Named` theme is applied with
  `cosmic::command::set_theme(Theme::custom(Arc::new(builder.build())))`, which sets
  `ThemeType::Custom` and therefore opts this process out of `SystemThemeChange`
  (`src/app/cosmic.rs:947`). That is the desired behaviour and it is free. **[V]**
- On Linux under COSMIC, `AppTheme::System` must keep working exactly as now, including
  the `apply_theme_global` / daemon path. Phase 1's fallback must not trigger when the
  stores are populated.

### 8.5 Migration and fallback

- Existing configs have `app_theme` ∈ {Dark, Light, System}; all three keep their meaning.
  No migration, no `CONFIG_VERSION` bump.
- A `Named` theme whose file has vanished or fails to parse → log once, fall back to
  `System`, do not rewrite the config (so plugging the drive back in restores it).
- A theme file that parses but produces an unreadable result is not our problem to
  validate; `ThemeBuilder::build()` does its own contrast derivation.

### 8.6 Open questions

- Should the dialog (`src/dialog.rs`) follow the app's theme? It is hosted out-of-process
  by the portal on Linux, in-process on macOS. The `// TODO: Should dialog be updated here
  too?` at `src/app.rs:2311` is already asking this. Unresolved.
- Do we want per-theme light/dark pairing, so that `AppTheme::Named` + macOS appearance
  switching picks a light or dark variant of the same theme? `ThemeBuilder` carries
  `palette.is_dark()`, so a pair would have to be two files plus a naming convention.
  Probably out of scope.
- Is there any value in *writing* `com.system76.CosmicTheme.*` when running standalone on
  macOS, so that other libcosmic apps on the same Mac agree? Tempting (it is the only
  shared channel that exists) but it makes a file manager into a settings daemon. Say no
  unless someone asks.
- `Theme::write_exports()` / `apply_exports()` exist in our dependency graph (the
  `export` feature is on by default, and `serde_json` is in `Cargo.lock:1574` under
  `cosmic-theme`). They write to `dirs::config_dir()/gtk-{3,4}.0`
  (`cosmic-theme/src/output/gtk4_output.rs:152-235`), which on macOS is inside
  `~/Library/Application Support`. Harmless as long as we never call them — and we should
  never call them.

---

## 9. Sources

**Primary, this machine (pinned libcosmic `d9431dc3670575602385e1e2523600ba5315508c`,
at `~/.local/share/cargo/git/checkouts/libcosmic-41009aea1d72760b/d9431dc/`):**
`src/theme/mod.rs`, `src/theme/portal.rs`, `src/app/mod.rs`, `src/app/cosmic.rs`,
`src/app/action.rs`, `src/app/settings.rs`, `src/core.rs`, `src/command.rs`,
`src/config/mod.rs`, `src/lib.rs`, `build.rs`;
`cosmic-theme/src/lib.rs`, `cosmic-theme/src/model/theme.rs`,
`cosmic-theme/src/model/mode.rs`, `cosmic-theme/src/output/mod.rs`,
`cosmic-theme/src/output/gtk4_output.rs`, `cosmic-theme/Cargo.toml`;
`cosmic-config/src/lib.rs`, `cosmic-config/src/subscription.rs`,
`cosmic-config/src/dbus.rs`, `cosmic-config/Cargo.toml`;
`cosmic-config-derive/src/lib.rs`;
`iced/build_helpers/src/lib.rs`, `iced/runtime/src/system.rs`,
`iced/futures/src/subscription.rs`, `iced/futures/src/event.rs`, `iced/winit/src/lib.rs`,
`iced/winit/src/conversion.rs`.

**Primary, this machine (winit `71ce08c043814514a8fd92d9d0599f115ae854e8`, tag
`cosmic-0.14`):** `winit-appkit/src/window.rs`, `winit-appkit/src/window_delegate.rs`.

**Primary, this repo:** `Cargo.toml`, `Cargo.lock`, `src/config.rs`, `src/app.rs`,
`src/lib.rs`, `src/main.rs`, `src/launch_macos.rs`, `src/dialog.rs`.

**Primary, this machine (empirical):** a throwaway crate compiled against the pinned
`cosmic-theme`/`cosmic-config` and run here, printing `Theme::default()`,
`ThemeMode::get_entry`, `Theme::get_entry` on the light store, and a
`write_entry`/`get_entry` round-trip under an overridden `HOME`; plus `find` and `cat`
over `~/Library/Application Support/cosmic/`. The probe was deleted afterwards and wrote
nothing into the real config directory.

**Primary, upstream (read-only, `master`, fetched 2026-09-21):**
`pop-os/cosmic-settings` —
`cosmic-settings/src/pages/desktop/appearance/{mod,theme_manager,mode_and_colors,style,icon_themes,commands}.rs`;
`pop-os/cosmic-settings-daemon` — `src/theme.rs`, `src/main.rs`.
Fetched via `raw.githubusercontent.com` and the GitHub contents API. Nothing was written,
commented on or filed anywhere upstream.

**Packaging / system defaults (upstream, read-only, `master`, fetched 2026-09-21):**
- https://github.com/pop-os/cosmic-settings/blob/master/justfile (lines 8, 35, 42)
- https://github.com/pop-os/cosmic-settings/blob/master/debian/install (lines 37-41)
- https://github.com/pop-os/cosmic-settings/tree/master/resources/default_schema — and
  `.../com.system76.CosmicTheme.Light/v2/`, `.../com.system76.CosmicTheme.Light.Builder/v2/`,
  `.../com.system76.CosmicTheme.Mode/v1/`
- https://github.com/pop-os/cosmic-panel/blob/master/README.md (the `default_schema` →
  `$HOME/.config/cosmic` pattern)

**First-party theme catalog:**
- https://raw.githubusercontent.com/pop-os/cosmic-initial-setup/master/src/page/appearance.rs
  (lines 89-155 scanner, 169-215 apply) — **fetched and re-verified in this session**
- https://github.com/pop-os/cosmic-epoch (component list includes `cosmic-initial-setup`)
- https://raw.githubusercontent.com/tiiuae/ghaf/main/modules/common/theming/cosmic/default.nix
  (lines 80-101)
- https://raw.githubusercontent.com/NixOS/nixpkgs/master/nixos/modules/services/desktop-managers/cosmic.nix
  (line 76)

**Community catalog and tooling:**
- https://github.com/cosmic-utils/tweaks — `src/app/pages/color_schemes/storage.rs`
  (lines 45-48, 99-100, 113-131) — **fetched and re-verified in this session**
- https://cosmic-themes.org and its API `https://cosmic-themes.org/api/themes/?limit=N`
  — **verified live in this session: HTTP 200, RON payloads inline**
- https://github.com/Fingel/cosmic-themes-org-py · https://github.com/Fingel/cosmic-theme-tools
  · https://github.com/Fingel/cosmic-theme-collection (`gruvbox-dark.ron`)
- https://github.com/catppuccin/cosmic-desktop — README "Usage → COSMIC Desktop
  Appearance", and `themes/cosmic-settings/*.ron`
- https://github.com/KodeBarista/cosmic-themes · https://github.com/cosmic-utils/cosmic-ext-themes

**cosmic-settings import/export:**
- https://raw.githubusercontent.com/pop-os/cosmic-settings/master/cosmic-settings/src/pages/desktop/appearance/mod.rs
  (lines 322-341, 345-365, 369-390, 709-710)

**Dead ends:**
- https://github.com/pop-os/cosmic-theme-editor (archived 2024-02-08) ·
  https://github.com/pop-os/cosmic-theme (archived 2023-05-22)

**Additionally flagged unverified:**
- That themes installed via Cosmic Tweaks would be picked up end-to-end by a cosmic-files
  scanner on macOS — the paths are verified, the experience is not.
- Whether `cosmic-initial-setup`'s `-dark` suffix rule is documented anywhere, or is only
  implicit in the code. No doc found; the only evidence is ghaf and
  `Fingel/cosmic-theme-collection` following it.
- The one-off `install.sh` scripts in individual theme repos that reportedly copy `.ron`
  straight into `~/.config/cosmic/...Builder/v2/` — only their existence is confirmed via
  code search, not deep-read.

**No secondary sources were used. No issues, pull requests or comments were opened
anywhere; all upstream access was read-only.**
