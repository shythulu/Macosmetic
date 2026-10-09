// SPDX-License-Identifier: GPL-3.0-only

//! Installed freedesktop icon themes, found the way COSMIC Settings finds them.
//!
//! A theme is a directory under one of the XDG `icons` directories with an
//! `index.theme` file. Its id is the directory name, which is what the
//! `com.system76.CosmicTk` `icon_theme` key stores; `Name=` is only for display.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Directories searched for icon themes, highest priority first.
///
/// `$XDG_DATA_HOME/icons`, then `$XDG_DATA_DIRS/icons`, then `~/.icons`: the same set
/// `cosmic-freedesktop-icons` searches, so every theme listed here can be looked up.
pub fn icon_base_dirs() -> Vec<PathBuf> {
    let home = crate::home_dir();
    let mut dirs = Vec::new();
    dirs.push(data_home(&home).join("icons"));
    let data_dirs = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    dirs.extend(
        data_dirs
            .split(':')
            .filter(|dir| Path::new(dir).is_absolute())
            .map(|dir| Path::new(dir).join("icons")),
    );
    dirs.push(home.join(".icons"));
    dirs.dedup();
    dirs
}

/// `$XDG_DATA_HOME`, or its spec default `~/.local/share`.
pub fn data_home(home: &Path) -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".local").join("share"))
}

/// The `[Icon Theme]` keys this app reads.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexTheme {
    pub name: Option<String>,
    pub hidden: bool,
    pub inherits: Vec<String>,
    pub directories: Vec<String>,
}

/// Parses the `[Icon Theme]` group of an `index.theme` file.
pub fn parse_index_theme(text: &str) -> IndexTheme {
    let mut index = IndexTheme::default();
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Icon Theme]";
            continue;
        }
        if !in_group || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let list = || {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(String::from)
        };
        match key.trim() {
            "Name" if index.name.is_none() && !value.is_empty() => {
                index.name = Some(value.to_string());
            }
            "Hidden" => index.hidden = value.eq_ignore_ascii_case("true"),
            "Inherits" => index.inherits = list().collect(),
            "Directories" | "ScaledDirectories" => index.directories.extend(list()),
            _ => (),
        }
    }
    index
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconThemeInfo {
    /// Directory name, the value stored in the toolkit config.
    pub id: String,
    /// Display name from `Name=`.
    pub name: String,
    pub inherits: Vec<String>,
    /// Every installed copy of the theme, highest priority first.
    pub roots: Vec<PathBuf>,
    pub directories: Vec<String>,
}

/// Themes that are offered as a choice: visible, named, and holding icons.
///
/// Cursor-only themes have no `Directories=` and are left out, as are `hicolor`
/// (the fallback every theme ends in) and `default` (a cursor alias).
pub fn installed_themes() -> Vec<IconThemeInfo> {
    installed_themes_in(&icon_base_dirs())
}

pub fn installed_themes_in(base_dirs: &[PathBuf]) -> Vec<IconThemeInfo> {
    let mut themes: BTreeMap<String, IconThemeInfo> = BTreeMap::new();
    for base in base_dirs {
        let Ok(entries) = fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let root = entry.path();
            let Some(id) = root.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if matches!(id, "hicolor" | "default") {
                continue;
            }
            if let Some(theme) = themes.get_mut(id) {
                theme.roots.push(root);
                continue;
            }
            let Ok(text) = fs::read_to_string(root.join("index.theme")) else {
                continue;
            };
            let index = parse_index_theme(&text);
            let Some(name) = index.name else {
                continue;
            };
            if index.hidden || index.directories.is_empty() {
                continue;
            }
            themes.insert(
                id.to_string(),
                IconThemeInfo {
                    id: id.to_string(),
                    name,
                    inherits: index.inherits,
                    roots: vec![root],
                    directories: index.directories,
                },
            );
        }
    }
    let mut themes: Vec<_> = themes.into_values().collect();
    themes.sort_by_cached_key(|theme| theme.name.to_lowercase());
    themes
}

/// Folder icon names a theme ships in its `places` directories.
///
/// Open, drag and symbolic variants are left out: those are states of a folder,
/// not looks for one. `folder` comes first, then the rest alphabetically.
pub fn folder_icon_names(theme: &IconThemeInfo) -> Vec<String> {
    let mut names = without_colour_variants(all_folder_icon_names(theme));
    if let Some(index) = names.iter().position(|name| name == "folder") {
        let folder = names.remove(index);
        names.insert(0, folder);
    }
    names
}

