# Installing icon themes from inside the app

Design and implementation spec. Written 2026-10-09. No code was changed.

Basis: branch `feat/icon-theme-catalog`, worktree `the `feat/icon-theme-catalog` worktree`, commit `493f2dd`, rebased on `master` `018a5ae`. It builds, and `cargo test --lib icon_theme_catalog` passes 4 of 4. This spec says what to keep on that branch, what to change, what is missing, and the order to finish it in.

Inputs read: the branch diff (`src/icon_theme_catalog.rs`, `src/icon_theme_gallery.rs`, `src/app.rs`, `scripts/icon-theme-catalog.py`, `res/icon-themes/*`), `docs/icon-theme-customization.md` Phases 3, 3b, 3c and §5, `docs/folder-icon-libraries.md`, `src/icon_themes.rs`, `src/archive.rs`, `Cargo.toml`, `Cargo.lock`, the `cosmic-freedesktop-icons` checkout at `ab4c57b`, the `shythulu/libcosmic` fork at `4561417`, and the web for the theme repos (section 3).

---

## 1. Recommendation

Finish the branch. Do not restart the design.

| Decision | Call | Reason |
|---|---|---|
| Source of themes | Curated catalog (already on the branch) plus "Install from file..." and drag-and-drop | The catalog covers 34 ready-to-use themes from 16 archives. File install covers everything else, including KDE Store downloads, with no network code. |
| `install.sh` themes (WhiteSur, Tela, Colloid, Fluent, Qogir) | Build in this fork's CI, attach to a release, add to `sources.json` as `url` entries | The script already supports release-asset URLs (Nordzy uses them). The app never runs a script. |
| KDE Store browser | Not now | Each OCS call costs about 1 s, uploads are unchecked, links expire in 48 h. File install gives the same result in two steps. Revisit only with the daily-index CI from Phase 3b. |
| Arbitrary URL field | No | Same coverage as file install, worse error surface. |
| Restart after install | Remove it | Fork `pop-os/freedesktop-icons`, add `reload_themes()`, pull it in through a `[patch]` like libcosmic. Until then the branch's "Restart to use" button stays. |
| Download client | Keep `curl` | It is on every Mac, the branch already drives it, and the fixes needed are four flags. `ureq` would add a TLS stack for no user-visible gain. |

What finishing buys the user: Settings, Icons, Browse..., installed themes on top, 34 more below with real previews, Install with a progress bar and Cancel, Remove on anything the app installed, Update when the catalog moves, drop a `.tar.gz` on the drawer to install anything else, and no restart.

---

## 2. What the branch already does

| Piece | Where | State |
|---|---|---|
| Catalog data: 16 pinned archives with sha256 and size, 34 themes with id, licence, homepage, archive index, path inside archive, `requires`, preview file names | `res/icon-themes/catalog.json`, 792 lines | Done. 15 codeload tag or commit URLs, 2 release assets. |
| Catalog source of truth, one entry per repo with ref and theme paths | `res/icon-themes/sources.json` | Done. |
| Generator: downloads each archive once to `~/Library/Caches/macosmetic-icon-themes`, hashes it, finds each theme's `Inherits=` and symlink targets to compute `requires`, pulls the 8 preview icons out of the archive, writes `catalog.json` and `previews/<id>/N.svg` | `scripts/icon-theme-catalog.py`, 302 lines | Done. Previews total 2.7 MB of SVG, compiled in with `rust_embed`. |
| Catalog loading and lookup: `catalog()`, `theme(id)`, `previews(theme)`, `install_plan(id, installed)` with dependencies first, `plan_size` | `src/icon_theme_catalog.rs:38-144` | Done, tested. |
| Install: staging `tempdir_in(icons_dir)` with prefix `.macosmetic-staging-`, per archive: download, `verify_sha256`, `extract` only the plan's targets, check `index.theme` exists, write marker, `rename` into place, skip if the target already exists | `:174-238` | Done. |
| Download with `curl --fail --location --silent --show-error --proto =https`, progress by polling the output file size every 200 ms | `:241-278` | Works. Needs hardening (4.2). |
| Extraction rules: rejects absolute paths and `..`, rejects hard links and special files, keeps relative symlinks that stay inside `~/.local/share/icons` (so Papirus-Dark can link into Papirus), refuses to write through a symlink, keeps the first of two case-colliding paths, copies an out-of-tree `index.theme` into the theme (Adwaita) | `:320-441`, tests `:511-589` | Done, tested. Needs caps (4.2). |
| Marker `.macosmetic-catalog` holding the archive sha256; `remove(id)` refuses without it | `:36`, `:457-471` | Done. The sha256 doubles as a version stamp (5.2). |
| Gallery: "Installed" section with Remove on catalog installs that nothing depends on, "Available to download" section with licence, download size, "also installs X", Source link, Install / progress bar / Retry, one install at a time | `src/icon_theme_gallery.rs` | Done. |
| App wiring: `IconThemeInstall`, `IconThemeInstallEvent`, `IconThemeRemove`, `IconThemeRemoved`, `Restart`; install runs on a `std::thread`, events come back through an mpsc channel and `Task::run` | `src/app.rs:3392-3471` | Done. |
| Strings | `i18n/en/cosmic_files.ftl:359-368` | Done for what exists. |
| `serde_json` dependency, `format_size` made `pub(crate)` | `Cargo.toml`, `src/tab.rs:372` | Done. |

