use serde::{Deserialize, Serialize};

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
pub const DELETE_FOR_ME: u8 = 1 << 0;
pub const DELETE_FOR_EVERYONE: u8 = 1 << 1;
/// The `Chat::block` bits: the `block_user`, `no_unblock` and `report_spam` room features, and Desktop's
/// `userBlocked`, the `com.beeper.user_blocked` of the `m.bridge` channel.
pub const BLOCK_USER: u8 = 1 << 0;
pub const NO_UNBLOCK: u8 = 1 << 1;
pub const REPORT_SPAM: u8 = 1 << 2;
pub const USER_BLOCKED: u8 = 1 << 3;
/// Desktop's `canCurrentUserEditTitle`, `canCurrentUserEditAvatar` and `canCurrentUserEditDescription`, by state event.
pub const EDITS: [&str; 3] = ["m.room.name", "m.room.avatar", "m.room.topic"];
/// Desktop's `member_actions` room features.
pub const MEMBER_ACTIONS: [&str; 5] = ["ban", "invite", "kick", "leave", "revoke_invite"];
pub const ALL_MEMBER_ACTIONS: u8 = 31;
pub const MEMBER_INVITE: u8 = 1 << 1;
pub const MEMBER_KICK: u8 = 1 << 2;
pub const MEMBER_LEAVE: u8 = 1 << 3;
pub const ALL_EDITS: u8 = (1 << EDITS.len()) - 1;
pub const DISAPPEARING_TIMER: &str = "com.beeper.disappearing_timer";

/// date-fns `formatDistanceStrict` in English, as Desktop shows a disappearing timer.
pub fn timer_distance(ms: u64) -> String {
    let n = |v: f64, unit: &str| {
        let v = v.round() as u64;
        format!("{v} {unit}{}", if v == 1 { "" } else { "s" })
    };
    match ms as f64 / 60_000.0 {
        m if m < 1.0 => n(ms as f64 / 1000.0, "second"),
        m if m < 60.0 => n(m, "minute"),
        m if m < 1440.0 => n(m / 60.0, "hour"),
        m if m < 43_200.0 => n(m / 1440.0, "day"),
        m if m < 525_600.0 => n(m / 43_200.0, "month"),
        m => n(m / 525_600.0, "year"),
    }
}
/// Desktop's `qLt`: the disappearing timers in ms when the room features do not list them.
pub const DEFAULT_TIMERS: [u64; 11] = [
    10_000,
    300_000,
    1_800_000,
    3_600_000,
    14_400_000,
    86_400_000,
    172_800_000,
    259_200_000,
    604_800_000,
    2_592_000_000,
    31_536_000_000,
];
/// Desktop's `es` file kinds of `com.beeper.room_features`.
pub const FILE_IMAGE: &str = "m.image";
pub const FILE_AUDIO: &str = "m.audio";
pub const FILE_VIDEO: &str = "m.video";
pub const FILE_FILE: &str = "m.file";
pub const FILE_GIF: &str = "fi.mau.gif";
pub const FILE_VOICE: &str = "org.matrix.msc3245.voice";
/// The limits of one file kind of `com.beeper.room_features`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FileRule {
    /// The `Xn` level of each MIME type, `type/*` or `*/*`: `Rejected` is -2, `Unsupported` 0, `FullySupported` 2.
    #[serde(default)]
    pub mime_types: std::collections::BTreeMap<String, i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    /// In seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<f64>,
}
/// The file kinds that a chat accepts.
pub type FileRules = std::collections::BTreeMap<String, FileRule>;
/// Desktop's `OBe.file`: the file features of a Beeper room without `com.beeper.room_features`.
pub fn beeper_files() -> FileRules {
    default_files(&serde_json::Value::Null)
}

/// Desktop's `roomFeatures` of a room without `com.beeper.room_features`: the `file` of the `defaultFeatures` of the
/// `m.bridge` protocol, else of the bridge (Desktop's `lP`), else of the generic bridge.
pub fn default_files(bridge: &serde_json::Value) -> FileRules {
    static TABLE: std::sync::LazyLock<std::collections::HashMap<String, FileRules>> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("room_files.json")).expect("the default room features parse")
    });
    // Desktop's `Od` entries that share a table.
    let table = |k: &str| match k {
        "android-sms" | "dummybridge" | "google-messages" | "gvoice" => TABLE.get("generic"),
        "facebook" | "instagram" | "instagramgo" => TABLE.get("facebookgo"),
        k => TABLE.get(k),
    };
    // Desktop's `lP` leaves out Google Messages and the iMessage cloud bridges.
    let platform = match bridge["com.beeper.bridge_name"].as_str() {
        None | Some("hungryserv") => Some("beeper"),
        Some("androidsms") => Some("android-sms"),
        Some("gmessages" | "imessage" | "imessagecloud" | "imessagego") => None,
        name => name,
    };
    let rules = bridge["protocol"]["id"].as_str().and_then(table).or_else(|| platform.and_then(table));
    rules.unwrap_or_else(|| table("generic").expect("a generic table")).clone()
}
/// The `com.beeper.room_features` formatting keys of Desktop's formatting menu buttons, in menu order.
pub const FORMATS: [&str; 9] = [
    "bold",
    "italic",
    "strikethrough",
    "inline_link",
    "unordered_list",
    "ordered_list",
    "blockquote",
    "inline_code",
    "code_block",
];
/// Desktop's formatting for a Matrix room without room features.
pub const ALL_FORMATS: u16 = (1 << FORMATS.len()) - 1;

