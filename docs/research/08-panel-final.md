# Panel final: verified claims, resolved disagreements, roadmap

Role: adversarial reviewer and final word. Inputs: 01, 02, 05, 06 (architect), 07 (macOS). Repo: master at `018a5ae`, 65 commits ahead of upstream and 7 behind. Written 2026-10-09. Nothing in the repo was changed.

## Verdict in five lines

1. The app has more broken-on-day-one defects than either panel found. The worst one, drag and drop, is dead everywhere on macOS, not just "maybe it doesn't reach Mail".
2. Copy loses tags, Finder comments, quarantine flags and resource forks. Both panels proposed `clonefile` for speed and missed that the byte copy destroys metadata. So a tags feature built before this fix would hand users a copy that strips their tags.
3. Undo is an upstream stub. Three docs list it as present.
4. The six-piece infrastructure plan is mostly premature. Four small pieces pay for themselves, and each is built by its first consumer, not by a separate infra batch.
5. Do not write anything into Finder's storage (tags, Tahoe xattrs) until the in-app redesign ships and the reader exists. Then make it opt-in, default off.

## 1. Claim verification

"Verified" means I read the code path. "Runtime" means the code is clear but nobody has run it on 26.6 in a bundle.

| # | Claim | Source | Verified? | Evidence |
|---|---|---|---|---|
| 1 | `trash::delete` on macOS spawns `osascript` and tells Finder to delete | 07 | Verified | `trash-5.2.9/src/macos/mod.rs:22-55`: `DeleteMethod::new()` returns `Finder`. The app never calls `set_delete_method`; it calls plain `trash::delete` once per path, `src/operation/mod.rs:861`. |
| 2 | "Move to Trash exists, trash crate calls `trashItemAtURL`" | 06 | False | Same evidence as 1. `trashItemAtURL` is the non-default `NsFileManager` method. |
| 3 | A bundled build gets the Automation prompt denied silently | 07 | Unknown, runtime | `res/macos/Info.plist.in` has no `NSAppleEventsUsageDescription` (verified). The silent denial needs one manual test. Moot once item T0-1 lands. |
| 4 | One `osascript` per file | new | Verified | `src/operation/mod.rs:850-866` loops `trash::delete(path)` per path. Trashing 200 files spawns 200 `osascript` processes and plays 200 Finder sounds. |
| 5 | Put Back / Restore does nothing on macOS | 06, 07 | Verified, and worse | `src/operation/mod.rs:1137-1143`: `Restore` returns the error "Restoring from trash is not supported on macos". The Undo button on the trash toast scans `Location::Trash`, which is empty on macOS (`src/trash.rs:229-238` uses the default `scan` that returns nothing), so Undo silently does nothing: `src/app.rs:5246-5275`. |
| 6 | Undo exists (EditHistory, `Undo(usize)`) | 01, 05 Q21 | False | `src/app.rs:5243-5245` is `Message::Undo(_id) => { // TODO: undo }`. No Cmd+Z bind in `src/key_bind.rs:160-230`. |
| 7 | libcosmic DnD does not reach other apps | 07 | Verified, and worse | `window_clipboard` (`f68595e`) `src/platform/macos.rs:16-50`: every `DndProvider` method is an empty stub and `peek_offer` returns "DnD not supported". iced routes all drags through it: `libcosmic 4561417 iced/winit/src/clipboard.rs:309-328`. So drag inside the app, drop in from Finder and drag out to Mail are all dead. Spring-loaded folders (`tab.rs:5415-5440`) never fire. |
| 8 | Mounter is gvfs-only, so Network does nothing | 06, 07 | Verified, and worse | `src/mounter/mod.rs:129-135` registers only gvfs; `gvfs` is off in the macOS default features, `Cargo.toml:107-112`. With no mounters the Network entry is hidden (`src/app.rs:1949`), and no volume list exists. USB drives and mounted shares never appear in the sidebar, and eject has nothing to act on. |
| 9 | Open With is XDG-shaped | 06, 07 | Verified, and worse | `desktop` is off by default (`Cargo.toml:107,110`), so `MimeAppCache::reload` is a no-op (`src/mime_app.rs:317-318`). The Open With dialog lists no apps. Double-click still works through the `open` crate fallback, `src/app.rs:987-989`. |
| 10 | Open in Terminal works | 01 (implied), 05 Q7 "partial" | False | `terminal()` reads `self.terminals`, filled only by the no-op `reload` and by `xdg-mime` (`src/mime_app.rs:518-556`). Returns `None`, so the menu item (`src/menu.rs:189,289`) does nothing. |
| 11 | Recents is probably empty | 06 | Partly false | It is the app's own history. `recently-used-xbel 1.2.0 src/lib.rs:152` reads `~/.local/share/recently-used.xbel`, which the app writes on open (`src/app.rs:992`). So Recents shows only files opened in Macosmetic. 07 had this right. |
| 12 | `.app` and other packages show as folders | 06, 07 | Verified | No package check in `src/tab.rs`. "bundle launch fixes" in 01 means launching Macosmetic itself (`src/launch_macos.rs`). |
| 13 | Same-volume copy streams bytes | 05, 06, 07 | Verified | `src/operation/recursive.rs:439-600`. |
| 14 | Copy preserves metadata | new | False | The copy sets permissions (`recursive.rs:499-507`) and mtime/atime (`:581-591`). No xattrs, so tags, Finder comments, `com.apple.quarantine`, resource forks and FinderInfo are dropped. Directories are plain `create_dir_all` (`:401-403`), so folder tags and Tahoe folder looks are dropped. Cross-volume move is copy then delete, so it loses them too. |
| 15 | `.DS_Store`, `._*`, `.localized` junk rules do not exist | 05 Q18, 06 | Partly false | `.DS_Store` is always hidden already, `src/tab.rs:501-503`. |
| 16 | macOS hidden flag is honoured | new | False | `hidden_attribute` returns `false` on every non-Windows platform, `src/tab.rs:490-493`. So `/` shows `bin`, `usr`, `private`, `cores`; Home shows `Library`; customised folders show their `Icon\r` file. |
| 17 | Remote FS detection exists on macOS | 06 (implied) | False | `fs_kind` returns `Local` everywhere but Linux, `src/tab.rs:582-585`. SMB mounts get content sniffing and thumbnails like a local disk. |
| 18 | iCloud dataless files are handled | 07 asks for it | False | No `SF_DATALESS` or ubiquitous-key check anywhere in `src/`. JPEG and PNG go to the `image` crate, not Quick Look (`src/quicklook_macos.rs:96-107`), so thumbnailing a dataless photo folder reads, and so downloads, every file. Whether mime sniffing also materialises files is runtime-unverified. |
| 19 | No `odoc` Apple Event handler; startup paths come from argv | 07 | Verified | No match for `AppleEvent`/`odoc` in `src/`; `src/lib.rs:182` loops `env::args`. |
| 20 | Reveal in Finder uses `open -R` | 07 | Verified | `src/tab.rs:2037` (07 cited 2044). |
| 21 | New window re-executes the binary | 06 | Verified | `src/app.rs:5329`, also 4232, 5043, 5054. Each window is a separate process with its own Dock icon. |
| 22 | Status bar absent; `footer()` is the home | 06 | Verified | `src/app.rs:6873-6876` returns `None` unless operations run. |
| 23 | Sort is Name, Modified, Size, TrashedOn only | 05, 06 | Verified | `src/tab.rs:3131-3136`. |
| 24 | Cut is wired to Cmd+X | 05 Q1 | Verified | `src/key_bind.rs:199`. Dimmed rendering not checked. |
| 25 | No Duplicate, no Cmd+I, no Cmd+Z | new | Verified | `src/key_bind.rs:120-230` binds none of them. |
| 26 | Upstream landed favourite rename `7cebbef` | 06 | Verified | Ancestor of master. |
| 27 | `objc2-disk-arbitration`, `objc2-core-services`, `objc2-quick-look-ui` exist | 06, 07 | Verified | crates.io: all at 0.3.2. |
| 28 | Bonjour through `NWBrowser` in `objc2-network` | 07 | False | `objc2-network` is a 0.0.0 placeholder on crates.io. |
| 29 | `libc` has `clonefile` and `copyfile` with `COPYFILE_CLONE` | new | Verified | `libc-0.2.190 src/unix/bsd/apple/mod.rs:4256-4271, 5195-5218`. `libc` is already a direct dep, `Cargo.toml:22`. |
| 30 | `NSWorkspace.icon(forFile:)` returns a plain folder for Tahoe looks | 07 | Unknown | External source only. |
| 31 | Tahoe folder xattr can be written with plain `setxattr` | 07 | Unknown | 07 says it tested this; I did not repeat it. |
| 32 | `statvfs` free space differs from Finder's | 06, 07 | Unknown | Plausible: purgeable space. One-line check during the status bar work. |

