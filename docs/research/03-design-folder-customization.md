# Folder customization: design review and redesign spec

Reviewed 2026-10-09 against `master` at `018a5ae`. Scope: the "Customize folder..."
context menu item, the `FolderAppearance` context drawer, and the `FolderLook` model
behind them. Nothing in the repo was changed.

## Verdict

The storage model is right and the drawer works. The UI makes the common case slow and
the rare case confusing.

| Problem | Cost to the user | Fix |
|---|---|---|
| A colour takes 3 steps. Finder, Dolphin and Nautilus make it 1 | The 80% case is the slowest path | "Folder colour" submenu in the context menu |
| Icon grid shows raw names like `folder-git`, capped at 96 with no count | Users scroll a wall of look-alike folders and miss icons that exist | Friendly labels, groups, search synonyms, "Showing N of M" |
| "Icon set" is unexplained | Nobody knows that picking Papirus here pins the folder to Papirus forever | Rename to "Icons from", one line of helper text that changes with the choice |
| Colour, icon and image are mutually exclusive, and the UI hides that | Picking an icon silently drops the colour | Make the one-look rule visible now. Model change later |
| Multi-select has no "Mixed" state | Nothing looks selected, and Reset is enabled for no visible reason | "Mixed" caption, apply-to-all wording |

The rest of this document ranks every finding, then gives the redesigned drawer and menu,
then the code changes.

## How the competition does it

| | Entry point | Colour | Icon or glyph | Own image | Reset | Multi-select | Applies live |
|---|---|---|---|---|---|---|---|
| Finder, macOS 26 | Context menu: 7 colour dots, then "Customize Folder..." below them. Also File menu | One click on a dot. Tag colours tint the folder. Default folder colour in System Settings > Appearance | Popover: SF Symbols in categories plus an Emoji button with search. Symbol is embossed, emoji is a full-colour sticker. Colour and glyph combine | Get Info, click the small icon, paste. Does not sync | "Clear" button bottom-left of the popover. Click the dot again to drop a colour | Yes | Yes, click outside to dismiss |
| Dolphin | Properties > click the icon. Third-party service menu for colours | Service menu, one click | Icon dialog with "System icons" by category and "Other icons" | Browse in the same dialog | Pick `folder` again | No | After OK |
| Nautilus plus folder-color | Context menu | Submenu of named colours, one click | Emblems: Important, Favorite, Finished and others | Properties > click the icon | "Default" item appears in the same submenu once set | Yes | Yes |
| Folder Colorizer Pro, macOS | App window, drag a folder on | Palette and custom colours | Emoji and decals | Yes | Button | Yes | Yes |
| Macosmetic today | Context menu "Customize folder..." | Drawer swatches, 3 steps | Drawer grid of theme icons, 96 max | Drawer "Browse..." | "Reset to default" destructive button | Partly | Yes |

Sources: Apple's guide "Customize the look of folders and files on Mac"
(support.apple.com/guide/mac-help/mchlp2313/mac), Howard Oakley's "Customising folders in
Tahoe" (eclecticlight.co, 2025-09-18), MacMost "How To Customize Folders In macOS Tahoe",
AppleInsider "How folder emoji & customization works in macOS Tahoe", KDE UserBase
"Dolphin/Customize Folder Icon", costales/folder-color README, Folder Colorizer Pro App
Store page.

Two Finder facts matter most for this design.

- Finder splits the fast path from the browse path. Colour is in the menu. Glyphs are in a
  popover. The popover also carries "Clear".
- Finder shows a live preview and dismisses on click-outside. There is no OK button.
  The current drawer already matches this. Keep it.

## 1. Findings, ranked by user impact

Each finding names where it lives and what to do. Size estimates are in section 3.

### 1.1 The 80% case needs 3 steps

A colour is the most common customization. Finder makes it one click. Here it is
right-click, "Customize folder...", wait for the drawer, click a swatch, close the drawer.

`src/menu.rs:242-248` adds only one item. The libcosmic fork in `Cargo.lock` has
`menu::Item::Folder(label, children)` for submenus and `menu::Entry::new(..).icon(..)
.checked(..)` for items with a 14 px leading icon, which `sort_item` at `src/menu.rs:66-78`
already uses. So a colour submenu can show a small coloured folder next to each name. The
brief's assumption that items are text-only is too strict: text plus one icon handle is
what the widget supports, and that is enough.

