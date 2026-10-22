//! Wraps the query editor and reports where its caret is on screen, so the
//! autocomplete popup can be drawn next to it.
//!
//! Iced's text editor doesn't expose the caret position, but it does tell
//! the runtime where to put an input-method (IME) window on every redraw.
//! That rectangle is exactly the caret, in window coordinates.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Operation, Tree, Widget};
use iced::advanced::{Clipboard, InputMethod, Shell, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Vector, window};

pub struct Caret<'a, Message> {
    content: Element<'a, Message>,
    /// The caret position the app last heard about.
    known: Option<Rectangle>,
    on_change: Box<dyn Fn(Rectangle) -> Message + 'a>,
}

pub fn caret<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    known: Option<Rectangle>,
    on_change: impl Fn(Rectangle) -> Message + 'a,
) -> Caret<'a, Message> {
    Caret { content: content.into(), known, on_change: Box::new(on_change) }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Caret<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // Only a request made by our editor counts, not one from earlier in the tree.
        let unclaimed = !shell.input_method().is_enabled();
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
        if !matches!(event, Event::Window(window::Event::RedrawRequested(_))) || !unclaimed {
            return;
        }
        if let InputMethod::Enabled { cursor, .. } = shell.input_method()
            && self.known != Some(*cursor)
        {
            let rect = *cursor;
            shell.publish((self.on_change)(rect));
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(tree, layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Caret<'a, Message>> for Element<'a, Message> {
    fn from(caret: Caret<'a, Message>) -> Self {
        Element::new(caret)
    }
}
