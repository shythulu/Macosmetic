// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use bstr::{BString, ByteSlice, ByteVec};
use cosmic::widget;
pub use mime_guess::Mime;
#[cfg(feature = "desktop")]
use notify_debouncer_full::notify;
use rustc_hash::{FxHashMap, FxHashSet};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock, atomic};
use std::time::{self, Instant};
use std::{fs, io, process};

#[cfg(feature = "desktop")]
pub async fn watch(mut emitter: impl FnMut() + 'static + Send) {
    let watcher_result = notify_debouncer_full::new_debouncer(
        time::Duration::from_millis(250),
        Some(time::Duration::from_millis(250)),
        move |event_res: notify_debouncer_full::DebounceEventResult| {
            let Ok(events) = event_res else {
                return;
            };

            if events.iter().any(|event| {
                event.kind.is_create() || event.kind.is_modify() || event.kind.is_remove()
            }) {
                emitter();
            }
        },
    );

    if let Ok(mut watcher) = watcher_result {
        let system_paths = cosmic_mime_apps::list_paths();
        let local_paths = (|| {
            let base_dirs = xdg::BaseDirectories::new();
            let Some(home) = base_dirs.get_config_home() else {
                return Err(std::io::Error::other("XDG config home not set"));
            };

            let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") else {
                return Err(std::io::Error::other("XDG_CURRENT_DESKTOP unset"));
            };

            let default_mimeapps = home.join("mimeapps.list");
            let desktop_mimeapps =
                home.join([&desktop.to_ascii_lowercase(), "-mimeapps.list"].concat());

            Ok([desktop_mimeapps, default_mimeapps])
        })()
        .ok();

        for path in system_paths
            .iter()
            .chain(local_paths.as_ref().into_iter().flatten())
        {
            _ = watcher.watch(path.as_path(), notify::RecursiveMode::NonRecursive);
        }

        std::future::pending().await
    }
}