pub const MSG_SENDER_SELF: u16 = 1 << 0;
pub const MSG_UNREAD: u16 = 1 << 1;
pub const MSG_BOOKMARKED: u16 = 1 << 2;
pub const MSG_VOICE_NOTE: u16 = 1 << 3;
pub const MSG_EDITED: u16 = 1 << 5;
pub const MSG_DELETED: u16 = 1 << 6;
/// The message mentions the user in `m.mentions`.
pub const MSG_MENTION: u16 = 1 << 7;

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

/// The secrets of the account that this device has. The recovery code is the key that the user sees.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeeperStateSecrets {
    pub master_key: bool,
    pub self_signing_key: bool,
    pub user_signing_key: bool,
    pub megolm_backup_key: bool,
    pub recovery_code: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

impl BeeperStateRecord {
    /// This device has the private cross-signing keys and the key of the key backup.
    fn has_secrets(&self) -> bool {
        let s = &self.secrets;
        s.master_key && s.self_signing_key && s.user_signing_key && s.megolm_backup_key
    }

    /// Desktop's `getAppState`: the next setup step of the account.
    pub fn app_state(&self) -> &'static str {
        let secrets = self.has_secrets();
        match self {
            r if r.user_id.is_empty() && r.device_id.is_empty() && r.access_token.is_empty() => "needs-login",
            r if !r.initialized => "initializing",
            r if !r.cross_signing => "needs-cross-signing-setup",
            r if !r.verified => "needs-verification",
            r if !secrets || !r.key_backup => "needs-secrets",
            r if !r.first_sync_done => "needs-first-sync",
            _ => "ready",
        }
    }

    /// Desktop's `authenticationStage` after the verification: the keys and the sync that the onboarding waits for.
    pub fn stage(&self) -> &'static str {
        match (self.has_secrets(), self.key_backup, self.first_sync_done) {
            (false, _, _) => "awaiting-secrets",
            (true, false, _) => "awaiting-key-backup",
            (true, true, false) => "syncing",
            (true, true, true) => "done",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatrixSession {
    pub user_id: String,
    pub access_token: String,
    pub device_id: String,
    /// The Beeper JWT of the sign-in.
    #[serde(skip)]
    pub jwt: Option<String>,
}

/// Desktop's `subscriptionPlanId`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Plan {
    #[default]
    Free,
    Plus,
    PlusPlus,
}

/// The Beeper account from `whoami`. The default is what Desktop shows before `whoami` answers.
#[derive(Debug, Clone)]
pub struct Profile {
    pub full_name: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    /// Desktop's `subscriptionPlanId`. Plus and Plus Plus are Desktop's `hasActiveSubscription`.
    pub plan: Plan,
    /// Desktop's `subscriptionAction` is "subscribe", which shows Beeper Plus in Settings.
    pub can_subscribe: bool,
    /// Desktop's `loggedInUser.analyticsDisabled`, where a missing value is true.
    pub analytics_disabled: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            full_name: None,
            username: None,
            email: None,
            plan: Plan::Free,
            can_subscribe: true,
            analytics_disabled: true,
        }
    }
}

