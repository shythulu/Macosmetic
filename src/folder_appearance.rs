// SPDX-License-Identifier: GPL-3.0-only

//! The "Customize folder" context drawer: pick a colour, a theme icon or an image for
//! one or more folders.
//!
//! The quick path, a colour, also lives in the context menu (`menu::folder_colour_menu`).
//! This drawer is for everything that needs a grid, a search or a file dialog.

use cosmic::iced::Length;
use cosmic::widget::{self, icon, settings};
use cosmic::{Element, theme};
use std::path::PathBuf;

use crate::app::{ContextPage, Message};
use crate::fl;
use crate::folder_look::{self, FOLDER_COLOURS, FolderLook, Shared};
use crate::icon_themes::{self, IconGroup, IconThemeInfo};
use crate::tab;

/// Icons shown at once in the icon grid; the footer says how many more the search can reach.
const MAX_ICONS: usize = 96;
const PREVIEW_SIZE: u16 = 64;
const PREVIEW_LIST_SIZE: u16 = 32;
const PREVIEW_SIDEBAR_SIZE: u16 = 16;
const SWATCH_SIZE: u16 = 40;
const GRID_ICON_SIZE: u16 = 40;
const THUMBNAIL_SIZE: u16 = 40;
const CHECK_SIZE: u16 = 12;

/// One icon in the grid, with the look pressing it applies.
struct IconEntry {
    look: FolderLook,
    label: String,
    handle: icon::Handle,
}

pub struct FolderAppearance {
    pub paths: Vec<PathBuf>,
    /// `None` is the active theme; the rest are installed themes to pin icons to.
    icon_sets: Vec<Option<IconThemeInfo>>,
    icon_set_labels: Vec<String>,
    icon_set: usize,
    search: String,
    /// Folder icon names in the chosen set.
    names: Vec<String>,
    /// The last icons picked, most recent first, from the config.
    recent_looks: Vec<FolderLook>,
    shared: Shared,
    preview: icon::Handle,
    preview_list: icon::Handle,
    preview_sidebar: icon::Handle,
    swatches: Vec<(&'static str, icon::Handle)>,
    recent: Vec<IconEntry>,
    places: Vec<IconEntry>,
    purpose: Vec<IconEntry>,
    /// Icons the search matched, before the [`MAX_ICONS`] cap.
    total_matches: usize,
}

impl FolderAppearance {
    pub fn new(paths: Vec<PathBuf>, themes: &[IconThemeInfo], recent: Vec<FolderLook>) -> Self {
        let active = cosmic::icon_theme::default();
        let mut icon_sets = vec![None];
        let mut icon_set_labels = vec![String::new()];
        for theme in themes {
            icon_set_labels.push(theme.name.clone());
            icon_sets.push(Some(theme.clone()));
        }
        // Start on whatever set the folder's icon already comes from. A pinned theme that
        // is no longer installed falls back to the active one; `caption` says so.
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
        let placeholder = icon::from_name("folder").handle();
        let mut page = Self {
            paths,
            icon_sets,
            icon_set_labels,
            icon_set,
            search: String::new(),
            names: Vec::new(),
            recent_looks: recent,
            shared: Shared::None,
            preview: placeholder.clone(),
            preview_list: placeholder.clone(),
            preview_sidebar: placeholder,
            swatches: Vec::new(),
            recent: Vec::new(),
            places: Vec::new(),
            purpose: Vec::new(),
            total_matches: 0,
        };
        page.load_names(&active);
        page.refresh();
        page
    }

    /// The icon name the grid and swatches are drawn on. Several folders share the plain
    /// `folder`: the first one's Documents emblem would not match the others.
    fn base_name(&self) -> &'static str {
        match self.paths.as_slice() {
            [path] => tab::folder_base_icon_name(path),
            _ => "folder",
        }
    }

    fn theme_id(&self) -> Option<String> {
        self.icon_sets[self.icon_set]
            .as_ref()
            .map(|theme| theme.id.clone())
    }

