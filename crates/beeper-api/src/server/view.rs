//! The API shapes of Desktop's `mapChat`, `mapMessage`, and bridges, in their key order, made from the store.

use beeper_core::config::{account_network, cloud_bridge, find_network, rfc3339_from_millis, NETWORK_SPECS};
use beeper_core::models::*;
use beeper_core::store::BeeperStore;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Desktop's `Bridge` of `listBridges`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Bridge<'a> {
    id: &'static str,
    #[serde(rename = "type")]
    bridge_type: &'static str,
    provider: &'static str,
    display_name: &'static str,
    network: &'static str,
    active_account_count: usize,
    supports_multiple_accounts: bool,
    status: &'static str,
    accounts: Vec<&'a Account>,
}

/// Desktop's bridge ID of a network: the ID of its Beeper Cloud bridge, such as `slackgo`.
pub(super) fn bridge_id(spec: &NetworkSpec) -> &'static str {
    cloud_bridge(spec).unwrap_or(spec.bridge_id)
}

/// The network of a bridge ID of the API.
pub(super) fn bridge_network(id: &str) -> Option<&'static NetworkSpec> {
    NETWORK_SPECS.iter().find(|n| bridge_id(n) == id)
}

/// The accounts of a network. Cloud accounts carry the cloud bridge ID, and the demo accounts the network ID.
fn network_accounts<'a>(st: &'a BeeperStore, spec: &NetworkSpec) -> Vec<&'a Account> {
    let of = |a: &&Account| find_network(&a.bridge.id).is_some_and(|n| n.bridge_id == spec.bridge_id);
    st.accounts.iter().filter(of).collect()
}

/// Desktop's `listBridges` entry of a network. No network allows more than one account here.
pub(super) fn bridge<'a>(st: &'a BeeperStore, spec: &NetworkSpec) -> Bridge<'a> {
    let accounts = network_accounts(st, spec);
    Bridge {
        id: bridge_id(spec),
        bridge_type: bridge_id(spec),
        provider: if spec.supports_cloud { "cloud" } else { "local" },
        display_name: spec.display_name,
        network: spec.display_name,
        active_account_count: accounts.len(),
        supports_multiple_accounts: false,
        status: if accounts.is_empty() { "available" } else { "connected" },
        accounts,
    }
}

/// Desktop's `BridgeLogin`: the accounts of one bridge login.
#[derive(Serialize)]
pub(super) struct BridgeLogin<'a> {
    #[serde(rename = "loginID")]
    pub(super) login_id: &'a str,
    #[serde(rename = "bridgeID")]
    bridge_id: &'a str,
    #[serde(rename = "accountIDs")]
    pub(super) account_ids: Vec<&'a str>,
    user: &'a User,
    status: &'static str,
    #[serde(rename = "removeScopes")]
    pub(super) remove_scopes: &'static [&'static str],
}

/// Desktop's `mapBridgeAccountsToLogin` for each login of the bridge.
pub(super) fn logins<'a>(st: &'a BeeperStore, spec: &NetworkSpec) -> Vec<BridgeLogin<'a>> {
    let accounts = network_accounts(st, spec);
    let mut ids: Vec<&str> = accounts.iter().map(|a| a.login_id.as_str()).collect();
    ids.dedup();
    let login = |id: &'a str| {
        let mine: Vec<&'a Account> = accounts.iter().copied().filter(|a| a.login_id == id).collect();
        let any = |s: &[&str]| mine.iter().any(|a| s.contains(&a.status.as_str()));
        BridgeLogin {
            login_id: id,
            bridge_id: bridge_id(spec),
            account_ids: mine.iter().map(|a| a.account_id.as_str()).collect(),
            user: &mine[0].user,
            status: match () {
                _ if any(&["connected"]) => "connected",
                _ if any(&["connecting", "backfilling"]) => "connecting",
                _ if any(&["reconnect_required", "connection_required"]) => "needs_login",
                _ if any(&["disconnected"]) => "logged_out",
                _ => "unknown",
            },
            remove_scopes: match mine[0].bridge.provider.as_str() {
                "local" => &["current-device", "all-devices"],
                _ => &["all-devices"],
            },
        }
    };
    ids.into_iter().map(login).collect()
}

