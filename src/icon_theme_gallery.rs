// SPDX-License-Identifier: GPL-3.0-only

//! The icon theme gallery: every installed theme as a card holding a strip of its own icons,
//! so a theme can be judged by sight before it is picked.

use cosmic::widget::{self, icon};
use cosmic::{Element, theme};

use crate::app::Message;
use crate::icon_themes::IconThemeInfo;

/// The icons a card shows, each with the names to try in order. Folders come first, since a
/// file manager is mostly folders, then a few common file types.
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
        .filter_map(|names| names.iter().find_map(|name| themed_icon(&theme.id, name)))
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

/// One card per theme; pressing a card makes its theme the active one. `previews` runs
/// parallel to `themes`.
pub fn view<'a>(
    themes: &'a [IconThemeInfo],
    previews: &'a [Vec<icon::Handle>],
    active: &str,
) -> Element<'a, Message> {
    let cosmic::cosmic_theme::Spacing {
        space_xxs,
        space_xs,
        space_s,
        ..
    } = theme::spacing();

    let cards: Vec<_> = themes
        .iter()
        .enumerate()
        .map(|(index, info)| -> Element<'a, Message> {
            let is_active = info.id == active;
            let mut title = vec![
                widget::text::heading(info.name.as_str()).into(),
                widget::space::horizontal().into(),
            ];
            if is_active {
                title.push(
                    widget::icon::from_name("object-select-symbolic")
                        .size(16)
                        .into(),
                );
            }
            let icons = previews.get(index).map_or(&[][..], Vec::as_slice);
            let strip = widget::flex_row(
                icons
                    .iter()
                    .map(|handle| widget::icon::icon(handle.clone()).size(PREVIEW_SIZE).into())
                    .collect(),
            )
            .row_spacing(space_xs)
            .column_spacing(space_xs);

            let card = widget::container(
                widget::column::with_children(vec![
                    widget::row::with_children(title)
                        .align_y(cosmic::iced::Alignment::Center)
                        .into(),
                    strip.into(),
                ])
                .spacing(space_xs),
            )
            .padding(space_s)
            .width(cosmic::iced::Length::Fill)
            .class(theme::Container::Card);

            widget::button::custom(card)
                .class(theme::Button::Image)
                .padding(0)
                .width(cosmic::iced::Length::Fill)
                .selected(is_active)
                .on_press(Message::IconTheme(index))
                .into()
        })
        .collect();

    widget::column::with_children(cards)
        .spacing(space_xxs)
        .into()
}
