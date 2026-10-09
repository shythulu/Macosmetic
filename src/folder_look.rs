// SPDX-License-Identifier: GPL-3.0-only

//! Per-folder looks: a colour, an icon from a theme, or the user's own image.
//!
//! The user's choices live in the app config (`Config::folder_looks`), keyed by path,
//! and are mirrored here so the folder scan, which runs off the UI thread, can read
//! them. A KDE `.directory` file with `Icon=` is honoured as a fallback but never
//! written: the app keeps its own choices out of the user's folders.
//!
//! Colours and theme icons store intent rather than pixels, so they keep matching
//! when the icon theme changes.

use cosmic::widget::icon;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, RwLock};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum FolderLook {
    /// One of [`FOLDER_COLOURS`], by id. Drawn from the theme's own coloured folder
    /// (`folder-<colour>-<kind>`, `folder-<colour>`) when it has one, otherwise by
    /// recolouring the theme's folder.
    Colour(String),
    /// A themed icon name such as `folder-git`. `theme: Some(..)` pins it to one
    /// installed theme; `None` follows the active theme.
    Icon { theme: Option<String>, name: String },
    /// An image the user picked, copied into the app's data directory.
    Image(PathBuf),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FolderColour {
    /// The name icon themes use, as in Papirus' `folder-red` or Breeze's `folder-red`.
    pub id: &'static str,
    /// The colour used when the theme has no coloured folder of its own.
    pub rgb: [u8; 3],
}

/// Colour names shared by Papirus, Breeze and the GNOME folder-color extension.
pub const FOLDER_COLOURS: &[FolderColour] = &[
    FolderColour {
        id: "red",
        rgb: [0xe0, 0x1b, 0x24],
    },
    FolderColour {
        id: "orange",
        rgb: [0xff, 0x78, 0x00],
    },
    FolderColour {
        id: "yellow",
        rgb: [0xf6, 0xd3, 0x2d],
    },
    FolderColour {
        id: "green",
        rgb: [0x33, 0xd1, 0x7a],
    },
    FolderColour {
        id: "cyan",
        rgb: [0x2e, 0xc2, 0xd6],
    },
    FolderColour {
        id: "blue",
        rgb: [0x35, 0x84, 0xe4],
    },
    FolderColour {
        id: "violet",
        rgb: [0x91, 0x41, 0xac],
    },
    FolderColour {
        id: "magenta",
        rgb: [0xc0, 0x61, 0xcb],
    },
    FolderColour {
        id: "brown",
        rgb: [0x98, 0x6a, 0x44],
    },
    FolderColour {
        id: "grey",
        rgb: [0x9a, 0x99, 0x96],
    },
];

pub fn folder_colour(id: &str) -> Option<&'static FolderColour> {
    FOLDER_COLOURS.iter().find(|colour| colour.id == id)
}

/// The translated name of a colour, for the menu and the drawer.
pub fn colour_label(id: &str) -> String {
    match id {
        "red" => crate::fl!("colour-red"),
        "orange" => crate::fl!("colour-orange"),
        "yellow" => crate::fl!("colour-yellow"),
        "green" => crate::fl!("colour-green"),
        "cyan" => crate::fl!("colour-cyan"),
        "blue" => crate::fl!("colour-blue"),
        "violet" => crate::fl!("colour-violet"),
        "magenta" => crate::fl!("colour-magenta"),
        "brown" => crate::fl!("colour-brown"),
        "grey" => crate::fl!("colour-grey"),
        other => other.to_string(),
    }
}

/// What a selection of folders has in common.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Shared {
    /// No folder has a look.
    None,
    /// The folders have different looks, or only some have one.
    Mixed,
    /// Every folder has this look.
    Look(FolderLook),
}

impl Shared {
    pub fn look(&self) -> Option<&FolderLook> {
        match self {
            Self::Look(look) => Some(look),
            _ => None,
        }
    }

