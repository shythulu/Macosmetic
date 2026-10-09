// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

//! The default folder opener: which application macOS hands a folder to when it is opened
//! from the Dock, Spotlight, a shell's `open <dir>`, or another application.
//!
//! Launch Services keeps the choice per content type. [`FolderOpener`] reads the handler for
//! `public.folder`, offers to make this bundle the handler or hand it back to Finder, and
//! turns the paths the system delivers into the tabs to open. The AppKit side that receives
//! those paths lives in [`crate::appkit_macos`].
//!
//! Only a bundle can be the handler: Launch Services records applications by bundle, so the
//! bare binary `cargo run` starts has nothing to register. The setting is disabled there, with
//! the reason underneath it. The record is also per bundle path, so a copy built to
//! `target/macos` and one in `/Applications` are two applications to Launch Services, and the
//! default follows whichever was set last.

use std::env;
use std::path::{Path, PathBuf};

use block2::RcBlock;
use cosmic::Element;
use cosmic::iced::Task;
use cosmic::iced::futures::StreamExt;
use cosmic::iced::futures::channel::mpsc;
use cosmic::widget::settings;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSError, NSString, NSURL};
use objc2_uniform_type_identifiers::UTTypeFolder;

use crate::fl;
use crate::tab::Location;

/// Finder's bundle, the handler to restore. The path is fixed on every macOS release.
const FINDER_PATH: &str = "/System/Library/CoreServices/Finder.app";
const FINDER_BUNDLE_ID: &str = "com.apple.finder";

/// Who opens folders right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Handler {
    /// Finder, the system default.
    Finder,
    /// The bundle this process runs from.
    ThisApp,
    /// Another application, named by its bundle.
    Other(String),
    /// Launch Services named no application, which does not happen on a working system.
    Unknown,
}

#[derive(Clone, Debug)]
pub enum Message {
    /// The system asked for these paths to be opened.
    Opened(Vec<PathBuf>),
    /// The switch was flipped: `true` asks for this application, `false` for Finder.
    SetDefault(bool),
    /// A change finished, with the error Launch Services reported if it refused.
    Changed(Result<(), String>),
}

/// A tab to open for the documents the system delivered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTarget {
    pub location: Location,
    /// The files to select, when the target is a file's parent rather than a folder.
    pub selection: Option<Vec<PathBuf>>,
}

/// The tabs that open `paths`: one per folder, and one per parent folder of the files, with
/// those files selected. Order follows the first appearance of each folder. `is_dir` says
/// which paths are folders; a path with no parent is dropped.
pub fn open_targets(paths: &[PathBuf], is_dir: impl Fn(&Path) -> bool) -> Vec<OpenTarget> {
    let mut targets: Vec<OpenTarget> = Vec::new();
    for path in paths {
        if is_dir(path) {
            if !targets.iter().any(|target| {
                target.selection.is_none() && target.location == Location::Path(path.clone())
            }) {
                targets.push(OpenTarget {
                    location: Location::Path(path.clone()),
                    selection: None,
                });
            }
            continue;
        }
        let Some(parent) = path.parent() else {
            log::warn!(
                "cannot show {} in a folder: it has no parent",
                path.display()
            );
            continue;
        };
        let location = Location::Path(parent.to_path_buf());
        match targets
            .iter_mut()
            .find(|target| target.selection.is_some() && target.location == location)
        {
            Some(target) => target.selection.get_or_insert_default().push(path.clone()),
            None => targets.push(OpenTarget {
                location,
                selection: Some(vec![path.clone()]),
            }),
        }
    }
    targets
}

/// The settings row and the state behind it.
#[derive(Debug)]
pub struct FolderOpener {
    /// The bundle this process runs from, or `None` for a bare binary.
    bundle: Option<PathBuf>,
    handler: Handler,
    /// Why the last change failed, shown until the next one.
    error: Option<String>,
}

impl FolderOpener {
    /// Reads the bundle and the current handler. Main thread.
    pub fn new() -> Self {
        let bundle = env::current_exe()
            .ok()
            .as_deref()
            .and_then(crate::launch_macos::bundle_resources)
            .and_then(|resources| {
                // `<name>.app/Contents/Resources` back up to `<name>.app`.
                resources
                    .parent()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            });
        let mut this = Self {
            bundle,
            handler: Handler::Unknown,
            error: None,
        };
        this.refresh();
        this
    }

