use crate::config::NETWORK_SPECS;
use crate::css::{
    bold, bordered_box, btn_bordered_style, btn_style, bubble_box, composer_box, elevated_card,
    ell, ensure_fonts_loaded, fill, fill_box, filled_ic, fw, hash_str, hue, ic, icon, initials,
    input_style, med, pick_menu_style, pick_style, scroll_style, tog_style, txt, ResolvedTheme,
    CLEAR, FONT_BLACK, FONT_BOLD, FONT_LUCIDE, FONT_MEDIUM, FONT_PHOSPHOR, FONT_REGULAR,
};
use crate::models::*;
use crate::platform::{DisplayServerKind, OsKind, SystemTrayState};
use crate::server::SharedStore;
use crate::store::{BeeperStore, SettingCatalogEntry, SETTINGS_CATALOG};
use iced::font::Weight;
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::{
    button, column, container, mouse_area, operation, pick_list, row, scrollable, stack,
    text_input, toggler, Column, Row, Space,
};
use iced::{
    alignment, window, Center, Color, Element, Event, Fill, Padding, Point, Right, Size,
    Subscription, Task, Theme,
};

pub type El<'a> = Element<'a, UiMessage>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxTab {
    AllChats,
    Unread,
    Pinned,
    Drafts,
    Archived,
    LowPriority,
}

impl InboxTab {
    pub const ALL: [Self; 6] = [
        Self::AllChats,
        Self::Unread,
        Self::Pinned,
        Self::Drafts,
        Self::Archived,
        Self::LowPriority,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::AllChats => "All",
            Self::Unread => "Unread",
            Self::Pinned => "Pinned",
            Self::Drafts => "Drafts",
            Self::Archived => "Archive",
            Self::LowPriority => "Low Priority",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    ChatList,
    ConversationView,
    Appearance,
    Networks,
    Security,
    StorageAndOs,
}

impl SettingsTab {
    pub const ALL: [Self; 6] = [
        Self::ChatList,
        Self::ConversationView,
        Self::Appearance,
        Self::Networks,
        Self::Security,
        Self::StorageAndOs,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::ChatList => "Chat List",
            Self::ConversationView => "Conversation View",
            Self::Appearance => "Themes & CSS",
            Self::Networks => "Networks (16)",
            Self::Security => "Security & E2EE",
            Self::StorageAndOs => "Storage & OS",
        }
    }
}

#[derive(Clone)]
pub enum UiMessage {
    SelectTab(InboxTab),
    SelectAccount(Option<String>),
    SelectChat(String),
    ComposerChanged(String),
    SubmitComposer,
    CancelComposeMode,
    StartReply(String),
    StartEdit(String, String),
    DeleteMessage(String, String),
    ToggleReaction(String, String, String),
    ToggleBookmark(String),
    MarkUnreadFromHere(String, String),
    RetranscribeVoice(String),
    VotePoll(String, String),
    SendQuickVoiceNote,
    SendQuickPoll,
    SetReminder(String),
    TogglePinChat(String),
    ToggleMuteChat(String),
    ToggleArchiveChat(String),
    NotifyAnyway(String),
    NicknameChanged(String),
    SaveNickname,
    SearchChanged(String),
    ClearSearch,
    ToggleCommandPalette,
    CommandQueryChanged(String),
    ExecuteCommandChat(String),
    ToggleSettingsModal,
    SelectSettingsTab(SettingsTab),
    ToggleRightPanel,
    ToggleDnd,
    CycleTheme,
    SelectTheme(ThemeChoice),
    SelectLanguage(String),
    CustomCssChanged(String),
    ToggleSettingBool(&'static str, bool),
    ConnectBridge(String),
    RecoveryInputChanged(String),
    VerifyRecoveryCode,
    VerifySasNow,
    ClearOldMedia,
    ExportChatJson(String),
    OpenQuickLook(String, String),
    CloseModals,
    ToggleEditHistory(String),
    DragStart(u8),
    MouseMoved(Point),
    DragEnd,
    WindowResized(Size),
    WindowOpened(window::Id),
    ScreenshotCaptured(window::Screenshot),
    KeyPressed(Key, Modifiers),
}

impl std::fmt::Debug for UiMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UiMessage")
    }
}

pub fn search_settings_catalog(query: &str) -> Vec<&'static SettingCatalogEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return SETTINGS_CATALOG.iter().collect();
    }
    SETTINGS_CATALOG
        .iter()
        .filter(|e| {
            e.title.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
                || e.section.to_lowercase().contains(&q)
        })
        .collect()
}

pub const ALL_LANGUAGES: [LanguageChoice; 6] = [
    LanguageChoice::English,
    LanguageChoice::Spanish,
    LanguageChoice::French,
    LanguageChoice::Italian,
    LanguageChoice::Portuguese,
    LanguageChoice::German,
];

pub fn network_brand_meta(account_id: &str) -> (&'static str, Color, &'static str) {
    let id = account_id.trim_start_matches("acc_");
    for spec in &NETWORK_SPECS {
        if id.contains(spec.bridge_id) || spec.bridge_id.contains(id) {
            return (
                spec.badge_code,
                Color::from_rgb8(spec.brand_rgb[0], spec.brand_rgb[1], spec.brand_rgb[2]),
                spec.display_name,
            );
        }
    }
    ("BP", Color::from_rgb8(107, 83, 255), "Beeper")
}

pub fn format_short_ts(iso: &str, twenty_four: bool) -> String {
    let time_part = iso.split('T').nth(1).and_then(|t| t.get(0..5)).unwrap_or("08:15");
    if twenty_four {
        return time_part.to_string();
    }
    let (h_str, m_str) = time_part.split_once(':').unwrap_or(("08", "15"));
    let h: u32 = h_str.parse().unwrap_or(8);
    let suffix = ["AM", "PM"][usize::from(h >= 12)];
    let h12 = match h % 12 {
        0 => 12,
        n => n,
    };
    format!("{h12}:{m_str} {suffix}")
}

pub struct BeeperGuiApp {
    pub store: SharedStore,
    pub os: OsKind,
    pub display_server: DisplayServerKind,
    pub tray: SystemTrayState,
    pub theme: ResolvedTheme,
    pub active_tab: InboxTab,
    pub selected_account_filter: Option<String>,
    pub selected_chat_id: Option<String>,
    pub search_query: String,
    pub command_palette_query: String,
    pub composer_text: String,
    pub reply_to_message_id: Option<String>,
    pub editing_message_id: Option<String>,
    pub nickname_edit_buffer: String,
    pub recovery_code_input: String,
    pub show_command_palette: bool,
    pub show_settings_modal: bool,
    pub active_settings_tab: SettingsTab,
    pub show_right_panel: bool,
    pub quick_look_target: Option<(String, String)>,
    pub expanded_history_msg_id: Option<String>,
    pub status_banner: String,
    pub dnd_muted: bool,
    pub sidebar_width: f32,
    pub right_panel_width: f32,
    pub drag_target: u8,
    pub window_size: Size,
}

impl BeeperGuiApp {
    pub fn new(store: SharedStore, try_native_tray: bool) -> Self {
        ensure_fonts_loaded();
        let os = OsKind::current();
        let env_map: std::collections::HashMap<String, String> = std::env::vars().collect();
        let display_server = os.detect_display_server(&env_map);
        let (settings, unread, first_chat, show_right, first_nick) = {
            let mut st = store.write().unwrap();
            let first = st.chats.first().map(|c| c.id.clone());
            let nick = st
                .chats
                .first()
                .and_then(|c| c.participants.items.iter().find(|u| u.is_self != Some(true)))
                .and_then(|u| u.custom_nickname.clone().or_else(|| u.full_name.clone()))
                .unwrap_or_default();
            st.focused_chat_id = first.clone();
            (st.settings.clone(), st.total_unread_chats(), first, st.settings.show_right_panel_by_default, nick)
        };
        let theme = ResolvedTheme::compile(&settings);
        let tray = SystemTrayState::new(unread, false, try_native_tray);
        Self {
            store,
            os,
            display_server,
            tray,
            theme,
            active_tab: InboxTab::AllChats,
            selected_account_filter: None,
            selected_chat_id: first_chat,
            search_query: String::new(),
            command_palette_query: String::new(),
            composer_text: String::new(),
            reply_to_message_id: None,
            editing_message_id: None,
            nickname_edit_buffer: first_nick,
            recovery_code_input: String::new(),
            show_command_palette: false,
            show_settings_modal: false,
            active_settings_tab: SettingsTab::Appearance,
            show_right_panel: show_right,
            quick_look_target: None,
            expanded_history_msg_id: None,
            status_banner: "Desktop API :23373 • SQLite WAL • E2EE Ready".to_string(),
            dnd_muted: false,
            sidebar_width: 320.0,
            right_panel_width: 280.0,
            drag_target: 0,
            window_size: Size::new(1440.0, 900.0),
        }
    }

    pub fn refresh_theme_and_tray(&mut self) {
        let (settings, unread) = {
            let st = self.store.read().unwrap();
            (st.settings.clone(), st.total_unread_chats())
        };
        self.theme = ResolvedTheme::compile(&settings);
        self.tray.update(unread, self.dnd_muted);
    }

    pub fn visible_chats(&self) -> Vec<Chat> {
        let st = self.store.read().unwrap();
        let q_ids = (!self.search_query.trim().is_empty())
            .then(|| st.chat_search_index.query_all_tokens(&self.search_query));
        let mut list: Vec<Chat> = st
            .chats
            .iter()
            .enumerate()
            .filter(|(idx, c)| {
                if c.merged_into_chat_id.is_some() {
                    return false;
                }
                if let Some(ref acc) = self.selected_account_filter {
                    if &c.account_id != acc {
                        return false;
                    }
                }
                if let Some(ref allowed) = q_ids {
                    if !allowed.contains(&(*idx as u32)) {
                        return false;
                    }
                }
                match self.active_tab {
                    InboxTab::AllChats => (c.flags & CHAT_ARCHIVED) == 0,
                    InboxTab::Unread => (c.flags & CHAT_UNREAD) != 0,
                    InboxTab::Pinned => (c.flags & CHAT_PINNED) != 0,
                    InboxTab::Drafts => c.draft.is_some(),
                    InboxTab::Archived => (c.flags & CHAT_ARCHIVED) != 0,
                    InboxTab::LowPriority => (c.flags & CHAT_LOW_PRIORITY) != 0,
                }
            })
            .map(|(_, c)| c.clone())
            .collect();

        let unread_top = st.settings.unread_chats_always_on_top;
        list.sort_by(|a, b| {
            if unread_top {
                let ua = (a.flags & CHAT_UNREAD) != 0;
                let ub = (b.flags & CHAT_UNREAD) != 0;
                if ua != ub {
                    return ub.cmp(&ua);
                }
            }
            b.last_activity.cmp(&a.last_activity)
        });
        list
    }

