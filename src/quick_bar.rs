// SPDX-License-Identifier: GPL-3.0-only

//! The quick-action bar at the top of a context menu: a row of icon buttons for the
//! actions people reach for most (cut, copy, paste, rename), as in the Windows 11 File
//! Explorer menu.
//!
//! [`items`] decides which buttons the bar shows for a selection and clipboard state. It
//! is a pure function so the rules can be tested without a window. [`view`] turns that
//! list into the element the menu holds as its first row. The menu widget never asks its
//! rows for overlays, so a plain `widget::tooltip` would not show inside it; [`QuickBar`]
//! paints the hovered button's name and shortcut itself instead.

use std::collections::HashMap;

use cosmic::iced::core::border::Border;
use cosmic::iced::core::mouse;
use cosmic::iced::core::renderer::{self, Quad, Renderer as _};
use cosmic::iced::core::text::{
    self, LineHeight, Paragraph as _, Renderer as _, Shaping, Wrapping,
};
use cosmic::iced::core::widget::{Operation, Tree, tree};
use cosmic::iced::core::{
    Clipboard, Color, Layout, Length, Pixels, Point, Rectangle, Shell, Size, Vector, Widget,
    alignment, layout, overlay,
};
use cosmic::widget::menu::key_bind::KeyBind;
use cosmic::{Element, Renderer, Theme, widget};

use crate::app::Action;
use crate::fl;
use crate::key_bind::menu_label;
use crate::tab;

/// Height of the bar row inside the menu. Matches the menu's own item height.
const BAR_HEIGHT: f32 = 40.0;
/// Horizontal inset of the first button, matching the menu entries' text inset.
const BAR_INSET: u16 = 12;
/// Gap between the bar and its tooltip, and between buttons.
const GAP: f32 = 4.0;
const TOOLTIP_PADDING: f32 = 8.0;
const TOOLTIP_FONT_SIZE: f32 = 14.0;
/// Icon opacity of a button that cannot be pressed right now.
const DISABLED_OPACITY: f32 = 0.4;

/// What the bar needs to know about the menu it opens in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Items selected in the tab when the menu opened.
    pub selected: usize,
    /// How many of those are mount points. Mount points cannot be cut or renamed.
    pub selected_mount_points: usize,
    /// The clipboard holds something this location can take.
    pub can_paste: bool,
}

/// The actions the bar can carry, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickAction {
    Cut,
    Copy,
    Paste,
    Rename,
}

impl QuickAction {
    /// The [`Action`] the menu list runs for the same entry.
    pub const fn action(self) -> Action {
        match self {
            Self::Cut => Action::Cut,
            Self::Copy => Action::Copy,
            Self::Paste => Action::Paste,
            Self::Rename => Action::Rename,
        }
    }

    /// Icon name in the COSMIC set; every theme the port ships or inherits has them.
    pub const fn icon_name(self) -> &'static str {
        match self {
            Self::Cut => "edit-cut-symbolic",
            Self::Copy => "edit-copy-symbolic",
            Self::Paste => "edit-paste-symbolic",
            Self::Rename => "edit-symbolic",
        }
    }

    /// The name shown in the tooltip; the menu list's label without its ellipsis.
    pub fn label(self) -> String {
        let label = match self {
            Self::Cut => fl!("cut"),
            Self::Copy => fl!("copy"),
            Self::Paste => fl!("paste"),
            Self::Rename => fl!("rename"),
        };
        label
            .trim_end_matches("...")
            .trim_end_matches('…')
            .to_string()
    }
}

/// One button of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub action: QuickAction,
    /// Disabled buttons are drawn dimmed and do nothing when pressed.
    pub enabled: bool,
}

/// Which buttons the bar shows, left to right, for `context`.
///
/// Mirrors the conditions of the matching list entries: cut and rename need a selection with
/// no mount point, copy needs a selection, rename needs exactly one item. Paste is always
/// present and enabled only when the clipboard has something, so the bar keeps its shape
/// while copying and pasting.
pub fn items(context: Context) -> Vec<Item> {
    let has_selection = context.selected > 0;
    let movable = has_selection && context.selected_mount_points == 0;
    let mut items = Vec::with_capacity(4);
    if movable {
        items.push(Item {
            action: QuickAction::Cut,
            enabled: true,
        });
    }
    if has_selection {
        items.push(Item {
            action: QuickAction::Copy,
            enabled: true,
        });
    }
    items.push(Item {
        action: QuickAction::Paste,
        enabled: context.can_paste,
    });
    if movable && context.selected == 1 {
        items.push(Item {
            action: QuickAction::Rename,
            enabled: true,
        });
    }
    items
}