Fix: a "Folder colour" submenu with the ten colours, a divider and "None". The current
colour is checked. See section 2.1.

### 1.2 Raw icon names, a silent 96 cap, no grouping

`src/folder_appearance.rs:17` sets `MAX_ICONS = 96`. `refresh()` at line 152 takes the
first 96 matches and drops the rest with no message. Papirus has well over 100 folder
names after `without_colour_variants`, so the tail is invisible until the user guesses a
search term. The tooltip is the raw name: `folder-git`, `folder-publicshare`,
`folder-development`.

Search at line 155 matches the raw name only. Typing "code" misses `folder-development`.
Typing "public" finds `folder-publicshare`, but typing "shared" does not.

Fix, in three parts.

| Part | What changes |
|---|---|
| Friendly labels | `folder-git` becomes "Git". `folder-publicshare` becomes "Public". Unknown names: strip `folder-`, replace `-` and `_` with spaces, capitalise the first letter |
| Search synonyms | A small table: code = development, source, programming; downloads = download; public = publicshare, shared. Search matches label, raw name and synonyms |
| Groups | Places: `folder`, the XDG kinds and `user-*`. Purpose: everything else. Plus a Recent row of the last picks. A footer reads "Showing 96 of 142. Search to narrow the list." |

### 1.3 "Icon set" is a hidden pinning decision

The dropdown at `src/folder_appearance.rs:248-252` says "Icon set" with options "Current
icon theme" and every installed theme by name. Nothing says what differs. The difference
is large: `icon_look()` at line 177 stores `theme: Some(id)` for any set other than the
first, so the folder is pinned to that theme and ignores future theme switches.
`theme_icon_handle` at `src/folder_look.rs:217-239` then looks the icon up in that theme
on unix, and only falls back to the active theme when the pinned theme has no such icon.

The default is right. `FolderAppearance::new` at line 36 starts on index 0 unless the
stored look is already pinned. So a fresh folder follows the theme. The problem is only
that the user cannot tell.

Fix: label "Icons from". Option 0 reads "Current theme (Papirus)" with the active theme's
display name filled in. One line of helper text under the dropdown changes with the
choice: "Follows the icon theme when you change it." or "Stays on Papirus when you change
the icon theme." Add a "not installed" state for a pinned theme that is gone.

### 1.4 Colour, icon and image are exclusive, and the UI hides it

`FolderLook` at `src/folder_look.rs:25-35` is an enum. A folder has one of colour, icon or
image. Finder lets a colour and a glyph combine, and that combination is its best feature:
colour for the category, glyph for the item.

Today the three drawer sections sit side by side like independent settings. Click a swatch,
then click an icon, and the swatch deselects with no explanation.

Fix now: say it once, under the title: "A folder shows one look: a colour, an icon or an
image." Also show which section is active with the `selected` state, which already exists,
and grey the other two section titles slightly. Fix later: a struct with
`colour: Option<..>` and `icon: Option<..>`, drawn as the theme's coloured folder with the
icon's emblem when the theme has one. That is a model and renderer change. Section 3 sizes
it L.

### 1.5 Multi-select has no Mixed state

`current_look()` at `src/folder_appearance.rs:169-175` returns `None` when the selected
folders differ. The drawer then shows no selection anywhere, the header says "3 folders",
and "Reset to default" is enabled because at least one folder has a look. The user sees an
enabled destructive button and nothing selected.

The header preview at line 136 draws only the first folder. If the first is a Documents
folder, the grid shows Documents-emblem variants that will not match the other two.

Fix: a caption under the title with three states: "No custom look", the shared look's
name, or "Mixed looks. Picking one applies it to all 3." For multi-select, draw grid and
swatches with the plain `folder` base, not the first folder's base.

### 1.6 Reset is overstated, and there is no partial clear

"Reset to default" uses `button::destructive` at line 290. Nothing is destroyed. A look is
one config entry and the user can set it again in one click. Finder calls this "Clear" and
uses a plain button.