/// Desktop's `mapLoginStep`: the API shape of a bridge login step. Like Desktop, it reads only the keys that it
/// knows, with their types.
fn login_step(raw: &Value) -> Value {
    let text = |v: &Value| v.as_str().map(Value::from);
    let set = |o: &mut Value, k: &str, v: Option<Value>| {
        if let Some(v) = v {
            o[k] = v;
        }
    };
    let id = raw["step_id"].as_str().unwrap_or_default();
    let mut step = json!({ "type": raw["type"], "stepID": id });
    set(&mut step, "instructions", text(&raw["instructions"]));
    match raw["type"].as_str() {
        Some("user_input") => {
            let fields = raw["user_input"]["fields"].as_array().into_iter().flatten().map(|f| {
                let mut field = json!({ "id": f["id"].as_str().unwrap_or_default() });
                for (k, from) in [
                    ("type", "type"),
                    ("label", "label"),
                    ("placeholder", "placeholder"),
                    ("initialValue", "initial_value"),
                ] {
                    set(&mut field, k, text(&f[from]));
                }
                set(&mut field, "optional", f["optional"].as_bool().map(Value::from));
                field
            });
            step["fields"] = fields.collect::<Vec<_>>().into();
            set(&mut step, "attachments", raw["user_input"]["attachments"].as_array().map(|a| a.clone().into()));
        }
        Some("cookies") => {
            let c = &raw["cookies"];
            step["url"] = c["url"].as_str().unwrap_or_default().into();
            set(&mut step, "userAgent", text(&c["user_agent"]));
            let fields = c["fields"].as_array().into_iter().flatten().map(|f| {
                let mut field = json!({ "id": f["id"].as_str().or(f["name"].as_str()).unwrap_or_default() });
                let kind = f["type"].as_str().filter(|t| ["cookie", "header", "local_storage"].contains(t));
                set(&mut field, "type", kind.map(Value::from));
                set(&mut field, "name", text(&f["name"]));
                field
            });
            step["fields"] = fields.collect::<Vec<_>>().into();
            set(&mut step, "expectedFinalURLRegex", text(&c["wait_for_url_pattern"]));
            set(&mut step, "extractJS", text(&c["extract_js"]));
        }
        Some("display_and_wait") => {
            let d = &raw["display_and_wait"];
            let data = d["data"].as_str().unwrap_or_default();
            step["display"] = match d["type"].as_str() {
                Some("qr") => json!({ "type": "qr", "data": data }),
                Some("emoji") => json!({ "type": "emoji", "imageURL": d["image_url"].as_str().unwrap_or_default() }),
                Some("code") => json!({ "type": "code", "data": data }),
                _ => json!({ "type": "nothing" }),
            };
        }
        Some("complete") => {}
        _ => return Value::Null,
    }
    step
}

/// Desktop's `buildLoginSession`: the session of a login process with its current step. A finished login has the
/// status `complete`.
pub(super) fn login_session(id: &str, bridge: &str, login: Value, account: Value, raw: &Value) -> Value {
    let step = login_step(raw);
    let status = match step["type"].as_str() {
        Some("user_input") => "waiting_for_input",
        Some("cookies") => "waiting_for_cookies",
        Some("display_and_wait") => "waiting_for_display",
        Some("complete") => "complete",
        _ => "failed",
    };
    let mut session = json!({ "loginSessionID": id, "bridgeID": bridge });
    for (k, v) in [("loginID", login), ("accountID", account)] {
        if !v.is_null() {
            session[k] = v;
        }
    }
    session["status"] = status.into();
    if !step.is_null() {
        session["currentStep"] = step;
    }
    session
}

