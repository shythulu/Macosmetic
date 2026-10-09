// SPDX-License-Identifier: GPL-3.0-only

//! The icon themes the gallery offers for download, and their installation.
//!
//! The catalog is `res/icon-themes/catalog.json`, written by `scripts/icon-theme-catalog.py`
//! from `res/icon-themes/sources.json`. Each theme names a pinned archive with its SHA-256,
//! the directory the theme sits in inside that archive, and the catalog themes it needs
//! beside it. The previews were drawn from the archives by the same script and are compiled
//! in, so browsing needs no network.
//!
//! Installing downloads each archive with `curl`, checks it against the catalog's SHA-256,
//! and unpacks only the theme directories into a staging directory under the user's icon
//! directory, which is then renamed into place. Archives are treated as untrusted: entries
//! that would land outside the theme are dropped, as are symlinks that point outside the
//! icon directory or through which a later entry would be written. APFS is case-insensitive,
//! and several themes ship paths that differ only by case, so the first of those wins.
//!
//! The icon lookup reads the installed themes once per process, so a theme installed here
//! is only usable after a restart.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use cosmic::widget::icon;
use rust_embed::RustEmbed;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Written into every theme directory this module installs; only those may be removed.
pub const MARKER: &str = ".macosmetic-catalog";

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub archives: Vec<Archive>,
    pub themes: Vec<CatalogTheme>,
}

#[derive(Debug, Deserialize)]
pub struct Archive {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct CatalogTheme {
    /// Directory name, the value stored in the toolkit config.
    pub id: String,
    pub name: String,
    pub license: String,
    pub homepage: String,
    /// Index into [`Catalog::archives`].
    pub archive: usize,
    /// The theme's directory inside the archive.
    pub path: String,
    /// An `index.theme` kept outside the theme directory, as Adwaita's is.
    #[serde(default)]
    pub index_theme: Option<String>,
    /// Catalog themes that have to be installed beside this one, dependencies first.
    pub requires: Vec<String>,
    /// Preview file names under `res/icon-themes/previews/<id>/`.
    pub previews: Vec<String>,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../res/icon-themes/catalog.json"))
        .expect("res/icon-themes/catalog.json is valid")
});

pub fn catalog() -> &'static Catalog {
    &CATALOG
}

pub fn theme(id: &str) -> Option<&'static CatalogTheme> {
    CATALOG.themes.iter().find(|theme| theme.id == id)
}

#[derive(RustEmbed)]
#[folder = "res/icon-themes/previews"]
struct Previews;

/// The compiled-in preview strip for `theme`.
pub fn previews(theme: &CatalogTheme) -> Vec<icon::Handle> {
    theme
        .previews
        .iter()
        .filter_map(|file| {
            let data = Previews::get(&format!("{}/{file}", theme.id))?.data;
            Some(if file.ends_with(".svg") {
                icon::from_svg_bytes(data)
            } else {
                icon::from_raster_bytes(data.into_owned())
            })
        })
        .collect()
}

/// The user's icon directory, where themes are installed.
pub fn user_icons_dir() -> PathBuf {
    crate::icon_themes::data_home(&crate::home_dir()).join("icons")
}

/// What installing `id` takes: the theme and every catalog theme it needs that is not in
/// `installed`, dependencies first.
pub fn install_plan(id: &str, installed: &HashSet<String>) -> Vec<&'static CatalogTheme> {
    fn visit(
        id: &str,
        installed: &HashSet<String>,
        seen: &mut HashSet<String>,
        plan: &mut Vec<&'static CatalogTheme>,
    ) {
        if installed.contains(id) || !seen.insert(id.to_string()) {
            return;
        }
        let Some(theme) = theme(id) else {
            return;
        };
        for parent in &theme.requires {
            visit(parent, installed, seen, plan);
        }
        plan.push(theme);
    }
    let mut plan = Vec::new();
    visit(id, installed, &mut HashSet::new(), &mut plan);
    plan
}

/// The download size of `plan`, counting each archive once.
pub fn plan_size(plan: &[&CatalogTheme]) -> u64 {
    let mut archives: Vec<usize> = plan.iter().map(|theme| theme.archive).collect();
    archives.sort_unstable();
    archives.dedup();
    archives
        .into_iter()
        .filter_map(|index| CATALOG.archives.get(index))
        .map(|archive| archive.size)
        .sum()
}

#[derive(Clone, Debug)]
pub enum InstallEvent {
    /// Overall progress through the downloads, from 0 to 1.
    Progress(String, f32),
    /// The theme asked for is installed, along with the listed themes it needed.
    Installed(String, Vec<String>),
    Failed(String, String),
}