/// Desktop's `/v1/info`, in its key order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoResponse {
    pub app: InfoApp,
    pub platform: InfoPlatform,
    pub server: InfoServer,
    pub endpoints: InfoEndpoints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoApp {
    pub name: String,
    pub version: String,
    pub bundle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoEndpoints {
    pub oauth: InfoOAuthEndpoints,
    pub spec: String,
    pub mcp: String,
    pub ws_events: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoOAuthEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub introspection_endpoint: String,
    pub userinfo_endpoint: String,
    pub revocation_endpoint: String,
    pub registration_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoPlatform {
    pub os: String,
    pub arch: String,
    pub release: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoServer {
    pub status: String,
    pub base_url: String,
    pub port: u16,
    pub hostname: String,
    pub remote_access: bool,
    pub mcp_enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(rename = "isAdmin", skip_serializing_if = "Option::is_none")]
    pub is_admin: Option<bool>,
    /// The `MANAGE_` bits: what the power level of the own user allows on this member.
    #[serde(default, skip_serializing)]
    pub manage: u8,
}

/// Desktop's `participantsCurrentUserCanKick`.
pub const MANAGE_KICK: u8 = 1;
/// Desktop's `participantsCurrentUserCanChangePowerLevels`.
pub const MANAGE_ROLE: u8 = 2;

/// Desktop's API `Account`, in its key order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    #[serde(rename = "accountID")]
    pub account_id: String,
    #[serde(rename = "loginID")]
    pub login_id: String,
    pub bridge: AccountBridge,
    pub network: String,
    pub user: User,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBridge {
    pub id: String,
    #[serde(rename = "type")]
    pub bridge_type: String,
    /// `cloud` or `local`.
    pub provider: String,
}

/// Desktop's `mapAccountStatus`: the API status of a bridge state event.
pub fn account_status(state_event: &str) -> &'static str {
    match state_event {
        "CONNECTED" => "connected",
        "CONNECTING" => "connecting",
        "BACKFILLING" => "backfilling",
        "BAD_CREDENTIALS" => "reconnect_required",
        "UNCONFIGURED" => "connection_required",
        "TRANSIENT_DISCONNECT" | "UNKNOWN_ERROR" | "LOGGED_OUT" => "disconnected",
        _ => "attention_required",
    }
}

/// A Desktop label: a private Matrix space with `com.beeper.label` in its create event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    /// `com.beeper.label.color`: an index into Desktop's label colors.
    #[serde(skip)]
    pub color: Option<u8>,
    /// `com.beeper.label.icon`.
    #[serde(skip)]
    pub icon: Option<String>,
    /// `com.beeper.muted`.
    #[serde(skip)]
    pub muted: bool,
    /// `com.beeper.label.inbox_visibility`: "dmOnly" or "none". Desktop shows all chats without it.
    #[serde(skip)]
    pub inbox: Option<&'static str>,
    /// The chats of the `m.space.child` events with a `via`.
    #[serde(skip)]
    pub rooms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatParticipants {
    #[serde(rename = "hasMore")]
    pub has_more: bool,
    pub items: Vec<User>,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatDraftAttachment {
    #[serde(rename = "uploadID")]
    pub upload_id: String,
    #[serde(rename = "fileName", skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatDraft {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attachments: Vec<ChatDraftAttachment>,
    #[serde(rename = "replyToMessageID", skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Desktop's draft `send_later`: the Unix milliseconds to send at, and "if no reply".
    #[serde(rename = "sendLater", skip_serializing_if = "Option::is_none", default)]
    pub send_later: Option<(i64, bool)>,
    /// The links whose preview you removed. Desktop keeps them as `{ matched_url }` link previews.
    #[serde(rename = "discardedLinks", skip_serializing_if = "Vec::is_empty", default)]
    pub discarded_links: Vec<String>,
    /// The mention pills of the text: the label without "@", and the user ID. Desktop keeps them in the editor document.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mentions: Vec<(String, String)>,
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
    /// Desktop's local `note` thread prop, the Notes of the Chat Info panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The sort key of the newest message or reaction; a newer one than the done mark unarchives the chat.
    #[serde(rename = "lastNotifiableOrder", skip_serializing_if = "Option::is_none")]
    pub notifiable: Option<String>,
    /// Whether the inbox rule of the account hides the chat.
    #[serde(rename = "isHiddenFromInbox", skip_serializing_if = "Option::is_none")]
    pub is_hidden_from_inbox: Option<bool>,
    /// Whether "Only DMs" hides the chat: a group that is not a group DM.
    #[serde(rename = "inboxGroup", default)]
    pub inbox_group: bool,
    #[serde(rename = "isRequest", skip_serializing_if = "Option::is_none")]
    pub is_request: Option<bool>,
    #[serde(rename = "networkName", skip_serializing_if = "Option::is_none")]
    pub network_name: Option<String>,
    #[serde(rename = "networkID", skip_serializing_if = "Option::is_none")]
    pub network_id: Option<String>,
    #[serde(rename = "imgURL", skip_serializing_if = "Option::is_none")]
    pub img_url: Option<String>,
    #[serde(rename = "deleteModes", default)]
    pub delete_modes: u8,
    /// Whether the bridge accepts a message request with a button (`message_request.accept_with_button`).
    #[serde(rename = "canAcceptRequest", default)]
    pub can_accept_request: bool,
    /// The `BLOCK_USER`, `NO_UNBLOCK`, `REPORT_SPAM` and `USER_BLOCKED` bits of a DM.
    #[serde(default)]
    pub block: u8,
    /// The `EDITS` that the user can send, one bit each.
    #[serde(default = "all_edits")]
    pub edits: u8,
    /// The `MEMBER_ACTIONS` that the room supports, one bit each.
    #[serde(rename = "memberActions", default = "all_member_actions")]
    pub member_actions: u8,
    /// The `FORMATS` the network fully supports, one bit each.
    #[serde(default = "all_formats")]
    pub formatting: u16,
    /// Desktop's `roomFeatures.file`: the file kinds that the network accepts.
    #[serde(default = "beeper_files")]
    pub file: FileRules,
    /// The room has `com.beeper.room_features`. Desktop converts the files for such a room before the upload, so it
    /// checks the image size only after the conversion. Without them, `file` and the other features are defaults.
    #[serde(rename = "hasRoomFeatures", default)]
    pub room_features: bool,
    /// The poll features of the network, or `None` when it has no polls.
    #[serde(default = "beeper_polls")]
    pub polls: Option<PollRules>,
    /// The edit, delete, reaction and reply features of the network.
    #[serde(default = "beeper_messages")]
    pub messages: MessageRules,
    /// Desktop's `disappearingTimer`: the `com.beeper.disappearing_timer` of new messages in ms.
    #[serde(rename = "disappearingTimer", default, skip_serializing_if = "Option::is_none")]
    pub disappearing: Option<u64>,
    /// Desktop's `roomFeatures.disappearing_timer`, or `None` when the room has no disappearing messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timers: Option<Timers>,
    #[serde(default, skip_serializing_if = "Older::is_unknown")]
    pub older: Older,
    /// Desktop's `isReadOnly`: your power level is below the one of messages, or an upgrade replaced the room.
    #[serde(rename = "isReadOnly", default)]
    pub is_read_only: bool,
    /// Desktop's `extra.replacementRoomID`: the room of the `m.room.tombstone` of the chat.
    #[serde(rename = "replacementRoomID", default, skip_serializing_if = "Option::is_none")]
    pub replacement_room: Option<String>,
    /// Desktop's `canMentionRoom` with the room's `at_room_mention` feature: the composer offers "@room".
    #[serde(rename = "canMentionRoom", default)]
    pub can_mention_room: bool,
    /// Desktop's `supportsThreads`: the `thread` room feature is above 0.
    #[serde(rename = "supportsThreads", default)]
    pub threads: bool,
    /// Desktop's `com.beeper.thread_follows` room account data.
    #[serde(rename = "threadFollows", default, skip_serializing_if = "ThreadFollows::is_empty")]
    pub follows: ThreadFollows,
    /// Desktop's `lastReadHsOrder` of each thread: the sort key that the thread is read through, by root ID.
    #[serde(rename = "threadReads", default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub thread_reads: std::collections::BTreeMap<String, String>,
    /// The personal name or photo, which `title` and `img_url` show instead of the room's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal: Option<Box<Personal>>,
    #[serde(skip)]
    pub flags: u16,
}

/// Desktop's `com.beeper.thread_follows`: the threads of a room that the user follows or unfollows explicitly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadFollows {
    /// Desktop's `threadFollowAll`, or `None` to follow all threads of a room with 20 members or fewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_all: Option<bool>,
    #[serde(default)]
    pub followed: Vec<String>,
    #[serde(default)]
    pub unfollowed: Vec<String>,
}

impl ThreadFollows {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Desktop's `parseThreadFollows`, where `following` is the old name of `followed`, or `None` for bad content.
    pub fn parse(v: &serde_json::Value) -> Option<Self> {
        let ids = |k: &str| -> Option<Vec<String>> {
            v[k].as_array()?.iter().map(|i| i.as_str().map(String::from)).collect()
        };
        let followed = if v.get("followed").is_some() { ids("followed") } else { ids("following") };
        Some(Self { follow_all: v["follow_all"].as_bool(), followed: followed?, unfollowed: ids("unfollowed")? })
    }

    /// Desktop's `followsAllThreads`: the follow-all state, which is on in a room with 20 `members` or fewer.
    pub fn follows_all(&self, members: usize) -> bool {
        self.follow_all.unwrap_or(members <= 20)
    }

    /// Desktop's `followsThread`: the explicit follow, else whether the user `participated`, else the follow-all
    /// state.
    pub fn follows(&self, root: &str, members: usize, participated: bool) -> bool {
        match (self.followed.iter().any(|r| r == root), self.unfollowed.iter().any(|r| r == root)) {
            (true, _) => true,
            (_, true) => false,
            _ => participated || self.follows_all(members),
        }
    }

    /// Desktop's `setThreadFollowing` (`Fp`): the thread of `root` moves to the newest 1000 follows or unfollows.
    pub fn set(&mut self, root: &str, on: bool) {
        self.followed.retain(|r| r != root);
        self.unfollowed.retain(|r| r != root);
        let list = if on { &mut self.followed } else { &mut self.unfollowed };
        list.push(root.into());
        list.drain(..list.len().saturating_sub(1000));
    }
}

/// Where the older messages of a chat load from, like the room pagination state of Desktop's SDK.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Older {
    /// The chat did not load a page yet, so the older messages load back from the newest event.
    #[default]
    Unknown,
    /// The pagination token of the next older page.
    From(String),
    /// The chat loaded all messages back to the start of the room.
    Start,
}

