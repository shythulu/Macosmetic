# Panel: software architecture verdicts

Role: architecture judge. Inputs: 01, 02, 05 (features), 03 and 04 (designs in flight, context only). Repo state: master at `018a5ae`. Written 2026-10-09.

## Ground truth the verdicts rest on

Checked in the repo, not assumed.

| Fact | Consequence |
|---|---|
| `src/app.rs` 7985 lines, `src/tab.rs` 9212, `src/menu.rs` 797, `src/dialog.rs` 2112. Fork already carries +703 in app.rs, +1432 in tab.rs, 36 `cfg(target_os = "macos")` sites in app.rs | Every new line in these files is a rebase tax. Budget them. |
| `src/key_bind.rs` is +1396 vs upstream with its own Cmd table | Effectively fork-owned. Key changes are cheap. |
| Owned modules today: `appkit_macos.rs` (`with_ns_window`, `MainThreadMarker`, NSNotification to `Subscription` via mpsc), `gesture_macos.rs` (NSEvent local monitor), `quicklook_macos.rs` (QLThumbnailGenerator to PNG), `launch_macos.rs`, `folder_look.rs`, `folder_appearance.rs`, `icon_themes.rs`, `icon_theme_gallery.rs`, `gesture.rs` | The patterns for "AppKit object in, iced message out" already exist. New FFI copies them. |
| Deps present: `objc2 0.6`, `block2`, `objc2-app-kit 0.3`, `objc2-foundation`, `objc2-quick-look-thumbnailing`, `objc2-uniform-type-identifiers`, `notify-debouncer-full` (FSEvents), `trash 5.2.6`, `sha2`, `walkdir` | No xattr, no plist, no Spotlight, no DiskArbitration, no NetFS. |
| `tab::Item` already has `metadata`, `mime`, `dir_size: DirSize`, `checksums: ChecksumState`, `cut`, `thumbnail_scale_opt` | There is precedent for lazily computed per-item state. One more field is the right shape for tags, badges, cloud state. |
| `App::footer()` in app.rs renders the operations progress bar and returns `None` otherwise | The status bar has a home. |
| `mounter/` is `gvfs` only | On macOS: no volumes, no Connect to Server, no eject. A `Mounter` impl for macOS is the fix. |
| `mime_app.rs` reads XDG `.desktop` files | "Open With" on macOS is Linux-shaped today. NSWorkspace is the fix. |
| `scan_recents` is the freedesktop recently-used list | Recents on macOS is probably empty. |
| "Open in new window" re-executes the binary (`env::current_exe()`) | One window per process. Anything "across windows" (merge all, drag tab out) is cross-process. Treat as impossible for now. |
| `trash` crate restore is `os_limited` (Linux, Windows) | Put Back on macOS is probably not functional. Needs checking, and if so an own journal. |
| iced/libcosmic: no accessibility tree wired, no NSView hosting, draws the whole window | Anything needing an AppKit view inside the window (NSBrowser, NSGlassEffectView, terminal view) is out. Anything that is its own panel or window (QLPreviewPanel, share picker, NSOpenPanel) or a plain API call is in. |
| Upstream since April: libcosmic context_menu widget, favourite renaming, remote-FS speed detection, i18n. Open issues: column view #77 (60), dual pane #260 (23), tree rows #19 (26), sort by kind #1262 (20), reflink #980, network stall #2084 | Upstream moves slowly on views. Cross-platform items with big reaction counts are the ones they will eventually land. |

## 1. Enabling infrastructure

Six pieces. Each one is a Macosmetic-owned module, each unlocks several features, and together they cap the number of times `app.rs`, `tab.rs` and `menu.rs` get touched.

### I1. `src/macos/` bridge directory

**What.** Move the four `*_macos.rs` files into `src/macos/` (`appkit.rs`, `gesture.rs`, `quicklook.rs`, `launch.rs`) and add `src/macos/bridge.rs` with the shared primitives that are currently private to `appkit_macos.rs`:

| Primitive | Source today | Used by |
|---|---|---|
| `on_main(window_id, FnOnce(&NSWindow, &NSView) -> T) -> Task<M>` | `with_ns_window` | Share picker anchor, QL panel, colour space, drag-out |
| `notification_subscription(name) -> Subscription<Event>` | `watch_activation` / `activation_subscription` | Volume mount/unmount, screen change, appearance change |
| `block_once<T>(…) -> oneshot::Receiver<T>` | porting notes 6.2, not yet written | Every completion-handler API: NetFS mount, QL generation, NSMetadataQuery |
| `MainThreadMarker` guards and the "nothing in here touches app state" rule | module doc | all |

**Why.** Four modules already reimplement bits of this. Features in 02 that need AppKit (Share, AirDrop, Quick Actions, Quick Look panel, NSWorkspace, DiskArbitration) are each small once the bridge exists, and each becomes a 300-line file with no hand-rolled main-thread plumbing.

**Unlocks.** Share/AirDrop, Quick Actions, QL panel, Open With, eject, mount events, folder-icon rendering of SF Symbols.

**Cost.** One commit; touches `lib.rs` only among shared files. Do it first, alone, so nobody rebases across the rename.

### I2. Metadata layer: `src/macos/metadata.rs` (xattr + flags, pure Rust)

**What.** One struct per file read without AppKit:

