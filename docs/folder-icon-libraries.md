# Open-source folder icon libraries

A survey of folder icon artwork and folder-name mappings that could feed icon
customization. This is a companion to `docs/icon-theme-customization.md`, which covers how
icons are resolved and how a theme browser would install themes.

Researched 2026-10-07. Stars, dates and releases come from the GitHub API on that day.
**Lic✓** means the LICENSE or COPYING file itself was read. Anything else comes from
GitHub's licence detection or the README and is flagged. None of the projects listed uses
a non-commercial licence.

The sources fall into three groups, and each feeds a different feature:

| Kind | Example | What the app would do with it |
|---|---|---|
| **Freedesktop icon themes** with folder sets and colour variants | Papirus, Tela, Sweet-folders | Install as an icon theme (the System76 model). Folders change through `folder`, `folder-documents`, … |
| **Name → icon mappings** from editors | VS Code Material Icon Theme | Give a folder an icon based on its *name* (`src`, `.git`, `node_modules`) |
| **General icon libraries** | Lucide, Material Symbols | Badge glyphs drawn over a generic folder, or a fallback set |

---

## 1. Freedesktop (GNOME/KDE) folder themes and colour systems

### 1.1 Folder colours

| Project | URL | Licence | Lic✓ | Activity | How it works |
|---|---|---|---|---|---|
| **Papirus** | github.com/PapirusDevelopmentTeam/papirus-icon-theme | GPL-3.0 | Y | 2026-09, 8.1k★ | 25 colours shipped side by side as `folder-<colour>-<suffix>` (81 icons per colour) plus `user-<colour>-{desktop,home}`. Quirk: the coloured form is `-public`, not `-publicshare`. |
| papirus-folders | github.com/PapirusDevelopmentTeam/papirus-folders | MIT (script only) | Y | 2025-05 | Repoints `folder-X.svg` → `folder-<colour>-X.svg` with `ln -sf` at sizes 22, 24, 32, 48 and 64. Symbolic icons and `user-trash` are never recoloured. |
| Catppuccin papirus-folders | github.com/catppuccin/papirus-folders | MIT label, but the artwork is recoloured Papirus, so **treat as GPL-3.0** | Y | 2024-06, 274★ | Adds 56 `cat-<flavour>-<accent>` colours to Papirus |
| Papirus-Colors | github.com/varlesh/papirus-colors | GPL-3.0 | Y | **2022, stale** | Places-only overlay, `Inherits=Papirus,breeze,hicolor`. Follows the accent colour through a `ColorScheme-Highlight` CSS class in the SVG |
| papirus-folders-nordic | github.com/basigur/papirus-folders-nordic | GPL-3.0 | Y | 2024-01 | Overlay theme, `Inherits=Papirus,…` |
| Gruvbox Plus | github.com/SylEleuth/gruvbox-plus-icon-pack | GPL-3.0 | Y | 2026-10, 803★ | 25 colours (one, "plasma", follows the KDE accent). Covers all 32 extra names in §4 |
| Tela | github.com/vinceliuice/Tela-icon-theme | GPL-3.0 | Y | 2026-08, 1.9k★ | 15 colours, made at install time by `sed s/#5294e2/<colour>/` |
| Colloid | github.com/vinceliuice/Colloid-icon-theme | GPL-3.0 | Y | 2026-08, 1.2k★ | 9 accents × schemes (Nord, Dracula, Gruvbox, Everforest, Catppuccin) |
| WhiteSur (macOS look) | github.com/vinceliuice/WhiteSur-icon-theme | GPL-3.0 | Y | 2026-09, 2.1k★ | 9 accents |
| Fluent | github.com/vinceliuice/Fluent-icon-theme | GPL-3.0 | Y | 2026-08 | 9 accents |
| Vimix | github.com/vinceliuice/Vimix-icon-theme | **CC-BY-SA-4.0** | Y | 2025-10 | 10 gem-named colours |
| Reversal | github.com/yeyushengfan258/Reversal-icon-theme | GPL-3.0 | Y | 2026-03 | 12 colours |
| Flat Remix | github.com/daniruiz/flat-remix | GPL-3.0 | Y | 2025-11, 1.8k★ | 12 colours × Light/Dark, each a separate full theme |
| Nordzy | github.com/MolassesLover/Nordzy-icon | GPL-3.0 | Y | 2026-04 | 9 colours |
| Yaru | github.com/ubuntu/yaru | Icons CC-BY-SA-4.0 | Y | 2026-09 | 12 accent overlays. Mostly **PNG**, few extra names |
| Numix folders | github.com/numixproject/numix-folders | GPL-3.0 | Y | 2024-02 | Script that rewrites Numix in place (6 styles × 9 colours) |

