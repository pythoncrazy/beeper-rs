use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const CHAT_PINNED: u16 = 1 << 0;
pub const CHAT_ARCHIVED: u16 = 1 << 1;
pub const CHAT_MUTED: u16 = 1 << 2;
pub const CHAT_UNREAD: u16 = 1 << 3;
pub const CHAT_LOW_PRIORITY: u16 = 1 << 4;
pub const CHAT_MARKED_UNREAD: u16 = 1 << 5;
pub const CHAT_NOTIFY_ANYWAY: u16 = 1 << 6;
pub const CHAT_MERGED_PARENT: u16 = 1 << 7;
pub const CHAT_HIDDEN: u16 = 1 << 8;
pub const CHAT_REQUEST: u16 = 1 << 9;

pub const MSG_SENDER_SELF: u16 = 1 << 0;
pub const MSG_UNREAD: u16 = 1 << 1;
pub const MSG_BOOKMARKED: u16 = 1 << 2;
pub const MSG_VOICE_NOTE: u16 = 1 << 3;
pub const MSG_EDITED: u16 = 1 << 5;
pub const MSG_DELETED: u16 = 1 << 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkKind {
    Beeper,
    WhatsApp,
    Messenger,
    Instagram,
    Telegram,
    Signal,
    Viber,
    IMessage,
    GoogleMessages,
    GoogleVoice,
    GoogleChat,
    TwitterX,
    Slack,
    Discord,
    LinkedIn,
    Line,
    Tumblr,
    Snapchat,
    WeChat,
    Bluesky,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeMode {
    OnDevice,
    Cloud,
}

#[derive(Debug, Clone)]
pub struct NetworkSpec {
    pub kind: NetworkKind,
    pub bridge_id: &'static str,
    pub bridge_type: &'static str,
    pub display_name: &'static str,
    pub brand_rgb: [u8; 3],
    pub supports_on_device: bool,
    pub supports_cloud: bool,
    pub supports_sms_mms: bool,
    pub supports_e2ee: bool,
    pub supports_polls: bool,
    pub supports_edit_history: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeeperStateSecrets {
    pub master_key: String,
    pub self_signing_key: String,
    pub user_signing_key: String,
    pub megolm_backup_key: String,
    pub recovery_code: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeperStateRecord {
    pub access_token: String,
    pub device_id: String,
    pub user_id: String,
    pub homeserver: String,
    pub first_sync_done: bool,
    pub initialized: bool,
    pub secret_storage: bool,
    pub cross_signing: bool,
    pub verified: bool,
    pub key_backup: bool,
    pub has_backed_up_code: bool,
    pub secrets: BeeperStateSecrets,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatrixSession {
    pub user_id: String,
    pub access_token: String,
    pub device_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub full_name: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoResponse {
    pub app: InfoApp,
    pub endpoints: InfoEndpoints,
    pub platform: InfoPlatform,
    pub server: InfoServer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoApp {
    pub bundle_id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoEndpoints {
    pub mcp: String,
    pub oauth: InfoOAuthEndpoints,
    pub spec: String,
    pub ws_events: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoOAuthEndpoints {
    pub authorization_endpoint: String,
    pub introspection_endpoint: String,
    pub registration_endpoint: String,
    pub revocation_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoPlatform {
    pub arch: String,
    pub os: String,
    pub release: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoServer {
    pub base_url: String,
    pub hostname: String,
    pub mcp_enabled: bool,
    pub port: u16,
    pub remote_access: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(rename = "fullName", skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(rename = "phoneNumber", skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    #[serde(rename = "imgURL", skip_serializing_if = "Option::is_none")]
    pub img_url: Option<String>,
    #[serde(rename = "isSelf", skip_serializing_if = "Option::is_none")]
    pub is_self: Option<bool>,
    #[serde(rename = "cannotMessage", skip_serializing_if = "Option::is_none")]
    pub cannot_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_nickname: Option<String>,
    #[serde(rename = "isNetworkBot", skip_serializing_if = "Option::is_none")]
    pub is_network_bot: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    #[serde(rename = "accountID")]
    pub account_id: String,
    pub user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatParticipants {
    #[serde(rename = "hasMore")]
    pub has_more: bool,
    pub items: Vec<User>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatDraftAttachment {
    #[serde(rename = "uploadID")]
    pub upload_id: String,
    #[serde(rename = "fileName", skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatDraft {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attachments: Vec<ChatDraftAttachment>,
    #[serde(rename = "replyToMessageID", skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMergeInfo {
    #[serde(rename = "chatIDs")]
    pub chat_ids: Vec<String>,
    #[serde(rename = "defaultChatID", skip_serializing_if = "Option::is_none")]
    pub default_chat_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatReminder {
    #[serde(rename = "remindAtMs")]
    pub remind_at_ms: i64,
    #[serde(rename = "dismissOnIncomingMessage", default)]
    pub dismiss_on_incoming_message: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatThemeCustom {
    pub bubble_self_rgb: [u8; 3],
    pub bubble_other_rgb: [u8; 3],
    pub wallpaper_tint_rgb: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    #[serde(rename = "accountID")]
    pub account_id: String,
    pub participants: ChatParticipants,
    pub title: String,
    #[serde(rename = "type")]
    pub chat_type: String,
    #[serde(rename = "unreadCount")]
    pub unread_count: u32,
    #[serde(rename = "unreadMentionsCount", default)]
    pub unread_mentions: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<ChatDraft>,
    #[serde(rename = "isArchived", skip_serializing_if = "Option::is_none")]
    pub is_archived: Option<bool>,
    #[serde(rename = "isMarkedUnread", skip_serializing_if = "Option::is_none")]
    pub is_marked_unread: Option<bool>,
    #[serde(rename = "isMuted", skip_serializing_if = "Option::is_none")]
    pub is_muted: Option<bool>,
    #[serde(rename = "isPinned", skip_serializing_if = "Option::is_none")]
    pub is_pinned: Option<bool>,
    #[serde(rename = "isLowPriority", skip_serializing_if = "Option::is_none")]
    pub is_low_priority: Option<bool>,
    #[serde(rename = "labelIDs", skip_serializing_if = "Vec::is_empty", default)]
    pub label_ids: Vec<String>,
    #[serde(rename = "lastActivity", skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    #[serde(rename = "lastReadMessageSortKey", skip_serializing_if = "Option::is_none")]
    pub last_read_message_sort_key: Option<String>,
    #[serde(rename = "localChatID", skip_serializing_if = "Option::is_none")]
    pub local_chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<ChatMergeInfo>,
    #[serde(rename = "mergedIntoChatID", skip_serializing_if = "Option::is_none")]
    pub merged_into_chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<ChatReminder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_theme: Option<ChatThemeCustom>,
    #[serde(rename = "isHiddenFromInbox", skip_serializing_if = "Option::is_none")]
    pub is_hidden_from_inbox: Option<bool>,
    #[serde(rename = "isRequest", skip_serializing_if = "Option::is_none")]
    pub is_request: Option<bool>,
    #[serde(rename = "networkName", skip_serializing_if = "Option::is_none")]
    pub network_name: Option<String>,
    #[serde(rename = "networkID", skip_serializing_if = "Option::is_none")]
    pub network_id: Option<String>,
    #[serde(rename = "imgURL", skip_serializing_if = "Option::is_none")]
    pub img_url: Option<String>,
    #[serde(skip)]
    pub flags: u16,
}

fn flag_bits(bits: &[(bool, u16)]) -> u16 {
    bits.iter().filter(|(on, _)| *on).fold(0, |f, (_, bit)| f | bit)
}

impl Chat {
    pub fn sync_flags(&mut self) {
        let marked = self.is_marked_unread == Some(true);
        self.flags = flag_bits(&[
            (self.is_pinned == Some(true), CHAT_PINNED),
            (self.is_archived == Some(true), CHAT_ARCHIVED),
            (self.is_muted == Some(true), CHAT_MUTED),
            (self.unread_count > 0 || marked, CHAT_UNREAD),
            (self.is_low_priority == Some(true), CHAT_LOW_PRIORITY),
            (marked, CHAT_MARKED_UNREAD),
            (self.merge.is_some(), CHAT_MERGED_PARENT),
            (self.is_hidden_from_inbox == Some(true), CHAT_HIDDEN),
            (self.is_request == Some(true), CHAT_REQUEST),
        ]) | (self.flags & CHAT_NOTIFY_ANYWAY);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentSize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(rename = "type")]
    pub attachment_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(rename = "fileName", skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(rename = "fileSize", skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    #[serde(rename = "isGif", skip_serializing_if = "Option::is_none")]
    pub is_gif: Option<bool>,
    #[serde(rename = "isSticker", skip_serializing_if = "Option::is_none")]
    pub is_sticker: Option<bool>,
    #[serde(rename = "isVoiceNote", skip_serializing_if = "Option::is_none")]
    pub is_voice_note: Option<bool>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(rename = "posterImg", skip_serializing_if = "Option::is_none")]
    pub poster_img: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<AttachmentSize>,
    #[serde(rename = "srcURL", skip_serializing_if = "Option::is_none")]
    pub src_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub id: String,
    #[serde(rename = "participantID")]
    pub participant_id: String,
    #[serde(rename = "reactionKey")]
    pub reaction_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
    #[serde(rename = "imgURL", skip_serializing_if = "Option::is_none")]
    pub img_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollOption {
    pub id: String,
    pub label: String,
    pub voter_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    pub site: String,
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollData {
    pub question: String,
    pub options: Vec<PollOption>,
    pub allow_multiple: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditHistoryEntry {
    pub text: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    #[serde(rename = "accountID")]
    pub account_id: String,
    #[serde(rename = "chatID")]
    pub chat_id: String,
    #[serde(rename = "senderID")]
    pub sender_id: String,
    #[serde(rename = "sortKey")]
    pub sort_key: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attachments: Vec<Attachment>,
    #[serde(rename = "isSender", skip_serializing_if = "Option::is_none")]
    pub is_sender: Option<bool>,
    #[serde(rename = "isUnread", skip_serializing_if = "Option::is_none")]
    pub is_unread: Option<bool>,
    #[serde(rename = "isBookmarked", skip_serializing_if = "Option::is_none")]
    pub is_bookmarked: Option<bool>,
    #[serde(rename = "isDeleted", skip_serializing_if = "Option::is_none")]
    pub is_deleted: Option<bool>,
    #[serde(rename = "linkedMessageID", skip_serializing_if = "Option::is_none")]
    pub linked_message_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub reactions: Vec<Reaction>,
    #[serde(rename = "senderName", skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The Matrix HTML of the text, when the sender formatted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub message_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll: Option<PollData>,
    #[serde(rename = "unresolvedMedia", skip_serializing_if = "Option::is_none")]
    pub unresolved_media: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub edit_history: Vec<EditHistoryEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub links: Vec<MessageLink>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub seen: std::collections::BTreeMap<String, String>,
    #[serde(skip)]
    pub flags: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageLink {
    pub url: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl Message {
    pub fn sync_flags(&mut self) {
        self.flags = flag_bits(&[
            (self.is_sender == Some(true), MSG_SENDER_SELF),
            (self.is_unread == Some(true), MSG_UNREAD),
            (self.is_bookmarked == Some(true), MSG_BOOKMARKED),
            (self.attachments.iter().any(|a| a.is_voice_note == Some(true)), MSG_VOICE_NOTE),
            (!self.edit_history.is_empty(), MSG_EDITED),
            (self.is_deleted == Some(true), MSG_DELETED),
        ]);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorPage<T> {
    pub items: Vec<T>,
    pub has_more: bool,
    pub oldest_cursor: Option<String>,
    pub newest_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSasEmoji {
    pub emoji: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSas {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decimals: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub emojis: Vec<VerificationSasEmoji>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationQr {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationOtherDevice {
    #[serde(rename = "deviceID")]
    pub device_id: String,
    #[serde(rename = "userID")]
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationState {
    #[serde(rename = "verificationID")]
    pub verification_id: String,
    pub direction: String,
    pub methods: Vec<String>,
    pub state: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sas: Option<VerificationSas>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr: Option<VerificationQr>,
    #[serde(rename = "otherDevice", skip_serializing_if = "Option::is_none")]
    pub other_device: Option<VerificationOtherDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupE2ee {
    pub state: String,
    pub cross_signing: bool,
    pub first_sync_done: bool,
    pub has_backed_up_code: bool,
    pub initialized: bool,
    pub key_backup: bool,
    pub secret_storage: bool,
    pub verified: bool,
    pub verification_methods: Vec<String>,
    pub secrets: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupMatrix {
    pub homeserver: String,
    #[serde(rename = "userID")]
    pub user_id: String,
    #[serde(rename = "deviceID", skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupSession {
    pub state: String,
    pub e2ee: SetupE2ee,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<SetupMatrix>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStatusResponse {
    pub session: SetupSession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeAccountInfo {
    #[serde(rename = "accountID")]
    pub account_id: String,
    #[serde(rename = "bridgeID")]
    pub bridge_id: String,
    pub bridge_type: String,
    pub is_running: bool,
    #[serde(rename = "loginID")]
    pub login_id: String,
    pub mode: BridgeMode,
    pub network_name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_profile: Option<RemoteProfile>,
}

impl BridgeAccountInfo {
    pub fn display_text(&self) -> Option<String> {
        self.remote_name.clone().or_else(|| {
            let p = self.remote_profile.as_ref()?;
            p.email.clone().or(p.phone.clone()).or(p.username.as_ref().map(|u| format!("@{u}"))).or(p.name.clone())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeSummary {
    #[serde(rename = "bridgeID")]
    pub bridge_id: String,
    pub bridge_type: String,
    pub is_running: bool,
    pub logins: Vec<BridgeAccountInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginFlow {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginField {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginStep {
    #[serde(rename = "stepID")]
    pub step_id: String,
    #[serde(rename = "type")]
    pub step_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(rename = "userInput", skip_serializing_if = "Option::is_none")]
    pub user_input: Option<BridgeLoginStepUserInput>,
    #[serde(rename = "displayAndWait", skip_serializing_if = "Option::is_none")]
    pub display_and_wait: Option<BridgeLoginStepDisplay>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookies: Option<BridgeLoginStepCookies>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complete: Option<BridgeLoginStepComplete>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginStepUserInput {
    pub fields: Vec<BridgeLoginField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginStepDisplay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginStepCookies {
    pub names: Vec<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginStepComplete {
    #[serde(rename = "userLoginID")]
    pub user_login_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeLoginSession {
    #[serde(rename = "loginSessionID")]
    pub login_session_id: String,
    #[serde(rename = "bridgeID")]
    pub bridge_id: String,
    #[serde(rename = "flowID")]
    pub flow_id: String,
    pub step: BridgeLoginStep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "chatID", skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(rename = "messageID", skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat: Option<Chat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[serde(alias = "TextsLight")]
    Light,
    #[serde(alias = "TextsDark", alias = "Dracula", alias = "Nocturne", alias = "DarkPurple")]
    Dark,
    #[serde(alias = "Custom")]
    System,
}

impl ThemeChoice {
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::System];

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::System => "System",
        }
    }

    pub const fn is_dark(&self, system_dark: bool) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::System => system_dark,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BeeperSettings {
    pub theme: ThemeChoice,
    pub zoom_percent: u16,
    pub enable_pas_api: bool,
    pub pas_port: u16,
    pub prefs: std::collections::BTreeMap<String, u8>,
    pub hotkeys_off: std::collections::BTreeSet<String>,
    pub reminders_order: u8,
}

impl BeeperSettings {
    pub fn pref(&self, key: &str) -> u8 {
        let p = crate::prefs::find(key);
        self.prefs.get(key).copied().unwrap_or(p.default_value())
    }

    pub fn on(&self, key: &str) -> bool {
        assert!(matches!(crate::prefs::find(key).control, crate::prefs::Control::Toggle(_)), "{key} is a toggle");
        self.pref(key) == 1
    }

    pub fn hotkey_on(&self, name: &str) -> bool {
        crate::prefs::assert_hotkey(name);
        !self.hotkeys_off.contains(name)
    }

    pub fn set_pref(&mut self, key: &str, value: u8) {
        assert!(value <= crate::prefs::find(key).max_value(), "{key} = {value} is out of range");
        self.prefs.insert(key.to_string(), value);
    }

    pub fn assert_valid(&self) {
        for (k, v) in &self.prefs {
            self.clone().set_pref(k, *v);
        }
        self.hotkeys_off.iter().for_each(|h| crate::prefs::assert_hotkey(h));
        assert!(self.reminders_order < 4, "reminders order {} is out of range", self.reminders_order);
        assert!((50..=300).contains(&self.zoom_percent), "zoom {}% is out of range", self.zoom_percent);
    }
}