Fix: rename to "Clear", use `button::standard`, and move it into the context drawer's
`.actions()` slot so it sits in the header where Finder's Clear sits at the bottom of its
popover. Also add "None" to the colour submenu, so colour can be cleared without the
drawer.

### 1.7 The image section shows a hash

After picking an image, `image_label` at line 272 shows the file name from
`import_image`, which is the content hash: `3fa9c2d1e0b4a7c8.png`. The user picked
`cover.png`.

Fix now: show a 40 px thumbnail of the chosen image as a selected icon button and the text
"Custom image". Fix later: store the original file name alongside the imported path, which
changes `FolderLook::Image`.

### 1.8 Preview fidelity is good at 64 px and absent at small sizes

The header preview goes through `tab::folder_icon_with_look`, the same path the grid
uses. Good. It is only 64 px. Colours and emblems that read at 64 px can vanish at the
list view's 16 px and the sidebar's symbolic form drops colour entirely, by design at
`folder_handle_symbolic`, `src/folder_look.rs:201-215`.

Fix: a second preview row at 32 px and 16 px next to the large one, labelled "List" and
"Sidebar". The sidebar one uses `folder_icon_symbolic`. Cheap and honest.

### 1.9 Keyboard and accessibility

| Issue | Where | Fix |
|---|---|---|
| Swatches differ by colour only. Tooltips need hover | `view()` swatch loop, line 206 | A caption line under the swatches names the selected colour. The submenu names every colour in text |
| Selected state is a highlight ring only | `button::icon(..).selected(..)` | Overlay a 12 px `object-select-symbolic` on the selected swatch and grid icon |
| Grid buttons are not reachable by Tab in Iced | libcosmic limitation | Keep the search field as the keyboard entry point. Enter on a search with one match applies it |
| Escape does not close the drawer | `ContextPage::FolderAppearance` | Esc closes any context drawer. Check whether the app already binds it; if not, add it in `app.rs` key handling |
| Reduced motion, contrast | Not affected | None |

### 1.10 Drawer vs popover vs submenu

| Container | Fits | Does not fit |
|---|---|---|
| Submenu | Ten colours, Clear, recent picks | Anything that needs search or a grid |
| Popover anchored to the item | Finder's choice for glyphs | libcosmic's `widget::popover` anchors to a widget inside the same element tree. Anchoring to a grid item inside a scrollable tab is fragile, and the popover would cover the folder it previews |
| Context drawer | Grid, search, dropdown, image browse. The COSMIC idiom. Stays open while the user tries several looks | A one-click colour |

Decision: submenu for colour and recents, drawer for everything else. Keep the drawer.

### 1.11 Menu placement and the background menu

"Customize folder..." sits after "Show details" and before the sidebar and trash groups,
`src/menu.rs:241-248`. Finder puts colour dots and "Customize Folder..." together at the
bottom. Grouping the two new items in their own divider group reads better.

Right-clicking the empty background of a folder offers nothing for the folder you are in.
Finder's File menu does. Add "Customize this folder..." to the background menu in
`Mode::App` when the location is a path.

### 1.12 Looks follow the theme: mostly yes

| Look | After an icon theme switch |
|---|---|
| `Colour` | Follows. `themed_colour_handle` tries the new theme's `folder-red`, then `recoloured_handle` repaints the new theme's folder. Cache is cleared in `icon_theme_changed`, `src/app.rs:2515` |
| `Icon { theme: None }` | Follows, with fallback to `base` then `folder` when the new theme lacks the name |
| `Icon { theme: Some }` | Pinned. Falls back to the active theme silently when the pinned theme is uninstalled |
| `Image` | Unchanged |

One gap: a raster-only theme cannot be recoloured, so `colour_handle` returns `None` and
the swatch row is empty with no message. Add the state text "This icon theme has no
coloured folders." when `swatches` is empty.

### 1.13 Naming consistency

Menu says "Customize folder...". Drawer title says "Folder appearance". The user clicks one
word and reads another. Use "Customize folder" for both. The project uses sentence case
everywhere, so keep sentence case even though macOS menus are title case.

## 2. Redesigned spec

### 2.1 Context menu

