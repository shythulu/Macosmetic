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
//! Every theme the app installs carries a marker file naming the archive it came from, so
//! only those can be removed, and an installed theme whose archive the catalog has since
//! moved on from can be updated in place. After any of these the app asks the icon lookup
//! to scan the theme directories again, so no restart is needed.

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cosmic::widget::icon;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::icon_themes::IconThemeInfo;

/// Written into every theme directory this module installs; only those may be removed.
pub const MARKER: &str = ".macosmetic-catalog";
/// Prefixes of the temporary directories this module makes beside the themes.
const STAGING_PREFIX: &str = ".macosmetic-staging-";
const REMOVING_PREFIX: &str = ".macosmetic-removing-";
/// How long a temporary directory may sit before [`cleanup_staging`] takes it as abandoned.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// How much larger than the catalog says a download may be before curl gives up on it.
const DOWNLOAD_SLACK: f64 = 1.25;

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

/// The ids of every theme directory in the user's icon directory, hidden ones included.
/// Dependencies such as Adwaita are `Hidden=true`, so the settings list leaves them out,
/// but they still count as installed when planning.
pub fn installed_ids() -> HashSet<String> {
    let Ok(entries) = fs::read_dir(user_icons_dir()) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().join("index.theme").is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
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

/// What updating the installed theme `id` takes: the theme itself, from the catalog's
/// current archive, and whatever it needs that is still missing.
pub fn update_plan(id: &str, installed: &HashSet<String>) -> Vec<&'static CatalogTheme> {
    let mut without = installed.clone();
    without.remove(id);
    install_plan(id, &without)
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

/// What this module leaves in every theme directory it installs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Marker {
    /// SHA-256 of the archive the theme came from; the catalog's copy moving on from it is
    /// what makes a theme updatable.
    pub sha256: String,
    /// `catalog`, or `file:<name>` for a theme installed from a local archive or folder.
    #[serde(default = "catalog_source")]
    pub source: String,
    /// RFC 3339 time of the installation; empty for markers written before it was recorded.
    #[serde(default)]
    pub installed: String,
}

fn catalog_source() -> String {
    "catalog".to_string()
}

impl Marker {
    fn new(sha256: &str, source: String) -> Self {
        Self {
            sha256: sha256.to_string(),
            source,
            installed: jiff::Timestamp::now().to_string(),
        }
    }

    /// The file name the theme was installed from, if it did not come from the catalog.
    pub fn file(&self) -> Option<&str> {
        self.source.strip_prefix("file:")
    }

    /// The calendar date of the installation, in the local time zone.
    pub fn installed_date(&self) -> Option<String> {
        let at: jiff::Timestamp = self.installed.parse().ok()?;
        Some(at.to_zoned(jiff::tz::TimeZone::system()).date().to_string())
    }
}

/// The marker in the theme directory `dir`, if this module installed it. The first markers
/// held only the archive's SHA-256 on a line of its own.
pub fn read_marker(dir: &Path) -> Option<Marker> {
    let text = fs::read_to_string(dir.join(MARKER)).ok()?;
    serde_json::from_str(&text).ok().or_else(|| {
        let sha256 = text.trim();
        (!sha256.is_empty() && !sha256.starts_with('{')).then(|| Marker {
            sha256: sha256.to_string(),
            source: catalog_source(),
            installed: String::new(),
        })
    })
}

fn write_marker(dir: &Path, marker: &Marker) -> io::Result<()> {
    let mut text = serde_json::to_string(marker).map_err(io::Error::other)?;
    text.push('\n');
    fs::write(dir.join(MARKER), text)
}

/// Whether the installed theme at `dir` came from the catalog.
pub fn is_catalog_install(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

/// Whether the catalog holds a newer archive than the installed copy of `theme` came from.
/// Themes installed by hand or from a file never do.
pub fn needs_update(theme: &IconThemeInfo) -> bool {
    let Some(entry) = self::theme(&theme.id) else {
        return false;
    };
    let Some(archive) = CATALOG.archives.get(entry.archive) else {
        return false;
    };
    theme
        .roots
        .first()
        .and_then(|root| read_marker(root))
        .is_some_and(|marker| marker.file().is_none() && marker.sha256 != archive.sha256)
}

/// Why an installation stopped. [`InstallError::detail`] keeps the raw text for a Details
/// view; the gallery shows a plain sentence per variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallError {
    /// curl could not reach or keep talking to the host.
    Network { host: String, detail: String },
    /// The pinned URL answers 404 or 410: the catalog has to be regenerated.
    Moved,
    /// The download or the unpacked theme passed the cap, in bytes.
    TooLarge(u64),
    Checksum,
    /// A local archive had this many entries that were not safe to unpack.
    UnsafeArchive(usize),
    /// A local archive or folder holds no `index.theme`.
    NoTheme,
    /// A theme with this id is installed, and not by this app.
    Exists(String),
    NoSpace { needed: u64 },
    Cancelled,
    Io(String),
}

impl InstallError {
    /// The raw text behind the error, for a Details view.
    pub fn detail(&self) -> String {
        match self {
            Self::Network { detail, .. } => detail.clone(),
            Self::Io(detail) => detail.clone(),
            other => format!("{other:?}"),
        }
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network { host, detail } => write!(f, "could not reach {host}: {detail}"),
            Self::Moved => f.write_str("the download link has moved"),
            Self::TooLarge(size) => write!(f, "larger than the {size} byte cap"),
            Self::Checksum => f.write_str("checksum mismatch"),
            Self::UnsafeArchive(count) => write!(f, "{count} unsafe entries"),
            Self::NoTheme => f.write_str("no icon theme found"),
            Self::Exists(id) => write!(f, "{id} is already installed outside the app"),
            Self::NoSpace { needed } => write!(f, "not enough space ({needed} bytes needed)"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Io(detail) => f.write_str(detail),
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::StorageFull => Self::NoSpace { needed: 0 },
            _ => Self::Io(err.to_string()),
        }
    }
}

/// Where an installation stands, for the card's progress bar and caption.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Bytes received so far of all the plan's downloads.
    Downloading { done: u64, total: u64 },
    Extracting,
}

