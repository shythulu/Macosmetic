// SPDX-License-Identifier: GPL-3.0-only

//! The icon theme gallery: every installed theme as a card holding a strip of its own icons,
//! so a theme can be judged by sight before it is picked, and below them the catalog themes
//! that can be downloaded.

use std::collections::{HashMap, HashSet};

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, icon};
use cosmic::{Element, theme};

use crate::app::Message;
use crate::fl;
use crate::icon_theme_catalog::{self, CatalogTheme};
use crate::icon_themes::IconThemeInfo;

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
    /// Downloading, with the overall progress from 0 to 1.
    Installing(f32),
    Failed(String),
}

#[derive(Default)]
pub struct Gallery {
    /// Parallel to the installed themes last passed to [`Gallery::refresh`].
    installed_previews: Vec<Vec<icon::Handle>>,
    /// Parallel to the catalog's themes; built once, since they are compiled in.
    catalog_previews: Vec<Vec<icon::Handle>>,
    /// Installed themes that came from the catalog, and so may be removed.
    removable: HashSet<String>,
    pub installs: HashMap<String, InstallState>,
    /// Themes installed since launch, which the icon lookup cannot see until a restart.
    pub needs_restart: HashSet<String>,
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
        self.installed_previews = themes
            .iter()
            .map(|theme| {
                // A theme installed since launch has nothing the lookup can find yet.
                if self.needs_restart.contains(&theme.id)
                    && let Some(index) = catalog.themes.iter().position(|t| t.id == theme.id)
                {
                    return self.catalog_previews[index].clone();
                }
                previews(theme)
            })
            .collect();
        self.removable = themes
            .iter()
            .filter(|theme| {
                theme
                    .roots
                    .first()
                    .is_some_and(|root| icon_theme_catalog::is_catalog_install(root))
            })
            .map(|theme| theme.id.clone())
            .collect();
    }

    pub fn view<'a>(&'a self, themes: &'a [IconThemeInfo], active: &str) -> Element<'a, Message> {
        let cosmic::cosmic_theme::Spacing {
            space_xxs,
            space_xs,
            space_s,
            space_m,
            ..
        } = theme::spacing();
        let installed: HashSet<String> = themes.iter().map(|theme| theme.id.clone()).collect();

        let mut children: Vec<Element<'a, Message>> =
            vec![widget::text::heading(fl!("installed-icon-themes")).into()];
        for (index, info) in themes.iter().enumerate() {
            let is_active = info.id == active;
            let needs_restart = self.needs_restart.contains(&info.id);
            let mut title = vec![
                widget::text::heading(info.name.as_str()).into(),
                widget::space::horizontal().into(),
            ];
            if needs_restart {
                title.push(
                    widget::button::suggested(fl!("restart-to-use"))
                        .on_press(Message::Restart)
                        .into(),
                );
            } else if is_active {
                title.push(
                    widget::icon::from_name("object-select-symbolic")
                        .size(16)
                        .into(),
                );
            } else if self.removable.contains(&info.id)
                && !required_by_installed(&info.id, &installed)
            {
                title.push(
                    widget::button::text(fl!("remove"))
                        .on_press(Message::IconThemeRemove(info.id.clone()))
                        .into(),
                );
            }
            let card = card(
                vec![
                    widget::row::with_children(title)
                        .align_y(Alignment::Center)
                        .into(),
                    strip(self.installed_previews.get(index), space_xs),
                ],
                space_xs,
                space_s,
            );
            children.push(
                widget::button::custom(card)
                    .class(theme::Button::Image)
                    .padding(0)
                    .width(Length::Fill)
                    .selected(is_active)
                    .on_press_maybe((!needs_restart).then_some(Message::IconTheme(index)))
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

        widget::column::with_children(children)
            .spacing(space_xxs)
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
                size = crate::tab::format_size(icon_theme_catalog::plan_size(&plan))
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

        let control: Element<'a, Message> = match self.installs.get(&theme.id) {
            Some(InstallState::Installing(progress)) => widget::determinate_linear(*progress)
                .width(Length::Fixed(96.0))
                .girth(Length::Fixed(4.0))
                .into(),
            Some(InstallState::Failed(_)) => widget::button::standard(fl!("retry"))
                .on_press(Message::IconThemeInstall(theme.id.clone()))
                .into(),
            None => widget::button::standard(fl!("install"))
                .on_press_maybe(
                    // One install at a time keeps shared dependencies from racing.
                    self.installs
                        .values()
                        .all(|state| !matches!(state, InstallState::Installing(_)))
                        .then(|| Message::IconThemeInstall(theme.id.clone())),
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
            .into(),
            strip(self.catalog_previews.get(index), space_xs),
            widget::row::with_children(vec![
                widget::text::caption(details).width(Length::Fill).into(),
                // The licences ask for the source to be a click away.
                widget::button::link(fl!("icon-theme-source"))
                    .on_press(Message::LaunchUrl(theme.homepage.clone()))
                    .into(),
            ])
            .align_y(Alignment::Center)
            .into(),
        ];
        if let Some(InstallState::Failed(error)) = self.installs.get(&theme.id) {
            rows.push(
                widget::text::caption(fl!(
                    "icon-theme-install-failed",
                    error = error.as_str()
                ))
                .into(),
            );
        }
        card(rows, space_xs, space_s)
    }
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
