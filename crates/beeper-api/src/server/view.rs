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

/// Whether Desktop's `listBridges` lists the network: not the Beeper bridge (`bridge-hungryserv`) or iMessage, whose
/// bridges (`bridge-imessagecloud` and the macOS platform SDK) it leaves out.
pub(super) fn listed(spec: &NetworkSpec) -> bool {
    !["hungryserv", "imessage"].contains(&spec.bridge_id)
}

/// Desktop's `listBridges` entry of a network. No network allows more than one account here.
pub(super) fn bridge<'a>(st: &'a BeeperStore, spec: &NetworkSpec) -> Bridge<'a> {
    let accounts = network_accounts(st, spec);
    Bridge {
        id: bridge_id(spec),
        bridge_type: bridge_id(spec),
        provider: if spec.supports_cloud { "cloud" } else { "local" },
        display_name: spec.display_name,
        // The `network` of Desktop's bridge configs is the ID of the network, not its name.
        network: if spec.bridge_id == "tumblr" { "tumblrdms" } else { spec.bridge_id },
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
    is_read_only: bool,
    participants: Participants<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_activity: Option<&'a str>,
    unread_count: u32,
    unread_mentions_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_read_message_sort_key: Option<&'a str>,
    draft: Option<Draft<'a>>,
    reminder: Option<Reminder>,
    snooze: Option<Snooze>,
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
    capabilities: Option<Value>,
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

/// Desktop's `snooze`. The store keeps only the end of the snooze, not when the user set it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snooze {
    snooze_until: String,
}

/// A zod check of a value, which gives its output or fails.
type Check<'a> = &'a dyn Fn(&Value) -> Option<Value>;

/// A zod object in its key order: a missing field stays out, a missing required field fails, and a value that fails
/// its check fails the object.
fn shape(fields: &[(&str, Option<&Value>, Check, bool)]) -> Option<Value> {
    let mut out = serde_json::Map::new();
    for &(k, v, check, required) in fields {
        match v {
            Some(v) => drop(out.insert(k.into(), check(v)?)),
            None if required => return None,
            None => {}
        }
    }
    Some(out.into())
}

/// zod's `nativeEnum` of Desktop's `CapabilitySupportLevel`: -2 to 2.
fn level(v: &Value) -> Option<Value> {
    v.as_f64().filter(|n| [-2.0, -1.0, 0.0, 1.0, 2.0].contains(n)).map(|n| (n as i64).into())
}

/// zod's `number().int()`, which JavaScript writes without a fraction.
fn int(v: &Value) -> Option<Value> {
    let n = v.as_f64().filter(|n| n.fract() == 0.0)?;
    Some(match v.as_i64() {
        Some(i) => i.into(),
        None if n.abs() < 9e15 => (n as i64).into(),
        None => v.clone(),
    })
}

fn boolean(v: &Value) -> Option<Value> {
    v.is_boolean().then(|| v.clone())
}

fn array(v: &Value, item: Check) -> Option<Value> {
    v.as_array()?.iter().map(item).collect::<Option<Vec<_>>>().map(Value::from)
}

fn record(v: &Value, item: Check) -> Option<Value> {
    let entries = v.as_object()?.iter().map(|(k, v)| Some((k.clone(), item(v)?)));
    entries.collect::<Option<serde_json::Map<_, _>>>().map(Value::from)
}

/// Desktop's `be`: the value when it is an object.
fn plain(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| v.is_object())
}

/// A field that Desktop's `Cp` builds itself: missing without its source, and failing the capabilities when the
/// built value fails its check.
fn built(v: Option<Option<Value>>) -> Result<Option<Value>, ()> {
    v.map_or(Ok(None), |v| v.map(Some).ok_or(()))
}

/// Desktop's `kc`: the capabilities of one attachment message type.
fn file_kind(c: &Value) -> Option<Value> {
    let kind = |t: &Value| t.as_str().filter(|t| ["time", "count"].contains(t)).map(Value::from);
    let limit = |t: &Value| {
        let t = plain(Some(t))?;
        shape(&[
            ("type", t.get("type"), &kind, true),
            ("count", t.get("count"), &int, false),
            ("time", t.get("time"), &int, false),
        ])
    };
    shape(&[
        ("mimeTypes", c.get("mime_types"), &|v| record(v, &level), true),
        ("caption", c.get("caption"), &level, false),
        ("maxCaptionLength", c.get("max_caption_length"), &int, false),
        ("maxSize", c.get("max_size"), &int, false),
        ("maxWidth", c.get("max_width"), &int, false),
        ("maxHeight", c.get("max_height"), &int, false),
        ("maxDuration", c.get("max_duration"), &int, false),
        ("viewLimitedTypes", c.get("view_limited_types"), &|v| array(v, &limit), false),
    ])
}