impl Step {
    /// Overall progress from 0 to 1.
    pub fn fraction(self) -> f32 {
        match self {
            Self::Downloading { done, total } => (done as f32 / total.max(1) as f32).min(1.0),
            Self::Extracting => 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum InstallEvent {
    Progress(String, Step),
    /// The theme asked for is installed, along with the listed themes it needed.
    Installed(String, Vec<String>),
    Failed(String, InstallError),
    Cancelled(String),
}

/// Install `plan`, reporting through `emit`. Blocking: run it on its own thread. `id` is the
/// theme the user asked for, which every event is about. Themes in `replace` are swapped
/// for the freshly unpacked copy; any other theme already in place is kept. Setting
/// `cancel` stops the download and removes everything staged.
pub fn install(
    id: String,
    plan: Vec<&'static CatalogTheme>,
    replace: HashSet<String>,
    cancel: Arc<AtomicBool>,
    emit: impl Fn(InstallEvent),
) {
    let icons_dir = user_icons_dir();
    let result = install_in(&icons_dir, &plan, &replace, &cancel, |step| {
        emit(InstallEvent::Progress(id.clone(), step));
    });
    match result {
        Ok(()) => {
            let ids = plan.iter().map(|theme| theme.id.clone()).collect();
            emit(InstallEvent::Installed(id, ids));
        }
        Err(InstallError::Cancelled) => {
            log::info!("installing icon theme {id} was cancelled");
            emit(InstallEvent::Cancelled(id));
        }
        Err(InstallError::NoSpace { .. }) => {
            // The unpacked themes are mostly symlinks, so this is a loose upper bound.
            let needed = plan_size(&plan) * 2;
            log::warn!("installing icon theme {id} failed: out of space");
            emit(InstallEvent::Failed(id, InstallError::NoSpace { needed }));
        }
        Err(err) => {
            log::warn!("installing icon theme {id} failed: {err}");
            emit(InstallEvent::Failed(id, err));
        }
    }
}

fn install_in(
    icons_dir: &Path,
    plan: &[&CatalogTheme],
    replace: &HashSet<String>,
    cancel: &AtomicBool,
    progress: impl Fn(Step),
) -> Result<(), InstallError> {
    fs::create_dir_all(icons_dir)?;
    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(icons_dir)?;

    let total = plan_size(plan);
    let mut done = 0;
    let mut archive_indexes: Vec<usize> = plan.iter().map(|theme| theme.archive).collect();
    archive_indexes.dedup();
    let mut seen = HashSet::new();
    for index in archive_indexes {
        if !seen.insert(index) {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        let archive = CATALOG
            .archives
            .get(index)
            .ok_or_else(|| InstallError::Io("catalog archive missing".to_string()))?;
        let file = staging.path().join(format!("archive-{index}.tar.gz"));
        download(
            Path::new("curl"),
            &archive.url,
            &file,
            (archive.size as f64 * DOWNLOAD_SLACK) as u64,
            cancel,
            |bytes| progress(Step::Downloading { done: done + bytes, total }),
        )?;
        verify_sha256(&file, &archive.sha256)?;
        done += archive.size;
        progress(Step::Extracting);

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
        let stats = extract(decoder, &targets, staging.path(), &Limits::default())?;
        log::info!("unpacked {index}: {stats:?}");
        fs::remove_file(&file)?;

        for theme in themes {
            let staged = staging.path().join(&theme.id);
            if !staged.join("index.theme").is_file() {
                return Err(InstallError::Io(format!("{} has no index.theme", theme.id)));
            }
            write_marker(&staged, &Marker::new(&archive.sha256, catalog_source()))?;
            let target = icons_dir.join(&theme.id);
            if !target.exists() {
                fs::rename(&staged, &target)?;
                log::info!("installed icon theme {} at {}", theme.id, target.display());
            } else if replace.contains(&theme.id) {
                // Two renames, so a lookup never sees a half-written or missing theme.
                let removing = retire(&target)?;
                fs::rename(&staged, &target)?;
                fs::remove_dir_all(&removing)?;
                log::info!("updated icon theme {} at {}", theme.id, target.display());
            } else {
                // Installed meanwhile, by another install that needed the same theme.
                log::info!("{} is already installed, keeping it", theme.id);
            }
        }
    }
    Ok(())
}

/// Moves the theme at `target` out of the way, to a name no lookup will take for a theme.
fn retire(target: &Path) -> io::Result<PathBuf> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("theme");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let removing = target.with_file_name(format!("{REMOVING_PREFIX}{name}-{nonce:x}"));
    fs::rename(target, &removing)?;
    Ok(removing)
}

/// Download `url` to `dest` with the `curl` at `program`, reporting the bytes received so far.
/// Redirects may not leave HTTPS, the transfer stops past `max_size` bytes or once it stalls
/// for a minute, and setting `cancel` kills it.
fn download(
    program: &Path,
    url: &str,
    dest: &Path,
    max_size: u64,
    cancel: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<(), InstallError> {
    let mut child = Command::new(program)
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "5",
            "--max-filesize",
            &max_size.to_string(),
            "--connect-timeout",
            "15",
            "--speed-time",
            "60",
            "--speed-limit",
            "1024",
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
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(InstallError::Cancelled);
        }
        if let Ok(metadata) = fs::metadata(dest) {
            progress(metadata.len());
        }
        thread::sleep(Duration::from_millis(200));
    };
    if status.success() {
        return Ok(());
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    Err(curl_error(status.code(), stderr.trim(), url, max_size))
}

/// The error behind a curl exit code. 6 is an unresolved host, 7 a refused connection, 28 a
/// timeout, 35 a TLS failure, 22 an HTTP error with `--fail`, 63 the size cap.
fn curl_error(code: Option<i32>, stderr: &str, url: &str, max_size: u64) -> InstallError {
    match code {
        Some(6 | 7 | 28 | 35) => InstallError::Network {
            host: host_of(url).to_string(),
            detail: stderr.to_string(),
        },
        Some(22) if stderr.contains("404") || stderr.contains("410") => InstallError::Moved,
        Some(63) => InstallError::TooLarge(max_size),
        _ => InstallError::Io(format!("download failed: {stderr}")),
    }
}

fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), InstallError> {
    let actual = sha256_of(path)?;
    if actual == expected {
        Ok(())
    } else {
        log::warn!("checksum mismatch for {}: expected {expected}, got {actual}", path.display());
        Err(InstallError::Checksum)
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

/// Caps on what one archive may unpack to. A crafted archive past either is refused whole.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: u64,
    pub max_entries: usize,
}

impl Default for Limits {
    /// Papirus, the largest catalog theme, unpacks to about 300 MB and 42 000 entries.
    fn default() -> Self {
        Self {
            max_bytes: 1_500_000_000,
            max_entries: 250_000,
        }
    }
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

/// Where the entries written so far sit, lowercased, so a later entry differing only by case
/// is caught before the file system sees it.
#[derive(Default)]
struct Written {
    files: HashSet<String>,
    dirs: HashSet<String>,
    /// Symlinks; nothing may be written below one.
    links: HashSet<String>,
    /// Directories dropped for a collision, along with everything below them.
    skipped: HashSet<String>,
}

impl Written {
    /// Records a directory at `lower`, or says why it cannot be made.
    fn dir(&mut self, lower: &str) -> Placement {
        if ancestors(lower).any(|ancestor| self.links.contains(ancestor)) {
            return Placement::ThroughLink;
        }
        if ancestors(lower).any(|ancestor| self.skipped.contains(ancestor))
            || self.files.contains(lower)
            || ancestors(lower).any(|ancestor| self.files.contains(ancestor))
        {
            self.skipped.insert(lower.to_string());
            return Placement::Collision;
        }
        self.dirs.insert(lower.to_string());
        Placement::Fresh
    }

    /// Records a file or symlink at `lower`, or says why it cannot be made.
    fn file(&mut self, lower: &str) -> Placement {
        if ancestors(lower).any(|ancestor| self.links.contains(ancestor)) {
            return Placement::ThroughLink;
        }
        if ancestors(lower).any(|ancestor| self.skipped.contains(ancestor))
            || self.files.contains(lower)
            || self.dirs.contains(lower)
            || ancestors(lower).any(|ancestor| self.files.contains(ancestor))
        {
            return Placement::Collision;
        }
        self.files.insert(lower.to_string());
        for ancestor in ancestors(lower) {
            self.dirs.insert(ancestor.to_string());
        }
        Placement::Fresh
    }
}

enum Placement {
    Fresh,
    Collision,
    ThroughLink,
}

/// One entry of an archive or folder, as the extraction sees it.
enum EntryKind {
    Dir,
    File,
    Symlink(PathBuf),
    /// Hard links, devices and the like, which have no place in an icon theme.
    Other,
}

/// Unpacks entries into `dest/<id>` for each target, whatever they are read from.
///
/// Symlinks are kept, since most themes are built on them, but only relative ones that stay
/// inside the icon directory once the theme sits at `<icons>/<id>`; a variant may point into
/// a sibling theme installed beside it. Nothing is written through a symlink.
struct Extractor<'a> {
    targets: &'a [ExtractTarget],
    dest: &'a Path,
    limits: &'a Limits,
    stats: ExtractStats,
    written: Vec<Written>,
    entries: usize,
    bytes: u64,
}

impl<'a> Extractor<'a> {
    fn new(targets: &'a [ExtractTarget], dest: &'a Path, limits: &'a Limits) -> Self {
        Self {
            targets,
            dest,
            limits,
            stats: ExtractStats::default(),
            written: targets.iter().map(|_| Written::default()).collect(),
            entries: 0,
            bytes: 0,
        }
    }

    fn entry(
        &mut self,
        path: &Path,
        kind: EntryKind,
        size: u64,
        reader: &mut dyn Read,
    ) -> Result<(), InstallError> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(InstallError::TooLarge(self.limits.max_bytes));
        }
        let Some(path) = normal_path(path) else {
            self.stats.rejected += 1;
            return Ok(());
        };

        // Where the entry goes: (target, path inside the theme).
        let mut placement = None;
        for (index, target) in self.targets.iter().enumerate() {
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
            return Ok(());
        };

        let lower = relative.to_lowercase();
        let out = self.dest.join(&self.targets[index].id).join(&relative);
        let placement = if matches!(kind, EntryKind::Dir) {
            self.written[index].dir(&lower)
        } else {
            self.written[index].file(&lower)
        };
        match placement {
            Placement::Fresh => {}
            Placement::Collision => {
                self.stats.case_collisions += 1;
                return Ok(());
            }
            Placement::ThroughLink => {
                // Its directory is a symlink; writing it would land outside this theme.
                self.stats.rejected += 1;
                return Ok(());
            }
        }
        match kind {
            EntryKind::Dir => {
                fs::create_dir_all(&out)?;
            }
            EntryKind::File => {
                self.bytes += size;
                if self.bytes > self.limits.max_bytes {
                    return Err(InstallError::TooLarge(self.limits.max_bytes));
                }
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file = File::create(&out)?;
                io::copy(reader, &mut file)?;
                self.stats.files += 1;
            }
            EntryKind::Symlink(link) => {
                if !link_stays_inside(&self.targets[index].id, &relative, &link) {
                    self.stats.rejected += 1;
                    return Ok(());
                }
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent)?;
                }
                symlink(&link, &out)?;
                self.written[index].links.insert(lower);
                self.stats.symlinks += 1;
            }
            EntryKind::Other => {
                self.stats.rejected += 1;
            }
        }
        Ok(())
    }
}