impl Older {
    fn is_unknown(&self) -> bool {
        *self == Older::Unknown
    }
}

/// Desktop's `PERSONAL_ROOM_NAME_EVENT_TYPE` and `PERSONAL_ROOM_AVATAR_EVENT_TYPE` (MSC4431) room account data.
pub const PERSONAL_NAME: &str = "de.gematik.msc4431.room.name.private";
pub const PERSONAL_AVATAR: &str = "de.gematik.msc4431.room.avatar.private";
/// Desktop's `PERSONAL_ROOM_NAME_MAX_LENGTH`.
pub const PERSONAL_NAME_MAX: usize = 256;

/// Desktop's `personalTitle` and `personalImgURL`, with the `canonicalTitle` and `imgURL` of the room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Personal {
    pub title: Option<String>,
    pub img_url: Option<String>,
    pub room_title: String,
    pub room_img_url: Option<String>,
}

/// Desktop's `BOOKMARK_EVENT_TYPE` and `BOOKMARK_REL_TYPE`.
pub const BOOKMARK: &str = "com.beeper.bookmark";
/// Desktop's `BOOKMARKS_ROOM_ID`: the managed room of the bookmarks on the Beeper homeserver.
pub const BOOKMARKS_ROOM: &str = "!bookmarks:beeper.local";
/// The event type and the relation type of a transcription, and Desktop's `TRANSCRIPTIONS_ROOM_ID`, its room.
pub const TRANSCRIPTION: &str = "com.beeper.transcription";
pub const TRANSCRIPTIONS_ROOM: &str = "!transcriptions:beeper.local";
/// The managed rooms on the Beeper homeserver that keep data and are not chats: the rooms of Desktop's
/// `managedRoomEventHandlers` and `TRANSCRIPTIONS_ROOM_ID`. `!tips:beeper.local` is also a managed room, but Desktop
/// shows it as a chat.
pub const MANAGED_ROOMS: [&str; 5] = [
    BOOKMARKS_ROOM,
    TRANSCRIPTIONS_ROOM,
    "!blasts:beeper.local",
    "!removed-messages:beeper.local",
    "!reminders:beeper.local",
];

/// A `com.beeper.bookmark` event of the bookmarks room, which relates to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub room_id: String,
    pub event_id: String,
    /// `com.beeper.hs.order`, which Desktop sorts the bookmarks of a chat by.
    pub order: i64,
}

/// The disappearing timers of a room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timers {
    /// The first `types` entry, which Desktop sets with a timer.
    pub kind: String,
    /// The timers in ms, without Off.
    pub timers: Vec<u64>,
    /// Desktop's `canChangeTimer`: the room supports the state event and the power level allows it.
    pub can_set: bool,
}

fn all_edits() -> u8 {
    ALL_EDITS
}

fn all_member_actions() -> u8 {
    ALL_MEMBER_ACTIONS
}

fn beeper_polls() -> Option<PollRules> {
    Some(BEEPER_POLLS)
}

