// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

//! LaunchServices, for Open With.
//!
//! The freedesktop MIME database knows no Mac apps, so on macOS the app list for a file
//! type comes from `NSWorkspace`. The cache in `mime_app.rs` is keyed by MIME type, and
//! LaunchServices speaks UTType, so [`content_type_for_mime`] bridges the two. Everything
//! here is safe to call from any thread: the `NSWorkspace` query methods are documented
//! thread-safe, and `NSImage` is only used to rasterise an icon into a PNG.

use mime_guess::Mime;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSError, NSFileManager, NSString, NSURL};
use objc2_uniform_type_identifiers::UTType;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Pixel edge of the icons rendered for the Open With list. The widgets draw them at 32
/// points, so 64 pixels stays sharp on a Retina display.
pub const ICON_PIXELS: isize = 64;

fn ns_url(path: &Path) -> Option<Retained<NSURL>> {
    let path = path.to_str()?;
    Some(NSURL::fileURLWithPath(&NSString::from_str(path)))
}

fn url_to_path(url: &NSURL) -> Option<PathBuf> {
    url.path().map(|path| PathBuf::from(path.to_string()))
}

/// Whether a UTType is one LaunchServices made up on the spot for an unknown MIME type or
/// extension. Those carry no declared handlers, so a better match is worth looking for.
fn is_dynamic(content_type: &UTType) -> bool {
    content_type.identifier().to_string().starts_with("dyn.")
}

/// The UTType a MIME type maps to, or `None` when neither the type nor any extension
/// `mime_guess` lists for it is known to the system.
///
/// `inode/directory` is the shared-mime-info spelling for a folder and has no MIME
/// registration in LaunchServices, so it is mapped by hand to `public.folder`.
pub fn content_type_for_mime(mime: &Mime) -> Option<Retained<UTType>> {
    if mime.essence_str() == "inode/directory" {
        return UTType::typeWithIdentifier(&NSString::from_str("public.folder"));
    }

    let by_mime = UTType::typeWithMIMEType(&NSString::from_str(mime.essence_str()));
    if let Some(content_type) = &by_mime
        && !is_dynamic(content_type)
    {
        return by_mime;
    }

    let by_extension = mime_guess::get_mime_extensions(mime)
        .into_iter()
        .flatten()
        .filter_map(|ext| UTType::typeWithFilenameExtension(&NSString::from_str(ext)))
        .find(|content_type| !is_dynamic(content_type));

    by_extension.or(by_mime)
}

/// Every application registered to open `content_type`, most relevant first, and the one
/// LaunchServices would pick by default. The default is included in the list.
pub fn apps_for_content_type(content_type: &UTType) -> (Vec<PathBuf>, Option<PathBuf>) {
    let workspace = NSWorkspace::sharedWorkspace();
    let default = workspace
        .URLForApplicationToOpenContentType(content_type)
        .as_deref()
        .and_then(url_to_path);
    let mut apps: Vec<PathBuf> = workspace
        .URLsForApplicationsToOpenContentType(content_type)
        .iter()
        .filter_map(|url| url_to_path(&url))
        .collect();
    if let Some(default) = &default
        && !apps.contains(default)
    {
        apps.insert(0, default.clone());
    }
    (apps, default)
}

/// The name Finder shows for a bundle: localised, without the `.app` extension.
pub fn display_name(path: &Path) -> String {
    match path.to_str() {
        Some(path) => NSFileManager::defaultManager()
            .displayNameAtPath(&NSString::from_str(path))
            .to_string(),
        None => path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// The application bundle registered under `bundle_id`, if installed.
pub fn app_for_bundle_id(bundle_id: &str) -> Option<PathBuf> {
    NSWorkspace::sharedWorkspace()
        .URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))
        .as_deref()
        .and_then(url_to_path)
}

/// Make `app` the default handler for `content_type`, system wide.
///
/// LaunchServices applies the change asynchronously; a failure is logged from its
/// completion handler. The synchronous `Err` only covers a path that cannot become a URL.
pub fn set_default_app(app: &Path, content_type: &UTType) -> Result<(), String> {
    let app_url =
        ns_url(app).ok_or_else(|| format!("path is not valid UTF-8: {}", app.display()))?;
    let app_name = app.display().to_string();
    let type_name = content_type.identifier().to_string();

    // SAFETY: the block only reads the error it is handed, which is valid for the duration
    // of the call, and owns the two strings it logs.
    let completion = block2::RcBlock::new(move |error: *mut NSError| {
        if error.is_null() {
            log::info!("{app_name} is now the default for {type_name}");
        } else {
            let description = unsafe { (*error).localizedDescription() };
            log::warn!("failed to make {app_name} the default for {type_name}: {description}");
        }
    });

    NSWorkspace::sharedWorkspace().setDefaultApplicationAtURL_toOpenContentType_completionHandler(
        &app_url,
        content_type,
        Some(&completion),
    );
    Ok(())
}