---

## 3. Verified catalog

Checked 2026-10-09 on the web. "On branch" says whether `sources.json` already lists it. "Layout": **ready** means the archive holds directories with `index.theme` at the top level; **build** means an `install.sh` generates the theme. "Folder colours" is what the per-folder Customize page can list once the theme is installed.

All 16 repos exist and none is archived. No latest release on any of them has attached assets, so the archive URL is the tag tarball, `https://github.com/<owner>/<repo>/archive/refs/tags/<tag>.tar.gz`, or the same bytes from `codeload.github.com/<owner>/<repo>/tar.gz/<tag>`, which the branch uses.

| Theme | Repo | Licence | Archive URL pattern | Layout | Folder colours | On branch |
|---|---|---|---|---|---|---|
| Papirus, Papirus-Dark, Papirus-Light | github.com/PapirusDevelopmentTeam/papirus-icon-theme | GPL-3.0 | tag `20260801` tarball | ready, 3 dirs; `Inherits=breeze,hicolor` (Dark: `breeze-dark,hicolor`) | yes, 25 sets as `folder-<colour>*` | yes |
| kora, kora-pgrey | github.com/bikass/kora | GPL-3.0 | tag `v2.0.6` | ready, 2 dirs; `Inherits=breeze,hicolor` | no | yes |
| Numix-Circle | github.com/numixproject/numix-icon-theme-circle | GPL-3.0 | tag `26.02.21` | ready; `Inherits=Numix`, so needs numix-icon-theme `25.12.15` beside it | no | yes, with Numix |
| MoreWaita | github.com/somepaulo/MoreWaita | GPL-3.0 | tag `v50.1` | ready, `index.theme` at repo root; `Inherits=Adwaita,AdwaitaLegacy,hicolor` | no (extra names, not colours) | yes |
| Candy | github.com/EliverLara/candy-icons | GPL-3.0 | no tags, pin a commit | ready, root; `Inherits=breeze-dark,Adwaita,hicolor` | no | yes, commit `83512fb` |
| Adwaita | github.com/GNOME/adwaita-icon-theme; tarballs at download.gnome.org/sources/adwaita-icon-theme/51/ | LGPL-3.0 or CC-BY-SA-3.0 (dual, inferred from COPYING files) | GitHub tag `51.0` tarball, or `adwaita-icon-theme-51.0.tar.xz` | ready after assembling `Adwaita/*` plus the root `index.theme`, which the branch's `index_theme` field does | no | yes |
| WhiteSur | github.com/vinceliuice/WhiteSur-icon-theme | GPL-3.0 | tag `2026-09-10` | build: `install.sh` copies `src/`, sed-substitutes colours, writes `index.theme` from a template | 9 accents | no |
| Tela | github.com/vinceliuice/Tela-icon-theme | GPL-3.0 | tag `2026-07-07` | build, same pattern | 15 colour variant themes | no |
| Colloid | github.com/vinceliuice/Colloid-icon-theme | GPL-3.0 | tag `2026-08-10` | build | 9 | no |
| Fluent | github.com/vinceliuice/Fluent-icon-theme | GPL-3.0 | tag `2026-07-27` | build; variants are a shared base plus symlinks | 8 | no |
| Qogir | github.com/vinceliuice/Qogir-icon-theme | GPL-3.0 | tag `2025-02-15`; last push 2025-11, going stale | build | 3 | no |
| Reversal | github.com/yeyushengfan258/Reversal-icon-theme | GPL-3.0 | no tags, pin a commit | build | 12 | no |
| Tela-circle | github.com/vinceliuice/Tela-circle-icon-theme | none in repo | tag tarball | build | 15 | no, skip: no licence |
| BeautyLine | github.com/gvolpe/BeautyLine | none in repo; last push 2022-01 | tag `0.0.4` | ready | no | no, skip: no licence |
| Breeze | github.com/KDE/breeze-icons; download.kde.org/stable/frameworks/6.30/breeze-icons-6.30.0.tar.xz | LGPL-3.0-or-later | KDE tarball | build: CMake writes `index.theme` from `index.theme.in` | no (follows the KDE accent through CSS classes) | no |