/// Feeds every entry of the tar stream `reader` to `f`.
fn tar_entries(
    reader: impl Read,
    f: &mut dyn FnMut(&Path, EntryKind, u64, &mut dyn Read) -> Result<(), InstallError>,
) -> Result<(), InstallError> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let header = entry.header().entry_type();
        let kind = if header.is_dir() {
            EntryKind::Dir
        } else if header.is_file() {
            EntryKind::File
        } else if header.is_symlink() {
            match entry.link_name()? {
                Some(link) => EntryKind::Symlink(link.into_owned()),
                None => EntryKind::Other,
            }
        } else {
            EntryKind::Other
        };
        let size = entry.size();
        f(&path, kind, size, &mut entry)?;
    }
    Ok(())
}

/// Feeds every entry of the zip archive in `file` to `f`.
fn zip_entries(
    file: File,
    f: &mut dyn FnMut(&Path, EntryKind, u64, &mut dyn Read) -> Result<(), InstallError>,
) -> Result<(), InstallError> {
    let mut archive = zip::ZipArchive::new(file).map_err(|err| InstallError::Io(err.to_string()))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| InstallError::Io(err.to_string()))?;
        let path = PathBuf::from(entry.name());
        let size = entry.size();
        let kind = if entry.is_dir() {
            EntryKind::Dir
        } else if entry.is_symlink() {
            let mut target = Vec::new();
            entry.read_to_end(&mut target)?;
            match String::from_utf8(target) {
                Ok(target) => EntryKind::Symlink(PathBuf::from(target)),
                Err(_) => EntryKind::Other,
            }
        } else {
            EntryKind::File
        };
        f(&path, kind, size, &mut entry)?;
    }
    Ok(())
}