    /// Re-read who opens folders.
    pub fn refresh(&mut self) {
        self.handler = current_handler(self.bundle.as_deref());
        log::info!("folders open in {:?}", self.handler);
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // Routed by the application, which owns the tabs.
            Message::Opened(_) => Task::none(),
            Message::SetDefault(this_app) => {
                let Some(bundle) = self.bundle.as_deref() else {
                    log::warn!("not a bundle, so cannot become the default folder opener");
                    return Task::none();
                };
                let target = if this_app {
                    bundle
                } else {
                    Path::new(FINDER_PATH)
                };
                self.error = None;
                set_default_handler(target)
            }
            Message::Changed(result) => {
                self.error = result.err();
                self.refresh();
                Task::none()
            }
        }
    }

    /// The settings section: one switch, with who opens folders now underneath.
    pub fn view(&self) -> Element<'_, Message> {
        let description = match (&self.bundle, &self.error, &self.handler) {
            (None, _, _) => fl!("folder-opener-not-bundled"),
            (_, Some(error), _) => fl!("folder-opener-failed", error = error.as_str()),
            (_, None, Handler::Finder) => fl!("folders-open-in", app = "Finder"),
            (_, None, Handler::ThisApp) => fl!("folders-open-in", app = "Macosmetic"),
            (_, None, Handler::Other(name)) => fl!("folders-open-in", app = name.as_str()),
            (_, None, Handler::Unknown) => fl!("folder-opener-unknown"),
        };
        settings::section()
            .title(fl!("folder-opener"))
            .add(
                settings::item::builder(fl!("use-for-folders"))
                    .description(description)
                    .toggler_maybe(
                        self.handler == Handler::ThisApp,
                        self.bundle.as_ref().map(|_| Message::SetDefault),
                    ),
            )
            .into()
    }
}

/// Ask Launch Services who opens `public.folder`, and name it relative to `bundle`.
fn current_handler(bundle: Option<&Path>) -> Handler {
    // SAFETY: `UTTypeFolder` is a constant the framework defines.
    let folder = unsafe { UTTypeFolder };
    // SAFETY: the content type is a valid `UTType`; the call returns an owned URL or nil.
    let Some(url) = NSWorkspace::sharedWorkspace().URLForApplicationToOpenContentType(folder)
    else {
        return Handler::Unknown;
    };
    let path = url.path().map(|path| PathBuf::from(path.to_string()));
    let id = NSBundle::bundleWithURL(&url)
        .and_then(|bundle| bundle.bundleIdentifier())
        .map(|id| id.to_string());
    classify_handler(path.as_deref(), id.as_deref(), bundle)
}

/// Which [`Handler`] a Launch Services answer names. Pure, so the comparison is testable: the
/// bundle identifier settles Finder, the path settles this bundle, the file name is the rest.
fn classify_handler(path: Option<&Path>, bundle_id: Option<&str>, this: Option<&Path>) -> Handler {
    if bundle_id == Some(FINDER_BUNDLE_ID) {
        return Handler::Finder;
    }
    let Some(path) = path else {
        return Handler::Unknown;
    };
    if this.is_some_and(|this| this == path) {
        return Handler::ThisApp;
    }
    match path.file_stem() {
        Some(name) => Handler::Other(name.to_string_lossy().into_owned()),
        None => Handler::Unknown,
    }
}

