# Finder feature catalogue and the Apple APIs behind them

Scope: macOS 26 Tahoe and later. Target: a third-party, non-sandboxed, Developer-ID-signed file manager (Macosmetic). Researched 2026-10-09.

## How to read the tables

| Column | Meaning |
|---|---|
| Third-party feasible | `yes`, `partial` or `no`, then the reason. |
| Effort code in brackets | S = days, M = 1 to 3 weeks, L = 1 to 2 months, XL = more than 2 months or needs native Swift/ObjC glue. |
| API / format | Public Apple API first. Undocumented formats are marked "private format". |

Rule of thumb from the research. Anything that touches files, metadata, Spotlight, Quick Look, sharing, mounting, or Launch Services is open. Anything that lives inside Finder's process (Desktop, Finder extensions, Dock integration, Put Back records, Open/Save panels) is closed.

## 1. Views and per-folder view options

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Icon view | Grid of icons, free or snap-to-grid arrangement, icon positions remembered per folder | Own rendering. Positions stored by Finder in `.DS_Store` (private format, reverse-engineered) | yes (M). Reading `.DS_Store` positions is optional; a reverse-engineered parser exists | https://en.wikipedia.org/wiki/.DS_Store |
| List view | Columns (Name, Date Modified, Size, Kind, Tags...) with sort, resizable columns, disclosure triangles | `URLResourceKey` values via `URL.resourceValues(forKeys:)`; `NSTableView`/own widget | yes (M) | https://developer.apple.com/documentation/foundation/urlresourcekey |
| Column view | Miller columns, preview column at the end | Own widget. AppKit has `NSBrowser` | yes (M) | https://developer.apple.com/documentation/appkit/nsbrowser/allowstypeselect |
| Gallery view | Big preview plus thumbnail strip plus metadata | `QLThumbnailGenerator` (QuickLookThumbnailing) for thumbnails, `MDItem` for metadata | yes (M) | https://developer.apple.com/documentation/quicklookui/qlpreviewpanel |
| View Options per folder (Cmd-J) | Icon size, grid spacing, text size, label position, show item info, show icon preview, background, "Always open in X view", "Browse in X view", "Use as Defaults" | Finder stores all of this in `.DS_Store` per folder (private format). Nothing public for writing | partial (M). Own store for own settings is easy. Reading Finder's `.DS_Store` works with a reverse-engineered parser. Writing it is risky and unsupported | https://sindresorhus.com/ds-store-inspector |
| Calculate all sizes | Folder sizes in list view Size column, computed lazily, per-folder setting | Directory walk with `FileManager.enumerator` or `URLResourceKey.fileAllocatedSizeKey`; `.totalFileAllocatedSizeKey` | yes (S). Background thread with cancellation | https://support.apple.com/en-us/guide/mac-help/mchlp1745/14.0/mac/14.0 |
| Sort By / Group By (Use Groups) | Sort by name, kind, date opened/added/modified/created, size, tags. Groups are fixed-order sections (Kind, Date, Size, Tags, Application) | `URLResourceKey` dates (`contentModificationDateKey`, `creationDateKey`, `addedToDirectoryDateKey`), `MDItemLastUsedDate` for Date Last Opened | yes (S) | https://support.apple.com/en-us/guide/mac-help/mchlp1745/14.0/mac/14.0 |
| Clean Up / Clean Up By | Snaps icons to grid, optional sort | Own layout | yes (S) | https://support.apple.com/en-us/guide/mac-help/mchlp1745/14.0/mac/14.0 |
| Show item info | Second line under icon (dimensions, item count, duration) | `MDItemPixelWidth/Height`, `MDItemDurationSeconds`, `directoryEntryCountKey` | yes (S) | https://developer.apple.com/documentation/foundation/urlresourcekey |
| Show icon preview | Thumbnail instead of generic icon | `QLThumbnailGenerator.generateBestRepresentation` | yes (S) | https://developer.apple.com/documentation/quicklookui/qlpreviewpanel |
| Show Library Folder items (Sequoia) | View Options checkbox that unhides `~/Library` in Home | Toggle `isHiddenKey` filtering for that one path (chflags nohidden equivalent) | yes (S) | https://macmost.com/whats-coming-in-macos-sequoia.html |
| Hidden files toggle (Cmd-Shift-.) | Show dotfiles and `hidden` flag items | `URLResourceKey.isHiddenKey`; Finder pref `com.apple.finder AppleShowAllFiles` | yes (S) | https://en.wikipedia.org/wiki/.DS_Store |