/// Feeds every entry under the folder `dir` to `f`, with the folder's own name as the top
/// path component, the way a tarball of it would read. Symlinks are reported, not followed.
fn dir_entries(
    dir: &Path,
    f: &mut dyn FnMut(&Path, EntryKind, u64, &mut dyn Read) -> Result<(), InstallError>,
) -> Result<(), InstallError> {
    fn walk(
        dir: &Path,
        at: &Path,
        f: &mut dyn FnMut(&Path, EntryKind, u64, &mut dyn Read) -> Result<(), InstallError>,
    ) -> Result<(), InstallError> {
        let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<io::Result<_>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = at.join(entry.file_name());
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_symlink() {
                let link = fs::read_link(entry.path())?;
                f(&path, EntryKind::Symlink(link), 0, &mut io::empty())?;
            } else if metadata.is_dir() {
                f(&path, EntryKind::Dir, 0, &mut io::empty())?;
                walk(&entry.path(), &path, f)?;
            } else if metadata.is_file() {
                let mut file = File::open(entry.path())?;
                f(&path, EntryKind::File, metadata.len(), &mut file)?;
            } else {
                f(&path, EntryKind::Other, 0, &mut io::empty())?;
            }
        }
        Ok(())
    }
    let name = dir.file_name().map(PathBuf::from).unwrap_or_default();
    f(&name, EntryKind::Dir, 0, &mut io::empty())?;
    walk(dir, &name, f)
}

/// Unpack `targets` from the tar stream `reader` into `dest/<id>`.
pub fn extract(
    reader: impl Read,
    targets: &[ExtractTarget],
    dest: &Path,
    limits: &Limits,
) -> Result<ExtractStats, InstallError> {
    let mut extractor = Extractor::new(targets, dest, limits);
    tar_entries(reader, &mut |path, kind, size, reader| {
        extractor.entry(path, kind, size, reader)
    })?;
    Ok(extractor.stats)
}

/// A local archive or folder to install themes from, told apart by its first bytes rather
/// than its name.
#[derive(Debug)]
pub enum Source {
    Tar(PathBuf, Compression),
    Zip(PathBuf),
    Dir(PathBuf),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compression {
    None,
    Gzip,
    Xz,
    Bzip2,
}

impl Source {
    pub fn open(path: &Path) -> Result<Self, InstallError> {
        if path.is_dir() {
            return Ok(Self::Dir(path.to_path_buf()));
        }
        let mut head = [0u8; 262];
        let read = File::open(path)?.read(&mut head)?;
        let head = &head[..read];
        if head.starts_with(&[0x1f, 0x8b]) {
            Ok(Self::Tar(path.to_path_buf(), Compression::Gzip))
        } else if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
            Ok(Self::Tar(path.to_path_buf(), Compression::Xz))
        } else if head.starts_with(b"BZh") {
            Ok(Self::Tar(path.to_path_buf(), Compression::Bzip2))
        } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
            Ok(Self::Zip(path.to_path_buf()))
        } else if head.len() >= 262 && &head[257..262] == b"ustar" {
            Ok(Self::Tar(path.to_path_buf(), Compression::None))
        } else {
            Err(InstallError::Io(format!(
                "{} is not a tar, tar.gz, tar.xz, tar.bz2 or zip archive",
                path.display()
            )))
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Tar(path, _) | Self::Zip(path) | Self::Dir(path) => path,
        }
    }

    /// The file or folder name, as the gallery shows it.
    pub fn name(&self) -> String {
        self.path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn entries(
        &self,
        f: &mut dyn FnMut(&Path, EntryKind, u64, &mut dyn Read) -> Result<(), InstallError>,
    ) -> Result<(), InstallError> {
        match self {
            Self::Tar(path, compression) => {
                let file = File::open(path)?;
                match compression {
                    Compression::None => tar_entries(file, f),
                    Compression::Gzip => tar_entries(flate2::read::GzDecoder::new(file), f),
                    #[cfg(feature = "lzma-rust2")]
                    Compression::Xz => tar_entries(lzma_rust2::XzReader::new(file, true), f),
                    #[cfg(not(feature = "lzma-rust2"))]
                    Compression::Xz => Err(InstallError::Io(
                        "xz archives are not supported by this build".to_string(),
                    )),
                    #[cfg(feature = "bzip2")]
                    Compression::Bzip2 => tar_entries(bzip2::read::BzDecoder::new(file), f),
                    #[cfg(not(feature = "bzip2"))]
                    Compression::Bzip2 => Err(InstallError::Io(
                        "bzip2 archives are not supported by this build".to_string(),
                    )),
                }
            }
            Self::Zip(path) => zip_entries(File::open(path)?, f),
            Self::Dir(path) => dir_entries(path, f),
        }
    }

    /// Unpack `targets` into `dest/<id>`.
    pub fn extract(
        &self,
        targets: &[ExtractTarget],
        dest: &Path,
        limits: &Limits,
    ) -> Result<ExtractStats, InstallError> {
        let mut extractor = Extractor::new(targets, dest, limits);
        self.entries(&mut |path, kind, size, reader| extractor.entry(path, kind, size, reader))?;
        Ok(extractor.stats)
    }

    /// The themes in this source: every directory at most two levels down holding an
    /// `index.theme` and something else below it. The root counts as a directory, for an
    /// archive of one theme; the second level covers a wrapper directory, as GitHub's
    /// tarballs have. Only the shallowest level with a theme is taken, so a theme's own
    /// subdirectories are never mistaken for more themes.
    pub fn detect_themes(&self) -> Result<Vec<ExtractTarget>, InstallError> {
        let mut with_index: Vec<String> = Vec::new();
        let mut with_other: HashSet<String> = HashSet::new();
        let mut count = 0;
        self.entries(&mut |path, kind, _size, _reader| {
            count += 1;
            if count > Limits::default().max_entries {
                return Err(InstallError::TooLarge(Limits::default().max_bytes));
            }
            let Some(path) = normal_path(path) else {
                return Ok(());
            };
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            if name == "index.theme" && matches!(kind, EntryKind::File) {
                if dir.matches('/').count() < 2 {
                    with_index.push(dir.to_string());
                }
            } else {
                // Everything above the entry holds something other than an index.theme.
                with_other.insert(dir.to_string());
                with_other.extend(ancestors(dir).map(str::to_string));
                with_other.insert(String::new());
            }
            Ok(())
        })?;
        with_index.retain(|dir| with_other.contains(dir));
        with_index.sort();
        with_index.dedup();
        let Some(shallowest) = with_index.iter().map(|dir| dir.matches('/').count()).min() else {
            return Ok(Vec::new());
        };
        let shallowest = if with_index.contains(&String::new()) { 0 } else { shallowest + 1 };
        let own_name = self.name();
        Ok(with_index
            .into_iter()
            .filter(|dir| {
                let depth = if dir.is_empty() { 0 } else { dir.matches('/').count() + 1 };
                depth == shallowest
            })
            .map(|dir| {
                let name = dir.rsplit('/').next().filter(|name| !name.is_empty());
                ExtractTarget {
                    id: theme_id(name.unwrap_or(&own_name)),
                    root: dir,
                    index_theme: None,
                }
            })
            .collect())
    }
}

