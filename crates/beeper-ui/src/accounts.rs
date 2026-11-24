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
    /// Desktop's "Message yourself" of an account (`createThread(account.user)`).
    MessageSelf(String),
    /// Desktop's "Send a link to %s" of Settings > Devices on an account.
    SendLink(String),
    /// The dropdown of "Send a link to %s" with several phone numbers.
    ToggleLinkMenu,
    /// Desktop's `fetchSubAccounts` of an account: its "Refresh List", and the servers that the bridge sent.
    FetchServers(String),
    Servers(String, Result<Vec<beeper_core::matrix::SubAccount>, String>),
    /// Desktop's `toggleSubAccountConnection`: the switch of a server, which asks before it disconnects a Discord
    /// server, then the change and its reply.
    ToggleServer(String, String),
    SetServer(String, String),
    ServerSet(String, String, bool, Result<(), String>),
    /// A frame of the spinning Refresh List.
    Spin,
}

/// Desktop's `SUPPORTS_SUB_ACCOUNTS`, which only the Discord platform has, and its cloud bridge's `getSubAccounts`.
pub(crate) fn has_servers(st: &BeeperStore, account_id: &str) -> bool {
    let b = st.bridge_accounts.iter().find(|b| b.account_id == account_id);
    b.is_some_and(|b| {
        b.mode == BridgeMode::Cloud && beeper_core::config::find_network(&b.bridge_id).is_some_and(|n| n.kind == NetworkKind::Discord)
    })
}

/// How long the demo's Discord bridge takes to answer, so that the demo shows the spinning Refresh List.
const DEMO_BRIDGE_DELAY: Duration = Duration::from_millis(700);

impl BeeperGuiApp {
    pub(crate) fn accounts(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Cloud(Ok((profile, accounts))) => {
                let (token, uid, demo) = {
                    let mut st = self.store.write();
                    let email = profile.email.clone();
                    st.features.subscribed = profile.plan != Plan::Free;
                    st.features.internal = profile.internal;
                    st.features.whoami.clone_from(&profile.features);
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
                    // Desktop's `fetchSubAccounts` when the remote of an account connects.
                    let st = self.store.read();
                    let servers: Vec<String> =
                        connected.clone().filter(|a| has_servers(&st, &a.account_id)).map(|a| a.account_id.clone()).collect();
                    drop(st);
                    let servers: Vec<_> = servers.iter().map(|a| self.fetch_servers(a)).collect();
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
                    return Task::batch(caps.into_iter().chain(servers)).chain(contacts);
                }
            }
            Msg::Cloud(Err(e)) => self.fail(Toast::new(format!("Could not load your chat accounts: {e}"))),
            Msg::DismissError(id) => _ = self.dismissed_errors.insert(id),
            Msg::MessageSelf(account) => return self.message_self(account, false),
            Msg::SendLink(account) => {
                self.link_pick = (Some(account.clone()), false);
                let send = self.message_self(account, true);
                // Desktop's "Link sent to %s" goes back to "Send a link to %s" after 3 s.
                let back = Task::perform(async { tokio::time::sleep(Duration::from_secs(3)).await }, |()| UiMessage::Noop);
                return Task::batch([send, back]);
            }
            Msg::ToggleLinkMenu => self.link_pick.1 ^= true,
            Msg::ShowAppQr => self.app_qr = true,
            Msg::OpenSettings(id) => {
                self.list.chat_menu = None;
                let open = self.open_settings(SettingsTab::Accounts);
                let servers = self.page_servers(&id);
                self.account_page = Some(id);
                return open.chain(servers);
            }
            Msg::Open(id) => {
                let servers = id.as_deref().map_or_else(Task::none, |id| self.page_servers(id));
                self.account_page = id;
                let top = operation::scroll_to("settings-pane", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                return top.chain(servers);
            }
            Msg::AskRemove(id) => {
                (self.list.chat_menu, self.delete_target, self.confirm) = (None, Some(id), Some(ASK_REMOVE_ACCOUNT));
            }
            Msg::Remove(id) => {
                self.confirm = None;
                let p = self.provision(&id);
                let mut st = self.store.write();
                // The demo has no Beeper Cloud to ask: its account goes from the store alone.
                if st.config.demo {
                    let left = st.bridge_accounts.iter().filter(|b| b.account_id != id).cloned().collect();
                    st.set_cloud_accounts(left);
                } else {
                    let sole = st.bridge_accounts.iter().filter(|b| b.bridge_id == p.bridge_id).all(|b| b.account_id == id);
                    let removing = beeper_core::matrix::remove_account(p, st.config.api_url.clone(), sole);
                    return Task::perform(removing, move |r| UiMessage::Accounts(Msg::Removed(id.clone(), r)));
                }
            }
            Msg::Removed(_, Ok(())) => return self.refresh_cloud_accounts(),
            Msg::Removed(id, Err(e)) => {
                let text = format!("There was an error disconnecting account {id}. {e}");
                let report = UiMessage::OpenUrl(ISSUES_URL.into());
                self.show(Toast { buttons: vec![("Report a problem", report)], ..Toast::new(text) });
            }
            Msg::SetProps(id, props) => {
                self.list.chat_menu = None;
                self.patch_settings(|s| {
                    s.accounts.insert(id, props);
                })
            }
            msg @ (Msg::FetchServers(..)
            | Msg::Servers(..)
            | Msg::ToggleServer(..)
            | Msg::SetServer(..)
            | Msg::ServerSet(..)
            | Msg::Spin
            | Msg::SetInboxVisibility(..)) => return self.accounts_servers(msg),
        }
        self.settle()
    }