pub fn exec_to_command(
    exec: &str,
    entry_name: &str,
    entry_path: Option<&Path>,
    path_opt: &[impl AsRef<OsStr>],
) -> Option<Vec<process::Command>> {
    let arguments = shlex::split(exec)?;

    if arguments.is_empty() {
        tracing::error!("command does not contain any arguments");
        return None;
    }

    let mut commands = Vec::new();

    let paths = path_opt
        .iter()
        .map(AsRef::as_ref)
        .map(Some)
        // Add a single `None` if no path was given.
        .chain(std::iter::repeat_n(
            None,
            if path_opt.is_empty() { 1 } else { 0 },
        ));

    for path in paths {
        let mut batch_process = false;
        let mut args = Vec::with_capacity(arguments.len());
        let mut field_code_used = false;

        for argument in arguments.iter().skip(1) {
            let mut new_argument = BString::new(Vec::with_capacity(argument.capacity()));
            let mut chars = argument.chars();
            while let Some(char) = chars.next() {
                // https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html
                if char == '%' {
                    match chars.next() {
                        Some('%') => new_argument.push_char(char),
                        Some('c') => new_argument.push_str(entry_name),
                        Some('k') => {
                            if let Some(path) = entry_path {
                                new_argument.push_str(path.as_os_str().as_bytes());
                            }
                        }

                        // %f and %u behave the same in a file manager.
                        Some('f' | 'u') => {
                            if let Some(path) = path
                                && !field_code_used
                            {
                                // TODO: files on remote file systems should be copied to a temporary local file.
                                batch_process = true;
                                field_code_used = true;
                                new_argument.push_str(path.as_bytes());
                            }
                        }

                        // %F and %U behave the same in a file manager.
                        Some('F') | Some('U') => {
                            if !field_code_used && new_argument.is_empty() {
                                field_code_used = true;
                                for path in path_opt.iter().map(AsRef::as_ref) {
                                    args.push(BString::new(path.as_bytes().to_owned()));
                                }
                            }
                        }

                        _ => (),
                    }
                } else {
                    new_argument.push_char(char);
                }
            }

            if !new_argument.is_empty() {
                args.push(new_argument);
            }
        }

        let mut command = process::Command::new(&arguments[0]);

        for arg in args {
            match arg.to_os_str() {
                Ok(arg) => {
                    command.arg(arg);
                }
                Err(_) => {
                    tracing::error!("invalid string encoding in command");
                    return None;
                }
            }
        }

        commands.push(command);

        if !batch_process {
            break;
        }
    }

    #[cfg(debug_assertions)]
    for command in &commands {
        log::debug!(
            "Parsed program {} with args: {:?}",
            command.get_program().to_string_lossy(),
            command.get_args()
        );
    }

    Some(commands)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MimeAppMatch {
    Exact,
    Related,
    Other,
}

#[derive(Clone, Debug)]
pub struct MimeApp {
    pub id: String,
    pub path: Option<PathBuf>,
    pub name: String,
    pub exec: Option<String>,
    icon_name: Box<str>,
    icon: std::sync::OnceLock<widget::icon::Handle>,
    is_default: Arc<RwLock<FxHashSet<Box<str>>>>,
    no_display: Arc<AtomicBool>,
}

impl MimeApp {
    //TODO: move to libcosmic, support multiple files
    pub fn command<O: AsRef<OsStr>>(&self, path_opt: &[O]) -> Option<Vec<process::Command>> {
        exec_to_command(
            self.exec.as_deref()?,
            &self.name,
            self.path.as_deref(),
            path_opt,
        )
    }

    pub fn is_default(&self, mime: &Mime) -> bool {
        self.is_default.read().unwrap().contains(mime.essence_str())
    }

    pub fn no_display(&self) -> bool {
        self.no_display.load(atomic::Ordering::Relaxed)
    }

    pub fn icon(&self) -> widget::icon::Handle {
        self.icon
            .get_or_init(|| {
                let name = &*self.icon_name;
                // A macOS entry names its bundle; the icon is rendered from it on first use.
                #[cfg(target_os = "macos")]
                if name.ends_with(".app") {
                    return match crate::workspace_macos::cached_icon(Path::new(name)) {
                        Some(png) => cosmic::widget::icon::from_path(png),
                        None => cosmic::widget::icon::from_name("application-x-executable")
                            .size(32)
                            .handle(),
                    };
                }
                if name.starts_with('/') {
                    cosmic::widget::icon::from_path(PathBuf::from(name))
                } else {
                    cosmic::widget::icon::from_name(name).size(32).handle()
                }
            })
            .clone()
    }
}

// This allows usage of MimeApp in a dropdown
impl AsRef<str> for MimeApp {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

pub struct MimeAppCache {
    apps: Vec<Arc<MimeApp>>,
    #[cfg(not(all(not(feature = "desktop"), target_os = "macos")))]
    cache: FxHashMap<Mime, Vec<Arc<MimeApp>>>,
    terminals: Vec<Arc<MimeApp>>,
    /// LaunchServices answers, filled one MIME type at a time as they are asked for.
    #[cfg(all(not(feature = "desktop"), target_os = "macos"))]
    launch_services: std::sync::Mutex<LaunchServicesCache>,
}

/// The macOS side of [`MimeAppCache`]. LaunchServices cannot list every type it knows, so
/// the per-MIME lists are looked up lazily from [`MimeAppCache::get`], which only has
/// `&self`. The lists are boxed slices that are never dropped or replaced except through
/// `&mut self`, which is what lets `get` hand out references into them.
#[cfg(all(not(feature = "desktop"), target_os = "macos"))]
#[derive(Default)]
struct LaunchServicesCache {
    by_mime: FxHashMap<Mime, Box<[Arc<MimeApp>]>>,
    /// One entry per bundle, so the default marks on an app are shared across MIME types.
    by_path: FxHashMap<PathBuf, Arc<MimeApp>>,
}

#[cfg(all(not(feature = "desktop"), target_os = "macos"))]
impl LaunchServicesCache {
    fn app(&mut self, bundle: &Path) -> Arc<MimeApp> {
        if let Some(app) = self.by_path.get(bundle) {
            return app.clone();
        }
        let app = Arc::new(macos_mime_app(bundle, "%F"));
        self.by_path.insert(bundle.to_path_buf(), app.clone());
        app
    }

    /// Ask LaunchServices for `mime` and cache the answer; the default handler comes first.
    fn lookup(&mut self, mime: &Mime) -> Box<[Arc<MimeApp>]> {
        use crate::workspace_macos;

        let Some(content_type) = workspace_macos::content_type_for_mime(mime) else {
            tracing::debug!(target: "mime-apps", mime = mime.essence_str(), "no UTType");
            return Box::default();
        };
        let (bundles, default) = workspace_macos::apps_for_content_type(&content_type);
        let mut apps: Vec<Arc<MimeApp>> = bundles.iter().map(|bundle| self.app(bundle)).collect();
        if let Some(default) = default
            && let Some(position) = bundles.iter().position(|bundle| *bundle == default)
        {
            let app = apps.remove(position);
            app.is_default
                .write()
                .unwrap()
                .insert(mime.essence_str().into());
            apps.insert(0, app);
        }
        tracing::debug!(target: "mime-apps", mime = mime.essence_str(), r#type = %content_type.identifier(), apps = ?(apps.iter().map(|app| &*app.id).collect::<Vec<&str>>()), "LaunchServices handlers");
        apps.into_boxed_slice()
    }
}

/// A [`MimeApp`] for an application bundle. Its exec goes through `/usr/bin/open`, so the
/// launch path in `app.rs` needs no macOS branch; `files` is the field code the paths go
/// into, `%F` for documents and `.` for a terminal started in the current directory.
#[cfg(all(not(feature = "desktop"), target_os = "macos"))]
fn macos_mime_app(bundle: &Path, files: &str) -> MimeApp {
    let path = bundle.to_string_lossy();
    let quoted = shlex::try_quote(&path).map_or_else(|_| path.to_string(), |q| q.into_owned());
    MimeApp {
        id: path.to_string(),
        path: Some(bundle.to_path_buf()),
        name: crate::workspace_macos::display_name(bundle),
        exec: Some(format!("/usr/bin/open -a {quoted} {files}")),
        icon_name: path.into(),
        icon: std::sync::OnceLock::new(),
        is_default: Arc::new(RwLock::default()),
        no_display: Arc::new(AtomicBool::new(false)),
    }
}

impl MimeAppCache {
    pub fn new() -> Self {
        let mut mime_app_cache = Self {
            apps: Vec::new(),
            #[cfg(not(all(not(feature = "desktop"), target_os = "macos")))]
            cache: FxHashMap::default(),
            terminals: Vec::new(),
            #[cfg(all(not(feature = "desktop"), target_os = "macos"))]
            launch_services: std::sync::Mutex::default(),
        };
        mime_app_cache.reload();
        mime_app_cache
    }

    pub fn get_apps_for_mime(
        &self,
        mime_type: &Mime,
        include_other: bool,
    ) -> Vec<(&Arc<MimeApp>, MimeAppMatch)> {
        let mut results = Vec::new();
        let mut dedupe = FxHashSet::default();

        // start with exact matches
        results.extend(
            self.get(mime_type)
                .iter()
                .filter(|&mime_app| dedupe.insert(&mime_app.id))
                .map(|mime_app| (mime_app, MimeAppMatch::Exact)),
        );

        let include_mime = match mime_type.type_().as_str() {
            "audio" => Some("video/mp4".parse::<Mime>().expect("video/mp4 mime")),
            "text" => Some(mime_guess::mime::TEXT_PLAIN),
            _ => None,
        };

        if let Some(mime) = include_mime {
            results.extend(
                self.get(&mime)
                    .iter()
                    .filter(|&mime_app| dedupe.insert(&mime_app.id))
                    .map(|mime_app| (mime_app, MimeAppMatch::Exact)),
            );
        }

        // grab matches based off of subclass / parent mime type
        if let Some(parent_types) = crate::mime_icon::parent_mime_types(mime_type) {
            for parent_type in parent_types {
                results.extend(
                    self.get(&parent_type)
                        .iter()
                        .filter(|&mime_app| dedupe.insert(&mime_app.id))
                        .map(|mime_app| (mime_app, MimeAppMatch::Related)),
                );
            }
        }

        if include_other {
            results.extend({
                let mut apps = self
                    .apps()
                    .iter()
                    .filter(|mime_app| !mime_app.no_display())
                    .filter(|&mime_app| dedupe.insert(&mime_app.id))
                    .map(|mime_app| (mime_app, MimeAppMatch::Other))
                    .collect::<Vec<_>>();
                apps.sort_by(|(a, _), (b, _)| {
                    crate::localize::LANGUAGE_SORTER.compare(&a.name, &b.name)
                });
                apps
            });
        }

        results
    }

    #[cfg(all(not(feature = "desktop"), not(target_os = "macos")))]
    pub fn reload(&mut self) {}

    /// Forget the LaunchServices answers and list the installed applications again. The
    /// per-type lists refill on demand; see [`LaunchServicesCache`].
    #[cfg(all(not(feature = "desktop"), target_os = "macos"))]
    pub fn reload(&mut self) {
        use crate::workspace_macos;

        let start = Instant::now();

        self.apps.clear();
        self.terminals.clear();
        let launch_services = self.launch_services.get_mut().unwrap();
        *launch_services = LaunchServicesCache::default();

        // Everything in the application folders, for the "Other apps" section of Open With.
        for bundle in workspace_macos::installed_apps() {
            self.apps.push(launch_services.app(&bundle));
        }
        self.apps
            .sort_by(|a, b| crate::localize::LANGUAGE_SORTER.compare(&a.name, &b.name));

        // Terminal.app, started in the directory `app.rs` sets as the command's cwd.
        let terminal = workspace_macos::app_for_bundle_id("com.apple.Terminal").or_else(|| {
            let path = PathBuf::from("/System/Applications/Utilities/Terminal.app");
            path.is_dir().then_some(path)
        });
        if let Some(bundle) = terminal {
            self.terminals.push(Arc::new(macos_mime_app(&bundle, ".")));
        }

        let elapsed = start.elapsed();
        tracing::info!(target: "mime-apps", apps = self.apps.len(), "listed installed applications in {elapsed:?}");
    }

    /// Reload mime types and their known app associations and defaults.
    #[cfg(feature = "desktop")]
    pub fn reload(&mut self) {
        use crate::localize::LANGUAGE_SORTER;
        use crate::mime_icon;
        use cosmic::desktop::fde;
        use std::borrow::Cow;

        let start = Instant::now();

        self.apps.clear();
        self.cache.clear();
        self.terminals.clear();

        let mut list = cosmic_mime_apps::List::default();
        let paths = cosmic_mime_apps::list_paths();
        list.load_from_paths(&paths);
        let locales = fde::get_languages_from_env();
        let desktop_entries = fde::Iter::new(fde::default_paths()).entries(Some(&locales));
        let mime_icon_cache = mime_icon::MIME_ICON_CACHE.lock().unwrap();
        let shared_mime_info = &mime_icon_cache.shared_mime_info;
        let mut aliased_mimes = FxHashMap::default();

        for desktop_entry in desktop_entries {
            let name = desktop_entry
                .name(&locales)
                .unwrap_or_else(|| Cow::Borrowed(desktop_entry.id()));

            let app = Arc::new(MimeApp {
                id: desktop_entry.appid.clone(),
                path: Some(desktop_entry.path.clone()),
                name: name.into(),
                exec: desktop_entry.exec().map(String::from),
                icon_name: desktop_entry.icon().unwrap_or_default().into(),
                icon: std::sync::OnceLock::new(),
                is_default: Arc::new(RwLock::default()),
                no_display: Arc::new(AtomicBool::new(false)),
            });

            tracing::info!(target: "mime-apps", id = app.id, "detected desktop entry");

            self.apps.push(app.clone());

            if desktop_entry
                .categories()
                .into_iter()
                .flatten()
                .any(|c| c == "TerminalEmulator")
            {
                self.terminals.push(app.clone());
            }

            // Cache associations defined by the desktop entry.
            let mime_types = desktop_entry.mime_type().unwrap_or_else(Vec::new);
            let associated_mime_types = mime_types.iter().filter_map(|m| {
                m.parse::<Mime>().ok().map(|mime| {
                    if let Some(unaliased) = shared_mime_info.unalias_mime_type(&mime) {
                        aliased_mimes.insert(unaliased.clone(), mime);
                        return unaliased;
                    }

                    mime
                })
            });

            for mime in associated_mime_types {
                let apps = self.cache.entry(mime.clone()).or_default();
                if apps.iter().all(|cached_app| cached_app.id != app.id) {
                    apps.push(app.clone());
                }
            }
        }

        // Cache added associations from mimeapps lists.
        for (mut added_mime, added_apps) in &list.added_associations {
            let _unaliased;
            if let Some(unaliased) = shared_mime_info.unalias_mime_type(added_mime) {
                aliased_mimes.insert(unaliased.clone(), added_mime.clone());
                _unaliased = unaliased;
                added_mime = &_unaliased;
            }

            for added_app in added_apps {
                if let Some(app) = self
                    .apps
                    .iter()
                    .find(|cached| cached.id.as_str() == added_app.as_ref())
                {
                    let apps = self.cache.entry(added_mime.clone()).or_default();
                    if apps.iter().all(|cached_app| cached_app.id != app.id) {
                        apps.push(app.clone());
                    }
                }
            }
        }

        // Remove associations
        for (mut removed_mime, removed_apps) in &list.removed_associations {
            let _unaliased;
            if let Some(unaliased) = shared_mime_info.unalias_mime_type(removed_mime) {
                aliased_mimes.insert(unaliased.clone(), removed_mime.clone());
                _unaliased = unaliased;
                removed_mime = &_unaliased;
            }

            for removed_app in removed_apps {
                if let Some(app) = self
                    .apps
                    .iter()
                    .find(|cached| cached.id.as_str() == removed_app.as_ref())
                    && let Some(apps) = self.cache.get_mut(removed_mime)
                {
                    apps.retain(|cached_app| cached_app.id != app.id);
                }
            }
        }

        // Fetch defaults and sort apps by their default precedence.
        for (mime, mut apps) in std::mem::take(&mut self.cache).into_iter() {
            let defaults = list.default_app_for(&mime);
            let aliased_defaults = aliased_mimes
                .get(&mime)
                .and_then(|mime| list.default_app_for(mime));

            let cache = self
                .cache
                .entry(mime.clone())
                .or_insert_with(|| Vec::with_capacity(apps.len()));

            // Sort cached apps for this mime by default precedence.
            for default in defaults
                .into_iter()
                .flatten()
                .chain(aliased_defaults.into_iter().flatten())
            {
                let default = default.strip_suffix(".desktop").unwrap_or(default.as_ref());
                let mut found_any = false;
                apps.retain(|app| {
                    let found = app.id.as_str() == default;
                    if found {
                        app.is_default
                            .write()
                            .unwrap()
                            .insert(mime.essence_str().into());
                        cache.push(app.clone());
                        found_any = true;
                    }

                    !found
                });

                if !found_any && let Some(app) = self.apps.iter().find(|app| app.id == default) {
                    app.is_default
                        .write()
                        .unwrap()
                        .insert(mime.essence_str().into());
                    cache.push(app.clone());
                }
            }

            // Sort remaining apps by name
            apps.sort_by(|a, b| LANGUAGE_SORTER.compare(&a.name, &b.name));
            cache.extend_from_slice(&apps);

            tracing::debug!(target: "mime-apps", mime = mime.essence_str(), apps = ?(cache.iter().map(|app| &*app.id).collect::<Vec<&str>>()), "mime defaults found")
        }

        let associated: rustc_hash::FxHashSet<&str> = self
            .cache
            .values()
            .flatten()
            .map(|app| app.id.as_str())
            .collect();
        for app in &self.apps {
            app.no_display.store(
                !associated.contains(app.id.as_str()),
                atomic::Ordering::Relaxed,
            );
        }

        let elapsed = start.elapsed();
        tracing::info!(target: "mime-apps", "loaded mime app cache in {elapsed:?}");
    }

    pub fn apps(&self) -> &[Arc<MimeApp>] {
        &self.apps
    }

    #[cfg(not(all(not(feature = "desktop"), target_os = "macos")))]
    pub fn get(&self, key: &Mime) -> &[Arc<MimeApp>] {
        self.cache.get(key).map_or(&[], Vec::as_slice)
    }

    /// The handlers for `key`, asking LaunchServices the first time a type comes up.
    #[cfg(all(not(feature = "desktop"), target_os = "macos"))]
    pub fn get(&self, key: &Mime) -> &[Arc<MimeApp>] {
        let mut launch_services = self.launch_services.lock().unwrap();
        if !launch_services.by_mime.contains_key(key) {
            let apps = launch_services.lookup(key);
            launch_services.by_mime.insert(key.clone(), apps);
        }
        let apps: *const [Arc<MimeApp>] = &*launch_services.by_mime[key];
        // SAFETY: the pointer targets a boxed slice's heap allocation, which stays put when
        // the map rehashes. Entries are only dropped or replaced by `reload` and
        // `set_default`, both `&mut self`, so no reference handed out here can outlive one.
        unsafe { &*apps }
    }

    pub fn icons(&self, key: &Mime) -> Vec<widget::icon::Handle> {
        self.get(key).iter().map(|app| app.icon()).collect()
    }

    #[cfg(target_os = "macos")]
    fn get_default_terminal(&self) -> Option<String> {
        None
    }

    #[cfg(not(target_os = "macos"))]
    fn get_default_terminal(&self) -> Option<String> {
        let output = process::Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/terminal"])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        String::from_utf8(output.stdout)
            .ok()
            .map(|string| string.trim().replace(".desktop", ""))
    }

    pub fn terminal(&self) -> Option<&Arc<MimeApp>> {
        //TODO: consider rules in https://github.com/Vladimir-csp/xdg-terminal-exec
        // The current approach works but might not adhere to the spec (yet)

        // Look for and return preferred terminals
        //TODO: fallback order beyond cosmic-term?

        let mut preference_order = vec!["com.system76.CosmicTerm".to_string()];

        if let Some(id) = self.get_default_terminal() {
            preference_order.insert(0, id);
        }

        for id in &preference_order {
            for terminal in &self.terminals {
                if &terminal.id == id {
                    return Some(terminal);
                }
            }
        }

        // Return whatever was the first terminal found
        self.terminals.first()
    }

    #[cfg(all(not(feature = "desktop"), not(target_os = "macos")))]
    pub fn set_default(&mut self, mime: Mime, id: String) {
        log::warn!(
            "failed to set default handler for {mime:?} to {id:?}: desktop feature not enabled"
        );
    }

    /// Make the bundle at `id` the system default for `mime`, and move it to the front of
    /// the cached list right away: LaunchServices applies the change asynchronously and
    /// the dropdown that asked for it is redrawn from this cache.
    #[cfg(all(not(feature = "desktop"), target_os = "macos"))]
    pub fn set_default(&mut self, mime: Mime, id: String) {
        use crate::workspace_macos;

        let bundle = PathBuf::from(&id);
        let Some(content_type) = workspace_macos::content_type_for_mime(&mime) else {
            log::warn!("failed to set default handler for {mime}: no UTType for it");
            return;
        };
        if let Err(err) = workspace_macos::set_default_app(&bundle, &content_type) {
            log::warn!("failed to set default handler for {mime} to {id}: {err}");
            return;
        }

        let launch_services = self.launch_services.get_mut().unwrap();
        let mut apps = launch_services
            .by_mime
            .remove(&mime)
            .map_or_else(Vec::new, Vec::from);
        for app in &apps {
            app.is_default.write().unwrap().remove(mime.essence_str());
        }
        let app = match apps.iter().position(|app| app.id == id) {
            Some(position) => apps.remove(position),
            None => launch_services.app(&bundle),
        };
        app.is_default
            .write()
            .unwrap()
            .insert(mime.essence_str().into());
        apps.insert(0, app);
        launch_services
            .by_mime
            .insert(mime, apps.into_boxed_slice());
    }

    #[cfg(feature = "desktop")]
    pub fn set_default(&mut self, mime: Mime, mut id: String) {
        let Some(path) = cosmic_mime_apps::local_list_path() else {
            log::warn!("failed to find mimeapps.list path");
            return;
        };

        let mut list = cosmic_mime_apps::List::default();
        match fs::read_to_string(&path) {
            Ok(string) => {
                list.load_from(&string);
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    log::warn!("failed to read {}: {}", path.display(), err);
                    return;
                }
            }
        }

        let suffix = ".desktop";
        if !id.ends_with(suffix) {
            id.push_str(suffix);
        }
        list.set_default_app(mime, id);

        let mut string = list.to_string();
        string.push('\n');
        match fs::write(&path, string) {
            Ok(()) => {
                self.reload();
            }
            Err(err) => {
                log::warn!("failed to write {}: {}", path.display(), err);
            }
        }
    }
}

impl Default for MimeAppCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, not(feature = "desktop"), target_os = "macos"))]
mod macos_tests {
    use super::{Mime, MimeAppCache};
    use std::path::Path;