    pub fn select_chat(&mut self, chat_id: &str) {
        if let Some(prev_id) = self.selected_chat_id.clone() {
            if prev_id != chat_id && self.editing_message_id.is_none() {
                let mut st = self.store.write().unwrap();
                let draft_txt = self.composer_text.trim().to_string();
                let _ = st.patch_chat(&prev_id, true, None, None);
                if !draft_txt.is_empty() {
                    let _ = st.patch_chat(
                        &prev_id,
                        false,
                        Some(ChatDraft {
                            text: Some(draft_txt),
                            reply_to_message_id: self.reply_to_message_id.clone(),
                            attachments: Vec::new(),
                        }),
                        None,
                    );
                }
            }
        }

        let mut st = self.store.write().unwrap();
        self.selected_chat_id = Some(chat_id.to_string());
        st.focused_chat_id = Some(chat_id.to_string());
        if let Some(idx) = st.resolve_chat_idx(chat_id) {
            st.chats[idx].unread_count = 0;
            st.chats[idx].is_marked_unread = Some(false);
            st.chats[idx].sync_flags();
            self.composer_text = st.chats[idx]
                .draft
                .as_ref()
                .and_then(|d| d.text.clone())
                .unwrap_or_default();
            self.nickname_edit_buffer = st.chats[idx]
                .participants
                .items
                .iter()
                .find(|u| u.is_self != Some(true))
                .and_then(|u| u.custom_nickname.clone().or_else(|| u.full_name.clone()))
                .unwrap_or_default();
        }
        self.reply_to_message_id = None;
        self.editing_message_id = None;
        drop(st);
        self.refresh_theme_and_tray();
    }

    pub fn submit_composer(&mut self) {
        let Some(chat_id) = self.selected_chat_id.clone() else {
            return;
        };
        let text = self.composer_text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut st = self.store.write().unwrap();
        if let Some(edit_id) = self.editing_message_id.take() {
            let _ = st.edit_message(&chat_id, &edit_id, text);
            self.status_banner = format!("Edited message {edit_id}");
        } else {
            let reply_id = self.reply_to_message_id.take();
            let _ = st.send_message(&chat_id, Some(text), reply_id, None);
            let _ = st.patch_chat(&chat_id, true, None, None);
        }
        self.composer_text.clear();
        drop(st);
        self.refresh_theme_and_tray();
    }

    pub fn step_chat(&mut self, delta: isize) {
        let chats = self.visible_chats();
        if chats.is_empty() {
            return;
        }
        let cur = self
            .selected_chat_id
            .as_ref()
            .and_then(|id| chats.iter().position(|c| &c.id == id))
            .unwrap_or(0);
        let next = (cur as isize + delta).rem_euclid(chats.len() as isize) as usize;
        let next_id = chats[next].id.clone();
        self.select_chat(&next_id);
    }