The vinceliuice scripts use GNU `sed -i` without a suffix, so they fail on a stock Mac anyway. That settles the question of running them locally: the only options are a Rust port of copy, sed and overlay steps, or a CI build. Section 3.2 picks CI.

KDE Store, checked the same day: `https://api.kde-look.org/ocs/v1/content/data?categories=132&sortmode=high&format=json` answers with `downloadlink1`, `downloadmd5sum1`, `downloadsize1`, `previewpic1..6`, 1227 items, 10 per page, `pagesize` ignored on this call, no licence field, only tags such as `gplv3`. That matches the Phase 3b notes apart from `pagesize`, which the notes said worked. It confirms the "not now" call in section 1.

### 3.1 Gaps against the requested list

| Requested | On branch | Why not, and what to do |
|---|---|---|
| WhiteSur, Tela, Colloid, Fluent, Qogir, Reversal | No | `install.sh` themes. Build in CI (3.2). Qogir is going stale; include only if the build is free. |
| Tela-circle, BeautyLine | No | No licence file. Leave out. |
| Breeze | No | Needs CMake to produce `index.theme`. Two options: have the CI job build it like the vinceliuice themes, or skip it. Papirus, kora and Candy inherit breeze, but the Cosmic fallback at the end of libcosmic's chain covers the missing names, so skipping costs little. |
| Candy, Kora, Numix-Circle, Adwaita, MoreWaita, Papirus | Yes | Nothing to do. |

### 3.2 `install.sh` themes without running scripts

The vinceliuice scripts copy `src/`, run `sed -i` on hex colours per variant, write `index.theme` from a template, and link dark variants. None of that has to run on the user's Mac.

Add `.github/workflows/icon-theme-builds.yml` to this fork:

1. Check out the upstream repo at a pinned tag on `ubuntu-latest`.
2. Run `./install.sh -d out -a` (all colours) or a chosen subset.
3. `tar czf <Theme>.tar.gz -C out <Theme> <Theme>-dark ...`
4. Attach to a release `icon-themes-<date>` on `shythulu/Macosmetic` with the upstream tag in the release notes (GPL-3 source link).
5. Add a `sources.json` entry with `"url": "https://github.com/shythulu/Macosmetic/releases/download/..."`, `"license"`, `"themes": [{ "path": "WhiteSur" }, ...]`. The generator already treats `url` sources as archives with themes at the root, which is what step 3 produces.

Pick the variants to publish per theme. WhiteSur: `WhiteSur`, `WhiteSur-dark`. Tela: `Tela`, `Tela-dark`, plus the blue and grey accents. Fifteen Tela colours would be fifteen themes in the gallery; the catalog is a list of cards, so keep it to two or three per family. Users who want a specific accent can drop the archive from GitHub onto the drawer.

---

## 4. Review of the branch: keep and change

### 4.1 Keep as is

- The data model: `archives[]` plus `themes[]` with an archive index. One download serves Papirus, Papirus-Dark and Papirus-Light. Keep.
- `install_plan` and the `requires` computed from both `Inherits=` and symlink targets. That is the only correct way to install Papirus-Dark, which is mostly links into Papirus. Keep.
- `link_stays_inside` measured from the installed location. More precise than "inside the theme". Keep.
- Marker equals archive sha256. It gives update detection for free (5.2). Keep.
- One install at a time, enforced in the view by disabling the other Install buttons. Keep.
- Previews compiled in. 2.7 MB in the binary for 34 themes is acceptable for a desktop app and means the drawer works offline on first run. Keep. If the binary size matters later, rasterise to 32 px PNG strips in the generator (about 6 KB per theme).

### 4.2 Change