/// The poll features of `com.beeper.room_features`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollRules {
    /// `poll_hidden_votes`: a poll can hide its results until it ends.
    pub hidden_votes: bool,
    /// `poll_end`: the sender can end a poll.
    pub end: bool,
    /// `poll_duplicate_options`: two options can have the same text.
    pub duplicates: bool,
    /// `poll_max_options`, or 0 for no limit.
    pub max_options: u16,
    /// `poll_option_max_length`.
    pub max_length: u16,
}

impl PollRules {
    /// Whether a poll with `n` options can have one more.
    pub fn room(self, n: usize) -> bool {
        self.max_options == 0 || n < usize::from(self.max_options)
    }
}

/// The message features of `com.beeper.room_features`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRules {
    /// `edit` is at least partly supported.
    pub edit: bool,
    /// `edit_max_age` in seconds.
    pub edit_max_age: Option<u64>,
    /// `delete` is fully supported.
    pub delete: bool,
    /// `delete_max_age` in seconds.
    pub delete_max_age: Option<u64>,
    /// `delete_for_me`.
    pub delete_for_me: bool,
    /// `reaction` is at least partly supported.
    pub react: bool,
    /// `reply` is at least partly supported.
    pub reply: bool,
}

impl MessageRules {
    /// Desktop's `Xk`: a message sent at `timestamp` is younger than `max_age`.
    pub fn young(max_age: Option<u64>, timestamp: &str) -> bool {
        max_age.is_none_or(|s| {
            let sent = crate::config::millis_from_rfc3339(timestamp).expect("RFC 3339 message");
            crate::config::now_unix_millis() - sent < s as i64 * 1000
        })
    }
}

/// Desktop's message features of a Beeper room.
pub const BEEPER_MESSAGES: MessageRules = MessageRules {
    edit: true,
    edit_max_age: None,
    delete: true,
    delete_max_age: None,
    delete_for_me: false,
    react: true,
    reply: true,
};

fn beeper_messages() -> MessageRules {
    BEEPER_MESSAGES
}

/// Desktop's poll features of a Beeper room.
pub const BEEPER_POLLS: PollRules =
    PollRules { hidden_votes: true, end: true, duplicates: true, max_options: 20, max_length: 200 };

fn all_formats() -> u16 {
    ALL_FORMATS
}

fn flag_bits(bits: &[(bool, u16)]) -> u16 {
    bits.iter().filter(|(on, _)| *on).fold(0, |f, (_, bit)| f | bit)
}

impl Chat {
    /// Desktop's `canonicalTitle`: the name of the room without the personal name.
    pub fn room_title(&self) -> &str {
        self.personal.as_ref().map_or(&self.title, |p| &p.room_title)
    }

    pub fn room_img_url(&self) -> Option<&str> {
        self.personal.as_ref().map_or(self.img_url.as_deref(), |p| p.room_img_url.as_deref())
    }

    /// Desktop's `personalTitle` and `personalImgURL`.
    pub fn personal_parts(&self) -> (Option<String>, Option<String>) {
        self.personal.as_ref().map_or((None, None), |p| (p.title.clone(), p.img_url.clone()))
    }

    /// Sets the personal name and photo, which show instead of the room's.
    pub fn set_personal(&mut self, title: Option<String>, img_url: Option<String>) {
        let (room_title, room_img_url) = (self.room_title().to_string(), self.room_img_url().map(str::to_string));
        self.title = title.clone().unwrap_or_else(|| room_title.clone());
        self.img_url = img_url.clone().or_else(|| room_img_url.clone());
        self.personal = (title.is_some() || img_url.is_some())
            .then(|| Box::new(Personal { title, img_url, room_title, room_img_url }));
    }

    /// Changes the name or photo of the room, under the personal ones.
    pub fn edit_room(&mut self, f: impl FnOnce(&mut Chat)) {
        let (title, img_url) = self.personal_parts();
        if let Some(p) = self.personal.take() {
            (self.title, self.img_url) = (p.room_title, p.room_img_url);
        }
        f(self);
        self.set_personal(title, img_url);
    }

    /// Desktop's `Yc`: the room features list the file kind, with a supported MIME type when they list types.
    pub fn takes(&self, kind: &str) -> bool {
        self.file.get(kind).is_some_and(|r| r.mime_types.is_empty() || r.mime_types.values().any(|&l| l > 0))
    }

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub transcription: Option<Transcription>,
}

/// Desktop's `attachment.transcription`, from a `com.beeper.transcription` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcription {
    pub transcription: String,
    pub engine: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
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
    #[serde(rename = "sortKey", default, skip_serializing_if = "String::is_empty")]
    pub sort_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollOption {
    pub id: String,
    pub label: String,
    /// The voters of this option, from the latest response of each voter.
    pub voter_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    pub title: String,
    pub summary: String,
}

/// Desktop's `extra.poll`, from an MSC3381 poll start event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollData {
    pub question: String,
    pub options: Vec<PollOption>,
    /// 1 for a single answer.
    pub max_selections: u32,
    /// Desktop's `kind: "undisclosed"`: the results show when the poll ends.
    #[serde(default)]
    pub undisclosed: bool,
    /// Desktop's `isFinal`: a poll end event refers to the poll.
    #[serde(default)]
    pub ended: bool,
    /// The text of the poll start event, which Desktop shows when the chat cannot vote.
    #[serde(default)]
    pub fallback: String,
}

impl PollData {
    /// Desktop's poll vote count: the number of different voters.
    pub fn voters(&self) -> usize {
        self.options.iter().flat_map(|o| &o.voter_ids).collect::<std::collections::HashSet<_>>().len()
    }

    /// The options that `user` voted for.
    pub fn picks(&self, user: &str) -> Vec<String> {
        self.options.iter().filter(|o| o.voter_ids.iter().any(|v| v == user)).map(|o| o.id.clone()).collect()
    }