/// Desktop's `Cp` and the zod `ChatCapabilities` that checks its result: the capabilities of a chat from the content
/// of its `com.beeper.room_features`, or `None` when a value does not fit the schema, which Desktop drops.
pub(super) fn capabilities(f: &Value) -> Option<Value> {
    let g = |k: &str| f.get(k);
    let attachments = plain(g("file")).map(|files| {
        let kinds = files.as_object().expect("an object").iter().filter(|(_, c)| c.is_object());
        kinds.map(|(k, c)| Some((k.clone(), file_kind(c)?))).collect::<Option<serde_json::Map<_, _>>>().map(Value::from)
    });
    let request = plain(g("message_request")).map(|r| {
        shape(&[
            ("acceptWithMessage", r.get("accept_with_message"), &level, false),
            ("acceptWithButton", r.get("accept_with_button"), &level, false),
        ])
    });
    let timer = plain(g("disappearing_timer")).map(|t| {
        let kind = |k: &Value| {
            let k = match k.as_str()? {
                "after_read" => "afterRead",
                "after_read_by_recipient" => "afterReadByRecipient",
                "after_send" => "afterSend",
                k => k,
            };
            ["afterRead", "afterReadByRecipient", "afterSend"].contains(&k).then(|| k.into())
        };
        // `types` that is not an array stays out.
        let types = t.get("types").filter(|v| v.is_array());
        shape(&[
            ("types", types, &|v| array(v, &kind), false),
            ("timers", t.get("timers"), &|v| array(v, &int), false),
            ("omitEmptyTimer", t.get("omit_empty_timer"), &boolean, false),
        ])
    });
    let members = plain(g("member_actions")).map(|m| {
        let fields = [
            ("ban", "ban"),
            ("invite", "invite"),
            ("kick", "kick"),
            ("leave", "leave"),
            ("revokeInvite", "revoke_invite"),
        ];
        shape(&fields.map(|(k, from)| (k, m.get(from), &level as Check, false)))
    });
    let state = plain(g("state")).map(|s| {
        let one = |v: &Value| shape(&[("level", plain(Some(v))?.get("level"), &level, true)]);
        let fields = [
            ("disappearingTimer", "com.beeper.disappearing_timer"),
            ("avatar", "m.room.avatar"),
            ("title", "m.room.name"),
            ("description", "m.room.topic"),
        ];
        shape(&fields.map(|(k, from)| (k, s.get(from), &one as Check, false)))
    });
    let [attachments, request, timer, members, state] =
        [attachments, request, timer, members, state].map(built).map(Result::ok);
    let id = |v: &Value| Some(v.clone());
    let strings = |v: &Value| array(v, &|s| s.is_string().then(|| s.clone()));
    let fields: [(&str, Option<&Value>, Check); 27] = [
        ("formatting", g("formatting"), &|v| record(v, &level)),
        ("attachments", attachments.as_ref()?.as_ref(), &id),
        ("maxTextLength", g("max_text_length"), &int),
        ("locationMessage", g("location_message"), &level),
        ("poll", g("poll"), &level),
        ("thread", g("thread"), &level),
        ("reply", g("reply"), &level),
        ("edit", g("edit"), &level),
        ("editMaxCount", g("edit_max_count"), &int),
        ("editMaxAge", g("edit_max_age"), &int),
        ("delete", g("delete"), &level),
        ("deleteMaxAge", g("delete_max_age"), &int),
        ("deleteForMe", g("delete_for_me"), &boolean),
        ("messageRequest", request.as_ref()?.as_ref(), &id),
        ("reaction", g("reaction"), &level),
        ("reactionCount", g("reaction_count"), &int),
        ("allowedReactions", g("allowed_reactions"), &strings),
        ("customEmojiReactions", g("custom_emoji_reactions"), &boolean),
        ("disappearingTimer", timer.as_ref()?.as_ref(), &id),
        ("readReceipts", g("read_receipts"), &boolean),
        ("typingNotifications", g("typing_notifications"), &boolean),
        ("archive", g("archive"), &boolean),
        ("markAsUnread", g("mark_as_unread"), &boolean),
        ("deleteChat", g("delete_chat"), &boolean),
        ("deleteChatForEveryone", g("delete_chat_for_everyone"), &boolean),
        ("participantActions", members.as_ref()?.as_ref(), &id),
        ("state", state.as_ref()?.as_ref(), &id),
    ];
    shape(&fields.map(|(k, v, check)| (k, v, check, false)))
}

