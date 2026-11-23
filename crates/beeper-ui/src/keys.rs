//! The keyboard of the windows: the text and the keys that each window gets, Desktop's keys of the main window and of
//! the chat windows, and the Escape that closes the modals.

use super::*;

impl BeeperGuiApp {
    /// The text that a key types in a window: Space on a sort or a focused control, Desktop's J, K and hotkeys
    /// without a modifier while no chat is open, and else the composer or the field of the window.
    pub(crate) fn window_text(&mut self, id: window::Id, text: String) -> Task<UiMessage> {
        // Desktop's `KeyboardSensor` takes Space, Enter, Tab, Escape and the arrows of its window while it moves
        // an item.
        if text == " " && self.key_sort.as_ref().is_some_and(|s| s.window == id) {
            return self.sortable(sortable::Msg::Drop);
        }
        // Chromium's Space on a focused control of the main window, when the composer does not take the keys.
        if text == " "
            && Some(id) == self.main_window
            && self.key_targets.contains_key(&id)
            && !self.composer_takes_keys()
        {
            return self.focus_space(id, false).unwrap_or_else(Task::none);
        }
        // Desktop's `handleThreadSelection`: J and K move or grow the selection of chats like ↓ and ↑ in the
        // selection mode, with Shift for J and K, also with a chat open, whose composer is gone in that mode.
        let main = Some(id) == self.main_window;
        if matches!(text.as_str(), "j" | "k" | "J" | "K") && main && self.selection_keys() {
            let arrow = Key::Named(if text.eq_ignore_ascii_case("j") { Named::ArrowDown } else { Named::ArrowUp });
            let shift = if text.chars().all(char::is_uppercase) { Modifiers::SHIFT } else { Modifiers::empty() };
            return self.update(UiMessage::WindowKey(id, arrow, shift));
        }
        // Desktop's X with no chat open selects the chat of the cursor, which starts the selection mode.
        if text == "x" && Some(id) == self.main_window && self.points_chats() {
            self.begin_chat_selection();
            return Task::none();
        }
        // Desktop's J and K move the cursor of the chat list like ↓ and ↑ while no chat is open.
        if matches!(text.as_str(), "j" | "k") && self.window_points_chats(id) {
            let arrow = Key::Named(if text == "j" { Named::ArrowDown } else { Named::ArrowUp });
            return self.update(UiMessage::WindowKey(id, arrow, Modifiers::empty()));
        }
        // Desktop's hotkeys without a modifier, like the "e" of Chat: Toggle Archive, while no chat is open.
        if self.find_hotkey(&Key::Character(text.as_str().into()), Modifiers::empty()).is_some()
            && self.window_points_chats(id)
        {
            return self.update(UiMessage::WindowKey(id, Key::Character(text.into()), Modifiers::empty()));
        }
        if self.chat_windows.iter().any(|w| w.0 == id) {
            return self.update(UiMessage::InWindow(id, Box::new(UiMessage::Composer(composer::Msg::Type(text)))));
        }
        if Some(id) == self.main_window {
            return self.update(UiMessage::Composer(composer::Msg::Type(text)));
        }
        if Some(id) == self.login_window {
            return self.login_text(&text);
        }
        if self.add_account_dropdown(id) {
            return self.update_add_account(id, add_account::AddMsg::Text(text));
        }
        // Chromium's Space on a focused button or `role="button"` row presses it.
        if text == " " && (self.key_targets.contains_key(&id) || self.asking(id)) {
            return self.focus_space(id, true).unwrap_or_else(Task::none);
        }
        self.settle()
    }

