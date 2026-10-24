// SPDX-License-Identifier: GPL-3.0-only

//! The quick-action bar at the top of a context menu: a row of icon buttons for the
//! actions people reach for most (cut, copy, paste, rename), as in the Windows 11 File
//! Explorer menu.
//!
//! [`items`] decides which buttons the bar shows for a selection and clipboard state. It
//! is a pure function so the rules can be tested without a window. [`view`] turns that
//! list into the element the menu holds as its first row. [`row_view`] is the row widget
//! itself, shared with the Open row and the tag row. The menu widget never asks its
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
pub fn row(items: &[Item], key_binds: &HashMap<KeyBind, Action>) -> Row {
    let buttons = items
        .iter()
        .map(|item| {
            let action = item.action.action();
            RowButton {
                icon: widget::icon::from_name(item.action.icon_name())
                    .size(16)
                    .handle(),
                tooltip: tooltip(item.action.label(), action, key_binds),
                on_press: item.enabled.then_some(tab::Message::ContextAction(action)),
            }
        })
        .collect();
    Row::new(buttons)
}

/// One icon button of a menu row.
pub struct RowButton {
    pub icon: widget::icon::Handle,
    /// Shown above the row while the button is hovered.
    pub tooltip: String,
    /// `None` draws the button dimmed and inert.
    pub on_press: Option<tab::Message>,
}

/// A row of icon buttons for the top of a context menu.
pub struct Row {
    buttons: Vec<RowButton>,
    /// Space around each icon inside its button.
    padding: u16,
}

impl Row {
    pub fn new(buttons: Vec<RowButton>) -> Self {
        Self {
            buttons,
            padding: cosmic::theme::spacing().space_xxs,
        }
    }

    /// Tighter buttons, for a row with more buttons than the menu is wide for at the normal
    /// size: the tag row's seven dots.
    pub fn compact(buttons: Vec<RowButton>) -> Self {
        Self {
            buttons,
            padding: cosmic::theme::spacing().space_xxxs,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buttons.is_empty()
    }
}

/// The icon rows as one menu entry, with a self-drawn tooltip for the hovered button.
///
/// The rows are one widget, not one menu entry each, so the tooltip is painted after every
/// row. With separate entries, a tooltip reaching into the next row was painted over by it.
pub fn stack_view(rows: Vec<Row>) -> Element<'static, tab::Message> {
    let mut column = widget::column::with_capacity(rows.len());
    let mut tooltips = Vec::with_capacity(rows.len());
    for row in rows {
        let (content, labels) = row_content(row);
        column = column.push(content);
        tooltips.push(labels);
    }
    QuickBar {
        content: column.into(),
        tooltips,
    }
    .into()
}

fn row_content(row: Row) -> (Element<'static, tab::Message>, Vec<String>) {
    let padding = row.padding;
    let mut buttons = widget::row::with_capacity(row.buttons.len())
        .spacing(GAP as u16)
        .align_y(alignment::Vertical::Center);
    let mut tooltips = Vec::with_capacity(row.buttons.len());
    for button in row.buttons {
        match button.on_press {
            Some(message) => {
                let icon = widget::icon::icon(button.icon).size(16);
                buttons = buttons.push(
                    widget::button::custom(icon)
                        .class(cosmic::theme::Button::Icon)
                        .padding(padding)
                        .on_press(message),
                );
            }
            None => {
                // The icon widget ignores the button's disabled colour, so dim it here.
                let icon = widget::icon::icon(button.icon)
                    .size(16)
                    .opacity(DISABLED_OPACITY);
                buttons = buttons.push(
                    widget::button::custom(icon)
                        .class(cosmic::theme::Button::Icon)
                        .padding(padding),
                );
            }
        }
        tooltips.push(button.tooltip);
    }
    let content = widget::container(buttons)
        .padding([0, BAR_INSET])
        .height(Length::Fixed(BAR_HEIGHT))
        .width(Length::Fill)
        .align_y(alignment::Vertical::Center);
    (content.into(), tooltips)
}

/// Where a tooltip of `size` goes, for the button `anchor` in row `hovered` of a stack of
/// rows whose bounds are `rows`. The stack is painted before the tooltip, so covering other
/// rows is fine; covering what comes after the stack is not, and neither is leaving it at
/// the top, where a popup window ends.
///
/// 1. Any row but the first: above the row, centred on the button.
/// 2. The first of several rows: below it, centred on the button.
/// 3. A row on its own: beside the button, on the right, or on the left if it does not fit.
fn tooltip_origin(rows: &[Rectangle], hovered: usize, anchor: Rectangle, size: Size) -> Point {
    let stack = rows
        .iter()
        .copied()
        .reduce(|a, b| a.union(&b))
        .unwrap_or(anchor);
    let row = rows.get(hovered).copied().unwrap_or(anchor);
    let centred_x = (anchor.center_x() - size.width / 2.0)
        .min(stack.x + stack.width - size.width)
        .max(stack.x);
    if hovered > 0 {
        Point::new(centred_x, row.y - GAP - size.height)
    } else if rows.len() > 1 {
        Point::new(centred_x, row.y + row.height + GAP)
    } else {
        let right = anchor.x + anchor.width + GAP;
        let x = if right + size.width <= stack.x + stack.width {
            right
        } else {
            (anchor.x - GAP - size.width).max(stack.x)
        };
        Point::new(x, row.center_y() - size.height / 2.0)
    }
}