| Finding | Where | Change |
|---|---|---|
| `--proto =https` only covers the first URL. curl allows redirects to `http` unless `--proto-redir` is set. | `icon_theme_catalog.rs:248-249` | Add `--proto-redir =https --max-redirs 5`. |
| No size cap. A moved URL that serves a huge file fills the disk. | `:241` | Add `--max-filesize <archive.size * 1.25>`. curl aborts with exit 63. |
| No timeouts. A stalled connection hangs the install forever. | `:241` | Add `--connect-timeout 15 --speed-time 60 --speed-limit 1024`. |
| No cancel. The gallery shows a progress bar with no way out. | `:258-266`, gallery `:231-234` | Pass an `Arc<AtomicBool>` into `install`; the poll loop checks it and calls `child.kill()`. Add `InstallEvent::Cancelled(id)`, `Message::IconThemeInstallCancel(id)`, a Cancel button next to the bar. |
| No extraction caps. A crafted archive with millions of entries or a 10 GB file is accepted. | `extract`, `:320` | Add `Limits { max_bytes: 1_500_000_000, max_entries: 250_000 }`, count `entry.size()` and entries, fail with `InvalidData`. Papirus extracts to about 300 MB with 42k symlinks, so this leaves room. |
| Directory entries skip the case-collision set. Archive order `places/Foo` (file) then `places/foo/` (dir) makes `create_dir_all` fail on APFS and the whole install errors. | `:364-367` | Run directories through `written` too; on collision, skip the directory and every entry under it. |
| Only gzip tar is read; the file name is hard-coded `archive-{index}.tar.gz`. Release assets from CI will be `.tar.gz`, so this holds for the catalog, but file install (5.3) needs `.tar.xz`, `.tar.bz2` and `.zip`. | `:196`, `:212` | Add `fn open_archive(path) -> Box<dyn Read>` that sniffs magic bytes: gzip `1f 8b`, xz `fd 37 7a 58 5a 00`, bzip2 `42 5a 68`, zip `50 4b 03 04`. The tar variants reuse `extract`. Zip needs a sibling `extract_zip` with the same rules; `archive.rs:191-232` shows how the `zip` crate exposes symlinks. |
| `remove` deletes in place. If the lookup is reading the directory at that moment it sees a half-deleted theme. | `:457-466` | `rename` to `.macosmetic-removing-<id>-<rand>` first, then `remove_dir_all`. |
| Orphaned staging after a crash. `tempdir_in` cleans up on drop, not on SIGKILL. | `:180-182` | Add `cleanup_staging(icons_dir)` that removes `.macosmetic-staging-*` and `.macosmetic-removing-*` older than a day. Call it from `launch_macos::prepare` after the `create_dir_all` at `launch_macos.rs:115-119`. |
| Error text is raw curl stderr or an `io::Error` string, e.g. "download failed: curl: (6) Could not resolve host". | `InstallEvent::Failed(String)`, `:152` | Replace with `InstallError` enum (5.4) mapped to the strings in 6.3. Keep the raw text for a Details dialog. |
| Restart re-execs `current_exe` with the same args. Inside a `.app`, the new process is not launched through LaunchServices, so Dock, menu bar and `open` behaviour differ. | `app.rs:3464-3470` | On macOS, when `current_exe` is inside `*.app/Contents/MacOS/`, run `open -n <bundle path>` instead. This whole message goes away once 5.1 lands; keep it behind the fork flag until then. |
| The marker holds only a sha256. Install date and source are useful on the card and for file installs. | `:225` | Write JSON: `{ "sha256": "...", "source": "catalog" \| "file:<name>", "installed": "<rfc3339>" }`. Read old single-line markers as `{ sha256 }` for compatibility. |
| `is_catalog_install` is checked on `roots.first()` only. A theme installed by the app in `~/.local/share/icons` and also present in the bundle's `share/icons` is fine, because data_home sorts first. No change, but note it. | gallery `:109-117` | None. |
| Comment at `:18-19` says install needs a restart. | `:18` | Update once 5.1 lands. |

### 4.3 Layout risk in the catalog itself

- `flat-remix` is one 113 MB archive for seven variants; `iconpack-obsidian` 71 MB; `paper-icon-theme` 49 MB. Pressing Install on `Flat-Remix-Grey-Light` downloads 113 MB and installs two themes. The card already says "113 MB download", which is honest. Consider a `[warn above 50 MB]` confirm step, or splitting Flat Remix out of v1.
- Four Sweet-folders variants each `requires` `candy-icons`, which in turn `requires` Adwaita. Installing Sweet-Blue pulls three archives. The "also installs" caption covers this.
- The catalog has 34 cards. The drawer is a vertical list, so it scrolls a long way. Section 6 groups by family.