/// Install `plan`, reporting through `emit`. Blocking: run it on its own thread. `id` is the
/// theme the user asked for, which every event is about.
pub fn install(id: String, plan: Vec<&'static CatalogTheme>, emit: impl Fn(InstallEvent)) {
    let icons_dir = user_icons_dir();
    let result = install_in(&icons_dir, &plan, |fraction| {
        emit(InstallEvent::Progress(id.clone(), fraction));
    });
    match result {
        Ok(()) => {
            let ids = plan.iter().map(|theme| theme.id.clone()).collect();
            emit(InstallEvent::Installed(id, ids));
        }
        Err(err) => {
            log::warn!("installing icon theme {id} failed: {err}");
            emit(InstallEvent::Failed(id, err.to_string()));
        }
    }
}

fn install_in(
    icons_dir: &Path,
    plan: &[&CatalogTheme],
    progress: impl Fn(f32),
) -> io::Result<()> {
    fs::create_dir_all(icons_dir)?;
    let staging = tempfile::Builder::new()
        .prefix(".macosmetic-staging-")
        .tempdir_in(icons_dir)?;

    let total = plan_size(plan).max(1);
    let mut done = 0;
    let mut archive_indexes: Vec<usize> = plan.iter().map(|theme| theme.archive).collect();
    archive_indexes.dedup();
    let mut seen = HashSet::new();
    for index in archive_indexes {
        if !seen.insert(index) {
            continue;
        }
        let archive = CATALOG.archives.get(index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "catalog archive missing")
        })?;
        let file = staging.path().join(format!("archive-{index}.tar.gz"));
        download(&archive.url, &file, |bytes| {
            progress((done + bytes) as f32 / total as f32);
        })?;
        verify_sha256(&file, &archive.sha256)?;
        done += archive.size;

        let themes: Vec<_> = plan.iter().filter(|theme| theme.archive == index).collect();
        let targets: Vec<ExtractTarget> = themes
            .iter()
            .map(|theme| ExtractTarget {
                root: theme.path.clone(),
                index_theme: theme.index_theme.clone(),
                id: theme.id.clone(),
            })
            .collect();
        let decoder = flate2::read::GzDecoder::new(File::open(&file)?);
        let stats = extract(decoder, &targets, staging.path())?;
        log::info!("unpacked {index}: {stats:?}");
        fs::remove_file(&file)?;

        for theme in themes {
            let staged = staging.path().join(&theme.id);
            if !staged.join("index.theme").is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} has no index.theme", theme.id),
                ));
            }
            fs::write(staged.join(MARKER), format!("{}\n", archive.sha256))?;
            let target = icons_dir.join(&theme.id);
            if target.exists() {
                // Installed meanwhile, by another install that needed the same theme.
                log::info!("{} is already installed, keeping it", theme.id);
            } else {
                fs::rename(&staged, &target)?;
                log::info!("installed icon theme {} at {}", theme.id, target.display());
            }
        }
    }
    progress(1.0);
    Ok(())
}

/// Download `url` to `dest` with `curl`, reporting the bytes received so far.
fn download(url: &str, dest: &Path, progress: impl Fn(u64)) -> io::Result<()> {
    let mut child = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--output",
        ])
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Ok(metadata) = fs::metadata(dest) {
            progress(metadata.len());
        }
        thread::sleep(Duration::from_millis(200));
    };
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        return Err(io::Error::other(format!(
            "download failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> io::Result<()> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    let actual = format!("{:x}", hasher.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("checksum mismatch: expected {expected}, got {actual}"),
        ))
    }
}

