//! The scroll of the timeline: the jump to its bottom, and the load of older messages at its top.

use super::*;

#[derive(Clone)]
pub enum Msg {
    /// The timeline moved, and is above its bottom (true) or at it.
    Scrolled(bool),
    ScrollToBottom,
    /// The top of the timeline came on screen (true) or went off screen.
    TopShown(bool),
    LoadOlder,
    /// A load of older messages ended.
    LoadedOlder(Result<(), String>),
}

impl BeeperGuiApp {
    pub(crate) fn timeline_scroll(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Scrolled(up) => {
                self.timeline_scrolled(up);
                self.read_open_chat();
            }
            Msg::ScrollToBottom => {
                return operation::scroll_to(self.pane.scroll_id(), operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            Msg::TopShown(shown) => {
                self.pane.top.shown = shown;
                return self.load_older();
            }
            Msg::LoadOlder => {
                self.pane.top.waiting = false;
                return self.load_older();
            }
            Msg::LoadedOlder(r) => return self.loaded_older(r),
        }
        self.settle()
    }
}