/// The `type` of a chat in the API: `group` or `single`.
pub(super) fn chat_type(c: &Chat) -> &'static str {
    if c.chat_type == "group" {
        "group"
    } else {
        "single"
    }
}

/// Desktop's `mapChat`: `participants` keeps at most `max_participants` members (all for `None`), and `preview` adds
/// the newest message like `listChats`.
pub(super) fn chat<'a>(
    st: &'a BeeperStore, c: &'a Chat, max_participants: Option<usize>, preview: bool,
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
        chat_type: chat_type(c),
        is_read_only: c.is_read_only,
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
        snooze: c.snoozed_until_ms.map(|ms| Snooze { snooze_until: rfc3339_from_millis(ms) }),
        is_archived: c.flags & CHAT_ARCHIVED != 0,
        is_marked_unread: c.is_marked_unread,
        is_muted: c.flags & CHAT_MUTED != 0,
        is_pinned: c.flags & CHAT_PINNED != 0,
        is_low_priority: c.flags & CHAT_LOW_PRIORITY != 0,
        labels: st.labels.iter().filter(|l| c.label_ids.contains(&l.id)).collect(),
        message_expiry_seconds: c.disappearing.map(|ms| ms / 1000),
        capabilities: c.features.as_deref().and_then(capabilities),
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
    send_status: Option<ApiSendStatus<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_deleted: Option<bool>,
    #[serde(rename = "linkedMessageID", skip_serializing_if = "Option::is_none")]
    linked_message_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mentions: Option<&'a [String]>,
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
    #[serde(rename = "participantID")]
    participant_id: &'a str,
    /// Desktop's `emoji`: the key is an emoji, not the `mxc://` URL of a custom image.
    emoji: bool,
    reaction_key: &'a str,
    #[serde(rename = "imgURL", skip_serializing_if = "Option::is_none")]
    img_url: Option<&'a str>,
}

/// Desktop's `ep`: the `sendStatus` of a message, with its time.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiSendStatus<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    status: &'a str,
    timestamp: String,
    delivered_to_users: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    internal_error: Option<&'a str>,
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
    transcription: Option<&'a Transcription>,
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
        // Desktop's `rp` takes the media URL without its encryption info, else the source URL, else the file name.
        // The store's own attachment IDs are not media URLs, so the API does not show them.
        id: a
            .src_url
            .as_deref()
            .map(|u| if u.starts_with("mxc://") { u.split('?').next().expect("a first item") } else { u })
            .or(a.file_name.as_deref())
            .unwrap_or("attachment"),
        kind: &a.attachment_type,
        mime_type: a.mime_type.as_deref(),
        file_name: a.file_name.as_deref(),
        file_size: a.file_size,
        is_gif: a.is_gif,
        is_sticker: a.is_sticker,
        is_voice_note: a.is_voice_note,
        duration: a.duration,
        transcription: a.transcription.as_ref(),
        poster_img: a.poster_img.as_deref(),
        size: a.size.as_ref(),
        src_url: a.src_url.as_deref(),
    }
}