    /// The display name of an installed theme, if it is installed.
    fn theme_name(&self, id: &str) -> Option<&str> {
        self.icon_sets
            .iter()
            .flatten()
            .find(|theme| theme.id == id)
            .map(|theme| theme.name.as_str())
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

    pub fn set_recent(&mut self, recent: Vec<FolderLook>) {
        self.recent_looks = recent;
        self.refresh();
    }

    /// Rebuilds every handle the view shows, after a change of set, search, look or theme.
    pub fn refresh(&mut self) {
        let active = cosmic::icon_theme::default();
        // Option 0 names the active theme, so a theme switch relabels it.
        let active_name = self.theme_name(&active).unwrap_or(&active).to_string();
        self.icon_set_labels[0] = fl!("icons-from-current", theme = active_name);

        self.shared = folder_look::shared_stored_look(self.paths.iter().map(PathBuf::as_path));
        let base = self.base_name();
        let look = self.shared.look().cloned();
        self.preview = self.preview_handle(look.as_ref(), base, PREVIEW_SIZE);
        self.preview_list = self.preview_handle(look.as_ref(), base, PREVIEW_LIST_SIZE);
        self.preview_sidebar = match self.paths.as_slice() {
            [path] => tab::folder_icon_symbolic(path, PREVIEW_SIDEBAR_SIZE),
            _ => look
                .as_ref()
                .and_then(|look| {
                    folder_look::folder_handle_symbolic(look, base, PREVIEW_SIDEBAR_SIZE)
                })
                .unwrap_or_else(|| {
                    icon::from_name("folder-symbolic")
                        .size(PREVIEW_SIDEBAR_SIZE)
                        .handle()
                }),
        };
        self.swatches = FOLDER_COLOURS
            .iter()
            .filter_map(|colour| {
                let look = FolderLook::Colour(colour.id.to_string());
                folder_look::folder_handle(&look, base, SWATCH_SIZE)
                    .map(|handle| (colour.id, handle))
            })
            .collect();

        let search = self.search.to_lowercase();
        let entry = |look: FolderLook| {
            let FolderLook::Icon { name, .. } = &look else {
                return None;
            };
            let label = icon_themes::icon_label(name);
            if !icon_themes::icon_matches(name, &label, &search) {
                return None;
            }
            folder_look::folder_handle(&look, base, GRID_ICON_SIZE).map(|handle| IconEntry {
                look,
                label,
                handle,
            })
        };
        // Recent picks keep the theme they were made in, whatever set is showing.
        self.recent = self
            .recent_looks
            .iter()
            .cloned()
            .filter_map(entry)
            .collect();

        let theme = self.theme_id();
        let mut matches: Vec<(IconGroup, IconEntry)> = self
            .names
            .iter()
            .filter_map(|name| {
                entry(FolderLook::Icon {
                    theme: theme.clone(),
                    name: name.clone(),
                })
                .map(|entry| (icon_themes::icon_group(name), entry))
            })
            .collect();
        self.total_matches = matches.len();
        // Places first, each group by label, the plain folder ahead of everything.
        matches.sort_by_cached_key(|(group, entry)| {
            let is_plain = matches!(&entry.look, FolderLook::Icon { name, .. } if name == "folder");
            (
                *group == IconGroup::Purpose,
                !is_plain,
                entry.label.to_lowercase(),
            )
        });
        matches.truncate(MAX_ICONS);
        self.places.clear();
        self.purpose.clear();
        for (group, entry) in matches {
            match group {
                IconGroup::Places => self.places.push(entry),
                IconGroup::Purpose => self.purpose.push(entry),
            }
        }
    }

    fn preview_handle(&self, look: Option<&FolderLook>, base: &str, size: u16) -> icon::Handle {
        match self.paths.as_slice() {
            [path] => tab::folder_icon_with_look(path, look, size),
            _ => look
                .and_then(|look| folder_look::folder_handle(look, base, size))
                .unwrap_or_else(|| icon::from_name(base).prefer_svg(true).size(size).handle()),
        }
    }

    /// The one icon the search narrowed the grid down to, for Enter to apply.
    pub fn single_match(&self) -> Option<FolderLook> {
        let mut looks = self
            .recent
            .iter()
            .chain(&self.places)
            .chain(&self.purpose)
            .map(|entry| &entry.look);
        let first = looks.next()?;
        looks.all(|look| look == first).then(|| first.clone())
    }

    /// Whether any selected folder has a stored look to clear.
    pub fn has_stored_look(&self) -> bool {
        self.paths
            .iter()
            .any(|path| folder_look::stored_look(path).is_some())
    }

    /// The drawer's header action: Clear, enabled while there is something to clear.
    pub fn actions(&self) -> Element<'_, Message> {
        widget::button::standard(fl!("clear"))
            .on_press_maybe(
                self.has_stored_look()
                    .then_some(Message::FolderLookSet(None)),
            )
            .into()
    }