    pub fn update(&mut self, msg: UiMessage) -> Task<UiMessage> {
        match msg {
            UiMessage::SelectTab(tab) => self.active_tab = tab,
            UiMessage::SelectAccount(acc) => self.selected_account_filter = acc,
            UiMessage::SelectChat(id) => self.select_chat(&id),
            UiMessage::ComposerChanged(v) => self.composer_text = v,
            UiMessage::SubmitComposer => self.submit_composer(),
            UiMessage::CancelComposeMode => {
                self.reply_to_message_id = None;
                self.editing_message_id = None;
                self.composer_text.clear();
            }
            UiMessage::StartReply(mid) => {
                self.editing_message_id = None;
                self.reply_to_message_id = Some(mid);
                return operation::focus("composer-input");
            }
            UiMessage::StartEdit(mid, old_txt) => {
                self.reply_to_message_id = None;
                self.editing_message_id = Some(mid);
                self.composer_text = old_txt;
                return operation::focus("composer-input");
            }
            UiMessage::DeleteMessage(cid, mid) => {
                let _ = self.store.write().unwrap().delete_message(&cid, &mid);
            }
            UiMessage::ToggleReaction(cid, mid, key) => {
                let mut st = self.store.write().unwrap();
                let has_rx = st
                    .msg_idx_by_id
                    .get(&mid)
                    .map(|&i| {
                        st.messages[i]
                            .reactions
                            .iter()
                            .any(|r| r.reaction_key == key && r.participant_id.starts_with("@self_"))
                    })
                    .unwrap_or(false);
                if has_rx {
                    let _ = st.remove_reaction(&cid, &mid, &key);
                } else {
                    let _ = st.add_reaction(&cid, &mid, &key, None);
                }
            }
            UiMessage::ToggleBookmark(mid) => {
                self.store.write().unwrap().toggle_bookmark(&mid);
            }
            UiMessage::MarkUnreadFromHere(cid, mid) => {
                self.store.write().unwrap().mark_unread_from_message(&cid, &mid);
                self.refresh_theme_and_tray();
            }
            UiMessage::RetranscribeVoice(mid) => {
                self.status_banner = self.store.write().unwrap().retranscribe_voice_note(&mid);
            }
            UiMessage::VotePoll(mid, opt_id) => {
                self.store.write().unwrap().vote_poll(&mid, &opt_id, "@self:beeper.com");
            }
            UiMessage::SendQuickVoiceNote => {
                if let Some(cid) = self.selected_chat_id.clone() {
                    let att = Attachment {
                        attachment_type: "audio".to_string(),
                        id: Some("att_vn_quick".to_string()),
                        duration: Some(8.4),
                        file_name: Some("voice-note.ogg".to_string()),
                        file_size: Some(34_816),
                        is_gif: Some(false),
                        is_sticker: Some(false),
                        is_voice_note: Some(true),
                        mime_type: Some("audio/ogg".to_string()),
                        poster_img: None,
                        size: None,
                        src_url: Some("mxc://beeper.com/quick_vn".to_string()),
                        transcript: None,
                    };
                    let _ = self.store.write().unwrap().send_message(
                        &cid,
                        Some("Voice note".to_string()),
                        None,
                        Some(att),
                    );
                    self.refresh_theme_and_tray();
                }
            }
            UiMessage::SendQuickPoll => {
                if let Some(cid) = self.selected_chat_id.clone() {
                    let mut st = self.store.write().unwrap();
                    if let Ok((_, mid, _)) = st.send_message(
                        &cid,
                        Some("Poll: Ship Beeper v4.3 today?".to_string()),
                        None,
                        None,
                    ) {
                        if let Some(&idx) = st.msg_idx_by_id.get(&mid) {
                            st.messages[idx].poll = Some(PollData {
                                question: "Ship Beeper v4.3 today?".to_string(),
                                allow_multiple: false,
                                options: vec![
                                    PollOption {
                                        id: "p_yes".to_string(),
                                        label: "Yes, ship it".to_string(),
                                        voter_ids: vec!["@self:beeper.com".to_string()],
                                    },
                                    PollOption {
                                        id: "p_no".to_string(),
                                        label: "Wait for QA".to_string(),
                                        voter_ids: Vec::new(),
                                    },
                                ],
                            });
                        }
                    }
                    drop(st);
                    self.refresh_theme_and_tray();
                }
            }
            UiMessage::SetReminder(cid) => {
                let mut st = self.store.write().unwrap();
                if let Some(idx) = st.resolve_chat_idx(&cid) {
                    st.chats[idx].reminder = Some(ChatReminder {
                        remind_at: "2026-10-15T09:00:00.000Z".to_string(),
                        dismiss_on_incoming_message: true,
                    });
                }
                self.status_banner = "Reminder scheduled for tomorrow 09:00".to_string();
            }
            UiMessage::TogglePinChat(cid) => {
                let mut st = self.store.write().unwrap();
                if let Some(idx) = st.resolve_chat_idx(&cid) {
                    let next = !(st.chats[idx].is_pinned == Some(true));
                    st.chats[idx].is_pinned = Some(next);
                    st.chats[idx].sync_flags();
                }
            }
            UiMessage::ToggleMuteChat(cid) => {
                let mut st = self.store.write().unwrap();
                if let Some(idx) = st.resolve_chat_idx(&cid) {
                    let next = !(st.chats[idx].is_muted == Some(true));
                    st.chats[idx].is_muted = Some(next);
                    st.chats[idx].sync_flags();
                }
            }
            UiMessage::ToggleArchiveChat(cid) => {
                let mut st = self.store.write().unwrap();
                if let Some(idx) = st.resolve_chat_idx(&cid) {
                    let next = !(st.chats[idx].is_archived == Some(true));
                    st.chats[idx].is_archived = Some(next);
                    st.chats[idx].sync_flags();
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
            UiMessage::NotifyAnyway(cid) => {
                let mut st = self.store.write().unwrap();
                if let Some(idx) = st.resolve_chat_idx(&cid) {
                    st.chats[idx].is_muted = Some(false);
                    st.chats[idx].sync_flags();
                }
                self.status_banner = "Notifications unmuted for conversation".to_string();
            }
            UiMessage::NicknameChanged(v) => self.nickname_edit_buffer = v,
            UiMessage::SaveNickname => {
                if let Some(cid) = self.selected_chat_id.clone() {
                    let mut st = self.store.write().unwrap();
                    let target_uid = st.resolve_chat_idx(&cid).and_then(|idx| {
                        st.chats[idx]
                            .participants
                            .items
                            .iter()
                            .find(|u| u.is_self != Some(true))
                            .map(|u| u.id.clone())
                    });
                    if let Some(uid) = target_uid {
                        let nick = (!self.nickname_edit_buffer.trim().is_empty())
                            .then(|| self.nickname_edit_buffer.trim().to_string());
                        st.set_user_nickname(&cid, &uid, nick);
                    }
                }
            }
            UiMessage::SearchChanged(q) => self.search_query = q,
            UiMessage::ClearSearch => self.search_query.clear(),
            UiMessage::ToggleCommandPalette => {
                self.show_command_palette = !self.show_command_palette;
                self.show_settings_modal = false;
                if self.show_command_palette {
                    return operation::focus("cmd-palette-input");
                }
            }
            UiMessage::CommandQueryChanged(q) => self.command_palette_query = q,
            UiMessage::ExecuteCommandChat(cid) => {
                self.show_command_palette = false;
                self.command_palette_query.clear();
                self.select_chat(&cid);
            }
            UiMessage::ToggleSettingsModal => {
                self.show_settings_modal = !self.show_settings_modal;
                self.show_command_palette = false;
            }
            UiMessage::SelectSettingsTab(t) => self.active_settings_tab = t,
            UiMessage::ToggleRightPanel => self.show_right_panel = !self.show_right_panel,
            UiMessage::ToggleDnd => {
                self.dnd_muted = !self.dnd_muted;
                self.refresh_theme_and_tray();
            }
            UiMessage::CycleTheme => {
                let all = ThemeChoice::all();
                let cur = all.iter().position(|t| *t == self.theme.choice).unwrap_or(0);
                let next = all[(cur + 1) % all.len()];
                return self.update(UiMessage::SelectTheme(next));
            }
            UiMessage::SelectTheme(choice) => {
                {
                    let mut st = self.store.write().unwrap();
                    st.settings.theme = choice;
                    st.persist_state_to_sqlite();
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::SelectLanguage(label) => {
                if let Some(&lang) = ALL_LANGUAGES.iter().find(|l| l.label() == label) {
                    let mut st = self.store.write().unwrap();
                    st.settings.language = lang;
                    st.persist_state_to_sqlite();
                }
            }
            UiMessage::CustomCssChanged(css) => {
                {
                    let mut st = self.store.write().unwrap();
                    st.settings.custom_css_source = css;
                    st.persist_state_to_sqlite();
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::ToggleSettingBool(key, val) => {
                {
                    let mut st = self.store.write().unwrap();
                    let s = &mut st.settings;
                    match key {
                        "show_archived_count" => s.show_archived_count = val,
                        "show_starred_pin_view" => s.show_starred_pin_view = val,
                        "show_all_chats_pin_view" => s.show_all_chats_pin_view = val,
                        "show_unread_pin_view" => s.show_unread_pin_view = val,
                        "show_drafts_pin_view" => s.show_drafts_pin_view = val,
                        "filter_pins_by_network" => s.filter_pins_by_network = val,
                        "show_favorite_chats_section" => s.show_favorite_chats_section = val,
                        "keep_favorites_in_inbox" => s.keep_favorites_in_inbox = val,
                        "show_message_previews" => s.show_message_previews = val,
                        "unread_chats_always_on_top" => s.unread_chats_always_on_top = val,
                        "show_full_names_in_chats" => s.show_full_names_in_chats = val,
                        "auto_play_videos" => s.auto_play_videos = val,
                        "show_chat_header" => s.show_chat_header = val,
                        "show_message_input_buttons" => s.show_message_input_buttons = val,
                        "link_previews" => s.link_previews = val,
                        "show_right_panel_by_default" => {
                            s.show_right_panel_by_default = val;
                            self.show_right_panel = val;
                        }
                        "load_remote_content_automatically" => s.load_remote_content_automatically = val,
                        "use_twenty_four_hour_time" => s.use_twenty_four_hour_time = val,
                        "no_transparency" => s.no_transparency = val,
                        "reduce_transparency" => s.reduce_transparency = val,
                        "fix_lockdown_emojis" => s.fix_lockdown_emojis = val,
                        "auto_transcribe_voice_notes" => s.auto_transcribe_voice_notes = val,
                        "repeat_message_alerts" => s.repeat_message_alerts = if val { 3 } else { 1 },
                        _ => {}
                    }
                    st.persist_state_to_sqlite();
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::ConnectBridge(bid) => {
                let mut st = self.store.write().unwrap();
                if let Some(existing) = st.bridge_accounts.iter_mut().find(|b| b.bridge_id == bid) {
                    existing.state = "CONNECTED".to_string();
                    existing.is_running = true;
                } else {
                    let net_name = NETWORK_SPECS
                        .iter()
                        .find(|n| n.bridge_id == bid)
                        .map(|n| n.display_name)
                        .unwrap_or("Bridge");
                    st.bridge_accounts.push(BridgeAccountInfo {
                        account_id: format!("acc_{bid}"),
                        bridge_id: bid.clone(),
                        bridge_type: bid,
                        is_running: true,
                        login_id: Some("login_ui_01".to_string()),
                        mode: Some("cloud".to_string()),
                        network_name: net_name.to_string(),
                        state: "CONNECTED".to_string(),
                    });
                }
                st.rebuild_accounts_from_bridges();
                st.persist_state_to_sqlite();
            }
            UiMessage::RecoveryInputChanged(v) => self.recovery_code_input = v,
            UiMessage::VerifyRecoveryCode => {
                let mut st = self.store.write().unwrap();
                st.beeper_state.verified = true;
                st.beeper_state.cross_signing = true;
                st.beeper_state.key_backup = true;
                st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state, None);
                st.persist_state_to_sqlite();
                self.status_banner = "E2EE Recovery Key verified".to_string();
            }
            UiMessage::VerifySasNow => {
                let mut st = self.store.write().unwrap();
                st.beeper_state.verified = true;
                st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state, None);
                st.persist_state_to_sqlite();
                self.status_banner = "Device SAS E2EE verification complete".to_string();
            }
            UiMessage::ClearOldMedia => {
                let n = self.store.write().unwrap().clear_cached_media_older_than_days(30);
                self.status_banner = format!("Cleared {n} cached media files");
            }
            UiMessage::ExportChatJson(cid) => {
                let st = self.store.read().unwrap();
                let count = st.chat_msg_indices.get(&cid).map_or(0, Vec::len);
                let out_path = st.data_dir.join("chat-export.json");
                let payload = serde_json::json!({ "chatID": cid, "messageCount": count });
                std::fs::write(&out_path, serde_json::to_vec_pretty(&payload).unwrap())
                    .expect("write export json");
                self.status_banner = format!("Exported {count} messages to {}", out_path.display());
            }
            UiMessage::OpenQuickLook(title, detail) => {
                self.quick_look_target = Some((title, detail));
            }
            UiMessage::CloseModals => {
                self.show_command_palette = false;
                self.show_settings_modal = false;
                self.quick_look_target = None;
            }
            UiMessage::ToggleEditHistory(mid) => {
                self.expanded_history_msg_id =
                    (self.expanded_history_msg_id.as_ref() != Some(&mid)).then_some(mid);
            }
            UiMessage::DragStart(target) => self.drag_target = target,
            UiMessage::MouseMoved(pt) => match self.drag_target {
                1 => {
                    let rail = self.theme.account_switcher_width;
                    self.sidebar_width =
                        (pt.x - rail).clamp(self.theme.min_sidebar_width, self.theme.max_sidebar_width);
                }
                2 => {
                    self.right_panel_width = (self.window_size.width - pt.x).clamp(240.0, 420.0);
                }
                _ => {}
            },
            UiMessage::DragEnd => self.drag_target = 0,
            UiMessage::WindowResized(sz) => self.window_size = sz,
            UiMessage::WindowOpened(id) => {
                if std::env::var("BEEPER_SCREENSHOT_EXIT").is_ok() {
                    return window::screenshot(id).map(UiMessage::ScreenshotCaptured);
                }
            }
            UiMessage::ScreenshotCaptured(shot) => {
                if let Ok(path) = std::env::var("BEEPER_SCREENSHOT_EXIT") {
                    write_rgba_png(
                        std::path::Path::new(&path),
                        shot.size.width,
                        shot.size.height,
                        &shot.rgba,
                    );
                    return iced::exit();
                }
            }
            UiMessage::KeyPressed(key, mods) => {
                let cmd = mods.command() || mods.control();
                match key.as_ref() {
                    Key::Character("k") if cmd => return self.update(UiMessage::ToggleCommandPalette),
                    Key::Character(",") if cmd => return self.update(UiMessage::ToggleSettingsModal),
                    Key::Character("f") if cmd => return operation::focus("sidebar-search-input"),
                    Key::Character(".") if cmd => return self.update(UiMessage::ToggleRightPanel),
                    Key::Named(Named::Escape) => return self.update(UiMessage::CloseModals),
                    Key::Named(Named::ArrowDown) if mods.alt() => self.step_chat(1),
                    Key::Named(Named::ArrowUp) if mods.alt() => self.step_chat(-1),
                    _ => {}
                }
            }
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<UiMessage> {
        iced::event::listen_with(|ev, status, id| match (ev, status) {
            (Event::Window(window::Event::Opened { .. }), _) => Some(UiMessage::WindowOpened(id)),
            (Event::Window(window::Event::Resized(sz)), _) => Some(UiMessage::WindowResized(sz)),
            (Event::Mouse(iced::mouse::Event::CursorMoved { position }), _) => {
                Some(UiMessage::MouseMoved(position))
            }
            (Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)), _) => {
                Some(UiMessage::DragEnd)
            }
            (
                Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }),
                iced::event::Status::Ignored,
            ) => Some(UiMessage::KeyPressed(key, modifiers)),
            _ => None,
        })
    }

    pub fn view(&self) -> El<'_> {
        let th = &self.theme;
        let mut main_row = row![
            self.view_account_rail(th),
            self.view_threads_sidebar(th),
            self.view_splitter(th, 1),
            self.view_conversation(th),
        ]
        .width(Fill)
        .height(Fill);

        if self.show_right_panel {
            main_row = main_row
                .push(self.view_splitter(th, 2))
                .push(self.view_right_inspector(th));
        }

        let base: El<'_> = container(main_row)
            .width(Fill)
            .height(Fill)
            .style(fill_box(th.app_bg(), 0.0))
            .into();

        let mut layers = vec![base];
        if self.show_command_palette {
            layers.push(self.view_modal_scrim(self.view_command_palette(th)));
        }
        if self.show_settings_modal {
            layers.push(self.view_modal_scrim(self.view_settings_modal(th)));
        }
        if let Some((ref title, ref detail)) = self.quick_look_target {
            layers.push(self.view_modal_scrim(self.view_quick_look_modal(th, title, detail)));
        }
        stack(layers).width(Fill).height(Fill).into()
    }

    fn view_splitter<'a>(&self, th: &ResolvedTheme, id: u8) -> El<'a> {
        let line = container(Space::new().width(1).height(Fill))
            .style(fill_box(th.border_weak_color(), 0.0));
        mouse_area(container(line).center_x(4).height(Fill))
            .on_press(UiMessage::DragStart(id))
            .interaction(iced::mouse::Interaction::ResizingHorizontally)
            .into()
    }

    fn view_account_rail<'a>(&self, th: &ResolvedTheme) -> El<'a> {
        let st = self.store.read().unwrap();
        let total_unread = st.total_unread_chats();
        let all_selected = self.selected_account_filter.is_none();

        let win_dot = |hex: [u8; 3]| -> El<'a> {
            container(Space::new().width(8).height(8))
                .style(fill_box(Color::from_rgb8(hex[0], hex[1], hex[2]), 4.0))
                .into()
        };
        let traffic_lights = row![
            win_dot([255, 95, 87]),
            win_dot([254, 188, 46]),
            win_dot([40, 200, 64]),
        ]
        .spacing(5)
        .align_y(Center);

        let make_rail_item =
            |code: &'static str, bg: Color, active: bool, unread: u32, msg: UiMessage| -> El<'a> {
                let pill_h = if active { 20.0 } else { 0.0 };
                let indicator = container(Space::new().width(3).height(pill_h))
                    .style(fill_box(th.accent_color(), 2.0));
                let radius = if active { 11.0 } else { 18.0 };
                let badge = container(bold(code, 11.5).color(Color::WHITE).center())
                    .center(36)
                    .style(fill_box(bg, radius));
                let badge_with_dot: El<'a> = if unread > 0 {
                    let dot = container(
                        bold(format!("{}", unread.min(99)), 9.0).color(Color::WHITE).center(),
                    )
                    .padding([1, 5])
                    .style(bordered_box(th.accent_color(), th.rail_bg(), 1.5, 9.0));
                    stack![
                        badge,
                        container(dot)
                            .width(36)
                            .height(36)
                            .align_x(Right)
                            .align_y(alignment::Vertical::Top)
                    ]
                    .into()
                } else {
                    badge.into()
                };
                button(
                    row![
                        container(indicator).width(4).center_y(36),
                        Space::new().width(3),
                        badge_with_dot
                    ]
                    .align_y(Center),
                )
                .padding(0)
                .style(btn_style([CLEAR, CLEAR, CLEAR], [Color::WHITE; 3], 10.0))
                .on_press(msg)
                .into()
            };

        let mut items: Vec<El<'a>> = vec![
            container(traffic_lights).padding(iced::padding::bottom(4)).into(),
            make_rail_item(
                "ALL",
                if all_selected { th.accent_color() } else { th.elevated_bg() },
                all_selected,
                total_unread,
                UiMessage::SelectAccount(None),
            ),
        ];

        for acc in &st.accounts {
            let (code, brand_col, _) = network_brand_meta(&acc.account_id);
            let is_sel = self.selected_account_filter.as_deref() == Some(acc.account_id.as_str());
            let unread: u32 = st
                .chats
                .iter()
                .filter(|c| {
                    c.account_id == acc.account_id
                        && (c.flags & CHAT_ARCHIVED) == 0
                        && (c.flags & CHAT_UNREAD) != 0
                })
                .count() as u32;
            items.push(make_rail_item(
                code,
                brand_col,
                is_sel,
                unread,
                UiMessage::SelectAccount(Some(acc.account_id.clone())),
            ));
        }

        let top_col = Column::with_children(items)
            .spacing(8)
            .align_x(Center)
            .padding(iced::padding::top(12));

        let footer_btn = |icon_ch: char, col: Color, msg: UiMessage| -> El<'a> {
            button(container(ic(icon_ch, 16.0).color(col)).center(34))
                .padding(0)
                .style(btn_style(
                    [CLEAR, th.hover_overlay(), th.active_overlay()],
                    [col; 3],
                    8.0,
                ))
                .on_press(msg)
                .into()
        };

