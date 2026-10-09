# Panel verdict: macOS platform side

Reviewed 2026-10-09 against master `018a5ae`, on a Mac running macOS 26.6.2. Inputs: `01-current-features.md`, `02-finder-features.md`, `05-qol-brainstorm.md`. Nothing in the repo was changed.

## Facts this verdict rests on

| Fact | Where it came from | What it changes |
|---|---|---|
| The app is not sandboxed. `scripts/macos-bundle.sh` signs ad-hoc with no entitlements file. `Info.plist.in` has the five folder usage strings and no `com.apple.security.*` keys. | repo | Every file API is open. App Store is off the table. Notarization needs hardened runtime, which changes two things below. |
| `trash::delete` on macOS defaults to `DeleteMethod::Finder`: it spawns `osascript` and tells Finder to delete. | `trash 5.2.9`, `src/macos/mod.rs:22-47`; `src/operation/mod.rs:861` | Every Move to Trash sends an Apple Event to Finder. That is an Automation TCC prompt. `Info.plist.in` has no `NSAppleEventsUsageDescription`, so a bundled build is denied silently. Needs fixing before anything else. |
| The Tahoe folder xattr `com.apple.icon.folder#S` can be written and removed with plain `setxattr`. Tested here: write, read back, delete, no error. | this machine | Mirroring glyphs to Finder needs no AppKit, only the `xattr` crate. |
| `NSWorkspace.icon(forFile:)` returns the plain folder for Tahoe-customised folders. The Dock also shows plain folders. | mjtsai.com 2025-10-04 update quoting Matthias Gansrigler | Macosmetic must composite Tahoe looks itself to show Finder-customised folders. |
| Tahoe colours a folder from its most recent colour tag. Symbol or emoji lives in the xattr. Both xattrs plus `com.apple.FinderInfo` must be present. | eclecticlight.co 2025-09-18 | Colour has a public API path: tags. |
| `trashItem` and `recycle` write Put Back records, but Finder and FileManager race on the Trash `.DS_Store`. DTS reproduced the loss with the Trash window open. | developer.apple.com/forums/thread/773997 | Put Back is "usually works" at best through the public API. |
| No `application:openURLs:` or Apple Event handler exists in `src/`. Startup paths come from `env::args` only (`src/lib.rs:182`). | repo | Being the default folder opener does nothing today: LaunchServices sends an `odoc` event, not argv. |
| `src/operation/recursive.rs:439` copies by streaming bytes through compio. | repo | Same-volume copies on APFS are byte copies while Finder clones. |

## (a) Should folder colours and icons also become real macOS custom icons?

Yes for colour and glyph, through Tahoe's own storage. No for the app's theme icons by default. Offer the classic `Icon\r` custom icon only as an opt-in for the Image look.

The reason is that Finder has three different storage mechanisms, and only one of them is both public and visible everywhere.

| Look | Finder-native storage | API | Finder shows it | Dock, Spotlight, Open/Save panels show it | Private? | Reversible |
|---|---|---|---|---|---|---|
| Colour | A colour tag in `com.apple.metadata:_kMDItemUserTags`. Tahoe tints the folder from the newest colour tag. | `NSURL.setResourceValue(_, forKey: NSURLTagNamesKey)`, crate `objc2-foundation` feature `NSURL`. Or the `xattr` crate plus a binary plist. | Yes, and it syncs through iCloud | Dock: yes. Spotlight results: no. Open/Save panels: yes, they use Finder's folder rendering. | No | Remove the tag. Record which tag we added so a user tag is never removed. |
| Glyph (SF Symbol or emoji) | `com.apple.icon.folder#S` holding `{"sym":"name"}` or `{"emoji":"x"}`, plus a `com.apple.FinderInfo` bit. | `xattr::set`, crate `xattr`. No AppKit. | Yes, synced | No. Dock, Spotlight and `icon(forFile:)` all return the plain folder. | Yes, undocumented JSON. Writing works on 26.0 to 26.6. | `xattr::remove`. Own marker xattr says whether we wrote it. |
| Full image | `Icon\r` file in the folder with an icns resource fork, `kHasCustomIcon` in FinderInfo. | `NSWorkspace.setIcon(_:forFile:options:)`, crate `objc2-app-kit` feature `NSWorkspace`, plus `NSImage` with representations from 16 to 1024 px. | Yes | Yes, everywhere: `icon(forFile:)` composites it. | No, public since Mac OS 9 | `setIcon(nil, forFile:)` removes the file and clears the bit. |

Why not write `Icon\r` for every look:

- It drops a visible-to-tools file in the user's folder. `ls -a`, git status, zip archives, rsync, SMB clients and Windows machines all see `Icon\r`. The current `folder_look.rs` design explicitly keeps the app's choices out of the user's folders. That rule is right.
- It kills the Tahoe tint. A folder with a pasted icon has no colour to change when tagged.
- It is pixels, not intent. `FolderLook::Colour` and `FolderLook::Icon` store intent so a theme change re-renders every folder. An `Icon\r` is stale the moment the icon theme changes, and rewriting hundreds of folders on theme change is a bad idea.
- It breaks code signatures on bundles. Never apply it to a package.
- Resource forks do not survive exFAT, FAT or many SMB servers. They become `._Icon\r` AppleDouble files or vanish.

