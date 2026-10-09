# Quality-of-life brainstorm for Macosmetic

58 ideas, sorted by value divided by effort. The top of the table is cheap and fixes daily Finder pain. The bottom is expensive or niche.

Researched 2026-10-09. Repo state: master at `018a5ae`.

## How to read the table

| Column | Meaning |
| --- | --- |
| Effort | S is under a week. M is one to three weeks. L is a month or more, or needs new AppKit FFI. |
| Upstream | Whether cosmic-files, and so this fork, already has it. "Yes" and "Partial" come from reading `src/`. "Check" means I could not confirm from a grep. |
| Signal | Where the demand shows up. `#N (R)` is a pop-os/cosmic-files issue with R reactions. |

## The pattern behind the complaints

Finder complaints cluster into five causes. Most ideas below hit one of them.

1. **Hidden power.** Finder has the feature but buries it: cut is Cmd+Opt+V, copy path needs Option on a context menu, folders-first lives in Advanced settings.
2. **View state that won't stick.** `.DS_Store` stores per-folder views, so column widths and sort order drift folder to folder. One sort change can also leak to every folder.
3. **Blocking I/O.** A stalled SMB share freezes the whole window. HN and r/MacOS both report force-quitting Finder for this.
4. **Search that guesses wrong.** It defaults to "This Mac" and matches file contents, so typing a filename returns noise.
5. **One folder at a time.** No dual pane, so people tile two windows. This is the reason MacRumors users give for buying ForkLift and Path Finder.

## Ideas