/// The theme id a downloaded archive or wrapper directory stands for: its name without the
/// archive extension and without the `-master`, `-main` or `-<commit>` GitHub appends.
fn theme_id(name: &str) -> String {
    let mut id = name;
    for extension in [".tar.gz", ".tgz", ".tar.xz", ".tar.bz2", ".tar", ".zip"] {
        if let Some(stem) = id.strip_suffix(extension) {
            id = stem;
            break;
        }
    }
    if let Some((stem, suffix)) = id.rsplit_once('-') {
        let is_commit = suffix.len() >= 7 && suffix.chars().all(|c| c.is_ascii_hexdigit());
        if (is_commit || matches!(suffix, "master" | "main")) && !stem.is_empty() {
            id = stem;
        }
    }
    id.to_string()
}

/// The gallery's key for an install from `path`, which has no catalog id.
pub fn file_install_key(path: &Path) -> String {
    format!(
        "file:{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    )
}

/// Install every theme in the archive or folder at `path`, reporting through `emit` under
/// [`file_install_key`]. Blocking: run it on its own thread.
pub fn install_from_path(path: PathBuf, emit: impl Fn(InstallEvent)) {
    let key = file_install_key(&path);
    emit(InstallEvent::Progress(key.clone(), Step::Extracting));
    match install_file_in(&user_icons_dir(), &path) {
        Ok(ids) => emit(InstallEvent::Installed(key, ids)),
        Err(err) => {
            log::warn!("installing icon themes from {} failed: {err}", path.display());
            emit(InstallEvent::Failed(key, err));
        }
    }
}

fn install_file_in(icons_dir: &Path, path: &Path) -> Result<Vec<String>, InstallError> {
    let source = Source::open(path)?;
    let targets = source.detect_themes()?;
    if targets.is_empty() {
        return Err(InstallError::NoTheme);
    }
    // Never overwrite a theme the user put there.
    for target in &targets {
        let dir = icons_dir.join(&target.id);
        if dir.exists() && !is_catalog_install(&dir) {
            return Err(InstallError::Exists(target.id.clone()));
        }
    }
    fs::create_dir_all(icons_dir)?;
    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(icons_dir)?;
    let stats = source.extract(&targets, staging.path(), &Limits::default())?;
    log::info!("unpacked {}: {stats:?}", path.display());
    if stats.rejected > 0 {
        return Err(InstallError::UnsafeArchive(stats.rejected));
    }
    let sha256 = match &source {
        Source::Dir(_) => String::new(),
        _ => sha256_of(path)?,
    };
    let mut ids = Vec::new();
    for target in &targets {
        let staged = staging.path().join(&target.id);
        if !staged.join("index.theme").is_file() {
            return Err(InstallError::NoTheme);
        }
        write_marker(&staged, &Marker::new(&sha256, format!("file:{}", source.name())))?;
        let installed = icons_dir.join(&target.id);
        if installed.exists() {
            let removing = retire(&installed)?;
            fs::rename(&staged, &installed)?;
            fs::remove_dir_all(&removing)?;
        } else {
            fs::rename(&staged, &installed)?;
        }
        log::info!("installed icon theme {} from {}", target.id, path.display());
        ids.push(target.id.clone());
    }
    Ok(ids)
}

fn sha256_of(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
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

/// Remove an installed theme, if this module installed it. The directory is moved aside
/// first, so a lookup running at the same time never sees it half deleted.
pub fn remove(id: &str) -> io::Result<()> {
    remove_in(&user_icons_dir(), id)
}

fn remove_in(icons_dir: &Path, id: &str) -> io::Result<()> {
    let dir = icons_dir.join(id);
    if !dir.join(MARKER).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{id} was not installed from the catalog"),
        ));
    }
    let removing = retire(&dir)?;
    fs::remove_dir_all(&removing)
}