/// Desktop's `sp`.
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
        // Desktop's `ep` gives no status without a time.
        send_status: m.send_status.as_ref().and_then(|s| {
            Some(ApiSendStatus {
                reason: s.reason.as_deref(),
                message: s.message.as_deref(),
                status: &s.status,
                timestamp: rfc3339_from_millis(s.ts?),
                delivered_to_users: &s.delivered_to_users,
                internal_error: s.internal_error.as_deref(),
            })
        }),
        is_hidden: (m.message_type.as_deref() == Some("HIDDEN")).then_some(true),
        is_deleted: m.is_deleted,
        linked_message_id: m.linked_message_id.as_deref(),
        mentions: m.mentions.as_deref(),
        links: &m.links,
        reactions: m
            .reactions
            .iter()
            .map(|r| ApiReaction {
                id: &r.id,
                participant_id: &r.participant_id,
                emoji: !r.reaction_key.starts_with("mxc://") && r.img_url.is_none(),
                reaction_key: &r.reaction_key,
                img_url: r.img_url.as_deref(),
            })
            .collect(),
        seen: &m.seen,
        attachments: (!m.attachments.is_empty()).then(|| m.attachments.iter().map(attachment).collect()),
        is_unread: m.is_unread.filter(|_| m.is_sender != Some(true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's `Cp` renames the bridge's keys into the order of the zod schema, keeps only the object file kinds,
    /// maps the timer types, and its `safeParse` drops all the capabilities for one value that does not fit.
    #[test]
    fn room_features_become_desktop_capabilities() {
        let features = json!({
            "read_receipts": true,
            "formatting": { "bold": 2, "italic": 1 },
            "file": {
                "m.image": { "mime_types": { "image/png": 2, "image/*": 1 }, "caption": 2, "max_size": 104857600.0 },
                "m.video": null,
                "fi.mau.gif": { "mime_types": { "image/gif": 2 }, "view_limited_types": [{ "type": "count", "count": 1 }] }
            },
            "max_text_length": 65536,
            "reply": 2, "edit": 2, "edit_max_age": 900, "delete": 2, "delete_max_age": 172800, "delete_for_me": false,
            "reaction": 2, "reaction_count": 1, "allowed_reactions": ["👍"],
            "disappearing_timer": { "types": ["after_read", "after_send"], "timers": [86400000] },
            "member_actions": { "leave": 2, "revoke_invite": -1 },
            "state": { "m.room.name": { "level": 2 }, "m.room.topic": { "level": 0, "x": 1 } },
            "message_request": { "accept_with_button": 2 },
            "unknown_feature": 5
        });
        let expected = json!({
            "formatting": { "bold": 2, "italic": 1 },
            "attachments": {
                "m.image": { "mimeTypes": { "image/png": 2, "image/*": 1 }, "caption": 2, "maxSize": 104857600 },
                "fi.mau.gif": { "mimeTypes": { "image/gif": 2 }, "viewLimitedTypes": [{ "type": "count", "count": 1 }] }
            },
            "maxTextLength": 65536,
            "reply": 2, "edit": 2, "editMaxAge": 900, "delete": 2, "deleteMaxAge": 172800, "deleteForMe": false,
            "messageRequest": { "acceptWithButton": 2 },
            "reaction": 2, "reactionCount": 1, "allowedReactions": ["👍"],
            "disappearingTimer": { "types": ["afterRead", "afterSend"], "timers": [86400000] },
            "readReceipts": true,
            "participantActions": { "leave": 2, "revokeInvite": -1 },
            "state": { "title": { "level": 2 }, "description": { "level": 0 } }
        });
        let found = capabilities(&features).unwrap();
        assert_eq!(found.to_string(), expected.to_string(), "the values and the key order of Desktop");
        assert_eq!(capabilities(&json!({})), Some(json!({})));
        let timer = capabilities(&json!({ "disappearing_timer": { "types": "after_read" } }));
        assert_eq!(timer, Some(json!({ "disappearingTimer": {} })), "`types` that is not an array stays out");
        for bad in [
            json!({ "max_text_length": 1.5 }),
            json!({ "reply": 3 }),
            json!({ "reply": null }),
            json!({ "file": { "m.image": { "caption": 2 } } }),
            json!({ "state": { "m.room.name": {} } }),
            json!({ "disappearing_timer": { "types": ["after_reboot"] } }),
            json!({ "allowed_reactions": [1] }),
        ] {
            assert_eq!(capabilities(&bad), None, "{bad}");
        }
    }

    #[test]
    fn chats_carry_their_capabilities_and_snooze() {
        let store = beeper_core::testing::demo_store("view-capabilities");
        let mut st = store.write();
        let idx = st.resolve_chat_idx("!wa_eric:beeper.com").unwrap();
        let plain = serde_json::to_value(chat(&st, &st.chats[idx], None, false)).unwrap();
        assert!(plain.get("capabilities").is_none() && plain["snooze"].is_null(), "a room without features");
        st.update_chat(idx, |c| {
            c.features = Some(Box::new(json!({ "reply": 2 })));
            c.snoozed_until_ms = Some(1_900_000_000_000);
        });
        let shown = serde_json::to_value(chat(&st, &st.chats[idx], None, false)).unwrap();
        assert_eq!(shown["capabilities"], json!({ "reply": 2 }));
        assert_eq!(shown["snooze"], json!({ "snoozeUntil": "2030-03-17T17:46:40.000Z" }));
        let keys: Vec<&String> = shown.as_object().unwrap().keys().collect();
        let at = |k: &str| keys.iter().position(|x| *x == k).unwrap();
        assert!(at("messageExpirySeconds") < at("capabilities"), "Desktop's key order");
    }
}
