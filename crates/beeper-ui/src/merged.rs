//! Desktop's merged chats (`unionsStore`, `UnionThreadStore`): the chat that a merged chat sent a message from in its
//! timeline, the Merged Chats section of Chat Info, and Unmerge.

use super::*;
use crate::merge_editor::MergeEdit;
use beeper_core::store::activity_key;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `confirmUnmergeUnion`: "Unmerge Chat?" for a merged chat.
    Ask(String),
    /// Desktop's `removeUnion`; an open merged chat opens its member chat with the newest activity.
    Unmerge(String),
    /// Desktop's `D3`: the editor of a merged chat, or of a new one of the chats.
    Open(Option<String>, Vec<String>),
    Edit(MergeEdit),
}

impl BeeperGuiApp {
    pub(crate) fn merged(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Open(union, chats) => self.open_merge(union, chats),
            Msg::Edit(e) => self.merge_edit(e),
            Msg::Ask(id) => {
                self.list.chat_menu = None;
                (self.confirm, self.delete_target) = (Some(ASK_UNMERGE), Some(id));
                Task::none()
            }
            Msg::Unmerge(id) => {
                self.confirm = None;
                let newest = {
                    let st = self.store.read();
                    let Some(idx) = st.resolve_chat_idx(&id) else { return Task::none() };
                    let newest = st.members(idx).into_iter().max_by_key(|&i| activity_key(&st.chats[i]));
                    newest.map(|i| st.chats[i].id.clone())
                };
                // The member chat opens before the merged chat goes, which leaves its draft.
                if let Some(chat) = newest.filter(|_| self.pane.selected_chat_id.as_deref() == Some(&id)) {
                    self.select_chat(&chat);
                }
                self.store.write().unmerge(&id);
                self.update(UiMessage::StoreChanged)
            }
        }
    }
}

impl BeeperGuiApp {
    /// Desktop's `BrandChatInfoPaneMergedChats` (`R7t`) of a merged chat: "Merged Chats" with Manage, and its member
    /// chats with the default one marked, or a note when none is on this device.
    pub(crate) fn view_merged_section<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        chat.merge.as_ref()?;
        let p = &self.pal;
        let idx = st.chat_idx(&chat.id);
        let manage = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("merged:{}", chat.id)));
        let manage = button(semi(trx("Manage", "Similar to an edit button"), 12.0).color(p.strong))
            .padding([2, 8])
            .style(btn_style([CLEAR, p.chip, p.chip], [p.strong; 3], 999.0))
            .on_press(manage);
        let head = row![txt(tr("Merged Chats"), 12.0).color(p.strong), Space::new().width(Fill), manage];
        let head = container(head.align_y(Center)).padding(8);
        let members = st.members(idx);
        let default = st.default_member(idx);
        let body: El<'a> = match members.is_empty() {
            true => container(txt(tr("This merged chat is empty on this device."), 13.0).color(p.sub))
                .padding(8)
                .width(Fill)
                .style(fill_box(p.chip, 10.0))
                .into(),
            false => {
                let rows = members.into_iter().map(|i| -> El<'a> {
                    let c = &st.chats[i];
                    let name = ell(&c.title, 180.0, 13.0, Weight::Normal).color(p.strong).width(Fill);
                    let mark = (Some(i) == default).then(|| {
                        container(semi(tr("Default"), 10.0).color(p.strong))
                            .padding([2, 4])
                            .style(fill_box(p.card, 4.0))
                    });
                    row![self.chat_avatar(c, 28.0, 1.25), name].push(mark).spacing(8).align_y(Center).padding(4).into()
                });
                container(Column::with_children(rows).spacing(6)).padding(8).style(fill_box(p.chip, 10.0)).into()
            }
        };
        Some(column![head, body].into())
    }
}

/// Desktop's `chatSourceLabel` of a message of the member chat `member` in the merged chat `chat`: the network of
/// the member chat, then its `identifierOfMember` when another member chat is on that network too.
pub(crate) fn source_label(st: &BeeperStore, chat: &Chat, member: &str) -> Option<String> {
    let members: Vec<&Chat> = st.members(st.chat_idx(&chat.id)).into_iter().map(|i| &st.chats[i]).collect();
    let m = members.iter().find(|c| c.id == member)?;
    let network = network_name(&m.account_id);
    let same: Vec<&Chat> = members.iter().copied().filter(|c| network_name(&c.account_id) == network).collect();
    if same.len() < 2 {
        return Some(network.into());
    }
    Some(identifier(st, m, &same).map_or_else(|| network.into(), |id| format!("{network} • {id}")))
}

/// Desktop's `Tdt`: the account of the member chat when the member chats of its network are on several accounts,
/// else the other person of the chat (`g8`).
fn identifier(st: &BeeperStore, m: &Chat, same: &[&Chat]) -> Option<String> {
    if same.iter().any(|c| c.account_id != m.account_id) {
        return Some(account_user(st, &m.account_id));
    }
    let u = m.participants.items.iter().find(|u| u.is_self != Some(true))?;
    let shown = [u.email.clone(), u.phone_number.as_deref().map(fmt_phone), handle(u), u.full_name.clone()];
    shown.into_iter().flatten().find(|s| !s.is_empty()).or_else(|| Some(u.id.clone()))
}

/// The network of a chat as Desktop's `platform.displayName` shows it: "Merged Chat" for a merged chat.
pub(crate) fn chat_platform(c: &Chat) -> &'static str {
    match c.merge {
        Some(_) => tr("Merged Chat"),
        None => network_name(&c.account_id),
    }
}