---

## 5. Missing pieces

### 5.1 Restart-free visibility

Confirmed in the `ab4c57b` checkout that libcosmic and this app both use:

| Static | Where | Refreshable |
|---|---|---|
| `BASE_PATHS: LazyLock<Vec<PathBuf>>` | `src/theme/paths.rs:5` | No. Already handled: `launch_macos::prepare` creates `~/.local/share/icons` before the first lookup. |
| `THEMES: LazyLock<BTreeMap<Vec<u8>, Vec<Theme>>>` | `src/theme/mod.rs:16` | No. This is the blocker. |
| `CACHE` | `src/cache.rs:6` | Yes, `cache_clear()` on a builder. |
| `with_extra_paths` | `src/lib.rs:232` | Flat directories, no `index.theme`. Not a substitute for a theme. |

Fix:

1. Fork `pop-os/freedesktop-icons` to `shythulu/freedesktop-icons`, branch `macosmetic`. Creating a public fork is outward-facing; confirm with the maintainer first, as `AGENTS.md` and the libcosmic fork note require. No PR or issue upstream.
2. In the fork: `THEMES` becomes `LazyLock<RwLock<BTreeMap<..>>>`. Every reader takes `.read()`. Add `pub fn reload_themes()` that rebuilds with `get_all_themes()` under `.write()` and then clears `CACHE`. About 30 lines.
3. In this repo's `Cargo.toml`, next to the libcosmic patch:
   ```toml
   [patch.'https://github.com/pop-os/freedesktop-icons']
   freedesktop-icons = { package = "cosmic-freedesktop-icons", git = "https://github.com/shythulu/freedesktop-icons.git", branch = "macosmetic" }
   ```
   The libcosmic fork names `pop-os/freedesktop-icons` by URL, so the patch redirects both libcosmic's copy and the app's.
4. In `app.rs`, on `InstallEvent::Installed` and `IconThemeRemoved`: call `freedesktop_icons::reload_themes()`, then `load_icon_themes()`, then `icon_theme_gallery.refresh(..)`. Drop `needs_restart`, `Message::Restart`, the `restart-to-use` string and the `available-icon-themes-description` sentence about restarting.

Until step 1 is approved, the branch's restart button is the right fallback. Everything else in this spec works either way.

### 5.2 Update

Already half there. The marker holds the archive sha256; the catalog holds the current one. Rule: installed theme, marker sha256 differs from `catalog.archives[theme.archive].sha256`, show "Update" on the Installed card.

Update flow: `install_plan` with the theme forced in, extract to staging, then for each theme: `rename(target, removing)`, `rename(staged, target)`, `remove_dir_all(removing)`. Two renames, so the active theme can be updated. Then `reload_themes()` and `icon_theme_changed()` so open tabs redraw.

Hand-installed themes have no marker and never show Update.

### 5.3 Install from file and drag-and-drop

Reuses `extract` through `ExtractTarget { root, index_theme: None, id }`.

1. `Message::IconThemeInstallFile(PathBuf)`, from a button "Install from file..." in the drawer header (the existing `Dialog` in `OpenFile` mode, filter `*.tar.gz *.tgz *.tar.xz *.tar.bz2 *.zip`, directories allowed) or from a drop on the drawer.
2. `detect_themes(archive) -> Vec<(root, id)>`: one pass over the entries, collecting every `index.theme` at depth 0 or 1 whose containing directory has at least one `Normal` sibling entry. Depth 1 is the codeload wrapper directory. Zero hits: `InstallError::NoTheme`.
3. Reject any id that already exists in `~/.local/share/icons` without a marker: `InstallError::Exists(id)`. Never overwrite a hand-installed theme.
4. Second pass: `extract(open_archive(path), &targets, staging)`. Write marker `source: "file:<name>"`.
5. A dropped directory is walked instead of read as tar. The same rules apply: skip `..`, skip absolute or escaping symlinks, copy files, recreate relative links.
6. On success, the theme appears under Installed with "Installed from <file>", and it is removable because it has a marker.

Drop target: libcosmic's `dnd_destination` wrapper around the drawer content, accepting `text/uri-list`. The tab view already does this for file drops; copy the pattern, send `IconThemeInstallFile` for each path.

### 5.4 Error model

```rust
pub enum InstallError {
    Network { detail: String },        // curl exit 6, 7, 28, 35
    Moved,                             // curl exit 22 with 404 or 410
    TooLarge(u64),                     // curl exit 63, or extraction cap
    Checksum,
    UnsafeArchive(usize),              // rejected entries above 0 for a file install
    NoTheme,
    Exists(String),
    NoSpace { needed: u64 },           // ENOSPC
    Cancelled,
    Io(String),
}
```