```
Open
Open in new tab
Open in new window
Reveal in Finder
────────────────────────
Rename
Cut
Copy
Move to
Copy to
────────────────────────
Compress
────────────────────────
Show details
────────────────────────
Folder colour              ▸  ┌──────────────────────┐
Customize folder...           │ ▣  Red               │
────────────────────────      │ ▣  Orange            │
Add to sidebar                │ ▣  Yellow            │
────────────────────────      │ ▣  Green             │
Move to trash                 │ ▣  Cyan              │
                              │ ▣  Blue              │
                              │ ▣  Violet            │
                              │ ▣  Magenta           │
                              │ ▣  Brown             │
                              │ ▣  Grey              │
                              │ ──────────────────── │
                              │ ✓  None              │
                              └──────────────────────┘
```

Rules.

| Rule | Detail |
|---|---|
| `▣` | A 14 px handle from `folder_look::folder_handle(&Colour(id), "folder", 14)`. Use `menu::Entry::new(label, action).icon(handle)` |
| Checked item | `Entry::checked(true)` on the colour every selected folder shares. "None" is checked when none has a colour. Mixed selection: nothing checked |
| Click | `Action::SetFolderColour(Option<&'static str>)`. Applies to every selected folder through `set_folder_looks`. No drawer opens |
| A folder with an icon or image look | Picking a colour replaces it. Same rule as the drawer |
| Raster-only theme | Submenu still shows. Items without a handle show text only |
| When shown | Same condition as "Customize folder..." today: every selected item is a folder, no mount points, nothing in trash, `Mode::App` |

Recent picks do not go into the menu in this round. Ten colours plus None is already
eleven items. Recents live in the drawer.

### 2.2 Drawer