    /// The colour id when every folder has the same colour, `Some(None)` when none has
    /// any look, `None` when they differ.
    pub fn colour(&self) -> Option<Option<&str>> {
        match self {
            Self::None => Some(None),
            Self::Look(FolderLook::Colour(id)) => Some(Some(id)),
            _ => None,
        }
    }
}

/// Folds the looks of a selection into one [`Shared`] state.
pub fn shared_look(looks: impl IntoIterator<Item = Option<FolderLook>>) -> Shared {
    let mut looks = looks.into_iter();
    let Some(first) = looks.next() else {
        return Shared::None;
    };
    if looks.any(|look| look != first) {
        return Shared::Mixed;
    }
    first.map_or(Shared::None, Shared::Look)
}

/// The looks the selected `paths` share; see [`shared_look`].
pub fn shared_stored_look<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Shared {
    shared_look(paths.into_iter().map(stored_look))
}

/// How many icon picks the drawer's Recent row remembers.
pub const RECENT_MAX: usize = 8;

/// Puts `look` at the front of `recent`, dropping any older copy and anything past
/// [`RECENT_MAX`].
pub fn push_recent(recent: &mut Vec<FolderLook>, look: FolderLook) {
    recent.retain(|old| *old != look);
    recent.insert(0, look);
    recent.truncate(RECENT_MAX);
}

static LOOKS: LazyLock<RwLock<FxHashMap<PathBuf, FolderLook>>> =
    LazyLock::new(|| RwLock::new(FxHashMap::default()));

/// Replaces the looks the scan sees with the ones from the config.
pub fn set_looks(looks: &BTreeMap<PathBuf, FolderLook>) {
    let mut map = LOOKS.write().unwrap();
    map.clear();
    map.extend(
        looks
            .iter()
            .map(|(path, look)| (path.clone(), look.clone())),
    );
}

/// The look the user chose in this app, if any.
pub fn stored_look(path: &Path) -> Option<FolderLook> {
    LOOKS.read().unwrap().get(path).cloned()
}

/// The look to draw: the app's own choice, then a KDE `.directory` `Icon=`.
///
/// `read_directory_file` is false for remote folders, where one more file read per
/// subfolder would slow listings down.
pub fn look_for(path: &Path, read_directory_file: bool) -> Option<FolderLook> {
    stored_look(path).or_else(|| {
        if read_directory_file {
            directory_file_look(path)
        } else {
            None
        }
    })
}

/// Reads `<folder>/.directory`, the per-folder settings file KDE Dolphin writes.
pub fn directory_file_look(folder: &Path) -> Option<FolderLook> {
    const MAX_LEN: u64 = 64 * 1024;
    let path = folder.join(".directory");
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LEN {
        return None;
    }
    parse_directory_file(&fs::read_to_string(path).ok()?, folder)
}

/// The `Icon=` of a `.directory` file's `[Desktop Entry]` group.
///
/// A value with a slash is a path, relative to the folder unless absolute; anything
/// else is a themed icon name.
pub fn parse_directory_file(text: &str, folder: &Path) -> Option<FolderLook> {
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "Icon" {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        // Dolphin's colour menu writes the theme's coloured folder, e.g. `folder-red`.
        // Treat that as the colour, so it still shows in themes without that icon.
        if let Some(colour) = value.strip_prefix("folder-").and_then(folder_colour) {
            return Some(FolderLook::Colour(colour.id.to_string()));
        }
        return Some(if value.contains('/') {
            let path = Path::new(value);
            if path.is_absolute() {
                FolderLook::Image(path.to_path_buf())
            } else {
                FolderLook::Image(folder.join(path.strip_prefix("./").unwrap_or(path)))
            }
        } else {
            FolderLook::Icon {
                theme: None,
                name: value.to_string(),
            }
        });
    }
    None
}

