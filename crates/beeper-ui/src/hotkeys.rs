//! Settings > Hotkeys: the hotkey switches, the shortcut recorder, and the Keyboard Shortcuts list.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Toggle(&'static str),
    /// Desktop's click on a `ShortcutRecorder`, which starts to record keys for the hotkey.
    Record(&'static str),
    /// The Save (true) or Cancel (false) button of the recorder.
    Recorder(bool),
    /// The Revert button of a hotkey with keys other than the default.
    Revert(&'static str),
    RevertAll,
    Query(String),
    /// Opens Desktop's Keyboard Shortcuts list with the search text, or closes it.
    Modal(Option<String>),
    Scrolled(bool),
}

impl BeeperGuiApp {
    pub(crate) fn hotkeys(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Toggle(name) => {
                self.patch_settings(|s| {
                    match s.hotkey_keys(name) {
                        "disabled" if prefs::hotkey_default(name) == "disabled" => {
                            s.keybindings.insert(name.into(), "".into())
                        }
                        "disabled" => s.keybindings.remove(name),
                        _ => s.keybindings.insert(name.into(), "disabled".into()),
                    };
                });
                (self.hotkey_rec, self.relaunch_toast) = (None, Some(HOTKEYS_RELAUNCH));
            }
            Msg::Record(name) => {
                if self.hotkey_rec.as_ref().is_none_or(|r| r.name != name) {
                    self.hotkey_rec = Some(HotkeyRec { name, mods: [false; 3], key: None, buttons: false });
                }
            }
            Msg::Recorder(false) => self.hotkey_rec = None,
            Msg::Recorder(true) => self.save_hotkey(),
            Msg::Revert(name) => {
                self.patch_settings(|s| drop(s.keybindings.remove(name)));
                (self.hotkey_rec, self.relaunch_toast) = (None, Some(HOTKEYS_RELAUNCH));
            }
            Msg::RevertAll => {
                self.patch_settings(|s| s.keybindings.clear());
                return self.update(UiMessage::Window(windows::Msg::Relaunch));
            }
            Msg::Query(q) => self.hotkey_query = q,
            Msg::Modal(q) => {
                self.hotkeys_scrolled &= q.is_some();
                let open = self.hotkeys_modal.is_none() && q.is_some();
                self.hotkeys_modal = q;
                if open {
                    return operation::focus("hotkeys-search");
                }
            }
            Msg::Scrolled(down) => self.hotkeys_scrolled = down,
        }
        self.settle()
    }
}

impl BeeperGuiApp {
    /// The hotkey with the modifiers and the key, or with the modifiers alone before the digit 1 to 9.
    pub(crate) fn find_hotkey(&self, key: &Key, mods: keyboard::Modifiers) -> Option<(&'static str, usize)> {
        let name = hotkey_key(key)?;
        let digit = name.parse::<usize>().ok().filter(|d| (1..=9).contains(d));
        let mods = [mods.control() || mods.command(), mods.shift(), mods.alt()];
        let st = self.store.read();
        // Show Replies shares Ctrl Down with the scroll, and takes it when messages have the focus.
        let selecting = !self.pane().selected_msgs.is_empty();
        let hit = |mods, key: &str| {
            let hits = prefs::HOTKEYS.iter().filter(|h| prefs::hotkey_has(st.settings.hotkey_keys(h.0), mods, key));
            hits.min_by_key(|h| selecting && h.0 == "Scroll to the bottom of a chat").map(|h| h.0)
        };
        let unshifted = || hit([mods[0], false, mods[2]], &name).filter(|h| *h == CYCLE && mods[1]);
        let hit_digit = || Some((hit(mods, "")?, digit? - 1));
        hit(mods, &name).or_else(unshifted).map(|h| (h, 0)).or_else(hit_digit)
    }