### 1.2 Folder-only overlay themes

These ship only `places/` icons and get everything else from a parent theme through
`Inherits=`. This is the cleanest way to offer "change just the folders" inside the
freedesktop model.

| Project | URL | Licence | Lic✓ | Notes |
|---|---|---|---|---|
| **Sweet-folders** | github.com/EliverLara/Sweet-folders | GPL-3.0 | Y | 12 variants. `Inherits=candy-icons,breeze-dark,…`. Has every core name except `folder-remote`, and has symbolic icons. Active (2025-02) |
| **MoreWaita** | github.com/somepaulo/MoreWaita | GPL-3.0 | N | Adds *extra* names on top of Adwaita (`folder-blender`, `folder-flatpak`, …), each with a `-symbolic` form. Very active |
| Papirus-Colors, papirus-folders-nordic | see §1.1 | GPL-3.0 | Y | Good structural models, but stale |

The overlay must list the parent the user actually has installed. Point `Inherits=` at the
active theme, or generate the overlay's `index.theme` at install time.

### 1.3 Leave out or flag

- **Tela-circle** and **m4thewz/dracula-icons**: no licence file. The Dracula README links
  a LICENSE.md that does not exist, and the theme is built from Tela-circle.
- **Henriquehnnm/rose-pine-icon-theme**: has an MIT label, but its layout matches Papirus,
  so it is probably a recolour of GPL-3.0 artwork.
- **Suru++** (dead since 2019) and the **Fausto-Korpsvart** Everforest, Kanagawa and
  Gruvbox repos (archived 2025-10).
- No official Dracula, Rosé Pine, Everforest or Kanagawa icon theme exists.

---

## 2. Name → icon mappings (editor icon themes)

These map a folder's *name* to an icon, the way VS Code shows a special icon for `src` or
`.git`. COSMIC Files has nothing like this. The only fixed mapping is `SPECIAL_DIRS` for
the XDG user directories (`src/tab.rs:123-153`).

| Project | URL | Licence (artwork) | Lic✓ | Folder data | Notes |
|---|---|---|---|---|---|
| **Material Icon Theme** | github.com/material-extensions/vscode-material-icon-theme | MIT, which covers the SVGs too | Y | `dist/material-icons.json` in the npm package `material-icon-theme`: **4,659 name keys → 269 icons**. Source is `src/core/icons/folderIcons.ts` | v5.39.0 (2026-10-02), 3k★. One consistent 16×16 SVG structure (`id="folder"` + `id="motive"`), so recolouring is easy. Open-folder variants exist only in the npm build. Some icons carry brand logos (trademarks) |
| **Catppuccin Icons** | github.com/catppuccin/vscode-icons | MIT | Y | `src/defaults/folderIcons.ts`: `Record<icon, {folderNames}>`, ~114 icons, each with an `_open` form | 4 flavours plus a `css-variables` set that could take COSMIC accent colours |
| **Atom Material Icons** (JetBrains) | github.com/AtomMaterialUI/iconGenerator | MIT | Y | `folder_associations.xml` / `.json`: **351 regex rules** with `folderColor` and `folderIconColor` | Built as a tinted generic folder plus a glyph, which suits a native-looking macOS folder |
| Symbols | github.com/miguelsolorio/vscode-symbols | MIT | Y | Plain VS Code JSON, 161 names → 83 icons | Minimal look, close to Apple's style. Small |
| vscode-icons | github.com/vscode-icons/vscode-icons | Code MIT. **Icons CC BY-SA 4.0** (README only) | Partial | `src/iconsManifest/supportedFolders.ts`, ~202 folder types | ShareAlike. Compatible with GPL-3.0 one way only |
| Bearded Icons | github.com/BeardedBear/bearded-icons | GPL-3.0 | Y | ~350 entries in `src/shared/folderNames.ts` | No releases |
| file-icons (Atom) | github.com/file-icons/atom | MIT (font ISC) | Y | ~50 regex rules in `config.cson` | Glyphs only, inactive since 2021 |
| Seti UI | github.com/jesseweed/seti-ui | MIT | Y | Effectively no folder icons | Not useful |
| JetBrains expui | intellij-community `platform/icons/src/expui/nodes/` | Apache-2.0, stated in each SVG's header | Y | ~20 role icons (test source, library, home, log, web, github). No name map | Check the header of each SVG |

**Mapping formats worth copying:**

- **Zed's icon-theme schema** (`zed.dev/schema/icon_themes/v0.3.0.json`):
  `named_directory_icons: { "<name>": { collapsed, expanded } }`. It is the simplest
  exact-match format. Material Icon Theme and Catppuccin are already ported to it.
- **Material Icon Theme's expansion rule.** Each name also matches with a `.` or `_`
  prefix, among others. Layer this on top of the exact-match format.