/// The icon for a folder with `look`, where `base` is the name the folder would
/// otherwise use (`folder`, `folder-documents`, `user-home`, ...).
///
/// `None` means draw the default icon: the look's image is missing, or a colour
/// could not be produced for a raster-only theme.
pub fn folder_handle(look: &FolderLook, base: &str, size: u16) -> Option<icon::Handle> {
    match look {
        FolderLook::Colour(id) => colour_handle(folder_colour(id)?, base, size),
        FolderLook::Icon { theme, name } => {
            Some(theme_icon_handle(theme.as_deref(), name, base, size))
        }
        FolderLook::Image(path) => path.exists().then(|| icon::from_path(path.clone())),
    }
}

/// The monochrome icon for the sidebar. A colour has no symbolic form, so it keeps
/// the default symbolic icon.
pub fn folder_handle_symbolic(look: &FolderLook, base: &str, size: u16) -> Option<icon::Handle> {
    match look {
        FolderLook::Colour(_) => None,
        FolderLook::Icon { name, .. } => Some(
            icon::from_name(format!("{name}-symbolic"))
                .size(size)
                .fallback(Some(icon::IconFallback::Names(vec![
                    Cow::Owned(format!("{base}-symbolic")),
                    Cow::Borrowed("folder-symbolic"),
                ])))
                .handle(),
        ),
        FolderLook::Image(path) => path.exists().then(|| icon::from_path(path.clone())),
    }
}

fn theme_icon_handle(theme: Option<&str>, name: &str, base: &str, size: u16) -> icon::Handle {
    #[cfg(unix)]
    if let Some(theme) = theme
        && let Some(path) = freedesktop_icons::lookup(name)
            .with_theme(theme)
            .with_size(size)
            .with_cache()
            .force_svg()
            .find()
    {
        return icon::from_path(path);
    }
    #[cfg(not(unix))]
    let _ = theme;
    icon::from_name(name)
        .prefer_svg(true)
        .size(size)
        .fallback(Some(icon::IconFallback::Names(vec![
            Cow::Owned(base.to_string()),
            Cow::Borrowed("folder"),
        ])))
        .handle()
}

/// Theme icon names that already show `base` in `colour`, best match first.
///
/// Papirus names a coloured Documents folder `folder-red-documents` and its coloured
/// Public folder `folder-red-public`; Breeze only has the plain `folder-red`.
pub fn colour_candidates(base: &str, colour: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(kind) = base.strip_prefix("folder-") {
        let aliases: &[&str] = match kind {
            "publicshare" => &["publicshare", "public"],
            "download" => &["download", "downloads"],
            _ => &[],
        };
        if aliases.is_empty() {
            names.push(format!("folder-{colour}-{kind}"));
        } else {
            names.extend(
                aliases
                    .iter()
                    .map(|alias| format!("folder-{colour}-{alias}")),
            );
        }
    } else if let Some(kind) = base.strip_prefix("user-") {
        names.push(format!("user-{colour}-{kind}"));
    }
    names.push(format!("folder-{colour}"));
    names
}

type ColourKey = (String, &'static str, u16);

static COLOUR_CACHE: LazyLock<Mutex<FxHashMap<ColourKey, Option<icon::Handle>>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));

/// Drops cached coloured icons, after the icon theme changed.
pub fn clear_cache() {
    COLOUR_CACHE.lock().unwrap().clear();
}

fn colour_handle(colour: &'static FolderColour, base: &str, size: u16) -> Option<icon::Handle> {
    let key = (base.to_string(), colour.id, size);
    if let Some(handle) = COLOUR_CACHE.lock().unwrap().get(&key) {
        return handle.clone();
    }
    let handle =
        themed_colour_handle(colour, base, size).or_else(|| recoloured_handle(colour, base, size));
    COLOUR_CACHE.lock().unwrap().insert(key, handle.clone());
    handle
}

fn themed_colour_handle(colour: &FolderColour, base: &str, size: u16) -> Option<icon::Handle> {
    colour_candidates(base, colour.id)
        .into_iter()
        .find_map(|name| {
            icon::from_name(name)
                .prefer_svg(true)
                .size(size)
                .fallback(None)
                .path()
                .map(icon::from_path)
        })
}