    /// A key in a window: the sort that the keyboard moves, Close and Minimize, the login, the menu bar, Picture in
    /// Picture, Settings and the focus of the sheets, and then the keys of each window.
    pub(crate) fn window_key(&mut self, id: window::Id, key: Key, mods: Modifiers) -> Task<UiMessage> {
        if self.key_sort.as_ref().is_some_and(|s| s.window == id) {
            if let Some(m) = sortable::key_msg(&key, mods) {
                return self.sortable(m);
            }
        }
        // Desktop's menu roles "close" and "minimize", whose accelerators work while the menu bar hides.
        if let Key::Character(c) = &key {
            if mods == Modifiers::COMMAND && matches!(c.to_lowercase().as_str(), "w" | "m") {
                return match c.eq_ignore_ascii_case("w") {
                    true => self.update(UiMessage::Window(windows::Msg::CloseRequested(id))),
                    false => window::minimize(id, true),
                };
            }
        }
        if Some(id) == self.login_window || !self.store.read().signed_in() {
            return self.login_key(key, mods);
        }
        let escape = key == Key::Named(Named::Escape);
        if escape && self.menu_bar.is_some() {
            self.menu_bar = None;
            return self.settle();
        }
        // Desktop's `onEscapePressed` of the Picture in Picture window.
        if escape && self.pip.as_ref().is_some_and(|p| p.0 == id) {
            return match self.pip_menu.take() {
                None => self.close_window(id),
                Some(_) => self.settle(),
            };
        }
        if Some(id) == self.settings_window {
            return self.settings_key(key, mods);
        }
        // Chromium's menus take the keyboard while they are open: their arrows and Enter move and run their selection
        // (see `menu_keys`).
        let menu_key = matches!(key, Key::Named(Named::ArrowUp | Named::ArrowDown | Named::Enter));
        if menu_key && mods.is_empty() && self.native_menu_open() {
            return self.settle();
        }
        // Chromium's Tab, Enter and arrows in Desktop's sheets; a confirmation over Label Settings has the
        // focus on its default button.
        let sheet = self.nickname_windows.iter().any(|d| d.id == id)
            || self.label_settings.iter().any(|w| w.id == id)
            || self.label_window.as_ref().is_some_and(|d| d.id == id)
            || self.spaces_window.as_ref().is_some_and(|d| d.id == id)
            || Some(id) == self.blocked_window
            || self.poll_windows.iter().any(|w| w.id == id);
        if let Some(task) = sheet.then(|| self.focus_key(id, &key, mods, true)).flatten() {
            return task;
        }
        if self.label_asking(id) && escape {
            self.confirm = None;
            return Task::none();
        }
        self.child_window_key(id, key, mods)
    }

    /// A key in a child window, which closes on Escape or confirms on Ctrl+Enter, or in a chat window or the main
    /// window, which take Desktop's keys (`key_pressed`).
    fn child_window_key(&mut self, id: window::Id, key: Key, mods: Modifiers) -> Task<UiMessage> {
        if self.label_settings.iter().any(|w| w.id == id) || Some(id) == self.how_window {
            return match key {
                Key::Named(Named::Escape) => self.close_window(id),
                Key::Named(Named::Enter) if mods.command() => self.close_window(id),
                _ => Task::none(),
            };
        }
        // The open dropdown of a phone field takes the keys, so Escape closes it and not the window.
        if self.add_account_dropdown(id) {
            return self.update_add_account(id, add_account::AddMsg::Key(key));
        }
        if self.theme_windows.iter().any(|w| w.0 == id)
            || self.labels_windows.iter().any(|w| w.0 == id)
            || self.vote_windows.iter().any(|w| w.0 == id)
            || self.spaces_window.as_ref().is_some_and(|d| d.id == id)
            || self.merge_window.as_ref().is_some_and(|d| d.id == id)
            || self.group_window.as_ref().is_some_and(|d| d.id == id)
            || Some(id) == self.blocked_window
            || self.nickname_windows.iter().any(|d| d.id == id)
            || self.add_accounts.iter().any(|a| a.id == id)
        {
            return if key == Key::Named(Named::Escape) { self.close_window(id) } else { Task::none() };
        }
        // Desktop's `onEscapePressed` closes the Add an Account window, after the menu of a network.
        if self.networks.window == Some(id) {
            if key == Key::Named(Named::Escape) && self.networks.menu.take().is_none() {
                return self.close_window(id);
            }
            return Task::none();
        }
        if self.reauth.window == Some(id) {
            return self.reauth_key(key);
        }
        if self.enter_key.window == Some(id) {
            return self.enter_key_key(key);
        }
        if self.label_window.as_ref().is_some_and(|d| d.id == id) {
            return match key {
                Key::Named(Named::Escape) => self.close_window(id),
                Key::Named(Named::Enter) if mods.command() => self.label_edit(LabelEdit::Advance),
                _ => Task::none(),
            };
        }
        if self.poll_windows.iter().any(|w| w.id == id) {
            return match key {
                Key::Named(Named::Escape) => self.poll_edit(id, PollEdit::Close),
                Key::Named(Named::Enter) if mods.command() => self.poll_edit(id, PollEdit::Send),
                _ => Task::none(),
            };
        }
        if self.media_window.as_ref().is_none_or(|w| w.id != id) {
            let key = UiMessage::KeyPressed(key, mods);
            return match self.chat_windows.iter().any(|w| w.0 == id) {
                true => self.update(UiMessage::InWindow(id, Box::new(key))),
                false => self.update(key),
            };
        }
        self.media_window_key(id, key, mods)
    }