## 2. Disagreements resolved

| Topic | 06 architect | 07 macOS | Ruling | Why |
|---|---|---|---|---|
| Move to Trash | Exists | Broken, needs fixing first | 07. Tier 0. Use `NSFileManager trashItemAtURL:resultingItemURL:` on the worker, not `NSWorkspace recycleURLs` | Verified false claim in 06. `trashItemAtURL` is synchronous and returns the trashed URL, which the Undo journal needs. `recycleURLs` adds a completion block for no gain. |
| Put Back | ADD-LATER, M | ADD-NOW | Tier 0, scoped to the toast Undo plus Restore of items in our journal | The current Undo button is a silent no-op. Finder's own Put Back stays unreliable either way. |
| `clonefile` | ADD-NOW | ADD-NOW | Both used the wrong primitive. Use `copyfile` with clone plus metadata | Clone alone fixes speed. The real defect is metadata loss on every copy. |
| Tags | ADD-LATER via I2/I3 | ADD-NOW | Read in Tier 1. Write in a later Tier 1 batch with read-modify-write. Copy must preserve them first (Tier 0) | Reading is safe. Writing is where user data can be clobbered. |
| Quick Look panel | ADD-LATER, maybe `qlmanage` | ADD-NOW, M | Tier 2 | Space already opens the gallery. The responder-chain insertion is fragile across winit upgrades. |
| Hidden-junk rules | ADD-NOW, new module | ADD-NOW | Replace with "honour `UF_HIDDEN`", Tier 0, one function | `.DS_Store` is already hidden. The real gap is the BSD hidden flag. |
| Per-folder view memory | ADD-LATER | ADD-NOW | Tier 2 | Nothing is broken. It is M and sits in upstream-shaped code. |
| Session restore | ADD-LATER | ADD-NOW | Tier 2 | Separate processes per window make it awkward. Not broken. |
| Eject and volumes | ADD-LATER after I6c | ADD-NOW | Tier 0, list and eject only | USB drives never appear in the sidebar. That reads as broken in minute one. |
| Connect to Server | ADD-LATER, L | ADD-LATER | Tier 2 | Agreed. Mounted shares will appear through the volume list anyway. |
| Default folder opener (`odoc`) | ADD-LATER | ADD-NOW | Tier 1 | Matters only once the user opts in. Small. |
| Share, AirDrop | ADD-LATER | ADD-NOW | Tier 1 | `with_ns_window` already exists, so no bridge is needed. |
| Recents | Probably empty | App-only history | 07 correct. Tier 1 Spotlight-backed Recents | Verified, claim 11. |
| Drag out | ADD-LATER, L | Verify first; if broken, #2 | Tier 0 spike, then Tier 1 | It is broken, and so is in-app drag. |
| Finder mirror of folder looks | Later, opt-in | Default on for colour | 06. Tier 2, opt-in, default off | See the section below. |
| Import Finder sidebar (sfl3) | IGNORE | ADD-NOW | Ignore | Private keyed archive. Re-adding six favourites is a one-minute job. |
| Quick filter | ADD-LATER | ADD-NOW | Tier 2 | Nice, not broken, touches an upstream enum. |
| Progress speed and ETA | Wait for upstream | ADD-NOW | Wait for upstream | Cross-platform, 14 reactions upstream, operation module. |
| Trash auto-empty, Finder-layout zip | IGNORE | ADD-LATER | Ignore | Finder and `ditto` already do both. |
| Infrastructure first | 6 pieces, Batch A of 4 agents | Not addressed | 4 small pieces, each built by its first consumer | See section 4. |