    /// The Discord servers of an account and their inbox visibility.
    fn accounts_servers(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::FetchServers(id) => return self.fetch_servers(&id),
            Msg::Servers(id, result) => {
                self.servers_loading.remove(&id);
                match result {
                    Ok(servers) => self.store.write().set_sub_accounts(&id, servers),
                    // Desktop's `fetchSubAccounts` only logs the error, and keeps the servers that it had.
                    Err(e) => eprintln!("servers of {id}: {e}"),
                }
            }
            Msg::ToggleServer(id, server) => {
                let st = self.store.read();
                let found = st.sub_accounts.get(&id).and_then(|l| l.iter().find(|s| s.id == server));
                // The bridge may have sent a list without the server since the switch showed.
                let Some(connected) = found.map(|s| s.connected) else {
                    drop(st);
                    return self.settle();
                };
                let discord = st.bridge_accounts.iter().any(|b| b.account_id == id && b.bridge_type == "discordgo");
                drop(st);
                if connected && discord {
                    (self.server_target, self.confirm) = (Some((id, server)), Some(ASK_DISCONNECT_SERVER));
                    return self.settle();
                }
                return self.accounts(Msg::SetServer(id, server));
            }
            Msg::SetServer(id, server) => {
                self.confirm = None;
                let mut st = self.store.write();
                let mut servers = st.sub_accounts.get(&id).cloned().unwrap_or_default();
                let Some(s) = servers.iter_mut().find(|s| s.id == server) else {
                    drop(st);
                    return self.settle();
                };
                let (connect, name) = (!s.connected, s.name.clone());
                // Desktop turns the switch at once, so the space comes and goes before the bridge answers.
                s.connected = connect;
                st.set_sub_accounts(&id, servers);
                let demo = st.config.demo;
                if demo {
                    let i = st.demo_servers.iter().position(|s| s.id == server).expect("a demo server");
                    let s = &mut st.demo_servers[i];
                    (s.connected, s.mxid) = (connect, connect.then(|| format!("!{server}:beeper.local")));
                }
                drop(st);
                let text = match connect {
                    true => trf("Connecting %s...", &[&name]),
                    false => trf("Disconnecting %s...", &[&name]),
                };
                self.show(Toast { stays: true, id: Some(format!("server-{server}")), ..Toast::new(text) });
                let (p, sent) = (self.provision(&id), server.clone());
                let change = async move {
                    match demo {
                        true => {
                            tokio::time::sleep(DEMO_BRIDGE_DELAY).await;
                            Ok(())
                        }
                        false => beeper_core::matrix::connect_sub_account(p, sent, connect).await,
                    }
                };
                let done = move |r| UiMessage::Accounts(Msg::ServerSet(id.clone(), server.clone(), connect, r));
                return Task::perform(change, done);
            }
            Msg::ServerSet(id, server, connect, result) => {
                let name = self.store.read().sub_accounts.get(&id).and_then(|l| l.iter().find(|s| s.id == server)).map(|s| s.name.clone());
                let name = name.unwrap_or_default();
                let text = match (&result, connect) {
                    (Ok(()), true) => trf("Connected %s", &[&name]),
                    (Ok(()), false) => trf("Disconnected %s", &[&name]),
                    (Err(e), _) => trf("Something went wrong: %s", &[e]),
                };
                // Desktop's `addToast` with the ID of the "Connecting %s..." toast replaces it.
                let toast = format!("server-{server}");
                self.remove_toast(&toast);
                self.show(Toast { id: Some(toast), ..Toast::new(text) });
                return self.fetch_servers(&id);
            }
            Msg::Spin => {}
            Msg::SetInboxVisibility(key, option) => {
                self.list.chat_menu = None;
                let mut st = self.store.write();
                // The demo has no account data to write: its rules change in the store.
                if st.config.demo {
                    st.inbox_rules = beeper_core::matrix::with_inbox_visibility(st.inbox_rules.clone(), &key, option);
                    st.chat_changes += 1;
                    drop(st);
                    return self.settle();
                }
                let client = st.matrix.clone().expect("Show in Inbox needs a Matrix client");
                let rules = beeper_core::matrix::with_inbox_visibility(st.inbox_rules.clone(), &key, option);
                return Task::perform(beeper_core::matrix::set_global(client, beeper_core::matrix::RULES, rules), UiMessage::Pushed);
            }
            _ => unreachable!("not a accounts_servers message"),
        }
        self.settle()
    }

    /// Desktop's `RemoteSubView` fetches the servers when the account page shows them. An account whose servers
    /// were not fetched yet, like the demo's, fetches them too.
    fn page_servers(&mut self, id: &str) -> Task<UiMessage> {
        let st = self.store.read();
        let connected = st.bridge_accounts.iter().any(|b| b.account_id == id && b.state == "CONNECTED");
        let shown = st.sub_accounts.get(id).is_none_or(|l| !l.is_empty());
        let fetch = connected && has_servers(&st, id) && shown;
        drop(st);
        if fetch {
            self.fetch_servers(id)
        } else {
            Task::none()
        }
    }

    /// Desktop's `fetchSubAccounts`: the servers of an account from its bridge, or from the demo's Discord bridge.
    pub(crate) fn fetch_servers(&mut self, id: &str) -> Task<UiMessage> {
        self.servers_loading.insert(id.into(), std::time::Instant::now());
        let mut st = self.store.write();
        let id = id.to_string();
        if st.config.demo {
            if st.demo_servers.is_empty() {
                st.demo_servers = beeper_core::matrix::demo_sub_accounts();
            }
            let servers = st.demo_servers.clone();
            let later = async move {
                tokio::time::sleep(DEMO_BRIDGE_DELAY).await;
                Ok(servers)
            };
            return Task::perform(later, move |r| UiMessage::Accounts(Msg::Servers(id.clone(), r)));
        }
        let p = st.provision(&id);
        Task::perform(beeper_core::matrix::sub_accounts(p), move |r| UiMessage::Accounts(Msg::Servers(id.clone(), r)))
    }

    /// The frames of the spinning Refresh List while the servers of the shown account page load.
    pub(crate) fn server_ticks(&self) -> Option<Subscription<UiMessage>> {
        let page = self.account_page.as_ref().filter(|a| self.servers_loading.contains_key(*a));
        page.map(|_| iced::time::every(Duration::from_millis(16)).map(|_| UiMessage::Accounts(Msg::Spin)))
    }
}