| # | Idea | Who benefits | Why Finder fails | Effort | Upstream | Signal |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Cmd+X cuts, Cmd+V moves, and cut items render dimmed until pasted | Windows and Linux switchers | Cut exists only as Cmd+C then Cmd+Opt+V. The source shows no cut state. | S | Yes. `Action::Cut` exists. Confirm the Cmd mapping and the dimmed state. | Reddit "why can't I cut/paste" |
| 2 | Copy path variants: POSIX, `~/` form, shell-quoted, `file://` URL, name only, parent folder | Developers, support staff | One variant only, behind Option plus right-click. | S | Partial. `Action::CopyPath` has one form. | HN, r/MacOS |
| 3 | APFS `clonefile` for same-volume copy and duplicate | Everyone with big files | Finder clones instantly on APFS. A byte copy here will look slow next to it. | S | No. Upstream #980 is open for reflinks. | #980 (4) |
| 4 | Sort by kind and by extension | Everyone | Finder has Kind but not extension. | S | No. `HeadingOptions` is Name, Modified, Size, TrashedOn. | #1262 (20), #1817 (7) |
| 5 | Status bar shows selected count, selected size and free space | Everyone | Status bar shows item count and free space, never the selection's size. | S | Check | #1766 (7) |
| 6 | Quick filter: typing narrows the current folder in place, Esc clears | Keyboard users | Typing jumps to a prefix. Cmd+F launches a slow, global search. | S | Partial. `TypeToSearch` has Recursive, EnterPath and SelectByPrefix. No in-place filter mode. | DOpus "filter as you type" |
| 7 | Open terminal here, with a chosen app: Terminal, Ghostty, iTerm2, WezTerm | Developers | Needs a Services toggle in System Settings. No shortcut by default. | S | Partial. `Action::OpenTerminal` exists. Check the macOS app chooser. | #695, #1458 (7) |
| 8 | Open in editor: VS Code, Zed, Cursor, Sublime, with a shortcut | Developers | Needs drag-to-Dock or an Automator action. | S | Partial. Custom context actions could do it. Not preset. | Common in ForkLift, QSpace |
| 9 | Cmd+1 to Cmd+9 jump to sidebar favorites | Keyboard users | Sidebar has no keyboard access. Cmd+1..4 switch views instead. | S | Partial. Favorites exist. No numbered jump. | DOpus, Marta |
| 10 | New file from templates folder, with `.md`, `.txt` and `.sh` marked executable | Developers, writers | Finder cannot create an empty file at all. | S | Partial. `Action::NewFile` exists. Templates are a check. | r/MacOS perennial |
| 11 | Symlink badge on icons, plus a "Make symlink" action | Developers | Finder makes aliases, which CLI tools ignore. No symlink badge in list view. | S | No | #466 (17), #275 (15), #1809 (5) |
| 12 | Checksum panel in properties: SHA-256, MD5, compare to clipboard | Anyone verifying downloads | Needs Terminal. | S | No | Files (Windows), QSpace Stash |
| 13 | Select by pattern, invert selection, select same extension | Power users | No pattern select at all. | S | No | Total Commander, DOpus |
| 14 | Restore tabs, windows and scroll position on launch | Everyone | Restore depends on a system setting and often fails. | M | No. No session code in `src/`. | #76 (13) |
| 15 | Dual pane with F5 copy and F6 move to the other pane | Anyone moving files | No split view. Users tile two windows. | M | No | #260 (23), MacRumors "the one feature I want" |
| 16 | Per-folder view memory: sort, view mode, zoom, column widths | Everyone | Stored in `.DS_Store`, which drifts and pollutes shares. Sorting one folder can change all. | M | No. Only `sort_names` is per path. | #427 (13), Reddit top complaint |
| 17 | Folders-first toggle in the View menu, shared by windows and dialogs | Switchers | Buried in Settings, Advanced. A separate setting covers the desktop. | S | Yes. `ToggleFoldersFirst` exists. Confirm it is in the macOS menu bar. | r/MacOS |
| 18 | Show-hidden toggle that persists, plus "always hide `.DS_Store`, `._*`, `.localized`" | Developers | Cmd+Shift+. shows everything including Mac junk. | S | Partial. `show_hidden` persists. Junk rules do not exist. | #299 (9) |
| 19 | Copy and move progress with speed, ETA and the current file | Anyone copying large sets | ETA jumps wildly. No speed shown. | S | Check | #1283 (14) |
| 20 | Conflict dialog with side-by-side size, date and thumbnail, plus "rename new" | Anyone merging folders | Choices are Stop, Skip, Keep Both, Replace, with no detail. | M | Partial. A replace dialog exists. Compare detail is a check. | #887 (3) |
| 21 | Multi-level undo for copy, move, rename and trash, with a history panel | Everyone | Undo is one level and dies on relaunch. FloqFiles sells persistent undo as its headline. | M | Partial. `EditHistory` page and `Undo(usize)` exist. Check depth and persistence. | #1326 (7) |
| 22 | Batch rename with live preview, regex, counters, case and EXIF or ID3 date tokens | Photographers, archivists | Three fixed modes, no regex, no metadata. ForkLift 4.7.1 added metadata renaming in June 2026. | M | Check | #689 (16), #120 (13) |
| 23 | Folder sizes in list view, calculated in the background and cached | Anyone cleaning disk | "Calculate all sizes" is slow and works in list view only. | M | Partial. `DirSize` exists in properties. | DOpus, Path Finder |
| 24 | Operation queue: serialise copies to the same disk, pause, resume, reorder | Anyone copying to HDDs or NAS | Parallel copies thrash one disk. No queue. | M | Partial. `ControllerState::Paused` exists. No queue. | DOpus "queue multiple copies" |
| 25 | Never freeze on a stalled network mount. Load folders off the UI thread with a timeout. | NAS users | A hung SMB share beachballs Finder. | M | No. Upstream #2084 is open. | #2084 (5), HN, Reddit |
| 26 | Never write `.DS_Store`. Offer to skip `._*` and `.DS_Store` when copying to FAT, exFAT or SMB. | Anyone sharing USB drives with Windows or TVs | Finder litters every volume it touches. | S | Partial. The app writes none. The copy filter is new. | Apple support note on SMB `.DS_Store` |
| 27 | Move to and Copy to, with recent destinations | Everyone | Finder has no Move To menu. | S | Yes. `MoveTo` and `CopyTo` exist. Recent destinations are a check. | DOpus, Windows Explorer |
| 28 | Recent locations dropdown on long-press of Back | Everyone | Back menu exists but Go, Recent Folders is short and unordered. | S | Check | Browser habit |
| 29 | Reuse an existing tab when opening a folder that is already open | Tab users | "Refuses to reuse tabs", so tabs pile up. | S | Check | r/MacOS |
| 30 | Rename selects the stem only. Double-click a word stops at periods. | Everyone | Finder gets stem selection right. Parity needed. | S | Check | #1451 (5) |
| 31 | Configurable Enter key: rename or open | Switchers | Enter always renames. Windows users expect open. | S | Check | HN thread on Enter-to-rename |
| 32 | Sidebar drag-to-reorder and rename favorites | Everyone | Finder supports it. Parity needed. | S | Check. `reorder` appears in `app.rs`. | #309 (17), #839 (5) |
| 33 | Saved servers and persistent network bookmarks in the sidebar | NAS users | Cmd+K favorites are hidden. Shares drop from the sidebar after reboot. | M | Partial. `NetworkDrive` page exists. | #687 (11), #631 (7), #1605 (12) |
| 34 | Miller column view with auto-fit column widths | Long-time Mac users | Finder has it, but widths need Option-drag every time. | L | No | #77 (60), the most-reacted upstream issue |
| 35 | Expandable tree rows in list view | Long-time Mac users | Finder has it. Parity needed. | M | Check | #19 (26) |
| 36 | Command palette, Cmd+Shift+P, listing every action with its shortcut | Keyboard users | Finder has no palette. The Help menu search is the nearest. | M | No | Files (Windows) |
| 37 | Fuzzy go-to by frecency, like `zoxide` | Keyboard users | Go to Folder needs the exact path. | M | No | Marta, nnn |
| 38 | Media columns: dimensions, duration, codec, EXIF date, page count | Photographers, video editors | Some exist but load slowly and vary by view. | M | No | #108 (5) |
| 39 | Git status tints and branch name in the path bar | Developers | No VCS awareness. | M | No | FloqFiles, Files (Windows) |
| 40 | Read and write macOS Finder tags as the native xattr | Anyone moving from Finder | Finder has tags. Skipping them breaks the user's existing organisation. | M | No | #42 (5) |
| 41 | Spotlight-backed content search using `mdfind`, with kind and date filters | Everyone | Finder's search is its real strength. This is parity. | M | Check | r/MacOS "Finder is a better finder" |
| 42 | Keybinding editor in Settings | Power users | App shortcuts live in System Settings, by menu title. | M | Partial. `keybinds` is in config. No UI. | Files (Windows) |
| 43 | Preview pane with syntax-highlighted code, Markdown and PDF | Developers | Quick Look shows code as plain text. | M | Partial. Preview page and Quick Look bridge exist. | #74 (6) |
| 44 | Trash auto-empty after N days and an optional confirm-on-trash | Everyone | Finder has 30 days only. | S | No | #1333, #1813 |
| 45 | "New folder with selection" | Everyone | Finder has it. Parity needed. | S | Check | Reddit tip thread |
| 46 | Spring-loaded folders and tabs while dragging | Mouse users | Finder has it. Parity needed. | M | Check | Finder parity |
| 47 | Copy a folder listing as text, CSV or Markdown | Admins, writers | Needs `ls` in Terminal. | S | No | DOpus "export folder listings" |
| 48 | Flat view: every file in a subtree as one list | Photographers, cleanup | Only approximated with a search. | M | No | DOpus |
| 49 | Shelf for collecting files across folders, then acting on them together | Anyone gathering files | Users install Dropover or Yoink. | M | No | Path Finder Drop Stack, QSpace Stash Shelf |
| 50 | Browse archives as folders and pull single files out | Developers, admins | Finder can only extract all. | L | No. Extract only. | QSpace, Commander One |
| 51 | Folder compare and sync between panes | Backup users | Needs rsync or a separate app. | L | No | ForkLift, Path Finder, DOpus |
| 52 | Duplicate file finder by size then hash | Anyone cleaning disk | Needs a separate app. | M | No | DOpus |
| 53 | Image quick ops: rotate, resize, convert | Photographers | Quick Actions cover rotate and convert, not batch resize to a size. | M | No | DOpus |
| 54 | Embedded terminal panel that follows the current folder | Developers | No terminal panel. | L | No | #237 (9), Dolphin F4, Commander One Pro |
| 55 | Open or edit as administrator via an authorised helper | Admins | Finder prompts per item and refuses some operations. | L | No | #770 (14), #1044 (12) |
| 56 | One window across Spaces, plus "Merge all windows" | Multi-Space users | Windows scatter across Spaces. | M | Check | r/MacOS |
| 57 | Multi-pane layouts beyond two, up to four | Asset triage | No panes at all. | L | No. Needs idea 15 first. | QSpace up to 12 panes |
| 58 | Per-folder colour or rule labels, such as "files newer than 1 day in bold" | Project-heavy users | Tags are manual only. | M | No | DOpus folder formats |

