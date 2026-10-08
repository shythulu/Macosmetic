// SPDX-License-Identifier: GPL-3.0-only

//! The "Customize Folder" context drawer: pick a colour, a theme icon or an image for
//! one or more folders.

use cosmic::widget::{self, icon, settings};
use cosmic::{Element, theme};
use std::path::PathBuf;

use crate::app::Message;
use crate::fl;
use crate::folder_look::{self, FOLDER_COLOURS, FolderLook};
use crate::icon_themes::{self, IconThemeInfo};
use crate::tab;

/// Icons shown at once in the icon grid; the search narrows the rest down.
const MAX_ICONS: usize = 96;
const PREVIEW_SIZE: u16 = 64;
const SWATCH_SIZE: u16 = 40;
const GRID_ICON_SIZE: u16 = 40;

pub struct FolderAppearance {
    pub paths: Vec<PathBuf>,
    /// `None` is the active theme; the rest are installed themes to pin icons to.
    icon_sets: Vec<Option<IconThemeInfo>>,
    icon_set_labels: Vec<String>,
    icon_set: usize,
    search: String,
    /// Folder icon names in the chosen set.
    names: Vec<String>,
    preview: icon::Handle,
    swatches: Vec<(&'static str, icon::Handle)>,
    icons: Vec<(String, icon::Handle)>,
}

impl FolderAppearance {
    pub fn new(paths: Vec<PathBuf>, themes: &[IconThemeInfo]) -> Self {
        let active = cosmic::icon_theme::default();
        let mut icon_sets = vec![None];
        let mut icon_set_labels = vec![fl!("icon-set-current")];
        for theme in themes {
            icon_set_labels.push(theme.name.clone());
            icon_sets.push(Some(theme.clone()));
        }
        // Start on whatever set the folder's icon already comes from.
        let look = paths
            .first()
            .and_then(|path| folder_look::stored_look(path));
        let icon_set = match &look {
            Some(FolderLook::Icon {
                theme: Some(theme), ..
            }) => icon_sets
                .iter()
                .position(|set| set.as_ref().is_some_and(|set| &set.id == theme))
                .unwrap_or(0),
            _ => 0,
        };
        let mut page = Self {
            paths,
            icon_sets,
            icon_set_labels,
            icon_set,
            search: String::new(),
            names: Vec::new(),
            preview: icon::from_name("folder").handle(),
            swatches: Vec::new(),
            icons: Vec::new(),
        };
        page.load_names(&active);
        page.refresh();
        page
    }

    fn base_name(&self) -> &'static str {
        self.paths
            .first()
            .map_or("folder", |path| tab::folder_base_icon_name(path))
    }

    fn theme_id(&self) -> Option<String> {
        self.icon_sets[self.icon_set]
            .as_ref()
            .map(|theme| theme.id.clone())
    }

    fn load_names(&mut self, active: &str) {
        let theme = match &self.icon_sets[self.icon_set] {
            Some(theme) => Some(theme.clone()),
            None => self
                .icon_sets
                .iter()
                .flatten()
                .find(|theme| theme.id == active)
                .cloned(),
        };
        self.names = theme
            .as_ref()
            .map(icon_themes::folder_icon_names)
            .unwrap_or_default();
        if self.names.is_empty() {
            // The active theme may be the bundled fallback, which has no directory to list.
            self.names = [
                "folder",
                "folder-documents",
                "folder-download",
                "folder-music",
            ]
            .into_iter()
            .chain([
                "folder-pictures",
                "folder-videos",
                "user-home",
                "user-desktop",
            ])
            .map(String::from)
            .collect();
        }
    }

    pub fn set_icon_set(&mut self, index: usize) {
        if index < self.icon_sets.len() {
            self.icon_set = index;
            self.load_names(&cosmic::icon_theme::default());
            self.refresh();
        }
    }

    pub fn set_search(&mut self, search: String) {
        self.search = search;
        self.refresh();
    }

    /// Rebuilds every handle the view shows, after a change of set, search, look or theme.
    pub fn refresh(&mut self) {
        let base = self.base_name();
        let look = self.current_look();
        self.preview = self
            .paths
            .first()
            .map(|path| tab::folder_icon_with_look(path, look.as_ref(), PREVIEW_SIZE))
            .unwrap_or_else(|| icon::from_name("folder").size(PREVIEW_SIZE).handle());
        self.swatches = FOLDER_COLOURS
            .iter()
            .filter_map(|colour| {
                let look = FolderLook::Colour(colour.id.to_string());
                folder_look::folder_handle(&look, base, SWATCH_SIZE)
                    .map(|handle| (colour.id, handle))
            })
            .collect();
        let theme = self.theme_id();
        let search = self.search.to_lowercase();
        self.icons = self
            .names
            .iter()
            .filter(|name| search.is_empty() || name.to_lowercase().contains(&search))
            .take(MAX_ICONS)
            .filter_map(|name| {
                let look = FolderLook::Icon {
                    theme: theme.clone(),
                    name: name.clone(),
                };
                folder_look::folder_handle(&look, base, GRID_ICON_SIZE)
                    .map(|handle| (name.clone(), handle))
            })
            .collect();
    }