- **Terminal file-listing tools ship small, curated directory lists:**
  - eza `src/output/icons.rs` `DIRECTORY_ICONS`: about 40 names, Rust `phf_map!`,
    **EUPL-1.2**.
  - yazi `theme-dark.toml` `[icon] dirs`: 14 names including the macOS home folders, MIT.
  - lsd: Apache-2.0.
  - These use Nerd Font glyphs, not artwork.

**Exclude:** Material Theme UI (JetBrains; closed source since 2021, EULA).

---

## 3. General icon libraries (folder glyphs)

For "a generic folder plus a badge" or a fallback set. Licences verified from LICENSE files.

| Library | Licence | Folder icons | Notable |
|---|---|---|---|
| **Lucide** | ISC (+MIT) | 33 | `folder-git`, `folder-code`, `folder-archive`, `folder-key`, `folder-lock`, `folder-sync`, `folder-tree`. The best developer-oriented set. Zed uses it |
| **Tabler** | MIT | 32 outline + 3 filled | `folder-code`, `folder-root`, `folder-symlink`, `folder-share` |
| Pictogrammers MDI | Apache-2.0 (icons) | 104 | Broadest (`folder-home`, `folder-music`, `folder-zip`, `folder-network`, `folder-google-drive`, …). No git or code folder |
| Fluent UI System Icons | MIT | 30 groups | Updated daily |
| Material Symbols | Apache-2.0 | 22 | `folder_code`, `folder_zip`, `folder_shared`. Variable weight and fill axes |
| Phosphor | MIT | 16 × 6 weights | |
| IBM Carbon | Apache-2.0 | 11 | |
| Bootstrap Icons | MIT | 10 | |
| Iconoir | MIT | 7 | |
| Heroicons | MIT | 5 | |
| Font Awesome Free | Icons **CC BY 4.0** (attribution required) | 7 solid / 4 regular | |
| Primer Octicons | MIT | 4 | `file-directory`, `-symlink` |
| Simple Icons | CC0, but **trademarks stay with their owners** (`DISCLAIMER.md`) | 0 (brand logos) | Possible badge glyphs. Needs a trademark review |
| Devicon | MIT, with brand trademark caveats | 0 (tech logos) | Same |

**Do not use:**

- **Remix Icon v4.9.0 and later**: since 2026-01-25 it uses the "Remix Icon License
  v1.0", which is not OSI-approved and forbids competing icon packs. v4.8.0 and earlier
  were Apache-2.0.
- **Apple SF Symbols**: licensed only for apps on Apple platforms, with no redistribution
  and no use in app icons or logos.
  - **[I]** A macOS-only app could draw them at runtime through AppKit
    (`NSImage(systemSymbolName:)`), because nothing is redistributed.
  - They can't ship in a theme.

---

## 4. Folder icon names (what a theme must contain)

**Icon Naming Spec 0.8.90:**

- Place icons: `folder`, `folder-remote`, `network-server`, `network-workgroup`,
  `start-here`, `user-bookmarks`, `user-desktop`, `user-home`, `user-trash`.
- Status icons: `folder-open`, `folder-drag-accept`, `folder-visiting`, `user-trash-full`.
- Action icon: `folder-new`.

**Not in the spec, but every theme ships them:** `folder-documents`, `folder-download`,
`folder-music`, `folder-pictures`, `folder-publicshare`, `folder-templates`,
`folder-videos`. These are GNOME's names for the XDG user directories, and they are what
`SPECIAL_DIRS` uses.

**Common extra names** (a subset of Papirus `places/48x48`):

- Development and system: `folder-code`, `folder-development`, `folder-git`,
  `folder-github`, `folder-gitlab`, `folder-script`, `folder-docker`, `folder-java`,
  `folder-html`, `folder-root`, `folder-temp`.
- Cloud: `folder-cloud`, `folder-dropbox`, `folder-nextcloud`, `folder-gdrive`,
  `folder-onedrive`, `folder-sync`, `folder-syncthing`.
- Apps: `folder-games`, `folder-steam`, `folder-vbox`, `folder-wine`, `folder-obsidian`.
- Status: `folder-favorites`, `folder-recent`, `folder-important`, `folder-locked`,
  `folder-unlocked`, `folder-backup`, `folder-projects`, `folder-apple`.

Which themes ship them:

- **Most:** Papirus and its derivatives, the vinceliuice family, Reversal, Kora, Nordzy,
  Gruvbox Plus, MoreWaita.
- **About 20 each:** Breeze, Candy, Flat Remix.
- **2 to 4 each:** COSMIC, Pop and Yaru, so a mapping must always fall back to `folder`.