```rust
pub struct FileExtras {
    pub tags: Vec<Tag>,                 // com.apple.metadata:_kMDItemUserTags, bplist of "Name\n<colour>"
    pub finder_comment: Option<String>, // com.apple.metadata:kMDItemFinderComment
    pub locked: bool,                   // st_flags & UF_IMMUTABLE
    pub hidden_extension: bool,         // FinderInfo bit 0x0010
    pub is_alias: bool,                 // FinderInfo kIsAlias, or isAliasFileKey
    pub is_package: bool,               // bundle bit or extension table
    pub dataless: bool,                 // st_flags & SF_DATALESS (iCloud / File Provider placeholder)
    pub folder_customization: Option<FolderCustomization>, // com.apple.icon.folder#S JSON
}
```

Read with the `xattr` crate plus `plist` for the binary plists, `libc::stat` for `st_flags`. Writes the same way for tags, comment, locked, hidden extension. No main thread, no AppKit, fully unit-testable against a temp dir on APFS.

**Why.** Six features in 02 are "read or write one xattr". Doing them one at a time means six parsers and six `tab.rs` edits. Doing the layer once means one `Item` field and six consumers.

**Unlocks.** Tags, Finder comments, Locked, Hide extension, alias badge, package detection, cloud placeholder badge, Tahoe folder customisation read, rule labels (Q58).

**Deps.** `xattr` (tiny, pure), `plist`. Both static, dylib graph stays clean.

### I3. `Item.extras` slot and a background metadata worker

**What.** One field on `tab::Item`: `pub extras: ItemExtras` (type defined in an owned module, `Default`, filled lazily). One new message `Message::ItemExtras(PathBuf, ItemExtras)`. One worker, `src/metadata_worker.rs`, modelled on `thumbnail_cacher.rs`: bounded queue, visible-items-first, cancelled on navigation, honours `tab::is_protected_tree`, never retries `EPERM` (porting notes 4.3). It computes whatever is expensive per item: `FileExtras`, directory sizes, media dimensions, git status.

**Why.** `dir_size`, `checksums` and `thumbnail_opt` are three separate lazy pipelines in `tab.rs` today. A fourth, fifth and sixth (tags, sizes-in-list, dimensions) would each add a field, a message and a scan hook. One slot and one worker turns that into one `tab.rs` edit, ever.

**Unlocks.** Folder sizes in list view, show item info, media columns, tags column, cloud badges, git tints, rule labels.

**Where it touches shared files.** `tab.rs`: struct field, the three `item_from_*` constructors, one render call for badges, one message arm. That is the whole budget.

### I4. Sub-message delegation for owned features

**What.** New features add one `Message` variant and one `update` arm to `app.rs`, not ten. `Message::Tags(tags::Message)`, `Message::IconTheme(icon_theme::Message)`, and the owned module exposes `update(&mut State, Message) -> Task<app::Message>` and `view(&State) -> Element`.

**Why.** Design 04 adds ten `Message` variants and a handler table to `app.rs`. Design 03 adds four more. At that rate `app.rs` grows by a few hundred fork lines per feature, and every upstream change to the `Message` enum or the `update` match conflicts. Nested messages shrink each feature's footprint in `app.rs` to about ten lines. This is the single most effective merge-cost control available, and it is free to adopt in 03 and 04 now.

**Flag for 03 and 04.** Both are specified as flat `Message` variants. Recommend converting before implementation; it changes no behaviour.

### I5. macOS menu extension point

**What.** One hook in `menu.rs::context_menu` and one in `menu_bar`: `#[cfg(target_os = "macos")] items.extend(crate::macos::menu::context_items(&selection, &tab))`. The owned `src/macos/menu.rs` builds Tags, Share, AirDrop, Quick Actions, Make Alias, Show Package Contents, Copy Path variants, Open in Terminal/Editor.

**Why.** Every feature in 02 §7 wants a context menu entry. Without this, each one is a separate `cfg` block in `menu.rs`, and `menu.rs` is the file upstream just rewrote around libcosmic's new `context_menu` widget. Two hooks, added once.

### I6. Spotlight, Workspace and Mounter providers

Three thin adapters, each behind an existing seam so `tab.rs` and `app.rs` swap providers with one `cfg`:

| Adapter | Owned module | Seam it plugs into | First implementation |
|---|---|---|---|
| Spotlight | `src/macos/spotlight.rs` | `tab::scan_search`, `tab::scan_recents` | Spawn `/usr/bin/mdfind -0 -onlyin <dir> <query>` and `mdfind -onlyin ~ 'kMDItemLastUsedDate > $time.today(-30)'`. Zero FFI, cancel by killing the child. `NSMetadataQuery` for live results later. |
| Workspace | `src/macos/workspace.rs` | `mime_app.rs` app lookup; new eject/open calls | `NSWorkspace URLsForApplicationsToOpenURL:`, `setDefaultApplicationAtURL:toOpenContentType:`, `iconForFile:`, `unmountAndEjectDeviceAtURL:`, `NSURL bookmarkData` for aliases. Foundation calls, no main thread needed except icons. |
| Mounter | `src/mounter/macos.rs` | the `Mounter` trait in `mounter/mod.rs` | `FileManager.mountedVolumeURLs` + DiskArbitration callbacks for the Locations list and eject; `NetFSMountURLAsync` for Connect to Server; Bonjour via `NWBrowser` later. |