    /// Desktop's `Oz`: the latest response of `user` replaces the earlier one.
    pub fn respond(&mut self, user: &str, answers: &[String]) {
        for o in &mut self.options {
            o.voter_ids.retain(|v| v != user);
            if answers.contains(&o.id) {
                o.voter_ids.push(user.into());
            }
        }
    }
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
    /// Desktop's `isEdited`, which a deletion sets too and which outlives the edit history.
    #[serde(rename = "isEdited", skip_serializing_if = "Option::is_none")]
    pub is_edited: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub links: Vec<MessageLink>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub seen: std::collections::BTreeMap<String, String>,
    /// Desktop's `action.timer` of a `DISAPPEARING_TIMER` row: the new timer in ms, or 0 for Off.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timer: Option<u64>,
    /// Desktop's `sendStatus` of a message that the homeserver or the bridge did not take: `isErrored`, or
    /// `isRetrying` again.
    #[serde(rename = "sendError", skip_serializing_if = "Option::is_none", default)]
    pub send_error: Option<SendError>,
    /// Desktop's `isDelivered`: the bridge delivered the message to a user.
    #[serde(rename = "isDelivered", skip_serializing_if = "std::ops::Not::not", default)]
    pub is_delivered: bool,
    /// Desktop's `threadRootEventID`: the root of the Matrix thread of a reply.
    #[serde(rename = "threadRootEventID", default, skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<String>,
    /// Desktop's `extra.broadcastOutsideThread`: the thread reply shows in the chat too.
    #[serde(rename = "broadcastOutsideThread", default, skip_serializing_if = "std::ops::Not::not")]
    pub broadcast: bool,
    /// Desktop's `extra.isReplyFallback`: `linkedMessageID` is the fallback reply of a thread reply, not a quote.
    #[serde(rename = "isReplyFallback", default, skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
    #[serde(skip)]
    pub flags: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendError {
    /// The reason, which the error tooltip shows.
    pub message: Option<String>,
    pub retrying: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageLink {
    pub url: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The preview image, and its size when the preview has it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub img: Option<String>,
    #[serde(rename = "imgSize", skip_serializing_if = "Option::is_none", default)]
    pub img_size: Option<AttachmentSize>,
}

/// Desktop's `ScheduledStore` entry: a Send Later message, kept on this device only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scheduled {
    pub id: String,
    pub chat_id: String,
    pub text: String,
    pub reply_to: Option<String>,
    pub added_on: i64,
    pub send_on: i64,
    pub send_only_on_no_response: bool,
    pub canceled: bool,
    /// The stored upload of a file part, which goes with `text` as its caption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<String>,
}

impl Message {
    /// Desktop's silent rows, which neither preview a chat nor notify.
    pub fn silent(&self) -> bool {
        matches!(self.message_type.as_deref(), Some("MEMBERSHIP" | "HIDDEN" | "DISAPPEARING_TIMER"))
    }

    pub fn edited(&self) -> bool {
        self.is_edited == Some(true) || !self.edit_history.is_empty()
    }

    /// Desktop's `isThreadOnly` with `threads` on: a thread reply that only its thread shows.
    pub fn thread_only(&self, threads: bool) -> bool {
        threads && self.thread_root.is_some() && !self.broadcast
    }