/// Make the bundle at `app` the handler for `public.folder`. Launch Services answers on a
/// queue of its own, so the result comes back as a message.
fn set_default_handler(app: &Path) -> Task<Message> {
    let (tx, mut rx) = mpsc::unbounded::<Result<(), String>>();
    let completion = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: Launch Services passes nil or an error object it keeps alive for the call.
        let result = match unsafe { error.as_ref() } {
            Some(error) => Err(error.localizedDescription().to_string()),
            None => Ok(()),
        };
        let _ = tx.unbounded_send(result);
    });

    let url = NSURL::fileURLWithPath(&NSString::from_str(&app.to_string_lossy()));
    // SAFETY: `UTTypeFolder` is a constant the framework defines, and the block outlives the
    // call: `RcBlock` moves it to the heap and the framework retains it until it has answered.
    let folder = unsafe { UTTypeFolder };
    log::info!(
        "asking Launch Services to open folders with {}",
        app.display()
    );
    NSWorkspace::sharedWorkspace().setDefaultApplicationAtURL_toOpenContentType_completionHandler(
        &url,
        folder,
        Some(&completion),
    );

    Task::future(async move {
        match rx.next().await {
            Some(result) => Message::Changed(result),
            None => Message::Changed(Err("no answer from Launch Services".to_string())),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn a_folder_opens_as_its_own_tab() {
        let targets = open_targets(&[path("/Users/me/Documents")], |_| true);
        assert_eq!(
            targets,
            vec![OpenTarget {
                location: Location::Path(path("/Users/me/Documents")),
                selection: None,
            }]
        );
    }

    #[test]
    fn a_file_opens_its_parent_with_the_file_selected() {
        let targets = open_targets(&[path("/Users/me/Documents/a.txt")], |_| false);
        assert_eq!(
            targets,
            vec![OpenTarget {
                location: Location::Path(path("/Users/me/Documents")),
                selection: Some(vec![path("/Users/me/Documents/a.txt")]),
            }]
        );
    }

    #[test]
    fn files_in_one_folder_share_a_tab_and_folders_are_not_repeated() {
        let paths = [
            path("/Users/me/Documents/a.txt"),
            path("/Users/me"),
            path("/Users/me/Documents/b.txt"),
            path("/Users/me"),
        ];
        let targets = open_targets(&paths, |p| p == Path::new("/Users/me"));
        assert_eq!(
            targets,
            vec![
                OpenTarget {
                    location: Location::Path(path("/Users/me/Documents")),
                    selection: Some(vec![
                        path("/Users/me/Documents/a.txt"),
                        path("/Users/me/Documents/b.txt"),
                    ]),
                },
                OpenTarget {
                    location: Location::Path(path("/Users/me")),
                    selection: None,
                },
            ]
        );
    }

    #[test]
    fn a_folder_and_a_file_inside_it_are_two_tabs() {
        // Browsing the folder and showing the file in it are different views of it.
        let paths = [path("/Users/me"), path("/Users/me/notes.md")];
        let targets = open_targets(&paths, |p| p == Path::new("/Users/me"));
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].selection, None);
        assert_eq!(targets[1].selection, Some(vec![path("/Users/me/notes.md")]));
    }

    #[test]
    fn finder_is_known_by_its_bundle_identifier() {
        assert_eq!(
            classify_handler(
                Some(Path::new(FINDER_PATH)),
                Some(FINDER_BUNDLE_ID),
                Some(Path::new("/Applications/COSMIC Files.app"))
            ),
            Handler::Finder
        );
    }

    #[test]
    fn this_bundle_is_known_by_its_path() {
        let this = Path::new("/Applications/COSMIC Files.app");
        assert_eq!(
            classify_handler(Some(this), Some("com.system76.CosmicFiles"), Some(this)),
            Handler::ThisApp
        );
    }

    #[test]
    fn a_copy_of_this_bundle_elsewhere_is_another_application() {
        // Launch Services records one application per path, so a build in target/macos and an
        // install in /Applications are two handlers even with the same identifier.
        assert_eq!(
            classify_handler(
                Some(Path::new("/Users/me/dev/target/macos/COSMIC Files.app")),
                Some("com.system76.CosmicFiles"),
                Some(Path::new("/Applications/COSMIC Files.app"))
            ),
            Handler::Other("COSMIC Files".to_string())
        );
    }

    #[test]
    fn any_other_application_is_named_by_its_bundle() {
        assert_eq!(
            classify_handler(
                Some(Path::new("/Applications/ForkLift.app")),
                Some("com.binarynights.ForkLift"),
                None
            ),
            Handler::Other("ForkLift".to_string())
        );
        assert_eq!(classify_handler(None, None, None), Handler::Unknown);
    }
}