/// A theme directory to take out of an archive.
#[derive(Clone, Debug)]
pub struct ExtractTarget {
    /// The theme's directory inside the archive.
    pub root: String,
    /// An `index.theme` elsewhere in the archive, copied into the theme.
    pub index_theme: Option<String>,
    /// The directory it is unpacked to, under the destination.
    pub id: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct ExtractStats {
    pub files: usize,
    pub symlinks: usize,
    /// Entries dropped because an entry differing only by case came first.
    pub case_collisions: usize,
    /// Entries dropped as unsafe or unsupported.
    pub rejected: usize,
}

/// Unpack `targets` from the tar stream `reader` into `dest/<id>`.
///
/// Symlinks are kept, since most themes are built on them, but only relative ones that stay
/// inside the icon directory once the theme sits at `<icons>/<id>`; a variant may point into
/// a sibling theme installed beside it. Nothing is written through a symlink.
pub fn extract(reader: impl Read, targets: &[ExtractTarget], dest: &Path) -> io::Result<ExtractStats> {
    let mut stats = ExtractStats::default();
    // Per target, the lowercased paths written so far and the ones that are symlinks.
    let mut written: Vec<HashSet<String>> = vec![HashSet::new(); targets.len()];
    let mut links: Vec<HashSet<String>> = vec![HashSet::new(); targets.len()];

    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(path) = normal_path(&entry.path()?) else {
            stats.rejected += 1;
            continue;
        };

        // Where the entry goes: (target, path inside the theme).
        let mut placement = None;
        for (index, target) in targets.iter().enumerate() {
            if target.index_theme.as_deref() == Some(path.as_str()) {
                placement = Some((index, "index.theme".to_string()));
                break;
            }
            let relative = if target.root.is_empty() {
                Some(path.as_str())
            } else {
                path.strip_prefix(target.root.as_str())
                    .and_then(|rest| rest.strip_prefix('/'))
            };
            if let Some(relative) = relative.filter(|relative| !relative.is_empty()) {
                placement = Some((index, relative.to_string()));
                break;
            }
        }
        let Some((index, relative)) = placement else {
            continue;
        };

        let lower = relative.to_lowercase();
        if ancestors(&lower).any(|ancestor| links[index].contains(ancestor)) {
            // Its directory is a symlink; writing it would land outside this theme.
            stats.rejected += 1;
            continue;
        }
        let out = dest.join(&targets[index].id).join(&relative);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if !written[index].insert(lower.clone()) {
            stats.case_collisions += 1;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        if kind.is_file() {
            let mut file = File::create(&out)?;
            io::copy(&mut entry, &mut file)?;
            stats.files += 1;
        } else if kind.is_symlink() {
            let Some(link) = entry.link_name()? else {
                stats.rejected += 1;
                continue;
            };
            let link = link.into_owned();
            if !link_stays_inside(&targets[index].id, &relative, &link) {
                stats.rejected += 1;
                continue;
            }
            symlink(&link, &out)?;
            links[index].insert(lower);
            stats.symlinks += 1;
        } else {
            // Hard links, devices and the like have no place in an icon theme.
            stats.rejected += 1;
        }
    }
    Ok(stats)
}

/// `path` as `/`-joined normal components, or `None` if it is absolute or climbs with `..`.
fn normal_path(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Every proper ancestor of a `/`-joined path, nearest first.
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').rev().map(move |(at, _)| &path[..at])
}

/// Whether the symlink at `<icons>/<id>/<relative>` pointing to `link` resolves inside
/// `<icons>`.
fn link_stays_inside(id: &str, relative: &str, link: &Path) -> bool {
    let mut position: Vec<&str> = std::iter::once(id)
        .chain(relative.split('/'))
        .collect();
    position.pop();
    for component in link.components() {
        match component {
            Component::Normal(part) => match part.to_str() {
                Some(part) => position.push(part),
                None => return false,
            },
            Component::CurDir => {}
            Component::ParentDir => {
                if position.pop().is_none() {
                    return false;
                }
            }
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    !position.is_empty()
}

#[cfg(unix)]
fn symlink(link: &Path, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(link, at)
}

#[cfg(not(unix))]
fn symlink(_link: &Path, _at: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symlinks are not supported here",
    ))
}

/// Remove an installed theme, if this module installed it.
pub fn remove(id: &str) -> io::Result<()> {
    let dir = user_icons_dir().join(id);
    if !dir.join(MARKER).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{id} was not installed from the catalog"),
        ));
    }
    fs::remove_dir_all(&dir)
}

