//! The API shapes of Desktop's `mapChat`, in its key order, made from the stored chats.

use beeper_core::config::{account_network, rfc3339_from_millis};
use beeper_core::models::*;
use beeper_core::store::BeeperStore;
use serde::Serialize;
use std::collections::BTreeMap;

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
    preview: Option<&'a Message>,
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
        preview: c.preview.as_ref().filter(|_| preview),
    }
}
