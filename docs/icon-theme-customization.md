# Icon theme customization: hand-off notes

Goal: let the user pick an icon theme in Settings, the way they pick the app theme today,
and add an in-app **icon theme browser** that downloads themes from public GitHub
repositories. The model is how System76 does it in COSMIC on Pop!_OS.

These started as research notes for whoever implements the feature.

> **Status (2026-10-07):** stage 1 is built on branch `feat/icon-themes`:
>
> - §0 is fixed by the `macosmetic` branch of `shythulu/libcosmic`, pulled in through
>   the `[patch]` in `Cargo.toml`.
> - Phase 2: an "Icons" setting that writes `CosmicTk.icon_theme`.
> - §6: right-click → "Customize folder..." with colour, theme icon or image, stored in
>   `Config::folder_looks`.
> - Phase 3c, `.directory` part: `.directory` `Icon=` is read.
>
> Stage 2, the theme browsers (Phase 3 and 3b), is not built yet. The rest of this
> document is the original research and describes `master` as of `da30c9d`.

For folder icon sets, folder colour variants and folder-name mappings, see the companion
survey `docs/folder-icon-libraries.md`.
Claims are tagged **[V]** if they were checked against source code (this repo, the pinned
libcosmic `d9431dc`, upstream pop-os repos) or by calling a live endpoint, and **[I]**
if they are inferred. Line numbers refer to `master` at `da30c9d`.

---

## 0. Read this first: on macOS the icon theme setting does nothing today

**[V]** libcosmic resolves every `widget::icon::from_name(..)` through `Named::path()`
(`src/widget/icon/named.rs` in libcosmic). That function has two versions:

```rust
#[cfg(all(unix, not(target_os = "macos")))]
pub fn path(self) -> Option<PathBuf> { /* freedesktop_icons::lookup(..).with_theme(..) */ }

#[cfg(any(not(unix), target_os = "macos"))]
pub fn path(self) -> Option<PathBuf> {
    //TODO: implement icon lookup for Windows
    None
}
```