    /// One line on what the selected folders show now.
    fn caption(&self) -> String {
        match &self.shared {
            Shared::None => fl!("look-none"),
            Shared::Mixed => fl!("look-mixed", count = self.paths.len()),
            Shared::Look(FolderLook::Colour(id)) => {
                fl!("look-colour", colour = folder_look::colour_label(id))
            }
            Shared::Look(FolderLook::Icon { theme: None, name }) => {
                fl!("look-icon-follows", icon = icon_themes::icon_label(name))
            }
            Shared::Look(FolderLook::Icon {
                theme: Some(theme),
                name,
            }) => {
                let icon = icon_themes::icon_label(name);
                match self.theme_name(theme) {
                    Some(theme) => fl!("look-icon-pinned", icon = icon, theme = theme),
                    None => fl!(
                        "look-icon-pinned-missing",
                        icon = icon,
                        theme = theme.as_str()
                    ),
                }
            }
            Shared::Look(FolderLook::Image(path)) => {
                if path.exists() {
                    fl!("look-image")
                } else {
                    fl!("look-image-missing")
                }
            }
        }
    }

    /// What picking from the current icon set means for the folder later.
    fn icon_set_help(&self) -> String {
        match &self.icon_sets[self.icon_set] {
            None => fl!("icons-follow-theme"),
            Some(theme) => fl!("icons-pinned-to", theme = theme.name.as_str()),
        }
    }