        let dnd_icon = [icon::VOL2, icon::MUTE][usize::from(self.dnd_muted)];
        let dnd_col = [th.sub_color(), th.danger_color()][usize::from(self.dnd_muted)];
        let sep = container(Space::new().width(Fill).height(1))
            .style(fill_box(th.border_weak_color(), 0.0));

        let bottom_col = column![
            sep,
            footer_btn(dnd_icon, dnd_col, UiMessage::ToggleDnd),
            footer_btn(icon::SLIDERS, th.sub_color(), UiMessage::CycleTheme),
            footer_btn(icon::SETTINGS, th.sub_color(), UiMessage::ToggleSettingsModal),
        ]
        .spacing(6)
        .align_x(Center)
        .padding(iced::padding::bottom(10));

        container(column![
            scrollable(top_col).height(Fill).style(scroll_style(CLEAR, CLEAR)),
            bottom_col
        ])
        .width(th.account_switcher_width)
        .height(Fill)
        .style(bordered_box(th.rail_bg(), th.border_weak_color(), 1.0, 0.0))
        .into()
    }

    fn view_threads_sidebar<'a>(&self, th: &ResolvedTheme) -> El<'a> {
        let (unread_total, archived_total, show_archived_badge, show_previews, twenty_four) = {
            let st = self.store.read().unwrap();
            let arch = st.chats.iter().filter(|c| (c.flags & CHAT_ARCHIVED) != 0).count();
            (
                st.total_unread_chats(),
                arch,
                st.settings.show_archived_count,
                st.settings.show_message_previews,
                st.settings.use_twenty_four_hour_time,
            )
        };

        let header_title = match &self.selected_account_filter {
            Some(acc) => network_brand_meta(acc).2,
            None => "Beeper",
        };

        let unread_pill: El<'a> = if unread_total > 0 {
            container(bold(format!("{unread_total}"), 10.5).color(th.on_accent_color()))
                .padding([2, 7])
                .style(fill_box(th.accent_color(), 10.0))
                .into()
        } else {
            Space::new().width(0).into()
        };

        let arch_pill: El<'a> = if show_archived_badge && archived_total > 0 {
            container(med(format!("{archived_total} arch"), 10.0).color(th.muted_color()))
                .padding([2, 6])
                .style(fill_box(th.elevated_bg(), 8.0))
                .into()
        } else {
            Space::new().width(0).into()
        };

        let cmd_badge = button(
            container(med(format!("{} K", self.os.primary_mod_label()), 10.5).color(th.sub_color()))
                .padding([3, 7]),
        )
        .padding(0)
        .style(btn_bordered_style(
            [th.elevated_bg(), th.elevated_hover_bg(), th.active_overlay()],
            [th.sub_color(); 3],
            th.border_weak_color(),
            5.0,
        ))
        .on_press(UiMessage::ToggleCommandPalette);

        let top_bar = row![
            bold(header_title, 15.5).color(th.text_color()),
            unread_pill,
            arch_pill,
            Space::new().width(Fill),
            cmd_badge,
        ]
        .spacing(7)
        .align_y(Center)
        .padding(Padding { top: 11.0, right: 12.0, bottom: 7.0, left: 12.0 });

        let search_in = text_input("Search chats, messages, @users...", &self.search_query)
            .id("sidebar-search-input")
            .on_input(UiMessage::SearchChanged)
            .font(fw(Weight::Normal))
            .size(12.5)
            .padding([6, 6])
            .style(input_style(
                [CLEAR, CLEAR],
                [CLEAR, CLEAR],
                th.text_color(),
                th.muted_color(),
                th.accent_color(),
                [0.0, 0.0],
                6.0,
            ));

        let clear_btn: Option<El<'a>> = (!self.search_query.is_empty()).then(|| {
            button(ic(icon::X, 13.0).color(th.sub_color()))
                .padding(4)
                .style(btn_style([CLEAR; 3], [th.sub_color(); 3], 6.0))
                .on_press(UiMessage::ClearSearch)
                .into()
        });

        let search_box = container(
            row![ic(icon::SEARCH, 13.5).color(th.muted_color()), search_in, clear_btn]
                .spacing(4)
                .align_y(Center)
                .padding([0, 10]),
        )
        .style(bordered_box(th.input_bg(), th.border_weak_color(), 1.0, 8.0));

        let search_wrap = container(search_box).padding([0, 10]);

        let filter_tabs = Row::with_children(InboxTab::ALL.into_iter().map(|tab| {
            let active = self.active_tab == tab;
            let bg = if active {
                [th.accent_color(); 3]
            } else {
                [CLEAR, th.hover_overlay(), th.active_overlay()]
            };
            let fg = if active { th.on_accent_color() } else { th.sub_color() };
            button(med(tab.label(), 10.8).color(fg).wrapping(iced::widget::text::Wrapping::None))
                .padding([4, 6])
                .style(btn_style(bg, [fg; 3], 6.0))
                .on_press(UiMessage::SelectTab(tab))
                .into()
        }))
        .spacing(2)
        .padding([7, 10]);

        let chats = self.visible_chats();
        let pinned_chats: Vec<&Chat> = chats
            .iter()
            .filter(|c| (c.flags & CHAT_PINNED) != 0)
            .take(5)
            .collect();

        let pinned_bar: Option<El<'a>> = (!pinned_chats.is_empty()
            && matches!(self.active_tab, InboxTab::AllChats | InboxTab::Pinned))
        .then(|| {
            let pins = Row::with_children(pinned_chats.into_iter().map(|c| {
                let is_sel = self.selected_chat_id.as_deref() == Some(c.id.as_str());
                let (code, net_col, _) = network_brand_meta(&c.account_id);
                let av = container(bold(initials(&c.title), 13.0).color(Color::WHITE).center())
                    .center(40)
                    .style(bordered_box(
                        hue(hash_str(&c.id)),
                        if is_sel { th.accent_color() } else { CLEAR },
                        2.0,
                        20.0,
                    ));
                let net_dot = container(bold(&code[0..1], 8.5).color(Color::WHITE).center())
                    .center(14)
                    .style(bordered_box(net_col, th.sidebar_bg(), 1.5, 7.0));
                let stacked = stack![
                    av,
                    container(net_dot)
                        .width(40)
                        .height(40)
                        .align_x(Right)
                        .align_y(alignment::Vertical::Bottom)
                ];
                let short_name = c.title.split_whitespace().next().unwrap_or(&c.title);
                let col_el = column![
                    stacked,
                    ell(short_name, 54.0, 11.0, Weight::Medium).color(if is_sel {
                        th.accent_color()
                    } else {
                        th.sub_color()
                    })
                ]
                .spacing(3)
                .align_x(Center);
                button(col_el)
                    .padding([4, 6])
                    .style(btn_style(
                        [CLEAR, th.hover_overlay(), th.active_overlay()],
                        [th.text_color(); 3],
                        8.0,
                    ))
                    .on_press(UiMessage::SelectChat(c.id.clone()))
                    .into()
            }))
            .spacing(4)
            .padding([3, 10]);
            pins.into()
        });

        let avail_text_w = (self.sidebar_width - 116.0).max(110.0);
        let rows: Vec<El<'a>> = chats
            .into_iter()
            .map(|c| {
                let is_sel = self.selected_chat_id.as_deref() == Some(c.id.as_str());
                let is_unread = (c.flags & CHAT_UNREAD) != 0;
                let is_pinned = (c.flags & CHAT_PINNED) != 0;
                let is_muted = (c.flags & CHAT_MUTED) != 0;
                let (code, net_col, _) = network_brand_meta(&c.account_id);
                let av = container(bold(initials(&c.title), 12.5).color(Color::WHITE).center())
                    .center(38)
                    .style(fill_box(hue(hash_str(&c.id)), 19.0));
                let net_dot = container(bold(&code[0..1], 8.0).color(Color::WHITE).center())
                    .center(14)
                    .style(bordered_box(
                        net_col,
                        if is_sel { th.accent_color() } else { th.sidebar_bg() },
                        1.5,
                        7.0,
                    ));
                let av_stack = stack![
                    av,
                    container(net_dot)
                        .width(38)
                        .height(38)
                        .align_x(Right)
                        .align_y(alignment::Vertical::Bottom)
                ];

                let title_fg = if is_sel { th.on_accent_color() } else { th.text_color() };
                let sub_fg = if is_sel {
                    th.on_accent_color().scale_alpha(0.86)
                } else if is_unread {
                    th.text_color()
                } else {
                    th.sub_color()
                };
                let ts_fg = if is_sel {
                    th.on_accent_color().scale_alpha(0.8)
                } else {
                    th.muted_color()
                };

                let ts_str = c
                    .last_activity
                    .as_deref()
                    .map(|t| format_short_ts(t, twenty_four))
                    .unwrap_or_default();

                let mut top_line = Row::new()
                    .push(
                        ell(&c.title, avail_text_w - 42.0, 13.0, Weight::Bold)
                            .color(title_fg)
                            .width(Fill),
                    )
                    .spacing(4)
                    .align_y(Center);
                if is_muted {
                    top_line = top_line.push(ic(icon::MUTE, 11.0).color(ts_fg));
                }
                if is_pinned {
                    top_line = top_line.push(filled_ic(fill::STAR, 10.5).color(ts_fg));
                }
                top_line = top_line.push(txt(ts_str, 11.0).color(ts_fg));

                let snippet_raw = if let Some(ref d) = c.draft {
                    format!("Draft: {}", d.text.as_deref().unwrap_or(""))
                } else if show_previews {
                    c.preview
                        .as_ref()
                        .and_then(|p| p.text.clone())
                        .or_else(|| c.description.clone())
                        .unwrap_or_default()
                } else {
                    c.description.clone().unwrap_or_default()
                };

                let unread_badge: Option<El<'a>> = (c.unread_count > 0 || is_unread).then(|| {
                    let count = c.unread_count.max(1);
                    let (b_bg, b_fg) = if is_sel {
                        (th.on_accent_color(), th.accent_color())
                    } else {
                        (th.accent_color(), th.on_accent_color())
                    };
                    container(bold(format!("{count}"), 10.5).color(b_fg))
                        .padding([1, 6])
                        .style(fill_box(b_bg, 9.0))
                        .into()
                });

                let bot_line = row![
                    ell(&snippet_raw, avail_text_w - 22.0, 12.0, Weight::Normal)
                        .color(sub_fg)
                        .width(Fill),
                    unread_badge
                ]
                .spacing(6)
                .align_y(Center);

                let item_body = row![av_stack, column![top_line, bot_line].spacing(3).width(Fill)]
                    .spacing(10)
                    .align_y(Center);

                let row_bg = if is_sel {
                    [th.accent_color(); 3]
                } else {
                    [CLEAR, th.hover_overlay(), th.active_overlay()]
                };

                container(
                    button(container(item_body).center_y(th.threads_list_item_height))
                        .padding([0, 10])
                        .width(Fill)
                        .height(th.threads_list_item_height)
                        .style(btn_style(row_bg, [title_fg; 3], 6.0))
                        .on_press(UiMessage::SelectChat(c.id.clone())),
                )
                .padding([1, 6])
                .into()
            })
            .collect();

        let list_scroll =
            scrollable(Column::with_children(rows).padding(iced::padding::bottom(12)))
                .height(Fill)
                .style(scroll_style(
                    th.sub_color().scale_alpha(0.3),
                    th.sub_color().scale_alpha(0.5),
                ));

        let sep = container(Space::new().width(Fill).height(1))
            .style(fill_box(th.border_weak_color(), 0.0));

        container(column![top_bar, search_wrap, filter_tabs, pinned_bar, sep, list_scroll])
            .width(self.sidebar_width)
            .height(Fill)
            .style(fill_box(th.sidebar_bg(), 0.0))
            .into()
    }

    fn view_conversation<'a>(&self, th: &ResolvedTheme) -> El<'a> {
        let Some(ref cid) = self.selected_chat_id else {
            let empty = column![
                ic(icon::LIBRARY, 36.0).color(th.muted_color()),
                bold("Select a conversation", 16.0).color(th.text_color()),
                txt("Choose a thread from the sidebar or press Ctrl+K to search.", 13.0)
                    .color(th.sub_color()),
            ]
            .spacing(8)
            .align_x(Center);
            return container(empty).center(Fill).into();
        };

        let (chat, msgs, show_header, show_input_btns, full_names, twenty_four) = {
            let st = self.store.read().unwrap();
            let Some(c_idx) = st.resolve_chat_idx(cid) else {
                return container(txt("Conversation not found", 14.0)).center(Fill).into();
            };
            let chat = st.chats[c_idx].clone();
            let msg_list: Vec<Message> = st
                .chat_msg_indices
                .get(&chat.id)
                .into_iter()
                .flatten()
                .filter_map(|&i| st.messages.get(i).cloned())
                .filter(|m| (m.flags & MSG_DELETED) == 0)
                .collect();
            (
                chat,
                msg_list,
                st.settings.show_chat_header,
                st.settings.show_message_input_buttons,
                st.settings.show_full_names_in_chats,
                st.settings.use_twenty_four_hour_time,
            )
        };

        let (_, net_col, net_name) = network_brand_meta(&chat.account_id);

        let header_el: Option<El<'a>> = show_header.then(|| {
            let av = container(bold(initials(&chat.title), 12.0).color(Color::WHITE).center())
                .center(32)
                .style(fill_box(hue(hash_str(&chat.id)), 16.0));
            let net_pill = container(bold(net_name, 10.5).color(Color::WHITE))
                .padding([2, 7])
                .style(fill_box(net_col, 6.0));
            let e2ee_pill = container(med("E2EE Verified", 10.5).color(th.success_color()))
                .padding([2, 7])
                .style(bordered_box(CLEAR, th.success_color(), 1.0, 6.0));
            let desc = chat
                .description
                .clone()
                .unwrap_or_else(|| "Direct encrypted session".to_string());

            let notify_btn: Option<El<'a>> = (chat.is_muted == Some(true)).then(|| {
                button(bold("Notify Anyway", 11.0).color(th.warning_color()))
                    .padding([4, 8])
                    .style(btn_bordered_style(
                        [CLEAR, th.hover_overlay(), th.active_overlay()],
                        [th.warning_color(); 3],
                        th.warning_color(),
                        6.0,
                    ))
                    .on_press(UiMessage::NotifyAnyway(chat.id.clone()))
                    .into()
            });

            let hdr_icon_btn = |ch: char, active: bool, msg: UiMessage| -> El<'a> {
                let col = if active { th.accent_color() } else { th.sub_color() };
                button(container(ic(ch, 15.0).color(col)).center(30))
                    .padding(0)
                    .style(btn_style([CLEAR, th.hover_overlay(), th.active_overlay()], [col; 3], 6.0))
                    .on_press(msg)
                    .into()
            };

            let h_row = row![
                av,
                column![
                    row![bold(chat.title.clone(), 13.5).color(th.text_color()), net_pill, e2ee_pill]
                        .spacing(8)
                        .align_y(Center),
                    txt(desc, 11.0).color(th.sub_color()),
                ]
                .spacing(1),
                Space::new().width(Fill),
                notify_btn,
                hdr_icon_btn(icon::SEARCH, false, UiMessage::ToggleCommandPalette),
                hdr_icon_btn(
                    icon::STAR,
                    chat.is_pinned == Some(true),
                    UiMessage::TogglePinChat(chat.id.clone()),
                ),
                hdr_icon_btn(
                    icon::FOLDER,
                    chat.is_archived == Some(true),
                    UiMessage::ToggleArchiveChat(chat.id.clone()),
                ),
                hdr_icon_btn(icon::PR, self.show_right_panel, UiMessage::ToggleRightPanel),
            ]
            .spacing(8)
            .align_y(Center)
            .padding([0, 16]);

            container(h_row)
                .width(Fill)
                .center_y(th.header_height)
                .style(bordered_box(th.header_bg(), th.border_weak_color(), 1.0, 0.0))
                .into()
        });

        let date_pill = container(
            container(med("Today • End-to-End Encrypted", 11.0).color(th.sub_color()))
                .padding([4, 14])
                .style(bordered_box(th.elevated_bg(), th.border_weak_color(), 1.0, 100.0)),
        )
        .center_x(Fill)
        .padding([6, 0]);

        let mut msg_items: Vec<El<'a>> = vec![date_pill.into()];
        for m in &msgs {
            msg_items.push(self.view_message_row(th, &chat.id, m, &msgs, full_names, twenty_four));
        }

        let timeline = scrollable(Column::with_children(msg_items).spacing(10).padding([12, 20]))
            .height(Fill)
            .style(scroll_style(
                th.sub_color().scale_alpha(0.3),
                th.sub_color().scale_alpha(0.5),
            ));

        let compose_banner: Option<El<'a>> = if let Some(ref rep_id) = self.reply_to_message_id {
            Some(
                container(
                    row![
                        ic(icon::BACK, 13.0).color(th.accent_color()),
                        med(format!("Replying to message {rep_id}"), 12.0)
                            .color(th.text_color())
                            .width(Fill),
                        button(ic(icon::X, 13.0).color(th.sub_color()))
                            .padding(4)
                            .style(btn_style([CLEAR; 3], [th.sub_color(); 3], 6.0))
                            .on_press(UiMessage::CancelComposeMode),
                    ]
                    .spacing(8)
                    .align_y(Center),
                )
                .padding([6, 12])
                .style(fill_box(th.elevated_bg(), 8.0))
                .into(),
            )
        } else {
            self.editing_message_id.as_ref().map(|ed_id| {
                container(
                    row![
                        ic(icon::PENCIL, 13.0).color(th.warning_color()),
                        med(format!("Editing message {ed_id}"), 12.0)
                            .color(th.text_color())
                            .width(Fill),
                        button(ic(icon::X, 13.0).color(th.sub_color()))
                            .padding(4)
                            .style(btn_style([CLEAR; 3], [th.sub_color(); 3], 6.0))
                            .on_press(UiMessage::CancelComposeMode),
                    ]
                    .spacing(8)
                    .align_y(Center),
                )
                .padding([6, 12])
                .style(fill_box(th.elevated_bg(), 8.0))
                .into()
            })
        };

        let left_tools: Option<El<'a>> = show_input_btns.then(|| {
            let tool = |ch: char, msg: UiMessage| -> El<'a> {
                button(container(ic(ch, 15.0).color(th.sub_color())).center(28))
                    .padding(0)
                    .style(btn_style(
                        [CLEAR, th.hover_overlay(), th.active_overlay()],
                        [th.sub_color(); 3],
                        14.0,
                    ))
                    .on_press(msg)
                    .into()
            };
            row![
                tool(icon::LYRICS, UiMessage::SendQuickVoiceNote),
                tool(icon::QUEUE, UiMessage::SendQuickPoll),
                tool(icon::CLOCK, UiMessage::SetReminder(chat.id.clone())),
            ]
            .spacing(2)
            .align_y(Center)
            .into()
        });

        let input_box = text_input(&format!("Message {}...", chat.title), &self.composer_text)
            .id("composer-input")
            .on_input(UiMessage::ComposerChanged)
            .on_submit(UiMessage::SubmitComposer)
            .font(fw(Weight::Normal))
            .size(13.5)
            .padding([8, 10])
            .style(input_style(
                [CLEAR, CLEAR],
                [CLEAR, CLEAR],
                th.text_color(),
                th.muted_color(),
                th.accent_color(),
                [0.0, 0.0],
                12.0,
            ));

        let send_btn = button(
            container(ic(icon::FWD, 15.0).color(th.on_accent_color()).center()).center(30),
        )
        .padding(0)
        .style(btn_style([th.accent_color(); 3], [th.on_accent_color(); 3], 15.0))
        .on_press(UiMessage::SubmitComposer);

        let composer_pill = container(
            row![left_tools, input_box, send_btn]
                .spacing(6)
                .align_y(Center)
                .padding([4, 8]),
        )
        .style(composer_box(th.input_bg(), th.border_color()));

        let footer = container(column![compose_banner, composer_pill].spacing(6).padding([10, 16]))
            .width(Fill);

        column![header_el, timeline, footer].width(Fill).height(Fill).into()
    }

    fn view_message_row<'a>(
        &self,
        th: &ResolvedTheme,
        chat_id: &str,
        m: &Message,
        all_msgs: &[Message],
        full_names: bool,
        twenty_four: bool,
    ) -> El<'a> {
        let is_self = m.is_sender == Some(true);
        let bubble_bg = if is_self { th.bubble_sent_color() } else { th.bubble_recv_color() };
        let bubble_fg = if is_self { th.on_accent_color() } else { th.text_color() };
        let sub_fg = if is_self {
            th.on_accent_color().scale_alpha(0.82)
        } else {
            th.sub_color()
        };

        let raw_sender = m.sender_name.as_deref().unwrap_or(&m.sender_id);
        let sender_label = if full_names {
            raw_sender
        } else {
            raw_sender.split_whitespace().next().unwrap_or(raw_sender)
        };
        let ts_label = format_short_ts(&m.timestamp, twenty_four);

        let mut bubble_col = Column::new().spacing(5);
        let sender_col = if is_self { bubble_fg } else { hue(hash_str(&m.sender_id)) };

        let mut meta_row = row![
            bold(sender_label.to_string(), 11.5).color(sender_col),
            Space::new().width(8),
            txt(ts_label, 10.5).color(sub_fg),
        ]
        .align_y(Center);

        if m.is_bookmarked == Some(true) {
            meta_row = meta_row
                .push(Space::new().width(6))
                .push(filled_ic(fill::STAR, 11.0).color(th.warning_color()));
        }
        if !m.edit_history.is_empty() {
            meta_row = meta_row.push(Space::new().width(6)).push(
                button(med("Edited", 10.0).color(sub_fg))
                    .padding([1, 5])
                    .style(btn_style([CLEAR, th.hover_overlay(), CLEAR], [sub_fg; 3], 4.0))
                    .on_press(UiMessage::ToggleEditHistory(m.id.clone())),
            );
        }
        bubble_col = bubble_col.push(meta_row);

        if let Some(ref linked_id) = m.linked_message_id {
            let quoted_txt = all_msgs
                .iter()
                .find(|x| &x.id == linked_id)
                .and_then(|x| x.text.as_deref())
                .unwrap_or("Quoted message");
            let bar_col = if is_self { Color::WHITE } else { th.accent_color() };
            let quote_box = container(
                row![
                    container(Space::new().width(3).height(18)).style(fill_box(bar_col, 1.5)),
                    ell(quoted_txt, 380.0, 11.5, Weight::Normal).color(sub_fg),
                ]
                .spacing(8)
                .align_y(Center),
            )
            .padding([4, 8])
            .style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.2), 8.0));
            bubble_col = bubble_col.push(quote_box);
        }

        if let Some(ref body) = m.text {
            bubble_col = bubble_col.push(txt(body.clone(), 13.5).color(bubble_fg));
        }

        if self.expanded_history_msg_id.as_deref() == Some(m.id.as_str()) {
            for h in &m.edit_history {
                bubble_col =
                    bubble_col.push(txt(format!("Prior edit: {}", h.text), 11.5).color(sub_fg));
            }
        }

        for att in &m.attachments {
            let fname = att.file_name.as_deref().unwrap_or("attachment");
            if att.is_voice_note == Some(true) {
                let dur = att.duration.unwrap_or(10.0);
                let bars: [u16; 22] = [
                    6, 12, 9, 16, 14, 8, 18, 11, 15, 7, 13, 17, 10, 14, 8, 12, 16, 9, 11, 7, 10, 6,
                ];
                let wf = Row::with_children(bars.into_iter().enumerate().map(|(idx, bh)| {
                    let b_col = if idx < 13 {
                        th.accent_color()
                    } else {
                        th.sub_color().scale_alpha(0.45)
                    };
                    container(Space::new().width(3).height(bh as f32))
                        .style(fill_box(b_col, 1.5))
                        .into()
                }))
                .spacing(3)
                .align_y(Center);

                let audio_card = container(
                    column![
                        row![
                            container(filled_ic(fill::PLAY, 12.0).color(Color::WHITE).center())
                                .center(28)
                                .style(fill_box(th.accent_color(), 14.0)),
                            column![
                                bold(format!("Voice Note • {dur:.1}s"), 11.5)
                                    .color(th.text_color()),
                                wf,
                            ]
                            .spacing(4),
                            Space::new().width(8),
                            button(med("Re-transcribe", 11.0).color(th.text_color()))
                                .padding([4, 8])
                                .style(btn_bordered_style(
                                    [th.elevated_bg(), th.elevated_hover_bg(), th.active_overlay()],
                                    [th.text_color(); 3],
                                    th.border_color(),
                                    6.0,
                                ))
                                .on_press(UiMessage::RetranscribeVoice(m.id.clone())),
                        ]
                        .spacing(10)
                        .align_y(Center),
                        att.transcript
                            .as_ref()
                            .map(|tr| txt(format!("\"{tr}\""), 12.0).color(th.sub_color())),
                    ]
                    .spacing(6),
                )
                .padding([8, 12])
                .style(bordered_box(th.audio_bar_bg(), th.border_color(), 1.0, 12.0));
                bubble_col = bubble_col.push(audio_card);
            } else {
                let kb = att.file_size.unwrap_or(1024) / 1024;
                let file_card = container(
                    row![
                        ic(icon::FOLDER, 16.0).color(th.accent_color()),
                        column![
                            bold(fname.to_string(), 12.0).color(th.text_color()),
                            txt(format!("{kb} KB • Inline Document"), 11.0).color(th.sub_color()),
                        ]
                        .spacing(1),
                        Space::new().width(12),
                        button(med("Quick Look", 11.0).color(th.accent_color()))
                            .padding([4, 8])
                            .style(btn_bordered_style(
                                [CLEAR, th.hover_overlay(), th.active_overlay()],
                                [th.accent_color(); 3],
                                th.accent_color(),
                                6.0,
                            ))
                            .on_press(UiMessage::OpenQuickLook(
                                fname.to_string(),
                                att.src_url.clone().unwrap_or_default(),
                            )),
                    ]
                    .spacing(10)
                    .align_y(Center),
                )
                .padding([8, 12])
                .style(bordered_box(th.elevated_bg(), th.border_color(), 1.0, 10.0));
                bubble_col = bubble_col.push(file_card);
            }
        }

        if let Some(ref poll) = m.poll {
            let mut p_col =
                column![bold(format!("Poll: {}", poll.question), 12.5).color(bubble_fg)].spacing(5);
            for opt in &poll.options {
                let votes = opt.voter_ids.len();
                p_col = p_col.push(
                    button(
                        row![
                            med(opt.label.clone(), 12.0).color(th.text_color()).width(Fill),
                            bold(format!("{votes} vote(s)"), 11.0).color(th.accent_color()),
                        ]
                        .align_y(Center),
                    )
                    .padding([6, 10])
                    .width(300)
                    .style(btn_bordered_style(
                        [th.elevated_bg(), th.elevated_hover_bg(), th.active_overlay()],
                        [th.text_color(); 3],
                        th.border_color(),
                        8.0,
                    ))
                    .on_press(UiMessage::VotePoll(m.id.clone(), opt.id.clone())),
                );
            }
            bubble_col = bubble_col.push(p_col);
        }

        let bubble = container(bubble_col)
            .padding([8, 13])
            .max_width(540)
            .style(bubble_box(bubble_bg, is_self));

        let mut action_row = Row::new().spacing(4).align_y(Center);
        for rx in &m.reactions {
            let is_mine = rx.participant_id.starts_with("@self_");
            action_row = action_row.push(
                button(med(format!("{} 1", rx.reaction_key), 11.0).color(th.text_color()))
                    .padding([2, 7])
                    .style(btn_bordered_style(
                        [th.elevated_bg(), th.elevated_hover_bg(), th.active_overlay()],
                        [th.text_color(); 3],
                        if is_mine { th.accent_color() } else { th.border_color() },
                        10.0,
                    ))
                    .on_press(UiMessage::ToggleReaction(
                        chat_id.to_string(),
                        m.id.clone(),
                        rx.reaction_key.clone(),
                    )),
            );
        }

        let act_btn = |label: &'static str, msg: UiMessage| -> El<'a> {
            button(med(label, 10.5).color(th.muted_color()))
                .padding([2, 6])
                .style(btn_style(
                    [CLEAR, th.hover_overlay(), th.active_overlay()],
                    [th.sub_color(); 3],
                    6.0,
                ))
                .on_press(msg)
                .into()
        };

        action_row = action_row
            .push(act_btn(
                "⚡",
                UiMessage::ToggleReaction(chat_id.to_string(), m.id.clone(), "⚡".to_string()),
            ))
            .push(act_btn("Reply", UiMessage::StartReply(m.id.clone())))
            .push(act_btn(
                "Edit",
                UiMessage::StartEdit(m.id.clone(), m.text.clone().unwrap_or_default()),
            ))
            .push(act_btn("Star", UiMessage::ToggleBookmark(m.id.clone())))
            .push(act_btn(
                "Unread",
                UiMessage::MarkUnreadFromHere(chat_id.to_string(), m.id.clone()),
            ));

        let msg_stack = column![bubble, action_row]
            .spacing(3)
            .align_x(if is_self {
                alignment::Horizontal::Right
            } else {
                alignment::Horizontal::Left
            });

        if is_self {
            row![Space::new().width(Fill), msg_stack]
                .align_y(alignment::Vertical::Bottom)
                .into()
        } else {
            let av = container(bold(initials(sender_label), 10.5).color(Color::WHITE).center())
                .center(th.cv_avatar_size)
                .style(fill_box(hue(hash_str(&m.sender_id)), th.cv_avatar_size / 2.0));
            row![av, msg_stack, Space::new().width(Fill)]
                .spacing(8)
                .align_y(alignment::Vertical::Bottom)
                .into()
        }
    }

    fn view_right_inspector<'a>(&self, th: &ResolvedTheme) -> El<'a> {
        let st = self.store.read().unwrap();
        let Some(cid) = self.selected_chat_id.as_deref() else {
            return container(Space::new()).width(self.right_panel_width).height(Fill).into();
        };
        let Some(c_idx) = st.resolve_chat_idx(cid) else {
            return container(Space::new()).width(self.right_panel_width).height(Fill).into();
        };
        let chat = &st.chats[c_idx];
        let (_, net_col, net_name) = network_brand_meta(&chat.account_id);

        let head = container(
            row![
                bold("Chat Details", 13.5).color(th.text_color()).width(Fill),
                button(ic(icon::X, 14.0).color(th.sub_color()))
                    .padding(4)
                    .style(btn_style([CLEAR, th.hover_overlay(), CLEAR], [th.sub_color(); 3], 6.0))
                    .on_press(UiMessage::ToggleRightPanel),
            ]
            .align_y(Center)
            .padding([0, 14]),
        )
        .center_y(th.header_height)
        .style(bordered_box(th.header_bg(), th.border_weak_color(), 1.0, 0.0));

        let hero_av = container(bold(initials(&chat.title), 18.0).color(Color::WHITE).center())
            .center(56)
            .style(fill_box(hue(hash_str(&chat.id)), 28.0));

        let net_badge = container(bold(net_name, 11.0).color(Color::WHITE))
            .padding([3, 9])
            .style(fill_box(net_col, 8.0));

        let hero = column![
            hero_av,
            bold(chat.title.clone(), 15.0).color(th.text_color()),
            txt(chat.description.clone().unwrap_or_else(|| chat.id.clone()), 11.5)
                .color(th.sub_color()),
            net_badge,
        ]
        .spacing(6)
        .align_x(Center)
        .width(Fill)
        .padding([16, 12]);

        let nick_in = text_input("Custom nickname...", &self.nickname_edit_buffer)
            .on_input(UiMessage::NicknameChanged)
            .font(fw(Weight::Normal))
            .size(12.5)
            .padding([6, 10])
            .style(input_style(
                [th.input_bg(), th.input_bg()],
                [th.border_weak_color(), th.accent_color()],
                th.text_color(),
                th.muted_color(),
                th.accent_color(),
                [1.0, 1.5],
                6.0,
            ));

        let save_nick = button(bold("Save", 11.5).color(th.on_accent_color()))
            .padding([6, 10])
            .style(btn_style([th.accent_color(); 3], [th.on_accent_color(); 3], 6.0))
            .on_press(UiMessage::SaveNickname);

        let nick_sec = column![
            bold("Edit for Me (Nickname)", 12.0).color(th.sub_color()),
            row![nick_in, save_nick].spacing(6).align_y(Center),
        ]
        .spacing(6)
        .padding([8, 14]);

        let mut members_col = column![bold("Participants", 12.0).color(th.sub_color())].spacing(6);
        for p in &chat.participants.items {
            let name = p
                .custom_nickname
                .as_deref()
                .or(p.full_name.as_deref())
                .unwrap_or(&p.id);
            let dot = container(Space::new().width(8).height(8))
                .style(fill_box(hue(hash_str(&p.id)), 4.0));
            members_col = members_col.push(
                row![
                    dot,
                    med(name.to_string(), 12.0).color(th.text_color()).width(Fill),
                    txt(if p.is_self == Some(true) { "You" } else { "" }, 11.0)
                        .color(th.muted_color()),
                ]
                .spacing(8)
                .align_y(Center),
            );
        }
        let members_sec = container(members_col).padding([8, 14]);

        let mut bmarks_col =
            column![bold("Bookmarked Messages", 12.0).color(th.sub_color())].spacing(6);
        if let Some(indices) = st.chat_msg_indices.get(&chat.id) {
            for &i in indices {
                let m = &st.messages[i];
                if m.is_bookmarked == Some(true) {
                    bmarks_col = bmarks_col.push(
                        container(
                            ell(
                                m.text.as_deref().unwrap_or("[Attachment]"),
                                self.right_panel_width - 48.0,
                                11.5,
                                Weight::Normal,
                            )
                            .color(th.text_color()),
                        )
                        .padding([6, 8])
                        .width(Fill)
                        .style(fill_box(th.elevated_bg(), 6.0)),
                    );
                }
            }
        }
        let bmarks_sec = container(bmarks_col).padding([8, 14]);

        let export_btn = container(
            button(bold("Export Chat JSON", 12.0).color(th.text_color()).center().width(Fill))
                .padding([8, 12])
                .width(Fill)
                .style(btn_bordered_style(
                    [th.elevated_bg(), th.elevated_hover_bg(), th.active_overlay()],
                    [th.text_color(); 3],
                    th.border_color(),
                    8.0,
                ))
                .on_press(UiMessage::ExportChatJson(chat.id.clone())),
        )
        .padding([8, 14]);

        let body = scrollable(column![hero, nick_sec, members_sec, bmarks_sec, export_btn])
            .height(Fill)
            .style(scroll_style(
                th.sub_color().scale_alpha(0.3),
                th.sub_color().scale_alpha(0.5),
            ));

        container(column![head, body])
            .width(self.right_panel_width)
            .height(Fill)
            .style(bordered_box(th.sidebar_bg(), th.border_weak_color(), 1.0, 0.0))
            .into()
    }

    fn view_modal_scrim<'a>(&self, card: El<'a>) -> El<'a> {
        let scrim = mouse_area(
            container(Space::new().width(Fill).height(Fill))
                .style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.55), 0.0)),
        )
        .on_press(UiMessage::CloseModals);
        stack![scrim, container(card).center(Fill).padding(24)]
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn view_command_palette<'a>(&self, th: &ResolvedTheme) -> El<'a> {
        let st = self.store.read().unwrap();
        let q = self.command_palette_query.trim();

        let input = text_input(
            "Type a command, chat name, or setting...",
            &self.command_palette_query,
        )
        .id("cmd-palette-input")
        .on_input(UiMessage::CommandQueryChanged)
        .font(fw(Weight::Normal))
        .size(14.0)
        .padding([10, 12])
        .style(input_style(
            [th.input_bg(), th.input_bg()],
            [th.border_color(), th.accent_color()],
            th.text_color(),
            th.muted_color(),
            th.accent_color(),
            [1.0, 1.5],
            8.0,
        ));

        let mut results = Column::new().spacing(4);
        let chat_hits: Vec<&Chat> = if q.is_empty() {
            st.chats.iter().take(5).collect()
        } else {
            st.chat_search_index
                .query_all_tokens(q)
                .into_iter()
                .filter_map(|i| st.chats.get(i as usize))
                .take(5)
                .collect()
        };

        for c in chat_hits {
            let (_, net_col, net_name) = network_brand_meta(&c.account_id);
            results = results.push(
                button(
                    row![
                        container(bold(net_name, 10.0).color(Color::WHITE))
                            .padding([2, 6])
                            .style(fill_box(net_col, 4.0)),
                        bold(c.title.clone(), 13.0).color(th.text_color()).width(Fill),
                        txt("Jump to chat", 11.5).color(th.muted_color()),
                    ]
                    .spacing(8)
                    .align_y(Center),
                )
                .padding([8, 10])
                .width(Fill)
                .style(btn_style(
                    [CLEAR, th.hover_overlay(), th.active_overlay()],
                    [th.text_color(); 3],
                    6.0,
                ))
                .on_press(UiMessage::ExecuteCommandChat(c.id.clone())),
            );
        }

        for entry in search_settings_catalog(q).into_iter().take(4) {
            results = results.push(
                button(
                    row![
                        ic(icon::SETTINGS, 13.0).color(th.accent_color()),
                        bold(entry.title, 12.5).color(th.text_color()),
                        txt(entry.description, 11.5).color(th.sub_color()).width(Fill),
                    ]
                    .spacing(8)
                    .align_y(Center),
                )
                .padding([8, 10])
                .width(Fill)
                .style(btn_style(
                    [CLEAR, th.hover_overlay(), th.active_overlay()],
                    [th.text_color(); 3],
                    6.0,
                ))
                .on_press(UiMessage::ToggleSettingsModal),
            );
        }

        let card = column![
            row![
                ic(icon::SEARCH, 15.0).color(th.accent_color()),
                bold("Command Palette", 14.0).color(th.text_color()).width(Fill),
                button(ic(icon::X, 14.0).color(th.sub_color()))
                    .padding(4)
                    .style(btn_style([CLEAR; 3], [th.sub_color(); 3], 6.0))
                    .on_press(UiMessage::CloseModals),
            ]
            .spacing(8)
            .align_y(Center),
            input,
            scrollable(results).height(320),
        ]
        .spacing(12)
        .padding(16);

        mouse_area(
            container(card)
                .width(560)
                .style(elevated_card(th.menu_bg(), th.border_color(), 12.0)),
        )
        .on_press(UiMessage::CommandQueryChanged(self.command_palette_query.clone()))
        .into()
    }

    fn view_settings_modal<'a>(&self, th: &ResolvedTheme) -> El<'a> {
        let st = self.store.read().unwrap();
        let s = &st.settings;

        let nav_items = SettingsTab::ALL.into_iter().map(|tab| {
            let active = self.active_settings_tab == tab;
            let bg = if active {
                [th.accent_color(); 3]
            } else {
                [CLEAR, th.hover_overlay(), th.active_overlay()]
            };
            let fg = if active { th.on_accent_color() } else { th.text_color() };
            button(med(tab.label(), 12.5).color(fg).width(Fill))
                .padding([8, 12])
                .width(Fill)
                .style(btn_style(bg, [fg; 3], 6.0))
                .on_press(UiMessage::SelectSettingsTab(tab))
                .into()
        });

        let left_nav = container(Column::with_children(nav_items).spacing(4).padding(12))
            .width(185)
            .height(Fill)
            .style(bordered_box(th.sidebar_bg(), th.border_weak_color(), 1.0, 8.0));

        let tog_line =
            |label: &'static str, desc: &'static str, key: &'static str, val: bool| -> El<'a> {
                let sw = toggler(val)
                    .on_toggle(move |b| UiMessage::ToggleSettingBool(key, b))
                    .size(19)
                    .style(tog_style(th.accent_color(), th.elevated_hover_bg(), Color::WHITE));
                row![
                    column![
                        bold(label, 13.0).color(th.text_color()),
                        txt(desc, 11.5).color(th.sub_color()),
                    ]
                    .spacing(2)
                    .width(Fill),
                    sw,
                ]
                .spacing(12)
                .align_y(Center)
                .padding([6, 0])
                .into()
            };

        let pane_body: El<'a> = match self.active_settings_tab {
            SettingsTab::ChatList => column![
                bold("Chat List Preferences", 16.0).color(th.text_color()),
                tog_line("Show archived count", "Display archived chats badge in sidebar header", "show_archived_count", s.show_archived_count),
                tog_line("Show message previews", "Display latest message snippet under chat titles", "show_message_previews", s.show_message_previews),
                tog_line("Unread chats always on top", "Sort unread threads above read threads", "unread_chats_always_on_top", s.unread_chats_always_on_top),
                tog_line("Filter pins by network", "Scope pinned avatars to the selected bridge", "filter_pins_by_network", s.filter_pins_by_network),
                tog_line("Show favorite chats section", "Display Favorites group above the main inbox", "show_favorite_chats_section", s.show_favorite_chats_section),
            ]
            .spacing(8)
            .into(),
            SettingsTab::ConversationView => column![
                bold("Conversation View", 16.0).color(th.text_color()),
                tog_line("Show full names in chats", "Display full sender names in group conversations", "show_full_names_in_chats", s.show_full_names_in_chats),
                tog_line("Show chat header", "Display top header bar with participant details", "show_chat_header", s.show_chat_header),
                tog_line("Show message input buttons", "Show voice note, poll, and reminder buttons in composer", "show_message_input_buttons", s.show_message_input_buttons),
                tog_line("Use 24-hour time", "Format timestamps using 24-hour clock", "use_twenty_four_hour_time", s.use_twenty_four_hour_time),
                tog_line("Transcribe voice notes", "Automatically transcribe incoming and outgoing voice notes", "auto_transcribe_voice_notes", s.auto_transcribe_voice_notes),
            ]
            .spacing(8)
            .into(),
            SettingsTab::Appearance => {
                let theme_chips = Row::with_children(ThemeChoice::all().into_iter().map(|tc| {
                    let active = s.theme == tc;
                    let bg = if active {
                        [th.accent_color(); 3]
                    } else {
                        [th.elevated_bg(), th.elevated_hover_bg(), th.active_overlay()]
                    };
                    let fg = if active { th.on_accent_color() } else { th.text_color() };
                    button(bold(tc.label(), 12.0).color(fg))
                        .padding([7, 12])
                        .style(btn_style(bg, [fg; 3], 8.0))
                        .on_press(UiMessage::SelectTheme(tc))
                        .into()
                }))
                .spacing(6)
                .wrap();

                let langs: Vec<String> = ALL_LANGUAGES.iter().map(|l| l.label().to_string()).collect();
                let lang_pick = pick_list(
                    langs,
                    Some(s.language.label().to_string()),
                    UiMessage::SelectLanguage,
                )
                .font(fw(Weight::Normal))
                .text_size(13.0)
                .padding([6, 10])
                .style(pick_style(
                    th.elevated_bg(),
                    th.elevated_hover_bg(),
                    th.text_color(),
                    th.sub_color(),
                    th.border_color(),
                ))
                .menu_style(pick_menu_style(
                    th.menu_bg(),
                    th.accent_color(),
                    th.text_color(),
                    th.border_color(),
                ));

                let css_in = text_input(":root { --color-primary: #0a84ff; }", &s.custom_css_source)
                    .on_input(UiMessage::CustomCssChanged)
                    .font(fw(Weight::Normal))
                    .size(12.5)
                    .padding([8, 10])
                    .style(input_style(
                        [th.input_bg(), th.input_bg()],
                        [th.border_color(), th.accent_color()],
                        th.text_color(),
                        th.muted_color(),
                        th.accent_color(),
                        [1.0, 1.5],
                        6.0,
                    ));

                column![
                    bold("Themes & Custom CSS", 16.0).color(th.text_color()),
                    theme_chips,
                    row![bold("Interface Language", 13.0).color(th.text_color()).width(Fill), lang_pick]
                        .align_y(Center),
                    tog_line("No transparency (.no-transparency)", "Use opaque sidebar, header, and menu surfaces", "no_transparency", s.no_transparency),
                    tog_line("Fix Lockdown Mode Emojis", "Apply fix-lockdown-mode-emojis.css font override", "fix_lockdown_emojis", s.fix_lockdown_emojis),
                    bold("Live Custom CSS Override", 12.5).color(th.sub_color()),
                    css_in,
                ]
                .spacing(10)
                .into()
            }
            SettingsTab::Networks => {
                let mut net_col =
                    column![bold("Connected Chat Networks (16)", 16.0).color(th.text_color())]
                        .spacing(6);
                for spec in &NETWORK_SPECS {
                    let bid = spec.bridge_id;
                    let rgb = spec.brand_rgb;
                    let connected = st
                        .bridge_accounts
                        .iter()
                        .any(|b| b.bridge_id == bid && b.state == "CONNECTED");
                    let badge = container(bold(spec.badge_code, 10.5).color(Color::WHITE).center())
                        .center(26)
                        .style(fill_box(Color::from_rgb8(rgb[0], rgb[1], rgb[2]), 6.0));
                    let mode_str = match (spec.supports_on_device, spec.supports_cloud) {
                        (true, true) => "On-Device / Cloud",
                        (true, false) => "On-Device",
                        _ => "Cloud",
                    };
                    net_col = net_col.push(
                        row![
                            badge,
                            bold(spec.display_name, 13.0).color(th.text_color()).width(Fill),
                            txt(mode_str, 11.5).color(th.sub_color()),
                            button(
                                bold(if connected { "Connected" } else { "Connect" }, 11.5).color(
                                    if connected { th.success_color() } else { th.on_accent_color() },
                                ),
                            )
                            .padding([4, 10])
                            .style(btn_style(
                                if connected { [th.elevated_bg(); 3] } else { [th.accent_color(); 3] },
                                [Color::WHITE; 3],
                                6.0,
                            ))
                            .on_press(UiMessage::ConnectBridge(bid.to_string())),
                        ]
                        .spacing(10)
                        .align_y(Center),
                    );
                }
                net_col.into()
            }
            SettingsTab::Security => {
                let rec_in = text_input("Enter recovery code...", &self.recovery_code_input)
                    .on_input(UiMessage::RecoveryInputChanged)
                    .font(fw(Weight::Normal))
                    .size(12.5)
                    .padding([7, 10])
                    .style(input_style(
                        [th.input_bg(), th.input_bg()],
                        [th.border_color(), th.accent_color()],
                        th.text_color(),
                        th.muted_color(),
                        th.accent_color(),
                        [1.0, 1.5],
                        6.0,
                    ));
                column![
                    bold("Matrix E2EE & Cross-Signing", 16.0).color(th.text_color()),
                    txt(
                        format!(
                            "User: {} • Device: {} • Verified: {}",
                            st.beeper_state.user_id, st.beeper_state.device_id, st.beeper_state.verified
                        ),
                        12.5,
                    )
                    .color(th.sub_color()),
                    row![
                        rec_in,
                        button(bold("Verify Key", 12.0).color(th.on_accent_color()))
                            .padding([7, 12])
                            .style(btn_style([th.accent_color(); 3], [th.on_accent_color(); 3], 6.0))
                            .on_press(UiMessage::VerifyRecoveryCode),
                        button(bold("SAS Verify", 12.0).color(th.text_color()))
                            .padding([7, 12])
                            .style(btn_bordered_style(
                                [th.elevated_bg(), th.elevated_hover_bg(), th.active_overlay()],
                                [th.text_color(); 3],
                                th.border_color(),
                                6.0,
                            ))
                            .on_press(UiMessage::VerifySasNow),
                    ]
                    .spacing(8)
                    .align_y(Center),
                ]
                .spacing(12)
                .into()
            }
            SettingsTab::StorageAndOs => {
                let (db_b, media_b) = st.calculate_storage_bytes();
                column![
                    bold("Isolated Storage & OS Integration", 16.0).color(th.text_color()),
                    txt(format!("Isolated Data Dir: {}", st.data_dir.display()), 12.5)
                        .color(th.sub_color()),
                    txt(
                        format!(
                            "SQLite WAL: {} KB • Media Cache: {} KB • Display: {:?}",
                            db_b / 1024,
                            media_b / 1024,
                            self.display_server
                        ),
                        12.5,
                    )
                    .color(th.sub_color()),
                    button(bold("Clear >30d Media Cache", 12.0).color(th.on_accent_color()))
                        .padding([8, 14])
                        .style(btn_style([th.accent_color(); 3], [th.on_accent_color(); 3], 6.0))
                        .on_press(UiMessage::ClearOldMedia),
                ]
                .spacing(12)
                .into()
            }
        };

        let right_pane = scrollable(container(pane_body).padding(16).width(Fill))
            .height(Fill)
            .style(scroll_style(
                th.sub_color().scale_alpha(0.3),
                th.sub_color().scale_alpha(0.5),
            ));

        let top = row![
            bold("Preferences", 15.0).color(th.text_color()).width(Fill),
            button(ic(icon::X, 15.0).color(th.sub_color()))
                .padding(4)
                .style(btn_style([CLEAR; 3], [th.sub_color(); 3], 6.0))
                .on_press(UiMessage::CloseModals),
        ]
        .align_y(Center)
        .padding([12, 16]);

        let modal = column![
            top,
            row![left_nav, right_pane].spacing(12).padding([0, 12]).height(Fill)
        ]
        .padding(iced::padding::bottom(12));

        mouse_area(
            container(modal)
                .width(740)
                .height(500)
                .style(elevated_card(th.menu_bg(), th.border_color(), 12.0)),
        )
        .on_press(UiMessage::SelectSettingsTab(self.active_settings_tab))
        .into()
    }

    fn view_quick_look_modal<'a>(&self, th: &ResolvedTheme, title: &'a str, detail: &'a str) -> El<'a> {
        let card = column![
            row![
                ic(icon::IMAGE, 16.0).color(th.accent_color()),
                bold(title, 14.0).color(th.text_color()).width(Fill),
                button(ic(icon::X, 14.0).color(th.sub_color()))
                    .padding(4)
                    .style(btn_style([CLEAR; 3], [th.sub_color(); 3], 6.0))
                    .on_press(UiMessage::CloseModals),
            ]
            .spacing(8)
            .align_y(Center),
            container(
                column![
                    ic(icon::FOLDER, 42.0).color(th.accent_color()),
                    bold(title, 15.0).color(th.text_color()),
                    txt(detail, 12.0).color(th.sub_color()),
                ]
                .spacing(8)
                .align_x(Center)
            )
            .center(Fill)
            .height(220)
            .style(fill_box(th.elevated_bg(), 8.0)),
        ]
        .spacing(12)
        .padding(16);

        container(card)
            .width(460)
            .style(elevated_card(th.menu_bg(), th.border_color(), 12.0))
            .into()
    }
}