/// Whether `items` has a button for `action`, so the list below can drop its own entry.
pub fn carries(items: &[Item], action: QuickAction) -> bool {
    items.iter().any(|item| item.action == action)
}

/// The bar as a menu row. Each press runs the same message as the list entry would.
pub fn view(
    items: &[Item],
    key_binds: &HashMap<KeyBind, Action>,
) -> Element<'static, tab::Message> {
    let mut row = widget::row::with_capacity(items.len())
        .spacing(GAP as u16)
        .align_y(alignment::Vertical::Center);
    let mut tooltips = Vec::with_capacity(items.len());
    for item in items {
        let action = item.action.action();
        let handle = widget::icon::from_name(item.action.icon_name())
            .size(16)
            .handle();
        if item.enabled {
            row = row
                .push(widget::button::icon(handle).on_press(tab::Message::ContextAction(action)));
        } else {
            // The icon widget ignores the button's disabled colour, so dim it here.
            let icon = widget::icon::icon(handle)
                .size(16)
                .opacity(DISABLED_OPACITY);
            row = row.push(
                widget::button::custom(icon)
                    .class(cosmic::theme::Button::Icon)
                    .padding(cosmic::theme::spacing().space_xxs),
            );
        }
        tooltips.push(tooltip_text(item.action, key_binds));
    }
    let content = widget::container(row)
        .padding([0, BAR_INSET])
        .height(Length::Fixed(BAR_HEIGHT))
        .width(Length::Fill)
        .align_y(alignment::Vertical::Center);
    QuickBar {
        content: content.into(),
        tooltips,
    }
    .into()
}

/// "Cut  ⌘X": the action's name and, when bound, its shortcut in menu notation.
fn tooltip_text(action: QuickAction, key_binds: &HashMap<KeyBind, Action>) -> String {
    let target = action.action();
    let shortcut = key_binds
        .iter()
        .filter(|(_, bound)| **bound == target)
        .map(|(key_bind, _)| menu_label(key_bind))
        .min();
    match shortcut {
        Some(shortcut) if !shortcut.is_empty() => format!("{}  {shortcut}", action.label()),
        _ => action.label(),
    }
}

/// The bar's row plus a self-drawn tooltip for the hovered button.
///
/// Everything is delegated to the wrapped row. `draw` adds the tooltip: the menu overlay
/// paints its rows in order and never collects their overlays, so the label has to be
/// drawn here, above the bar where no later row can cover it.
struct QuickBar<Message> {
    content: Element<'static, Message>,
    /// One per button of the row, in order.
    tooltips: Vec<String>,
}

impl<Message> QuickBar<Message> {
    /// The row's child whose bounds hold the cursor, with those bounds.
    fn hovered(&self, layout: Layout<'_>, cursor: mouse::Cursor) -> Option<(usize, Rectangle)> {
        let position = cursor.position()?;
        // container -> row -> buttons
        let row = layout.children().next()?;
        row.children()
            .enumerate()
            .find(|(_, child)| child.bounds().contains(position))
            .map(|(index, child)| (index, child.bounds()))
    }