## 2. Window chrome

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Sidebar: Favorites | User-pinned folders, drag to reorder | Finder keeps these in `com.apple.LSSharedFileList.FavoriteItems.sfl3` (private format, bookmark data inside). Own list is trivial | partial (S). Own favourites: yes. Importing Finder's: parse sfl3 (keyed archive of bookmark data), read-only | https://support.apple.com/en-me/guide/mac-help/aside/glos200755a1/26/mac/26 |
| Sidebar: iCloud / Locations / Tags | Tahoe's guide lists three sections: Favorites, Locations, Tags (iCloud folded into Favorites). Locations lists volumes, network, File Provider domains | Volumes: `FileManager.mountedVolumeURLs`, DiskArbitration callbacks. Network: Bonjour `_smb._tcp` via `NWBrowser`. File Provider domains appear under `~/Library/CloudStorage` | yes (M) | https://support.apple.com/en-me/guide/mac-help/aside/glos200755a1/26/mac/26 |
| Path bar | Breadcrumb at bottom, drag targets, Option-click copies POSIX path | Own widget | yes (S) | https://support.apple.com/guide/mac-help/mchlp1236 |
| Status bar | Item count, selection count, free space | `volumeAvailableCapacityForImportantUsageKey` matches Finder's number | yes (S) | https://developer.apple.com/documentation/foundation/urlresourcekey |
| Tabs | Multiple folders per window, Merge All Windows, drag tab out | Own tab bar | yes (M) | https://support.apple.com/guide/mac-help/mchlp1745/14.0/mac/14.0 |
| Toolbar customisation | Drag items in/out, Customize Toolbar sheet | Own toolbar model. AppKit gives `NSToolbar` for free; iced needs a custom editor | yes (M) | https://developer.apple.com/videos/play/wwdc2025/310/ |
| Preview pane | Right-hand inspector with large preview, metadata list ("Show Preview Options" picks which keys), Quick Action buttons | `QLPreviewView` (QuickLookUI) for the image; `MDItemCopyAttribute` for metadata | yes (M) | https://support.apple.com/guide/mac-help/mchl97ff9142/ |
| Liquid Glass chrome (Tahoe) | Translucent floating toolbar and sidebar, scroll-edge effect, content extends under sidebar | AppKit: `NSGlassEffectView`, `NSBackgroundExtensionView`, `NSSplitView` safe-area under sidebar, `NSToolbar` glass items. SwiftUI equivalents | partial (L). Needs AppKit host window around the iced surface. 26.1 added a user "Tinted" option that the app must respect | https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass |

## 3. Get Info window

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| General fields | Kind, Size, Where, Created, Modified, version, Spotlight comment, Tags | `localizedTypeDescriptionKey`, `fileSizeKey`, dates, `MDItemFinderComment` | yes (S) | https://developer.apple.com/forums/thread/680925 |
| Spotlight comments (Comments field) | Free text indexed by Spotlight | Stored in `com.apple.metadata:kMDItemFinderComment` xattr (binary plist) plus `.DS_Store` copy. Read: `MDItemCopyAttribute(kMDItemFinderComment)`. Write: xattr write; Finder also mirrors it, Spotlight reindexes from xattr | yes (S) | https://www.macworld.com/article/187618/handcode.html |
| Open with + Change All | Default app for this file or for the whole type | `NSWorkspace.urlsForApplications(toOpen:)`, `setDefaultApplication(at:toOpenFileAt:)` (one file, macOS 12+), `setDefaultApplication(at:toOpen: UTType)` (Change All, macOS 12+). Forum reports `toOpen:` content-type variant returning permErr in some cases | yes (S). Test the type-wide call | https://developer.apple.com/documentation/appkit/nsworkspace/setdefaultapplication(at:toopenfileat:completion:) |
| Sharing & Permissions | Owner/group/everyone POSIX, ACL entries, "Apply to enclosed items", lock icon | `FileManager.setAttributes` (`posixPermissions`, owner), ACLs via `acl(3)` C API. Admin rights need `AuthorizationExecuteWithPrivileges` replacement (XPC helper + SMAppService) | yes (M). Privileged changes need a helper | https://support.apple.com/en-gb/guide/mac-help/-mchlp1203/mac |
| Locked | Sets the user-immutable flag; Finder refuses edits/deletes | `URLResourceKey.isUserImmutableKey` (read-write); `chflags uchg` | yes (S) | https://developer.apple.com/documentation/foundation/urlresourcekey |
| Stationery pad | Opening creates a copy | Finder flag `kIsStationery` (0x0800) in `com.apple.FinderInfo` xattr; no `URLResourceKey` | yes (S). Write the FinderInfo bit | https://support.apple.com/en-sa/guide/mac-help/mchlp1341/15.0/mac/15.0 |
| Hide extension | Per-file hidden extension | `URLResourceKey.hasHiddenExtensionKey` (read-write) | yes (S) | https://developer.apple.com/documentation/foundation/urlresourcekey |
| Custom icon paste / Cut / Delete | Paste image onto icon well; Delete restores | `NSWorkspace.setIcon(_:forFile:options:)`, `NSWorkspace.icon(forFile:)`. On disk: `Icon\r` file in folder (or resource fork of file) holding icns resource -16455 plus `kHasCustomIcon` bit in `com.apple.FinderInfo`. Setting icon on a signed app bundle breaks its signature | yes (S). `setIcon` does the whole write; Finder may show a stale desktop icon until relaunch | https://developer.apple.com/documentation/appkit/nsworkspace/iconcreationoptions |
| Custom folder icon via Icon\r (manual) | Same result without AppKit | Write `Icon\r` + `com.apple.ResourceFork` xattr + FinderInfo flag | yes (M). Use `setIcon` instead unless pure Rust is required | https://eclecticlight.co/2023/03/04/custom-finder-icons-resources-and-mac-os-history/ |
| Customize Folder (Tahoe) | Colour, SF Symbol or emoji on a folder icon; syncs via iCloud; Spotlight/Dock show plain blue; aliases do not inherit | Stored in xattr `com.apple.icon.folder#S` as JSON, e.g. `{"sym":"camera.viewfinder"}` or `{"emoji":"..."}`. Colour comes from the tag colour. No public API; private format | partial (M). Read the xattr and render with SF Symbols yourself. Writing the xattr works today but is undocumented. `NSWorkspace.icon(forFile:)` returns the composited icon (verify on 26.x) | https://mjtsai.com/blog/2025/07/11/macos-tahoes-folder-icon-customization/ |
| Customize Folder: Apple's steps | Action menu > Customize Folder; Clear resets | Apple user guide | reference | https://support.apple.com/guide/mac-help/mchlp2313/mac |
| Preview in Get Info | Thumbnail at top | `QLThumbnailGenerator` | yes (S) | https://developer.apple.com/documentation/quicklookui/qlpreviewpanel |

