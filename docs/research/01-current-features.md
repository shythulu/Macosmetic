# Current features (inventory, master at 018a5ae)

Source: Haiku inventory agent, 2026-10-09. Line numbers approximate.

## Present
- Views: grid, list. Sort by name, modified, size, trashed-on. Per-folder sort memory. Folders first. Hidden toggle. Zoom (scroll, pinch). Gallery (Space).
- Navigation: tabs, windows, back/forward, parent, Go to Folder (Cmd+Shift+G, text location edit), ancestor-segment menu, Recents, Network/Connect to Server (gvfs-based; macOS not verified), eject, spring-loaded hover on drag.
- Selection: select all/first/last, keyboard, rubber band, type-to-search (recursive / enter path / select by prefix).
- File operations: copy, move, trash, restore, permanently delete, empty trash, rename, new file/folder, compress (zip, tgz), extract (with password), set permissions, set executable. Operation queue with pause/resume/cancel. Undo. Replace/skip conflict dialog. Paste image/video/text as files.
- Context menu: open, open with, show details, reveal in Finder, rename, cut, copy, copy path (Shift), move to, copy to, trash, open in new tab/window, customize folder, add to sidebar, open terminal, custom user context actions.
- Preview pane (details): owner, group, item count, dir size, SHA256, permissions editing, Quick Look thumbnails.
- Settings: theme, icon theme (gallery), type-to-search mode, single-click, show recents, desktop options.
- Customization: per-folder looks (colour, theme icon, image), icon themes, keybind overrides, custom context actions.
- macOS: Cmd key binds, Cmd+H/Cmd+Q, Quick Look thumbnails, trackpad gestures, bundle launch fixes.

## Absent or partial
| Feature | Status |
|---|---|
| Tags | Absent |
| Column view | Absent |
| Clickable path bar | Partial (text entry + ancestor menu) |
| Status bar | Absent |
| Get Info window | Partial (preview pane) |
| Smart folders | Absent |
| Aliases | Absent |
| Services / Quick Actions | Partial (custom actions only) |
| Share menu, AirDrop | Absent |
| Spotlight search | Absent (directory walk) |
| Show package contents | Absent |
| Finder comments | Absent |
| Locked flag, hide extension | Absent |