```
┌ Customize folder ──────────────────────────────── [Clear] [✕] ┐
│                                                               │
│  ┌──────┐   Projects                                          │
│  │ 64px │   Red folder · follows the icon theme               │
│  │      │   List ▣ 32   Sidebar ▢ 16                          │
│  └──────┘                                                     │
│  A folder shows one look: a colour, an icon or an image.      │
│                                                               │
│  Colour ──────────────────────────────────────────────────    │
│   ●  ●  ●  ●  ●  ●  ●  ●  ●  ●                                │
│   ✓                                                           │
│   Red                                                         │
│                                                               │
│  Icon ────────────────────────────────────────────────────    │
│   Icons from          [ Current theme (Papirus)        ▾ ]    │
│   Follows the icon theme when you change it.                  │
│   [ 🔍 Search icons                                      ]    │
│                                                               │
│   Recent                                                      │
│   ▣ ▣ ▣                                                       │
│   Places                                                      │
│   ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣                                         │
│   Purpose                                                     │
│   ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ▣ ...    │
│   Showing 96 of 142. Search to narrow the list.               │
│   Browse icon themes ›                                        │
│                                                               │
│  Image ───────────────────────────────────────────────────    │
│   ┌────┐                                                      │
│   │ 40 │  Custom image                      [ Change... ]     │
│   └────┘                                                      │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

Widgets, top to bottom.

| Element | libcosmic widget | Notes |
|---|---|---|
| Title | `context_drawer(..).title(fl!("customize-folder-title"))` | Same noun as the menu |
| Clear | `.actions(button::standard(fl!("clear")))` | Enabled when any selected folder has a stored look |
| Large preview | `icon::icon(handle).size(64)` | Through `folder_icon_with_look`, as today |
| Small previews | `row![icon 32, text "List", icon 16 symbolic, text "Sidebar"]` | Sidebar uses `folder_icon_symbolic`. Colours show as plain there, on purpose |
| State caption | `text::caption` | See the states table |
| One-look line | `text::caption` in the secondary text colour | Static |
| Colour section | `settings::section().title(fl!("folder-colour"))` holding a `flex_row` of `button::icon(..).selected(..)` wrapped in `tooltip` | Add a 12 px check overlay on the selected swatch with `widget::container` stacking, or `widget::layer_container` if the fork has it |
| Colour caption | `text::body(label)` | Selected colour's name, or "No colour" |
| Icons from | `settings::item::builder(fl!("icons-from")).control(dropdown)` | Option 0 label takes the active theme's name |
| Helper line | `text::caption` | Changes with the dropdown |
| Search | `search_input` | Enter with exactly one match applies it |
| Recent | `flex_row` under `text::heading` | Up to 8. Hidden when empty |
| Places, Purpose | `flex_row` under `text::heading` | Sorted by friendly label. `folder` first in Places as "Default folder" |
| Count footer | `text::caption` | Hidden when all matches fit |
| Browse icon themes | `button::link` or `button::text` with trailing `go-next-symbolic` | Opens `ContextPage::IconThemes`. The gallery's back action must return here, so `folder_appearance` stays in memory |
| Image section | `settings::item::builder` with a 40 px `button::icon(from_path(image)).selected(true)` as the leading element and `button::standard("Change...")` | With no image: text "Use your own picture" and `button::standard("Browse...")` |

### 2.3 Copy strings

Replace the "Customize folder" block in `i18n/en/cosmic_files.ftl:319-344`.

```ftl
## Customize folder
customize-folder = Customize folder...
customize-this-folder = Customize this folder...
customize-folder-title = Customize folder
folders-selected = {$count} folders
one-look-rule = A folder shows one look: a colour, an icon or an image.
look-none = No custom look
look-mixed = Mixed looks. Picking one applies it to all {$count}.
look-colour = {$colour} folder · follows the icon theme
look-icon-follows = {$icon} · follows the icon theme
look-icon-pinned = {$icon} from {$theme}
look-icon-pinned-missing = {$icon} from {$theme}, which isn't installed. Showing the current theme's icon.
look-image = Custom image
preview-list = List
preview-sidebar = Sidebar
folder-colour = Colour
colour-none = None
no-coloured-folders = This icon theme has no coloured folders.
folder-icon = Icon
icons-from = Icons from
icons-from-current = Current theme ({$theme})
icons-follow-theme = Follows the icon theme when you change it.
icons-pinned-to = Stays on {$theme} when you change the icon theme.
search-icons = Search icons
no-matching-icons = No matching icons
icon-group-recent = Recent
icon-group-places = Places
icon-group-purpose = Purpose
icons-shown = Showing {$shown} of {$total}. Search to narrow the list.
browse-icon-themes = Browse icon themes
folder-image = Image
use-own-picture = Use your own picture
choose-image = Choose image
browse = Browse...
change = Change...
choose = Choose
images = Images
clear = Clear
icon-default-folder = Default folder
colour-red = Red
colour-orange = Orange
colour-yellow = Yellow
colour-green = Green
colour-cyan = Cyan
colour-blue = Blue
colour-violet = Violet
colour-magenta = Magenta
colour-brown = Brown
colour-grey = Grey
```

Friendly icon labels are not translated in this round. They derive from the raw name, and
a translator cannot know every pack's names. The explicit map below covers the common ones.

| Raw name | Label | Search synonyms |
|---|---|---|
| `folder` | Default folder | plain, default |
| `folder-documents` | Documents | docs |
| `folder-download`, `folder-downloads` | Downloads | |
| `folder-music` | Music | audio |
| `folder-pictures` | Pictures | photos, images |
| `folder-videos` | Videos | movies |
| `folder-publicshare`, `folder-public` | Public | shared |
| `folder-templates` | Templates | |
| `folder-desktop`, `user-desktop` | Desktop | |
| `user-home` | Home | |
| `folder-git`, `folder-github` | Git, GitHub | repo, source |
| `folder-development`, `folder-code` | Code | dev, programming, source |
| `folder-projects` | Projects | work |
| `folder-games` | Games | |
| `folder-cloud`, `folder-dropbox`, `folder-google-drive` | Cloud, Dropbox, Google Drive | sync |
| `folder-locked` | Locked | private, secure |
| `folder-favorites` | Favourites | starred |
| `folder-important` | Important | urgent |
| `folder-recent` | Recent | |
| `folder-script` | Scripts | shell |
| anything else | Strip `folder-`, `folder_`, `user-`. Replace `-` and `_` with spaces. Capitalise the first letter | none |

### 2.4 States

| State | Caption | Colour row | Icon grid | Image row | Clear |
|---|---|---|---|---|---|
| No look | "No custom look" | None selected. Caption "No colour" | None selected | "Use your own picture", Browse... | Disabled |
| Colour | "Red folder · follows the icon theme" | Red selected with check | None selected | Browse... | Enabled |
| Icon, follows | "Git · follows the icon theme" | None | Git selected. Dropdown on "Current theme (X)" | Browse... | Enabled |
| Icon, pinned | "Git from Papirus" | None | Dropdown on Papirus. Helper "Stays on Papirus..." | Browse... | Enabled |
| Icon, pinned theme missing | "Git from Papirus, which isn't installed. Showing the current theme's icon." | None | Dropdown on "Current theme" | Browse... | Enabled |
| Image | "Custom image" | None | None | Thumbnail selected, Change... | Enabled |
| Image file missing | "Custom image (file missing)" | None | None | "Use your own picture", Browse... | Enabled |
| Mixed, 3 folders | "Mixed looks. Picking one applies it to all 3." | None | None. Grid uses the plain `folder` base | Browse... | Enabled |
| Raster-only theme | as above | Hidden. Text "This icon theme has no coloured folders." | as above | as above | as above |
| Search with no match | as above | as above | "No matching icons" | as above | as above |

### 2.5 Interactions

| Action | Result |
|---|---|
| Click a swatch, grid icon or image | Applies at once to every selected folder. The preview, the tab and the sidebar redraw. No OK button |
| Click the selected swatch again | Nothing. Clearing is the Clear button or the menu's None. Finder's toggle-off on a tag dot is a tag behaviour, not a colour behaviour, and a toggle on a swatch is easy to hit by accident |
| Change "Icons from" | Reloads the grid from that theme. Does not change the folder until an icon is clicked |
| Type in search | Filters by label, raw name and synonyms. Groups that empty out are hidden |
| Enter in search with one match | Applies that icon |
| Clear | Removes the look from every selected folder. Button disables |
| Esc | Closes the drawer |
| Switch icon theme while the drawer is open | `icon_theme_changed` already calls `page.refresh()`. The dropdown's option 0 label must update too |
| Pick an image | File dialog, then import, then apply. On import failure the caption reads the error for 5 seconds, then reverts |
| Browse icon themes | Opens the gallery drawer. Its back action returns to this drawer with state intact |

## 3. Code changes

Sizes: S is under an hour, M is a half day, L is a day or more.

| # | Change | File and function | Size |
|---|---|---|---|
| 1 | Add `Action::SetFolderColour(Option<&'static str>)` and `Message::SetFolderColour(Option<Entity>, Option<&'static str>)`. Handler collects `selected_paths` that are dirs and calls `set_folder_looks` | `src/app.rs`: `enum Action`, `Action::message`, `enum Message`, `App::update` | S |
| 2 | Build the "Folder colour" submenu: `menu::Item::Folder(fl!("folder-colour"), children)` with one `menu::Item::Entry(Entry::new(label, action).icon(handle).checked(..))` per colour, a divider and "None". Compute the shared colour from `folder_look::stored_look` over the selected paths | `src/menu.rs`: `context_menu`, next to the `customize-folder` push | M |
| 3 | Move "Folder colour" and "Customize folder..." into their own divider group above "Add to sidebar" | `src/menu.rs`: `context_menu` | S |
| 4 | Add "Customize this folder..." to the no-selection branch for `Mode::App` and `Location::Path`. Reuse `Action::CustomizeFolder`; the handler falls back to the tab's location when nothing is selected | `src/menu.rs`: `context_menu` else branch. `src/app.rs`: `Message::CustomizeFolder` | S |
| 5 | Move `colour_label` to `folder_look.rs` as `pub fn colour_label(id) -> String` so the menu and the drawer share it | `src/folder_appearance.rs`: `colour_label`. `src/folder_look.rs` | S |
| 6 | Add `pub fn icon_label(name: &str) -> String` and `pub fn icon_search_terms(name: &str) -> Vec<&'static str>` with the table in 2.3 | `src/icon_themes.rs`, new functions next to `folder_icon_names` | M |
| 7 | Add `pub fn icon_group(name: &str) -> IconGroup` returning Places or Purpose | `src/icon_themes.rs` | S |
| 8 | Replace `icons: Vec<(String, Handle)>` with grouped vectors plus `total_matches: usize`. Filter on label, raw name and synonyms. Sort each group by label. Keep the 96 cap but count before capping | `src/folder_appearance.rs`: struct fields, `refresh` | M |
| 9 | Render groups with headings, the "Showing N of M" footer, and tooltips that show `icon_label` | `src/folder_appearance.rs`: `view` | S |
| 10 | Rename the dropdown to "Icons from", fill option 0 with the active theme's display name, add the helper caption that changes with `icon_set`. Update option 0 in `refresh` so a theme switch relabels it | `src/folder_appearance.rs`: `new`, `refresh`, `view` | S |
| 11 | Add `enum Shared { None, Mixed, Look(FolderLook) }` returned by `current_look`, and `fn caption(&self) -> String` producing the 2.4 strings. Use the plain `folder` base for the grid when `paths.len() > 1` | `src/folder_appearance.rs`: `current_look`, `base_name`, new `caption` | S |
| 12 | Detect a pinned theme that is not installed in `new`, store it, and show the "isn't installed" caption | `src/folder_appearance.rs`: `new`, `caption` | S |
| 13 | Replace "Reset to default" destructive button with "Clear" as `button::standard` in `context_drawer(..).actions()`. Keep `on_press_maybe` | `src/app.rs`: `ContextPage::FolderAppearance` arm in the context drawer builder. `src/folder_appearance.rs`: `view` | S |
| 14 | Add the 32 px and 16 px previews. The 16 px one calls `tab::folder_icon_symbolic` | `src/folder_appearance.rs`: `refresh`, `view` | S |
| 15 | Show the selected colour's name under the swatches, and the "no coloured folders" text when `swatches` is empty | `src/folder_appearance.rs`: `view` | S |
| 16 | Add a check overlay on selected swatches and grid icons | `src/folder_appearance.rs`: `view`, a small helper `selected_icon_button` | M |
| 17 | Image row: 40 px thumbnail via `icon::from_path`, "Custom image" text, "Change..." when set, "Use your own picture" and "Browse..." when not. Detect a missing file with `Path::exists` | `src/folder_appearance.rs`: `view` | S |
| 18 | Enter in the search field applies the single match | `src/folder_appearance.rs`: `view` `.on_submit(..)`. `src/app.rs`: new `Message::FolderLookSearchSubmit` | S |
| 19 | Esc closes the open context drawer, if not already bound | `src/app.rs`: key handling in `update` or `subscription` | S |
| 20 | Recent picks: `recent_folder_looks: Vec<FolderLook>` in `Config`, max 8, most recent first, deduplicated. Push in the `FolderLookSet` handler for `Icon` looks only | `src/config.rs`: `Config`. `src/app.rs`: `Message::FolderLookSet`. `src/folder_appearance.rs`: `refresh`, `view` | M |
| 21 | "Browse icon themes" link from the drawer, and a back action on the gallery that returns to `ContextPage::FolderAppearance` when `folder_appearance` is `Some` | `src/folder_appearance.rs`: `view`. `src/app.rs`: `ContextPage::IconThemes` arm `.actions(..)` | S |
| 22 | Rename strings per 2.3 and drop `reset-folder-appearance`, `folder-appearance`, `icon-set`, `icon-set-current` | `i18n/en/cosmic_files.ftl` and the other locales | S |
| 23 | Later: combine colour and icon. `FolderLook` becomes a struct or gains a `Both` variant. `folder_handle` draws the theme's `folder-<colour>-<kind>` when the icon is an XDG kind, otherwise recolours the icon's SVG. Needs a config migration from the enum | `src/folder_look.rs`: `FolderLook`, `folder_handle`, `colour_candidates`. `src/config.rs` | L |
| 24 | Later: keep the original image file name. `FolderLook::Image` gains a `name` field with a serde default, so old configs still load | `src/folder_look.rs`: `FolderLook`, `import_image`. `src/folder_appearance.rs`: `view` | M |

Suggested order: 1 to 5 first, since the submenu is the biggest win for the least code.
Then 10 to 13 and 22, which are the copy and state fixes. Then 6 to 9 for the grid. Items
14 to 21 are polish. 23 and 24 wait for user feedback on whether people want colour plus
icon.