### Writing folder looks into Finder: not now

Ruling: build nothing that writes Finder storage until 03 ships and a reader exists. Then add it as opt-in, default off.

Reasons, in order of weight:

1. A colour tag is user data, not decoration. Users give tags meaning, such as "Red = urgent". Writing "Red" because they picked a red folder colour puts that folder into their Red tag searches, the Finder sidebar and Smart Folders.
2. The write syncs. Tags and the Tahoe xattr travel through iCloud to every device, so a bug multiplies.
3. The glyph xattr is undocumented JSON. A format change in 26.x would make us write something Finder misreads.
4. Copy currently strips xattrs (claim 14). Until T0-3 lands, anything we write is lost on the next copy anyway.
5. 03 is mid-implementation on `feat/folder-customize-v2`. Adding a second storage path now changes its data model while it is being built.

What is safe now: nothing. What comes next, in Tier 2: a read-only reader for Finder folder colours, so Finder-customised folders look right here.

## 3. Roadmap

Size: S is under 3 agent-days, M is up to 2 weeks, L is more. "Shared" files are `app.rs`, `tab.rs`, `menu.rs`, `operation/`. In-flight branches already touch `app.rs`, `menu.rs`, `key_bind.rs`, `config.rs`, `folder_*.rs` and `icon_themes.rs`, so Tier 0 avoids `menu.rs` and `key_bind.rs` where it can.