    fn icon_grid<'a>(
        &'a self,
        title: String,
        entries: &'a [IconEntry],
    ) -> Option<Element<'a, Message>> {
        let cosmic::cosmic_theme::Spacing { space_xxs, .. } = theme::spacing();
        if entries.is_empty() {
            return None;
        }
        let current = self.shared.look();
        let grid = widget::flex_row(
            entries
                .iter()
                .map(|entry| {
                    selected_icon_button(
                        entry.handle.clone(),
                        GRID_ICON_SIZE,
                        current == Some(&entry.look),
                        entry.label.clone(),
                        Message::FolderLookSet(Some(entry.look.clone())),
                    )
                })
                .collect(),
        )
        .row_spacing(space_xxs)
        .column_spacing(space_xxs);
        Some(
            widget::column::with_children(vec![widget::text::heading(title).into(), grid.into()])
                .spacing(space_xxs)
                .into(),
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        let cosmic::cosmic_theme::Spacing {
            space_xxs, space_s, ..
        } = theme::spacing();
        let current = self.shared.look();

        let title = match self.paths.as_slice() {
            [path] => tab::Location::Path(path.clone()).title(),
            paths => fl!("folders-selected", count = paths.len()),
        };
        let small_previews = widget::row::with_children(vec![
            widget::icon::icon(self.preview_list.clone())
                .size(PREVIEW_LIST_SIZE)
                .into(),
            widget::text::caption(fl!("preview-list")).into(),
            widget::icon::icon(self.preview_sidebar.clone())
                .size(PREVIEW_SIDEBAR_SIZE)
                .into(),
            widget::text::caption(fl!("preview-sidebar")).into(),
        ])
        .spacing(space_xxs)
        .align_y(cosmic::iced::Alignment::Center);
        let header = widget::row::with_children(vec![
            widget::icon::icon(self.preview.clone())
                .size(PREVIEW_SIZE)
                .into(),
            widget::column::with_children(vec![
                widget::text::title4(title).into(),
                widget::text::caption(self.caption()).into(),
                small_previews.into(),
            ])
            .spacing(space_xxs)
            .into(),
        ])
        .spacing(space_s)
        .align_y(cosmic::iced::Alignment::Center);

        let mut colour_section = settings::section().title(fl!("folder-colour"));
        if self.swatches.is_empty() {
            colour_section = colour_section.add(widget::text::body(fl!("no-coloured-folders")));
        } else {
            let swatches = widget::flex_row(
                self.swatches
                    .iter()
                    .map(|(id, handle)| {
                        let look = FolderLook::Colour((*id).to_string());
                        selected_icon_button(
                            handle.clone(),
                            SWATCH_SIZE,
                            current == Some(&look),
                            folder_look::colour_label(id),
                            Message::FolderLookSet(Some(look)),
                        )
                    })
                    .collect(),
            )
            .row_spacing(space_xxs)
            .column_spacing(space_xxs);
            let colour_name = match current {
                Some(FolderLook::Colour(id)) => folder_look::colour_label(id),
                _ => fl!("colour-unset"),
            };
            colour_section = colour_section
                .add(swatches)
                .add(widget::text::body(colour_name));
        }

        let mut icon_section = settings::section()
            .title(fl!("folder-icon"))
            .add(
                settings::item::builder(fl!("icons-from")).control(widget::dropdown(
                    &self.icon_set_labels,
                    Some(self.icon_set),
                    Message::FolderLookIconSet,
                )),
            )
            .add(widget::text::caption(self.icon_set_help()))
            .add(
                widget::search_input(fl!("search-icons"), &self.search)
                    .on_input(Message::FolderLookSearch)
                    .on_clear(Message::FolderLookSearch(String::new()))
                    .on_submit(|_| Message::FolderLookSearchSubmit),
            );
        let groups = [
            self.icon_grid(fl!("icon-group-recent"), &self.recent),
            self.icon_grid(fl!("icon-group-places"), &self.places),
            self.icon_grid(fl!("icon-group-purpose"), &self.purpose),
        ];
        if groups.iter().all(Option::is_none) {
            icon_section = icon_section.add(widget::text::body(fl!("no-matching-icons")));
        }
        for group in groups.into_iter().flatten() {
            icon_section = icon_section.add(group);
        }
        let shown = self.places.len() + self.purpose.len();
        if shown < self.total_matches {
            icon_section = icon_section.add(widget::text::caption(fl!(
                "icons-shown",
                shown = shown,
                total = self.total_matches
            )));
        }
        icon_section = icon_section.add(
            widget::button::text(fl!("browse-icon-themes"))
                .trailing_icon(widget::icon::from_name("go-next-symbolic"))
                .on_press(Message::ToggleContextPage(ContextPage::IconThemes)),
        );

        let image_row = match current {
            Some(FolderLook::Image(path)) if path.exists() => {
                settings::item::builder(fl!("look-image"))
                    .icon(
                        widget::button::icon(icon::from_path(path.clone()))
                            .icon_size(THUMBNAIL_SIZE)
                            .selected(true)
                            .on_press(Message::FolderLookChooseImage),
                    )
                    .control(
                        widget::button::standard(fl!("change"))
                            .on_press(Message::FolderLookChooseImage),
                    )
            }
            _ => settings::item::builder(fl!("use-own-picture")).control(
                widget::button::standard(fl!("browse")).on_press(Message::FolderLookChooseImage),
            ),
        };

        widget::column::with_children(vec![
            header.into(),
            widget::text::caption(fl!("one-look-rule")).into(),
            colour_section.into(),
            icon_section.into(),
            settings::section()
                .title(fl!("folder-image"))
                .add(image_row)
                .into(),
        ])
        .spacing(space_s)
        .into()
    }
}

/// An icon button with a tooltip, marked with a small check when it is the current look.
/// The highlight ring alone is easy to miss on a row of folders that differ only by hue.
fn selected_icon_button<'a>(
    handle: icon::Handle,
    size: u16,
    selected: bool,
    label: String,
    on_press: Message,
) -> Element<'a, Message> {
    let button: Element<_> = widget::button::icon(handle)
        .icon_size(size)
        .selected(selected)
        .on_press(on_press)
        .into();
    let content: Element<_> = if selected {
        cosmic::iced::widget::stack([
            button,
            widget::container(
                widget::icon::from_name("object-select-symbolic")
                    .size(CHECK_SIZE)
                    .icon(),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_right(Length::Fill)
            .align_bottom(Length::Fill)
            .into(),
        ])
        .into()
    } else {
        button
    };
    widget::tooltip(
        content,
        widget::text::body(label),
        widget::tooltip::Position::Bottom,
    )
    .into()
}
