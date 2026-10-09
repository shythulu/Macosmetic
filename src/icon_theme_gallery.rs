// SPDX-License-Identifier: GPL-3.0-only

//! The icon theme gallery: every installed theme as a card holding a strip of its own icons,
//! so a theme can be judged by sight before it is picked, and below them the catalog themes
//! that can be downloaded.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, DndDestination, icon};
use cosmic::{Element, theme};

use crate::app::Message;
use crate::clipboard::ClipboardPaste;
use crate::fl;
use crate::icon_theme_catalog::{self, CatalogTheme, InstallError, Marker, Step};
use crate::icon_themes::IconThemeInfo;
use crate::tab::format_size;

/// The icons a card shows, each with the names to try in order. Folders come first, since a
/// file manager is mostly folders, then a few common file types. Keep in step with
/// `PREVIEW_ICONS` in `scripts/icon-theme-catalog.py`.
const PREVIEW_ICONS: &[&[&str]] = &[
    &["folder"],
    &["folder-documents"],
    &["folder-download"],
    &["folder-pictures"],
    &["user-home"],
    &["text-x-generic", "text-plain"],
    &["image-x-generic"],
    &["package-x-generic", "application-x-archive"],
];
const PREVIEW_SIZE: u16 = 32;

/// The preview strip for `theme`, drawn from that theme whatever theme is active. An icon the
/// theme and the themes it inherits from all lack is left out.
pub fn previews(theme: &IconThemeInfo) -> Vec<icon::Handle> {
    PREVIEW_ICONS
        .iter()
        .filter_map(|names| {
            names
                .iter()
                .find_map(|name| themed_icon(&theme.id, name))
        })
        .collect()
}

#[cfg(unix)]
fn themed_icon(theme: &str, name: &str) -> Option<icon::Handle> {
    freedesktop_icons::lookup(name)
        .with_theme(theme)
        .with_size(PREVIEW_SIZE)
        .with_cache()
        .force_svg()
        .find()
        .map(icon::from_path)
}

#[cfg(not(unix))]
fn themed_icon(_theme: &str, _name: &str) -> Option<icon::Handle> {
    None
}

/// Where a catalog theme's installation stands.
#[derive(Clone, Debug)]
pub enum InstallState {
    Installing {
        step: Step,
        /// Set by the Cancel button; the install thread polls it.
        cancel: Arc<AtomicBool>,
    },
    Failed(InstallError),
}

#[derive(Default)]
pub struct Gallery {
    /// Parallel to the installed themes last passed to [`Gallery::refresh`].
    installed_previews: Vec<Vec<icon::Handle>>,
    /// Parallel to the installed themes: the marker of each theme this app installed.
    markers: Vec<Option<Marker>>,
    /// Parallel to the catalog's themes; built once, since they are compiled in.
    catalog_previews: Vec<Vec<icon::Handle>>,
    /// Installed themes whose catalog archive has moved on since they were installed.
    updatable: HashSet<String>,
    /// Theme directories on disk that the settings list leaves out, such as hidden
    /// dependencies; they still count as installed when sizing a download.
    hidden_installed: HashSet<String>,
    pub installs: HashMap<String, InstallState>,
    /// Failed installs whose raw error text is shown.
    pub details: HashSet<String>,
}

impl Gallery {
    /// Rebuild the previews for `themes`, the installed themes.
    pub fn refresh(&mut self, themes: &[IconThemeInfo]) {
        let catalog = icon_theme_catalog::catalog();
        if self.catalog_previews.is_empty() {
            self.catalog_previews = catalog
                .themes
                .iter()
                .map(icon_theme_catalog::previews)
                .collect();
        }
        self.installed_previews = themes.iter().map(previews).collect();
        self.markers = themes
            .iter()
            .map(|theme| {
                theme
                    .roots
                    .first()
                    .and_then(|root| icon_theme_catalog::read_marker(root))
            })
            .collect();
        self.updatable = themes
            .iter()
            .filter(|theme| icon_theme_catalog::needs_update(theme))
            .map(|theme| theme.id.clone())
            .collect();
        self.hidden_installed = icon_theme_catalog::installed_ids();
    }