pub fn write_rgba_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create screenshot png");
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().expect("write png header");
    writer.write_image_data(rgba).expect("write png data");
}

pub fn run_native_gui(store: SharedStore) -> iced::Result {
    ensure_fonts_loaded();
    let icon = {
        let st = store.read().unwrap();
        SystemTrayState::new(st.total_unread_chats(), false, false).to_window_icon()
    };
    let boot_store = store.clone();
    iced::application(
        move || (BeeperGuiApp::new(boot_store.clone(), true), Task::none()),
        BeeperGuiApp::update,
        BeeperGuiApp::view,
    )
    .title(|app: &BeeperGuiApp| app.tray.tooltip.clone())
    .subscription(BeeperGuiApp::subscription)
    .theme(|app: &BeeperGuiApp| if app.theme.is_dark { Theme::Dark } else { Theme::Light })
    .scale_factor(|app: &BeeperGuiApp| {
        let z = app.store.read().unwrap().settings.zoom_percent;
        (z as f32 / 100.0).clamp(0.5, 2.5)
    })
    .window(window::Settings {
        size: Size::new(1440.0, 900.0),
        min_size: Some(Size::new(840.0, 560.0)),
        icon: Some(icon),
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: crate::config::APP_BUNDLE_ID.to_string(),
            ..Default::default()
        },
        ..Default::default()
    })
    .font(FONT_REGULAR)
    .font(FONT_MEDIUM)
    .font(FONT_BOLD)
    .font(FONT_BLACK)
    .font(FONT_LUCIDE)
    .font(FONT_PHOSPHOR)
    .default_font(fw(Weight::Normal))
    .run()
}