/// The theme's own folder icon for `base`, repainted in `colour`.
fn recoloured_handle(colour: &FolderColour, base: &str, size: u16) -> Option<icon::Handle> {
    let handle = icon::from_name(base)
        .prefer_svg(true)
        .size(size)
        .fallback(Some(icon::IconFallback::Names(vec![Cow::Borrowed(
            "folder",
        )])))
        .handle();
    let icon::Data::Svg(svg) = handle.data else {
        return None;
    };
    let bytes = match svg.data() {
        cosmic::iced::advanced::svg::Data::Path(path) => fs::read(path).ok()?,
        cosmic::iced::advanced::svg::Data::Bytes(bytes) => bytes.to_vec(),
    };
    let text = String::from_utf8(bytes).ok()?;
    Some(icon::from_svg_bytes(
        recolour_svg(&text, colour.rgb).into_bytes(),
    ))
}

/// Repaints every colour in an SVG with `target`'s hue and saturation.
///
/// Lightness is shifted as a whole so the icon's average matches the target while
/// its own shading is kept. Near-white highlights and near-black shadows stay as
/// they are. KDE's `ColorScheme-*` style blocks are plain hex colours too, so
/// Breeze-style folders recolour the same way.
pub fn recolour_svg(svg: &str, target: [u8; 3]) -> String {
    let (target_h, target_s, target_l) = rgb_to_hsl(target);
    let colours: Vec<_> = hex_colours(svg).collect();
    let paintable: Vec<f32> = colours
        .iter()
        .map(|(_, _, rgb)| rgb_to_hsl(*rgb).2)
        .filter(|l| is_paintable(*l))
        .collect();
    if paintable.is_empty() {
        return svg.to_string();
    }
    let mean_l = paintable.iter().sum::<f32>() / paintable.len() as f32;
    let shift = target_l - mean_l;

    let mut out = String::with_capacity(svg.len());
    let mut last = 0;
    for (start, end, rgb) in colours {
        let (_, _, l) = rgb_to_hsl(rgb);
        if !is_paintable(l) {
            continue;
        }
        let new = hsl_to_rgb(target_h, target_s, (l + shift).clamp(0.05, 0.95));
        out.push_str(&svg[last..start]);
        out.push_str(&format!("#{:02x}{:02x}{:02x}", new[0], new[1], new[2]));
        last = end;
    }
    out.push_str(&svg[last..]);
    out
}

fn is_paintable(lightness: f32) -> bool {
    (0.05..=0.94).contains(&lightness)
}

/// `#rgb` and `#rrggbb` colours with their byte ranges, skipping `url(#id)` and
/// `href="#id"` references, whose ids can look like hex.
fn hex_colours(svg: &str) -> impl Iterator<Item = (usize, usize, [u8; 3])> + '_ {
    let bytes = svg.as_bytes();
    svg.match_indices('#').filter_map(move |(start, _)| {
        let before = &svg[..start];
        if before.ends_with("url(") || before.ends_with("href=\"") || before.ends_with("href='") {
            return None;
        }
        let digits = bytes[start + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_alphanumeric())
            .count();
        let hex = &svg[start + 1..start + 1 + digits];
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let value = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
        let rgb = match digits {
            6 => [value(0..2)?, value(2..4)?, value(4..6)?],
            3 => [value(0..1)? * 17, value(1..2)? * 17, value(2..3)? * 17],
            _ => return None,
        };
        Some((start, start + 1 + digits, rgb))
    })
}