    /// Whether an install is running, which keeps the other Install buttons disabled: one at
    /// a time keeps shared dependencies from racing.
    fn installing(&self) -> bool {
        self.installs
            .values()
            .any(|state| matches!(state, InstallState::Installing { .. }))
    }

    /// Flags the running install of `id` to stop.
    pub fn cancel(&self, id: &str) {
        if let Some(InstallState::Installing { cancel, .. }) = self.installs.get(id) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn view<'a>(&'a self, themes: &'a [IconThemeInfo], active: &str) -> Element<'a, Message> {
        let cosmic::cosmic_theme::Spacing {
            space_xxs,
            space_xs,
            space_s,
            space_m,
            ..
        } = theme::spacing();
        let mut installed: HashSet<String> = themes.iter().map(|theme| theme.id.clone()).collect();
        installed.extend(self.hidden_installed.iter().cloned());

        let mut children: Vec<Element<'a, Message>> = vec![
            widget::row::with_children(vec![
                widget::text::heading(fl!("installed-icon-themes")).into(),
                widget::space::horizontal().into(),
                widget::button::standard(fl!("install-from-file"))
                    .on_press_maybe((!self.installing()).then_some(Message::IconThemeInstallFile))
                    .into(),
            ])
            .align_y(Alignment::Center)
            .into(),
        ];
        // Installs from local archives, which have no card of their own until they land.
        let mut file_installs: Vec<(&String, &InstallState)> = self
            .installs
            .iter()
            .filter(|(key, _)| key.starts_with("file:"))
            .collect();
        file_installs.sort_by_key(|(key, _)| *key);
        for (key, state) in file_installs {
            let name = key.strip_prefix("file:").unwrap_or(key);
            let mut rows = vec![widget::text::heading(name).into()];
            match state {
                InstallState::Installing { step, .. } => {
                    rows.push(widget::text::caption(step_text(*step)).into());
                }
                InstallState::Failed(error) => rows.extend(self.error_rows(key, error)),
            }
            children.push(card(rows, space_xs, space_s));
        }
        for (index, info) in themes.iter().enumerate() {
            let is_active = info.id == active;
            let marker = self.markers.get(index).and_then(Option::as_ref);
            let entry = icon_theme_catalog::theme(&info.id);
            let mut title = vec![
                widget::text::heading(info.name.as_str()).into(),
                widget::space::horizontal().into(),
            ];
            match self.installs.get(&info.id) {
                Some(InstallState::Installing { step, cancel: _ }) => {
                    title.push(progress_control(*step, &info.id));
                }
                Some(InstallState::Failed(_)) => {
                    title.push(
                        widget::button::standard(fl!("retry"))
                            .on_press(Message::IconThemeUpdate(info.id.clone()))
                            .into(),
                    );
                }
                None => {
                    if self.updatable.contains(&info.id) {
                        title.push(
                            widget::button::suggested(fl!("update"))
                                .on_press_maybe(
                                    (!self.installing())
                                        .then(|| Message::IconThemeUpdate(info.id.clone())),
                                )
                                .into(),
                        );
                    }
                    if is_active {
                        title.push(
                            widget::icon::from_name("object-select-symbolic")
                                .size(16)
                                .into(),
                        );
                    } else if marker.is_some() && !required_by_installed(&info.id, &installed) {
                        title.push(
                            widget::button::text(fl!("remove"))
                                .on_press(Message::IconThemeRemove(info.id.clone()))
                                .into(),
                        );
                    }
                }
            }
            let mut rows = vec![
                widget::row::with_children(title)
                    .align_y(Alignment::Center)
                    .spacing(space_xs)
                    .into(),
            ];
            if let Some(InstallState::Installing { step, .. }) = self.installs.get(&info.id) {
                rows.push(widget::text::caption(step_text(*step)).into());
            } else {
                rows.push(strip(self.installed_previews.get(index), space_xs));
            }
            if let Some(InstallState::Failed(error)) = self.installs.get(&info.id) {
                rows.extend(self.error_rows(&info.id, error));
            }
            // Where the theme came from. The bundled set has no roots and says nothing.
            let mut details: Vec<String> = Vec::new();
            if let Some(entry) = entry.filter(|_| marker.is_some()) {
                details.push(entry.license.clone());
            }
            let origin = match marker {
                Some(marker) => match marker.file() {
                    Some(file) => Some(fl!("installed-from-file", file = file)),
                    None => marker
                        .installed_date()
                        .map(|date| fl!("installed-on", date = date)),
                },
                // The bundled theme is the app's own, whatever copies sit on disk.
                None if !info.roots.is_empty() && info.id != cosmic::icon_theme::COSMIC => {
                    Some(fl!("installed-outside-app"))
                }
                None => None,
            };
            details.extend(origin);
            if !details.is_empty() {
                let mut caption = vec![
                    widget::text::caption(details.join(" · "))
                        .width(Length::Fill)
                        .into(),
                ];
                if let Some(entry) = entry.filter(|_| marker.is_some()) {
                    caption.push(source_link(entry));
                }
                rows.push(
                    widget::row::with_children(caption)
                        .align_y(Alignment::Center)
                        .into(),
                );
            }
            let card = card(rows, space_xs, space_s);
            children.push(
                widget::button::custom(card)
                    .class(theme::Button::Image)
                    .padding(0)
                    .width(Length::Fill)
                    .selected(is_active)
                    .on_press(Message::IconTheme(index))
                    .into(),
            );
        }

        let available: Vec<_> = icon_theme_catalog::catalog()
            .themes
            .iter()
            .enumerate()
            .filter(|(_, theme)| !installed.contains(&theme.id))
            .collect();
        if !available.is_empty() {
            children.push(widget::space::vertical().height(space_m).into());
            children.push(widget::text::heading(fl!("available-icon-themes")).into());
            children.push(widget::text::caption(fl!("available-icon-themes-description")).into());
            for (index, theme) in available {
                children.push(self.available_card(theme, index, &installed, space_xs, space_s));
            }
        }

        children.push(widget::space::vertical().height(space_m).into());
        children.push(widget::text::caption(fl!("drop-theme-hint")).into());

        let content = widget::column::with_children(children).spacing(space_xxs);
        DndDestination::for_data::<ClipboardPaste>(content, |data, _action| {
            Message::IconThemeInstallFiles(data.map(|data| data.paths).unwrap_or_default())
        })
        .into()
    }