Why Tahoe storage is still worth doing even though it is partly private:

- Colour is the 80% case and the colour path is public.
- Finder-customised folders already exist on the user's Mac. Reading all three mechanisms is table stakes. Once the reader exists, the writer for colour and glyph is small.
- Finder, Files on iPhone and iCloud sync all read the same xattr. One write and the folder looks the same on every device.

Recommended model:

1. `Config::folder_looks` stays the source of truth for the app. Finder storage is a mirror, written on change and on a per-folder "Show in Finder too" toggle in the drawer. Default on for Colour, on for Icon when a mapping exists, off for Image.
2. Colour maps to the nearest of Finder's seven tag colours. Only an exact match is written as a tag. The app's 25-colour palette cannot be represented, so the drawer should show a small Finder dot next to the seven that will mirror.
3. Icon maps through a static table from theme icon names to SF Symbol names, for the symbols Finder's picker offers. `folder-git` has no symbol and is not mirrored. Emoji are mirrored as `{"emoji":...}`.
4. Image is mirrored only when the user ticks the box. Write with `setIcon`. Skip packages and bundles.
5. Ownership marker. Write `org.macosmetic.folder-look#S` with a hash of what was written. On reset, remove only what matches. A user who also changed the folder in Finder keeps their change.
6. Reader. On scan, read the tag colour, the Tahoe xattr and `kHasCustomIcon`. Render the composite in Macosmetic. For `Icon\r`, `icon(forFile:)` returns the composite and is safe from any thread. For Tahoe glyphs, render the SF Symbol with `NSImage.imageWithSystemSymbolName` and draw it over the themed folder. Cache by path and mtime of the xattr.

Open items to verify on 26.6 before building: which FinderInfo bit Tahoe sets alongside the xattr. Customise one folder in Finder, then `xattr -px com.apple.FinderInfo <folder>` and diff against a plain folder. Also check whether the 26.2 custom colour slider wrote a colour key into the xattr; dump the xattr on a folder given a non-tag colour.

Threading. `setIcon` and `icon(forFile:)` do not touch the window. Call them from the scan thread, not through `with_ns_window`. `icon(forFile:)` is documented thread-safe. Treat `setIcon` as main-thread until tested.

## (b) Minimum to be the default folder opener, and what Reveal in Finder does

Minimum, in order:

| Step | What | API |
|---|---|---|
| 1 | Keep `CFBundleDocumentTypes` for `public.folder`. `LSHandlerRank Alternate` is fine. Rank only orders the Open With list. The user preference decides the default. | `res/macos/Info.plist.in`, already there |
| 2 | Handle the `odoc` Apple Event. Without this the folder arrives and nothing happens. Install a handler on `NSAppleEventManager` for `kCoreEventClass`/`kAEOpenDocuments` and read the `keyDirectObject` list as `NSURL`s. Route to "open in new tab" when a window exists, to the first window when the app is launching. | `objc2-foundation` features `NSAppleEventManager`, `NSAppleEventDescriptor`. Avoids touching winit's app delegate. Main thread only. |
| 3 | Handle the reopen case. A second double-click in the Dock while running delivers another `odoc`, not a new process. Same handler. | as above |
| 4 | Register the default. From inside the app with `NSWorkspace.shared.setDefaultApplication(at: bundleURL, toOpen: .folder)`, completion on an arbitrary queue. From a shell with `duti -s com.system76.CosmicFiles public.folder all`. No consent dialog, that is only for browser and mail. | `objc2-app-kit` `NSWorkspace` with `objc2-uniform-type-identifiers` `UTType` |
| 5 | Live at a stable path with a stable signature. LaunchServices keeps one record per bundle path. Builds in `target/macos` register a second copy and the default flaps between them. Install to `/Applications`, sign with Developer ID. | `scripts/macos-bundle.sh` |
| 6 | Provide a "Make Macosmetic the default folder opener" switch in Settings and a "Restore Finder" switch. Restore sets `com.apple.finder` as handler. | same call |

What then opens in Macosmetic: Dock folder clicks, Spotlight folder results, `open <dir>` from a shell, double-clicking a folder in Open/Save panels does not change (panels browse in place), Mail "Show in Finder" for attachments, apps that call `NSWorkspace.open(folderURL)`.

What stays with Finder: the Desktop, Finder's own windows, Dock stacks and "Show in Finder" from a Dock item, Spotlight "Show in Finder".

Reveal in Finder from other apps:

- Xcode, Chrome, Safari downloads and most apps call `NSWorkspace.activateFileViewerSelecting(_:)`. Apple documents it as "Activates the Finder". There is no public hook.
- There is one undocumented global default, `defaults write -g NSFileViewer -string com.system76.CosmicFiles`. ForkLift and Path Finder set it, and users report "almost every app except the Desktop" then reveals in ForkLift. It is unsupported and the event AppKit sends the viewer is not documented. Verify what arrives: set the default, call `open -R <file>` and log every Apple Event the handler receives. Expect `odoc` on the parent folder with a selection parameter, or a plain `odoc`.
- Macosmetic's own Reveal in Finder uses `open -R` (`src/tab.rs:2044`). With `NSFileViewer` pointing at itself that loops back. Guard it: when the app is the registered viewer, replace the menu item with "Reveal in Finder" that targets `com.apple.finder` explicitly through `open -b com.apple.finder -R`.

Verdict: steps 1 to 6 are ADD-NOW and small. `NSFileViewer` is ADD-LATER behind a switch labelled as unsupported.

## Platform constraints that apply across the table

| Constraint | Consequence |
|---|---|
| winit owns `NSApplication` and the main run loop. iced holds the app state borrowed during update and view. | Any AppKit call that needs a view or window goes through `with_ns_window` or a `MainThreadMarker`. AppKit callbacks must post a message, never touch state. `NSMetadataQuery` posts results through `NSNotificationCenter` on the main run loop. Prefer the `MDQuery` C API with `MDQuerySetDispatchQueue` so results arrive on a worker. |
| The responder chain is winit's. `QLPreviewPanel`, the Services menu and some sharing delegates look up the chain for an object that answers. | Insert one app-owned `NSResponder` subclass between winit's content view and the window with `setNextResponder`. It answers `acceptsPreviewPanelControl:`, `validRequestorForSendType:returnType:` and `writeSelectionToPasteboard:types:`. One class serves Quick Look, Services and Quick Actions. Check after each winit upgrade that `nextResponder` is still ours. |
| No sandbox, no App Store. | Full filesystem access with the five usage strings. Trash needs Full Disk Access, which has no prompt API, only the deep link already in `tab::open_privacy_settings`. App Store distribution would need the sandbox, and a sandboxed file manager cannot browse. Ignore App Store. |
| Notarization needs the hardened runtime. | Two effects. Sending Apple Events needs the `com.apple.security.automation.apple-events` entitlement and `NSAppleEventsUsageDescription`. Loading unsigned dylibs is refused, which matters only if a future feature links a non-Apple framework. Everything else in the table is unaffected. |
| Ad-hoc signatures change on every build. | TCC grants reset per build until Developer ID. Nothing in the table can be tested for grant persistence before that. |
| Finder Sync, Open/Save panels, Desktop, Put Back records, `.DS_Store` view options. | Finder-internal. No third-party browser gets them. Items that depend on them are IGNORE or own-store-only. |

## Feature table

Verdicts: ADD-NOW is this quarter, ADD-LATER is after the ADD-NOW set, IGNORE is not worth building. Stakes: T is table stakes, D is delighter. Duplicates from 02 and 05 are merged; the 05 idea number is in the Feature column.

### Trash and file operations