### Tier 0: broken on day one

| ID | One-line spec | Files touched | Size | Acceptance check |
|---|---|---|---|---|
| T0-1 | Trash through `NSFileManager trashItemAtURL:resultingItemURL:` in one worker pass, and record original to trashed URL in a journal | new `src/trash_macos.rs`; `src/operation/mod.rs` `Delete` arm only (849-866) | S | Trash 200 files from a bundled build: no Automation prompt, no `osascript` in `ps`, one progress run. |
| T0-2 | Undo toast and Restore use the journal on macOS | `src/trash_macos.rs`; `src/operation/mod.rs` macOS `Restore` arm (1137-1143); `src/app.rs` `UndoTrash` arm (5246) under `cfg` | S | Trash a file, click Undo: it is back at its original path. Restore after relaunch works for journaled items. Same agent as T0-1. |
| T0-3 | Copy with `copyfile(COPYFILE_CLONE)` first, then fall back to the existing stream followed by `fcopyfile(COPYFILE_METADATA)`. Copy directory metadata after its children. | `src/operation/recursive.rs` `copy` and the `Mkdir` arm only | S | Unit test on a temp dir: a file with a tag, `com.apple.quarantine`, a resource fork and a Finder comment keeps all four (`xattr -l` identical). A tagged folder keeps its tag. A 2 GB same-volume copy finishes in under 1 s. |
| T0-4 | Honour `UF_HIDDEN` in `hidden_attribute` on macOS | new `src/fs_flags_macos.rs`; `src/tab.rs` `hidden_attribute` (490-493) | S | `/` shows Applications, Library, System, Users only. Home hides Library. Cmd+Shift+. reveals them. |
| T0-5 | Mark SMB, NFS, AFP, WebDAV mounts as `Remote` via `statfs` `MNT_LOCAL` | `src/fs_flags_macos.rs`; `src/tab.rs` `fs_kind` (582-585) | S | Unit test: `/` is Local. An SMB mount reports Remote, so mime sniffing uses names only. |
| T0-6 | Never read the content of a dataless file: skip thumbnail decode, checksum and dir size when `SF_DATALESS` is set | `src/fs_flags_macos.rs`; `src/tab.rs` thumbnail constructor (~2225) and the checksum and dir-size triggers | S | With Optimize Mac Storage on, open a folder of evicted photos: `brctl status` shows no downloads start. Icons show, not thumbnails. |
| T0-7 | Open With lists LaunchServices apps, and Open in Terminal opens Terminal.app. Build `MimeApp` entries whose exec is `/usr/bin/open -a <app> %F` so `app.rs` needs no change. | `src/mime_app.rs` `cfg(target_os = "macos")` `reload` and `terminal`; new `src/workspace_macos.rs` (`URLsForApplicationsToOpenURL:`) | M | Open With on a `.txt` lists TextEdit and installed editors with icons. "Set as default" changes the default app. Open in Terminal opens Terminal at that folder. |
| T0-8 | Packages behave as files: double-click opens the app or document; add "Show Package Contents" | new `src/url_values_macos.rs` (`NSURLIsPackageKey`); `src/tab.rs` `item_from_entry` and the open-or-navigate decision; `src/menu.rs` one entry | S | Double-clicking Safari.app launches Safari. A `.rtfd` opens in TextEdit. Show Package Contents browses inside. |
| T0-9 | Volumes in the sidebar with eject. List with `getmntinfo` filtered by `MNT_DONTBROWSE`; eject with `NSWorkspace unmountAndEjectDeviceAtURL:` | new `src/mounter/macos.rs`; `src/mounter/mod.rs` registration | M | Plug in a USB stick: it appears under the sidebar within 2 s. Eject removes it. A mounted SMB share also appears. |
| T0-10 | Drag and drop spike, then fix. Implement the macOS `DndProvider` in a `window_clipboard` fork: an in-process loopback for internal drags, then `NSDraggingDestination` for drops from Finder. Drag-out is Tier 1. | fork of `pop-os/window_clipboard` on branch `macosmetic`; `Cargo.toml` one `[patch]` line. No Macosmetic source. | L (spike S first) | Drag a file onto a folder in the app: it moves. Drag from Finder into a window: it copies into the hovered folder. Spring-load opens a folder after the delay. |