    fn available_card<'a>(
        &'a self,
        theme: &'a CatalogTheme,
        index: usize,
        installed: &HashSet<String>,
        space_xs: u16,
        space_s: u16,
    ) -> Element<'a, Message> {
        let plan = icon_theme_catalog::install_plan(&theme.id, installed);
        let mut details = format!(
            "{} · {}",
            theme.license,
            fl!(
                "icon-theme-download-size",
                size = format_size(icon_theme_catalog::plan_size(&plan))
            )
        );
        let others: Vec<&str> = plan
            .iter()
            .filter(|other| other.id != theme.id)
            .map(|other| other.name.as_str())
            .collect();
        if !others.is_empty() {
            details.push_str(" · ");
            details.push_str(&fl!(
                "icon-theme-also-installs",
                names = others.join(", ")
            ));
        }

        let state = self.installs.get(&theme.id);
        let control: Element<'a, Message> = match state {
            Some(InstallState::Installing { step, .. }) => progress_control(*step, &theme.id),
            Some(InstallState::Failed(_)) => widget::button::standard(fl!("retry"))
                .on_press(Message::IconThemeInstall(theme.id.clone()))
                .into(),
            None => widget::button::standard(fl!("install"))
                .on_press_maybe(
                    (!self.installing()).then(|| Message::IconThemeInstall(theme.id.clone())),
                )
                .into(),
        };
        let mut rows = vec![
            widget::row::with_children(vec![
                widget::text::heading(theme.name.as_str()).into(),
                widget::space::horizontal().into(),
                control,
            ])
            .align_y(Alignment::Center)
            .spacing(space_xs)
            .into(),
        ];
        match state {
            Some(InstallState::Installing { step, .. }) => {
                rows.push(widget::text::caption(step_text(*step)).into());
            }
            Some(InstallState::Failed(error)) => {
                rows.push(strip(self.catalog_previews.get(index), space_xs));
                rows.extend(self.error_rows(&theme.id, error));
            }
            None => {
                rows.push(strip(self.catalog_previews.get(index), space_xs));
                rows.push(
                    widget::row::with_children(vec![
                        widget::text::caption(details).width(Length::Fill).into(),
                        source_link(theme),
                    ])
                    .align_y(Alignment::Center)
                    .into(),
                );
            }
        }
        card(rows, space_xs, space_s)
    }

    /// The sentence for a failed install, a Details link, and the raw text once it is pressed.
    fn error_rows<'a>(&'a self, id: &str, error: &InstallError) -> Vec<Element<'a, Message>> {
        let mut rows = vec![
            widget::row::with_children(vec![
                widget::text::caption(error_text(error))
                    .width(Length::Fill)
                    .into(),
                widget::button::link(fl!("details"))
                    .on_press(Message::IconThemeToggleDetails(id.to_string()))
                    .into(),
            ])
            .align_y(Alignment::Center)
            .into(),
        ];
        if self.details.contains(id) {
            rows.push(widget::text::caption(error.detail()).into());
        }
        rows
    }
}