## 4. Tags, Smart Folders, Spotlight, Recents

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Tags (read/write) | Multiple named tags per item, shown as coloured dots | `URLResourceKey.tagNamesKey` (read-write array of String). On disk: `com.apple.metadata:_kMDItemUserTags` binary plist, each entry `"Name\n<colourIndex>"` | yes (S) | https://developer.apple.com/documentation/foundation/urlresourcekey/tagnameskey |
| Tag colours and the tag list | Seven colours (0 none, 1 gray, 2 green, 3 purple, 4 blue, 5 yellow, 6 red, 7 orange), user's tag set and sidebar order | Colour index lives in the tag string. The master list is Finder's `~/Library/SyncedPreferences/com.apple.finder.plist` (`FavoriteTagNames`, private) | partial (S). Colours per item: yes. Finder's global tag list: read-only parse | https://support.apple.com/guide/mac-help/mchlp15236 |
| Tags in sidebar, Ctrl-1..7 shortcuts | Click a tag to search; keys toggle favourite tags | Spotlight query `kMDItemUserTags == "Red"` | yes (S) | https://support.apple.com/guide/mac-help/mchlp15236 |
| Smart Folders | Saved live Spotlight query shown as a folder | `.savedSearch` file: plist with `RawQuery`, `SearchCriteria`, `SearchScopes`. Run with `NSMetadataQuery` (predicate + `searchScopes`) | yes (M). Parse `RawQuery` and scopes; the `SearchCriteria` UI blob is Finder-private, so build your own criteria editor | https://developer.apple.com/library/mac/documentation/Carbon/Conceptual/SpotlightQuery/Concepts/QueryingMetadata.html |
| Spotlight search in window | Search field, "This Mac / current folder", filter tokens (Kind, Date, Name), save as Smart Folder | `NSMetadataQuery` / `MDQueryCreate`; attribute list `kMDItem*` | yes (M) | https://developer.apple.com/library/content/documentation/Carbon/Conceptual/SpotlightQuery/Concepts/Introduction.html |
| Metadata keys reference | Full `kMDItem*` list | Apple Metadata Attributes Reference (archive) | reference | https://developer.apple.com/library/archive/documentation/CoreServices/Reference/MetadataAttributesRef/Index/index_of_book.html |
| Recents | Sidebar item listing recently used files | Spotlight query on `kMDItemLastUsedDate` (Finder's `Recents` is a saved search over the home and volumes, excluding system) | yes (S) | https://developer.apple.com/library/mac/documentation/Carbon/Conceptual/SpotlightQuery/Concepts/QueryingMetadata.html |

## 5. Aliases and symlinks

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Make Alias | Creates a Finder alias file that survives moves of the target | `URL.bookmarkData(options: .suitableForBookmarkFile)` then `URL.writeBookmarkData(_:to:)`. Alias files carry `kIsAlias` FinderInfo bit and `isAliasFileKey` = true | yes (S) | https://developer.apple.com/forums/thread/12972 |
| Resolve alias / Show Original | Opens target or selects it | `URL(resolvingAliasFileAt:options:)` or `URL(resolvingBookmarkData:options:relativeTo:bookmarkDataIsStale:)`; `URLResourceKey.isAliasFileKey` | yes (S). Pass the alias file URL as `relativeTo` | https://indiestack.com/?p=514 |
| Symbolic links | POSIX symlink, shown with alias badge in Finder | `FileManager.createSymbolicLink(at:withDestinationURL:)`, `isSymbolicLinkKey` | yes (S) | https://developer.apple.com/library/archive/documentation/cocoa/Conceptual/LowLevelFileMgmt/Articles/FileManagementNSURL.html |
| Alias format history and behaviour | Bookmark-based since 10.6; old alias records | Background reading | reference | https://eclecticlight.co/2024/08/24/a-brief-history-of-the-finder-alias/ |

## 6. Drag, drop, desktop

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Drag modifiers | Default move within a volume, copy across volumes. Option = copy, Cmd = move, Cmd+Option = alias | Destination returns `NSDragOperation` based on `NSEvent.modifierFlags`; source `draggingSession(_:sourceOperationMaskFor:)` | yes (S). iced's drag layer needs the native drop session to read modifiers | https://developer.apple.com/documentation/appkit/drag-and-drop |
| Drag out to other apps | Files as URLs or promises (e.g. from cloud/archives) | `NSPasteboard` file URLs; `NSFilePromiseProvider` for not-yet-materialised files; receive with `NSFilePromiseReceiver` | yes (M) | https://developer.apple.com/documentation/appkit/supporting-collection-view-drag-and-drop-through-file-promises |
| Spring-loaded folders | Hover or force-click a folder during drag to open it; Space opens at once; speed in Accessibility > Pointer Control | `NSSpringLoadingDestination` protocol, `NSSpringLoadingOptions`, `NSDraggingInfo.springLoadingHighlight` | yes (M). Protocol is AppKit-view based; a custom iced surface must implement it on the host `NSView` | https://developer.apple.com/documentation/appkit/nsspringloadingdestination |
| Desktop icons and Stacks | Finder draws the Desktop; Stacks group by kind/date/tag; Ctrl-Cmd-0 | No public API. Finder owns the Desktop window layer. Only knob: `defaults write com.apple.finder CreateDesktop false` hides it | no. A replacement can draw its own desktop-level window at `kCGDesktopIconWindowLevel`, but Dock/Mission Control integration and iCloud Desktop sync stay Finder's | https://support.apple.com/guide/mac-help/organize-files-in-stacks-mh35846/10.15/mac |

## 7. Actions on items

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Quick Look (Space) | Full-size preview panel, arrow keys through selection, full-screen, Open With and Share buttons | `QLPreviewPanel.shared()` with `QLPreviewPanelDataSource`/`Delegate`; panel follows the responder chain, so the host `NSWindow`/`NSResponder` must accept control | yes (M). Quick Look plug-ins (`QLPreviewingController` extensions) from other apps work automatically | https://developer.apple.com/documentation/quicklookui/qlpreviewpanel |
| Quick Actions (context menu + preview pane) | Rotate, Markup, Create PDF, Trim, Convert Image, plus user Automator/Shortcuts Quick Actions | Quick Actions are Services (`~/Library/Services/*.workflow`, Shortcuts with "Use as Quick Action"). Invoke any Service with `NSPerformService(name, pasteboard)`; list via `/System/Library/CoreServices/pbs -dump_pboard` | partial (M). Invoking is public. Finder's built-in ones (Rotate, Markup, Create PDF) are Finder-internal; rebuild with ImageIO / PDFKit / `QLPreviewView` markup is not exposed | https://support.apple.com/guide/mac-help/mchl97ff9142/ |
| How Quick Actions work internally | Services with `NSIconName`, `NSRequiredContext` | Eclectic Light series | reference | https://eclecticlight.co/2019/02/08/quick-actions-4-how-they-work/ |
| Action extensions (NSExtension) | App-provided actions in Share/Action sheets | `NSExtensionPointIdentifier = com.apple.ui-services` (macOS). Hosting them requires `NSExtension` private hosting API | partial. Appear through `NSSharingServicePicker` automatically; direct hosting is private | https://developer.apple.com/documentation/bundleresources/information-property-list/nsextension |
| Finder Sync extensions (badges, menus, toolbar button from Dropbox-style apps) | Overlay badges and context items inside Finder | `FinderSync.framework` is hosted only by Finder. Apple says sync cases moved to Replicated File Provider; Sequoia removed the settings UI; 26.1 ARM breakage reported | no. A third-party browser cannot host other apps' Finder Sync extensions | https://developer.apple.com/documentation/findersync |
| Share menu | System share sheet: Mail, Messages, AirDrop, Notes, third-party share extensions | `NSSharingServicePicker(items:)`, `show(relativeTo:of:preferredEdge:)` | yes (S). Needs an `NSView` anchor | https://developer.apple.com/documentation/appkit/nssharingservice |
| AirDrop | Send selection to nearby device | `NSSharingService(named: .sendViaAirDrop)?.perform(withItems:)` | yes (S) | https://developer.apple.com/documentation/appkit/nssharingservice/init(named:) |
| Compress | Zip via Archive Utility; multiple items become `Archive.zip` | Finder calls Archive Utility (BOM). Equivalents: `ditto -c -k --sequesterRsrc --keepParent`, libarchive, or `Compression`/`AppleArchive` frameworks for other formats | yes (S). Match Finder's zip layout (`__MACOSX` resource fork folder) with `ditto` flags | https://www.howtogeek.com/672240/how-to-zip-and-unzip-files-and-folders-on-mac/ |
| Duplicate (Cmd-D) | Copy next to original, " copy" / " copy 2" naming | `FileManager.copyItem` plus own naming | yes (S) | https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/LowLevelFileMgmt/Articles/FileManagement.html |
| Rename multiple | Replace Text, Add Text (before/after), Format (Name+Index, Name+Counter, Name+Date, start number); Undo Rename | Own sheet plus `FileManager.moveItem`. Undo via own journal | yes (M) | https://macmost.com/batch-rename-multiple-files-on-a-mac.html |
| New Folder with Selection | Makes folder, moves selection in, names it "New Folder With Items" | `FileManager.createDirectory` + move | yes (S) | https://support.apple.com/guide/mac-help/mchlp1745/14.0/mac/14.0 |
| Go to Folder (Cmd-Shift-G) | Path field with fuzzy completion, `~`, Tab accepts, matches anywhere in name, tolerates typos | Own overlay; completion from `FileManager.contentsOfDirectory` plus fuzzy match | yes (S) | https://support.apple.com/guide/mac-help/mchlp1236 |
| Connect to Server (Cmd-K) | smb, afp (read), nfs, ftp, webdav URL; recent servers; browse Bonjour | `NetFSMountURLAsync` (NetFS.framework, public C), `NetFSMountURLSync`; Bonjour browse with `NWBrowser` for `_smb._tcp`; credentials in Keychain | yes (M) | https://support.apple.com/fr-afri/guide/mac-help/mchlp1140/26/mac/26 |
| Eject / Unmount | Eject button in sidebar; Sequoia "Eject When Finished" | `NSWorkspace.unmountAndEjectDevice(at:)`; DiskArbitration `DADiskUnmount`/`DADiskEject` with dissenter info for "in use" | yes (S) | https://developer.apple.com/documentation/appkit/nsworkspace |
| Burn | Burn folder, Burn to disc | `DiscRecording`/`DiscRecordingUI` frameworks (legacy, still ship). Hardware nearly absent | yes (M), low value | https://developer.apple.com/documentation/appkit/nsworkspace |
| Show Package Contents | Treat bundle as folder | `URLResourceKey.isPackageKey` (read-write); just navigate inside | yes (S) | https://developer.apple.com/documentation/foundation/urlresourcekey/ispackagekey |
| Open With submenu | All apps claiming the type, default first, "Other..." | `NSWorkspace.urlsForApplications(toOpen: URL)` / `urlForApplication(toOpen:)`, `open(_:withApplicationAt:configuration:)` | yes (S) | https://developer.apple.com/documentation/appkit/nsworkspace |
| Undo (Cmd-Z) for move/copy/rename/trash | Reverses the last file op | Own undo stack (`NSUndoManager` or Rust equivalent). No system file-op journal | yes (M). Record every op with resulting URLs | https://developer.apple.com/documentation/foundation/filemanager/trashitem(at:resultingitemurl:) |
| Move to Trash | Per-volume `.Trashes/<uid>` or `~/.Trash` | `FileManager.trashItem(at:resultingItemURL:)` (uses file coordination) or `NSWorkspace.recycle(_:completionHandler:)` | yes (S) | https://developer.apple.com/documentation/foundation/filemanager/trashitem(at:resultingitemurl:) |
| Put Back | Restores a trashed item to its original folder | Finder stores original paths in the Trash's `.DS_Store` (private). `trashItem` writes no record; forum reports only the first of several API-trashed items gets Put Back | partial (M). Keep an own journal keyed by trashed URL for own Put Back. Finder's Put Back for items you trashed stays unreliable | https://developer.apple.com/forums/thread/773997 |
| Empty Trash | Deletes every `.Trash`/`.Trashes/<uid>` on all volumes; option to skip warning | `FileManager.removeItem` over trash dirs; TCC may prompt for Full Disk Access on other volumes' trashes | yes (S) | https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/LowLevelFileMgmt/Articles/FileManagement.html |
| Secure Empty Trash | Removed in OS X 10.11 (SSD/TRIM/FileVault) | none | n/a | https://macmost.com/secure-empty-trash.html |

## 8. iCloud Drive and File Provider

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Browse iCloud Drive and third-party clouds | Files appear under `~/Library/Mobile Documents/com~apple~CloudDocs` and `~/Library/CloudStorage/<Provider>` | Plain file APIs; items may be dataless (placeholder) until read | yes (S) | https://developer.apple.com/documentation/fileprovider/replicated-file-provider-extension |
| Cloud status badges (cloud, downloading, uploaded, pinned, shared) | Icon decorations drawn by Finder | Provider side: `NSFileProviderItemDecorating` + `NSFileProviderDecorations` in Info.plist. Browser side: iCloud items expose `ubiquitousItemDownloadingStatusKey`, `ubiquitousItemIsUploadingKey`, `ubiquitousItemIsSharedKey`; for third-party domains only `isUbiquitousItemKey`-style keys and dataless state are visible. Decoration identifiers are not readable by other apps | partial (M). Match iCloud exactly; approximate others from dataless/materialised state | https://developer.apple.com/documentation/fileprovider/nsfileprovideritemdecorating |
| Download Now | Force fetch | `FileManager.startDownloadingUbiquitousItem(at:)` (works for iCloud and File Provider items) | yes (S) | https://developer.apple.com/documentation/foundation/filemanager/startdownloadingubiquitousitem(at:) |
| Remove Download (evict) | Free local copy, keep in cloud | iCloud: `FileManager.evictUbiquitousItem(at:)`. Third-party domains: `NSFileProviderManager.evictItem(identifier:)` is only callable by the provider's own app; `fileproviderctl evict` CLI exists | partial (S). iCloud yes; other providers only through their own app or the CLI | https://developer.apple.com/documentation/fileprovider/nsfileprovidermanager |
| Keep Downloaded (Sequoia+) | Pin item so Optimize Storage never evicts it; pin icon | Provider side: content policy `downloadEagerlyAndKeepDownloaded`. No public call for a browser to set the pin on iCloud items | no for pinning, yes for reading iCloud state. Private API in Finder | https://support.apple.com/guide/mac-help/mchl1a02d711 |
| File Provider "Locations" entries and sidebar icon | Provider domains appear in sidebar with their icon | Provider declares icon; browser can enumerate `~/Library/CloudStorage` and read the bundle icon of the owning app | yes (S) for listing | https://developer.apple.com/videos/play/wwdc2021/10182/ |
| Replicated File Provider background | Where sync apps moved after Finder Sync | WWDC21 session | reference | https://tidbits.com/2023/03/10/apples-file-provider-forces-mac-cloud-storage-changes/ |

## 9. Accessibility, automation, system integration

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| VoiceOver and Accessibility Inspector support | Every view, row, icon exposed with role, label, value, actions | `NSAccessibility` protocols (macOS 10.10+), new `NSAccessibilityScrollToVisibleAction` in 26 | yes (XL for a custom-drawn iced UI). Standard AppKit controls are free; custom surfaces need an accessibility tree (e.g. AccessKit bridge) | https://developer.apple.com/library/archive/samplecode/AccessibilityUIExamples/Listings/ReadMe_md.html |
| Keyboard navigation and type-select | Arrow keys, Cmd-Up/Down, Return to rename, type first letters to select | AppKit `allowsTypeSelect` on `NSTableView`/`NSBrowser`; custom in iced | yes (S) | https://developer.apple.com/documentation/appkit/nstableview/allowstypeselect |
| Services menu | App menu > Services shows Services matching selection | `NSServices` in Info.plist to provide; `NSApplication.servicesMenu` + `registerServicesMenuSendTypes` to consume | yes (S) | https://developer.apple.com/library/mac/documentation/Cocoa/Conceptual/SysServices/Articles/properties.html |
| AppleScript / JXA dictionary | Finder's scripting dictionary (windows, selection, items) | `NSScriptSuite`/`.sdef` + `NSAppleEventsUsageDescription`; Cocoa Scripting requires ObjC classes | partial (L). Needs an ObjC/Swift shim describing the object model | https://www.kodeco.com:443/books/macos-by-tutorials/v1.0/chapters/14-automation-for-your-app |
| Shortcuts and Spotlight actions (App Intents) | Tahoe Spotlight runs app actions with parameters; Shortcuts actions | `AppIntents` framework, Swift-only, compiled into the bundle | partial (L). Requires a Swift target linked into the app | https://developer.apple.com/videos/play/wwdc2025/244 |
| Shortcuts folder automations (Tahoe) | "When items are added/modified/removed in folder" triggers | Shortcuts app feature, not Finder. A file manager only needs to expose intents the automation can call | n/a, system provides it | https://sixcolors.com/post/2025/08/get-started-with-folder-automation-in-macos-tahoe/ |
| Handoff | Continue a folder window on another device | `NSUserActivity`, `NSUserActivityTypes`, same Team ID | yes (S) | https://developer.apple.com/library/mac/documentation/UserExperience/Conceptual/Handoff/HandoffFundamentals/HandoffFundamentals.html |
| Focus filters | App behaviour per Focus mode | `SetFocusFilterIntent` (App Intents, Swift) | partial (M) | https://developer.apple.com/videos/play/wwdc2025/244 |
| Reveal in Finder from other apps | Xcode/Chrome call `NSWorkspace.activateFileViewerSelecting` | Always targets Finder. No hook | no | https://developer.apple.com/forums/thread/726663 |

## 10. Being the default file browser

| Feature | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|
| Desktop | Icons, Stacks, wallpaper click-through | Finder-internal | no. Hide Finder's with `CreateDesktop false`; draw own desktop window | https://support.apple.com/guide/mac-help/organize-files-in-stacks-mh35846/10.15/mac |
| Open / Save panels | Every app's file dialogs | `NSOpenPanel`/`NSSavePanel` run out-of-process (`com.apple.appkit.xpc.openAndSavePanelService`) since 10.15 | no | https://developer.apple.com/documentation/appkit/nsopenpanel |
| Default handler for folders | Double-click folder in Dock, Spotlight, other apps | LaunchServices default role handler for `public.folder` (`duti -s <bundle> public.folder all`, or `NSWorkspace.setDefaultApplication(at:toOpen: .folder)`) | partial (S). Works for many launch paths; Dock "Show in Finder" and `activateFileViewerSelecting` still go to Finder; forum reports it flaps across reboots | https://talk.macpowerusers.com/t/forklift-3-default-file-viewer-with-mac-os-catalina-10-15-7/33989 |
| `NSFileViewer` global default | `defaults write -g NSFileViewer -string <bundle id>` | Undocumented global default some apps honour | partial (S). Unreliable, undocumented | https://talk.macpowerusers.com/t/forklift-3-default-file-viewer-with-mac-os-catalina-10-15-7/33989 |
| Quitting Finder | Finder is relaunched by the system; `defaults write com.apple.finder QuitMenuItem -bool true` adds Quit | Finder pref | partial. Finder restarts on demand (Dock "Show in Finder", Desktop) | https://en.wikipedia.org/wiki/.DS_Store |
| Keyboard shortcuts parity | Cmd-Shift-N, Cmd-Delete, Cmd-Shift-G, Cmd-I, Cmd-Y, Space, Cmd-Up, Cmd-Shift-., Ctrl-Cmd-T (add to sidebar) | Own bindings | yes (S) | https://support.apple.com/guide/mac-help/mchlp1745/14.0/mac/14.0 |

## 11. New in macOS 15 Sequoia and 26 Tahoe

| Feature | Release | What it does | API / format | Third-party feasible | Source URL |
|---|---|---|---|---|---|
| Keep Downloaded | 15 | Pin iCloud items against Optimize Storage | Private (see section 8) | no (pin), yes (read) | https://eshop.macsales.com/blog/96633-how-to-use-keep-downloaded-in-macos-sequoia-to-prevent-files-from-sneaking-back-up-to-icloud/ |
| Eject When Finished | 15 | Volume ejects after a copy from it completes | Own logic + `unmountAndEjectDevice` | yes (S) | https://appleinsider.com/editor/chip+loder |
| Show Library Folder items | 15 | View Options toggle | hidden-flag filter | yes (S) | https://macmost.com/whats-coming-in-macos-sequoia.html |
| Finder Sync settings removed | 15 | Extensions still load but no UI to toggle | n/a | n/a | https://mjtsai.com/blog/2024/10/03/finder-sync-extensions-removed-from-system-settings-in-sequoia |
| Customize Folder: colour, symbol, emoji | 26 | Per-folder look, iCloud-synced, not shown in Spotlight/Dock | `com.apple.icon.folder#S` xattr (private JSON), colour from tag | partial (M) | https://eclecticlight.co/2025/09/18/customising-folders-in-tahoe/ |
| Liquid Glass Finder window | 26 | Floating glass toolbar and sidebar, content under sidebar, 26.1 Tinted option | `NSGlassEffectView`, `NSBackgroundExtensionView`, `NSSplitView` safe areas | partial (L) | https://developer.apple.com/videos/play/wwdc2025/310/ |
| Icon themes (Default, Dark, Clear, Tinted) | 26 | System-wide app icon rendering, applies to Finder's folder icons too | Icon Composer `.icon` bundles for the app's own icon; folder icons come from system | yes (S) for app icon | https://macmost.com/top-10-new-features-in-macos-tahoe-26.html |
| Spotlight actions and Quick Keys | 26 | Run app actions from Spotlight with parameters; two-letter quick keys | App Intents | partial (L) | https://developer.apple.com/videos/play/wwdc2025/244 |
| Shortcuts folder automations | 26 | Folder-triggered shortcuts (Hazel-like) | Shortcuts app | n/a | https://macmost.com/an-introduction-to-shortcuts-automation-in-macos-tahoe.html |
| Finder icon colour revert | 26 beta 2 | Cosmetic | n/a | n/a | https://9to5mac.com/2025/06/23/macos-tahoe-26-beta-2-changes-finder-icon/ |
| Finder Sync broken on 26.1 ARM | 26.1 | Developer report | n/a | n/a | https://developer.apple.com/forums/thread/806607 |
| Archive Utility pref pane removed | 26.1 | Compress settings UI gone | n/a | n/a | https://eclecticlight.co/2025/11/04/what-has-changed-in-macos-26-1-tahoe/ |

## 12. Open questions to verify on a 26.x machine

| Question | Why it matters |
|---|---|
| Does `NSWorkspace.icon(forFile:)` return the Tahoe colour/symbol/emoji composite for a customised folder? | Decides whether Macosmetic renders folder customisation itself or reuses the system image |
| Does `setDefaultApplication(at:toOpen: UTType)` still return permErr for some types? | Needed for "Change All" |
| Does `NSWorkspace.icon(forFile:)` include File Provider decorations (cloud badge) for dataless items? | Decides how cloud badges are drawn |
| Does `trashItem` on 26.x write a Put Back record for the first item only? | Decides whether to ship an own Put Back journal |
| Does LaunchServices honour a `public.folder` handler for Dock folder clicks on 26.x? | Default-browser story |