curl's exit code is in `status.code()`. Map 22 by scanning stderr for "404" or "410". `InstallEvent::Failed(id, InstallError)` replaces the `String`.

### 5.5 Not in scope

KDE Store browser, remote catalog refresh (the catalog ships with the app; a new catalog is an app update), licence text dialog (the Source link satisfies GPL-3 attribution; CC-BY-SA Paper needs the author named, which the homepage does).

---

## 6. UI

### 6.1 Drawer layout

The branch's two-section layout stays. Three additions: a header row with "Install from file...", a family grouping in the download list, and card states for Cancel, Update and file installs.

```
+--------------------------------------------------------------+
|  < Settings            Icon themes                        x  |
+--------------------------------------------------------------+
|  Installed                           [ Install from file... ]|
|  +--------------------------------------------------------+  |
|  | COSMIC                                          (check)|  |
|  | [f] [f] [f] [f] [h] [t] [i] [p]                        |  |
|  +--------------------------------------------------------+  |
|  +--------------------------------------------------------+  |
|  | Papirus                           25 colours  [Update] |  |
|  | [f] [f] [f] [f] [h] [t] [i] [p]                        |  |
|  | GPL-3.0 . Installed 2026-10-09                 Source  |  |
|  +--------------------------------------------------------+  |
|  +--------------------------------------------------------+  |
|  | Sweet-Blue                                    [Remove] |  |
|  | [f] [f] [f] [f] [h] [t] [i] [p]                        |  |
|  | GPL-3.0 . Installed from Sweet-folders.tar.gz          |  |
|  +--------------------------------------------------------+  |
|                                                              |
|  Available to download                                       |
|  Each theme comes from its own GitHub repository.            |
|                                                              |
|  Papirus                                                     |
|  +--------------------------------------------------------+  |
|  | Papirus-Dark                       25 colours [Install]|  |
|  | [f] [f] [f] [f] [h] [t] [i] [p]                        |  |
|  | GPL-3.0 . 33 MB download . also installs Papirus Source|  |
|  +--------------------------------------------------------+  |
|  Flat Remix                                                  |
|  +--------------------------------------------------------+  |
|  | Flat-Remix-Blue-Dark               12 colours          |  |
|  | ==========================--------------  61% [Cancel] |  |
|  | Downloading 69 of 113 MB                               |  |
|  +--------------------------------------------------------+  |
|  +--------------------------------------------------------+  |
|  | Flat-Remix-Grey-Light                          [Retry] |  |
|  | Could not reach codeload.github.com.          Details  |  |
|  +--------------------------------------------------------+  |
|                                                              |
|  Drop a theme archive or folder here to install it.          |
+--------------------------------------------------------------+
```

Family heading: the part of the id before the first `-`, or the repo name from `homepage`. Add an optional `"family"` to `sources.json` and let the generator write it; fall back to the repo name.

### 6.2 Card states

| State | Right of title | Second row | Third row |
|---|---|---|---|
| Installed, active | check icon | preview strip | licence, installed date, Source |
| Installed by app, idle | Remove (hidden when another installed theme requires it or it is active) | strip | licence, "Installed {date}" or "Installed from {file}", Source |
| Installed by app, catalog has a newer archive | Update | strip | same |
| Installed by hand | nothing | strip | "Installed outside the app" |
| Available | colours badge, Install | compiled-in strip | licence, size, "also installs ...", Source |
| Installing | Cancel | progress bar | step text |
| Failed | Retry | error text | Details |
| Restart needed (only until 5.1) | "Restart to use" | compiled-in strip | licence |

Colours badge: for available themes, a `folder_colours` count the generator computes by counting `folder-<X>-documents` names in the archive. For installed themes, count the same in `icon_themes::folder_icon_names`. Zero shows nothing. This is the hook into the per-folder Customize page, whose "Icon set" dropdown already lists installed themes.

### 6.3 Copy strings

Keep the branch's ten strings. Add:

