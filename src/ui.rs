use crate::config::{network_spec_by_bridge, NETWORK_SPECS};
use crate::css::ResolvedTheme;
use crate::models::*;
use crate::platform::{OsKind, SystemTrayState};
use crate::server::SharedStore;
use crate::store::{SettingCatalogEntry, SETTINGS_CATALOG};
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxTab {
    AllChats,
    Unread,
    Starred,
    Drafts,
    Archived,
    LowPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    ChatList,
    ConversationView,
    Appearance,
    AccountsAndBridges,
    SecurityAndE2ee,
    StorageAndSystem,
}

pub struct BeeperGuiApp {
    pub store: SharedStore,
    pub theme: ResolvedTheme,
    pub tray: SystemTrayState,
    pub os: OsKind,
    pub active_network: Option<String>,
    pub active_tab: InboxTab,
    pub selected_chat_id: Option<String>,
    pub selected_message_id: Option<String>,
    pub composer_text: String,
    pub reply_to_message_id: Option<String>,
    pub editing_message_id: Option<String>,
    pub search_query: String,
    pub settings_search_query: String,
    pub show_settings_modal: bool,
    pub settings_section: SettingsSection,
    pub show_command_palette: bool,
    pub show_quick_look_modal: bool,
    pub quick_look_label: String,
    pub nickname_input: String,
    pub custom_css_buffer: String,
    pub login_email_input: String,
    pub login_code_input: String,
    pub recovery_key_input: String,
    pub selected_bridge_to_connect: String,
}

impl BeeperGuiApp {
    pub fn new(store: SharedStore, try_native_tray: bool) -> Self {
        let (settings, unread, first_chat, custom_css) = {
            let st = store.read().unwrap();
            (
                st.settings.clone(),
                st.total_unread_chats(),
                st.chats.first().map(|c| c.id.clone()),
                st.settings.custom_css_source.clone(),
            )
        };
        let theme = ResolvedTheme::compile(&settings);
        let tray = SystemTrayState::new(unread, false, try_native_tray);
        Self {
            store,
            theme,
            tray,
            os: OsKind::current(),
            active_network: None,
            active_tab: InboxTab::AllChats,
            selected_chat_id: first_chat,
            selected_message_id: None,
            composer_text: String::new(),
            reply_to_message_id: None,
            editing_message_id: None,
            search_query: String::new(),
            settings_search_query: String::new(),
            show_settings_modal: false,
            settings_section: SettingsSection::ChatList,
            show_command_palette: false,
            show_quick_look_modal: false,
            quick_look_label: String::new(),
            nickname_input: String::new(),
            custom_css_buffer: custom_css,
            login_email_input: "alex@beeper.com".to_string(),
            login_code_input: String::new(),
            recovery_key_input: String::new(),
            selected_bridge_to_connect: "whatsapp".to_string(),
        }
    }

    pub fn reload_theme(&mut self) {
        let settings = self.store.read().unwrap().settings.clone();
        self.theme = ResolvedTheme::compile(&settings);
    }

    pub fn sync_tray(&mut self) {
        let unread = self.store.read().unwrap().total_unread_chats();
        self.tray.update(unread, self.tray.muted);
    }

    pub fn visible_chats(&self) -> Vec<Chat> {
        let st = self.store.read().unwrap();
        let candidate_indices: Vec<usize> = if self.search_query.trim().is_empty() {
            (0..st.chats.len()).collect()
        } else {
            st.chat_search_index
                .query_all_tokens(&self.search_query)
                .into_iter()
                .map(|i| i as usize)
                .collect()
        };

        let mut list: Vec<Chat> = candidate_indices
            .into_iter()
            .filter_map(|i| st.chats.get(i).cloned())
            .filter(|c| {
                if let Some(ref bridge_id) = self.active_network {
                    if c.account_id != format!("acc_{bridge_id}") {
                        return false;
                    }
                }
                match self.active_tab {
                    InboxTab::AllChats => (c.flags & (CHAT_ARCHIVED | CHAT_LOW_PRIORITY)) == 0,
                    InboxTab::Unread => (c.flags & CHAT_UNREAD) != 0,
                    InboxTab::Starred => c.label_ids.iter().any(|l| l == "lbl_favorite"),
                    InboxTab::Drafts => c.draft.is_some(),
                    InboxTab::Archived => (c.flags & CHAT_ARCHIVED) != 0,
                    InboxTab::LowPriority => (c.flags & CHAT_LOW_PRIORITY) != 0,
                }
            })
            .collect();

        let unread_on_top = st.settings.unread_chats_always_on_top;
        list.sort_by(|a, b| {
            if unread_on_top {
                let au = (a.flags & CHAT_UNREAD) != 0;
                let bu = (b.flags & CHAT_UNREAD) != 0;
                if au != bu {
                    return bu.cmp(&au);
                }
            }
            b.last_activity.cmp(&a.last_activity)
        });
        list
    }