fn rgb_to_hsl([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = delta / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [byte(r), byte(g), byte(b)]
}

/// Paths whose look differs between two sets of looks.
pub fn changed_paths(
    old: &BTreeMap<PathBuf, FolderLook>,
    new: &BTreeMap<PathBuf, FolderLook>,
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = old
        .iter()
        .filter(|(path, look)| new.get(*path) != Some(look))
        .map(|(path, _)| path.clone())
        .collect();
    paths.extend(
        new.iter()
            .filter(|(path, look)| old.get(*path) != Some(look))
            .map(|(path, _)| path.clone()),
    );
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// Moves looks along with folders the app renamed or moved.
///
/// Each change is `(from, to)`; looks on `from` and on anything inside it follow.
/// Returns whether anything moved.
pub fn rekey(
    looks: &mut BTreeMap<PathBuf, FolderLook>,
    changes: &[(impl AsRef<Path>, impl AsRef<Path>)],
) -> bool {
    let mut moved = Vec::new();
    for path in looks.keys() {
        for (from, to) in changes {
            if let Ok(relative) = path.strip_prefix(from.as_ref()) {
                moved.push((path.clone(), to.as_ref().join(relative)));
                break;
            }
        }
    }
    for (old, new) in &moved {
        if let Some(look) = looks.remove(old) {
            looks.insert(new.clone(), look);
        }
    }
    !moved.is_empty()
}

/// Where imported images live: `<data dir>/<app id>/folder-images`.
pub fn images_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| crate::home_dir().join(".local").join("share"))
        .join(<crate::app::App as cosmic::Application>::APP_ID)
        .join("folder-images")
}

/// Image types the folder icon can show.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "svg", "webp"];