    /// The look every selected folder shares, or `None` if they differ or have none.
    pub fn current_look(&self) -> Option<FolderLook> {
        let mut looks = self.paths.iter().map(|path| folder_look::stored_look(path));
        let first = looks.next()??;
        looks
            .all(|look| look.as_ref() == Some(&first))
            .then_some(first)
    }

    pub fn icon_look(&self, name: &str) -> FolderLook {
        FolderLook::Icon {
            theme: self.theme_id(),
            name: name.to_string(),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let cosmic::cosmic_theme::Spacing {
            space_xxs, space_s, ..
        } = theme::spacing();
        let current = self.current_look();

        let title = match self.paths.as_slice() {
            [path] => tab::Location::Path(path.clone()).title(),
            paths => fl!("folders-selected", count = paths.len()),
        };
        let header = widget::row::with_children(vec![
            widget::icon::icon(self.preview.clone())
                .size(PREVIEW_SIZE)
                .into(),
            widget::text::title4(title).into(),
        ])
        .spacing(space_s)
        .align_y(cosmic::iced::Alignment::Center);

        let swatches = widget::flex_row(
            self.swatches
                .iter()
                .map(|(id, handle)| {
                    let look = FolderLook::Colour((*id).to_string());
                    widget::tooltip(
                        widget::button::icon(handle.clone())
                            .icon_size(SWATCH_SIZE)
                            .selected(current.as_ref() == Some(&look))
                            .on_press(Message::FolderLookSet(Some(look))),
                        widget::text::body(colour_label(id)),
                        widget::tooltip::Position::Bottom,
                    )
                    .into()
                })
                .collect(),
        )
        .row_spacing(space_xxs)
        .column_spacing(space_xxs);

        let icon_grid: Element<_> = if self.icons.is_empty() {
            widget::text::body(fl!("no-matching-icons")).into()
        } else {
            widget::flex_row(
                self.icons
                    .iter()
                    .map(|(name, handle)| {
                        let look = self.icon_look(name);
                        widget::tooltip(
                            widget::button::icon(handle.clone())
                                .icon_size(GRID_ICON_SIZE)
                                .selected(current.as_ref() == Some(&look))
                                .on_press(Message::FolderLookSet(Some(look))),
                            widget::text::body(name.clone()),
                            widget::tooltip::Position::Bottom,
                        )
                        .into()
                    })
                    .collect(),
            )
            .row_spacing(space_xxs)
            .column_spacing(space_xxs)
            .into()
        };

        let image_label = match &current {
            Some(FolderLook::Image(path)) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            _ => String::new(),
        };

        widget::column::with_children(vec![
            header.into(),
            settings::section()
                .title(fl!("folder-colour"))
                .add(swatches)
                .into(),
            settings::section()
                .title(fl!("folder-icon"))
                .add(
                    settings::item::builder(fl!("icon-set")).control(widget::dropdown(
                        &self.icon_set_labels,
                        Some(self.icon_set),
                        Message::FolderLookIconSet,
                    )),
                )
                .add(
                    widget::search_input(fl!("search-icons"), &self.search)
                        .on_input(Message::FolderLookSearch)
                        .on_clear(Message::FolderLookSearch(String::new())),
                )
                .add(icon_grid)
                .into(),
            settings::section()
                .title(fl!("folder-image"))
                .add(
                    settings::item::builder(fl!("choose-image"))
                        .description(image_label)
                        .control(
                            widget::button::standard(fl!("browse"))
                                .on_press(Message::FolderLookChooseImage),
                        ),
                )
                .into(),
            widget::row::with_children(vec![
                widget::space::horizontal().into(),
                widget::button::destructive(fl!("reset-folder-appearance"))
                    .on_press_maybe(
                        self.paths
                            .iter()
                            .any(|path| folder_look::stored_look(path).is_some())
                            .then_some(Message::FolderLookSet(None)),
                    )
                    .into(),
            ])
            .into(),
        ])
        .spacing(space_s)
        .into()
    }
}

fn colour_label(id: &str) -> String {
    match id {
        "red" => fl!("colour-red"),
        "orange" => fl!("colour-orange"),
        "yellow" => fl!("colour-yellow"),
        "green" => fl!("colour-green"),
        "cyan" => fl!("colour-cyan"),
        "blue" => fl!("colour-blue"),
        "violet" => fl!("colour-violet"),
        "magenta" => fl!("colour-magenta"),
        "brown" => fl!("colour-brown"),
        "grey" => fl!("colour-grey"),
        other => other.to_string(),
    }
}