    pub fn select_chat(&mut self, chat_id: &str) {
        let mut st = self.store.write().unwrap();
        if let Some(old_id) = self.selected_chat_id.as_deref() {
            if let Some(old_idx) = st.resolve_chat_idx(old_id) {
                st.chats[old_idx].draft = if self.composer_text.trim().is_empty() {
                    None
                } else {
                    Some(ChatDraft {
                        text: Some(self.composer_text.clone()),
                        attachments: Vec::new(),
                        reply_to_message_id: self.reply_to_message_id.clone(),
                    })
                };
            }
        }
        let idx = st.resolve_chat_idx(chat_id).expect("selected chat exists");
        self.composer_text = st.chats[idx]
            .draft
            .as_ref()
            .and_then(|d| d.text.clone())
            .unwrap_or_default();
        self.selected_chat_id = Some(st.chats[idx].id.clone());
        self.reply_to_message_id = None;
        self.editing_message_id = None;
    }

    pub fn submit_composer(&mut self) {
        let text = self.composer_text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let chat_id = self
            .selected_chat_id
            .clone()
            .expect("selected chat required");
        {
            let mut st = self.store.write().unwrap();
            if let Some(edit_mid) = self.editing_message_id.take() {
                st.edit_message(&chat_id, &edit_mid, text)
                    .expect("edit message");
            } else {
                let reply_to = self.reply_to_message_id.take();
                st.send_message(&chat_id, Some(text), reply_to, None)
                    .expect("send message");
                if let Some(c_idx) = st.resolve_chat_idx(&chat_id) {
                    st.chats[c_idx].draft = None;
                }
            }
        }
        self.composer_text.clear();
        self.sync_tray();
    }