/// Copies an image into [`images_dir`] so the look survives the original moving.
///
/// The copy is named by a hash of its contents, so importing the same image twice
/// shares one file.
pub fn import_image(source: &Path) -> io::Result<PathBuf> {
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .filter(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a supported image"))?;
    let bytes = fs::read(source)?;
    let digest = Sha256::digest(&bytes);
    let name: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let dir = images_dir();
    fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{name}.{extension}"));
    if !target.exists() {
        fs::write(&target, bytes)?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_file_icon_names_and_paths() {
        let folder = Path::new("/home/me/Projects");
        assert_eq!(
            parse_directory_file("[Desktop Entry]\nIcon=folder-git\n", folder),
            Some(FolderLook::Icon {
                theme: None,
                name: "folder-git".into()
            })
        );
        assert_eq!(
            parse_directory_file("[Desktop Entry]\nIcon=folder-red\n", folder),
            Some(FolderLook::Colour("red".into()))
        );
        assert_eq!(
            parse_directory_file("[Desktop Entry]\nIcon=./cover.png\n", folder),
            Some(FolderLook::Image(folder.join("cover.png")))
        );
        assert_eq!(
            parse_directory_file("[Desktop Entry]\nIcon=/usr/share/x.svg\n", folder),
            Some(FolderLook::Image("/usr/share/x.svg".into()))
        );
        // View settings Dolphin also writes, without an icon.
        assert_eq!(
            parse_directory_file("[Dolphin]\nViewMode=1\nIcon=nope\n", folder),
            None
        );
        assert_eq!(
            parse_directory_file("[Desktop Entry]\nIcon=\n", folder),
            None
        );
    }

    #[test]
    fn colour_candidates_cover_papirus_names() {
        assert_eq!(colour_candidates("folder", "red"), ["folder-red"]);
        assert_eq!(
            colour_candidates("folder-documents", "red"),
            ["folder-red-documents", "folder-red"]
        );
        assert_eq!(
            colour_candidates("folder-publicshare", "blue"),
            [
                "folder-blue-publicshare",
                "folder-blue-public",
                "folder-blue"
            ]
        );
        assert_eq!(
            colour_candidates("user-home", "green"),
            ["user-green-home", "folder-green"]
        );
    }

    #[test]
    fn recolour_tints_greys_and_keeps_highlights_and_references() {
        let svg = r##"<svg><defs><linearGradient id="abc"/></defs><path fill="url(#abc)"/><use href="#bad"/><path fill="#808080"/><path style="fill:#484848;stroke:#fff"/><path fill="#000000"/></svg>"##;
        let out = recolour_svg(svg, [0xe0, 0x1b, 0x24]);
        assert!(out.contains("url(#abc)"));
        assert!(out.contains("href=\"#bad\""));
        assert!(out.contains("stroke:#fff"));
        assert!(out.contains("fill=\"#000000\""));
        assert!(!out.contains("#808080"));
        assert!(!out.contains("#484848"));
        // The repainted greys are now red: red channel well above green and blue.
        for (_, _, rgb) in hex_colours(&out) {
            if rgb != [0xff; 3] && rgb != [0; 3] {
                assert!(rgb[0] > rgb[1] + 40 && rgb[0] > rgb[2] + 40, "{rgb:?}");
            }
        }
    }

    #[test]
    fn recolour_handles_kde_colour_scheme_blocks() {
        let svg = ".ColorScheme-Accent { color:#3daee9; }";
        let out = recolour_svg(svg, [0x33, 0xd1, 0x7a]);
        let (_, _, rgb) = hex_colours(&out).next().unwrap();
        assert!(rgb[1] > rgb[0] && rgb[1] > rgb[2], "{rgb:?}");
    }

    #[test]
    fn hsl_round_trips() {
        for rgb in [
            [0xe0, 0x1b, 0x24],
            [0x35, 0x84, 0xe4],
            [0x80, 0x80, 0x80],
            [0x12, 0xab, 0x34],
        ] {
            let (h, s, l) = rgb_to_hsl(rgb);
            let back = hsl_to_rgb(h, s, l);
            for i in 0..3 {
                assert!(rgb[i].abs_diff(back[i]) <= 1, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn shared_look_distinguishes_none_mixed_and_same() {
        let red = || Some(FolderLook::Colour("red".into()));
        assert_eq!(shared_look(Vec::new()), Shared::None);
        assert_eq!(shared_look([None, None]), Shared::None);
        assert_eq!(shared_look([red(), None]), Shared::Mixed);
        assert_eq!(
            shared_look([red(), Some(FolderLook::Colour("blue".into()))]),
            Shared::Mixed
        );
        let same = shared_look([red(), red()]);
        assert_eq!(same, Shared::Look(red().unwrap()));
        assert_eq!(same.colour(), Some(Some("red")));
        assert_eq!(Shared::None.colour(), Some(None));
        assert_eq!(Shared::Mixed.colour(), None);
        let icon = Shared::Look(FolderLook::Icon {
            theme: None,
            name: "folder-git".into(),
        });
        // An icon look has no colour to check, but it is not "none" either.
        assert_eq!(icon.colour(), None);
    }

    #[test]
    fn recent_picks_dedupe_and_cap() {
        let icon = |name: &str| FolderLook::Icon {
            theme: None,
            name: name.into(),
        };
        let mut recent = Vec::new();
        for i in 0..RECENT_MAX + 2 {
            push_recent(&mut recent, icon(&format!("folder-{i}")));
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent[0], icon(&format!("folder-{}", RECENT_MAX + 1)));
        // Picking an old one again moves it to the front without a duplicate.
        push_recent(&mut recent, icon("folder-5"));
        assert_eq!(recent[0], icon("folder-5"));
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(
            recent
                .iter()
                .filter(|look| **look == icon("folder-5"))
                .count(),
            1
        );
    }

    #[test]
    fn rekey_follows_renames_and_moves_including_children() {
        let mut looks = BTreeMap::new();
        looks.insert(PathBuf::from("/a/Work"), FolderLook::Colour("red".into()));
        looks.insert(
            PathBuf::from("/a/Work/Sub"),
            FolderLook::Colour("blue".into()),
        );
        looks.insert(
            PathBuf::from("/a/Workshop"),
            FolderLook::Colour("green".into()),
        );
        assert!(rekey(&mut looks, &[("/a/Work", "/b/Job")]));
        assert_eq!(
            looks.keys().collect::<Vec<_>>(),
            [
                Path::new("/a/Workshop"),
                Path::new("/b/Job"),
                Path::new("/b/Job/Sub")
            ]
        );
        assert!(!rekey(&mut looks, &[("/nowhere", "/else")]));
    }
}
