/// The `Widget` methods of a wrapper widget that only pass the call to its `content`, the first child of its tree.
/// `delegate!(children, diff, size)` in an `impl Widget` writes them, and `update` and `overlay` take the message
/// type, as in `delegate!(overlay(Message))`. `state(T)` writes `tag` and `state` for a tree state of `T::default()`.
/// The widget writes the methods that do more itself.
macro_rules! delegate {
    ($($method:ident $(($msg:ty))?),* $(,)?) => {
        $($crate::delegate::delegate!(@ $method $(($msg))?);)*
    };
    (@ state($state:ty)) => {
        fn tag(&self) -> iced::advanced::widget::tree::Tag {
            iced::advanced::widget::tree::Tag::of::<$state>()
        }

        fn state(&self) -> iced::advanced::widget::tree::State {
            iced::advanced::widget::tree::State::new(<$state>::default())
        }
    };
    (@ children) => {
        fn children(&self) -> Vec<iced::advanced::widget::Tree> {
            vec![iced::advanced::widget::Tree::new(&self.content)]
        }
    };
    (@ diff) => {
        fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
            tree.diff_children(std::slice::from_ref(&self.content));
        }
    };
    (@ size) => {
        fn size(&self) -> iced::Size<iced::Length> {
            self.content.as_widget().size()
        }
    };
    (@ layout) => {
        fn layout(
            &mut self,
            tree: &mut iced::advanced::widget::Tree,
            renderer: &iced::Renderer,
            limits: &iced::advanced::layout::Limits,
        ) -> iced::advanced::layout::Node {
            self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
        }
    };
    (@ operate) => {
        fn operate(
            &mut self,
            tree: &mut iced::advanced::widget::Tree,
            layout: iced::advanced::Layout<'_>,
            renderer: &iced::Renderer,
            operation: &mut dyn iced::advanced::widget::Operation,
        ) {
            self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
        }
    };
    (@ update($msg:ty)) => {
        fn update(
            &mut self,
            tree: &mut iced::advanced::widget::Tree,
            event: &iced::Event,
            layout: iced::advanced::Layout<'_>,
            cursor: iced::advanced::mouse::Cursor,
            renderer: &iced::Renderer,
            clipboard: &mut dyn iced::advanced::Clipboard,
            shell: &mut iced::advanced::Shell<'_, $msg>,
            viewport: &iced::Rectangle,
        ) {
            let tree = &mut tree.children[0];
            self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
        }
    };
    (@ mouse_interaction) => {
        fn mouse_interaction(
            &self,
            tree: &iced::advanced::widget::Tree,
            layout: iced::advanced::Layout<'_>,
            cursor: iced::advanced::mouse::Cursor,
            viewport: &iced::Rectangle,
            renderer: &iced::Renderer,
        ) -> iced::advanced::mouse::Interaction {
            self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
        }
    };
    (@ draw) => {
        fn draw(
            &self,
            tree: &iced::advanced::widget::Tree,
            renderer: &mut iced::Renderer,
            theme: &iced::Theme,
            style: &iced::advanced::renderer::Style,
            layout: iced::advanced::Layout<'_>,
            cursor: iced::advanced::mouse::Cursor,
            viewport: &iced::Rectangle,
        ) {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        }
    };
    (@ overlay($msg:ty)) => {
        fn overlay<'b>(
            &'b mut self,
            tree: &'b mut iced::advanced::widget::Tree,
            layout: iced::advanced::Layout<'b>,
            renderer: &iced::Renderer,
            viewport: &iced::Rectangle,
            translation: iced::Vector,
        ) -> Option<iced::advanced::overlay::Element<'b, $msg, iced::Theme, iced::Renderer>> {
            self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
        }
    };
}

pub(crate) use delegate;
