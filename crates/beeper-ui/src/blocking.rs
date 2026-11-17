//! Desktop's block and unblock of a direct chat, with Report spam, and the Blocked users window.

use super::*;

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `confirmBlockUser` (true) or `confirmUnblockUser` of a DM.
    Ask(String, bool),
    /// Desktop's `setUserBlockedOrAlert`, as (chat, block, report spam).
    Block(String, bool, bool),
    /// The "Report spam" checkbox of the Block dialog.
    ReportSpam(bool),
    /// Desktop's `BlockedUsersBrowserWindow` from Settings, and its Done button.
    OpenList,
    CloseList,
    /// A row of the Blocked users window: the chat with its Chat Info in the main window.
    OpenUser(String),
}

impl BeeperGuiApp {
    pub(crate) fn blocking(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Ask(id, block) => self.ask_block(id, block),
            Msg::Block(id, block, spam) => return self.block_user(&id, block, spam),
            Msg::ReportSpam(on) => self.report_spam = on,
            Msg::OpenList => return self.open_blocked_users(),
            Msg::CloseList => return window::close(self.blocked_window.expect("Blocked users is open")),
            Msg::OpenUser(id) => return self.open_blocked_user(id),
        }
        self.settle()
    }
}

/// Desktop's `blockedUsersStore.threads`: the chats whose other person is blocked, by title.
pub(crate) fn blocked_chats(st: &BeeperStore) -> Vec<&Chat> {
    let mut chats: Vec<&Chat> = st.chats.iter().filter(|c| st.user_blocked(c)).collect();
    chats.sort_by_cached_key(|c| c.title.to_lowercase());
    chats
}

/// Desktop's Block (true) or Unblock action of a DM, when the bridge supports it.
pub(crate) fn block_action(st: &BeeperStore, c: &Chat) -> Option<bool> {
    match st.user_blocked(c) {
        true => (c.block & NO_UNBLOCK == 0).then_some(false),
        false => st.can_block(c).then_some(true),
    }
}

impl BeeperGuiApp {
    /// Desktop's `confirmBlockUser` (true) and `confirmUnblockUser`: the dialog, with "Report spam" off.
    pub(crate) fn ask_block(&mut self, id: String, block: bool) {
        self.chat_menu = None;
        self.show_command_palette = false;
        self.report_spam = false;
        (self.confirm, self.delete_target) = (Some(if block { ASK_BLOCK } else { ASK_UNBLOCK }), Some(id));
    }

    /// Desktop's `setUserBlockedOrAlert` and `setUserBlocked`: a disconnected account asks to reconnect first. Then
    /// the bridge blocks or unblocks, a toast tells it with Undo when it can be undone, and a blocked request closes.
    pub(crate) fn block_user(&mut self, id: &str, block: bool, spam: bool) -> Task<UiMessage> {
        let mut st = self.store.write();
        let c = st.chat(id);
        if st.bridge_accounts.iter().any(|b| b.account_id == c.account_id && b.state == "BAD_CREDENTIALS") {
            (self.confirm, self.delete_target) =
                (Some(if block { ASK_BLOCK_OFFLINE } else { ASK_UNBLOCK_OFFLINE }), Some(id.into()));
            return Task::none();
        }
        let (title, request) = (c.title.clone(), block && c.is_request == Some(true));
        let can_undo = !block || (!spam && c.block & NO_UNBLOCK == 0);
        self.confirm = None;
        st.set_user_blocked(id, block, spam);
        drop(st);
        // The Undo of a blocked request opens it again, so the chat that is open stays open.
        let open = self.pane.selected_chat_id.as_deref() == Some(id);
        let selected = request && open;
        let mut undo = vec![UiMessage::Blocking(blocking::Msg::Block(id.into(), !block, false))];
        undo.extend(open.then(|| UiMessage::SelectChat(id.into())));
        let text = trf(if block { "%s blocked" } else { "%s unblocked" }, &[&title]);
        match can_undo {
            true => {
                let what = trf(if block { "Block %s" } else { "Unblock %s" }, &[&title]);
                self.act(what, UiMessage::Commands(undo), Some((text, None)));
            }
            false => self.show(Toast { strong: true, ..Toast::new(text) }),
        }
        if selected {
            self.leave_chat(&mut self.store.write(), None);
            self.pane.selected_chat_id = None;
        }
        self.update(UiMessage::StoreChanged)
    }
}