When `path()` returns `None`, `handle()` falls back to `bundle::get(&name)`. That is a
`phf` map of about 677 Cosmic SVGs that libcosmic's `build.rs` compiles into the binary on
macOS and Windows (`cosmic-icons/{freedesktop,extra}/scalable`). On a miss it returns an
**empty SVG**, which draws nothing. The `cosmic-freedesktop-icons` dependency is gated to
`cfg(all(unix, not(target_os = "macos")))`, so on macOS the crate is not even compiled.
The macOS split came in with upstream commit `46d9f0c` (2026-04-14, "widget/icon: Bundle
icons on macOS, not just Windows").

What follows from that:

- `cosmic::icon_theme::set_default(..)` and the `CosmicTk` `icon_theme` key have **no
  effect on macOS**. Only the bundled Cosmic set is ever drawn.
- `IconFallback::Default` (trim the name at each `-`) and `IconFallback::Names` are
  ignored on macOS. `src/mime_icon.rs` relies on `IconFallback::Names`, so a MIME type
  whose first icon name is not bundled draws blank.
- `scripts/macos-bundle.sh` copies `share/icons/Cosmic` into the `.app`, and
  `src/launch_macos.rs:104-113` puts `Contents/Resources/share` on `XDG_DATA_DIRS`.
  **libcosmic never reads that icon copy.** Only the MIME database half of the copy is
  used (by `xdg-mime`). `docs/macos-porting-notes.md` ("Making the bundle
  self-contained"), `src/launch_macos.rs:98` and `scripts/test-macos-bundle.sh:113-129`
  all claim the theme is found at runtime. The code says otherwise. **[I]** A cheap way to
  confirm: rename the bundled `Cosmic` directory and check that nothing changes.

So **phase 1 below is a prerequisite**. Without it a theme picker would have nothing to
switch.

---

## 1. How icons work in this app today

### 1.1 File, folder and MIME icons (the file grid and list)

| What | Where |
|---|---|
| Each `Item` holds three handles built **at scan time**: `icon_handle_grid`, `icon_handle_list`, `icon_handle_list_condensed` | `src/tab.rs:2632-2659` (fields at `2640-2642`) |
| Views only wrap the handle they are given: `widget::icon::icon(item.icon_handle_grid.clone()).size(..)` | grid `tab.rs:6452`, `6613`; list `6842`, `6859`, `6885`, `6948`, `6965`, `6991`; preview pane `2766-2795` |
| Handles are built in `item_from_entry` (`806-863`), `item_from_gvfs_info` (`693-737`, Linux only), `item_from_denied_entry` (`921-944`), `item_from_trash_entry` (`965-1011`) and `scan_desktop` (`1412-1500`) | `src/tab.rs` |
| `folder_icon` / `folder_icon_symbolic`: `from_name(SPECIAL_DIRS.get(path).map_or("folder", ..)).prefer_svg(true).size(n).handle()` | `tab.rs:284-298` |
| `SPECIAL_DIRS` maps XDG user dirs to `folder-documents`, `folder-download`, `folder-music`, `folder-pictures`, `folder-publicshare`, `folder-templates`, `folder-videos`, `user-desktop`, `user-home` | `tab.rs:123-153` |
| `.desktop` icons: an absolute path goes to `from_path`, anything else to `from_name` | `tab.rs:586-616` |
| Trash: `user-trash` / `user-trash-full` and their `-symbolic` forms | `src/trash.rs:60-78` |
| Denied entries: `changes-prevent-symbolic` | `tab.rs:958-962` |

**MIME icon cache: `src/mime_icon.rs`**

- `MimeIconKey { mime, size }` (`:12-16`). **There is no theme and no scale in the key.**
- `get()` (`:32-52`) asks shared-mime-info for the icon names, builds
  `from_name(first).prefer_svg(true).size(size)` with the rest as
  `IconFallback::Names`, resolves it to a `Handle` immediately and caches it for the life
  of the process.
- `MIME_ICON_CACHE: LazyLock<Mutex<MimeIconCache>>` (`:54-55`) has no clear method. The
  same mutex also guards `shared_mime_info` (used by `mime_for_path` and `mime_app.rs:339`),
  so a reset must clear `cache` only.
- `FALLBACK_MIME_ICON = "text-x-generic"` is effectively dead code: `xdg-mime`'s
  `lookup_icon_names` never returns an empty list.

**Sizes.** `src/config.rs:22-33`: list 32, condensed 48, grid 64, scaled by `IconSizes`
percentages. `.size(n)` on a `Named` is only a hint for choosing a size directory; the
drawn size comes from `icon::icon(h).size(..)`. Zoom (`app.rs:5103-5118`) does not rescan.

**Symbolic vs full colour.** `Named::new` sets `symbolic = name.ends_with("-symbolic")`,
and `Handle.symbolic` makes libcosmic recolour the SVG to the theme's foreground. File
icons are full colour; UI icons are symbolic.

### 1.2 UI icons (all `icon::from_name`, resolved where they are used)

- **Sidebar** (`app.rs` `update_nav_model`, `1874-1960`):
  `document-open-recent-symbolic`, `folder_icon_symbolic(..)`, `text-x-generic-symbolic`,
  `Trash::icon_symbolic`, `network-workgroup-symbolic`, mounter icons, and eject
  `media-eject-symbolic` (`2666`). The file dialog builds its own copy
  (`dialog.rs:873-915`, `1262-1265`).
- **Header and toolbar** (`src/menu.rs`): `view-grid-symbolic`, `view-list-symbolic`
  (`441-443`), `view-sort-{ascending,descending}-symbolic` (`469-472`, `70-75`),
  `view-more-symbolic` (`517`). Search uses `system-search-symbolic` (`app.rs:6707, 6725`).
- **Operations**: `media-playback-{start,pause}-symbolic`, `window-close-symbolic`
  (`app.rs:2183-2205`, `6639-6657`).
- **Dialogs**: `dialog-error` at 64 px (`app.rs:5892` and others), `dialog-question`
  (`dialog.rs:1181`), `checkbox-checked-symbolic` (`app.rs:6237`).
- **Tab chrome** (`tab.rs`): `go-{previous,next}-symbolic`, `view-fullscreen-symbolic`,
  `edit-copy-symbolic`, `pan-{up,down}-symbolic`, `edit-symbolic`,
  `window-close-symbolic`, `folder-symbolic`, and `text-x-generic` for drag previews
  (`2801-2812`, `3021`, `5878-6174`, `6296-6305`, `7300`).
- **Open-with app icons**: `MimeApp.icon` is a `OnceLock<icon::Handle>` per app
  (`src/mime_app.rs:192-227`).
- **libcosmic's own widgets** (header bar buttons, dropdown arrows, context drawer close)
  call `from_name` inside libcosmic, so only a fix inside libcosmic reaches them.

The app has no `include_bytes!` icons of its own. The only non-`from_name` handles are
`from_path` (absolute `.desktop` icons, gvfs) and thumbnails.

### 1.3 Thumbnails replace icons

**[V]** `Message::Thumbnail` (`tab.rs:5249-5288`) writes thumbnail images into all three
`icon_handle_*` fields of an item. On macOS, `quicklook_macos::owns_preview` (`96-107`)
claims every MIME type except `text/*`, `inode/*` and the built-in image types, so PDFs,
Office documents and video usually show a Quick Look image, not a themed icon.

What this means for the feature:

- A theme switch must not overwrite thumbnails. The simplest correct approach is a full
  rescan of every tab (`rescan_tab`, `app.rs:1636-1679`, which ends in `set_items`). That
  rebuilds the handles and re-requests thumbnails, which come from the disk cache and are
  cheap.
- Set expectations: on macOS an icon theme visibly changes **folders, text and source
  files, generic files, the sidebar and the toolbar**. Most rich documents keep their
  Quick Look previews.

### 1.4 How libcosmic chooses the theme (Linux behaviour, and macOS after phase 1)

- `cosmic::icon_theme` (`src/icon_theme.rs`): `COSMIC = "Cosmic"`, a global
  `DEFAULT: Mutex<Cow<str>>`, and `default()` / `set_default(name)`.
- Toolkit config `CosmicTk` (`src/config/mod.rs`): cosmic-config id
  `com.system76.CosmicTk`, version 1, field `icon_theme: String` with default `"Cosmic"`.
  `CosmicTk::config()` returns a writable handle. On macOS cosmic-config stores it at
  `~/Library/Application Support/cosmic/com.system76.CosmicTk/v1/icon_theme`.
- Startup (`src/app/mod.rs:39-48`): if the app passed `Settings::default_icon_theme(..)`,
  that name wins and `core.icon_theme_override = true`. Otherwise `CosmicTk.icon_theme` is
  used.
- Live switching (`src/app/cosmic.rs:600-619`, `1337-1346`): libcosmic always watches
  `CosmicTk`. On a change it calls `icon_theme::set_default(config.icon_theme)` **unless**
  `icon_theme_override` is set. That field is `pub(super)`, so an app can only set it at
  startup.
- Lookup order: the chosen theme, then `Cosmic`. Inside each one,
  `cosmic-freedesktop-icons` follows the `index.theme` `Inherits=` chain. After that come
  the fallback names.
- `cosmic-freedesktop-icons` (pop-os/freedesktop-icons, rev `ab4c57b`):
  - `BASE_PATHS` (`$XDG_DATA_DIRS/{icons,pixmaps}`, `$XDG_DATA_HOME/{icons,pixmaps}`,
    `~/.icons`) and `THEMES` are both `LazyLock`s, filled on first use.
  - **A theme installed after the first icon lookup is invisible until the app
    restarts.**
  - **A base directory that did not exist at first use is never scanned.**
  - The resolved-path `CACHE` is keyed by theme, so switching between themes that were
    already installed is safe.
- **This app** never calls `default_icon_theme` or `set_default` (`src/lib.rs:229-232`,
  `115-119`). On Linux it already follows `CosmicTk.icon_theme` live, except for the
  caches in §1.1 and §1.2, which keep showing the old theme until a rescan.

---

## 2. How System76 does it (the model to follow)

**[V]** cosmic-settings, `src/pages/desktop/appearance/icon_themes.rs` and `drawer.rs`
("Icons and toolkit theming"):

- **Discovery (`fetch()`)**:
  - Scans `$XDG_DATA_DIRS/icons` and `$XDG_DATA_HOME/icons` (default
    `~/.local/share/icons`). It does not look in `~/.icons`.
  - Every directory with an `index.theme` is a candidate. `Hidden=true` skips it. The
    first `Name=` is the display name, and `Inherits=` gives the parent themes.
  - Directories with no `Name=` are dropped. There is no explicit filter for cursor-only
    themes.
- **Identity**: the theme id is the **directory name**, not `Name=`. Example: directory
  `Cosmic`, `Name=COSMIC`.
- **Preview**: six icons in two rows of three at 32 px, trying sizes 32, 48 and 64 with a
  `-symbolic` fallback: `folder`, `user-home`, `text-x-generic`, `image-x-generic`,
  `audio-x-generic`, `video-x-generic`. A resolved path counts only if it lives in the
  theme itself or one of its `Inherits=` parents. Upstream resolves previews by swapping
  the global `set_default` temporarily, which is racy. Don't copy that; see §4.3.
- **Apply**: `CosmicTk::config()?.set::<String>("icon_theme", id)`. That is all.
  libcosmic's watcher switches every running COSMIC app.
  cosmic-settings-daemon also mirrors the value into
  `gsettings org.gnome.desktop.interface icon-theme`, which does not matter on macOS.
- **There is no catalog or store.**
  - cosmic-themes.org hosts colour themes (`.ron`) only. Its JSON API is
    `GET https://cosmic-themes.org/api/themes/?order=..&search=..&limit=..&offset=..`.
  - COSMIC Store lists only AppStream `DesktopApplication` and `Addon` components, never
    icon themes.
  - The System76 approach is simply: get a freedesktop icon theme into an XDG `icons/`
    directory, then pick it from the list.

Related prior work in this fork:

- Branch `feat/theme-picker` (not merged) extends `AppTheme` with a bundled colour-theme
  catalog (`src/theme_catalog.rs`, `res/themes/cosmic/*.ron`, a `BUILT_IN_APP_THEMES`
  offset for dropdown rows).
- Branch `docs/cosmic-theme-selection` has `docs/cosmic-theme-selection.md`. It covers
  cosmic-themes.org and cosmic-settings' theme handling.
- The icon feature edits the same Settings section and `Config`. Coordinate or rebase
  rather than diverge.

---

## 3. The existing app-theme setting (the pattern to copy)

| Step | Where |
|---|---|
| Model: `AppTheme { Dark, Light, System }` in `Config.app_theme`. `Config` derives `CosmicConfigEntry`, which generates `set_app_theme(&handler, v)` | `src/config.rs:35-58`, `199-217`, `264` |
| Load: `Config::load()` with cosmic-config id `App::APP_ID`, `CONFIG_VERSION = 1`. External edits arrive through `Config::subscription()` → `Message::Config` | `config.rs:220-246`; `app.rs:6948-6958`, `3067-3077` |
| UI: `fn settings()` is the "Appearance" section, a `widget::dropdown(&self.app_themes, ..)` whose labels are built in `init` | `app.rs:2322-2347`, `2496`, `2556`; opened through `ContextPage::Settings` (`app.rs:322`, `5738-5742`) |
| Message: `Message::AppTheme(AppTheme)` → `config_set!(app_theme, ..)` (macro persists through cosmic-config) → `self.update_config()` | `app.rs:396`, `3043-3046`, macro `2966-2990` |
| Apply: `update_config()` rebuilds key binds, `update_nav_model()`, `cosmic::command::set_theme(..)`, and sends `tab::Message::Config` to every tab | `app.rs:1817-1832` |
| Strings | `i18n/en/cosmic_files.ftl` (`match-desktop`, `dark`, `light`, around lines 320-329) |

`tab::Message::Config` (`tab.rs:4286-4311`) does **not** rebuild item icons. That is the
main difference from the colour theme: the icon feature needs an explicit rescan.

---

## 4. Recommended implementation

### Phase 1: make theme lookup work on macOS (prerequisite)

**Option A, recommended: patch libcosmic.**

- Patch libcosmic in a fork and point this repo at it with the `[patch]` block that is
  already stubbed (commented out) at `Cargo.toml:210-216`.
- The patch is small **[I]**:
  - Change `#[cfg(all(unix, not(target_os = "macos")))]` on `Named::path()` to
    `#[cfg(unix)]`, and drop the macOS half of the `None` stub.
  - Gate the `cosmic-freedesktop-icons` dependency on `cfg(unix)` in libcosmic's
    `Cargo.toml`. The crate depends only on `xdg = "3.0"`, which builds on macOS; this
    repo already uses `xdg` 3.0 behind the `desktop` feature.
  - Leave `build.rs` and `bundle.rs` alone. `handle()` already falls back to the
    compiled-in bundle when `path()` returns `None`, so the bundle becomes the last-resort
    fallback, not the only source.
- What it buys:
  - Every `from_name` call, libcosmic's own widgets included, follows the theme.
  - The fallback chain works on macOS, which also fixes blank MIME icons.
  - The bundled `share/icons/Cosmic` copy is finally used, and the porting notes become
    true.
- What it costs: a libcosmic fork to rebase whenever `Cargo.lock` moves.
- **Creating a GitHub fork of libcosmic is outward-facing. Confirm with the maintainer
  first.** Per `AGENTS.md`, nothing goes to `pop-os` upstream: no PRs or issues against
  libcosmic either.

**Option B: resolve icons in the app.**

- Add `cosmic-freedesktop-icons` (same git source and rev as libcosmic uses on Linux) as
  a direct macOS dependency.
- Add `src/icon.rs` with a helper `named(name) -> Named`/`Handle`. It does the
  freedesktop lookup with the current theme and returns `icon::from_path`. On a miss it
  falls back to `from_name(..)`, which reaches the bundle.
- Route every call site in §1.1 and §1.2 through the helper (about 60).
- libcosmic's own widgets stay on the bundled Cosmic icons. Expect a lot of churn in
  files we share with upstream.

Choose A unless a libcosmic fork is ruled out.

Either way:

- **Create `~/.local/share/icons` early**, in `launch_macos::prepare()` before the first
  icon lookup. `BASE_PATHS` drops directories that do not exist at first use, so without
  this the first downloaded theme stays invisible even after a restart.
- **Add a test** that resolves one Cosmic icon from the bundled `share/icons/Cosmic` path
  on macOS.
- **Update `docs/macos-porting-notes.md`**, which currently describes this wrongly (see
  §0).

### Phase 2: icon theme setting in Settings → Appearance

1. **Storage: write `CosmicTk.icon_theme`, the System76 key.** Do not add a field to the
   app's own `Config`.
   - libcosmic already watches that key and calls `set_default`, so the switch is live.
   - The app does not set `icon_theme_override`, so a field of its own would be
     overwritten by the watcher anyway (§1.4).
   - Write with `cosmic::config::CosmicTk::config()?.set("icon_theme", id)`.
2. **Discovery: a `src/icon_theme_catalog.rs` that mirrors cosmic-settings `fetch()`
   (§2).**
   - Scan `XDG_DATA_DIRS` (this includes the `.app`'s `Contents/Resources/share` via
     `launch_macos`) and `XDG_DATA_HOME`.
   - Parse `index.theme` for `Name=`, `Hidden=` and `Inherits=`. Use the directory name
     as the id.
   - Scan on opening Settings, not at startup.
   - Unit-test it against a temporary directory tree.
3. **UI**:
   - Add an "Icons" row to `fn settings()` (`app.rs:2322`): a dropdown of installed themes
     plus a "Browse icon themes…" button that opens the browser (phase 3).
   - Add a small preview strip using the six cosmic-settings sample names. Resolve them
     with an explicit `freedesktop_icons::lookup(name).with_theme(id)`, not by swapping
     the global default.
   - Add strings to `i18n/en/cosmic_files.ftl`.
4. **Invalidation**: add `Message::IconThemeChanged(String)`.
   - Add an app-side subscription to `CosmicTk` (`cosmic_config::config_subscription`)
     so changes from outside the app also arrive. The handler must:
     - **call `cosmic::icon_theme::set_default(new)` first**, because the app's
       subscription and libcosmic's can fire in either order;
     - clear `MIME_ICON_CACHE.cache` (add a `clear()` that leaves `shared_mime_info`
       alone);
     - reset `MimeApp` icons (`Message::ReloadMimeAppCache`, `app.rs:4414`);
     - rescan every tab (`rescan_tab`), including desktop, trash and recents tabs;
     - call `update_nav_model()`.
   - Open file dialogs build their own sidebar. Refresh it or accept that it updates
     when next opened.
   - Linux benefits too: today a `CosmicTk` change leaves cached icons stale until a
     rescan.

### Phase 3: icon theme browser

**Source: a curated catalog stored in this fork.**

- Store the catalog as `res/icon-themes/catalog.json`.
- Ship a copy in the bundle and refresh it at runtime from
  `https://raw.githubusercontent.com/shythulu/Macosmetic/master/res/icon-themes/catalog.json`.
- Do **not** browse GitHub live. Unauthenticated REST is limited to 60 requests/hour per
  IP. Raw and codeload downloads are not subject to that limit.
- This keeps the System76 shape (themes are plain freedesktop directories under XDG
  `icons/`) and adds only a list of where to get them.

Suggested entry:

```jsonc
{
  "id": "Papirus",                       // directory name = CosmicTk icon_theme value
  "name": "Papirus",
  "repo": "https://github.com/PapirusDevelopmentTeam/papirus-icon-theme",
  "license": "GPL-3.0-only",
  "version": "20260801",                 // pinned tag or commit
  "archive_url": "https://codeload.github.com/.../tar.gz/refs/tags/20260801",
  "sha256": "…",
  "size_bytes": 34603008,
  "subdirs": ["Papirus", "Papirus-Dark", "Papirus-Light"],  // what to install from the archive
  "requires": [],                        // parent themes from Inherits= that must also be installed
  "preview": "https://…/preview.png",     // optional; otherwise render the six sample icons after install
  "symbolic_coverage": "good",
  "macos_notes": "223 case-only path collisions; see install rules"
}
```

**Install rules.** `tar` 0.4, `flate2` and `sha2` are already dependencies.

- Download to a temporary file and check `sha256` before extracting anything.
- Extract only the listed `subdirs` into a staging directory under
  `~/.local/share/icons`, then rename into place.
- Treat archives as untrusted:
  - reject absolute paths and `..` components;
  - reject symlinks whose target resolves outside the theme directory.
- **Keep symlinks.** Every theme except Adwaita relies on thousands of them.
- **Never write themes into the `.app`.** A dangling symlink there breaks
  `codesign --verify` (porting notes).
- **Handle case collisions.** APFS is case-insensitive by default, and Papirus has 223
  paths that differ only by case, Numix Circle 251, Kora 99, the vinceliuice themes about
  100, and Pop 8. Detect a collision, keep the first entry and log the rest. Do not fail
  the whole install.
- Install `requires` first. hicolor, Adwaita and breeze do not exist on a Mac, but the
  Cosmic fallback at the end of libcosmic's chain covers anything still missing.
- Copy the theme's license file next to it and show the license in the browser. GPL-3
  redistribution needs a source link (the pinned upstream tag is enough). CC-BY-SA
  themes (Cosmic, Pop) need attribution.
- **After installing a new theme, prompt a restart** (`THEMES` is a `LazyLock`, §1.4).
  Switching between themes that are already installed stays live. Removing the restart
  would need a patch to `cosmic-freedesktop-icons` as well; leave that for later.
- An HTTP client is **not** a dependency yet. Either add a small one (`ureq` with rustls)
  or shell out to `/usr/bin/curl` through `tokio::process`, which is already enabled.
  Run downloads as a background task with progress messages, like the existing
  operations.

**Browser UI.** A new `ContextPage::IconThemes`, next to `ContextPage::Settings`.

- One card per catalog entry: name, preview, license, download size, symbolic coverage.
- Buttons: Install / Apply / Remove, plus a progress bar.
- "Installed" also includes themes found on disk that are not in the catalog.
- Remove only directories this feature installed: keep a marker file in each theme
  directory.

**Seed catalog.** Checked against each repo's tree; sizes are measured `.tar.gz` downloads.

| Theme | Repo | License | Ready to use without building | Notes |
|---|---|---|---|---|
| Cosmic | pop-os/cosmic-icons | CC-BY-SA-4.0 | No (`freedesktop/` + `extra/` must be assembled) | Already bundled in the `.app`. List it as built-in, not as a download |
| Adwaita | GNOME/adwaita-icon-theme (tarballs on download.gnome.org) | LGPL-3 / CC-BY-SA-3.0 | Mostly | **Cleanest first entry**: 1 symlink, 0 case collisions, mostly symbolic |
| MoreWaita | somepaulo/MoreWaita | GPL-3.0 | Yes | 11 MB. Needs Adwaita (`requires`) |
| Papirus (+Dark/Light) | PapirusDevelopmentTeam/papirus-icon-theme | GPL-3.0 | Yes | 33 MB, more than 35k symlinks, 223 case collisions. Inherits breeze, which is missing on a Mac |
| Kora | bikass/kora | GPL-3.0 | Yes | 6.1 MB, 99 collisions |
| Numix / Numix Circle | numixproject/* | GPL-3.0 | Yes | Circle needs Numix. Weak symbolic coverage |
| **WhiteSur** (macOS look) | vinceliuice/WhiteSur-icon-theme | GPL-3.0 | **No**: `install.sh` uses GNU `sed -i` and generates `index.theme` | The obvious headline theme for a macOS port. See below |
| Tela, Colloid, Fluent, Qogir, McMojave-circle | vinceliuice/* | GPL-3.0 | No (`install.sh`) | Same build problem as WhiteSur |

For the `install.sh` themes: run `install.sh` in this fork's CI on a Linux runner. Then
resolve the case collisions, repack as `.tar.gz` (not zip, which loses symlinks), attach
the result to a release of `shythulu/Macosmetic`, and point `archive_url` there. That is
a release-packaging task, separate from the app code. Ship the build-free themes first.

### Phase 3b: KDE Store browser (store.kde.org, formerly kde-look.org)

KDE already has a working in-app browser for icon themes: Plasma's "Get New Icons…",
built on KNewStuff. It reads the **OCS API** of store.kde.org, which runs on Pling, the
same backend as gnome-look.org. COSMIC has no equivalent (§2), so this is the established
precedent for browsing themes inside an app. It is a second source next to the curated
catalog, not a replacement for it.

**What the API gives.** Checked live on 2026-10-07 without authentication. **[V]**

- Categories: `https://api.kde-look.org/ocs/v1/content/categories?format=json` lists 167.
  The icon ones are `132` "KDE Icon Theme" (shown as "Full Icon Themes",
  `xdg_type: icons`), `107` cursors and `113` emoticons.
- **There is no folder-only category.** Folder packs are filed under 132, and only 13
  items carry a `folders` tag. To browse folder packs, filter 132 with `search=folder`
  (391 hits). After downloading, detect folder-only packs by checking that they contain
  only `places/` (§1.2 of `docs/folder-icon-libraries.md`).
- Listing: `…/ocs/v1/content/data?categories=132&sortmode=down|high|new&page=N&search=…&format=json`
  returns 10 items per page. Detail: `…/content/data/<id>?format=json`.
- Fields per item:
  - `name`, `summary`, `description`, `version`, `changed`, `downloads`, `score`
  - `previewpic1..6`, `smallpreviewpic1..6`, `detailpage`
  - `downloadlink{n}`, `downloadname{n}`, `downloadsize{n}` (KB), `downloadmd5sum{n}`,
    `downloadtags{n}`
  - `ghns_excluded` (items the publisher hid from in-app browsers)
- **The licence is in `tags`, not in the empty `license` field.**
  - 98 of the 100 most recently updated icon themes had a licence tag: `gplv3` 74,
    `agplv3` 13, `cc-by-sa` 9, `lgplv3` 1, `gplv2-later` 1.
  - The top-rated ones (Papirus, Tela, Kora, Candy, Reversal, Fluent, BeautyLine) are
    tagged `gplv3`.
  - This corrects an earlier note that the API gives no licence.
- Of the 50 top-rated items: 42 have an MD5, and 36 offer more than one file (variants).
  Archive types: `tar.xz` 29, `tar.gz` 15, `zip` 4, `tar.bz2` 1, and one is a bare
  **`.sh` installer**.

**Rules for the KDE Store source:**

- **Only list items with an OSI or CC-BY(-SA) licence tag.** Show the licence on the card.
  Hide `ghns_excluded` items, which matches KNewStuff.
- **Never execute anything downloaded.**
  - Accept only `tar.xz`, `tar.gz`, `tar.bz2` and `zip`.
  - Skip `.sh` and every other file type.
  - Extract with the same untrusted-archive rules as the curated catalog (§ Install
    rules): no absolute paths or `..`, no symlinks escaping the theme, handle case
    collisions.
- **Fetch a fresh link at install time.**
  - Download links are signed per requesting IP and expire after 48 h.
  - Re-fetch the item detail right before downloading; never cache `downloadlink`.
  - Verify `downloadmd5sum` when it is present. MD5 only detects a corrupted download, not
    tampering.
- **Archive layout varies** (one theme at the root, several variant directories, or a
  wrapper directory).
  - Install every top-level directory that has an `index.theme`.
  - A pack with **no** `index.theme` (a loose folder-icon pack) is not a theme. It can be
    offered as artwork for per-folder looks (§6 `FolderLook::Image`), or skipped in v1.
- **`zip` packs lose symlinks.** Expect duplicated files or missing aliases. Prefer tar
  variants when an item offers several.
- **Decompression:**
  - `flate2` and `tar` are already dependencies.
  - `tar.xz` needs an xz decoder. **[I]** The `lzma-rust2` feature this repo builds with
    may cover it.
  - The repo already depends on `zip` 8.
- **Cache previews and listings.** Stay gentle with the API: no prefetching of all pages.
  - API terms for third-party clients were not found. **[I]**
  - KNewStuff's provider list is `https://autoconfig.kde.org/ocs/providers.xml`, which
    shows these endpoints are meant for client apps. **[I]**
- **Prior art:** `debasish-patra-1987/linuxthemestore` (Rust, GPL-3, on Flathub) browses
  the same API.

#### Why Plasma's "Get New…" feels slow, and how to avoid it

Plasma System Settings' browser (KNewStuff) is known to be laggy and unreliable. I did not
profile KNewStuff itself. The causes below are what **any** client of this API runs into,
measured from this Mac on 2026-10-07, plus what KDE says about its own code.

**Measured: [V]**

| Request | Server wait | Total | Size |
|---|---|---|---|
| `providers.xml` (443 B) | 0.91 s | 0.91 s | 443 B |
| categories | 0.84 s | 0.84 s | 18 KB |
| one item's detail | 0.87 s | 0.87 s | 4 KB |
| listing, 10 items (default page) | 0.79–0.83 s | 1.26–1.45 s | 100–140 KB |
| listing, `pagesize=100` | 1.28 s | 2.32 s | 500 KB |
| one preview image (`previewpic1`) | – | 1.25–1.39 s | **145 KB** (770×540 PNG) |

- **Each API call costs about 0.8–1.3 s** of server time, however small the response. A
  chain of dependent calls adds up: providers → categories → page → detail → fresh
  download link is 4–5 s before a download even starts.
- **`smallpreviewpic` is the same 770×540 image as `previewpic`.** The API offers no real
  thumbnail, so one page of 10 cards pulls about 1.4 MB of images at about 1.3 s each.
  - The Pling image CDN also serves a resized copy when the size segment of the path is
    changed: `…/cache/100x100-1/…` returned the same image in **14 KB**.
  - **[I]** That URL pattern is undocumented, so fall back to the full image if it fails.
- **`pagesize` works**, up to at least 100. An earlier note said it was ignored; that was
  wrong. The default is 10, so a client that keeps the default makes 10× the round trips.
- **Listing payloads carry full descriptions and changelogs.** Most of each 100–140 KB
  page is text a card never shows.

**KDE's own assessment** (Plasma 2025 sprint, KNewStuff topic): "Our KNS code feels very
fragile". There is "no checking for structure or file types; uploads can be broken … or
confusing (e.g. when people upload multiple versions of things to one entry)".

**User reports:**

- "Get New…" opens a blank window, or doesn't open at all, until the `~/.cache/knewstuff*`
  and QML caches are cleared.
- Installs fail with "no download URL". **[I]** That fits download links that expire after
  48 h being served from a stale listing.

**Design rules for our browser:**

1. **Never wait on Pling to show the browser.**
   - A scheduled CI job in this fork crawls category 132 once a day: about 13 calls at
     `pagesize=100`, roughly 30 s. It writes a **trimmed index** to
     `res/icon-themes/kde-store-index.json`, keeping only id, name, summary, licence tag,
     downloads, score, `changed`, archive names and types, and the small preview URL.
   - It drops items without an OSI or CC licence tag and `ghns_excluded` items.
   - The app loads that file from `raw.githubusercontent.com`, a fast CDN, in one request,
     and keeps the last copy on disk.
   - Browsing, sorting and **search all run locally on that index**, so they are instant.
2. **Pling is called only at install time**: one detail request for a fresh download link,
   then the download. Show a progress state for that wait.
3. **No network or decoding on the UI path.**
   - Every fetch is an iced `Task` or subscription that reports back through messages.
     Cards render placeholders immediately.
   - Decode and downscale images on a background thread, once.
4. **Thumbnails:**
   - Request the small CDN copy first (`100x100` or `200x200`), falling back to
     `previewpic1`.
   - Load only the cards that are visible.
   - Allow at most 4–6 concurrent requests.
   - Keep an on-disk cache keyed by URL under `~/Library/Caches/<app id>/kde-store/`.
5. **Fail fast and visibly.**
   - Give each request a timeout (about 10 s).
   - Retry once with backoff.
   - Show per-card error states. A failed preview must never block the grid.
6. **Validate before installing**, because the store does no structure checks: archive type
   allow-list, an `index.theme` must be present (§3b), and the extraction rules.
   - A broken upload fails with a clear message and leaves nothing half-installed (stage
     the install, then rename it into place).
7. **No long-lived state to corrupt.**
   - The index, thumbnails and install markers are plain files.
   - Deleting the cache directory is always safe and is rebuilt on demand.

The daily index job is a CI task in this fork, like the release builds of the
`install.sh` themes (Phase 3). It reads public data only and needs no credentials.

### Phase 3c: KDE-specific theme features

KDE and GNOME themes share one format (§1.4 and §2). Two KDE extras are worth supporting:

- **`FollowsColorScheme=true` recolouring.**
  - Breeze, and many themes derived from it (Papirus-Colors, Gruvbox Plus "plasma"),
    paint folders with CSS classes inside the SVG, e.g.:
    ```xml
    <style id="current-color-scheme">.ColorScheme-Accent { color:#3daee9; } …</style>
    ```
    Plasma rewrites that block to the user's colours before drawing.
  - **[V]** Breeze `places/64/folder.svg` uses `ColorScheme-Accent` for the folder body.
    Its `places/16` folders use only `ColorScheme-Text`.
  - **[I]** Do the same here:
    - If the theme's `index.theme` has `FollowsColorScheme=true`, read the SVG, replace
      the `current-color-scheme` style with COSMIC's accent and text colours, and load the
      result with `icon::from_svg_bytes`.
    - Cache the result per (path, accent).
    - KDE folder themes then follow the COSMIC accent, and a `FolderLook::Colour` can
      recolour a single folder from the same SVG.
    - Without this, those icons draw in Breeze's default blue.
- **`.directory` `Icon=`, read-only.**
  - Honour existing `.directory` files from KDE, for example on folders copied from
    Linux or on shared drives.
  - `Icon=` is a themed name or a path; `./` means relative to the folder. There is also
    `EmptyIcon=`.
  - Treat it as a fallback after the app's own assignment (§6).
  - Do **not** use it as our store. Writing it puts files into the user's folders (§6.1).
  - An explicit "Save in folder (KDE compatible)" export could come later.

---

## 5. Gotchas checklist

- [ ] macOS: `Named::path()` returns `None` until phase 1 (§0).
- [ ] `MIME_ICON_CACHE` key has no theme. Clear `cache` only, never `shared_mime_info`.
- [ ] Item icon handles are baked at scan time. Rescan every tab on a theme change.
- [ ] Thumbnails overwrite `icon_handle_*`. A rescan is the safe way to refresh.
- [ ] `MimeApp.icon` is a `OnceLock` per app. Reload `MimeAppCache`.
- [ ] Call `icon_theme::set_default` in the app's own handler before invalidating, so
      the order of subscriptions doesn't matter.
- [ ] `icon_theme_override` can only be set at startup. Use `CosmicTk`, not
      `Settings::default_icon_theme`.
- [ ] `freedesktop-icons` `THEMES` and `BASE_PATHS` are fixed at first use. Create
      `~/.local/share/icons` early and prompt a restart after installing a new theme.
- [ ] Symlinks and case collisions on APFS. Never install into the `.app`.
- [ ] Hard-coded names (`changes-prevent-symbolic`, `dialog-error`, …) may be missing
      from third-party themes. The Cosmic fallback in libcosmic's chain covers them once
      phase 1 lands.
- [ ] No scale is ever passed to `Named` (porting notes §3.3). This is unrelated to
      themes, but HiDPI PNG themes will look soft until it is fixed.
- [ ] Do not open issues or PRs against `pop-os/*` for any of this (`AGENTS.md`).

---

## 6. Per-folder looks: where to store "this folder is red / uses `folder-git`"

The feature: the user picks a look for one particular folder (a colour, a themed icon name,
or their own image), and the app remembers it.

The look is applied where folder icons are built. The order, most specific first:

1. A per-folder assignment.
2. `SPECIAL_DIRS`.
3. A name-based rule (`docs/folder-icon-libraries.md` §4).
4. The theme's plain `folder`.

The open question is where the assignment lives.

### 6.1 Options compared

Checked on this Mac (macOS 26, APFS) by creating folders and inspecting them.

| | Finder custom icon (`NSWorkspace.setIcon`) | Custom extended attribute on the folder | `.directory` file in the folder (KDE style) | App-owned file (cosmic-config/RON or JSON) | SQLite |
|---|---|---|---|---|---|
| What gets written | **[V]** A hidden file `Icon\r` inside the folder holding a **~190 KB resource fork**, plus a `com.apple.FinderInfo` flag on the folder | **[V]** One xattr, e.g. `com.macosmetic.look = "folder-red"` | A visible-to-tools text file inside the folder | One file under `~/Library/Application Support/cosmic/…` | One database file |
| Follows the folder when it's renamed or moved | Yes | **[V]** Yes: survived `mv`, `cp -R` and `ditto` | Yes | No. It's keyed by path, so the app must update it on its own renames | No (same as JSON) |
| Touches the user's folder | Yes, and **`Icon\r` shows up in `git status`**, in Dropbox and in zip files | Changes ctime only. Invisible to git, but git doesn't keep it either | Yes, and it pollutes repos | No | No |
| Works on folders you can't write to (`/Applications`, shared or read-only volumes) | No | No | No | **Yes** | Yes |
| Works on exFAT, FAT, SMB | Partly (creates AppleDouble `._` files) | Partly (same) | Yes | Yes | Yes |
| Can store "follow the theme" (an icon *name* or colour) | **No.** It stores a bitmap, which stops matching once the theme or accent changes | Yes | Yes | Yes | Yes |
| Cost when listing a folder | Expensive to read back (resource fork or `NSWorkspace` call per item) | One `getxattr` per subfolder | One file open + read per subfolder | **A `HashMap` lookup, already in memory** | A query per item, or load it all, which makes it a JSON equivalent |
| Live sync between windows | No | No (re-read on rescan) | No | **Yes**: cosmic-config already watches its files | Needs its own change notification |
| Time to build | Medium (AppKit, icns rendering) | Small (needs the `xattr` crate or `libc`) | Small | **Smallest: a pattern this repo already has** | Larger (new C dependency, schema, migrations) |
| Visible to Finder and other apps | Yes | No | Dolphin only | No | No |

### 6.2 Recommendation: an app-owned map in cosmic-config, keyed by path

The precedent is already in the repo. `State.sort_names` (`src/config.rs:150-167`) stores
a per-folder sort order as an `FxOrderMap<String, …>` keyed by the normalized location
string. It is saved through cosmic-config and updated at `src/app.rs:4835-4865`.

Copy that shape:

```rust
// src/config.rs, next to `sort_names` in `State` (or its own CosmicConfigEntry)
pub folder_looks: FxOrderMap<String, FolderLook>,

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum FolderLook {
    /// Themed colour: resolved as `folder-<colour>-<xdg suffix>` → `folder-<colour>` → `folder`.
    Colour(String),
    /// A themed icon name, e.g. "folder-git", falling back to `folder`. `theme: Some(..)`
    /// pins it to an installed theme (e.g. a folder pack from the KDE Store) instead of
    /// the active one, so one folder can use a pack's icon while the rest follow the theme.
    Icon { theme: Option<String>, name: String },
    /// User image, copied into the app's data dir so it can't dangle.
    Image(PathBuf),
}
```

Why this one:

- **Fastest to build.** `CosmicConfigEntry` provides load, save and change notification.
  The settings handler and `config_set!` pattern already exist (§3).
- **Fast to read.** Load it once into memory. The scan code in `item_from_entry` looks the
  path up in a shared map, the same way it already consults `SPECIAL_DIRS`.
  - **[I]** It could be an `Arc` snapshot passed into the scan, or a
    `LazyLock<RwLock<…>>` like `SPECIAL_DIRS`.
  - **[I]** Thousands of entries make a RON file of a few hundred KB. That is fine to
    rewrite on each change.
- **Stores intent, not pixels.** `Colour` and `Icon` stay correct when the user switches
  icon theme or accent colour. A Finder custom icon would not.
- **Never writes into the user's folders.** No `Icon\r` in git repos, and it works on
  read-only and network volumes.
- **Syncs live across windows.** cosmic-config's watcher handles this for free.

Rules that differ from `sort_names`:

- **No eviction.** `sort_names` drops entries above 999 (`app.rs:4851`). Looks are
  explicit user choices, so keep them all.
- **Follow in-app renames and moves.** Update the key when an app operation renames or
  moves a folder, including the paths of every entry under it. Renames done outside the
  app (Finder, the shell) orphan the entry. That is the same limit `sort_names` has today.
  - **[I]** A later improvement could also store the folder's file ID (`st_dev` +
    `st_ino`) and, when the path misses, try matching the ID during the scan.
  - **[I]** `st_dev` is not stable across mounts for external disks, so treat that match
    as best-effort.
- **Prune lazily.** Drop an entry when its path no longer exists, but only on local
  volumes. Never prune just because a network volume is unmounted.

Not recommended:

- **SQLite** solves problems this feature doesn't have: complex queries, very large data
  sets, several processes writing. It still has the same rename problem, and it adds a C
  dependency.
- **`.directory` files** and **Finder custom icons** both write into the user's folders.
  The Finder icon is also a frozen bitmap.

### 6.3 Optional macOS extras, after the core works

- **Read Finder's custom icon** for display. If a folder has an `Icon\r` file, show the
  icon the user already set in Finder. Use `NSWorkspace.icon(forFile:)` and only call it
  when the `FinderInfo` custom-icon flag is set, so normal folders pay nothing.
- **Finder tags as colours.** `com.apple.metadata:_kMDItemUserTags` (a binary plist on the
  item) holds Finder's colour tags. Showing a red-tagged folder as `folder-red` would match
  what Finder shows.
- **"Also apply in Finder"** as an explicit opt-in action that writes a Finder custom icon.
  Keep it separate from the app's own store, and warn that it adds an `Icon\r` file.