    pub(crate) fn run_hotkey(&mut self, name: &str, digit: usize) -> Task<UiMessage> {
        let chat = self.pane.selected_chat_id.clone();
        let flag = |f: ChatFlag| chat.clone().map(|c| UiMessage::ChatList(chat_list::Msg::ToggleFlag(c, f)));
        let selected = self.single_msg().cloned().filter(|_| chat.is_some());
        let msg = match name {
            "Chat: Archive All Read" => Some(UiMessage::Ask(Some(ASK_ARCHIVE_READ))),
            "Chat: Start New" => Some(UiMessage::NewChat(new_chat::Msg::Open)),
            "Chat: Toggle Archive" => flag(ChatFlag::Archived),
            "Chat: Toggle Mark As Read" => chat.map(|a0| UiMessage::ChatList(chat_list::Msg::ToggleRead(a0))),
            "Chat: Toggle Mute" => flag(ChatFlag::Muted),
            "Chat: Toggle Pin" => flag(ChatFlag::Pinned),
            CYCLE => {
                let task = self.step_history(if self.mods.shift() { -1 } else { 1 });
                self.cycling = true;
                return task;
            }
            "History: Next Chat" => return self.step_history(-1),
            "History: Previous Chat" => return self.step_history(1),
            "Select Next Chat" | "Select Previous Chat" => {
                self.step_chat(if name == "Select Next Chat" { 1 } else { -1 });
                None
            }
            // Desktop's `SCROLL_TO_END_OF_CHAT`: unless the caret is in the composer before the end of its text.
            "Scroll to the bottom of a chat" => {
                let text = self.pane.composer.text();
                let end = format::offset(&text, self.pane.composer.cursor().position) == text.len();
                let bottom = || operation::scroll_to("timeline", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                return match (chat.is_some(), end) {
                    (false, _) => Task::none(),
                    (true, true) => bottom(),
                    (true, false) => {
                        operation::is_focused("composer-input")
                            .then(move |focused| if focused { Task::none() } else { bottom() })
                    }
                };
            }
            "Select Next Unread Chat" => {
                let st = self.store.read();
                let next = self
                    .visible_chats(&st)
                    .into_iter()
                    .find(|c| c.flags & CHAT_UNREAD != 0 && Some(&c.id) != chat.as_ref());
                next.map(|c| UiMessage::SelectChat(c.id.clone()))
            }
            "Switch First 9 Chats Modifier" => {
                let id = self.visible_chats(&self.store.read()).get(digit).map(|c| c.id.clone());
                id.map(UiMessage::SelectChat)
            }
            "Toggle Chat Info" => Some(UiMessage::ToggleRightPanel),
            "Command Bar" if self.show_command_palette => Some(UiMessage::CommandBar(commands::Msg::ToggleMode)),
            "Command Bar" => {
                let open = self.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
                self.cmd_bar_commands_mode = true;
                return open;
            }
            "Filter By Account" => {
                Some(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::FilterAccount)))
            }
            "Remind Later" => {
                chat.map(|id| UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Remind(id))))
            }
            "Cycle Filters" => {
                self.set_filters(match self.list().filters {
                    f if f == 1 << 4 => 0,
                    f if f.count_ones() == 1 && f < 1 << 4 => f << 1,
                    _ => 1,
                });
                None
            }
            "Keyboard Shortcuts" => {
                Some(UiMessage::Hotkeys(hotkeys::Msg::Modal(self.hotkeys_modal.is_none().then(String::new))))
            }
            "Filter: All" => {
                self.set_filters(0);
                None
            }
            f if FILTER_HOTKEYS.contains(&f) => {
                let bit = 1 << FILTER_HOTKEYS.iter().position(|h| *h == f).expect("a filter hotkey");
                self.set_filters(self.list.filters ^ bit);
                None
            }
            "Chat: Send File" => chat.map(|_| UiMessage::Composer(composer::Msg::PickFiles)),
            "Open User Profile" => {
                let st = self.store.read();
                let c = chat.as_deref().and_then(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]);
                c.and_then(profile_link).map(UiMessage::OpenUrl)
            }
            "Message: Send Message and Archive" => chat.map(|_| UiMessage::Composer(composer::Msg::SendAndArchive)),
            "Message: Send Later" => chat.map(|_| UiMessage::Scheduled(scheduled::Msg::Open)),
            "Send Message" | "Insert New Line" => None,
            "Minimize To Tray" => return self.set_hidden(true),
            "Quit App" => return self.quit(),
            "Reload App" => Some(UiMessage::Window(windows::Msg::Relaunch)),
            "Report a Problem" => Some(UiMessage::OpenUrl(ISSUES_URL.into())),
            "Reset Zoom" => Some(UiMessage::Zoom(zoom::Msg::Step(0))),
            "Search" => Some(UiMessage::CommandBar(commands::Msg::TogglePalette)),
            "Search: Room" => chat.map(|a0| UiMessage::CommandBar(commands::Msg::SearchRoom(a0))),
            // Desktop's `disableSpaceSwitching` keeps a space window on its space.
            "Select Next Account" | "Select Previous Account" if self.list.locked => None,
            "Select Next Account" | "Select Previous Account" => {
                let rail = rail_accounts(&self.store.read());
                let at = self.list().selected_account_filter.as_ref().and_then(|a| rail.iter().position(|r| r == a));
                let to = match (name == "Select Next Account", at) {
                    (true, None) => rail.first().cloned().map(Some),
                    (true, Some(i)) => rail.get(i + 1).cloned().map(Some),
                    (false, Some(i)) => Some(i.checked_sub(1).map(|i| rail[i].clone())),
                    (false, None) => None,
                };
                to.map(|a0| UiMessage::Rail(rail::Msg::Select(a0)))
            }
            "Send Feedback" => chat.map(|_| UiMessage::OpenQuickLook("Feedback".into(), FEEDBACK.into())),
            // Desktop's `openPrefsInNewWindow` focuses the open Settings, else opens it at Accounts.
            "Settings" => {
                let tab = if self.settings_window.is_some() {
                    self.active_settings_tab
                } else {
                    crate::SettingsTab::Accounts
                };
                Some(UiMessage::Settings(settings::Msg::Open(tab)))
            }
            "Switch First 9 Spaces Modifier" => {
                let space = rail_spaces(&self.store.read()).into_iter().nth(digit);
                space.and_then(|id| match RAIL_VIEWS.iter().find(|v| v.0 == id) {
                    Some(view) => view.1.map(|a0| UiMessage::ChatList(chat_list::Msg::SelectTab(a0))),
                    None => Some(UiMessage::Rail(rail::Msg::Select(Some(id)))),
                })
            }
            "Toggle Incognito Mode" => {
                let st = self.store.read();
                st.feature("stealth-mode").on.then_some(UiMessage::ToggleIncognito)
            }
            "Toggle Sidebar" => {
                self.set_sidebar_hidden(!self.sidebar_hidden);
                None
            }
            "Toggle Space Bar" => {
                let on = self.store.read().settings.on("showSidebar");
                Some(UiMessage::Settings(settings::Msg::SetPref("showSidebar", u8::from(!on))))
            }
            "Zoom In" => Some(UiMessage::Zoom(zoom::Msg::Step(1))),
            "Zoom Out" => Some(UiMessage::Zoom(zoom::Msg::Step(-1))),
            "Download Attachments" => selected
                .filter(|_| self.selected_message().is_some_and(|m| !m.attachments.is_empty()))
                .map(|m| UiMessage::Message(message::Msg::Download(m, None))),
            "Message: Edit" => {
                let edit = self.selected_message().filter(|m| self.can_edit(m));
                let m = chat.and_then(|_| edit.or_else(|| self.last_own_message()));
                m.map(|m| {
                    self.pane.selected_msgs = vec![m.id.clone()];
                    UiMessage::Composer(composer::Msg::StartEdit(m.id, m.text.unwrap_or_default()))
                })
            }
            "Message: Forward" => {
                let st = self.store.read();
                let msgs = chat.iter().flat_map(|_| self.timeline(&st).0);
                let msgs: Vec<&Message> = msgs.filter(|m| self.pane.selected_msgs.contains(&m.id)).collect();
                let mids = msgs.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
                // Desktop's `canOpen` of the forward menu: no view-once media.
                let ok = !mids.is_empty() && !msgs.iter().any(|m| m.view_once());
                drop(st);
                if !ok {
                    self.status_banner = tr("Only works when messages are selected").into();
                }
                ok.then_some(UiMessage::Forward(forward::Msg::Open(mids)))
            }
            "Message: Open First Link" => {
                let text = self.selected_message().and_then(|m| m.text);
                text.and_then(|t| {
                    t.split_whitespace().find(|w| w.starts_with("http")).map(|u| UiMessage::OpenUrl(u.into()))
                })
            }
            "Message: Quick React" => selected.map(|m| self.quick_react(m)),
            "Message: Quote & Reply" => selected.map(|a0| UiMessage::Composer(composer::Msg::StartReply(a0))),
            "Message: Quote or Edit Selected Message" => {
                let empty = self.pane.composer.text().is_empty();
                self.selected_message().and_then(|m| match m.is_sender == Some(true) {
                    true => (empty && self.can_edit(&m))
                        .then(|| UiMessage::Composer(composer::Msg::StartEdit(m.id, m.text.unwrap_or_default()))),
                    false => Some(UiMessage::Composer(composer::Msg::StartReply(m.id))),
                })
            }
            "Message: Record Audio" => chat.map(|_| UiMessage::Record(record::Msg::Start)),
            "Message: Talk to Type" => chat.map(|_| UiMessage::Record(record::Msg::Talk)),
            "Message: Send GIF" => chat.map(|_| UiMessage::Composer(composer::Msg::TogglePicker(Picker::Gif))),
            "Message: Send Sticker" => chat.map(|_| UiMessage::Composer(composer::Msg::TogglePicker(Picker::Sticker))),
            "Show Replies" => chat.and_then(|_| {
                let st = self.store.read();
                let msgs = self.timeline(&st).0;
                let chat = &st.chats[st
                    .resolve_chat_idx(self.pane.selected_chat_id.as_ref().expect("an open chat"))
                    .expect("a known chat")];
                self.pane.selected_msgs.iter().find_map(|id| {
                    let m = st.message(id);
                    let replied = || msgs.iter().any(|r| r.id != m.id && reply_root(&st, r) == m.id);
                    let thread =
                        || can_thread(&st, chat, m).then(|| UiMessage::Thread(threads::Msg::Show(m.id.clone())));
                    let chain = || replied().then(|| UiMessage::Message(message::Msg::ShowReplies(m.id.clone())));
                    m.linked_message_id
                        .clone()
                        .map(|a0| UiMessage::Message(message::Msg::ShowReplies(a0)))
                        .or_else(thread)
                        .or_else(chain)
                })
            }),
            _ => unreachable!("{name} has no action"),
        };
        msg.map_or_else(Task::none, |m| self.update(m))
    }
}