    /// A key in the Media window: the keys of its viewer, and Escape, which closes its menus, its deletion, its search
    /// and its selection before the window.
    fn media_window_key(&mut self, id: window::Id, key: Key, mods: Modifiers) -> Task<UiMessage> {
        let w = self.media_window.as_ref().expect("the Media window is open");
        if w.viewer.is_some() && w.deleting.is_none() {
            return self.as_media(|s| s.carousel_key(&key, mods));
        }
        if key == Key::Named(Named::Escape) {
            let w = self.media_mut();
            if w.item_menu.is_some() {
                w.item_menu = None;
            } else if w.menu.is_some() {
                w.menu = None;
            } else if w.deleting.is_some() {
                w.deleting = None;
            } else if let Some(q) = w.search.as_ref().map(|q| (!q.is_empty()).then(String::new)) {
                return self.update(UiMessage::Media(media_view::Msg::Search(q)));
            } else if w.select {
                (w.select, w.picked) = (false, Vec::new());
            } else {
                return self.update(UiMessage::Window(windows::Msg::CloseRequested(id)));
            }
        }
        self.settle()
    }

    /// Desktop's keys of the main window and of the chat windows. The recorder, a view-once viewer, the carousel and
    /// a focused attachment take them first, and then the composer, the focus, the command palette, the lists, the
    /// dialogs, the selected messages and the hotkeys, in this order.
    pub(crate) fn key_pressed(&mut self, key: Key, mods: Modifiers) -> Task<UiMessage> {
        if let Key::Named(k @ (Named::Enter | Named::Escape)) = key {
            if (self.recorder.is_some() || self.talk.is_some()) && mods.is_empty() {
                return self.record(record::Msg::Key(k));
            }
        }
        if key == Key::Named(Named::Escape) && self.view_once.is_open() {
            return self.view_once(view_once::Msg::Dismiss);
        }
        if self.pane.carousel.is_some() {
            return self.carousel_key(&key, mods);
        }
        // Desktop's `C_t` takes every key while a pending attachment has the focus.
        if self.pane.att_focus.is_some() {
            return self.update(UiMessage::Composer(composer::Msg::AttachmentKey(key)));
        }
        let task = self
            .composer_key(&key, mods)
            .or_else(|| self.focus_keys(&key, mods))
            .or_else(|| self.palette_key(&key, mods))
            .or_else(|| self.palette_search_key(&key, mods))
            .or_else(|| self.navigation_key(&key, mods))
            .or_else(|| self.dialog_key(&key, mods))
            .or_else(|| self.selection_key(&key, mods))
            .or_else(|| self.find_hotkey(&key, mods).map(|(h, digit)| self.run_hotkey(h, digit)));
        task.unwrap_or_else(|| self.settle())
    }

