//! Desktop's BrandUserHoverCard: the card of a user that shows after the pointer rests on their name or avatar.

use super::*;

#[derive(Clone)]
pub enum Msg {
    /// The pointer came onto (`Some`, "chat user") or left a trigger of Desktop's `BrandUserHoverCard`.
    Hover(Option<String>),
    /// The pointer came onto or left the shown user card.
    HoverCard(bool),
    /// The wait of this generation of the user card ran out.
    Due(u32),
}

impl BeeperGuiApp {
    pub(crate) fn user_card(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Hover(key) => {
                let h = &mut self.user_hover;
                match key {
                    Some(k) if h.key != k => *h = UserHover { key: k, trigger: true, r#gen: h.r#gen, ..UserHover::default() },
                    Some(_) => h.trigger = true,
                    None => h.trigger = false,
                }
                return self.settle_user_hover();
            }
            Msg::HoverCard(on) => {
                self.user_hover.card = on;
                return self.settle_user_hover();
            }
            Msg::Due(n) if n == self.user_hover.r#gen => {
                let h = &mut self.user_hover;
                h.shown = (h.trigger || h.card).then(|| h.shown.unwrap_or(self.cursor.get()));
            }
            Msg::Due(_) => {}
        }
        self.settle()
    }
}

/// Desktop's `HoverCard` of a user (`hoverDelay: 300`): the card of "chat user" `key` shows where the pointer is 300 ms
/// after it rests on a trigger, and stays while the pointer is on the trigger or the card.
#[derive(Default)]
pub struct UserHover {
    pub key: String,
    pub trigger: bool,
    pub card: bool,
    pub shown: Option<Point>,
    /// Counts the changes, so that only the last wait acts.
    pub r#gen: u32,
}

impl BeeperGuiApp {
    /// The user card of `user_hover` over the open chat, which keeps the pointer from the chat under it.
    pub(crate) fn view_hover_card(&self) -> Option<El<'_>> {
        let h = &self.user_hover;
        let at = h.shown?;
        let (cid, uid) = h.key.split_once(' ')?;
        let st = self.store.read();
        let c = &st.chats[st.resolve_chat_idx(cid)?];
        let shown = self.pane().selected_chat_id.as_deref() == Some(c.id.as_str());
        (shown && c.participants.items.iter().any(|u| u.id == uid)).then_some(())?;
        drop(st);
        let (card, high) = self.view_user_card(&h.key);
        let (x, y) = self.card_spot(at, 12.0, high);
        let card = mouse_area(card)
            .on_enter(UiMessage::UserCard(user_card::Msg::HoverCard(true)))
            .on_exit(UiMessage::UserCard(user_card::Msg::HoverCard(false)))
            .interaction(mouse::Interaction::Idle);
        Some(container(card).padding(Padding::ZERO.left(x).top(y)).into())
    }
}

impl BeeperGuiApp {
    /// A trigger of Desktop's `BrandUserHoverCard`: the card of `uid`, a member of `chat`, shows while the pointer
    /// rests on `el`.
    pub(crate) fn user_trigger<'a>(&self, chat: &Chat, uid: &str, el: impl Into<El<'a>>) -> El<'a> {
        if !chat.participants.items.iter().any(|u| u.id == uid) {
            return el.into();
        }
        mouse_area(el)
            .on_enter(UiMessage::UserCard(user_card::Msg::Hover(Some(format!("{} {uid}", chat.id)))))
            .on_exit(UiMessage::UserCard(user_card::Msg::Hover(None)))
            .into()
    }
}
