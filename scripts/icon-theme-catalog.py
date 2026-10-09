#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Build the icon theme catalog the gallery offers for download.

Reads res/icon-themes/sources.json, downloads every archive it names once, and writes:

  res/icon-themes/catalog.json        what the app installs from: archive URL, SHA-256 and
                                      size, where each theme sits in its archive, and which
                                      other catalog themes it inherits from
  res/icon-themes/previews/<id>/N.*   the preview strip for each theme, the same icons the
                                      gallery shows for installed themes

Each source is a GitHub repository pinned to a tag or commit (downloaded from codeload), or a
release asset given by `url`. Theme paths are relative to the archive's single top-level
directory for codeload archives, and to the archive root for release assets. An optional
`family` groups a source's themes under one heading in the gallery; it defaults to the
repository name.

Archives are cached in ~/Library/Caches/macosmetic-icon-themes (or $XDG_CACHE_HOME), so a
rerun after editing sources.json only downloads what changed.

Usage: scripts/icon-theme-catalog.py
"""

import hashlib
import json
import os
import posixpath
import re
import shutil
import sys
import tarfile
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RES = REPO_ROOT / "res" / "icon-themes"

# Keep in step with PREVIEW_ICONS in src/icon_theme_gallery.rs.
PREVIEW_ICONS = [
    ["folder"],
    ["folder-documents"],
    ["folder-download"],
    ["folder-pictures"],
    ["user-home"],
    ["text-x-generic", "text-plain"],
    ["image-x-generic"],
    ["package-x-generic", "application-x-archive"],
]
PREVIEW_SIZE = 32

# Themes every system has (or the app's fallback covers), never worth a catalog dependency.
BASE_THEMES = {"hicolor", "default"}


def cache_dir():
    if sys.platform == "darwin":
        base = Path.home() / "Library" / "Caches"
    else:
        base = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    path = base / "macosmetic-icon-themes"
    path.mkdir(parents=True, exist_ok=True)
    return path


def download(url):
    name = hashlib.sha256(url.encode()).hexdigest()[:16] + ".tar.gz"
    path = cache_dir() / name
    if not path.exists():
        print(f"  downloading {url}", file=sys.stderr)
        tmp = path.with_suffix(".part")
        request = urllib.request.Request(url, headers={"User-Agent": "macosmetic-catalog"})
        with urllib.request.urlopen(request) as response, open(tmp, "wb") as out:
            shutil.copyfileobj(response, out)
        tmp.rename(path)
    return path


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_index_theme(text):
    """Return the [Icon Theme] keys and a {directory: keys} map for its subdirectories."""
    sections, current = {}, None
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            current = sections.setdefault(line[1:-1], {})
        elif current is not None and "=" in line:
            key, value = line.split("=", 1)
            current[key.strip()] = value.strip()
    head = sections.get("Icon Theme", {})
    dirs = {}
    for key in ("Directories", "ScaledDirectories"):
        for name in filter(None, (d.strip() for d in head.get(key, "").split(","))):
            dirs[name] = sections.get(name, {})
    return head, dirs


def size_distance(keys):
    """How far a theme directory is from PREVIEW_SIZE, as the icon theme spec measures it."""
    try:
        size = int(keys.get("Size", "0"))
        scale = int(keys.get("Scale", "1"))
    except ValueError:
        return 1 << 20
    kind = keys.get("Type", "Threshold")
    penalty = 0 if scale == 1 else 1000
    if kind == "Scalable":
        low = int(keys.get("MinSize", size))
        high = int(keys.get("MaxSize", size))
        if low <= PREVIEW_SIZE <= high:
            return penalty
        return penalty + min(abs(low - PREVIEW_SIZE), abs(high - PREVIEW_SIZE))
    if kind == "Fixed":
        return penalty + abs(size - PREVIEW_SIZE)
    threshold = int(keys.get("Threshold", "2"))
    if abs(size - PREVIEW_SIZE) <= threshold:
        return penalty
    return penalty + abs(size - PREVIEW_SIZE)


class Archive:
    def __init__(self, path, strip_prefix):
        self.tar = tarfile.open(path, "r:gz")
        self.members = {}
        for member in self.tar.getmembers():
            self.members[posixpath.normpath(member.name)] = member
        self.prefix = ""
        if strip_prefix:
            tops = {name.split("/", 1)[0] for name in self.members}
            if len(tops) != 1:
                raise SystemExit(f"{path}: expected one top-level directory, found {tops}")
            self.prefix = tops.pop()

    def path(self, relative):
        return posixpath.join(self.prefix, relative) if relative else self.prefix

    def canonical(self, name, depth=0):
        """`name` with every symlink along it followed, including symlinked directories."""
        if depth > 32:
            return None
        current = ""
        for part in name.split("/"):
            current = posixpath.join(current, part) if current else part
            member = self.members.get(current)
            if member is not None and member.issym():
                target = posixpath.normpath(
                    posixpath.join(posixpath.dirname(current), member.linkname)
                )
                current = self.canonical(target, depth + 1)
                if current is None:
                    return None
        return current

    def resolve(self, name, depth=0):
        """The regular-file member `name` ends at after following links, or None."""
        canonical = self.canonical(name)
        member = self.members.get(canonical) if canonical else None
        if member is not None and member.islnk() and depth < 16:
            return self.resolve(member.linkname.rstrip("/"), depth + 1)
        return member if member is not None and member.isfile() else None

    def link_targets(self, root):
        """Where the symlinks under `root` point that lie outside it."""
        targets = set()
        for name, member in self.members.items():
            if member.issym() and name.startswith(root + "/"):
                target = posixpath.normpath(posixpath.join(posixpath.dirname(name), member.linkname))
                if target != root and not target.startswith(root + "/"):
                    targets.add(target)
        return targets

    def read(self, name):
        member = self.resolve(name)
        return self.tar.extractfile(member).read() if member else None


class Theme:
    def __init__(self, source, spec, archive, archive_index):
        self.archive = archive
        self.archive_index = archive_index
        self.root = archive.path(spec["path"])
        index_path = archive.path(spec.get("index_theme", posixpath.join(spec["path"], "index.theme")))
        text = archive.read(index_path)
        if text is None:
            raise SystemExit(f"{source['repo']}: no index.theme at {index_path}")
        self.head, self.dirs = parse_index_theme(text.decode("utf-8", "replace"))
        self.id = spec.get("id") or posixpath.basename(spec["path"])
        self.name = self.head.get("Name", self.id)
        self.inherits = [i.strip() for i in self.head.get("Inherits", "").split(",") if i.strip()]
        self.index_theme = index_path if "index_theme" in spec else None
        self.source = source

    def folder_colours(self):
        """How many folder colours the theme ships, counted the way the app counts them: one
        per `folder-<colour>-documents` icon in its places directories."""
        colours = set()
        for directory in self.dirs:
            if "places" not in directory.lower().split("/"):
                continue
            # A variant's places directory is often a symlink into its base theme.
            canonical = self.archive.canonical(posixpath.join(self.root, directory))
            if canonical is None:
                continue
            prefix = canonical + "/"
            for name in self.archive.members:
                if not name.startswith(prefix) or "/" in name[len(prefix):]:
                    continue
                stem, dot, extension = posixpath.basename(name).rpartition(".")
                if extension not in ("svg", "png"):
                    continue
                match = re.fullmatch(r"folder-(.+)-documents", stem)
                if match:
                    colours.add(match.group(1))
        return len(colours)

    def find(self, name):
        """The archive member holding icon `name` in this theme alone, best size first."""
        ordered = sorted(self.dirs.items(), key=lambda item: size_distance(item[1]))
        for directory, _keys in ordered:
            for extension in ("svg", "png"):
                member = posixpath.join(self.root, directory, f"{name}.{extension}")
                if self.archive.resolve(member):
                    return member, extension
        return None


def lookup(theme, name, themes, seen=None):
    """Find `name` in `theme`, then in the catalog themes it inherits from."""
    seen = seen or set()
    if theme.id in seen:
        return None
    seen.add(theme.id)
    found = theme.find(name)
    if found:
        return theme, found
    for parent in theme.inherits:
        if parent in themes:
            result = lookup(themes[parent], name, themes, seen)
            if result:
                return result
    return None


def main():
    sources = json.loads((RES / "sources.json").read_text())
    archives, themes = [], {}
    for source in sources:
        url = source.get("url") or f"https://codeload.github.com/{source['repo']}/tar.gz/{source['ref']}"
        print(f"{source['repo']}", file=sys.stderr)
        path = download(url)
        archive = Archive(path, strip_prefix="url" not in source)
        archives.append({"url": url, "sha256": sha256(path), "size": path.stat().st_size})
        for spec in source["themes"]:
            theme = Theme(source, spec, archive, len(archives) - 1)
            if theme.id in themes:
                raise SystemExit(f"duplicate theme id {theme.id}")
            themes[theme.id] = theme

    previews_dir = RES / "previews"
    if previews_dir.exists():
        shutil.rmtree(previews_dir)
    entries = []
    for theme in themes.values():
        previews = []
        out_dir = previews_dir / theme.id
        out_dir.mkdir(parents=True)
        for index, names in enumerate(PREVIEW_ICONS):
            for name in names:
                result = lookup(theme, name, themes)
                if result:
                    owner, (member, extension) = result
                    file_name = f"{index}.{extension}"
                    (out_dir / file_name).write_bytes(owner.archive.read(member))
                    previews.append(file_name)
                    break
        requires = [
            parent
            for parent in theme.inherits
            if parent in themes and parent != theme.id and parent not in BASE_THEMES
        ]
        # Variants such as Papirus-Dark are mostly symlinks into a sibling theme, which then
        # has to be installed beside them whatever `Inherits=` says.
        for target in theme.archive.link_targets(theme.root):
            owner = next(
                (
                    other
                    for other in themes.values()
                    if other.archive is theme.archive
                    and other is not theme
                    and (target == other.root or target.startswith(other.root + "/"))
                ),
                None,
            )
            if owner is None:
                print(f"  {theme.id}: symlink to {target} leaves the catalog", file=sys.stderr)
            elif owner.id not in requires:
                requires.append(owner.id)
        entry = {
            "id": theme.id,
            "name": theme.name,
            "family": theme.source.get("family") or theme.source["repo"].split("/")[1],
            "license": theme.source["license"],
            "homepage": f"https://github.com/{theme.source['repo']}",
            "archive": theme.archive_index,
            "path": theme.root,
            "requires": requires,
            "folder_colours": theme.folder_colours(),
            "previews": previews,
        }
        if theme.index_theme:
            entry["index_theme"] = theme.index_theme
        entries.append(entry)
        print(f"  {theme.id}: {len(previews)} previews, requires {requires}", file=sys.stderr)

    catalog = {"version": 1, "archives": archives, "themes": entries}
    (RES / "catalog.json").write_text(json.dumps(catalog, indent=2) + "\n")
    print(f"wrote {len(entries)} themes from {len(archives)} archives", file=sys.stderr)


if __name__ == "__main__":
    main()