/// The cache file an icon for `app` goes to, named after the bundle path and its
/// modification time so an updated app gets a fresh render.
pub fn icon_cache_path(app: &Path) -> Option<PathBuf> {
    let mut hasher = rustc_hash::FxHasher::default();
    app.hash(&mut hasher);
    if let Ok(modified) = std::fs::metadata(app).and_then(|meta| meta.modified()) {
        modified.hash(&mut hasher);
    }
    Some(
        dirs::cache_dir()?
            .join("cosmic-files")
            .join("app-icons")
            .join(format!("{:016x}-{ICON_PIXELS}.png", hasher.finish())),
    )
}

/// Render the icon LaunchServices shows for `app` into a PNG at `dst`.
///
/// The icon comes back as an `NSImage` holding every size the bundle ships. Its TIFF form
/// is the one export that keeps them all as plain bitmaps, so the smallest one at least
/// [`ICON_PIXELS`] wide is picked from there and written out as PNG.
pub fn save_icon_png(app: &Path, dst: &Path) -> Result<(), String> {
    let app_str = app
        .to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", app.display()))?;

    let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(app_str));
    let tiff = image
        .TIFFRepresentation()
        .ok_or_else(|| format!("no bitmap for the icon of {}", app.display()))?;

    let mut best: Option<Retained<NSBitmapImageRep>> = None;
    for rep in NSBitmapImageRep::imageRepsWithData(&tiff).iter() {
        let Ok(bitmap) = rep.downcast::<NSBitmapImageRep>() else {
            continue;
        };
        let width = bitmap.pixelsWide();
        let better = match &best {
            None => true,
            Some(current) => {
                let current_width = current.pixelsWide();
                if current_width >= ICON_PIXELS {
                    width >= ICON_PIXELS && width < current_width
                } else {
                    width > current_width
                }
            }
        };
        if better {
            best = Some(bitmap);
        }
    }
    let bitmap = best.ok_or_else(|| format!("icon of {} has no bitmap", app.display()))?;

    // SAFETY: an empty properties dictionary is valid for a PNG export.
    let png = unsafe {
        bitmap.representationUsingType_properties(
            NSBitmapImageFileType::PNG,
            &NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new(),
        )
    }
    .ok_or_else(|| format!("PNG export failed for the icon of {}", app.display()))?;

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(dst, png.to_vec()).map_err(|err| err.to_string())
}

/// Render the icon for `app` into the cache if it is not there yet, and return its path.
pub fn cached_icon(app: &Path) -> Option<PathBuf> {
    let dst = icon_cache_path(app)?;
    if dst.is_file() {
        return Some(dst);
    }
    match save_icon_png(app, &dst) {
        Ok(()) => Some(dst),
        Err(err) => {
            log::warn!("failed to render the icon of {}: {err}", app.display());
            None
        }
    }
}

/// The application bundles in the standard application folders, one level deep.
pub fn installed_apps() -> Vec<PathBuf> {
    let mut folders = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    folders.push(crate::home_dir().join("Applications"));

    let mut apps = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "app") {
                apps.push(path);
            }
        }
    }
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mime(s: &str) -> Mime {
        s.parse().expect("valid mime")
    }

    #[test]
    fn plain_text_maps_to_the_public_plain_text_type() {
        let content_type = content_type_for_mime(&mime("text/plain")).expect("known type");
        assert_eq!(content_type.identifier().to_string(), "public.plain-text");
    }

    #[test]
    fn a_directory_maps_to_the_folder_type() {
        let content_type = content_type_for_mime(&mime("inode/directory")).expect("folder");
        assert_eq!(content_type.identifier().to_string(), "public.folder");
    }

    #[test]
    fn text_edit_opens_plain_text_and_a_default_exists() {
        let content_type = content_type_for_mime(&mime("text/plain")).expect("known type");
        let (apps, default) = apps_for_content_type(&content_type);
        assert!(
            apps.iter().any(|app| app.ends_with("TextEdit.app")),
            "expected TextEdit in {apps:?}"
        );
        let default = default.expect("a default handler for public.plain-text");
        assert!(apps.contains(&default));
    }

    #[test]
    fn a_bundle_has_a_display_name_without_its_extension() {
        assert_eq!(
            display_name(Path::new("/System/Applications/TextEdit.app")),
            "TextEdit"
        );
    }

    #[test]
    fn terminal_is_installed() {
        let terminal = app_for_bundle_id("com.apple.Terminal").expect("Terminal.app");
        assert!(terminal.ends_with("Terminal.app"), "{terminal:?}");
    }

    #[test]
    fn an_app_icon_renders_to_a_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dst = dir.path().join("textedit.png");
        save_icon_png(Path::new("/System/Applications/TextEdit.app"), &dst).expect("icon PNG");
        let bytes = std::fs::read(&dst).expect("read PNG");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}