/// The brand of a chat's network icon: Desktop's `fP` of a merged chat.
pub(crate) fn chat_brand(c: &Chat) -> &'static str {
    match c.merge {
        Some(_) => "merged-chat",
        None => brand_key(&c.account_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::demo_app;
    use iced_test::simulator::simulator;

    const KAI: &str = "!ig_kai:beeper.com";
    const LENA: &str = "!sg_lena:beeper.com";

    /// A demo app with a merged chat of Kai on Instagram and Lena on Signal, open.
    fn merged_app(name: &str) -> (BeeperGuiApp, String) {
        let mut app = demo_app(name);
        let members = Some(vec![KAI.to_string(), LENA.to_string()]);
        let title = Some("Friends".to_string());
        let (chat, _) =
            app.store.write().create_or_start_chat("hungryserv", "single", title, vec![], None, members, None);
        let _ = app.update(UiMessage::SelectChat(chat.id.clone()));
        (app, chat.id)
    }

    /// Desktop's merged chat row and timeline: the member chats leave the list, and the merged chat shows their
    /// messages by time, each run under the network it came from, 12 px apart (`isNextDifferentThread`). The composer
    /// names the member chat with the newest message.
    #[test]
    fn merged_chats_show_their_member_chats_like_desktop() {
        let (app, id) = merged_app("merged-timeline");
        let st = app.store.read();
        let listed: Vec<&str> = app.visible_chats(&st).iter().map(|c| c.id.as_str()).collect();
        assert!(listed.contains(&id.as_str()) && !listed.contains(&KAI) && !listed.contains(&LENA), "{listed:?}");
        let chat = st.chat(&id);
        assert_eq!(chat.preview.as_ref().map(|m| m.id.as_str()), Some("msg_x3"), "Lena's newer message previews it");
        let shown: Vec<&str> = app.pane_msgs(&st).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(shown, ["msg_x4", "msg_x3"], "Kai's message on Oct 11, then Lena's on Oct 12");
        assert_eq!(source_label(&st, chat, KAI).as_deref(), Some("Instagram"));
        assert_eq!(app.composer_placeholder(&st, chat), "Message Lena Fischer on Signal");
        let (kai, lena) = (st.message("msg_x4"), st.message("msg_x3"));
        assert_eq!(crate::message::gap_below(kai, lena), 12.0, "another chat is 12 px below, even on another day");
        drop(st);
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Instagram").is_ok() && sim.find("Signal").is_ok(), "a note over each run");
    }

    /// Desktop's Unmerge in the chat menu, in the place of Delete Chat, and its "Unmerge Chat?" confirmation: the
    /// merged chat goes, the user leaves its room, and the member chat with the newest activity opens.
    #[test]
    fn unmerge_splits_a_merged_chat_like_desktop() {
        let (mut app, id) = merged_app("merged-unmerge");
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(id.clone())));
        let picked: Vec<UiMessage> = {
            let mut sim = simulator(app.view_main());
            assert!(sim.find("Delete Chat").is_err(), "a merged chat has no Delete Chat");
            let _ = sim.click("Unmerge").expect("the Unmerge item");
            sim.into_messages().collect()
        };
        assert!(matches!(&picked[..], [UiMessage::Merged(Msg::Ask(c))] if *c == id));
        let _ = app.update(UiMessage::Merged(Msg::Ask(id.clone())));
        let picked: Vec<UiMessage> = {
            let mut sim = simulator(app.view_main());
            assert!(sim.find("This will split this merged chat back into individual chats.").is_ok());
            assert!(sim.find("Keep chats merged").is_ok());
            let _ = sim.click("Unmerge").expect("the confirm button");
            sim.into_messages().collect()
        };
        assert!(matches!(&picked[..], [UiMessage::Merged(Msg::Unmerge(c))] if *c == id));
        let _ = app.update(UiMessage::Merged(Msg::Unmerge(id.clone())));
        let st = app.store.read();
        assert!(st.find_chat(&id).is_none(), "the merged chat is gone");
        let listed: Vec<&str> = app.visible_chats(&st).iter().map(|c| c.id.as_str()).collect();
        assert!(listed.contains(&KAI) && listed.contains(&LENA), "its member chats are back");
        assert_eq!(app.pane.selected_chat_id.as_deref(), Some(LENA), "the newest member chat opens");
        let leave = |o: &beeper_core::matrix::Outgoing| matches!(o, beeper_core::matrix::Outgoing::Leave(true));
        assert!(st.outbox.iter().any(|(_, c, o)| *c == id && leave(o)), "the user leaves and forgets its room");
    }

    /// Desktop's `notificationSender` and `threadForNotificationReply`: a message of a member chat notifies as the
    /// merged chat, which it opens, and a reply from the notification goes to the member chat.
    #[test]
    fn merged_chats_notify_as_one_chat_like_desktop() {
        let (mut app, id) = merged_app("merged-notify");
        app.focused = true;
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mut m = app.store.read().message("msg_x3").clone();
        (m.id, m.timestamp) = ("$new".into(), "2099-01-01T00:00:00.000Z".into());
        let (note, _) = app.notification(&m).expect("a notification");
        assert_eq!((note.toast.title.as_str(), note.chat.as_str()), ("Friends", id.as_str()));
        let _ = app.update(UiMessage::SelectChat(id.clone()));
        m.id = "$newer".into();
        assert!(app.notification(&m).is_none(), "the open merged chat does not notify");
        app.notifications.insert(7, (id.clone(), "msg_x3".into()));
        let _ = app.notification_event(7, Some(format!("{}on my way", crate::notify::REPLY)));
        let st = app.store.read();
        assert!(st.timeline(LENA).any(|m| m.text.as_deref() == Some("on my way")), "the reply goes to Lena's chat");
    }
}
