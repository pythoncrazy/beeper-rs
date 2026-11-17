//! The chat accounts: Settings > Accounts, an account's settings and Show in Inbox, its removal, and its error banner.

use super::*;

#[derive(Clone)]
pub enum Msg {
    ShowAppQr,
    Open(Option<String>),
    /// Desktop's `openManageAccountInNewWindow`.
    OpenSettings(String),
    SetProps(String, beeper_core::models::AccountProps),
    /// "Show in Inbox": the `chat_list_rules` key and option.
    SetInboxVisibility(String, &'static str),
    /// The Beeper account profile and the chat accounts from `whoami`.
    Cloud(Result<(Profile, Vec<BridgeAccountInfo>), String>),
    /// The dismiss button of the disconnected banner of an account.
    DismissError(String),
    AskRemove(String),
    Remove(String),
    Removed(String, Result<(), String>),
}

impl BeeperGuiApp {
    pub(crate) fn accounts(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Cloud(Ok((profile, accounts))) => {
                let (token, uid, demo) = {
                    let mut st = self.store.write();
                    let email = profile.email.clone();
                    st.features.subscribed = profile.plan != Plan::Free;
                    st.features.internal = profile.internal;
                    st.whoami = profile;
                    let info = (st.beeper_state.access_token.clone(), st.beeper_state.user_id.clone(), st.config.demo);
                    if st.me().email != email {
                        let i = st.chat_idx(NOTE_CHAT_ID);
                        st.update_chat(i, |c| c.participants.items[0].email = email);
                    }
                    st.set_cloud_accounts(accounts.clone());
                    info
                };
                if !demo {
                    let connected = accounts.iter().filter(|a| a.state == "CONNECTED");
                    let caps: Vec<_> = connected
                        .map(|a| {
                            let id = a.account_id.clone();
                            Task::perform(beeper_core::matrix::capabilities(self.provision(&id)), move |r| {
                                UiMessage::Capabilities(id.clone(), r)
                            })
                        })
                        .collect();
                    let cloud = self.cloud.clone();
                    let contacts = Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || cloud.contacts(&token, &uid, &accounts))
                                .await
                                .expect("contacts worker thread")
                        },
                        |a0| UiMessage::NewChat(new_chat::Msg::ContactsLoaded(a0)),
                    );
                    return Task::batch(caps).chain(contacts);
                }
            }
            Msg::Cloud(Err(e)) => self.fail(Toast::new(format!("Could not load your chat accounts: {e}"))),
            Msg::DismissError(id) => _ = self.dismissed_errors.insert(id),
            Msg::ShowAppQr => self.app_qr = true,
            Msg::OpenSettings(id) => {
                self.chat_menu = None;
                let open = self.open_settings(SettingsTab::Accounts);
                self.account_page = Some(id);
                return open;
            }
            Msg::Open(id) => {
                self.account_page = id;
                return operation::scroll_to("settings-pane", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            Msg::AskRemove(id) => {
                (self.chat_menu, self.delete_target, self.confirm) = (None, Some(id), Some(ASK_REMOVE_ACCOUNT));
            }
            Msg::Remove(id) => {
                self.confirm = None;
                let p = self.provision(&id);
                let st = self.store.read();
                let sole = st.bridge_accounts.iter().filter(|b| b.bridge_id == p.bridge_id).all(|b| b.account_id == id);
                let removing = beeper_core::matrix::remove_account(p, st.config.api_url.clone(), sole);
                return Task::perform(removing, move |r| UiMessage::Accounts(Msg::Removed(id.clone(), r)));
            }
            Msg::Removed(_, Ok(())) => return self.refresh_cloud_accounts(),
            Msg::Removed(id, Err(e)) => {
                let text = format!("There was an error disconnecting account {id}. {e}");
                let report = UiMessage::OpenUrl(ISSUES_URL.into());
                self.show(Toast { buttons: vec![("Report a problem", report)], ..Toast::new(text) });
            }
            Msg::SetProps(id, props) => {
                self.chat_menu = None;
                self.patch_settings(|s| {
                    s.accounts.insert(id, props);
                })
            }
            Msg::SetInboxVisibility(key, option) => {
                self.chat_menu = None;
                let st = self.store.read();
                let client = st.matrix.clone().expect("Show in Inbox needs a Matrix client");
                let rules = beeper_core::matrix::with_inbox_visibility(st.inbox_rules.clone(), &key, option);
                return Task::perform(
                    beeper_core::matrix::set_global(client, beeper_core::matrix::RULES, rules),
                    UiMessage::Pushed,
                );
            }
        }
        self.settle()
    }
}