    /// What `mime_icon::mime_for_path` reports for a `.txt` file. The detection itself needs
    /// the shared MIME database, which the test process does not get.
    fn text_file_mime() -> Mime {
        "text/plain".parse().expect("valid mime")
    }

    #[test]
    fn a_text_file_lists_text_edit_with_the_default_first() {
        let cache = MimeAppCache::new();
        let mime = text_file_mime();

        let apps = cache.get(&mime);
        let text_edit = apps
            .iter()
            .find(|app| app.name == "TextEdit")
            .unwrap_or_else(|| {
                panic!(
                    "TextEdit missing from {:?}",
                    apps.iter().map(|a| &a.name).collect::<Vec<_>>()
                )
            });
        assert!(
            text_edit
                .exec
                .as_deref()
                .unwrap()
                .starts_with("/usr/bin/open -a ")
        );
        assert!(text_edit.id.ends_with("TextEdit.app"));

        let first = apps.first().expect("a default handler for text/plain");
        assert!(
            first.is_default(&mime),
            "{} should be marked default",
            first.name
        );
        assert_eq!(apps.iter().filter(|app| app.is_default(&mime)).count(), 1);
    }

    #[test]
    fn an_app_entry_launches_through_open() {
        let cache = MimeAppCache::new();
        let mime = text_file_mime();
        let app = cache.get(&mime).first().expect("a handler").clone();
        let commands = app
            .command(&["/tmp/a.txt", "/tmp/b.txt"])
            .expect("a command");
        assert_eq!(commands.len(), 1);
        let command = &commands[0];
        assert_eq!(command.get_program(), "/usr/bin/open");
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-a");
        assert_eq!(Path::new(&args[1]), Path::new(&app.id));
        assert_eq!(&args[2..], ["/tmp/a.txt", "/tmp/b.txt"]);
    }

