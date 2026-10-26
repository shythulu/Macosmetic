//! Two tabs side by side in one window.
//!
//! The focused pane is always the active tab. Every shortcut, menu item and paste already acts on
//! `tab_model.active()`, so none of them needs to know about panes. This module only remembers
//! the tab in the other pane and which side it sits on.

use cosmic::iced::core::event::Event;
use cosmic::iced::core::widget::{Operation, Tree, tree};
use cosmic::iced::core::{
    Clipboard, Layout, Length, Rectangle, Shell, Size, Vector, Widget, layout, mouse, overlay,
    renderer, touch,
};
use cosmic::widget::segmented_button::Entity;
use cosmic::{Element, Renderer, Theme};

#[derive(Clone, Debug)]
pub enum Message {
    /// Show the active tab beside another one, or go back to a single pane.
    Toggle,
    /// A press landed in this tab's pane.
    Focus(Entity),
    /// The pointer moved into this tab's pane. Gestures act on the pane under the pointer.
    Hover(Entity),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// The pane that does not have focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitView {
    pub other: Entity,
    pub other_side: Side,
}

impl SplitView {
    /// The two panes as (left, right), given the tab that has focus.
    pub fn panes(&self, focused: Entity) -> (Entity, Entity) {
        match self.other_side {
            Side::Left => (self.other, focused),
            Side::Right => (focused, self.other),
        }
    }

    /// Hand focus to the other pane. `focused` moves into the other slot and keeps its side.
    pub fn swap(&mut self, focused: Entity) {
        self.other = focused;
        self.other_side = match self.other_side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        };
    }
}

/// Emit `on_press` on any mouse or touch press inside `content`, and `on_enter` when the pointer
/// moves into it.
///
/// `mouse_area` only reports presses its children did not handle, so a click on a file row or a
/// path-bar button never reaches it. This widget reports first, then passes the event on
/// untouched.
pub fn pane<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    on_press: Message,
    on_enter: Message,
) -> Element<'a, Message> {
    Element::new(Pane {
        content: content.into(),
        on_press,
        on_enter,
    })
}

struct Pane<'a, Message> {
    content: Element<'a, Message>,
    on_press: Message,
    on_enter: Message,
}

/// Whether the pointer was over the pane at the last event.
#[derive(Default)]
struct Hovered(bool);

impl<Message: Clone> Widget<Message, Theme, Renderer> for Pane<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Hovered>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Hovered::default())
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
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let over = cursor.is_over(layout.bounds());
        let hovered = tree.state.downcast_mut::<Hovered>();
        if over && !hovered.0 {
            shell.publish(self.on_enter.clone());
        }
        hovered.0 = over;

        let pressed = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(_))
                | Event::Touch(touch::Event::FingerPressed { .. })
        );
        // Published before the content's own messages, so the pane has focus by the time the
        // click on a row or button is handled.
        if pressed && over {
            shell.publish(self.on_press.clone());
        }
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

    fn drag_destinations(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        dnd_rectangles: &mut cosmic::iced::core::clipboard::DndDestinationRectangles,
    ) {
        self.content.as_widget().drag_destinations(
            &tree.children[0],
            layout,
            renderer,
            dnd_rectangles,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::widget::segmented_button::{ModelBuilder, SingleSelect};

    fn two_tabs() -> (Entity, Entity) {
        let mut model = ModelBuilder::<SingleSelect>::default().build();
        let a = model.insert().id();
        let b = model.insert().id();
        (a, b)
    }

    #[test]
    fn panes_put_the_other_tab_on_its_side() {
        let (a, b) = two_tabs();
        let right = SplitView {
            other: b,
            other_side: Side::Right,
        };
        assert_eq!(right.panes(a), (a, b));
        let left = SplitView {
            other: b,
            other_side: Side::Left,
        };
        assert_eq!(left.panes(a), (b, a));
    }

    #[test]
    fn swapping_focus_keeps_each_tab_on_its_side() {
        let (a, b) = two_tabs();
        let mut split = SplitView {
            other: b,
            other_side: Side::Right,
        };
        // a is focused on the left; focus moves to b on the right.
        split.swap(a);
        assert_eq!(split.panes(b), (a, b));
        split.swap(b);
        assert_eq!(split.panes(a), (a, b));
    }
}