    pub fn render_frame(&mut self, ctx: &egui::Context) {
        let zoom = self.store.read().unwrap().settings.zoom_percent;
        self.theme.apply_to_egui(ctx, zoom);

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::K)) {
            self.show_command_palette = !self.show_command_palette;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Comma)) {
            self.show_settings_modal = !self.show_settings_modal;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D)) {
            self.tray.update(self.tray.unread_count, !self.tray.muted);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_command_palette = false;
            self.show_settings_modal = false;
            self.show_quick_look_modal = false;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) && !ctx.wants_keyboard_input() {
            if let Some(ref mid) = self.selected_message_id {
                let st = self.store.read().unwrap();
                if let Some(&m_idx) = st.msg_idx_by_id.get(mid) {
                    if let Some(att) = st.messages[m_idx].attachments.first() {
                        self.quick_look_label = format!(
                            "{} ({})",
                            att.file_name.as_deref().unwrap_or("attachment"),
                            att.mime_type.as_deref().unwrap_or("media")
                        );
                        self.show_quick_look_modal = !self.show_quick_look_modal;
                    }
                }
            }
        }

        {
            let mut st = self.store.write().unwrap();
            if let Some(focused_cid) = st.focused_chat_id.take() {
                self.selected_chat_id = Some(focused_cid);
            }
            if let Some(focused_mid) = st.focused_message_id.take() {
                self.selected_message_id = Some(focused_mid);
            }
        }

        egui::SidePanel::left("account_switcher_rail")
            .exact_width(self.theme.account_switcher_width)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    if ui
                        .selectable_label(self.active_network.is_none(), "ALL")
                        .on_hover_text("Unified Inbox (All Networks)")
                        .clicked()
                    {
                        self.active_network = None;
                    }
                    ui.separator();

                    let bridges = self.store.read().unwrap().bridge_accounts.clone();
                    for b in &bridges {
                        let spec = network_spec_by_bridge(&b.bridge_id);
                        let is_sel = self.active_network.as_deref() == Some(spec.bridge_id);
                        let color = egui::Color32::from_rgb(
                            spec.brand_rgb[0],
                            spec.brand_rgb[1],
                            spec.brand_rgb[2],
                        );
                        let btn = egui::Button::new(
                            egui::RichText::new(spec.badge_code)
                                .strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(if is_sel {
                            color
                        } else {
                            color.gamma_multiply(0.55)
                        })
                        .min_size(egui::vec2(36.0, 36.0));
                        if ui
                            .add(btn)
                            .on_hover_text(format!(
                                "{} ({})",
                                spec.display_name,
                                b.mode.as_deref().unwrap_or("cloud")
                            ))
                            .clicked()
                        {
                            self.active_network = Some(spec.bridge_id.to_string());
                        }
                        ui.add_space(4.0);
                    }

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if ui.button("⚙").on_hover_text("Settings").clicked() {
                            self.show_settings_modal = !self.show_settings_modal;
                        }
                        if ui
                            .button(if self.tray.muted { "🔕" } else { "🔔" })
                            .on_hover_text(&self.tray.tooltip)
                            .clicked()
                        {
                            self.tray.muted = !self.tray.muted;
                            self.sync_tray();
                        }
                    });
                });
            });

        egui::SidePanel::left("threads_sidebar")
            .default_width(320.0)
            .width_range(self.theme.min_sidebar_width..=self.theme.max_sidebar_width)
            .show(ctx, |ui| {
                let settings = self.store.read().unwrap().settings.clone();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .hint_text(format!("Search ({}+K)", self.os.primary_mod_label()))
                            .desired_width(ui.available_width() - 36.0),
                    );
                    if ui.button("⌘K").clicked() {
                        self.show_command_palette = true;
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    if settings.show_all_chats_pin_view
                        && ui
                            .selectable_label(self.active_tab == InboxTab::AllChats, "All")
                            .clicked()
                    {
                        self.active_tab = InboxTab::AllChats;
                    }
                    if settings.show_unread_pin_view
                        && ui
                            .selectable_label(
                                self.active_tab == InboxTab::Unread,
                                format!("Unread ({})", self.tray.unread_count),
                            )
                            .clicked()
                    {
                        self.active_tab = InboxTab::Unread;
                    }
                    if settings.show_starred_pin_view
                        && ui
                            .selectable_label(self.active_tab == InboxTab::Starred, "★ Starred")
                            .clicked()
                    {
                        self.active_tab = InboxTab::Starred;
                    }
                    if settings.show_drafts_pin_view
                        && ui
                            .selectable_label(self.active_tab == InboxTab::Drafts, "Drafts")
                            .clicked()
                    {
                        self.active_tab = InboxTab::Drafts;
                    }
                    let archived_label = if settings.show_archived_count {
                        let count = self
                            .store
                            .read()
                            .unwrap()
                            .chats
                            .iter()
                            .filter(|c| (c.flags & CHAT_ARCHIVED) != 0)
                            .count();
                        format!("Archive ({count})")
                    } else {
                        "Archive".to_string()
                    };
                    if ui
                        .selectable_label(self.active_tab == InboxTab::Archived, archived_label)
                        .clicked()
                    {
                        self.active_tab = InboxTab::Archived;
                    }
                });

                let pinned_chats: Vec<Chat> = {
                    let st = self.store.read().unwrap();
                    st.chats
                        .iter()
                        .filter(|c| {
                            if (c.flags & CHAT_PINNED) == 0 {
                                return false;
                            }
                            if settings.filter_pins_by_network {
                                if let Some(ref net) = self.active_network {
                                    return c.account_id == format!("acc_{net}");
                                }
                            }
                            true
                        })
                        .cloned()
                        .collect()
                };
                if !pinned_chats.is_empty() {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        for pc in &pinned_chats {
                            let is_sel = self.selected_chat_id.as_deref() == Some(pc.id.as_str());
                            let short: String = pc.title.chars().take(8).collect();
                            if ui.selectable_label(is_sel, format!("📌 {short}")).clicked() {
                                self.select_chat(&pc.id);
                            }
                        }
                    });
                }

                ui.separator();

                let chats = self.visible_chats();
                let row_h = self.theme.threads_list_item_height;
                egui::ScrollArea::vertical()
                    .id_salt("virtualized_chat_list")
                    .show_rows(ui, row_h, chats.len(), |ui, row_range| {
                        for idx in row_range {
                            let chat = &chats[idx];
                            let is_sel = self.selected_chat_id.as_deref() == Some(chat.id.as_str());
                            let unread = (chat.flags & CHAT_UNREAD) != 0;
                            let muted = (chat.flags & CHAT_MUTED) != 0;

                            ui.horizontal(|ui| {
                                let mut title_txt = egui::RichText::new(&chat.title);
                                if unread {
                                    title_txt = title_txt.strong();
                                }
                                if ui.selectable_label(is_sel, title_txt).clicked() {
                                    self.select_chat(&chat.id);
                                }
                                if muted {
                                    ui.label("🔕");
                                }
                                if unread {
                                    ui.colored_label(
                                        ResolvedTheme::to_color32(self.theme.brand_blue),
                                        format!("● {}", chat.unread_count.max(1)),
                                    );
                                }
                            });
                            if settings.show_message_previews {
                                if let Some(ref prev) = chat.preview {
                                    ui.colored_label(
                                        ResolvedTheme::to_color32(self.theme.text_secondary),
                                        prev.text.as_deref().unwrap_or("[Media attachment]"),
                                    );
                                }
                            }
                        }
                    });
            });

        if self
            .store
            .read()
            .unwrap()
            .settings
            .show_right_panel_by_default
        {
            egui::SidePanel::right("right_inspector_panel")
                .default_width(280.0)
                .show(ctx, |ui| {
                    ui.heading("Chat Details");
                    if let Some(ref cid) = self.selected_chat_id.clone() {
                        let st = self.store.read().unwrap();
                        if let Some(c_idx) = st.resolve_chat_idx(cid) {
                            let chat = st.chats[c_idx].clone();
                            drop(st);
                            ui.label(format!("Title: {}", chat.title));
                            ui.label(format!("Account: {}", chat.account_id));
                            ui.label(format!("Type: {}", chat.chat_type));
                            ui.separator();
                            ui.label("Custom Nickname (Edit for Me):");
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut self.nickname_input);
                                if ui.button("Save").clicked() && !self.nickname_input.is_empty() {
                                    if let Some(first_p) = chat.participants.items.first() {
                                        self.store.write().unwrap().set_user_nickname(
                                            &chat.id,
                                            &first_p.id,
                                            Some(self.nickname_input.clone()),
                                        );
                                    }
                                }
                            });
                            ui.separator();
                            ui.heading("Bookmarked Messages");
                            let st = self.store.read().unwrap();
                            if let Some(m_indices) = st.chat_msg_indices.get(&chat.id) {
                                for &mi in m_indices {
                                    let m = &st.messages[mi];
                                    if (m.flags & MSG_BOOKMARKED) != 0 {
                                        ui.label(format!(
                                            "🔖 {}",
                                            m.text.as_deref().unwrap_or("[Attachment]")
                                        ));
                                    }
                                }
                            }
                        }
                    }
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(cid) = self.selected_chat_id.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a conversation to start messaging");
                });
                return;
            };

            let (chat, messages, settings) = {
                let st = self.store.read().unwrap();
                let Some(c_idx) = st.resolve_chat_idx(&cid) else {
                    return;
                };
                let chat = st.chats[c_idx].clone();
                let msgs: Vec<Message> = st
                    .chat_msg_indices
                    .get(&chat.id)
                    .into_iter()
                    .flatten()
                    .map(|&mi| &st.messages[mi])
                    .filter(|m| (m.flags & MSG_DELETED) == 0)
                    .cloned()
                    .collect();
                (chat, msgs, st.settings.clone())
            };

            if settings.show_chat_header {
                ui.horizontal(|ui| {
                    ui.heading(&chat.title);
                    if let Some(ref desc) = chat.description {
                        ui.colored_label(
                            ResolvedTheme::to_color32(self.theme.text_secondary),
                            format!("— {desc}"),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("ℹ Inspector").clicked() {
                            let mut st = self.store.write().unwrap();
                            st.settings.show_right_panel_by_default =
                                !st.settings.show_right_panel_by_default;
                        }
                        if (chat.flags & CHAT_MUTED) != 0 && ui.button("🔔 Notify Anyway").clicked()
                        {
                            let mut st = self.store.write().unwrap();
                            if let Some(idx) = st.resolve_chat_idx(&chat.id) {
                                st.chats[idx].flags |= CHAT_NOTIFY_ANYWAY;
                            }
                        }
                    });
                });
                ui.separator();
            }

            let avail_h = (ui.available_height() - 68.0).max(120.0);
            egui::ScrollArea::vertical()
                .id_salt("conversation_messages_scroll")
                .max_height(avail_h)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for m in &messages {
                        let is_self = (m.flags & MSG_SENDER_SELF) != 0;
                        let bubble_bg = ResolvedTheme::to_color32(if is_self {
                            self.theme.bubble_self
                        } else {
                            self.theme.bubble_other
                        });
                        let bubble_fg = ResolvedTheme::to_color32(if is_self {
                            self.theme.bubble_self_text
                        } else {
                            self.theme.bubble_other_text
                        });

                        egui::Frame::none()
                            .fill(bubble_bg)
                            .rounding(self.theme.bubble_radius)
                            .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                            .show(ui, |ui| {
                                if let Some(ref sname) = m.sender_name {
                                    let display_sender = if settings.show_full_names_in_chats {
                                        sname.as_str()
                                    } else {
                                        sname.split_whitespace().next().unwrap_or(sname)
                                    };
                                    ui.colored_label(
                                        ResolvedTheme::to_color32(self.theme.text_secondary),
                                        display_sender,
                                    );
                                }
                                if let Some(ref txt) = m.text {
                                    ui.colored_label(bubble_fg, txt);
                                }
                                if !m.edit_history.is_empty() {
                                    ui.collapsing(
                                        format!("Edited ({} prior)", m.edit_history.len()),
                                        |ui| {
                                            for eh in &m.edit_history {
                                                ui.label(format!("{}: {}", eh.timestamp, eh.text));
                                            }
                                        },
                                    );
                                }
                                for att in &m.attachments {
                                    ui.horizontal(|ui| {
                                        ui.label(format!(
                                            "📎 {}",
                                            att.file_name.as_deref().unwrap_or("attachment")
                                        ));
                                        if att.mime_type.as_deref() == Some("application/pdf") {
                                            ui.label("[Inline PDF Viewer]");
                                        }
                                        if att.is_voice_note == Some(true)
                                            && ui.small_button("↻ Re-transcribe").clicked()
                                        {
                                            self.store
                                                .write()
                                                .unwrap()
                                                .retranscribe_voice_note(&m.id);
                                        }
                                    });
                                    if let Some(ref tr) = att.transcript {
                                        ui.colored_label(
                                            ResolvedTheme::to_color32(self.theme.text_secondary),
                                            format!("💬 {tr}"),
                                        );
                                    }
                                }
                                if let Some(ref poll) = m.poll {
                                    ui.label(egui::RichText::new(&poll.question).strong());
                                    for opt in &poll.options {
                                        if ui
                                            .button(format!(
                                                "{} ({} votes)",
                                                opt.label,
                                                opt.voter_ids.len()
                                            ))
                                            .clicked()
                                        {
                                            self.store.write().unwrap().vote_poll(
                                                &m.id,
                                                &opt.id,
                                                "@alex:beeper.com",
                                            );
                                        }
                                    }
                                }
                                ui.horizontal(|ui| {
                                    for rx in &m.reactions {
                                        let _ = ui.small_button(&rx.reaction_key);
                                    }
                                    if ui.small_button("👍").clicked() {
                                        let _ = self
                                            .store
                                            .write()
                                            .unwrap()
                                            .add_reaction(&chat.id, &m.id, "👍", None);
                                    }
                                    if ui.small_button("🔖").on_hover_text("Bookmark").clicked() {
                                        self.store.write().unwrap().toggle_bookmark(&m.id);
                                    }
                                    if ui.small_button("● Unread from here").clicked() {
                                        self.store
                                            .write()
                                            .unwrap()
                                            .mark_unread_from_message(&chat.id, &m.id);
                                        self.sync_tray();
                                    }
                                });
                            });
                        ui.add_space(6.0);
                    }
                });

            ui.separator();

            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.composer_text)
                        .hint_text(format!("Message {}...", chat.title))
                        .desired_width(ui.available_width() - 90.0),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.submit_composer();
                }
                if ui.button("Send").clicked() {
                    self.submit_composer();
                }
            });
        });

        if self.show_settings_modal {
            let mut open = self.show_settings_modal;
            let mut theme_dirty = false;
            egui::Window::new("Beeper Settings")
                .default_width(680.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.settings_section,
                            SettingsSection::ChatList,
                            "Chat List",
                        );
                        ui.selectable_value(
                            &mut self.settings_section,
                            SettingsSection::ConversationView,
                            "Conversation View",
                        );
                        ui.selectable_value(
                            &mut self.settings_section,
                            SettingsSection::Appearance,
                            "Themes & CSS",
                        );
                        ui.selectable_value(
                            &mut self.settings_section,
                            SettingsSection::AccountsAndBridges,
                            "Networks (16)",
                        );
                        ui.selectable_value(
                            &mut self.settings_section,
                            SettingsSection::SecurityAndE2ee,
                            "Security & E2EE",
                        );
                        ui.selectable_value(
                            &mut self.settings_section,
                            SettingsSection::StorageAndSystem,
                            "Storage & OS",
                        );
                    });
                    ui.separator();

                    let mut st = self.store.write().unwrap();
                    match self.settings_section {
                        SettingsSection::ChatList => {
                            ui.checkbox(
                                &mut st.settings.show_archived_count,
                                "Show archived count",
                            );
                            ui.checkbox(
                                &mut st.settings.show_starred_pin_view,
                                "Show starred pin view",
                            );
                            ui.checkbox(
                                &mut st.settings.show_all_chats_pin_view,
                                "Show all chats pin view",
                            );
                            ui.checkbox(
                                &mut st.settings.show_unread_pin_view,
                                "Show unread pin view",
                            );
                            ui.checkbox(
                                &mut st.settings.show_drafts_pin_view,
                                "Show drafts pin view",
                            );
                            ui.checkbox(
                                &mut st.settings.filter_pins_by_network,
                                "Filter pins by network",
                            );
                            ui.checkbox(
                                &mut st.settings.show_favorite_chats_section,
                                "Show favorite chats section",
                            );
                            ui.checkbox(
                                &mut st.settings.keep_favorites_in_inbox,
                                "Keep favorites in inbox",
                            );
                            ui.checkbox(
                                &mut st.settings.show_message_previews,
                                "Show message previews",
                            );
                            ui.checkbox(
                                &mut st.settings.unread_chats_always_on_top,
                                "Unread chats always on top",
                            );
                        }
                        SettingsSection::ConversationView => {
                            ui.checkbox(
                                &mut st.settings.show_full_names_in_chats,
                                "Show full names in chats",
                            );
                            ui.checkbox(&mut st.settings.auto_play_videos, "Auto-play videos");
                            ui.checkbox(&mut st.settings.show_chat_header, "Show chat header");
                            ui.checkbox(
                                &mut st.settings.show_message_input_buttons,
                                "Show message input buttons",
                            );
                            ui.checkbox(&mut st.settings.link_previews, "Link previews");
                            ui.checkbox(
                                &mut st.settings.show_right_panel_by_default,
                                "Show right panel by default",
                            );
                            ui.checkbox(
                                &mut st.settings.load_remote_content_automatically,
                                "Load remote content automatically",
                            );
                            ui.checkbox(
                                &mut st.settings.use_twenty_four_hour_time,
                                "Use 24-hour time",
                            );
                        }
                        SettingsSection::Appearance => {
                            ui.horizontal(|ui| {
                                for (choice, label) in [
                                    (ThemeChoice::TextsLight, "Texts Light"),
                                    (ThemeChoice::TextsDark, "Texts Dark"),
                                    (ThemeChoice::Dracula, "Dracula"),
                                    (ThemeChoice::Nocturne, "Nocturne"),
                                    (ThemeChoice::DarkPurple, "Dark Purple"),
                                    (ThemeChoice::Custom, "Custom CSS"),
                                ] {
                                    if ui
                                        .selectable_value(&mut st.settings.theme, choice, label)
                                        .changed()
                                    {
                                        theme_dirty = true;
                                    }
                                }
                            });
                            if ui
                                .checkbox(
                                    &mut st.settings.no_transparency,
                                    "No transparency (.no-transparency)",
                                )
                                .changed()
                            {
                                theme_dirty = true;
                            }
                            if ui
                                .checkbox(
                                    &mut st.settings.reduce_transparency,
                                    "Reduce transparency (.reduce-transparency)",
                                )
                                .changed()
                            {
                                theme_dirty = true;
                            }
                            if ui
                                .checkbox(
                                    &mut st.settings.fix_lockdown_emojis,
                                    "Fix Lockdown Mode Emojis",
                                )
                                .changed()
                            {
                                theme_dirty = true;
                            }
                            ui.label("Custom CSS Override:");
                            ui.text_edit_multiline(&mut self.custom_css_buffer);
                            ui.horizontal(|ui| {
                                if ui.button("Apply Custom CSS").clicked() {
                                    st.settings.custom_css_source = self.custom_css_buffer.clone();
                                    theme_dirty = true;
                                }
                                if ui.button("Reset CSS").clicked() {
                                    self.custom_css_buffer.clear();
                                    st.settings.custom_css_source.clear();
                                    theme_dirty = true;
                                }
                            });
                        }
                        SettingsSection::AccountsAndBridges => {
                            for spec in &NETWORK_SPECS {
                                let connected = st
                                    .bridge_accounts
                                    .iter()
                                    .any(|b| b.bridge_id == spec.bridge_id);
                                ui.horizontal(|ui| {
                                    ui.label(format!(
                                        "[{}] {}",
                                        spec.badge_code, spec.display_name
                                    ));
                                    ui.label(if connected {
                                        "✅ Connected"
                                    } else {
                                        "⚪ Disconnected"
                                    });
                                });
                            }
                        }
                        SettingsSection::SecurityAndE2ee => {
                            ui.label(format!("User ID: {}", st.beeper_state.user_id));
                            ui.label(format!("Device ID: {}", st.beeper_state.device_id));
                            ui.label(format!("E2EE Verified: {}", st.beeper_state.verified));
                            ui.label(format!(
                                "Recovery Code: {}",
                                st.beeper_state.secrets.recovery_code
                            ));
                        }
                        SettingsSection::StorageAndSystem => {
                            let (db_bytes, media_bytes) = st.calculate_storage_bytes();
                            ui.label(format!(
                                "Isolated data directory: {}",
                                st.data_dir.display()
                            ));
                            ui.label(format!(
                                "Imported files from original Beeper: {}",
                                st.copied_files_from_source
                            ));
                            ui.label(format!("SQLite index.db size: {db_bytes} bytes"));
                            ui.label(format!("Cached media size: {media_bytes} bytes"));
                            if ui.button("Clear All Cached Media").clicked() {
                                st.clear_cached_media_older_than_days(0);
                            }
                        }
                    }
                });
            self.show_settings_modal = open;
            if theme_dirty {
                self.reload_theme();
            }
        }

        if self.show_quick_look_modal {
            let mut open = self.show_quick_look_modal;
            egui::Window::new("Quick Look")
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.heading(&self.quick_look_label);
                });
            self.show_quick_look_modal = open;
        }
    }
}

impl eframe::App for BeeperGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render_frame(ctx);
    }
}

pub fn search_settings_catalog(query: &str) -> Vec<&'static SettingCatalogEntry> {
    let st_tokens = crate::store::tokenize(query);
    if st_tokens.is_empty() {
        return SETTINGS_CATALOG.iter().collect();
    }
    SETTINGS_CATALOG
        .iter()
        .filter(|entry| {
            let hay =
                format!("{} {} {}", entry.section, entry.title, entry.description).to_lowercase();
            st_tokens.iter().all(|t| hay.contains(t))
        })
        .collect()
}