/// "Cut  ⌘X": a button's name and, when `target` is bound, its shortcut in menu notation.
pub fn tooltip(label: String, target: Action, key_binds: &HashMap<KeyBind, Action>) -> String {
    let shortcut = key_binds
        .iter()
        .filter(|(_, bound)| **bound == target)
        .map(|(key_bind, _)| menu_label(key_bind))
        .min();
    match shortcut {
        Some(shortcut) if !shortcut.is_empty() => format!("{label}  {shortcut}"),
        _ => label,
    }
}

/// The stacked rows plus a self-drawn tooltip for the hovered button.
///
/// Everything is delegated to the wrapped column. `draw` adds the tooltip: the menu never
/// collects its rows' overlays, so the label is drawn here, after every row.
struct QuickBar<Message> {
    content: Element<'static, Message>,
    /// Per row, one per button, in order.
    tooltips: Vec<Vec<String>>,
}

impl<Message> QuickBar<Message> {
    /// The button under the cursor: its row, its index in the row, and its bounds.
    fn hovered(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) -> Option<(usize, usize, Rectangle)> {
        let position = cursor.position()?;
        // column -> container per row -> row -> buttons
        layout.children().enumerate().find_map(|(row, container)| {
            container
                .children()
                .next()?
                .children()
                .enumerate()
                .find(|(_, button)| button.bounds().contains(position))
                .map(|(index, button)| (row, index, button.bounds()))
        })
    }

    fn draw_tooltip(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        rows: &[Rectangle],
        hovered: usize,
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

        let bounds = Rectangle::new(tooltip_origin(rows, hovered, anchor, size), size);

        // The theme's primary container, as a popover would use, solid even when the
        // theme is transparent: the opaque variant, with alpha forced to 1.
        let component = &theme.cosmic().primary(false).component;
        let opaque = |colour| Color {
            a: 1.0,
            ..Color::from(colour)
        };
        // A layer of its own: within one layer iced draws every quad before any icon, so
        // a tooltip background in the rows' layer sat under the buttons' icons.
        renderer.with_layer(bounds, |renderer| {
            renderer.fill_quad(
                Quad {
                    bounds,
                    border: Border {
                        radius: theme.cosmic().radius_s().into(),
                        width: 1.0,
                        color: opaque(component.divider),
                    },
                    ..Default::default()
                },
                opaque(component.base),
            );
            renderer.fill_text(
                text::Text {
                    bounds: Size::new(measured.width, measured.height),
                    ..text
                },
                Point::new(bounds.x + TOOLTIP_PADDING, bounds.center_y()),
                opaque(component.on),
                bounds,
            );
        });
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
        if let Some((row, index, anchor)) = self.hovered(layout, cursor)
            && let Some(label) = self.tooltips.get(row).and_then(|row| row.get(index))
        {
            let rows: Vec<Rectangle> = layout.children().map(|row| row.bounds()).collect();
            self.draw_tooltip(renderer, theme, &rows, row, anchor, label);
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

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(width, height))
    }

    /// Three 40 px rows, 300 wide, as in a file's context menu.
    fn three_rows() -> Vec<Rectangle> {
        (0..3)
            .map(|i| rect(0.0, 40.0 * i as f32, 300.0, 40.0))
            .collect()
    }

    const TIP: Size = Size::new(60.0, 30.0);

    #[test]
    fn tooltip_of_a_lower_row_goes_above_it() {
        let at = tooltip_origin(&three_rows(), 1, rect(100.0, 48.0, 24.0, 24.0), TIP);
        assert_eq!(at, Point::new(82.0, 40.0 - GAP - 30.0));
    }

    #[test]
    fn tooltip_of_the_top_row_goes_below_it_when_rows_follow() {
        let at = tooltip_origin(&three_rows(), 0, rect(100.0, 8.0, 24.0, 24.0), TIP);
        assert_eq!(at, Point::new(82.0, 40.0 + GAP));
    }

    #[test]
    fn tooltip_stays_inside_the_stack_horizontally() {
        let at = tooltip_origin(&three_rows(), 2, rect(2.0, 88.0, 24.0, 24.0), TIP);
        assert_eq!(at.x, 0.0);
        let at = tooltip_origin(&three_rows(), 2, rect(290.0, 88.0, 24.0, 24.0), TIP);
        assert_eq!(at.x, 240.0);
    }

    #[test]
    fn tooltip_of_a_lone_row_goes_beside_the_button() {
        let row = [rect(0.0, 0.0, 300.0, 40.0)];
        let at = tooltip_origin(&row, 0, rect(100.0, 8.0, 24.0, 24.0), TIP);
        assert_eq!(at, Point::new(124.0 + GAP, 5.0));
    }

    #[test]
    fn tooltip_of_a_lone_row_flips_left_at_the_edge() {
        let row = [rect(0.0, 0.0, 300.0, 40.0)];
        let at = tooltip_origin(&row, 0, rect(250.0, 8.0, 24.0, 24.0), TIP);
        assert_eq!(at, Point::new(250.0 - GAP - 60.0, 5.0));
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
        let text = tooltip(QuickAction::Copy.label(), Action::Copy, &key_binds);
        assert!(text.starts_with("Copy"), "{text}");
        assert!(text.len() > "Copy".len(), "no shortcut in {text}");
    }
}