| Feature | Verdict | Stakes | API + crate | Platform risks | Reason |
|---|---|---|---|---|---|
| Move to Trash without controlling Finder | ADD-NOW | T | `NSWorkspace.recycleURLs:completionHandler:` with the whole batch in one call, `objc2-app-kit` `NSWorkspace`. Or `trash` crate `DeleteMethod::NsFileManager`. | Current default spawns `osascript` and needs an Automation prompt plus a missing usage string. Hardened runtime adds an entitlement. | A Finder replacement that asks permission to control Finder on first delete is broken on sight. One array call also gives Finder its best shot at Put Back records. |
| Put Back from Macosmetic's Trash view | ADD-NOW | T | Own journal keyed by trashed URL, written next to the config. `recycleURLs` returns the mapping of original to trashed URL. | Finder's Put Back for our items depends on the `.DS_Store` race. Our own view is reliable. | Users expect Put Back. The journal costs little and `recycleURLs` hands us the data. |
| Trash view listing | ADD-NOW, already partial | T | `NSFileManager.URLsForDirectory(.trashDirectory)` for the path, `objc2-foundation` `NSFileManager`. Listing needs Full Disk Access. | FDA has no prompt. The deep link exists. Per-volume `.Trashes` fall under the removable volumes string. | Already handled with the denial screen. Keep it. |
| Empty Trash on all volumes | ADD-LATER | T | `NSFileManager.removeItem` over each `.Trashes/<uid>`. | FDA on the boot volume, removable-volumes TCC on others. | Works once FDA is granted. Low effort after the listing. |
| Trash auto-empty after N days, confirm-on-trash (05 #44) | ADD-LATER | D | Own timer plus the Trash listing. | Same FDA dependency. | Finder has 30 days. Cheap once the Trash view works. |
| APFS clone for same-volume copy and Duplicate (05 #3) | ADD-NOW | T | `libc::clonefile` or `clonefileat`, fall back to the streaming copy on `EXDEV` or `ENOTSUP`. No ObjC. | `recursive.rs:439` streams bytes for progress. Clone first, then report 100%. Directories clone recursively in one call. | Finder duplicates a 10 GB file instantly. A byte copy makes the app look slow in the first minute. |
| Duplicate with " copy" naming | ADD-NOW | T | `NSWorkspace.duplicateURLs:completionHandler:` gives Finder's exact naming, or own naming over the clone. | None. | Cmd+D is muscle memory. |
| Copy and move progress with speed, ETA, current file (05 #19) | ADD-NOW | T | Own. | None. | Already has an operation queue. Add the numbers. |
| Operation queue: serialise same-disk copies, pause, resume, reorder (05 #24) | ADD-LATER | D | Own. `NSURLVolumeIdentifierKey` groups operations by volume, `objc2-foundation` `NSURL`. | None. | Good for HDD and NAS users. Not daily pain for SSD users. |
| Conflict dialog with size, date, thumbnail, "rename new" (05 #20) | ADD-LATER | T | Own, thumbnails from existing `quicklook_macos.rs`. | None. | Finder's dialog is already weak. Parity plus a little more. |
| Multi-level persistent undo with history (05 #21) | ADD-LATER | D | Own journal. `NSUndoManager` adds nothing in iced. | None. | Nice. Not expected. |
| Eject and unmount with "in use" detail | ADD-NOW | T | `NSWorkspace.unmountAndEjectDeviceAtURL:error:`, `objc2-app-kit`. Dissenter detail from `DADiskUnmount` in `objc2-disk-arbitration`. | DiskArbitration callbacks need a run loop or dispatch queue. Use `DASessionSetDispatchQueue`. | A file manager that cannot eject is unusable with USB drives. Verify the existing eject path uses AppKit, not gvfs. |
| Eject When Finished | ADD-LATER | D | Own flag plus the eject call. | None. | Sequoia feature, small. |
| Compress in Finder's zip layout | ADD-LATER | T | `ditto -c -k --sequesterRsrc --keepParent`, spawn. Or `AppleArchive` through `objc2-apple-archive` for `.aar`. | Hardened runtime allows spawning Apple binaries. | Existing zip support exists. Matching Finder's `__MACOSX` layout matters when the archive goes to another Mac. |
| Browse archives as folders (05 #50) | ADD-LATER | D | Own, `zip` and `tar` crates. | None. | Finder cannot do it. Strong differentiator, large. |
| Rename multiple with regex, counters, metadata (05 #22) | ADD-LATER | T | Own sheet. EXIF dates from `MDItemCopyAttribute(kMDItemContentCreationDate)`, `objc2-core-services` `MDItem`. | None. | Finder has it. Parity is table stakes, regex is the delighter. |
| New Folder with Selection (05 #45) | ADD-NOW | T | Own. | None. | Finder has it, two lines of code. |
| New file from templates (05 #10) | ADD-NOW | D | Own. Templates dir under Application Support. | None. | Finder cannot create an empty file. Cheap win. |
| Cut dims items, Cmd+X then Cmd+V moves (05 #1) | ADD-NOW | T | Own. | None. | `Action::Cut` exists. Finish the visual state. |
| Copy path variants (05 #2) | ADD-NOW | D | Own. | None. | Cheap. |
| Open or edit as administrator (05 #55) | IGNORE for now | D | `SMAppService` daemon plus XPC, `objc2-service-management`. | A privileged helper needs Developer ID, a launchd plist in the bundle, and user approval in Login Items. Notarization reviews it. | Large and risky before Developer ID exists. Revisit after signing. |
| Never write `.DS_Store`, skip `._*` on FAT and SMB (05 #26) | ADD-NOW | D | Own copy filter. `copyfile` with `COPYFILE_NOFOLLOW` already skips AppleDouble on non-APFS. | None. | The app already writes none. The copy filter is small. |

### Metadata Finder users expect

| Feature | Verdict | Stakes | API + crate | Platform risks | Reason |
|---|---|---|---|---|---|
| Tags read and write, colour dots, Finder-compatible (05 #40) | ADD-NOW | T | `NSURL.resourceValuesForKeys([NSURLTagNamesKey])` and `setResourceValue`, `objc2-foundation` `NSURL`. Colour index is in the tag string `Name\n<n>`. | Finder's master tag list is `~/Library/SyncedPreferences/com.apple.finder.plist`, private. Read it for names and order, never write it. | Skipping tags breaks the user's existing organisation. Any Mac file manager without tags reads as a Linux port. |
| Tag sidebar and tag search | ADD-NOW with tags | T | `MDQueryCreate` with `kMDItemUserTags == "Red"`, `objc2-core-services` `MDQuery`, results on a dispatch queue. | Spotlight must have indexed the volume. Network volumes are not indexed. | Tags without a way to find tagged files are half a feature. |
| Spotlight comments | ADD-LATER | D | Read `MDItemCopyAttribute(kMDItemFinderComment)`. Write the `com.apple.metadata:kMDItemFinderComment` xattr as a binary plist, `xattr` plus `plist` crates. | Finder also mirrors the comment to `.DS_Store`. Writing the xattr alone is enough for Spotlight. | Rarely used. Show in Get Info first. |
| Get Info window: kind, size, where, dates, open with, locked, hide extension, stationery | ADD-NOW | T | `NSURLLocalizedTypeDescriptionKey`, `NSURLIsUserImmutableKey` (read-write), `NSURLHasHiddenExtensionKey` (read-write), `NSURLAddedToDirectoryDateKey`, `objc2-foundation` `NSURL`. Stationery is bit `0x0800` in `com.apple.FinderInfo`, `xattr` crate. | None. | The preview pane covers half. Cmd+I users expect the rest. |
| Open With submenu and Change All | ADD-NOW | T | `NSWorkspace.URLsForApplicationsToOpenURL:`, `setDefaultApplicationAtURL:toOpenContentType:completionHandler:`, `setDefaultApplicationAtURL:toOpenFileAtURL:`, `objc2-app-kit` `NSWorkspace` with `UTType`. | Forum reports of `permErr` on some content types. Test on 26.6. Per-file default needs write access to the file. | The current Open With uses MIME apps from the freedesktop database, which knows nothing about Mac apps. LaunchServices is the only correct source here. |
| Sort and group by Kind, Date Added, Date Last Opened (05 #4) | ADD-NOW | T | `NSURLLocalizedTypeDescriptionKey`, `NSURLAddedToDirectoryDateKey`, `kMDItemLastUsedDate`. | Kind strings are localised by the system. Sort on `UTType` identifier, display the localised string. | Most-reacted upstream request. Date Added is Finder-only and users miss it elsewhere. |
| Show item info and icon preview | ADD-LATER | D | `kMDItemPixelWidth`, `kMDItemDurationSeconds`, `NSURLDirectoryEntryCountKey`. | `MDItem` reads hit the Spotlight store. Batch per visible page. | Nice second line. Not missed. |
| Media columns (05 #38) | ADD-LATER | D | Same `MDItem` keys. | Same. | Delighter for photographers. |
| Calculate all sizes, cached (05 #23) | ADD-LATER | T | `NSURLTotalFileAllocatedSizeKey` on a walk. | `tab::is_protected_tree` must keep the walk out of `~/Library`, `~/.Trash`, `~/.cache`. | Finder has it per folder. Users turn it on and forget it. |
| Aliases: make, resolve, Show Original | ADD-NOW | T | `NSURL.bookmarkDataWithOptions(.suitableForBookmarkFile)`, `NSURL.writeBookmarkData(_:toURL:options:)`, `NSURL(byResolvingAliasFileAtURL:options:)`, `NSURLIsAliasFileKey`, `objc2-foundation` `NSURL`. | None. Alias files from the Desktop and Dock exist on every Mac. | Double-clicking an existing alias must work or the app looks broken. Making one is small once resolving exists. |
| Symlink badge and Make Symlink (05 #11) | ADD-NOW | D | `std::os::unix::fs::symlink`, `NSURLIsSymbolicLinkKey`. | None. | Developers want it. Finder refuses. |
| Locked flag | ADD-NOW with Get Info | T | `NSURLIsUserImmutableKey`. | Locked items refuse delete and rename. Surface the state in the error. | Finder shows the lock and blocks edits. Users hit it on downloads. |
| Show Package Contents | ADD-NOW | T | `NSURLIsPackageKey`. Treat packages as files in the listing, offer the menu item to browse inside. | The app currently shows `.app` as a folder, which Mac users read as broken. | Table stakes for the Applications folder. |
| Folder sizes, Spotlight-backed content search with kind and date filters (05 #41) | ADD-LATER | T | `MDQueryCreate` with a predicate string, `MDQuerySetSearchScope`, `objc2-core-services`. | Results stream on a dispatch queue. Default to the current folder, not This Mac. | Finder search is Finder's strength. Parity needed, but the own recursive search covers the basic case today. |
| Smart Folders | ADD-LATER | D | Parse `.savedSearch` plist `RawQuery` and `SearchScopes`, run with `MDQuery`. | The `SearchCriteria` blob is Finder-private. Own criteria editor. | Few users build them. Opening existing ones should work. |
| Recents | ADD-LATER | T | `MDQuery` on `kMDItemLastUsedDate` within home. | Exclude `~/Library`. | Sidebar item Finder users click daily. The existing Recents is the app's own history, which differs. |

### Cloud and volumes

| Feature | Verdict | Stakes | API + crate | Platform risks | Reason |
|---|---|---|---|---|---|
| Browse iCloud Drive and File Provider domains | ADD-NOW | T | Plain paths under `~/Library/Mobile Documents/com~apple~CloudDocs` and `~/Library/CloudStorage`. Sidebar name and icon from the owning app bundle via `NSWorkspace.icon(forFile:)`. | Dataless files block on read. Never thumbnail or hash a dataless item. Check `NSURLUbiquitousItemDownloadingStatusKey` first. | Every Mac has iCloud Drive. A file manager that hangs on it is broken. |
| Cloud status badges | ADD-LATER | T | iCloud: `NSURLUbiquitousItemDownloadingStatusKey`, `NSURLUbiquitousItemIsUploadingKey`, `NSURLUbiquitousItemIsSharedKey`. Third-party domains: dataless state only. | Decoration identifiers of other providers are not readable. Approximate. | Users judge sync state from the badge. Exact for iCloud, approximate for Dropbox. |
| Download Now, Remove Download | ADD-LATER | T | `NSFileManager.startDownloadingUbiquitousItemAtURL:error:`, `evictUbiquitousItemAtURL:error:`, `objc2-foundation`. | Evict works for iCloud only. Other providers only through their own app or `fileproviderctl`. | Finder has both in the context menu. |
| Keep Downloaded pin | IGNORE | D | None public. | Private to Finder. | Cannot be built. |
| Connect to Server with Bonjour browse (05 #33) | ADD-LATER | T | `NetFSMountURLAsync`, hand-written `extern "C"` against `NetFS.framework` unless an `objc2-net-fs` crate exists. Bonjour through `NWBrowser` in `objc2-network`. Credentials in Keychain through `objc2-security`. | The gvfs path is Linux-only. On macOS mount through NetFS, then browse the mount point. | Cmd+K is table stakes for NAS users. The current gvfs code does not run here. |
| Never freeze on a stalled mount (05 #25) | ADD-NOW | T | Own. Listing on a worker with a timeout. `NSURLVolumeIsLocalKey` tells you when to apply it. | compio reads block a worker, not the UI, but a hung `stat` on the UI thread still freezes. Audit every synchronous `metadata` call in the view path. | The most emotional Finder complaint. Any regression here loses the user. |
| Saved servers in the sidebar | ADD-LATER | D | Own list of URLs plus NetFS mount on click. | None. | After Connect to Server. |

### Views, window, navigation

| Feature | Verdict | Stakes | API + crate | Platform risks | Reason |
|---|---|---|---|---|---|
| Column view (05 #34) | ADD-LATER | T | Own widget. | None. | Most-reacted upstream issue. Long-time Mac users call it the Finder view. Large. |
| Expandable tree rows in list view (05 #35) | ADD-LATER | T | Own. | None. | Finder parity. |
| Per-folder view memory in an own store (05 #16) | ADD-NOW | T | Own, keyed by path or by `NSURLFileResourceIdentifierKey` so renames keep settings. | Never write `.DS_Store`. Reading Finder's with a reverse-engineered parser is optional and I would skip it. | Finder's drift is a top complaint. An own store that never pollutes volumes is the differentiator. |
| Icon positions, free arrange | IGNORE | D | `.DS_Store` private. | Private. | Own store would not match Finder, and nobody arranges icons inside windows any more. |
| Path bar, status bar with selection size and free space (05 #5) | ADD-NOW | T | `NSURLVolumeAvailableCapacityForImportantUsageKey` gives Finder's exact free-space number, `objc2-foundation`. | `statvfs` reports a different number from Finder. Use the key. | Small and visible. Matching Finder's number avoids "the app is wrong" reports. |
| Toolbar customisation | ADD-LATER | D | Own. | None. | Nobody leaves Finder over it. |
| Liquid Glass chrome | IGNORE for now | D | `NSGlassEffectView`, `NSBackgroundExtensionView`. Not in `objc2-app-kit 0.3`; needs a newer release or hand bindings. | Needs AppKit views around the iced surface. winit gives one content view. | Months of work for looks. A cheap partial exists: `titlebarAppearsTransparent` and `fullSizeContentView` through `with_ns_window`. Do that only. |
| Restore tabs, windows, scroll on launch (05 #14) | ADD-NOW | T | Own session file. `NSWindow` frame through `with_ns_window` if iced loses it. | None. | Mac apps restore state. Users notice when one does not. |
| Dual pane, F5 and F6 (05 #15) | ADD-LATER | D | Own. | None. | The reason people buy ForkLift. Large. |
| Quick filter in place (05 #6) | ADD-NOW | D | Own. | None. | Small, daily use. |
| Cmd+1..9 to favourites, sidebar reorder and rename (05 #9, #32) | ADD-NOW | T | Own. Finder's favourites file `com.apple.LSSharedFileList.FavoriteItems.sfl3` is a keyed archive of bookmark data. Read with `NSKeyedUnarchiver` and `NSURL(byResolvingBookmarkData:)`, `objc2-foundation`. Read-only import. | Import only. Never write the sfl3. | Importing the user's Finder sidebar on first run removes the biggest "set it all up again" cost. |
| Go to Folder with fuzzy completion, frecency (05 #37) | ADD-LATER | D | Own. | None. | Cmd+Shift+G exists. Completion is polish. |
| Command palette (05 #36), keybinding editor (05 #42) | ADD-LATER | D | Own. Shortcuts also appear in System Settings only through real `NSMenu` items with key equivalents. | The menu bar is libcosmic's. Finder-parity shortcuts live in `key_bind.rs`. | Keyboard users love it. Not Finder parity. |
| Hidden files, Library toggle, junk hide rules (05 #18) | ADD-NOW | T | `NSURLIsHiddenKey`. Library shows through the hidden flag. | None. | Cmd+Shift+. is reflex. |
| Folders-first toggle in the View menu (05 #17) | ADD-NOW | T | Own, exists. | None. | Confirm it is in the macOS menu bar. |
| Reuse an existing tab (05 #29), recent locations on Back (05 #28), configurable Enter (05 #31), stem-only rename (05 #30) | ADD-NOW | T | Own. | None. | Each is small. Rename selection and Enter behaviour are things switchers judge in the first five minutes. |
| Merge all windows, one window across Spaces (05 #56) | ADD-LATER | D | `NSWindow.collectionBehavior` with `canJoinAllSpaces` through `with_ns_window`, `objc2-app-kit` `NSWindow`. | Main thread. Already have the helper. | Small once wanted. |

### Actions, sharing, system integration

| Feature | Verdict | Stakes | API + crate | Platform risks | Reason |
|---|---|---|---|---|---|
| Quick Look panel on Space | ADD-NOW | T | `QLPreviewPanel.sharedPreviewPanel`, `QLPreviewPanelDataSource`, `QLPreviewPanelDelegate`, crate `objc2-quick-look-ui`. | Needs the inserted `NSResponder` that answers `acceptsPreviewPanelControl:`. Main thread. The panel takes key focus, so arrow keys go to it. | The app has thumbnails, not the panel. Space on a file is the single most used Finder gesture. Other apps' Quick Look extensions come free. |
| Share menu and AirDrop | ADD-NOW | T | `NSSharingServicePicker(items:)`, `showRelativeToRect:ofView:preferredEdge:`, and `NSSharingService(named: .sendViaAirDrop)`, `objc2-app-kit` `NSSharingService`, `NSSharingServicePicker`. | Needs the `NSView` from the raw window handle and a rect in view coordinates. Main thread. iced coordinates are top-left, AppKit bottom-left: flip. | AirDrop from the file manager is how Mac users move files to a phone. |
| Quick Actions and Services (05 #8 partly) | ADD-LATER | T | `NSPerformService(name, pasteboard)`, `NSApplication.setServicesMenu`, `registerServicesMenuSendTypes:returnTypes:`, `objc2-app-kit`. Listing through `/System/Library/CoreServices/pbs -dump_pboard`. | The services menu populates only when a responder in the chain answers `validRequestorForSendType:`. Same inserted responder. Finder's built-in Rotate, Markup and Create PDF are Finder-internal. | User-made Quick Actions and Shortcuts appear in Finder's menu. Missing them breaks workflows. Rebuild rotate and convert with `objc2-image-io` later. |
| Open terminal here with app choice, open in editor (05 #7, #8) | ADD-NOW | D | `NSWorkspace.openURLs:withApplicationAtURL:configuration:completionHandler:`, `objc2-app-kit`. Terminal apps take a folder URL. | Ghostty and WezTerm accept a folder as the document. iTerm2 needs its URL scheme or AppleScript. | Developers expect it. |
| Drag out to other apps, file promises | ADD-LATER | T | `NSView.beginDraggingSessionWithItems:event:source:`, `NSPasteboardItem` with `NSPasteboardTypeFileURL`, `NSFilePromiseProvider`, `objc2-app-kit`. | Needs the real `NSEvent` of the mouse-down. `gesture_macos.rs` already runs an event monitor and can keep the last one. libcosmic's DnD is in-process on macOS. Verify by dragging a file to Mail. If it fails, this becomes ADD-NOW. | Dragging a file onto Mail or Slack is daily use. If it does not work today, it is the biggest broken thing in the app. |
| Drag modifiers: Option copy, Cmd move, Cmd+Option alias | ADD-LATER | T | `NSEvent.modifierFlags` at drop time, `objc2-app-kit` `NSEvent`. | Depends on the native drop session. | Finder reflex. |
| Spring-loaded folders (05 #46) | ADD-NOW, exists as hover | T | Own. `NSSpringLoadingDestination` is view-based and winit's view does not conform. | Skip the protocol. Hover delay from Accessibility > Pointer Control is `NSUserDefaults -g com.apple.springing.delay`. | Already done. Read the system delay for parity. |
| Services menu in the app menu | ADD-LATER | D | As Quick Actions. | Same responder. | Comes free with the Quick Actions work. |
| Handoff | IGNORE | D | `NSUserActivity`. | Needs Team ID and an iOS counterpart. | No iPhone app. |
| AppleScript dictionary | IGNORE | D | `NSScriptSuite`, `.sdef`. Needs ObjC classes per scriptable object. | Cocoa Scripting wants KVC-compliant ObjC objects. Large shim. | Only Finder automation scripts would want it, and they say `tell application "Finder"`. |
| App Intents, Spotlight actions, Shortcuts (Tahoe) | ADD-LATER | D | `AppIntents`, Swift only. A Swift static library linked into the Rust binary, intents discovered from the bundle's metadata. | Needs an Xcode toolchain step in `macos-bundle.sh` and the intents metadata extraction. | Tahoe's headline feature. Worth it after Developer ID. |
| Shortcuts folder automations | IGNORE | n/a | None. The Shortcuts app watches folders itself. | None. | System provides it. |
| VoiceOver and Accessibility | ADD-LATER | T | `NSAccessibility` protocols. For iced, an AccessKit bridge to AppKit, `accesskit_macos`. | Custom-drawn UI has no tree. A deep job. | Required for a Finder replacement to be a real replacement. Not blocking early adopters. |
| Default folder opener | ADD-NOW | T | See (b). | See (b). | Without the `odoc` handler the Open With entry does nothing. |
| `NSFileViewer` for Reveal in Finder | ADD-LATER | D | Undocumented `-g NSFileViewer`. | Unsupported. The event format must be observed. | See (b). |
| Desktop, Stacks | IGNORE | D | None. Own desktop-level window at `kCGDesktopIconWindowLevel`. | Finder owns the Desktop. Mission Control and iCloud Desktop stay Finder's. | Not worth fighting Finder for. |
| Open and Save panels | IGNORE | n/a | None. Panels run out of process. | None. | Cannot be replaced. See `docs/save-dialog-replacement.md`. |
| Finder Sync extensions from Dropbox-style apps | IGNORE | D | None. | Hosted by Finder only, and 26.1 broke them on ARM anyway. | Cannot be hosted. |
| Burn | IGNORE | D | `DiscRecording`. | Hardware is gone. | No. |
| Icon Composer `.icon` for the app icon, Tahoe icon themes | ADD-LATER | D | `.icon` bundle via Icon Composer, `CFBundleIconName`. | Needs the Xcode tool once. | The app icon looks out of place in Tinted and Clear modes without it. |
| Checksum panel (05 #12), select by pattern (05 #13), copy listing (05 #47), flat view (05 #48), shelf (05 #49), duplicate finder (05 #52), folder compare (05 #51), git status (05 #39), embedded terminal (05 #54), image ops (05 #53) | ADD-LATER | D | Own. Image ops through `objc2-image-io` and `objc2-core-graphics`. | None. | Differentiators. Nothing platform-specific blocks them. Order by effort after the table-stakes list. |
| Secure Empty Trash, Keep Downloaded, `.DS_Store` writing | IGNORE | n/a | None. | Removed, private, or harmful. | No. |

## Top 12 in priority order

Order: broken-on-sight first, then Finder reflexes, then the integration that makes the app the default.

| # | Feature | Why here | Size |
|---|---|---|---|
| 1 | Trash through `recycleURLs` plus own Put Back journal | Today every delete asks to control Finder and is denied in a bundle. | S |
| 2 | Quick Look panel on Space | The most used Finder gesture. One `NSResponder` insert unlocks it and Services. | M |
| 3 | Tags read and write, tag search | Without them the app ignores how the user already organised their files. | M |
| 4 | Open With and Change All through LaunchServices | The freedesktop MIME list knows no Mac apps. Wrong default apps look broken. | S |
| 5 | Aliases resolve and Show Package Contents | Existing aliases and `.app` bundles must behave. Both small. | S |
| 6 | APFS clone for copy and duplicate | Finder duplicates instantly. First-minute impression. | S |
| 7 | iCloud Drive without hangs, dataless awareness | Every Mac has it. A hang here is a force-quit. | M |
| 8 | Default folder opener: `odoc` handler plus the Settings switch | Makes the "Finder replacement" claim real for Dock, Spotlight and `open`. | S |
| 9 | Share menu and AirDrop | Phone transfer is daily. One picker call. | S |
| 10 | Finder-side folder looks: colour as tag, glyph as xattr, reader for all three | Finder-customised folders must look right here, and ours should show there. | M |
| 11 | Per-folder view memory, status bar with Finder's free-space number, session restore | The view-state complaints that drive people off Finder, answered without `.DS_Store`. | M |
| 12 | Drag out to other apps with file promises | Verify first. If libcosmic's DnD does not reach Mail today, move this to #2. | M |

Column view, dual pane and VoiceOver are the three large items waiting behind these.