/// Desktop's `Chat`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiChat<'a> {
    id: &'a str,
    #[serde(rename = "localChatID", skip_serializing_if = "Option::is_none")]
    local_chat_id: Option<&'a str>,
    #[serde(rename = "accountID")]
    account_id: &'a str,
    network: &'static str,
    title: &'a str,
    description: Option<&'a str>,
    #[serde(rename = "imgURL")]
    img_url: Option<&'a str>,
    #[serde(rename = "type")]
    chat_type: &'static str,
    participants: Participants<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_activity: Option<&'a str>,
    unread_count: u32,
    unread_mentions_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_read_message_sort_key: Option<&'a str>,
    draft: Option<Draft<'a>>,
    reminder: Option<Reminder>,
    snooze: Option<()>,
    is_archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_marked_unread: Option<bool>,
    is_muted: bool,
    is_pinned: bool,
    is_low_priority: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<&'a Label>,
    message_expiry_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge: Option<&'a ChatMergeInfo>,
    #[serde(rename = "mergedIntoChatID", skip_serializing_if = "Option::is_none")]
    merged_into_chat_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<ApiMessage<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Participants<'a> {
    items: Vec<Participant<'a>>,
    has_more: bool,
    total: usize,
}

/// Desktop's `Participant`: a `User` with the membership fields.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Participant<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phone_number: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_name: Option<&'a str>,
    #[serde(rename = "imgURL", skip_serializing_if = "Option::is_none")]
    img_url: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cannot_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_self: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_admin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_network_bot: Option<bool>,
}

#[derive(Serialize)]
struct Draft<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    attachments: BTreeMap<&'a str, DraftAttachment<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftAttachment<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reminder {
    remind_at: String,
    dismiss_on_incoming_message: bool,
}

/// Desktop's `mapChat`: `participants` keeps at most `max_participants` members (all for `None`), and `preview` adds
/// the newest message like `listChats`.
pub(super) fn chat<'a>(
    st: &'a BeeperStore,
    c: &'a Chat,
    max_participants: Option<usize>,
    preview: bool,
) -> ApiChat<'a> {
    let members = &c.participants.items;
    let shown = &members[..max_participants.map_or(members.len(), |n| n.min(members.len()))];
    let participant = |u: &'a User| Participant {
        id: &u.id,
        username: u.username.as_deref(),
        phone_number: u.phone_number.as_deref(),
        email: u.email.as_deref(),
        full_name: u.full_name.as_deref(),
        img_url: u.img_url.as_deref(),
        cannot_message: u.cannot_message,
        is_self: u.is_self,
        is_admin: u.is_admin,
        is_network_bot: u.is_network_bot,
    };
    let draft = c.draft.as_ref().filter(|d| d.text.is_some() || !d.attachments.is_empty()).map(|d| Draft {
        text: d.text.as_deref().unwrap_or_default(),
        attachments: d
            .attachments
            .iter()
            .map(|a| {
                let file = DraftAttachment {
                    id: &a.upload_id,
                    kind: "file",
                    mime_type: a.mime_type.as_deref(),
                    file_name: a.file_name.as_deref(),
                };
                (a.upload_id.as_str(), file)
            })
            .collect(),
    });
    ApiChat {
        id: &c.id,
        local_chat_id: c.local_chat_id.as_deref(),
        account_id: &c.account_id,
        network: account_network(&c.account_id).display_name,
        title: &c.title,
        description: c.description.as_deref(),
        img_url: c.img_url.as_deref(),
        chat_type: if c.chat_type == "group" { "group" } else { "single" },
        participants: Participants {
            items: shown.iter().map(participant).collect(),
            has_more: shown.len() < members.len(),
            total: members.len(),
        },
        last_activity: c.last_activity.as_deref(),
        unread_count: c.unread_count,
        unread_mentions_count: c.unread_mentions,
        last_read_message_sort_key: c.last_read_message_sort_key.as_deref(),
        draft,
        reminder: c.reminder.as_ref().map(|r| Reminder {
            remind_at: rfc3339_from_millis(r.remind_at_ms),
            dismiss_on_incoming_message: r.dismiss_on_incoming_message,
        }),
        snooze: None,
        is_archived: c.flags & CHAT_ARCHIVED != 0,
        is_marked_unread: c.is_marked_unread,
        is_muted: c.flags & CHAT_MUTED != 0,
        is_pinned: c.flags & CHAT_PINNED != 0,
        is_low_priority: c.flags & CHAT_LOW_PRIORITY != 0,
        labels: st.labels.iter().filter(|l| c.label_ids.contains(&l.id)).collect(),
        message_expiry_seconds: c.disappearing.map(|ms| ms / 1000),
        merge: c.merge.as_ref(),
        merged_into_chat_id: c.merged_into_chat_id.as_deref(),
        preview: c.preview.as_ref().filter(|_| preview).map(message),
    }
}