/// A progress bar with a Cancel button beside it.
fn progress_control<'a>(step: Step, id: &str) -> Element<'a, Message> {
    widget::row::with_children(vec![
        widget::determinate_linear(step.fraction())
            .width(Length::Fixed(96.0))
            .girth(Length::Fixed(4.0))
            .into(),
        widget::button::text(fl!("cancel"))
            .on_press(Message::IconThemeInstallCancel(id.to_string()))
            .into(),
    ])
    .align_y(Alignment::Center)
    .spacing(theme::spacing().space_xs)
    .into()
}

fn step_text(step: Step) -> String {
    match step {
        Step::Downloading { done, total } => fl!(
            "downloading-progress",
            done = format_size(done.min(total)),
            total = format_size(total)
        ),
        Step::Extracting => fl!("extracting-theme"),
    }
}

/// The plain sentence for an error; the raw text sits behind Details.
fn error_text(error: &InstallError) -> String {
    match error {
        InstallError::Network { host, .. } => fl!("theme-install-failed-network", host = host),
        InstallError::Moved => fl!("theme-install-failed-moved"),
        InstallError::Checksum => fl!("theme-install-failed-checksum"),
        InstallError::TooLarge(size) => {
            fl!("theme-install-failed-too-large", size = format_size(*size))
        }
        InstallError::UnsafeArchive(_) => fl!("theme-install-failed-unsafe"),
        InstallError::NoTheme => fl!("theme-install-failed-no-theme"),
        InstallError::Exists(id) => fl!("theme-install-failed-exists", id = id),
        InstallError::NoSpace { needed } => fl!(
            "theme-install-failed-space",
            dir = icon_theme_catalog::user_icons_dir().display().to_string(),
            needed = format_size(*needed)
        ),
        InstallError::Cancelled => fl!("cancelled"),
        InstallError::Io(detail) => fl!("theme-install-failed-other", error = detail),
    }
}

/// The licences ask for the source to be a click away.
fn source_link<'a>(theme: &CatalogTheme) -> Element<'a, Message> {
    widget::button::link(fl!("icon-theme-source"))
        .on_press(Message::LaunchUrl(theme.homepage.clone()))
        .into()
}

/// Whether an installed theme needs `id` beside it.
fn required_by_installed(id: &str, installed: &HashSet<String>) -> bool {
    installed.iter().any(|other| {
        icon_theme_catalog::theme(other).is_some_and(|theme| theme.requires.iter().any(|r| r == id))
    })
}

fn strip<'a>(icons: Option<&'a Vec<icon::Handle>>, spacing: u16) -> Element<'a, Message> {
    widget::flex_row(
        icons
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|handle| {
                widget::icon::icon(handle.clone())
                    .size(PREVIEW_SIZE)
                    .into()
            })
            .collect(),
    )
    .row_spacing(spacing)
    .column_spacing(spacing)
    .into()
}

fn card<'a>(rows: Vec<Element<'a, Message>>, spacing: u16, padding: u16) -> Element<'a, Message> {
    widget::container(widget::column::with_children(rows).spacing(spacing))
        .padding(padding)
        .width(Length::Fill)
        .class(theme::Container::Card)
        .into()
}