### Tier 0 batches

| Batch | Agents (one row each) | Why they don't collide |
|---|---|---|
| 0a | T0-1+T0-2 (trash); T0-3 (copy); T0-4+T0-5+T0-6 (`fs_flags_macos.rs`, three `tab.rs` functions); T0-7 (Open With) | Disjoint: `operation/mod.rs` arms vs `operation/recursive.rs` vs `tab.rs` helpers vs `mime_app.rs`. T0-2's `app.rs` touch is one arm. |
| 0b | T0-8 (packages); T0-9 (volumes); T0-10 (DnD fork) | T0-8 is the only `tab.rs` agent in this batch. T0-9 is new files plus `mounter/mod.rs`. T0-10 lives in another repo and can start during 0a. T0-8's `menu.rs` line lands after `feat/folder-customize-v2` merges. |

### Tier 1: next, buildable now

| ID | One-line spec | Files touched | Size | Acceptance check |
|---|---|---|---|---|
| T1-1 | Status bar: item count, selection count and size, free space from `NSURLVolumeAvailableCapacityForImportantUsageKey` | new `src/status_bar.rs`; `src/app.rs` `footer()` | S | Free space matches Finder's within 1%. Selecting 3 files shows "3 of N selected, X MB". |
| T1-2 | Cmd+Z undo for rename, same-volume move, new folder and trash. Fill upstream's `Message::Undo` stub. | new `src/undo.rs` (journal, pure, tested); `src/app.rs` `Undo` arm; `src/key_bind.rs` one bind | M | Rename then Cmd+Z restores the name. Move then Cmd+Z moves it back. Ten-step history. |
| T1-3 | Duplicate (Cmd+D, " copy" naming over the T0-3 clone) and New Folder with Selection | `src/operation/mod.rs` naming helper; `src/key_bind.rs`; macOS menu hook | S | Cmd+D on a 5 GB file is instant and names it "x copy.ext". |
| T1-4 | Cmd+I opens the details pane; add Kind, Where, Created and Date Added fields | `src/app.rs` preview-pane fields; `src/url_values_macos.rs` keys; `src/key_bind.rs` | S | Cmd+I on a file shows Kind as "PDF document" and the right Date Added. |
| T1-5 | Sort by Kind and Date Added; sort on the UTType id, show the localised Kind | `src/tab.rs` `HeadingOptions` and comparator; menu sort entries | S | Sorting a mixed folder by Kind groups PDFs together. |
| T1-6 | Tags, read only: coloured dots in grid and list, Tags column | `src/url_values_macos.rs` (`NSURLTagNamesKey`); `src/tab.rs` item field and renderer | M | Tag a file in Finder: the dot appears here after refresh. No writes happen. |
| T1-7 | Tags, write: Tags submenu set and unset, read-modify-write, unknown tags preserved | new `src/tags_macos.rs`; macOS menu hook | S | Add Red here: Finder shows Red and keeps the file's other tags. Unit test on a temp file. Starts after T1-6. |
| T1-8 | Drag out to other apps with `NSPasteboardTypeFileURL` | `window_clipboard` fork (`start_dnd`) | M | Drag a file to Mail: it attaches. Starts after T0-10. |
| T1-9 | Share and AirDrop with `NSSharingServicePicker` through the existing `with_ns_window` | new `src/share_macos.rs`; macOS menu hook | S | Share on a photo shows AirDrop, Messages, Mail. |
| T1-10 | Recents from Spotlight: `mdfind -onlyin ~ 'kMDItemLastUsedDate >= $time.today(-30)'`, excluding `~/Library` | new `src/spotlight_macos.rs`; `src/tab.rs` `scan_recents` under `cfg` | S | Recents lists a file opened in Preview today. |
| T1-11 | Default folder opener: `odoc` handler on `NSAppleEventManager`, plus a Settings switch with "Restore Finder" | `src/appkit_macos.rs` handler; `src/app.rs` one message; settings toggle | M | After the switch, a Dock folder click and `open ~/Documents` open a tab here. "Restore Finder" undoes it. |
| T1-12 | Copy path variants: POSIX, `~/`, shell-quoted, `file://`, name | new `src/copy_path.rs` (pure, tested); `src/app.rs` `CopyPath` arm; macOS menu hook | S | Each variant is in the clipboard exactly as specified. |