/// Desktop's `Message`: the message types of the API, and no store fields.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiMessage<'a> {
    id: &'a str,
    #[serde(rename = "chatID")]
    chat_id: &'a str,
    #[serde(rename = "accountID")]
    account_id: &'a str,
    #[serde(rename = "senderID")]
    sender_id: &'a str,
    sender_name: &'a str,
    timestamp: &'a str,
    sort_key: &'a str,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    message_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edited_timestamp: Option<&'a str>,
    is_sender: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_deleted: Option<bool>,
    #[serde(rename = "linkedMessageID", skip_serializing_if = "Option::is_none")]
    linked_message_id: Option<&'a str>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    links: &'a [MessageLink],
    reactions: Vec<ApiReaction<'a>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    seen: &'a BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attachments: Option<Vec<ApiAttachment<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_unread: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiReaction<'a> {
    id: &'a str,
    reaction_key: &'a str,
    #[serde(rename = "imgURL", skip_serializing_if = "Option::is_none")]
    img_url: Option<&'a str>,
    #[serde(rename = "participantID")]
    participant_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiAttachment<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_gif: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_sticker: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_voice_note: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    poster_img: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<&'a AttachmentSize>,
    #[serde(rename = "srcURL", skip_serializing_if = "Option::is_none")]
    src_url: Option<&'a str>,
}

/// Desktop's message types (`Qd`). The API omits the other types.
const MESSAGE_TYPES: [&str; 10] =
    ["TEXT", "NOTICE", "IMAGE", "VIDEO", "VOICE", "AUDIO", "FILE", "STICKER", "LOCATION", "REACTION"];

fn attachment(a: &Attachment) -> ApiAttachment<'_> {
    ApiAttachment {
        // Desktop's `rp` takes the ID, else the source URL, else the file name.
        id: a.id.as_deref().or(a.src_url.as_deref()).or(a.file_name.as_deref()).unwrap_or("attachment"),
        kind: &a.attachment_type,
        mime_type: a.mime_type.as_deref(),
        file_name: a.file_name.as_deref(),
        file_size: a.file_size,
        is_gif: a.is_gif,
        is_sticker: a.is_sticker,
        is_voice_note: a.is_voice_note,
        duration: a.duration,
        poster_img: a.poster_img.as_deref(),
        size: a.size.as_ref(),
        src_url: a.src_url.as_deref(),
    }
}

/// Desktop's `sp`. Attachments have no `transcription`: Desktop requires its `engine`, and the store does not keep it.
pub(super) fn message(m: &Message) -> ApiMessage<'_> {
    ApiMessage {
        id: &m.id,
        chat_id: &m.chat_id,
        account_id: &m.account_id,
        sender_id: &m.sender_id,
        sender_name: m.sender_name.as_deref().unwrap_or(&m.sender_id),
        timestamp: &m.timestamp,
        sort_key: &m.sort_key,
        message_type: m.message_type.as_deref().filter(|t| MESSAGE_TYPES.contains(t)),
        text: m.text.as_deref(),
        edited_timestamp: m.edit_history.last().map(|e| e.timestamp.as_str()),
        is_sender: m.is_sender == Some(true),
        is_deleted: m.is_deleted,
        linked_message_id: m.linked_message_id.as_deref(),
        links: &m.links,
        reactions: m
            .reactions
            .iter()
            .map(|r| ApiReaction {
                id: &r.id,
                reaction_key: &r.reaction_key,
                img_url: r.img_url.as_deref(),
                participant_id: &r.participant_id,
            })
            .collect(),
        seen: &m.seen,
        attachments: (!m.attachments.is_empty()).then(|| m.attachments.iter().map(attachment).collect()),
        is_unread: m.is_unread.filter(|_| m.is_sender != Some(true)),
    }
}