/// Remove staging and removal directories under `icons_dir` that a crash left behind. Only
/// those older than a day go, so another window's install in progress is left alone.
pub fn cleanup_staging(icons_dir: &Path) {
    let Ok(entries) = fs::read_dir(icons_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(STAGING_PREFIX) && !name.starts_with(REMOVING_PREFIX) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if stale {
            log::info!("removing abandoned {}", entry.path().display());
            if let Err(err) = fs::remove_dir_all(entry.path()) {
                log::warn!("failed to remove {}: {err}", entry.path().display());
            }
        }
    }
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

    fn add_dir(builder: &mut tar::Builder<Vec<u8>>, path: &str) {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        builder.append_data(&mut header, path, &[][..]).unwrap();
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

    fn limits() -> Limits {
        Limits::default()
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
        let stats = extract(
            &data[..],
            &[target("pkg-1.0/Theme", "Theme")],
            dest.path(),
            &limits(),
        )
        .unwrap();
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
        extract(&root[..], &[target("repo-1", "Root")], dest.path(), &limits()).unwrap();
        assert!(dest.path().join("Root/index.theme").is_file());
        assert!(dest.path().join("Root/places/folder.svg").is_file());

        let adwaita = ExtractTarget {
            root: "adwaita-51/Adwaita".to_string(),
            index_theme: Some("adwaita-51/index.theme".to_string()),
            id: "Adwaita".to_string(),
        };
        extract(&root[..], &[adwaita], dest.path(), &limits()).unwrap();
        assert_eq!(
            fs::read(dest.path().join("Adwaita/index.theme")).unwrap(),
            b"[Icon Theme]\nName=Adwaita\n"
        );
        assert!(dest.path().join("Adwaita/scalable/folder.svg").is_file());
    }

    #[test]
    fn extract_drops_a_directory_that_collides_with_a_file_by_case() {
        let mut builder = tar::Builder::new(Vec::new());
        add_file(&mut builder, "T/index.theme", b"[Icon Theme]\nName=T\n");
        add_file(&mut builder, "T/places/Foo", b"file");
        add_dir(&mut builder, "T/places/foo");
        add_file(&mut builder, "T/places/foo/inside.svg", b"<svg/>");
        // The other way round: a directory already there, then a file of the same name.
        add_file(&mut builder, "T/apps/bar/x.svg", b"<svg/>");
        add_file(&mut builder, "T/apps/Bar", b"file");
        add_file(&mut builder, "T/apps/kept.svg", b"<svg/>");
        let data = builder.into_inner().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let stats = extract(&data[..], &[target("T", "T")], dest.path(), &limits()).unwrap();
        let theme = dest.path().join("T");
        assert_eq!(fs::read(theme.join("places/Foo")).unwrap(), b"file");
        assert!(theme.join("apps/bar/x.svg").is_file());
        assert!(theme.join("apps/kept.svg").is_file());
        assert_eq!(
            stats,
            ExtractStats {
                files: 4,
                symlinks: 0,
                case_collisions: 3,
                rejected: 0,
            }
        );
    }

    #[test]
    fn extract_refuses_too_many_entries_or_bytes() {
        let mut builder = tar::Builder::new(Vec::new());
        for n in 0..4 {
            add_file(&mut builder, &format!("T/{n}.svg"), b"");
        }
        let data = builder.into_inner().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let few = Limits {
            max_bytes: 1_000,
            max_entries: 3,
        };
        assert_eq!(
            extract(&data[..], &[target("T", "T")], dest.path(), &few).unwrap_err(),
            InstallError::TooLarge(1_000)
        );

        let mut builder = tar::Builder::new(Vec::new());
        add_file(&mut builder, "T/big.svg", &[0; 2_000]);
        let data = builder.into_inner().unwrap();
        assert_eq!(
            extract(&data[..], &[target("T", "T")], dest.path(), &few).unwrap_err(),
            InstallError::TooLarge(1_000)
        );
        assert!(extract(&data[..], &[target("T", "T")], dest.path(), &limits()).is_ok());
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

        let installed = HashSet::from(["Papirus".to_string(), "Papirus-Dark".to_string()]);
        assert!(install_plan("Papirus-Dark", &installed).is_empty());
        let plan: Vec<_> = update_plan("Papirus-Dark", &installed)
            .iter()
            .map(|theme| theme.id.as_str())
            .collect();
        assert_eq!(plan, ["Papirus-Dark"]);
    }

    /// A stand-in for curl that exits with `code`.
    #[cfg(unix)]
    fn fake_curl(dir: &Path, code: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let program = dir.join("curl");
        fs::write(&program, format!("#!/bin/sh\necho 'curl: ({code}) fake' >&2\nexit {code}\n"))
            .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    #[cfg(unix)]
    #[test]
    fn download_maps_curl_exit_codes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        let cancel = AtomicBool::new(false);
        let url = "https://codeload.github.com/x/y/tar.gz/z";
        let run = |code| {
            download(&fake_curl(dir.path(), code), url, &dest, 10, &cancel, |_| {}).unwrap_err()
        };
        assert_eq!(
            run(6),
            InstallError::Network {
                host: "codeload.github.com".to_string(),
                detail: "curl: (6) fake".to_string(),
            }
        );
        assert_eq!(run(63), InstallError::TooLarge(10));
        assert_eq!(run(2), InstallError::Io("download failed: curl: (2) fake".to_string()));
        assert_eq!(
            curl_error(Some(22), "curl: (22) The requested URL returned error: 404", url, 10),
            InstallError::Moved
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_is_killed_when_cancelled() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("curl");
        fs::write(&program, "#!/bin/sh\nsleep 30\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = download(&program, "https://x", &dir.path().join("out"), 10, &cancel, |_| {});
        assert_eq!(result.unwrap_err(), InstallError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn markers_round_trip_and_old_ones_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let theme = dir.path().join("T");
        fs::create_dir(&theme).unwrap();
        fs::write(theme.join(MARKER), "abc123\n").unwrap();
        let old = read_marker(&theme).unwrap();
        assert_eq!(old.sha256, "abc123");
        assert_eq!(old.source, "catalog");
        assert_eq!(old.installed_date(), None);
        assert!(is_catalog_install(&theme));

        write_marker(&theme, &Marker::new("def456", "file:Sweet.tar.gz".to_string())).unwrap();
        let new = read_marker(&theme).unwrap();
        assert_eq!(new.sha256, "def456");
        assert_eq!(new.file(), Some("Sweet.tar.gz"));
        assert!(new.installed_date().is_some());
        assert!(read_marker(dir.path()).is_none());
    }

    #[test]
    fn needs_update_when_the_marker_names_an_older_archive() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = catalog();
        let entry = &catalog.themes[0];
        let root = dir.path().join(&entry.id);
        fs::create_dir(&root).unwrap();
        let info = |roots: Vec<PathBuf>| IconThemeInfo {
            id: entry.id.clone(),
            name: entry.name.clone(),
            inherits: Vec::new(),
            roots,
            directories: Vec::new(),
        };
        // Installed by hand: no marker, never updated.
        assert!(!needs_update(&info(vec![root.clone()])));
        fs::write(root.join(MARKER), "stale\n").unwrap();
        assert!(needs_update(&info(vec![root.clone()])));
        let current = &catalog.archives[entry.archive].sha256;
        write_marker(&root, &Marker::new(current, catalog_source())).unwrap();
        assert!(!needs_update(&info(vec![root.clone()])));
        // From a file: no catalog archive to compare with.
        write_marker(&root, &Marker::new("other", "file:x.zip".to_string())).unwrap();
        assert!(!needs_update(&info(vec![root])));
    }

    #[test]
    fn remove_moves_the_theme_aside_and_refuses_hand_installed_ones() {
        let icons = tempfile::tempdir().unwrap();
        let ours = icons.path().join("Ours");
        fs::create_dir(&ours).unwrap();
        fs::write(ours.join(MARKER), "abc\n").unwrap();
        fs::write(ours.join("index.theme"), "").unwrap();
        let theirs = icons.path().join("Theirs");
        fs::create_dir(&theirs).unwrap();

        remove_in(icons.path(), "Ours").unwrap();
        assert!(!ours.exists());
        assert_eq!(fs::read_dir(icons.path()).unwrap().count(), 1);
        assert_eq!(
            remove_in(icons.path(), "Theirs").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(theirs.is_dir());
    }

    /// A tarball of two themes under a GitHub-style wrapper directory.
    fn sweet_tar() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        add_dir(&mut builder, "Sweet-folders-40a5d36/");
        add_file(&mut builder, "Sweet-folders-40a5d36/README.md", b"readme");
        for theme in ["Sweet-Blue", "Sweet-Teal"] {
            add_dir(&mut builder, &format!("Sweet-folders-40a5d36/{theme}/"));
            add_file(
                &mut builder,
                &format!("Sweet-folders-40a5d36/{theme}/index.theme"),
                format!("[Icon Theme]\nName={theme}\nDirectories=places\n\n[places]\nSize=48\n")
                    .as_bytes(),
            );
            add_file(
                &mut builder,
                &format!("Sweet-folders-40a5d36/{theme}/places/folder.svg"),
                b"<svg/>",
            );
            add_link(
                &mut builder,
                &format!("Sweet-folders-40a5d36/{theme}/places/inode-directory.svg"),
                "folder.svg",
            );
        }
        builder.into_inner().unwrap()
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn install_marker(icons: &Path, id: &str) -> Marker {
        read_marker(&icons.join(id)).unwrap()
    }

    #[test]
    fn detect_themes_finds_the_shallowest_index_theme_directories() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("Sweet-folders-master.tar.gz");
        fs::write(&archive, gzip(&sweet_tar())).unwrap();
        let source = Source::open(&archive).unwrap();
        assert!(matches!(source, Source::Tar(_, Compression::Gzip)));
        let ids: Vec<(String, String)> = source
            .detect_themes()
            .unwrap()
            .into_iter()
            .map(|target| (target.root, target.id))
            .collect();
        assert_eq!(
            ids,
            [
                ("Sweet-folders-40a5d36/Sweet-Blue".to_string(), "Sweet-Blue".to_string()),
                ("Sweet-folders-40a5d36/Sweet-Teal".to_string(), "Sweet-Teal".to_string()),
            ]
        );

        // One theme at the root of a wrapper directory: the wrapper is the theme.
        let mut builder = tar::Builder::new(Vec::new());
        add_file(&mut builder, "candy-icons-83512fbcadc/index.theme", b"[Icon Theme]\n");
        add_file(&mut builder, "candy-icons-83512fbcadc/places/folder.svg", b"<svg/>");
        let plain = dir.path().join("candy.tar");
        fs::write(&plain, builder.into_inner().unwrap()).unwrap();
        let source = Source::open(&plain).unwrap();
        assert!(matches!(source, Source::Tar(_, Compression::None)));
        let targets = source.detect_themes().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].root, "candy-icons-83512fbcadc");
        assert_eq!(targets[0].id, "candy-icons");

        // Nothing but an index.theme is not a theme.
        let mut builder = tar::Builder::new(Vec::new());
        add_file(&mut builder, "index.theme", b"[Icon Theme]\n");
        let empty = dir.path().join("empty.tar");
        fs::write(&empty, builder.into_inner().unwrap()).unwrap();
        assert!(Source::open(&empty).unwrap().detect_themes().unwrap().is_empty());

        assert_eq!(theme_id("MoreWaita-main.zip"), "MoreWaita");
        assert_eq!(theme_id("Papirus-Dark"), "Papirus-Dark");
        assert_eq!(theme_id("Tela-circle-dark"), "Tela-circle-dark");
    }

    #[test]
    fn install_from_a_tarball_a_zip_and_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let icons = tempfile::tempdir().unwrap();

        let tarball = dir.path().join("Sweet-folders.tar.gz");
        fs::write(&tarball, gzip(&sweet_tar())).unwrap();
        assert_eq!(
            install_file_in(icons.path(), &tarball).unwrap(),
            ["Sweet-Blue", "Sweet-Teal"]
        );
        assert!(icons.path().join("Sweet-Blue/places/folder.svg").is_file());
        assert_eq!(
            fs::read_link(icons.path().join("Sweet-Teal/places/inode-directory.svg")).unwrap(),
            Path::new("folder.svg")
        );
        let marker = install_marker(icons.path(), "Sweet-Blue");
        assert_eq!(marker.file(), Some("Sweet-folders.tar.gz"));
        assert_eq!(marker.sha256, sha256_of(&tarball).unwrap());

        // The same themes as a zip replace the copies the app installed.
        let zipped = dir.path().join("Sweet-folders.zip");
        {
            use std::io::Write;
            let mut writer = zip::ZipWriter::new(File::create(&zipped).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            writer.add_directory("Sweet-Blue/", options).unwrap();
            writer.start_file("Sweet-Blue/index.theme", options).unwrap();
            writer.write_all(b"[Icon Theme]\nName=Sweet-Blue\n").unwrap();
            writer.start_file("Sweet-Blue/places/folder.svg", options).unwrap();
            writer.write_all(b"<svg>zip</svg>").unwrap();
            writer
                .add_symlink("Sweet-Blue/places/inode-directory.svg", "folder.svg", options)
                .unwrap();
            writer.finish().unwrap();
        }
        assert_eq!(install_file_in(icons.path(), &zipped).unwrap(), ["Sweet-Blue"]);
        assert_eq!(
            fs::read(icons.path().join("Sweet-Blue/places/folder.svg")).unwrap(),
            b"<svg>zip</svg>"
        );
        assert_eq!(
            fs::read_link(icons.path().join("Sweet-Blue/places/inode-directory.svg")).unwrap(),
            Path::new("folder.svg")
        );
        assert_eq!(
            install_marker(icons.path(), "Sweet-Blue").file(),
            Some("Sweet-folders.zip")
        );
        assert!(icons.path().join("Sweet-Teal/places/folder.svg").is_file());
        assert_eq!(fs::read_dir(icons.path()).unwrap().count(), 2);

        // An unpacked folder, dropped as is.
        let folder = dir.path().join("Sweet-Purple");
        fs::create_dir_all(folder.join("places")).unwrap();
        fs::write(folder.join("index.theme"), "[Icon Theme]\nName=Sweet-Purple\n").unwrap();
        fs::write(folder.join("places/folder.svg"), "<svg/>").unwrap();
        symlink(Path::new("folder.svg"), &folder.join("places/inode-directory.svg")).unwrap();
        assert_eq!(install_file_in(icons.path(), &folder).unwrap(), ["Sweet-Purple"]);
        assert!(icons.path().join("Sweet-Purple/places/folder.svg").is_file());
        assert_eq!(
            fs::read_link(icons.path().join("Sweet-Purple/places/inode-directory.svg")).unwrap(),
            Path::new("folder.svg")
        );
        let marker = install_marker(icons.path(), "Sweet-Purple");
        assert_eq!(marker.file(), Some("Sweet-Purple"));
        assert_eq!(marker.sha256, "");

        // A theme the user put there by hand is never replaced.
        let theirs = icons.path().join("Sweet-Teal");
        fs::remove_file(theirs.join(MARKER)).unwrap();
        assert_eq!(
            install_file_in(icons.path(), &tarball).unwrap_err(),
            InstallError::Exists("Sweet-Teal".to_string())
        );
        assert_eq!(fs::read_dir(icons.path()).unwrap().count(), 3);
    }

    #[test]
    fn install_from_file_refuses_unsafe_and_themeless_archives() {
        let dir = tempfile::tempdir().unwrap();
        let icons = tempfile::tempdir().unwrap();

        let zipped = dir.path().join("evil.zip");
        {
            use std::io::Write;
            let mut writer = zip::ZipWriter::new(File::create(&zipped).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("Evil/index.theme", options).unwrap();
            writer.write_all(b"[Icon Theme]\n").unwrap();
            writer.start_file("Evil/../../escaped.svg", options).unwrap();
            writer.write_all(b"<svg/>").unwrap();
            writer.add_symlink("Evil/etc", "/etc", options).unwrap();
            writer.finish().unwrap();
        }
        assert_eq!(
            install_file_in(icons.path(), &zipped).unwrap_err(),
            InstallError::UnsafeArchive(2)
        );
        assert!(!icons.path().join("Evil").exists());
        assert!(!dir.path().join("escaped.svg").exists());
        assert_eq!(fs::read_dir(icons.path()).unwrap().count(), 0);

        let mut builder = tar::Builder::new(Vec::new());
        add_file(&mut builder, "notes/README.md", b"no theme here");
        let themeless = dir.path().join("notes.tar.gz");
        fs::write(&themeless, gzip(&builder.into_inner().unwrap())).unwrap();
        assert_eq!(
            install_file_in(icons.path(), &themeless).unwrap_err(),
            InstallError::NoTheme
        );

        let text = dir.path().join("notes.txt");
        fs::write(&text, "hello").unwrap();
        assert!(matches!(
            install_file_in(icons.path(), &text).unwrap_err(),
            InstallError::Io(_)
        ));
    }

    #[test]
    fn cleanup_removes_only_stale_temporaries() {
        let icons = tempfile::tempdir().unwrap();
        let stale = icons.path().join(format!("{STAGING_PREFIX}old"));
        let fresh = icons.path().join(format!("{REMOVING_PREFIX}new"));
        let theme = icons.path().join("Theme");
        for dir in [&stale, &fresh, &theme] {
            fs::create_dir(dir).unwrap();
        }
        let two_days_ago = SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60);
        File::open(&stale)
            .unwrap()
            .set_modified(two_days_ago)
            .unwrap();
        File::open(&theme)
            .unwrap()
            .set_modified(two_days_ago)
            .unwrap();

        cleanup_staging(icons.path());
        assert!(!stale.exists());
        assert!(fresh.is_dir());
        assert!(theme.is_dir());
    }
}