### Tier 1 batches

Precondition: `feat/folder-customize-v2` merged, and the macOS menu hook (infra M3 below) landed, because five items add menu entries.

| Batch | Agents | Why they don't collide |
|---|---|---|
| 1a | T1-1 (status bar); T1-5 (sort); T1-10 (Recents); T1-12 (copy path) | `footer()` vs `HeadingOptions` vs `scan_recents` vs the `CopyPath` arm. T1-5 and T1-10 are both in `tab.rs` but in functions about 1,700 lines apart. |
| 1b | T1-2 (undo); T1-6 (tags read); T1-9 (share); T1-11 (`odoc`) | T1-2 owns `key_bind.rs` and the `Undo` arm. T1-6 owns the `tab.rs` item renderer. T1-9 is new files plus one menu line. T1-11 owns `appkit_macos.rs`. |
| 1c | T1-3 (duplicate); T1-4 (Get Info); T1-7 (tags write); T1-8 (drag out) | T1-3 and T1-4 each add one line to `key_bind.rs`. Merge T1-3 first and let T1-4 rebase. T1-8 is the fork. |

### Tier 2: later

Per-folder view memory. Session restore. Spotlight search with kind and date filters. Smart Folders (open existing only). Connect to Server through NetFS. Cloud badges, Download Now, Remove Download. Quick Look panel. Quick Actions and Services. Folder sizes in list view, with the background worker (I3) built then. Reader for Finder folder colours and Tahoe glyphs. Opt-in, default-off "Show in Finder too" for colour tags. Aliases: resolve to open here instead of handing off, then Make Alias. Locked flag and Hide extension. Batch rename. Command palette. In-process multi-window, which removes the extra Dock icons. Column view and dual pane, each after an upstream PR check. AccessKit experiment. Quick filter. Drag modifiers.