**Why.** Three whole feature areas (search, open-with, network/volumes) are Linux-shaped or dead on macOS today. They are not features to add, they are platform gaps, and the fix for each is a provider behind a seam upstream already has.

**Deps.** None for Spotlight v1. Workspace: already in `objc2-app-kit`. Mounter: `objc2-disk-arbitration` (exists in the objc2 family) and NetFS via `extern "C"` declarations (two functions).

### Sequencing of infrastructure

```
I1 bridge  ──┐
I2 metadata ─┼─► I3 Item.extras + worker ──► tags, sizes, badges, cloud, media
I4 delegation (adopt in 03/04 now)
I5 menu hook ──► every context-menu feature
I6a Spotlight ──► search, recents, smart folders, tag search
I6b Workspace ──► open with, eject, alias, packages
I6c Mounter ──► volumes, connect to server, saved servers
```

I1, I2, I4, I5 are independent and can be built in parallel by four agents with no shared files except `lib.rs` (I1) and `menu.rs` (I5). I3 waits for I2. I6a/b/c are independent of each other.

## 2. Verdicts

Duplicates merged: 02 rows and 05 ideas that describe the same thing appear once, with both references. Effort is S under a week, M one to three weeks, L a month, XL more or needs a native target. "Shared" means app.rs / tab.rs / menu.rs / dialog.rs / operation.