    pub fn sync_flags(&mut self) {
        self.flags = flag_bits(&[
            (self.is_sender == Some(true), MSG_SENDER_SELF),
            (self.is_unread == Some(true), MSG_UNREAD),
            (self.is_bookmarked == Some(true), MSG_BOOKMARKED),
            (self.attachments.iter().any(|a| a.is_voice_note == Some(true)), MSG_VOICE_NOTE),
            (self.edited(), MSG_EDITED),
            (self.is_deleted == Some(true), MSG_DELETED),
            (self.flags & MSG_MENTION != 0, MSG_MENTION),
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

#[derive(Debug, Clone)]
pub struct VerificationSasEmoji {
    pub emoji: String,
    pub description: String,
}

fn emoji_sequence<S: serde::Serializer>(emojis: &[VerificationSasEmoji], s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&emojis.iter().map(|e| e.emoji.as_str()).collect::<String>())
}

/// The emojis and numbers to compare on both devices. The API sends the emojis as one string, like Desktop.
#[derive(Debug, Clone, Serialize)]
pub struct VerificationSas {
    #[serde(serialize_with = "emoji_sequence")]
    pub emojis: Vec<VerificationSasEmoji>,
    pub decimals: String,
}

fn base64<S: serde::Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&crate::codec::base64_encode_standard(data))
}

/// The QR code that this device shows. The API sends it as base64, like Desktop's `qr_data`.
#[derive(Debug, Clone, Serialize)]
pub struct VerificationQr {
    #[serde(serialize_with = "base64")]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerificationDevice {
    pub id: String,
    /// The display name of the device, which the UI shows. The API leaves it out, like Desktop.
    #[serde(skip)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerificationError {
    pub code: String,
    pub reason: String,
}

/// Desktop's `AppVerification`: the progress of a trusted device verification.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVerification {
    pub id: String,
    pub purpose: &'static str,
    pub direction: &'static str,
    pub state: &'static str,
    pub methods: Vec<&'static str>,
    pub available_actions: Vec<&'static str>,
    #[serde(rename = "otherUserID", skip_serializing_if = "Option::is_none")]
    pub other_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_device: Option<VerificationDevice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr: Option<VerificationQr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sas: Option<VerificationSas>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VerificationError>,
}

impl AppVerification {
    /// Desktop's `getAvailableVerificationActions`.
    pub fn actions(&self) -> Vec<&'static str> {
        match self.state {
            "requested" if self.direction == "incoming" => vec!["accept", "cancel"],
            "requested" => vec!["cancel"],
            "ready" if self.methods.contains(&"sas") => vec!["cancel", "sas.start"],
            "ready" => vec!["cancel"],
            "sas_ready" => vec!["sas.confirm", "cancel"],
            "qr_scanned" => vec!["qr.confirmScanned", "cancel"],
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AppMatrix {
    #[serde(rename = "userID")]
    pub user_id: String,
    #[serde(rename = "deviceID")]
    pub device_id: String,
    pub homeserver: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSecrets {
    pub master_key: bool,
    pub self_signing_key: bool,
    pub user_signing_key: bool,
    pub megolm_backup_key: bool,
    pub recovery_key: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppE2ee {
    pub initialized: bool,
    pub secret_storage: bool,
    pub cross_signing: bool,
    pub verified: bool,
    pub secrets: AppSecrets,
    pub key_backup: bool,
    pub first_sync_done: bool,
    pub has_backed_up_recovery_key: bool,
}

/// Desktop's `AppStateSnapshot`: the sign-in and encrypted messaging setup state of the app.
#[derive(Debug, Clone, Serialize)]
pub struct AppState {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<AppMatrix>,
    pub e2ee: AppE2ee,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<AppVerification>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// The `message` of a remote state with `has_error`, which Desktop's account page shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_error: Option<String>,
}

impl BridgeAccountInfo {
    pub fn display_text(&self) -> Option<String> {
        self.remote_name.clone().or_else(|| {
            let p = self.remote_profile.as_ref()?;
            p.email.clone().or(p.phone.clone()).or(p.username.as_ref().map(|u| format!("@{u}"))).or(p.name.clone())
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

/// A login session of the Desktop API: Desktop's `LoginSession` and the bridge step that it shows.
#[derive(Debug, Clone)]
pub struct BridgeLoginSession {
    pub view: serde_json::Value,
    pub step: crate::bridge_login::Step,
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
    /// Desktop's `desktop_api_remote_access`: serve the Desktop API on all interfaces from the next launch.
    pub remote_access: bool,
    pub prefs: std::collections::BTreeMap<String, u8>,
    /// Desktop's `keybindings`: the keys of the hotkeys that are not the default, by name. "disabled" turns a
    /// hotkey off, and an empty value is on without keys.
    pub keybindings: std::collections::BTreeMap<String, String>,
    pub reminders_order: u8,
    /// The input device name, or empty for the system default.
    pub microphone: String,
    /// Desktop's local `accountProps`, by account ID.
    pub accounts: std::collections::BTreeMap<String, AccountProps>,
    /// Desktop's `time_suggestion_last_used`: the last time phrase that set a reminder, or empty.
    pub last_time: String,
    /// Desktop's local `chat_appearance`.
    pub chat_appearance: ChatAppearance,
    /// Desktop's local `spaces_order`: the sidebar order, and the spaces that Edit Spaces hides.
    pub spaces_order: Vec<SpaceOrder>,
    /// Desktop's local `has_ever_created_label`: the Create Label nudge stays hidden after the first label.
    pub ever_labeled: bool,
    /// Desktop's local `labels_intro_seen`: New Label shows its intro only the first time.
    pub labels_intro_seen: bool,
    /// Desktop's local `downloads_dir_path`, or empty for the downloads folder of the OS.
    pub downloads_dir: String,
    /// Desktop's synced `message_double_click_reaction`: the emoji of Quick React.
    pub quick_reaction: String,
    /// Desktop's local `customQuickReactions`: the custom quick reactions.
    pub quick_reactions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceOrder {
    pub id: String,
    pub visible: bool,
}

/// Desktop's `chatAppearanceStore`: the default chat theme, and the themes of single chats.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatAppearance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<ChatTheme>,
    #[serde(rename = "perChat")]
    pub per_chat: std::collections::BTreeMap<String, ChatTheme>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatTheme {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bubble: Option<Bubble>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Wallpaper {
    None,
    Color {
        id: String,
    },
    Gradient {
        id: String,
    },
    /// A photo in `chat_wallpapers` of the data directory.
    Photo {
        #[serde(rename = "fileName")]
        file_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Bubble {
    Default,
    Swatch { id: String },
}

impl ChatAppearance {
    /// Desktop's `configFor`: each part of the theme of a chat falls back to the default theme.
    pub fn theme(&self, chat: Option<&str>) -> ChatTheme {
        let own = chat.and_then(|c| self.per_chat.get(c));
        let part = |f: fn(&ChatTheme) -> bool| own.filter(|t| f(t)).or(self.global.as_ref()).filter(|t| f(t));
        ChatTheme {
            wallpaper: part(|t| t.wallpaper.is_some()).and_then(|t| t.wallpaper.clone()),
            bubble: part(|t| t.bubble.is_some()).and_then(|t| t.bubble.clone()),
        }
    }

    /// Desktop's `referencedPhotos`: the file names of the photo wallpapers.
    pub fn photos(&self) -> impl Iterator<Item = &str> {
        self.global.iter().chain(self.per_chat.values()).filter_map(|t| match &t.wallpaper {
            Some(Wallpaper::Photo { file_name }) => Some(file_name.as_str()),
            _ => None,
        })
    }

    /// Desktop's `patchConfig`: `f` changes the stored theme of the chat, or the default theme for `None`. A theme
    /// without parts is removed.
    pub fn patch(&mut self, chat: Option<&str>, f: impl FnOnce(&mut ChatTheme)) {
        let mut theme = match chat {
            Some(c) => self.per_chat.remove(c),
            None => self.global.take(),
        }
        .unwrap_or_default();
        f(&mut theme);
        let theme = (theme != ChatTheme::default()).then_some(theme);
        match (chat, theme) {
            (Some(c), Some(t)) => _ = self.per_chat.insert(c.to_string(), t),
            (Some(_), None) => {}
            (None, t) => self.global = t,
        }
    }
}

/// The notification settings of an account page, which Desktop keeps on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountProps {
    /// "Enable notifications" (`enabled_notifications`); off mutes the account.
    pub notifications: bool,
    /// "Show message previews" (`show_notification_previews`).
    pub previews: bool,
    /// The "Sound" None (`notification_sound`); Default uses the sound of the chat type.
    pub silent: bool,
}

impl Default for AccountProps {
    fn default() -> Self {
        Self { notifications: true, previews: true, silent: false }
    }
}

impl BeeperSettings {
    pub fn pref(&self, key: &str) -> u8 {
        let p = crate::prefs::find(key);
        self.prefs.get(key).copied().unwrap_or(p.default_value())
    }

    pub fn choice(&self, key: &str) -> &'static str {
        let crate::prefs::Control::Select(options, _) = crate::prefs::find(key).control else {
            panic!("{key} is a select")
        };
        options[usize::from(self.pref(key))]
    }

    pub fn secs(&self, key: &str) -> u64 {
        let Some(crate::prefs::Ad::Desktop(_, crate::prefs::Values::Secs(values))) = crate::prefs::synced(key) else {
            panic!("{key} is in seconds")
        };
        u64::try_from(values[usize::from(self.pref(key))]).expect("positive seconds")
    }

    pub fn on(&self, key: &str) -> bool {
        assert!(matches!(crate::prefs::find(key).control, crate::prefs::Control::Toggle(_)), "{key} is a toggle");
        self.pref(key) == 1
    }

    pub fn account(&self, id: &str) -> AccountProps {
        self.accounts.get(id).copied().unwrap_or_default()
    }

    pub fn hotkey_keys(&self, name: &str) -> &str {
        let default = crate::prefs::hotkey_default(name);
        self.keybindings.get(name).map_or(default, String::as_str)
    }

    pub fn set_pref(&mut self, key: &str, value: u8) {
        assert!(value <= crate::prefs::find(key).max_value(), "{key} = {value} is out of range");
        self.prefs.insert(key.to_string(), value);
    }

    pub fn assert_valid(&self) {
        for (k, v) in &self.prefs {
            self.clone().set_pref(k, *v);
        }
        self.keybindings.keys().for_each(|h| _ = crate::prefs::hotkey_default(h));
        assert!(self.reminders_order < 4, "reminders order {} is out of range", self.reminders_order);
        assert!((40..=300).contains(&self.zoom_percent), "zoom {}% is out of range", self.zoom_percent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooms_without_features_take_the_bridge_defaults_like_desktop() {
        let files = |bridge: serde_json::Value| default_files(&bridge);
        let beeper = beeper_files();
        assert_eq!(beeper[FILE_FILE].max_size, Some(100_000_000), "Desktop's `OBe`");
        assert_eq!(beeper[FILE_VOICE].mime_types["audio/ogg"], 2);
        let whatsapp = files(serde_json::json!({ "com.beeper.bridge_name": "whatsapp" }));
        assert_eq!(whatsapp[FILE_GIF].mime_types, [("image/gif".into(), -2), ("video/mp4".into(), 2)].into());
        assert_eq!(whatsapp[FILE_IMAGE].max_size, Some(2000 * 1024 * 1024));
        let telegram = files(serde_json::json!({ "com.beeper.bridge_name": "x", "protocol": { "id": "telegram" } }));
        assert_eq!(telegram[FILE_IMAGE].max_size, Some(10 * 1024 * 1024), "the protocol comes first");
        let instagram = files(serde_json::json!({ "com.beeper.bridge_name": "instagramgo" }));
        assert_eq!(instagram["m.sticker"].mime_types["image/webp"], 2);
        let generic = files(serde_json::json!({ "com.beeper.bridge_name": "gmessages" }));
        assert_eq!(generic[FILE_IMAGE].max_size, Some(20 * 1024 * 1024), "Desktop leaves Google Messages out");
        assert!(files(serde_json::json!({ "com.beeper.bridge_name": "groupme" })).is_empty());
    }

    #[test]
    fn verification_actions_follow_desktop() {
        let mut v = AppVerification {
            id: "flow".into(),
            purpose: "device",
            direction: "incoming",
            state: "requested",
            methods: Vec::new(),
            available_actions: Vec::new(),
            other_user_id: Some("@me:x".into()),
            other_device: Some(VerificationDevice { id: "DEV".into(), name: Some("Pixel".into()) }),
            qr: None,
            sas: None,
            error: None,
        };
        assert_eq!(v.actions(), ["accept", "cancel"]);
        v.direction = "outgoing";
        assert_eq!(v.actions(), ["cancel"]);
        (v.state, v.methods) = ("ready", vec!["sas"]);
        assert_eq!(v.actions(), ["cancel", "sas.start"]);
        v.state = "sas_ready";
        assert_eq!(v.actions(), ["sas.confirm", "cancel"]);
        v.state = "qr_scanned";
        assert_eq!(v.actions(), ["qr.confirmScanned", "cancel"]);
        v.state = "done";
        assert!(v.actions().is_empty());
        let json = serde_json::to_value(&v).expect("a verification is JSON");
        assert_eq!(json["otherUserID"], "@me:x");
        assert_eq!(json["otherDevice"], serde_json::json!({ "id": "DEV" }), "the API leaves out the device name");
    }
}