    fn draw_tooltip(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        bar: Rectangle,
        anchor: Rectangle,
        label: &str,
    ) {
        let font = renderer.default_font();
        let text = text::Text {
            content: label.to_string(),
            bounds: Size::INFINITE,
            size: Pixels(TOOLTIP_FONT_SIZE),
            line_height: LineHeight::default(),
            font,
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Center,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::None,
            ellipsize: text::Ellipsize::None,
        };
        let measured =
            <Renderer as text::Renderer>::Paragraph::with_text(text.as_ref()).min_bounds();
        let size = Size::new(
            measured.width + 2.0 * TOOLTIP_PADDING,
            measured.height + 2.0 * TOOLTIP_PADDING,
        );

        // Centred on the button, above the bar; below it when the bar sits at the top edge.
        let x = (anchor.center_x() - size.width / 2.0).max(0.0);
        let above = bar.y - GAP - size.height;
        let y = if above >= 0.0 {
            above
        } else {
            bar.y + bar.height + GAP
        };
        let bounds = Rectangle::new(Point::new(x, y), size);

        let cosmic = theme.cosmic();
        renderer.fill_quad(
            Quad {
                bounds,
                border: Border {
                    radius: cosmic.radius_s().into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            Color::from(cosmic.palette.neutral_2),
        );
        renderer.fill_text(
            text::Text {
                bounds: Size::new(measured.width, measured.height),
                ..text
            },
            Point::new(bounds.x + TOOLTIP_PADDING, bounds.center_y()),
            Color::from(cosmic.on_bg_color()),
            bounds,
        );
    }
}

impl<Message: Clone + 'static> Widget<Message, Theme, Renderer> for QuickBar<Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::stateless()
    }

    fn state(&self) -> tree::State {
        tree::State::None
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &cosmic::iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        if let Some((index, anchor)) = self.hovered(layout, cursor)
            && let Some(label) = self.tooltips.get(index)
        {
            self.draw_tooltip(renderer, theme, layout.bounds(), anchor, label);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<Message: Clone + 'static> From<QuickBar<Message>> for Element<'static, Message> {
    fn from(bar: QuickBar<Message>) -> Self {
        Self::new(bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(context: Context) -> Vec<(QuickAction, bool)> {
        items(context)
            .into_iter()
            .map(|item| (item.action, item.enabled))
            .collect()
    }

    #[test]
    fn empty_space_shows_paste_only_disabled_without_clipboard() {
        assert_eq!(
            actions(Context::default()),
            vec![(QuickAction::Paste, false)]
        );
    }

    #[test]
    fn empty_space_enables_paste_with_clipboard() {
        assert_eq!(
            actions(Context {
                can_paste: true,
                ..Context::default()
            }),
            vec![(QuickAction::Paste, true)]
        );
    }

    #[test]
    fn one_item_shows_cut_copy_paste_rename_in_order() {
        assert_eq!(
            actions(Context {
                selected: 1,
                selected_mount_points: 0,
                can_paste: true,
            }),
            vec![
                (QuickAction::Cut, true),
                (QuickAction::Copy, true),
                (QuickAction::Paste, true),
                (QuickAction::Rename, true),
            ]
        );
    }

    #[test]
    fn several_items_drop_rename() {
        assert_eq!(
            actions(Context {
                selected: 3,
                selected_mount_points: 0,
                can_paste: false,
            }),
            vec![
                (QuickAction::Cut, true),
                (QuickAction::Copy, true),
                (QuickAction::Paste, false),
            ]
        );
    }

    #[test]
    fn mount_point_can_only_be_copied() {
        assert_eq!(
            actions(Context {
                selected: 1,
                selected_mount_points: 1,
                can_paste: false,
            }),
            vec![(QuickAction::Copy, true), (QuickAction::Paste, false)]
        );
    }

    #[test]
    fn carries_reports_present_actions() {
        let bar = items(Context {
            selected: 2,
            selected_mount_points: 0,
            can_paste: false,
        });
        assert!(carries(&bar, QuickAction::Cut));
        assert!(carries(&bar, QuickAction::Paste));
        assert!(!carries(&bar, QuickAction::Rename));
    }

    #[test]
    fn labels_lose_the_ellipsis() {
        crate::localize::localize();
        assert_eq!(QuickAction::Rename.label(), "Rename");
        assert_eq!(QuickAction::Cut.label(), "Cut");
    }

    #[test]
    fn tooltip_names_the_shortcut() {
        crate::localize::localize();
        let key_binds = crate::key_bind::key_binds(&tab::Mode::App);
        let text = tooltip_text(QuickAction::Copy, &key_binds);
        assert!(text.starts_with("Copy"), "{text}");
        assert!(text.len() > "Copy".len(), "no shortcut in {text}");
    }
}