| Feature (source) | Verdict | Effort | Where it lives | Depends on | Reason |
|---|---|---|---|---|---|
| **Views** | | | | | |
| Column view (02 §1, Q34, upstream #77) | ADD-LATER | L | new `column_view.rs`; tab.rs `View::Column` + dispatch | nothing | Most-wanted feature, but all in tab.rs and upstream's most-reacted issue. Build as an owned renderer over `&[Item]`; keep tab.rs touches to the enum, one view arm, one key arm. Check for an upstream PR before starting. |
| Expandable tree rows in list (02, Q35, #19) | WAIT-FOR-UPSTREAM | M | tab.rs list layout | nothing | Pure cross-platform, deep in list rendering, 26 reactions upstream. Diverging here costs every rebase. |
| Per-folder view memory: sort, view, zoom, columns (02 View Options, Q16) | ADD-LATER | M | new `folder_view_memory.rs`; config.rs; small hooks where `sort_names` is read | nothing | Upstream has `sort_names` per path; extend the same seam to a struct. Never read or write `.DS_Store`. |
| Read Finder `.DS_Store` positions / view options | IGNORE | M | - | - | Private format, read-only value, and icon positions do not fit iced's flow grid. |
| Free icon arrangement, Clean Up | IGNORE | XL | tab.rs grid | - | iced grid is a flow layout; absolute positions mean a new widget and per-folder position store. |
| Folder sizes in list view, cached (02 Calculate all sizes, Q23) | ADD-LATER | M | `metadata_worker.rs`; tab.rs size column reads `Item.extras` | I3, status bar | `DirSize` exists for the preview pane; move the computation into the worker so list view and status bar share it. |
| Sort by kind and extension (Q4, #1262), Date Created, Date Added | ADD-NOW (kind/ext), ADD-LATER (dates) | S / S | tab.rs `HeadingOptions` + comparator; menu.rs sort entries | I2 for Date Added | One enum variant and a comparator. Upstream may land kind sort; the conflict is a few lines. Date Added needs `addedToDirectoryDate` from the metadata layer. |
| Group By (Use Groups) | ADD-LATER | M | tab.rs list/grid section headers | sort by kind | Section headers in both views; moderate tab.rs change. Low demand in the brainstorm signal. |
| Show item info second line (02) | ADD-LATER | S | tab.rs grid label; worker computes dims/count | I3 | Cheap once the worker exists. |
| Hidden-junk rules: always hide `.DS_Store`, `._*`, `.localized` (Q18); Show Library toggle (02) | ADD-NOW | S | new `hidden_rules.rs` (pure, tested); one line in tab.rs where `hidden` is set; config.rs toggle | nothing | Daily pain, one hook. |
| Media columns: dimensions, duration, EXIF date (Q38) | ADD-LATER | M | worker + `mdls`/MDItem adapter; tab.rs column | I3, I6a | Spotlight already has the data; read it, do not decode media. |
| Flat view of a subtree (Q48) | ADD-LATER | S | tab.rs: `Location::Search` with recursive empty query | nothing | Mostly exists as recursive search; expose it as a menu item. |
| Preview pane: syntax-highlighted code, Markdown, PDF (Q43) | ADD-LATER | M | `quicklook_macos.rs` already renders PDF/Markdown via QL; code needs `syntect` | nothing | QL covers PDF and Markdown today. Code highlighting is a new dep and a text widget; moderate value. |
| Preview pane metadata list + Quick Action buttons (02) | ADD-LATER | M | app.rs preview pane; `macos/spotlight.rs` for `mdls` keys | I6a, Quick Actions | Preview pane is app.rs territory; keep the key list in an owned module. |
| **Window chrome** | | | | | |
| Status bar: item count, selection count and size, free space (02, Q5, #1766) | ADD-NOW | S | new `status_bar.rs` (pure view fn, tested); app.rs `footer()` composes it with the progress bar | nothing | Prerequisite for showing folder sizes and selection size. Free space via `NSURL volumeAvailableCapacityForImportantUsage` matches Finder's number; `statvfs` does not. |
| Clickable breadcrumb path bar, Option-click copies path (02) | ADD-LATER | S | tab.rs `location_view` | copy path variants | Ancestor menu exists; upstream owns `location_view`. Low marginal value. |
| Locations sidebar: volumes, network, CloudStorage domains (02) | ADD-LATER | M | `mounter/macos.rs`; `~/Library/CloudStorage` enumeration | I6c | Dead on macOS today because mounter is gvfs-only. |
| Import Finder favourites (sfl3) | IGNORE | S | - | - | Private keyed-archive format; own favourites exist. |
| Sidebar drag-reorder and rename (Q32) | WAIT-FOR-UPSTREAM | - | - | - | Upstream landed rename (7cebbef); reorder is present. |
| Reuse an existing tab for an already-open folder (Q29) | ADD-NOW | S | app.rs open-folder handler | nothing | A lookup before `open_tab`. |
| Merge all windows, drag tab out (02 Tabs, Q56) | IGNORE | XL | - | - | Windows are separate processes. Would need in-process multi-window first. |
| Toolbar customisation (02) | IGNORE | M | - | - | libcosmic header bar is not a drag editor; low demand. |
| Liquid Glass chrome (02 §2, §11) | IGNORE | XL | - | - | Needs `NSGlassEffectView` under an iced surface that paints the whole window. Wait for libcosmic to grow a translucent window mode. Set `NSRequiresAquaSystemAppearance=NO` is already done. |
| Dual pane with F5/F6 (Q15, #260); multi-pane (Q57); folder compare (Q51) | ADD-LATER (dual); IGNORE (multi, compare) | L / - | app.rs: pane model, active-pane resolution for every `entity_opt` | a `Pane` abstraction design | The second-most-named reason people leave Finder, but it changes how app.rs resolves "the current tab" everywhere. Design the pane abstraction before coding; do not start before the Batch A infrastructure is merged. |
| Restore tabs, windows, scroll on launch (Q14) | ADD-LATER | M | new `session.rs` (JSON in config dir); app.rs init/exit hooks | nothing | Multi-process windows complicate it: only the first process restores, children get a flag. |
| **Get Info and per-file attributes** | | | | | |
| Get Info fields: Kind, Where, Created, comment, tags (02) | ADD-LATER | M | app.rs preview pane reads `Item.extras` | I2, I3 | The preview pane is the Get Info window; feed it from the layer. |
| Spotlight comments read/write (02) | ADD-LATER | S | `macos/metadata.rs`; preview pane field | I2 | One xattr, one text field. |
| Open With submenu + Change All (02) | ADD-NOW | M | `macos/workspace.rs`; `mime_app.rs` cfg swap of the app-list source | I1 | Open With is XDG-shaped on macOS today. Test `setDefaultApplication(toOpen: UTType)` for the permErr report before promising Change All. |
| Sharing & Permissions: ACLs (02) | ADD-LATER | M | new `acl_macos.rs` via `libc` acl(3); permissions UI in app.rs | nothing | POSIX bits exist; ACL editing is niche. |
| Open or edit as administrator (Q55, 02 privileged) | IGNORE | XL | - | - | SMAppService + XPC helper + Developer ID. Not before signing is solved. |
| Locked flag (02) | ADD-LATER | S | `macos/metadata.rs` `chflags`; menu entry via I5 | I2, I5 | Pure Rust. Also render the lock badge from `Item.extras`. |
| Hide extension (02) | ADD-LATER | S | `macos/metadata.rs` FinderInfo bit; tab.rs display-name honours it | I2 | Must affect display and rename stem selection. |
| Stationery pad (02) | IGNORE | S | - | - | Nobody uses it; FinderInfo bit only. |
| Custom icon via `NSWorkspace setIcon` (02) | ADD-LATER | S | `macos/workspace.rs`; `folder_appearance.rs` "Also apply in Finder" | I1, 03 shipped | See the 03 concern below: folder looks live in app config, Finder never sees them. `setIcon` is the opt-in bridge. |
| Read Tahoe folder customisation `com.apple.icon.folder#S` (02 §3, §11) | ADD-LATER | M | `macos/metadata.rs` parse; `folder_look.rs` extra look source; SF Symbol rasterised via `NSImage(systemSymbolName:)` to PNG like `quicklook_macos` does | I2, I1 | Lets folders the user customised in Finder look the same here. Verify whether `iconForFile:` already returns the composite (02 §12); if so, skip the symbol rendering. |
| **Tags, Spotlight, Smart Folders** | | | | | |
| Finder tags: read/write, coloured dots, column, Tags submenu, tag sidebar + Ctrl-1..7 search (02 §4, Q40) | ADD-LATER (first consumer of I2/I3) | M | `macos/tags.rs` (model, colours, write); dots via `Item.extras` in tab.rs; submenu via I5; sidebar search via I6a | I2, I3, I5, I6a | Breaking a user's existing tag organisation is the one thing a Finder replacement cannot do. Finder's global tag list is a private plist: read it, never write it. |
| Spotlight search in window, Kind/Date filters (02, Q41) | ADD-LATER | M | `macos/spotlight.rs`; tab.rs `scan_search` cfg swap | I6a | Current search walks the tree. `mdfind -onlyin` is faster and content-aware. Default to name match in the current folder, which is exactly the Finder default people complain about. |
| Recents from Spotlight (02) | ADD-LATER | S | `macos/spotlight.rs`; tab.rs `scan_recents` cfg swap | I6a | Recents is a freedesktop list today, so it is empty on macOS. |
| Smart Folders (`.savedSearch`) (02) | ADD-LATER | M | `macos/spotlight.rs` RawQuery + scopes; tab.rs open `.savedSearch` as a search location | I6a | Parse `RawQuery` only; the criteria editor is Finder-private, build none at first. |
| Quick filter: type to narrow in place (Q6) | ADD-LATER | M | tab.rs `TypeToSearch` mode | nothing | A fourth mode in an upstream enum; upstream might add it. Moderate conflict. |
| Fuzzy Go to Folder with frecency (02, Q37) | ADD-LATER | M | new `path_complete.rs` (pure, tested); `session.rs` frecency; tab.rs `edit_location` hook | session store | Good iced fit, owned engine, one hook. |
| Command palette Cmd+Shift+P (Q36) | ADD-LATER | M | new `command_palette.rs` over `Action` + `key_bind` labels; app.rs overlay arm | I4 | `key_bind.rs` already renders Cmd glyphs; the palette is a list over the `Action` enum. |
| **Aliases, symlinks, packages** | | | | | |
| Symlink and alias badge; Make Symlink; Make Alias; Show Original (02 §5, Q11) | ADD-NOW (badges, make symlink), ADD-LATER (alias) | S / S | badge: tab.rs render from `metadata.file_type()` and `Item.extras.is_alias`; alias: `macos/workspace.rs` `NSURL bookmarkData`; menu via I5 | I3 for alias badge, I1 for alias | Upstream #466 has 17 reactions; a badge is a few lines and worth the small conflict. |
| Packages shown as files, double-click launches, Show Package Contents (02) | ADD-NOW | M | `macos/metadata.rs` `is_package`; tab.rs `item_from_entry` treats packages as files; `macos/workspace.rs` icon; menu via I5 | I2, I6b | `/Applications` renders as folders today. This is the most visible "not a Mac app" defect for a Finder replacement. |
| **Drag and drop** | | | | | |
| Drag modifiers: Option copy, Cmd move, Cmd+Option alias (02) | ADD-LATER | S | app.rs drop handler reads winit modifiers | nothing | Modifier state is already available; check what the default is across volumes. |
| Drag out to other apps, file promises (02) | ADD-LATER | L | `macos/drag.rs`: `beginDraggingSession` from the content view with the captured mouse-down `NSEvent` | I1 | iced has no drag-out on macOS. Doable with the local event monitor pattern from `gesture_macos.rs`, but it fights iced's own drag layer. Needed before a Shelf is worth much. |
| Spring-loaded folders (02, Q46) | exists | - | - | - | `dnd_hovered` with a timer is already there. |
| Desktop icons and Stacks (02 §6, §10) | IGNORE | XL | - | - | Finder-internal; no public API. |
| **Actions** | | | | | |
| Quick Look panel (Space) via `QLPreviewPanel` (02) | ADD-LATER | M | `macos/quicklook_panel.rs` | I1 | Panel asks the responder chain for `acceptsPreviewPanelControl:`; winit's window does not answer. Options: isa-swizzle the `NSWindow` subclass at runtime (risky), or spawn `qlmanage -p` (S, zero FFI, works today). The in-app gallery already covers the common case. |
| Quick Actions and Services (02 §7, §9) | ADD-LATER | M | `macos/services.rs`: list via `pbs -dump_pboard`, invoke via `NSPerformService`; menu via I5; extends `context_action.rs` | I1, I5 | Markup, Rotate, Create PDF are Finder-internal and do not come along. Shortcuts marked "Use as Quick Action" do. |
| Share menu and AirDrop (02) | ADD-LATER | S | `macos/share.rs`: `NSSharingServicePicker` anchored on the content view, `NSSharingService(.sendViaAirDrop)`; menu via I5 | I1, I5 | Two calls once the bridge exists. Not testable; keep it thin. |
| Compress matching Finder (`ditto`, `__MACOSX`) (02) | IGNORE | S | - | - | Zips without resource-fork folders are a feature. |
| Duplicate with " copy" naming (02) | ADD-LATER | S | `operation/mod.rs` `copy_unique_path` | nothing | Shared file; a naming tweak. |
| APFS `clonefile` for same-volume copy (Q3, #980) | ADD-NOW | S | `operation/recursive.rs` `copy`: `cfg(macos)` try `libc::clonefile`, fall back | nothing | Finder copies are instant; a byte copy looks broken. One insertion; testable on APFS tmp. Upstream #980 is for reflinks generally; a macOS-only branch merges cleanly. |
| Batch rename with preview, regex, counters, metadata tokens (02, Q22) | ADD-LATER | M | new `batch_rename.rs` (pure engine, tested); dialog in app.rs via I4; `Operation::Rename` exists | I4; worker for EXIF tokens | Upstream #689 (16) may land a basic one. Own the engine; let the UI be the only thing that could conflict. |
| New Folder with Selection (02, Q45) | ADD-NOW | S | app.rs: mkdir then `Operation::Move`; menu entry | nothing | Two existing operations chained. |
| Select by pattern, invert, select same extension (Q13) | ADD-NOW | S | new `selection_pattern.rs` (glob, tested); tab.rs three selection fns; menu entries | nothing | Pure matcher, small hooks. |
| Connect to Server (NetFS), saved servers, Bonjour (02, Q33) | ADD-LATER | L | `mounter/macos.rs` | I6c | Dead on macOS today. NetFS is two C functions; DiskArbitration is in objc2's family. Depends on the stall fix below to be pleasant. |
| Eject / unmount, Eject When Finished (02, §11) | ADD-LATER | S | `mounter/macos.rs` | I6c | Comes with the mounter. |
| Never freeze on a stalled mount (Q25, #2084) | WAIT-FOR-UPSTREAM; ADD-NOW the macOS half | M / S | upstream: timeout model in scan; ours: `statfs` `MNT_LOCAL` in the existing remote-FS detection | nothing | Upstream added remote detection for SFTP; the macOS detection branch is one `cfg`. The general off-thread-with-timeout model is theirs to build. |
| Burn (02) | IGNORE | - | - | - | Hardware is gone. |
| Undo multi-level and persistent (02, Q21) | ADD-LATER | M | `session.rs` persists upstream's `EditHistory` | session store | Upstream has the stack; persistence is ours. |
| Move to Trash (02) | exists | - | - | - | `trash` crate calls `trashItemAtURL` on macOS. |
| Put Back for items trashed here (02) | ADD-LATER | M | new `trash_macos.rs` journal keyed by trashed URL; `trash.rs` restore path cfg | nothing | `trash` crate restore is Linux/Windows only, so Restore is probably a no-op on macOS. Verify, then ship an own journal. Finder's `.DS_Store` records: ignore. |
| Trash auto-empty after N days, confirm on trash (Q44) | IGNORE | S | - | - | Finder already does 30 days system-wide; listing `~/.Trash` needs Full Disk Access. |
| **iCloud and File Provider** | | | | | |
| Browse iCloud Drive and CloudStorage (02) | exists | - | - | - | Plain paths. |
| Cloud status badges (02) | ADD-LATER | M | `Item.extras.dataless` from `st_flags` (pure); iCloud detail via `NSURL` ubiquitous keys in `macos/metadata.rs` | I2, I3 | Dataless is enough for a first badge and needs no AppKit. |
| Download Now / Remove Download (02) | ADD-LATER | S | `macos/workspace.rs` `startDownloadingUbiquitousItemAtURL`, `evictUbiquitousItemAtURL`; menu via I5 | I1, I5 | Foundation calls. Evict is iCloud-only. |
| Keep Downloaded pin (02) | IGNORE | - | - | - | Private. |
| **System integration** | | | | | |
| VoiceOver / accessibility tree (02 §9) | WAIT-FOR-UPSTREAM, with an S experiment | S / XL | libcosmic `a11y` feature (AccessKit, has a macOS backend) | nothing | iced has no tree of its own; AccessKit via libcosmic is the only path. Try the feature flag, measure, report. Do not hand-roll. |
| Services menu provider (`NSServices`) (02) | ADD-LATER | S | `res/macos/Info.plist.in`; consumer side is Quick Actions above | Quick Actions | Plist only for "Open in Macosmetic". |
| AppleScript dictionary (02) | IGNORE | XL | - | - | Needs ObjC scripting classes. |
| App Intents / Shortcuts / Spotlight actions (02 §9, §11) | IGNORE | L | - | - | Swift target compiled into the bundle; new toolchain. Revisit when there is a Developer ID and a Swift shim is justified by another feature. |
| Handoff, Focus filters (02) | IGNORE | - | - | - | No demand. |
| Default folder handler (`public.folder`), `NSFileViewer` default (02 §10) | ADD-LATER | S | `macos/workspace.rs` `setDefaultApplication(toOpen: .folder)`; Settings toggle | I6b | Works for many launch paths, documented as flapping. Offer it, label it honestly. |
| Keyboard shortcut parity audit: Cmd+I, Cmd+Y, Cmd+Up, Cmd+Shift+., Ctrl+Cmd+T (02 §10) | ADD-NOW | S | `key_bind.rs` | nothing | Fork-owned file; tests exist for the Cmd table. |
| Cmd+1..9 jump to sidebar favourites (Q9) | ADD-NOW | S | `key_bind.rs`; app.rs one action | nothing | Finder uses Cmd+1..4 for views; pick Cmd+Option+N or make it a setting. |
| Icon Composer `.icon` app icon for Tahoe themes (02 §11) | ADD-LATER | S | `scripts/macos-bundle.sh`, `res/macos` | nothing | Bundle-only. |
| **QoL, developer** | | | | | |
| Cut renders dimmed, Cmd+X/Cmd+V (Q1) | exists; ADD-NOW audit | S | `key_bind.rs` | nothing | `Item.cut` exists; confirm the Cmd mapping and dimming. |
| Copy path variants: POSIX, `~`, shell-quoted, `file://`, name, parent (Q2) | ADD-NOW | S | new `copy_path.rs` (pure, tested); submenu via I5 or menu.rs; app.rs `CopyPath` handler takes a variant | I5 preferred | Pure function, one message. |
| Open in Terminal / editor with a chosen app (Q7, Q8) | ADD-NOW | S | new `macos/open_in.rs`: `open -a <App> <path>`; Settings dropdown fed by `macos/workspace.rs` app list | I6b for the list | `/usr/bin/open` is absolute, so it works from a bundle. |
| New file from templates (Q10) | ADD-LATER | S | new `templates.rs`; menu submenu | nothing | Finder cannot make an empty file at all; low effort. |
| Checksum panel: MD5, compare to clipboard (Q12) | ADD-LATER | S | `ChecksumState` exists in tab.rs; add algorithms | nothing | SHA-256 is there. |
| Folders-first in the View menu (Q17) | ADD-NOW | S | menu.rs `menu_bar` | nothing | Confirm it is in the macOS menu bar. |
| Progress with speed, ETA, current file (Q19, #1283) | WAIT-FOR-UPSTREAM | S | operation/controller.rs | - | 14 reactions, pure cross-platform, in the operation module. |
| Conflict dialog with side-by-side detail (Q20) | WAIT-FOR-UPSTREAM | M | dialog.rs | - | Upstream dialog territory. |
| Operation queue: serialise per disk, reorder (Q24) | WAIT-FOR-UPSTREAM | M | operation/ | - | Cross-platform, operation module. |
| Skip `._*` and `.DS_Store` when copying to FAT/exFAT/SMB (Q26) | ADD-LATER | S | operation/recursive.rs filter; `statfs` fstype check | nothing | Shared file; small. |
| Recent destinations in Move To / Copy To (Q27) | ADD-LATER | S | `session.rs`; menu entries | session store | |
| Recent locations on Back (Q28) | ADD-LATER | S | app.rs: right-click on Back lists `tab.history` | nothing | iced has no long-press; right-click is the equivalent. |
| Rename selects the stem only (Q30, #1451) | ADD-NOW | S | tab.rs rename start | nothing | A few lines; upstream may fix, trivial conflict. |
| Configurable Enter: rename or open (Q31) | ADD-LATER | S | `key_bind.rs`; config.rs | nothing | Fork-owned table. |
| Keybinding editor in Settings (Q42) | ADD-LATER | M | new `keybind_editor.rs`; `key_bind.rs` | I4 | Owned files. |
| Git status tints and branch in path bar (Q39) | ADD-LATER | M | worker + `gix` (pure Rust, no libgit2 dylib) | I3 | Keep the dylib graph clean: no `git2`. |
| Copy folder listing as text, CSV, Markdown (Q47) | ADD-NOW | S | new `listing_export.rs` (pure, tested); menu entry | nothing | Pure function over `&[Item]`. |
| Shelf for collecting files (Q49) | ADD-LATER | M | new `shelf.rs`; app.rs `ContextPage::Shelf` via I4 | I4; drag-out for full value | Good iced fit as a context drawer. Half its value needs drag-out to other apps. |
| Browse archives as folders (Q50) | ADD-LATER | L | `Location::Archive` in tab.rs; archive.rs | nothing | New Location variant across tab.rs. Low priority. |
| Duplicate file finder (Q52) | ADD-LATER | M | new `dedupe.rs` | nothing | Owned, low priority. |
| Image quick ops: rotate, resize, convert (Q53) | ADD-LATER | S | `/usr/bin/sips` via `macos/open_in.rs` pattern | nothing | `sips` ships with macOS and is absolute-path safe. |
| Embedded terminal panel (Q54) | IGNORE | XL | - | - | No NSView hosting, and a terminal widget is a project of its own. |
| Rule labels: "newer than 1 day in bold" (Q58) | ADD-LATER | M | worker + owned rules module | I3, tags | After tags. |

### Concerns on the designs in flight (03, 04)

| Design | Concern | Suggestion |
|---|---|---|
| 03 | Folder looks are stored in app config keyed by path. Finder, Spotlight and the Dock never see them, and a move outside the app orphans the entry (`folder_look::rekey` only sees moves made here). | Keep the design, but name the store as "Macosmetic look" in `FolderLook`'s API so a second source (Tahoe xattr, `setIcon`) can be merged in later. Add an "Also set in Finder" toggle in a later batch via `macos/workspace.rs`. |
| 03 item 23 | Colour plus icon ("Both") is deferred with a config migration. | Decide the data model before more configs accumulate. A struct with optional colour and optional icon now avoids the migration. |
| 03, 04 | Both add flat `Message` variants to app.rs (04 adds ten). | Adopt I4 before implementing: one `Message::FolderLook(..)` and one `Message::IconTheme(..)` arm each. |
| 04 | A second `[patch]` (freedesktop-icons fork) for `reload_themes`. | Fork maintenance is real cost. Acceptable, but a "restart to apply" fallback must stay so the fork can be dropped. |
| 04 | `ureq` adds a TLS stack. | Use the `rustls` feature set. `otool -L` on the binary must stay system-only; add that check to `scripts/test-macos-bundle.sh` now, since 04 is the first network dependency. |
| 04 | Installs to `~/.local/share/icons`. | Fine; `launch_macos` prepends the bundle `share` but keeps `XDG_DATA_HOME` default. Document that Finder-launched and `cargo run` builds see the same user dir. |

## 3. First twelve, in build order

Batch A is infrastructure, not features; it is listed because the twelve depend on it. Agents in the same batch touch disjoint files. Where two agents must both touch `app.rs` or `menu.rs`, the touch is confined to one named function and listed so the integrator can order the merges.

### Batch A: infrastructure (4 agents, parallel)

| Agent | Builds | Files touched | Notes |
|---|---|---|---|
| A1 | I1 bridge: `src/macos/{mod,bridge,appkit,gesture,quicklook,launch}.rs`; I5 menu hook | moves the four `*_macos.rs`; `src/lib.rs` mod list; `src/menu.rs` two hook lines (`context_menu`, `menu_bar`) calling `macos::menu::*`; new empty `src/macos/menu.rs` | Lands first; everyone else rebases once. |
| A2 | I2 metadata layer | new `src/macos/metadata.rs` + tests; `Cargo.toml` (`xattr`, `plist`) | No shared files. |
| A3 | I3 `Item.extras` + worker | `src/tab.rs`: struct field, three `item_from_*` constructors, one `Message::ItemExtras` arm; new `src/metadata_worker.rs`; `src/app.rs` one message forward | Only agent in tab.rs this batch. |
| A4 | I6b Workspace adapter | new `src/macos/workspace.rs`: app list for a URL, default app set, icon for file, eject, bookmark/alias | No shared files; consumers come in Batch B. |

### Batch B: six features (6 agents, parallel)

| # | Feature | Files touched | Depends on |
|---|---|---|---|
| 1 | Status bar | new `src/status_bar.rs`; `src/app.rs` `footer()` only | none |
| 2 | Copy path variants | new `src/copy_path.rs`; `src/macos/menu.rs` submenu; `src/app.rs` `Message::CopyPath` arm only | A1 |
| 3 | `clonefile` same-volume copy | `src/operation/recursive.rs` `copy` only | none |
| 4 | Open With via NSWorkspace (+ Change All if the UTType call works) | `src/mime_app.rs` cfg swap of the app-list source; `src/macos/workspace.rs` | A4 |
| 5 | Hidden-junk rules + Show Library toggle + folders-first in View menu | new `src/hidden_rules.rs`; `src/tab.rs` one line where `hidden` is set; `src/config.rs` one field; `src/menu.rs` `menu_bar` one entry | none (tab.rs line is far from A3's edits) |
| 6 | Shortcut parity audit + Cmd+N favourites + rename stem + reuse open tab | `src/key_bind.rs`; `src/tab.rs` rename start; `src/app.rs` open-folder handler | none |

### Batch C: three features (3 agents, parallel, after A2 and A3 merge)

| # | Feature | Files touched | Depends on |
|---|---|---|---|
| 7 | Packages as files + Show Package Contents + symlink/alias badge | `src/macos/metadata.rs` `is_package`, `is_alias`; `src/tab.rs` `item_from_entry` (package as file) and one badge call in the item renderer; `src/macos/menu.rs` entry; `src/macos/workspace.rs` icon | A2, A3, A4 |
| 8 | Tags: read, dots, Tags submenu set/unset, tag column | new `src/macos/tags.rs`; `src/macos/menu.rs` submenu; `src/tab.rs` dots render from `Item.extras` (same renderer region as #7: assign #7 and #8 to one agent or merge #7 first); `src/app.rs` one `Message::Tags(..)` arm | A2, A3, A1 |
| 9 | Share, AirDrop, Quick Actions | new `src/macos/share.rs`, `src/macos/services.rs`; `src/macos/menu.rs` entries; `src/context_action.rs` extension | A1 |

### Batch D: three features (3 agents, parallel)

| # | Feature | Files touched | Depends on |
|---|---|---|---|
| 10 | Folder sizes in list view + selection size in status bar | `src/metadata_worker.rs` size job; `src/tab.rs` size column reads `Item.extras`; `src/status_bar.rs` | A3, #1 |
| 11 | Spotlight search + Recents | new `src/macos/spotlight.rs` (`mdfind`); `src/tab.rs` `scan_search` and `scan_recents` cfg swaps | none |
| 12 | Put Back journal | new `src/trash_macos.rs`; `src/trash.rs` restore path cfg | none; first verify Restore is broken on macOS |

### What comes right after the twelve

Mounter for macOS (volumes, eject, Connect to Server), per-folder view memory, session restore with persistent undo, command palette, batch rename engine, Tahoe folder-customisation read. Column view and dual pane only after an upstream PR check and a written pane design.

### Testing rules for all of it

| Rule | Why |
|---|---|
| Pure logic in owned modules with unit tests (`copy_path`, `hidden_rules`, `metadata`, `status_bar`, `selection_pattern`, `listing_export`, `batch_rename`) | Matches `folder_look.rs` and `launch_macos.rs` today. |
| FFI wrappers thin, return `Option`/`Result`, log and no-op when `MainThreadMarker::new()` fails | Test threads are not the main thread; AppKit must never be reached from `cargo test`. |
| xattr and `clonefile` tests run against `std::env::temp_dir()` on APFS | Both work there; skip on non-APFS with a check, not a failure. |
| Spotlight, Share, QL panel: no unit tests; a manual checklist in `docs/` | Depend on the indexer and the desktop session. |
| `scripts/test-macos-bundle.sh` gains an `otool -L` system-only assertion | 04 adds the first network dependency; keep the dylib graph clean (porting notes 5.3). |