This leads straight to a cheap feature, **name-based special folders in theme terms**:

- Extend `SPECIAL_DIRS` with well-known macOS and developer paths, for example:
  - `~/Developer` → `folder-development`, then `folder-code`, then `folder`
  - `~/Library/Mobile Documents/com~apple~CloudDocs` → `folder-cloud`
  - any `.git` directory → `folder-git`
- Pass the chain as `IconFallback::Names`. This works once phase 1 of
  `docs/icon-theme-customization.md` lands.
- Any installed theme then gets the benefit, with no artwork shipped by us.

---

## 5. Per-folder custom icons elsewhere

- **GNOME (Nautilus):** reads the gvfs metadata keys `metadata::custom-icon` (a URI) and
  `metadata::custom-icon-name` (a themed name). costales/folder-color (GPL-3.0) sets the
  icon to `folder-<colour>[-<xdg>]` from the current theme.
- **KDE (Dolphin):** reads `<dir>/.directory` with `Icon=`, which takes a themed name or a
  path (`./` means relative to the folder). It also reads `EmptyIcon=`.
- **COSMIC Files upstream: no support.** The issues are still open (#591, #1427, #1511).
  PR #1934 added `.directory` `Icon=` support and was closed unmerged on 2026-09-30 because
  it was LLM-generated, not because of the feature.
- **macOS Finder: not researched.** **[I]**
  - `NSWorkspace.setIcon(_:forFile:options:)` stores a folder's custom icon in a hidden
    `Icon\r` file and sets the custom-icon Finder flag.
  - Reading that back would let us honour icons the user already set in Finder.
- **Decided for this fork:** see `docs/icon-theme-customization.md` §6 and Phase 3c.
  - Per-folder looks are stored in an app-owned cosmic-config map, which keeps files out
    of the user's folders.
  - `.directory` `Icon=` is **read** as a KDE-compatible fallback.
  - Finder custom icons and tags are optional macOS extras.

---

## 6. Recommendations

1. **Theme catalog (the System76 path):**
   - Start with **Papirus** (GPL-3.0, very active, 25 colours, the most extra names).
   - Offer folder colour as a setting that resolves `folder-<colour>-<suffix>` with a
     fallback to `folder-<suffix>`, rather than copying papirus-folders' symlink swap.
   - Mind the `-public` / `-publicshare` quirk.
   - Add **Sweet-folders** as the folder-only overlay, and WhiteSur for the macOS look.
2. **Recolour at render time (possible later step):**
   - Tela, Papirus-Colors and Gruvbox Plus "plasma" put the folder colour in one CSS
     class or a single hex value.
   - Rewriting it before rasterising would give any accent colour from one SVG set.
   - **[I]** Not checked against libcosmic's SVG path.
3. **Name-based folder icons (a separate feature from themes):**
   - Use **Material Icon Theme** for coverage (MIT artwork, a machine-readable JSON
     manifest, 269 icons).
   - Or use **Catppuccin Icons** for a smaller set that can take COSMIC accent colours.
   - Store the mapping in Zed's `named_directory_icons` shape, and generate it at build
     time from the upstream JSON.
   - Review logo-bearing icons for trademark issues before shipping them.
4. **Badge glyphs:** use Lucide (ISC) or Tabler (MIT) for a "tinted folder + glyph" style.
   This style suits native-looking macOS folders better than full-colour editor art.
5. **Per-folder icons:** the app's own store comes first, then `.directory` `Icon=`
   (read-only), then Finder extras. See `docs/icon-theme-customization.md` §6.
6. **Browsing for more:** the KDE Store (OCS category 132) is the large live source. In a
   sample of 100 items, licences were published as item tags. See Phase 3b of
   `docs/icon-theme-customization.md`, including how to avoid the slowness of Plasma's
   "Get New…".

**Not verified:**

- Licences marked N in Lic✓, vscode-icons' CC BY-SA (README only), and the macOS
  folder-icon repos below.
- Folder coverage for Flat Remix, Gruvbox Plus, Rosé Pine and BeautyLine is a lower bound,
  because the GitHub tree listings came back truncated or the scan was partial.
- Nerd Fonts' per-glyph licence audit was not read.

### Apple-derived artwork: avoid

- **folderify** (github.com/lgarron/folderify) is MIT, but its repo contains Apple's own
  `GenericFolderIcon` iconsets. Reuse its technique (mask → engraved glyph on the folder),
  not its PNGs.
- **krestaino/macos-folder-icons**, **GwynethLlewelyn/macOS-Big-Sur-folder-icons** and
  **othyn/github-folder-icon-macOS** all derive from Apple artwork. The last one has no
  licence at all.
- No maintained, clearly licensed collection of *original* Big Sur-style folder art was
  found.