    #[test]
    fn the_terminal_is_terminal_app_started_in_the_current_directory() {
        let cache = MimeAppCache::new();
        let terminal = cache.terminal().expect("Terminal.app");
        assert_eq!(terminal.name, "Terminal");
        let command = terminal
            .command::<&str>(&[])
            .and_then(|v| v.into_iter().next())
            .expect("a command");
        assert_eq!(command.get_program(), "/usr/bin/open");
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-a");
        assert!(args[1].ends_with("Terminal.app"), "{args:?}");
        assert_eq!(args[2], ".");
    }

    #[test]
    fn installed_applications_are_listed_for_the_other_apps_section() {
        let cache = MimeAppCache::new();
        assert!(cache.apps().iter().any(|app| app.name == "TextEdit"));
    }
}

#[cfg(test)]
mod tests {
    use super::exec_to_command;

    #[test]
    fn keys_within_words() {
        let exec = "/usr/bin/foo --option=%f";
        let paths = ["file1"];
        let commands = exec_to_command(exec, "keys_within_words", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();

        assert_eq!("/usr/bin/foo", command.get_program().to_str().unwrap());
        assert_eq!(
            "--option=file1",
            command.get_args().next().unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn no_path_f_field_code() {
        let exec = "/usr/bin/foo %f";
        let paths: [&str; 0] = [];
        let commands = exec_to_command(exec, "no_path_f_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();

        assert_eq!("/usr/bin/foo", command.get_program().to_str().unwrap());
        assert_eq!(0, command.get_args().len());
    }

    #[test]
    fn one_path_f_field_code() {
        let exec = "/usr/bin/foo %f";
        let paths = ["file1"];
        let commands = exec_to_command(exec, "one_path_f_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();

        assert_eq!("/usr/bin/foo", command.get_program().to_str().unwrap());
        assert_eq!(
            "file1",
            command.get_args().next().unwrap().to_str().unwrap()
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn one_path_F_field_code() {
        let exec = "/usr/bin/cosmic-term -w %F";
        let paths = ["/home/user"];
        let commands = exec_to_command(exec, "one_path_F_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();
        let mut args = command.get_args();

        assert_eq!(
            "/usr/bin/cosmic-term",
            command.get_program().to_str().unwrap()
        );
        assert_eq!("-w", args.next().unwrap().to_str().unwrap());
        assert_eq!(paths[0], args.next().unwrap().to_str().unwrap());
    }

    #[test]
    fn one_path_u_field_code() {
        let exec = "/usr/bin/cosmic-term -w %u";
        let paths = ["/home/user"];
        let commands = exec_to_command(exec, "one_path_u_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();
        let mut args = command.get_args();

        assert_eq!(
            "/usr/bin/cosmic-term",
            command.get_program().to_str().unwrap()
        );
        assert_eq!("-w", args.next().unwrap().to_str().unwrap());
        assert_eq!(paths[0], args.next().unwrap().to_str().unwrap());
    }

    #[test]
    #[allow(non_snake_case)]
    fn one_path_U_field_code() {
        let exec = "/usr/bin/rmrfbye %U";
        let paths = ["/"];
        let commands = exec_to_command(exec, "one_path_U_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();

        assert_eq!("/usr/bin/rmrfbye", command.get_program().to_str().unwrap());
        assert_eq!("/", command.get_args().next().unwrap().to_str().unwrap());
    }

    #[test]
    fn mult_path_f_field_code() {
        let exec = "/usr/games/ppsspp %f";
        let paths = [
            "/usr/share/games/psp/miku.iso",
            "/usr/share/games/psp/eternia.iso",
        ];
        let commands = exec_to_command(exec, "mult_path_f_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(paths.len(), commands.len());
        for (command, path) in commands.into_iter().zip(paths.iter()) {
            assert_eq!("/usr/games/ppsspp", command.get_program().to_str().unwrap());

            assert_eq!(1, command.get_args().len());
            let command_path = command.get_args().next().unwrap();
            assert_eq!(*path, command_path.to_str().unwrap());
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn mult_path_F_field_code() {
        let exec = "/usr/games/gzdoom %F";
        let paths = [
            "/usr/share/games/doom2/hr.wad",
            "/usr/share/games/doom2/hrmus.wad",
        ];
        let commands = exec_to_command(exec, "mult_path_F_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();

        assert_eq!("/usr/games/gzdoom", command.get_program().to_str().unwrap());
        assert!(
            paths
                .iter()
                .zip(command.get_args())
                .all(|(&expected, actual)| expected == actual.to_string_lossy())
        );
    }

    #[test]
    fn mult_path_u_field_code() {
        let exec = "/usr/bin/cosmic_browser %u";
        let paths = [
            "file:///home/josh/Books/osstep.pdf",
            "https://redox-os.org/",
            "https://system76.com/",
        ];
        let commands = exec_to_command(exec, "mult_path_u_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(paths.len(), commands.len());
        for (command, path) in commands.into_iter().zip(paths.iter()) {
            assert_eq!(
                "/usr/bin/cosmic_browser",
                command.get_program().to_str().unwrap()
            );

            assert_eq!(1, command.get_args().len());
            let command_path = command.get_args().next().unwrap();
            assert_eq!(*path, command_path.to_str().unwrap());
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn mult_path_U_field_code() {
        let exec = "/usr/bin/mpv %U";
        let paths = [
            "frieren01.mkv",
            "rtmp://example.org/this/video/doesnt/exist.avi",
        ];
        let commands = exec_to_command(exec, "mult_path_U_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();
        assert_eq!(paths.len(), command.get_args().count());

        assert_eq!("/usr/bin/mpv", command.get_program().to_str().unwrap());
        assert!(
            paths
                .iter()
                .zip(command.get_args())
                .all(|(&expected, actual)| expected == actual.to_string_lossy())
        );
    }

    #[test]
    fn flatpak_style_exec() {
        // Tests args before field codes
        let exec = "/usr/bin/flatpak run --branch=stable --command=ferris --file-forwarding org.joshfake.ferris @@u %U";
        let args = [
            "run",
            "--branch=stable",
            "--command=ferris",
            "--file-forwarding",
            "org.joshfake.ferris",
            "@@u",
        ];
        let paths = ["file1.rs", "file2.rs"];
        let commands = exec_to_command(exec, "flatpak_style_exec", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();
        assert_eq!(args.len() + paths.len(), command.get_args().count());

        assert_eq!("/usr/bin/flatpak", command.get_program().to_str().unwrap());
        assert!(
            args.iter()
                .chain(paths.iter())
                .zip(command.get_args())
                .all(|(&expected, actual)| expected == actual.to_string_lossy())
        );
    }

    #[test]
    fn multiple_field_codes() {
        // Tests that only one field code is used rather than passing paths to each field code
        let exec = "/usr/games/roguelike %U %f";
        let paths = [
            "file:///usr/share/games/roguelike/mods/mod1",
            "file:///usr/share/games/roguelike/mods/mod2",
        ];
        let commands = exec_to_command(exec, "multiple_field_codes", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();

        assert_eq!(
            "/usr/games/roguelike",
            command.get_program().to_str().unwrap()
        );
        assert!(
            paths
                .iter()
                .zip(command.get_args())
                .all(|(&expected, actual)| expected == actual.to_string_lossy())
        );
    }

    #[test]
    fn sandwiched_field_code() {
        // Tests that arguments before and after the field code works
        // (Borrowed from KDE because someone had this exact line in an issue)
        let exec = "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=okular --file-forwarding org.kde.okular @@u %U @@";
        let args_leading = [
            "run",
            "--branch=stable",
            "--arch=x86_64",
            "--command=okular",
            "--file-forwarding",
            "org.kde.okular",
            "@@u",
        ];
        let paths = ["rust_game_dev.pdf", "superhero_ferris.epub"];
        let args_trailing = ["@@"];
        let commands = exec_to_command(exec, "sandwiched_field_code", None, &paths)
            .expect("Should parse valid exec");

        assert_eq!(1, commands.len());
        let command = commands.first().unwrap();
        assert_eq!(
            args_leading.len() + paths.len() + args_trailing.len(),
            command.get_args().len()
        );

        assert_eq!("/usr/bin/flatpak", command.get_program().to_str().unwrap());
        assert!(
            args_leading
                .iter()
                .chain(paths.iter())
                .chain(args_trailing.iter())
                .zip(command.get_args())
                .all(|(&expected, actual)| expected == actual.to_string_lossy())
        );
    }
}
