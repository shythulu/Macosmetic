# Roadmap: Macosmetic as a Finder replacement

Decided 2026-10-09 by a review panel of three agents: a software architect, a macOS
platform expert and an adversarial reviewer. The adversary checked every claim against the
code before ruling. Full reasoning is in [`research/08-panel-final.md`](research/08-panel-final.md).

## How the decision was made

```
01 current features ─┐
02 Finder + Apple APIs ├─► 06 architect ─┐
05 quality-of-life ideas┘   07 macOS ─────┴─► 08 adversary: verify claims, final tiers
03 folder customization design ─┐
04 theme installer design ──────┴─► built directly (already scoped)
```

| Doc | What it holds |
|---|---|
| [01](research/01-current-features.md) | What the app does today |
| [02](research/02-finder-features.md) | Finder features, the Apple API behind each, and whether a third-party app can match it |
| [03](research/03-design-folder-customization.md) | Customize folder: design review and redesign spec |
| [04](research/04-design-theme-install.md) | Icon theme installation: design and finishing plan |
| [05](research/05-qol-brainstorm.md) | 58 quality-of-life ideas from other file managers and Finder complaints |
| [06](research/06-panel-architect.md) | Architecture verdicts: fit, merge cost, sequencing |
| [07](research/07-panel-macos.md) | Platform verdicts: APIs, entitlements, Finder interop |
| [08](research/08-panel-final.md) | Claim checks, disagreements resolved, final tiers |

## Tier 0: broken on day one

These are things a Finder user would hit in the first hour.

| ID | Fix | Status |
|---|---|---|
| T0-1, T0-2 | Trash through `NSFileManager`, Undo and Put Back from a journal | PR #26 |
| T0-3 | Copy with `copyfile`: APFS clones, keep all metadata | PR #27 |
| T0-4, T0-5, T0-6 | Hidden flag, network mounts as remote, never read iCloud placeholders | PR #28 |
| T0-7 | Open With and Open in Terminal through LaunchServices | PR #29 |
| T0-8 | Packages (`.app`, `.rtfd`) behave as files; Show Package Contents | In progress |
| T0-9 | Volumes in the sidebar, with eject | In progress |
| T0-10 | Drag and drop, which is stubbed out on macOS | In progress |

## Tier 1: next

| ID | Feature | Status |
|---|---|---|
| T1-1 | Status bar: item count, selection size, free space | In progress |
| T1-2 | Cmd+Z undo for rename, move, new folder, trash | Planned |
| T1-3 | Duplicate (Cmd+D) and New Folder with Selection | Planned |
| T1-4 | Cmd+I Get Info: Kind, Where, Created, Date Added | Planned |
| T1-5 | Sort by Kind and Date Added | In progress |
| T1-6 | Finder tags, read only | Planned |
| T1-7 | Finder tags, write | Planned, after T1-6 |
| T1-8 | Drag out to other apps | Planned, after T0-10 |
| T1-9 | Share and AirDrop | Planned |
| T1-10 | Recents from Spotlight | In progress |
| T1-11 | Default folder opener (`odoc` handler) with "Restore Finder" | Planned |
| T1-12 | Copy path variants | In progress |

## Customization work

| Work | Status |
|---|---|
| Folder colour submenu and redesigned Customize folder drawer | PR #25 |
| Icon theme installer: hardened downloads, cancel, update, install from file, no restart | In progress on `feat/icon-theme-catalog` |

## Decided against, for now

| Idea | Why not |
|---|---|
| Writing folder looks into Finder tags or the Tahoe folder xattr | Users give colour tags their own meanings, tag writes sync to every device, and the Tahoe format is undocumented. Later: read first, then an opt-in colour mirror |
| A `src/macos/` reorganisation and a shared metadata worker | Churn with no consumer yet. Small helpers ship with the first feature that needs them |
| Desktop, Stacks, Open/Save panels, Finder Sync hosting | Only Finder can provide these |
| Liquid Glass beyond a transparent titlebar | Large effort through iced, little function |
| App Store distribution | A sandboxed file manager cannot browse the disk |

## Shared conventions

- macOS code lives in `*_macos.rs` modules and is `cfg`-gated, so Linux builds are unchanged.
- Edits to upstream-shared files (`app.rs`, `tab.rs`, `menu.rs`) stay small and local.
- New features add one nested `Message::Feature(feature::Message)` arm in `app.rs`.