/// Every folder-like icon name in the theme's `places` directories, sorted, colour
/// variants included.
fn all_folder_icon_names(theme: &IconThemeInfo) -> Vec<String> {
    let mut names = Vec::new();
    for root in &theme.roots {
        for directory in &theme.directories {
            let is_places = Path::new(directory)
                .components()
                .any(|component| component.as_os_str().eq_ignore_ascii_case("places"));
            if !is_places {
                continue;
            }
            let Ok(entries) = fs::read_dir(root.join(directory)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_icon = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "svg" | "png"));
                if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
                    && is_icon
                    && is_folder_look_name(stem)
                {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// How many folder colours `theme` ships: one per `folder-<colour>-documents` icon, the
/// way papirus-folders finds a theme's colours.
pub fn folder_colours(theme: &IconThemeInfo) -> usize {
    let names = all_folder_icon_names(theme);
    colour_names(&names).len()
}

fn colour_names(names: &[String]) -> Vec<&str> {
    let mut colours: Vec<&str> = names
        .iter()
        .filter_map(|name| name.strip_prefix("folder-")?.strip_suffix("-documents"))
        .collect();
    colours.sort_unstable();
    colours.dedup();
    colours
}

/// Drops coloured copies of other icons, such as Papirus' `folder-red-documents`.
///
/// A colour is any `X` with a `folder-X-documents`, which is how papirus-folders finds
/// a theme's colours. The plain `folder-X` stays, so each colour is still offered once;
/// colours themselves are better picked from the colour row, which works in any theme.
fn without_colour_variants(names: Vec<String>) -> Vec<String> {
    let colours = colour_names(&names);
    let is_variant = |name: &str| {
        colours.iter().any(|colour| {
            ["folder-", "user-"].iter().any(|prefix| {
                name.strip_prefix(prefix)
                    .and_then(|rest| rest.strip_prefix(colour))
                    .is_some_and(|rest| rest.starts_with('-'))
            })
        })
    };
    let keep: Vec<bool> = names.iter().map(|name| !is_variant(name)).collect();
    names
        .into_iter()
        .zip(keep)
        .filter_map(|(name, keep)| keep.then_some(name))
        .collect()
}

/// Known folder icon names with a friendly label and the extra words a search should
/// match. Labels here are not translated: they derive from the pack's own names.
///
/// `folder` is special-cased in [`icon_label`], since "Default folder" is translated.
const KNOWN_ICONS: &[(&str, &str, &[&str])] = &[
    ("folder", "", &["plain", "default"]),
    ("folder-documents", "Documents", &["docs"]),
    ("folder-download", "Downloads", &["downloads"]),
    ("folder-downloads", "Downloads", &[]),
    ("folder-music", "Music", &["audio"]),
    ("folder-pictures", "Pictures", &["photos", "images"]),
    ("folder-videos", "Videos", &["movies"]),
    ("folder-publicshare", "Public", &["shared"]),
    ("folder-public", "Public", &["shared"]),
    ("folder-templates", "Templates", &[]),
    ("folder-desktop", "Desktop", &[]),
    ("user-desktop", "Desktop", &[]),
    ("user-home", "Home", &[]),
    ("folder-git", "Git", &["repo", "source"]),
    ("folder-github", "GitHub", &["repo", "source"]),
    (
        "folder-development",
        "Code",
        &["dev", "programming", "source"],
    ),
    ("folder-code", "Code", &["dev", "programming", "source"]),
    ("folder-projects", "Projects", &["work"]),
    ("folder-games", "Games", &[]),
    ("folder-cloud", "Cloud", &["sync"]),
    ("folder-dropbox", "Dropbox", &["sync"]),
    ("folder-google-drive", "Google Drive", &["sync"]),
    ("folder-locked", "Locked", &["private", "secure"]),
    ("folder-favorites", "Favourites", &["starred"]),
    ("folder-important", "Important", &["urgent"]),
    ("folder-recent", "Recent", &[]),
    ("folder-script", "Scripts", &["shell"]),
];

fn known_icon(
    name: &str,
) -> Option<&'static (&'static str, &'static str, &'static [&'static str])> {
    KNOWN_ICONS.iter().find(|(known, ..)| *known == name)
}

/// A readable label for a folder icon name: `folder-git` is "Git", `folder-publicshare`
/// is "Public". Unknown names lose their `folder-` or `user-` prefix, get spaces for
/// dashes and underscores, and a capital first letter.
pub fn icon_label(name: &str) -> String {
    if name == "folder" {
        return crate::fl!("icon-default-folder");
    }
    if let Some((_, label, _)) = known_icon(name) {
        return (*label).to_string();
    }
    let rest = ["folder-", "folder_", "user-"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(name);
    let words = rest.replace(['-', '_'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => name.to_string(),
    }
}

/// Words besides the label and the raw name that a search for this icon should match.
pub fn icon_search_terms(name: &str) -> &'static [&'static str] {
    known_icon(name).map_or(&[], |(_, _, terms)| terms)
}

/// Whether `name` matches a lower-case search, on its label, raw name or synonyms.
pub fn icon_matches(name: &str, label: &str, search: &str) -> bool {
    search.is_empty()
        || name.to_lowercase().contains(search)
        || label.to_lowercase().contains(search)
        || icon_search_terms(name)
            .iter()
            .any(|term| term.contains(search))
}

/// Where an icon sits in the drawer's grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconGroup {
    /// The plain folder, the XDG user directories and `user-*`.
    Places,
    /// Everything else: what a folder is for.
    Purpose,
}

pub fn icon_group(name: &str) -> IconGroup {
    const XDG_KINDS: &[&str] = &[
        "documents",
        "download",
        "downloads",
        "music",
        "pictures",
        "videos",
        "publicshare",
        "public",
        "templates",
        "desktop",
    ];
    let is_place = name == "folder"
        || name.starts_with("user-")
        || name
            .strip_prefix("folder-")
            .is_some_and(|kind| XDG_KINDS.contains(&kind));
    if is_place {
        IconGroup::Places
    } else {
        IconGroup::Purpose
    }
}

fn is_folder_look_name(name: &str) -> bool {
    let folder_like = name == "folder"
        || name.starts_with("folder-")
        || name.starts_with("folder_")
        || name == "user-home"
        || name == "user-desktop";
    let state = ["-symbolic", "-open", "-drag-accept", "-visiting"]
        .iter()
        .any(|suffix| name.ends_with(suffix));
    folder_like && !state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "cosmic-files-icon-themes-{name}-{}",
            fastrand::u64(..)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_the_icon_theme_group_only() {
        let index = parse_index_theme(
            "[Icon Theme]\nName=Papirus\nName[de]=Papirus DE\nInherits=breeze, hicolor\n\
             Directories=16x16/places,48x48/places\nScaledDirectories=48x48@2x/places\n\
             Hidden=false\n\n[16x16/places]\nName=ignored\nSize=16\n",
        );
        assert_eq!(index.name.as_deref(), Some("Papirus"));
        assert!(!index.hidden);
        assert_eq!(index.inherits, ["breeze", "hicolor"]);
        assert_eq!(
            index.directories,
            ["16x16/places", "48x48/places", "48x48@2x/places"]
        );
    }

    #[test]
    fn lists_icon_themes_and_skips_cursor_hidden_and_fallback_themes() {
        let base = temp_dir("list");
        write(
            &base.join("Papirus/index.theme"),
            "[Icon Theme]\nName=Papirus\nDirectories=48x48/places\n",
        );
        write(
            &base.join("Cursors/index.theme"),
            "[Icon Theme]\nName=Some Cursors\nInherits=Adwaita\n",
        );
        write(
            &base.join("Secret/index.theme"),
            "[Icon Theme]\nName=Secret\nHidden=true\nDirectories=places\n",
        );
        write(
            &base.join("hicolor/index.theme"),
            "[Icon Theme]\nName=Hicolor\nDirectories=48x48/apps\n",
        );
        write(
            &base.join("Nameless/index.theme"),
            "[Icon Theme]\nDirectories=a\n",
        );

        let themes = installed_themes_in(std::slice::from_ref(&base));
        let ids: Vec<_> = themes.iter().map(|theme| theme.id.as_str()).collect();
        assert_eq!(ids, ["Papirus"]);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_first_base_dir_wins_and_later_copies_are_extra_roots() {
        let high = temp_dir("high");
        let low = temp_dir("low");
        write(
            &high.join("Theme/index.theme"),
            "[Icon Theme]\nName=High\nDirectories=places\n",
        );
        write(
            &low.join("Theme/index.theme"),
            "[Icon Theme]\nName=Low\nDirectories=places\n",
        );
        let themes = installed_themes_in(&[high.clone(), low.clone()]);
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "High");
        assert_eq!(themes[0].roots, [high.join("Theme"), low.join("Theme")]);
        fs::remove_dir_all(high).unwrap();
        fs::remove_dir_all(low).unwrap();
    }

    #[test]
    fn colour_variants_are_dropped_but_plain_colours_stay() {
        let names = [
            "folder",
            "folder-documents",
            "folder-red",
            "folder-red-documents",
            "folder-red-git",
            "folder-cat-mocha-mauve",
            "folder-cat-mocha-mauve-documents",
            "folder-cat-mocha-mauve-music",
            "folder-git",
            "user-red-home",
            "user-home",
            "folder-redhat",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            without_colour_variants(names),
            [
                "folder",
                "folder-documents",
                "folder-red",
                "folder-cat-mocha-mauve",
                "folder-git",
                "user-home",
                "folder-redhat"
            ]
        );
    }

    #[test]
    fn icon_labels_are_friendly() {
        assert_eq!(icon_label("folder-git"), "Git");
        assert_eq!(icon_label("folder-publicshare"), "Public");
        assert_eq!(icon_label("folder-google-drive"), "Google Drive");
        assert_eq!(icon_label("user-home"), "Home");
        // Unknown names: strip the prefix, space the words, capitalise.
        assert_eq!(icon_label("folder-visual-studio"), "Visual studio");
        assert_eq!(icon_label("folder_snap"), "Snap");
        assert_eq!(icon_label("user-trash-full"), "Trash full");
        assert_eq!(icon_label("something"), "Something");
        assert!(!icon_label("folder").is_empty());
    }

    #[test]
    fn icon_search_matches_label_name_and_synonyms() {
        assert_eq!(
            icon_search_terms("folder-development"),
            ["dev", "programming", "source"]
        );
        assert!(icon_search_terms("folder-whatever").is_empty());
        let label = icon_label("folder-development");
        assert!(icon_matches("folder-development", &label, "code"));
        assert!(icon_matches("folder-development", &label, "develop"));
        assert!(icon_matches("folder-development", &label, "programming"));
        assert!(icon_matches("folder-development", &label, ""));
        assert!(!icon_matches("folder-development", &label, "music"));
        assert!(icon_matches("folder-publicshare", "Public", "shared"));
    }

    #[test]
    fn icon_groups_split_places_from_purpose() {
        for name in [
            "folder",
            "folder-documents",
            "folder-download",
            "folder-publicshare",
            "user-home",
            "user-desktop",
        ] {
            assert_eq!(icon_group(name), IconGroup::Places, "{name}");
        }
        for name in ["folder-git", "folder-code", "folder-red", "folder-games"] {
            assert_eq!(icon_group(name), IconGroup::Purpose, "{name}");
        }
    }

    #[test]
    fn folder_names_come_from_places_and_skip_states() {
        let base = temp_dir("names");
        write(
            &base.join("T/index.theme"),
            "[Icon Theme]\nName=T\nDirectories=48x48/places,48x48/apps,scalable/Places\n",
        );
        for name in [
            "folder.svg",
            "folder-red.svg",
            "folder-git.png",
            "folder-open.svg",
            "folder-symbolic.svg",
            "user-home.svg",
            "user-trash.svg",
            "network-server.svg",
        ] {
            write(&base.join("T/48x48/places").join(name), "");
        }
        write(&base.join("T/scalable/Places/folder-code.svg"), "");
        write(&base.join("T/48x48/apps/folder-app.svg"), "");

        let theme = &installed_themes_in(std::slice::from_ref(&base))[0];
        assert_eq!(folder_colours(theme), 0);
        write(&base.join("T/48x48/places/folder-red-documents.svg"), "");
        write(&base.join("T/48x48/places/folder-blue-documents.svg"), "");
        write(
            &base.join("T/scalable/Places/folder-blue-documents.svg"),
            "",
        );
        assert_eq!(folder_colours(theme), 2);
        assert_eq!(
            folder_icon_names(theme),
            [
                "folder",
                "folder-code",
                "folder-git",
                "folder-red",
                "user-home"
            ]
        );
        fs::remove_dir_all(base).unwrap();
    }
}