/// Whether the installed theme at `dir` came from the catalog.
pub fn is_catalog_install(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_file(builder: &mut tar::Builder<Vec<u8>>, path: &str, data: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, path, data).unwrap();
    }

    fn add_link(builder: &mut tar::Builder<Vec<u8>>, path: &str, target: &str) {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Symlink);
        builder.append_link(&mut header, path, target).unwrap();
    }

    /// Writes `path` into the raw header, past the checks `tar::Builder` makes.
    fn add_raw(builder: &mut tar::Builder<Vec<u8>>, path: &str, data: &[u8]) {
        let mut header = tar::Header::new_old();
        header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }

    fn target(root: &str, id: &str) -> ExtractTarget {
        ExtractTarget {
            root: root.to_string(),
            index_theme: None,
            id: id.to_string(),
        }
    }

    #[test]
    fn extract_keeps_safe_entries_and_drops_the_rest() {
        let mut builder = tar::Builder::new(Vec::new());
        add_file(&mut builder, "pkg-1.0/Theme/index.theme", b"[Icon Theme]\nName=Theme\n");
        add_file(&mut builder, "pkg-1.0/Theme/places/folder.svg", b"<svg/>");
        add_link(&mut builder, "pkg-1.0/Theme/places/inode-directory.svg", "folder.svg");
        // A sibling theme the variant links into is allowed; leaving the icon dir is not.
        add_link(&mut builder, "pkg-1.0/Theme/apps", "../Base/apps");
        add_link(&mut builder, "pkg-1.0/Theme/escape", "../../../etc");
        add_link(&mut builder, "pkg-1.0/Theme/absolute", "/etc/passwd");
        // Written through the `apps` symlink, so dropped.
        add_file(&mut builder, "pkg-1.0/Theme/apps/evil.svg", b"x");
        // Differs from folder.svg only by case.
        add_file(&mut builder, "pkg-1.0/Theme/places/FOLDER.svg", b"other");
        add_raw(&mut builder, "pkg-1.0/Theme/../../outside.svg", b"x");
        add_file(&mut builder, "pkg-1.0/Other/index.theme", b"ignored");
        let data = builder.into_inner().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let stats = extract(&data[..], &[target("pkg-1.0/Theme", "Theme")], dest.path()).unwrap();
        let theme = dest.path().join("Theme");

        assert_eq!(fs::read(theme.join("places/folder.svg")).unwrap(), b"<svg/>");
        assert_eq!(
            fs::read_link(theme.join("places/inode-directory.svg")).unwrap(),
            Path::new("folder.svg")
        );
        assert_eq!(fs::read_link(theme.join("apps")).unwrap(), Path::new("../Base/apps"));
        assert!(fs::symlink_metadata(theme.join("escape")).is_err());
        assert!(fs::symlink_metadata(theme.join("absolute")).is_err());
        assert!(!dest.path().join("Base").exists());
        assert!(!dest.path().join("Other").exists());
        assert!(!dest.path().parent().unwrap().join("outside.svg").exists());
        assert_eq!(
            stats,
            ExtractStats {
                files: 2,
                symlinks: 2,
                case_collisions: 1,
                rejected: 4,
            }
        );
    }

    #[test]
    fn extract_takes_a_root_theme_and_an_outside_index_theme() {
        let mut builder = tar::Builder::new(Vec::new());
        add_file(&mut builder, "repo-1/index.theme", b"[Icon Theme]\nName=Root\n");
        add_file(&mut builder, "repo-1/places/folder.svg", b"<svg/>");
        add_file(&mut builder, "adwaita-51/index.theme", b"[Icon Theme]\nName=Adwaita\n");
        add_file(&mut builder, "adwaita-51/Adwaita/scalable/folder.svg", b"<svg/>");
        let root = builder.into_inner().unwrap();

        let dest = tempfile::tempdir().unwrap();
        extract(&root[..], &[target("repo-1", "Root")], dest.path()).unwrap();
        assert!(dest.path().join("Root/index.theme").is_file());
        assert!(dest.path().join("Root/places/folder.svg").is_file());

        let adwaita = ExtractTarget {
            root: "adwaita-51/Adwaita".to_string(),
            index_theme: Some("adwaita-51/index.theme".to_string()),
            id: "Adwaita".to_string(),
        };
        extract(&root[..], &[adwaita], dest.path()).unwrap();
        assert_eq!(
            fs::read(dest.path().join("Adwaita/index.theme")).unwrap(),
            b"[Icon Theme]\nName=Adwaita\n"
        );
        assert!(dest.path().join("Adwaita/scalable/folder.svg").is_file());
    }

    #[test]
    fn link_stays_inside_measures_from_the_installed_location() {
        assert!(link_stays_inside("T", "a/b.svg", Path::new("c.svg")));
        assert!(link_stays_inside("T", "16x16", Path::new("../Papirus/16x16")));
        assert!(link_stays_inside("T", "16x16/apps", Path::new("../../Papirus/16x16/apps")));
        assert!(!link_stays_inside("T", "16x16", Path::new("../../x")));
        assert!(!link_stays_inside("T", "a", Path::new("/usr/share/icons")));
    }

    #[test]
    fn catalog_parses_and_its_plans_put_dependencies_first() {
        let catalog = catalog();
        assert!(!catalog.themes.is_empty());
        for theme in &catalog.themes {
            assert!(theme.archive < catalog.archives.len(), "{}", theme.id);
            assert_eq!(previews(theme).len(), theme.previews.len(), "{}", theme.id);
            for parent in &theme.requires {
                assert!(super::theme(parent).is_some(), "{} needs {parent}", theme.id);
            }
        }

        let plan: Vec<_> = install_plan("Papirus-Dark", &HashSet::new())
            .iter()
            .map(|theme| theme.id.as_str())
            .collect();
        assert_eq!(plan, ["Papirus", "Papirus-Dark"]);

        let installed = HashSet::from(["Papirus".to_string()]);
        let plan: Vec<_> = install_plan("Papirus-Dark", &installed)
            .iter()
            .map(|theme| theme.id.as_str())
            .collect();
        assert_eq!(plan, ["Papirus-Dark"]);
    }
}