    /// Undo, and the keys of the GIF and emoji pickers and of the suggestions of the composer.
    fn composer_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let z = mods.command() && matches!(key.as_ref(), Key::Character(c) if c.eq_ignore_ascii_case("z"));
        if z && !mods.shift() && self.shown_undo().is_none() && !self.undo_stack.is_empty() {
            return Some(self.update(UiMessage::UndoAction));
        }
        if z && (self.queued_here() || self.shown_undo().is_some()) {
            return Some(self.update(UiMessage::Composer(composer::Msg::Undo(mods.shift()))));
        }
        if self.pane.picker == Some(Picker::Gif) && mods.is_empty() {
            if let Some(task) = self.gif_key(key) {
                return Some(task);
            }
        }
        if self.pane.emoji.is_some() && mods.is_empty() {
            if let Some(task) = self.emoji_key(key) {
                return Some(task);
            }
        }
        let picks = self.suggestions(&self.store.read());
        let command = picks.as_ref().is_some_and(|(_, p)| matches!(p[0], composer::Suggestion::Command(_)));
        let n = picks.map_or(0, |(_, picks)| picks.len());
        if n == 0 || !mods.is_empty() {
            return None;
        }
        let at = self.pane.suggest.min(n - 1);
        match key.as_ref() {
            Key::Named(Named::ArrowUp | Named::ArrowDown) => {
                let down = *key == Key::Named(Named::ArrowDown);
                // Desktop's ↑ stops at the first slash command, and goes around the other lists.
                self.pane.suggest = match (down, command) {
                    (true, _) => (at + 1) % n,
                    (false, true) => at.saturating_sub(1),
                    (false, false) => (at + n - 1) % n,
                };
                Some(Task::none())
            }
            Key::Named(Named::Tab) => Some(self.update(UiMessage::Composer(composer::Msg::PickSuggestion(at)))),
            Key::Named(Named::Escape) => {
                self.pane.suggest_hidden = Some(self.pane.composer.text());
                Some(Task::none())
            }
            _ => None,
        }
    }

    /// Chromium's focus in a confirmation dialog, and in a window without an open chat, where Desktop's
    /// `KeyboardHandlers` leave Tab to it; then Desktop's `KeyboardHandlers` and Chromium in the composer row, where
    /// Tab and Shift+Tab move and Enter presses the focused button.
    fn focus_keys(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let window = self.swapped.or(self.main_window);
        let modal = self.show_command_palette
            || self.forward.is_some()
            || self.creating_chat.is_some()
            || self.pane.carousel.is_some();
        let free = self.pane.selected_chat_id.is_none() && !modal;
        // New Chat's own Enter starts the group from its encryption prompt.
        let asking = window.is_some_and(|w| self.asking(w));
        let defaults = asking && self.creating_chat.is_none();
        if let Some(task) = window.filter(|_| asking || free).and_then(|w| self.focus_key(w, key, mods, defaults)) {
            return Some(task);
        }
        // Tiptap's `Mod-Shift-7` and `Mod-Shift-8` of the lists, which `cursor::Root` keeps from the composer as
        // shortcuts on digits.
        let list = match key.as_ref() {
            Key::Character("7" | "&") => Some(5),
            Key::Character("8" | "*") => Some(4),
            _ => None,
        };
        let shortcut = mods.command() && mods.shift() && !mods.alt() && self.composer_takes_keys();
        if let Some((window, i)) = window.zip(list).filter(|_| shortcut) {
            let caret = focus::caret(window, self.composer_id());
            return Some(caret.map(move |at| UiMessage::Composer(composer::Msg::FormatKey(i, at))));
        }
        let window = window.filter(|_| self.composer_takes_keys() && (mods - Modifiers::SHIFT).is_empty())?;
        match key.as_ref() {
            Key::Named(Named::Tab) => {
                let back = mods.shift();
                let caret = focus::caret(window, self.composer_id());
                Some(caret.map(move |at| UiMessage::Composer(composer::Msg::Tab(back, at))))
            }
            Key::Named(Named::Enter) if mods.is_empty() => {
                if let Some(press) = self.composer_press() {
                    self.pane.key_focus.set(None);
                    return Some(self.update(press));
                }
                (!self.popup_menu_open() && self.pane.picker.is_none()).then(|| {
                    focus::caret(window, self.composer_id()).map(|at| UiMessage::Composer(composer::Msg::Enter(at)))
                })
            }
            _ => None,
        }
    }

    /// The arrows and Enter of the commands of the command palette.
    fn palette_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        if !(self.show_command_palette && self.cmd_bar_commands_mode && mods.is_empty()) {
            return None;
        }
        let cmds = self.commands(&self.store.read(), self.command_palette_query.trim());
        let n = cmds.len().max(1);
        let by = match key.as_ref() {
            Key::Named(Named::ArrowDown) => 1,
            Key::Named(Named::ArrowUp) => n - 1,
            Key::Named(Named::Enter) => {
                let Some(m) = cmds.into_iter().nth(self.cmd_index).and_then(|c| c.msg) else {
                    return Some(Task::none());
                };
                return Some(self.update(UiMessage::Commands(vec![UiMessage::CloseModals, m])));
            }
            _ => return None,
        };
        self.cmd_index = (self.cmd_index + by) % n;
        self.command_list.reveal(cmds[self.cmd_index].key());
        Some(locate(format!("cmd-{}", self.cmd_index), commands::COMMANDS, Align::Nearest))
    }

    /// The arrows and Enter of the search results of the command palette: Desktop's `CommandBarContainer` of the
    /// search, whose filter menus take the keys while open.
    fn palette_search_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let searching = self.show_command_palette && !self.cmd_bar_commands_mode && self.search_select.is_none();
        (searching && mods.is_empty()).then(|| self.search_key(key)).flatten()
    }

    /// Escape, and the arrows and Enter of the chat list, of the thread pane and of the messages, where ↑ can edit
    /// the last own message.
    fn navigation_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let plain = mods.is_empty() || mods == keyboard::Modifiers::SHIFT;
        Some(match key.as_ref() {
            Key::Named(Named::Escape) => self.update(UiMessage::CloseModals),
            // Desktop's `handleThreadSelection`: in the selection mode the arrows move or grow the selection of
            // chats, wherever the keyboard is.
            Key::Named(k @ (Named::ArrowUp | Named::ArrowDown)) if plain && self.selection_keys() => {
                self.select_chat_key(mods.shift(), k == Named::ArrowDown)
            }
            // Desktop's Shift+↓ and Shift+↑ with no chat open select the chat of the cursor and the next one.
            Key::Named(k @ (Named::ArrowUp | Named::ArrowDown)) if mods == Modifiers::SHIFT && self.points_chats() => {
                self.begin_chat_selection();
                self.select_chat_key(true, k == Named::ArrowDown)
            }
            Key::Named(k @ (Named::ArrowUp | Named::ArrowDown)) if mods.is_empty() && self.points_chats() => {
                self.point_chat(k == Named::ArrowDown);
                Task::none()
            }
            // Desktop's Enter on the chat of the cursor (`selectChatListItem`).
            Key::Named(Named::Enter) if mods.is_empty() && self.points_chats() => {
                let pointed = self.list.cursor.clone();
                let listed = pointed.filter(|c| self.list_ids(&self.store.read()).contains(c));
                listed.map_or_else(Task::none, |c| self.update(UiMessage::SelectChat(c)))
            }
            Key::Named(Named::ArrowUp | Named::ArrowDown | Named::ArrowRight)
                if plain && self.pane.thread.is_some() && self.pane.thread_focus =>
            {
                self.in_thread(UiMessage::KeyPressed(key.clone(), mods))
            }
            Key::Named(Named::ArrowUp | Named::ArrowDown) if plain && self.can_select_messages() => {
                let down = *key == Key::Named(Named::ArrowDown);
                let edit =
                    !down && self.pane.selected_msgs.is_empty() && self.store.read().settings.pref("upArrow") == 1;
                let step = self.update(UiMessage::Message(message::Msg::Step(down, mods.shift())));
                let Some(m) = edit.then(|| self.last_own_message()).flatten() else { return Some(step) };
                self.pane.selected_msgs = vec![m.id.clone()];
                let start = composer::Msg::StartEdit(m.id, m.text.unwrap_or_default());
                step.chain(self.update(UiMessage::Composer(start)))
            }
            _ => return None,
        })
    }

    /// Enter and Backspace in Forward and in New Chat.
    fn dialog_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let drafting = self.creating_chat.is_some() && !self.join_matrix_mode;
        match key.as_ref() {
            Key::Named(Named::Enter) if self.forward.is_some() => {
                let picked = !self.forward.as_ref().expect("forward").picked.is_empty();
                Some(match mods.control() {
                    false => self.update(UiMessage::Forward(forward::Msg::Submit)),
                    true if picked => self.update(UiMessage::Forward(forward::Msg::Send)),
                    true => Task::none(),
                })
            }
            Key::Named(Named::Backspace) if self.forward.as_ref().is_some_and(|f| f.query.is_empty()) => {
                self.forward.as_mut().expect("forward").picked.pop();
                Some(Task::none())
            }
            Key::Named(Named::Enter) if mods.is_empty() && drafting => {
                Some(self.update(UiMessage::NewChat(new_chat::Msg::Start)))
            }
            Key::Named(Named::Backspace) if drafting && self.new_chat_query.is_empty() => {
                let n = self.creating_chat.expect("an open draft");
                self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft").users.pop();
                Some(Task::none())
            }
            _ => None,
        }
    }

    /// The keys of a selection: → reacts to the selected message, Ctrl+C copies the selected text or messages,
    /// Ctrl+A selects all the messages or all the chats, and Ctrl+V pastes into the composer.
    fn selection_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let cmd = mods.command() && !mods.shift();
        match key.as_ref() {
            Key::Named(Named::ArrowRight)
                if mods.is_empty() && self.single_msg().is_some() && self.can_select_messages() =>
            {
                let mid = self.single_msg().cloned().expect("a selected message");
                Some(self.update(UiMessage::Message(message::Msg::ReactOn(mid))))
            }
            Key::Character("c") if cmd && selectable::has_selection() => {
                Some(iced::clipboard::write(selectable::text()))
            }
            Key::Character("c") if cmd && !self.pane.selected_msgs.is_empty() => {
                let stamp = self.store.read().settings.on("copyTimestamps");
                Some(self.update(UiMessage::Message(message::Msg::CopySelected(stamp))))
            }
            Key::Character("a") if cmd && !mods.alt() && self.can_select_messages() => {
                let st = self.store.read();
                let (msgs, live) = self.timeline(&st);
                let all = msgs.iter().zip(live).filter(|(_, l)| *l).map(|(m, _)| m.id.clone()).collect();
                drop(st);
                self.pane.selected_msgs = all;
                Some(Task::none())
            }
            Key::Character("a")
                if cmd
                    && !mods.alt()
                    && self.pane.selected_chat_id.is_none()
                    && !self.show_command_palette
                    && self.list().active_tab != InboxTab::Requests =>
            {
                (self.selection, self.select_mode) = (self.list_ids(&self.store.read()), false);
                if let Some(first) = self.selection.first().cloned() {
                    self.select_chat(&first);
                }
                Some(Task::none())
            }
            Key::Character("v") if mods.command() && self.pane.selected_chat_id.is_some() => {
                Some(self.update(UiMessage::Composer(composer::Msg::Paste)))
            }
            _ => None,
        }
    }

    /// Desktop's Escape (`CloseModals`): it closes the search select, leaves a full-screen video, or closes the menus
    /// and pickers; else it leaves the first of the selection, the modes, the reply, the edit, the thread and the
    /// chat that is open, in this order.
    pub(crate) fn close_modals(&mut self) -> Task<UiMessage> {
        if self.search_select.take().is_some() {
            return self.settle();
        }
        if self.pane.message_menu.is_none() && self.video.as_ref().is_some_and(video::Video::is_full) {
            return self.video(video::Msg::Fullscreen);
        }
        self.user_hover.shown = None;
        if self.show_command_palette
            || self.forward.is_some()
            || self.menu_open()
            || self.pane.gallery_reactions.is_some()
            || self.pane.picker.is_some()
            || self.pane.emoji.is_some()
            || self.quick_look_target.is_some()
            || self.hotkeys_modal.is_some()
        {
            self.close_menus();
        } else if self.select_mode || !self.selection.is_empty() {
            (self.selection, self.select_mode) = (Vec::new(), false);
            // Desktop's `setSelectedThread(null)` with the end of the selection mode: the open chat closes too.
            if self.pane.selected_chat_id.is_some() {
                self.leave_chat(&mut self.store.write(), None);
                self.pane.selected_chat_id = None;
                self.pane.composer = composer_with("");
                self.refresh_theme_and_tray();
            }
        } else if self.join_matrix_mode {
            self.join_matrix_mode = false;
        } else if self.confirm == Some(ASK_ROOM_ENCRYPTION) {
            self.confirm = None;
        } else if self.creating_chat.is_some() {
            self.leave_draft();
            self.pane.selected_chat_id = None;
        } else if self.pane.reply_to_message_id.is_some() {
            self.pane.reply_to_message_id = None;
        } else if self.pane.editing_message_id.is_some() {
            return self.update(UiMessage::Composer(composer::Msg::CancelMode));
        } else if !self.pane.selected_msgs.is_empty() {
            self.pane.selected_msgs.clear();
        } else if self.pane.reply_thread.is_some() {
            self.pane.reply_thread = None;
        } else if self.pane.thread_root.is_some() {
            // Desktop's `onEscapePressed` closes the thread window.
            self.pane.selected_chat_id = None;
        } else if self.pane.selected_chat_id.is_some() {
            self.leave_chat(&mut self.store.write(), None);
            // Desktop's `pointCurrentThread`: the cursor of the list stays on the chat that was open.
            self.list.cursor = self.pane.selected_chat_id.take();
            self.pane.composer = composer_with("");
            self.refresh_theme_and_tray();
        } else if self.list().active_tab != InboxTab::AllChats {
            self.list.active_tab = InboxTab::AllChats;
            self.list.space_view_account = None;
        } else if !self.status_banner.is_empty() {
            self.status_banner.clear();
        } else {
            // Desktop's `setThreadCursorIDByThread` of the first chat.
            self.list.cursor = self.list_ids(&self.store.read()).into_iter().next();
        }
        self.settle()
    }

    /// Closes the command palette, Forward, the menus, the pickers, Quick Look and the hotkeys.
    fn close_menus(&mut self) {
        self.hotkeys_modal = None;
        self.show_command_palette = false;
        self.forward = None;
        self.list.show_filter_menu = false;
        self.list.chat_menu = None;
        // Desktop unselects the gallery when its menu closes without an action.
        if self.pane.message_menu.take().is_some() && !self.pane.gallery_menu.is_empty() {
            self.pane.selected_msgs.clear();
        }
        self.pane.gallery_reactions = None;
        self.pane.thread_menu = None;
        self.unread_menu = None;
        self.pane.edit_menu = None;
        self.pane.sched_menu = None;
        self.list.reminder_menu = None;
        self.audio.menu = None;
        self.selection_menu = None;
        self.pane.show_attach_menu = false;
        self.pane.picker = None;
        self.pane.emoji = None;
        self.quick_look_target = None;
    }
}