```
install-from-file = Install from file...
install-theme-title = Install icon theme
cancel = Cancel
update = Update
remove = Remove
details = Details
installed-on = Installed {$date}
installed-from-file = Installed from {$file}
installed-outside-app = Installed outside the app
downloading-progress = Downloading {$done} of {$total}
extracting = Extracting...
folder-colours-count = {$count ->
    [one] 1 colour
   *[other] {$count} colours
}
drop-theme-hint = Drop a theme archive or folder here to install it.
theme-install-failed-network = Could not reach {$host}. Check your connection and try again.
theme-install-failed-moved = The download link has moved. This will be fixed in an app update.
theme-install-failed-checksum = The download does not match the catalog. Try again after the next app update.
theme-install-failed-too-large = The archive is larger than the app allows ({$size}).
theme-install-failed-unsafe = The archive contains unsafe paths and was not installed.
theme-install-failed-no-theme = No icon theme was found in this file.
theme-install-failed-exists = A theme named {$id} is already installed outside the app. Remove it first.
theme-install-failed-space = Not enough space in {$dir} ({$needed} needed).
theme-install-cancelled = Cancelled
```

Change `available-icon-themes-description` to "Each theme comes from its own GitHub repository." once 5.1 lands.

### 6.4 Offline

Catalog and previews are compiled in, so the drawer is identical offline. Install fails with the network string. No reachability probe, no network at startup.

---

## 7. Module and API deltas

All against `feat/icon-theme-catalog`.

| File | Change |
|---|---|
| `src/icon_theme_catalog.rs` | `download`: add the curl flags and a cancel flag. `install`: take `cancel: Arc<AtomicBool>`. `extract`: add `Limits`, directory case-collision handling. New `open_archive`, `extract_zip`, `detect_themes`, `install_from_path`, `cleanup_staging`, `update_plan`, `Marker { sha256, source, installed }` with `read_marker`. `remove`: rename then delete. `InstallEvent::Failed(id, InstallError)`, `InstallEvent::Cancelled(id)`, `InstallEvent::Step(id, Step)`. |
| `src/icon_theme_gallery.rs` | Header row with "Install from file...". Family headings. Cancel, Update, Details, file-install caption. Colours badge. Drop target. Remove `needs_restart` after 5.1. |
| `src/app.rs` | New messages: `IconThemeInstallCancel(String)`, `IconThemeInstallFile(PathBuf)`, `IconThemeInstallFileResult(DialogResult)`, `IconThemeUpdate(String)`, `IconThemeShowError(String)`. On install or remove success: `reload_themes()` then the existing reload. `Restart` uses `open -n` for bundles until it is deleted. |
| `src/launch_macos.rs` | Call `cleanup_staging` after creating the icons dir. |
| `Cargo.toml` | `[patch]` for `freedesktop-icons`. No new crates. |
| `scripts/icon-theme-catalog.py` | Emit `family` and `folder_colours` per theme. Accept `url` sources from this fork's releases (already works). |
| `res/icon-themes/sources.json` | Nothing new is ready to add (section 3.1). CI-built `url` entries come with task 13. |
| `.github/workflows/icon-theme-builds.yml` | New, section 3.2. |
| `i18n/en/cosmic_files.ftl` | Section 6.3. |
| `docs/icon-theme-customization.md` | Update the status block: Phase 3 built on `feat/icon-theme-catalog`; Phase 3b deferred; §5 restart item resolved by the fork. |

---

## 8. Finishing task list

Ordered so each step leaves the branch shippable.