## What I would do first

Ideas 1 to 13 are each under a week, and together they cover the most-repeated Finder complaints. Ideas 3, 4 and 25 deserve priority over their rank, for different reasons:

- **Idea 3, `clonefile`.** Finder copies on APFS are instant. A byte copy will make this app look slow in the first five minutes of use.
- **Idea 4, sort by type.** It is the most-reacted issue under upstream's `enhancement` label and costs one enum variant plus a comparator.
- **Idea 25, network stalls.** It is the most emotional Finder complaint. It also blocks idea 33, saved servers, from being useful.

Dual pane (15) and per-folder views (16) are the two features people name when they say why they left Finder. Both are M, so they belong in the first batch after the S items.

## Sources

- Upstream issues: `gh api search/issues` on pop-os/cosmic-files, open issues sorted by reactions, read only.
- r/MacOS, "I've been a mac user for almost 20 years and Finder still frustrates me daily" (194g7m6).
- r/macbookpro, "Why is Finder so shit?" (1d71y3q).
- Hacker News items 28084159 and 14032777.
- MacRumors, "Best Finder alternatives / File managers", page 20.
- MacMost, "How To Fix These 10 Mac Finder Annoyances".
- Apple Support 102064, SMB browsing and `.DS_Store`.
- floqfiles.dullware.com comparisons, July 2026. The author sells a competitor.
- BinaryNights blog, ForkLift 4.7.1 and 4.7.2 release notes.
- qspace.awehunt.com, files.community, PCWorld Directory Opus review, windowsforum.com Directory Opus review.