### Ignore

| Item | Why |
|---|---|
| Moving `*_macos.rs` into `src/macos/` (I1) | Pure churn. It forces every open branch to rebase and unlocks nothing. |
| The `FileExtras` mega-struct (I2) built up front | Eight fields, zero consumers today. Grow the two small helpers instead. |
| Writing the Tahoe glyph xattr | Undocumented JSON, synced through iCloud. |
| Finder sidebar import (sfl3), `.DS_Store` reading, icon positions, Clean Up | Private formats. |
| `NSFileViewer` hijack | Undocumented. It loops back into our own Reveal in Finder. |
| Liquid Glass chrome, toolbar customisation | No NSView hosting. Low demand. |
| AppleScript dictionary, App Intents, Handoff | New toolchains or an iOS app. |
| Desktop, Stacks, Open/Save panels, Finder Sync hosting | Finder-owned. |
| Admin helper, Burn, Stationery, Keep Downloaded, Secure Empty | Signing first, dead hardware, nobody uses it, private, removed. |
| Trash auto-empty, Finder-layout zip | The system already does both. |
| Multi-pane beyond two, embedded terminal, merge all windows | XL for niche value. |
| Bonjour through `objc2-network` | That crate is a placeholder. |

## 4. Minimum shared infrastructure

Four pieces. None gets its own agent. Each is created by the first Tier 0 or Tier 1 item that needs it, so infrastructure ships with a consumer and a test.

| ID | Piece | Built by | Consumers within the next 10 features | Size |
|---|---|---|---|---|
| M1 | `src/fs_flags_macos.rs`: pure `st_flags` and `statfs` helpers: `is_hidden`, `is_dataless`, `is_immutable`, `is_local_fs` | T0-4 | T0-4, T0-5, T0-6, Locked badge, cloud badges | about 60 lines |
| M2 | `src/url_values_macos.rs`: one `resource_values(path, &[keys])` over `NSURL`. Foundation, so it is safe off the main thread. | T0-8 | T0-8, T1-4, T1-5, T1-6, Date Added, cloud status | about 100 lines |
| M3 | macOS menu hook: one `cfg` call each in `menu.rs` `context_menu` and `menu_bar` into `src/menu_macos.rs` | first Tier 1 batch, after `feat/folder-customize-v2` merges | T0-8 (moves its entry here), T1-3, T1-7, T1-9, T1-12 | 2 lines in `menu.rs` |
| M4 | Convention, not code: new features add one nested `Message::Feature(feature::Message)` arm in `app.rs` | adopted from now on | every new owned feature | 0 |

What was cut from 06's six:

- I1 move: churn, see Ignore.
- I2 as one big struct: replaced by M1 and M2, which grow per consumer.
- I3 worker: deferred to folder sizes in Tier 2. Tags read at scan time are cheap on local disks, and T0-5 marks remote disks so the read can be skipped there.
- I4 retrofit of 03 and 04: both branches are already written with flat variants. Retrofitting mid-flight costs more than it saves. Apply M4 to new work only.
- I6 adapters: these are features. They are built as T0-7, T0-9 and T1-10.