| # | Task | Files | Done when |
|---|---|---|---|
| 1 | Harden curl: `--proto-redir =https --max-redirs 5 --max-filesize --connect-timeout 15 --speed-time 60 --speed-limit 1024`. Map exit codes to `InstallError`. | `icon_theme_catalog.rs` | A test with a fake `curl` on `PATH` returning exit 6 yields `Network`; exit 63 yields `TooLarge`. |
| 2 | Extraction caps and directory case-collision fix. | same | Tests: 250,001 empty entries fail; `Foo` file then `foo/` dir installs without error and drops the dir. |
| 3 | Cancel: `AtomicBool`, `child.kill()`, `InstallEvent::Cancelled`, Cancel button, staging removed. | catalog, gallery, app | Pressing Cancel mid-download returns the card to Install within a second and leaves no `.macosmetic-staging-*`. |
| 4 | Error strings and Details dialog. | gallery, app, ftl | Each `InstallError` variant has a string; Details shows the raw text. |
| 5 | Marker as JSON with source and date; `read_marker`; "Installed {date}" on cards. | catalog, gallery | Old one-line markers still count as catalog installs. |
| 6 | `remove` renames first; `cleanup_staging` on launch. | catalog, launch_macos | A `.macosmetic-staging-x` dir older than a day is gone after launch. |
| 7 | Update: compare marker sha256 to catalog; Update button; two-rename swap; `icon_theme_changed()` after. | catalog, gallery, app | Bumping a `sources.json` ref and regenerating shows Update on that card; pressing it swaps the directory while it is active. |
| 8 | Fork `freedesktop-icons`, `reload_themes()`, `[patch]`, call it after install, remove and update. Delete `needs_restart`, `Restart`, `restart-to-use`. Needs fork approval. | fork, Cargo.toml, app, gallery, ftl | Install Papirus, press the card, folders redraw with no restart. |
| 9 | `open_archive` with magic-byte sniffing; `extract_zip`; `detect_themes`; `install_from_path`; "Install from file..." button and dialog. | catalog, gallery, app, ftl | Dropping `Sweet-folders.tar.gz`, a `.zip` of it, and the unpacked folder each install and show "Installed from ...". A zip with `../` entries fails with the unsafe string. |
| 10 | Drag-and-drop on the drawer. | gallery | Drop from Finder installs. |
| 11 | Family headings and colours badge; generator emits `family` and `folder_colours`. | script, catalog.json, gallery | Papirus shows "25 colours"; Flat Remix cards sit under one heading. |
| 12 | Bump `sources.json` refs to the tags in section 3 (Papirus is already `20260801`), regenerate, check the sha256 values still match. | sources.json, catalog.json, previews | `cargo test --lib icon_theme_catalog` passes; every archive downloads and verifies. |
| 13 | CI workflow for WhiteSur, Tela, Colloid, Fluent, Reversal (Qogir optional); release; `url` entries. | workflow, sources.json | WhiteSur installs from the gallery with a sha256 that matches the release asset. |
| 14 | Docs: status block in `icon-theme-customization.md`; one paragraph in `macos-porting-notes.md` on where themes live. | docs | |

Tasks 1 to 7 are a day or two. Task 8 is half a day plus the approval wait. Tasks 9 to 11 are a day. 12 and 13 are catalog work, not app work.

---

## 9. Security rules

| Rule | Enforced where |
|---|---|
| Nothing from an archive runs | No `Command` touches extracted content. Files are created with default mode (0644 under the usual umask). `install.sh` themes are built in CI, never locally. |
| HTTPS only, including redirects | `curl --proto =https --proto-redir =https` |
| Download size cap | `--max-filesize`, 1.25x the catalog size |
| Extraction caps | `Limits`: 1.5 GB, 250k entries |
| Path traversal, absolute paths, escaping symlinks, hard links, devices | `normal_path`, `link_stays_inside`, entry type check in `extract`; same rules in `extract_zip` and the directory walk |
| Writes only under `~/.local/share/icons` | `user_icons_dir()` is the only destination; the `.app` is never written |
| Never delete what the app did not create | Marker gate in `remove`, `Exists` error in file install |
| sha256 on every catalog download, no override | `verify_sha256` |
| No network at startup | The catalog is compiled in; the only network call is a user-pressed Install |
| Case collisions do not fail or overwrite | First entry wins, logged in `ExtractStats` |

---

## 10. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| Fork approval for `freedesktop-icons` is refused or slow | Restart stays | Everything else ships; the restart button already works |
| GitHub regenerates codeload tarballs and the sha256 changes. GitHub's stated position is that auto-generated archive checksums are not guaranteed, and its advice is to use release assets. | Install fails with the checksum string until the catalog is regenerated | The generator makes rehashing one command. Prefer release assets where projects publish them (Nordzy does; Papirus and Adwaita publish tags only). Our CI-built themes are release assets by construction. |
| Flat Remix at 113 MB for one variant | Slow install, disk use | Honest size on the card; consider dropping Flat Remix variants to two |
| 34 cards plus future CI themes make the drawer long | Scrolling | Family headings; a filter field later |
| Quick Look thumbnails hide most file icons on macOS | A new theme changes less than the preview suggests | Known from `icon-theme-customization.md` §1.3; the preview strip is folders and generic files, which do change |
| Two app windows install at once | Both write the same `<id>`; the branch handles `target.exists()` by keeping the first | Acceptable |
| A theme's `Inherits=` names breeze or Adwaita that the user does not have | A few icons fall through to Cosmic | Already the case for the hand-installed Papirus on this Mac; fine |
| Previews are SVGs from the themes, compiled in | 2.7 MB in the binary, GPL artwork embedded | GPL-3 app, GPL-3 artwork, Source link on every card. Paper is CC-BY-SA, homepage names the author |
