//! The `messages`, `send`, and `presence` commands.

use super::chats::{chat_flag, resolve_accounts, resolve_chat, upload, CHAT, PICK};
use super::commands::cmd;
use super::render::Empty;
use super::{js_string, obj, pretty, truthy, CliError, Cmd, Ctx, Flag, Res};
use beeper_api::client::{chat_path, message_path};
use beeper_api::http::query_string;
use beeper_core::codec::url_encode;
use serde_json::{json, Value};

const ID: &str = "id";
const BEFORE_CURSOR: Flag = Flag::s("before-cursor", "Paginate messages older than this message ID");
const AFTER_CURSOR: Flag = Flag::s("after-cursor", "Paginate messages newer than this message ID");
const ASC: Flag = Flag::b("asc", "Order oldest first (default: newest first)");
const TO: Flag = Flag::s("to", "Chat selector (ID, local ID, title, or search text)").req();
const REPLY_TO: Flag = Flag::s("reply-to", "Send as a reply to this message ID");
const WAIT: Flag = Flag::b("wait", "Wait for the message to leave the pending state (or fail) before returning");
const WAIT_TIMEOUT: Flag = Flag::i("wait-timeout", "Maximum wait time in ms when --wait is set").def("30000");
const FILENAME: Flag = Flag::s("filename", "Override the displayed filename");
const TRANSACTION: Flag = Flag::s("transaction", "Optional transaction ID for deduplication");
const SEND_DESCRIPTION: &str =
    "Returns when Desktop accepts the send request. Pass `--wait` to wait until the message leaves the pending state or fails.";

pub const COMMANDS: &[Cmd] = &[
    Cmd {
        flags: &[
            CHAT,
            Flag::s(ID, "Target message ID to center the window on").req(),
            Flag::i("before", "Number of messages to include before the target").def("10"),
            Flag::i("after", "Number of messages to include after the target").def("10"),
            PICK,
        ],
        ..cmd("messages context", "Show message context", context)
    },
    Cmd {
        flags: &[
            CHAT,
            Flag::s(ID, "Message ID to delete (final message ID; pending IDs are rejected)").req(),
            PICK,
            Flag::b("for-everyone", "Delete for everyone when the network supports it (otherwise deletes only for you)"),
        ],
        ..cmd("messages delete", "Delete a message", delete)
    },
    Cmd {
        flags: &[
            CHAT,
            Flag::s(ID, "Message ID to edit (must be one of your own messages with no attachments)").req(),
            PICK,
            Flag::s("message", "New message text").req(),
        ],
        ..cmd("messages edit", "Edit a message", edit)
    },
    Cmd {
        description:
            "Lightweight per-chat JSON export. For a full export with transcripts, attachments, and multiple chats, use `beeper export`.",
        flags: &[
            CHAT,
            PICK,
            BEFORE_CURSOR,
            AFTER_CURSOR,
            Flag::s("after", "Only messages at or after this ISO timestamp (client-side filter)"),
            Flag::s("before", "Only messages at or before this ISO timestamp (client-side filter)"),
            Flag::i("limit", "Maximum messages to export"),
            Flag::s("output", "Output path; - writes JSON to stdout").c('o').def("-"),
            ASC,
        ],
        ..cmd("messages export", "Export one chat to JSON", export)
    },
    Cmd {
        flags: &[
            CHAT,
            BEFORE_CURSOR,
            AFTER_CURSOR,
            Flag::s("sender", "Filter by sender: me, others, or a specific user ID (client-side)"),
            ASC,
            Flag::b("ids", "Print only message IDs"),
            Flag::i("limit", "Maximum messages to print").def("50"),
            PICK,
        ],
        ..cmd("messages list", "List chat messages", list)
    },
    Cmd {
        args: &[super::Arg { required: false, ..super::arg("query", "Search text (literal word match)") }],
        flags: &[
            Flag::many("account", "Limit to an account selector. Repeat for multiple."),
            Flag::many("chat", "Limit to a chat selector. Repeat for multiple."),
            Flag::s("chat-type", "Only group chats or direct messages").opts(&["group", "single"]),
            Flag::s("after", "Only messages at or after this ISO timestamp"),
            Flag::s("before", "Only messages at or before this ISO timestamp"),
            Flag::no("exclude-low-priority", "Exclude low-priority chats"),
            Flag::b("ids", "Print only message IDs"),
            Flag::no("include-muted", "Include muted chats").def("true"),
            Flag::i("limit", "Maximum results").def("50"),
            Flag::many("media", "Filter by media type. Repeat for multiple.").opts(&["any", "video", "image", "link", "file"]),
            Flag::s("sender", "me, others, or a user ID"),
        ],
        examples: &[
            "beeper messages search \"quarterly report\"",
            "beeper messages search --chat \"Work\" --sender me --limit 20",
            "beeper messages search --media image --after 2024-01-01T00:00:00Z",
            "beeper messages search --chat-type group --sender others \"meeting\"",
        ],
        ..cmd("messages search", "Search messages across chats", search)
    },
    Cmd {
        flags: &[CHAT, Flag::s(ID, "Message ID, pendingMessageID, or Matrix event ID").req(), PICK],
        ..cmd("messages show", "Show one message", show)
    },
    Cmd {
        args: &[super::arg("url", "mxc:// or localmxc:// URL")],
        flags: &[Flag::s("out", "Output directory; pass - to stream the file to stdout").c('o').def(".")],
        ..cmd("media download", "Download message media", media_download)
    },
    Cmd {
        description: "Requires server-side support. Networks without typing notifications return an error.",
        flags: &[
            CHAT,
            PICK,
            Flag::s("state", "Indicator to send").def("typing").opts(&["typing", "paused"]),
            Flag::i("duration", "When --state is typing, send paused automatically after this many seconds"),
        ],
        ..cmd("presence", "Send a typing (or paused) indicator to a chat", presence)
    },
    Cmd {
        description: SEND_DESCRIPTION,
        flags: &[
            TO,
            Flag::s("file", "Local file path to upload (max 500 MB)").req(),
            Flag::s("caption", "Optional caption to send alongside the file"),
            FILENAME,
            Flag::s("mime", "Override MIME type detection"),
            PICK,
            REPLY_TO,
            WAIT,
            WAIT_TIMEOUT,
        ],
        examples: &[
            "beeper send file --to 10313 --file ./photo.jpg --caption \"Look at this\"",
            "beeper send file --to alice@whatsapp --file ./report.pdf",
            "beeper send file --to 8951 --file ./clip.mp4 --reply-to <msgID>",
        ],
        ..cmd("send file", "Send a file", send)
    },
    Cmd {
        flags: &[
            TO,
            Flag::s(ID, "Message ID to react to").req(),
            Flag::s("reaction", "Reaction key (emoji, shortcode, or custom emoji key)").req(),
            PICK,
            TRANSACTION,
        ],
        ..cmd("send react", "Send a reaction to a message", react)
    },
    Cmd {
        description: "Uploads the file and sends as a sticker message. Defaults --mime to image/webp.",
        flags: &[
            TO,
            Flag::s("file", "Sticker file (typically 512x512 WebP)").req(),
            FILENAME,
            Flag::s("mime", "MIME type for the sticker (default: image/webp)").def("image/webp"),
            PICK,
            REPLY_TO,
            WAIT,
            WAIT_TIMEOUT,
        ],
        ..cmd("send sticker", "Send a sticker", send)
    },
    Cmd {
        description: SEND_DESCRIPTION,
        flags: &[
            TO,
            Flag::s("message", "Message text to send").req(),
            PICK,
            REPLY_TO,
            Flag::many("mention", "User ID to @-mention (repeatable)"),
            Flag::b("no-preview", "Disable automatic link preview for URLs in the message"),
            WAIT,
            WAIT_TIMEOUT,
        ],
        examples: &[
            "beeper send text --to 10313 --message \"On my way\"",
            "beeper send text --to 8951 --message \"See @alice\" --mention alice@whatsapp",
            "beeper send text --to alice@whatsapp --message \"Got it\" --reply-to <msgID>",
        ],
        ..cmd("send text", "Send a text message", send)
    },
    Cmd {
        flags: &[
            TO,
            Flag::s(ID, "Message ID whose reaction to remove").req(),
            Flag::s("reaction", "Reaction key to remove (emoji, shortcode, or custom emoji key)").req(),
            PICK,
            TRANSACTION,
        ],
        ..cmd("send unreact", "Remove a reaction from a message", react)
    },
    Cmd {
        description: "Uploads the audio file and sends as a voice note. Defaults --mime to audio/ogg.",
        flags: &[
            TO,
            Flag::s("file", "Voice note audio file (OGG/Opus recommended)").req(),
            Flag::i("duration", "Voice note duration in seconds (overrides upload-detected duration)"),
            FILENAME,
            Flag::s("mime", "MIME type for the voice note (default: audio/ogg)").def("audio/ogg"),
            PICK,
            REPLY_TO,
            WAIT,
            WAIT_TIMEOUT,
        ],
        ..cmd("send voice", "Send a voice note", send)
    },
];

fn limit(c: &Ctx, flag: &str) -> usize {
    c.int(flag).map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(0))
}

/// `matchesSender`: `me`, `others`, or a sender ID.
fn sender_is(m: &Value, sender: &str) -> bool {
    match sender {
        "me" => m["isSender"] == true,
        "others" => m["isSender"] != true,
        id => m["senderID"] == id,
    }
}

/// The cursor query of `--before-cursor` or `--after-cursor`.
fn cursor_query(c: &Ctx) -> Res<Vec<(&'static str, String)>> {
    match (c.str("before-cursor"), c.str("after-cursor")) {
        (Some(_), Some(_)) => Err(CliError::bug("Use only one of --before-cursor or --after-cursor")),
        (Some(cur), None) => Ok(vec![("cursor", cur.into()), ("direction", "before".into())]),
        (None, Some(cur)) => Ok(vec![("cursor", cur.into()), ("direction", "after".into())]),
        (None, None) => Ok(Vec::new()),
    }
}

fn list(c: &mut Ctx) -> Res {
    let chat = chat_flag(c, "chat")?;
    let query = cursor_query(c)?;
    let sender = c.str("sender").map(str::to_string);
    let keep = |m: &Value| sender.as_deref().is_none_or(|s| sender_is(m, s));
    let mut items = c.collect(&chat_path(&chat, "/messages"), &query, limit(c, "limit"), keep)?;
    if c.on("asc") {
        items.reverse();
    }
    match c.on("ids") {
        true => c.print_ids(&items),
        false => c.print_list(&items, Empty::new("No messages yet", "This chat is empty.")),
    }
}

fn search(c: &mut Ctx) -> Res {
    let q = c.arg(0).map(str::to_string);
    let filtered = !c.many("account").is_empty()
        || !c.many("chat").is_empty()
        || ["chat-type", "after", "before", "sender"].iter().any(|f| c.str(f).is_some())
        || !c.many("media").is_empty();
    if q.is_none() && !filtered {
        let msg = "Provide a search query or at least one filter flag (--chat, --sender, --media, etc.).";
        return Err(CliError::abort(msg, 2));
    }
    let inputs = c.many("account").to_vec();
    let accounts = resolve_accounts(c, &inputs, true, true)?;
    let mut chats = Vec::new();
    for input in c.many("chat").to_vec() {
        chats.push(resolve_chat(c, &input, None, &accounts)?);
    }
    let bools = |v: Option<bool>| v.map(|b| b.to_string());
    let mut query: Vec<(&str, String)> = accounts.into_iter().map(|a| ("accountIDs", a)).collect();
    query.extend(chats.into_iter().map(|ch| ("chatIDs", ch)));
    let one = [
        ("chatType", c.str("chat-type").map(str::to_string)),
        ("dateAfter", c.str("after").map(str::to_string)),
        ("dateBefore", c.str("before").map(str::to_string)),
        ("excludeLowPriority", bools(c.opt_bool("exclude-low-priority"))),
        ("includeMuted", bools(c.opt_bool("include-muted"))),
    ];
    query.extend(one.into_iter().filter_map(|(k, v)| Some((k, v?))));
    query.extend(c.many("media").iter().map(|m| ("mediaTypes", m.clone())));
    query.extend([("query", q.clone()), ("sender", c.str("sender").map(str::to_string))].into_iter().filter_map(|(k, v)| Some((k, v?))));
    let n = limit(c, "limit");
    let load = |c: &mut Ctx| c.collect("/v1/messages/search", &query, n, |_| true);
    let items = match c.json() || c.on("ids") {
        true => load(c)?,
        false => {
            let label = q.as_ref().map_or("Searching messages…".into(), |q| format!("Searching messages for \"{q}\"…"));
            let done = |v: &Vec<Value>| format!("{} match{}", v.len(), if v.len() == 1 { "" } else { "es" });
            c.spinner(&label, load, done)?
        }
    };
    if c.on("ids") {
        return c.print_ids(&items);
    }
    let subtitle = q.map_or("Try a different filter combination.".into(), |q| format!("Nothing found for \"{q}\"."));
    let empty = Empty::new("No messages matched", subtitle)
        .try_("messages list --chat <chat>", "list messages from a chat")
        .try_("chats search \"<term>\"", "search chats instead");
    c.print_list(&items, empty)
}

fn show(c: &mut Ctx) -> Res {
    let chat = chat_flag(c, "chat")?;
    let id = c.str(ID).expect("--id is required").to_string();
    let message = c.get(&message_path(&chat, &id, ""))?;
    c.print(&message)
}

fn context(c: &mut Ctx) -> Res {
    let chat = chat_flag(c, "chat")?;
    let id = c.str(ID).expect("--id is required").to_string();
    let path = chat_path(&chat, "/messages");
    let around = |c: &mut Ctx, direction: &str| {
        let query = [("cursor", id.clone()), ("direction", direction.to_string())];
        c.collect(&path, &query, limit(c, direction), |_| true)
    };
    let before = around(c, "before")?;
    let after = around(c, "after")?;
    c.print(&json!({ "chatID": chat, "messageID": id, "before": before, "after": after }))
}

fn edit(c: &mut Ctx) -> Res {
    c.writable()?;
    let chat = chat_flag(c, "chat")?;
    let id = c.str(ID).expect("--id is required").to_string();
    let text = c.str("message").expect("--message is required").to_string();
    let reply = c.call("PUT", &message_path(&chat, &id, ""), Some(json!({ "text": text })))?;
    c.print(&reply)
}

fn delete(c: &mut Ctx) -> Res {
    c.writable()?;
    let chat = chat_flag(c, "chat")?;
    let id = c.str(ID).expect("--id is required").to_string();
    let everyone = c.on("for-everyone");
    let query: &[(&str, &str)] = if everyone { &[("forEveryone", "true")] } else { &[] };
    c.call("DELETE", &query_string(&message_path(&chat, &id, ""), query), None)?;
    let message = if everyone { "Deleted for everyone" } else { "Deleted" };
    c.success(message, None, json!({ "chatID": chat, "messageID": id, "forEveryone": everyone }))
}

/// `Date.parse` of an ISO timestamp, in milliseconds.
fn parse_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis()).or_else(|| {
        let day = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
        Some(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
    })
}

fn export(c: &mut Ctx) -> Res {
    let query = cursor_query(c)?;
    let chat = chat_flag(c, "chat")?;
    let bound = |flag: &str| -> Res<Option<i64>> {
        c.str(flag).map(|s| parse_ms(s).ok_or_else(|| CliError::bug(format!("--{flag} is not a valid ISO timestamp: {s}")))).transpose()
    };
    let (after, before) = (bound("after")?, bound("before")?);
    let keep = |m: &Value| {
        let ts = m["timestamp"].as_str().and_then(parse_ms);
        ts.is_none_or(|ts| after.is_none_or(|a| ts >= a) && before.is_none_or(|b| ts <= b))
    };
    let mut messages = c.collect(&chat_path(&chat, "/messages"), &query, limit(c, "limit"), keep)?;
    if c.on("asc") {
        messages.reverse();
    }
    let exported = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let all = obj([
        ("exportedAt", json!(exported)),
        ("chatID", json!(chat)),
        ("after", json!(c.str("after"))),
        ("before", json!(c.str("before"))),
        ("count", json!(messages.len())),
        ("messages", json!(messages)),
    ]);
    let text = format!("{}\n", pretty(&Value::Object(all)));
    match c.str("output").expect("--output has a default") {
        "-" => print!("{text}"),
        path => std::fs::write(path, text).map_err(|e| CliError::bug(format!("{e}, open '{path}'")))?,
    }
    Ok(())
}

/// `waitForMessage`: polls the pending message every 750 ms until it resolves or the time is out.
fn wait_for(c: &mut Ctx, chat: &str, pending: &str, timeout_ms: i64) -> Res<Value> {
    let started = std::time::Instant::now();
    let mut last = None;
    while (started.elapsed().as_millis() as i64) < timeout_ms {
        match c.get(&message_path(chat, pending, "")) {
            Ok(m) => return Ok(m),
            Err(e) => last = Some(e.msg),
        }
        std::thread::sleep(std::time::Duration::from_millis(750));
    }
    Err(CliError::bug(format!("Timed out waiting for {pending}{}", last.map_or(String::new(), |e| format!(": {e}")))))
}

/// `send text|file|sticker|voice` (`lib/send-message.js`): uploads the file, sends, and maybe waits.
fn send(c: &mut Ctx) -> Res {
    c.writable()?;
    let chat = chat_flag(c, "to")?;
    let kind = match c.cmd.id {
        "send sticker" => Some("sticker"),
        "send voice" => Some("voice-note"),
        _ => None,
    };
    let text = match c.cmd.id {
        "send text" => c.str("message").expect("--message is required").to_string(),
        "send file" => c.str("caption").unwrap_or_default().to_string(),
        _ => String::new(),
    };
    let attachment = match c.cmd.id {
        "send text" => Value::Null,
        _ => {
            let (file, name) = (c.str("file").expect("--file is required").to_string(), c.str("filename").map(str::to_string));
            let mime = c.str("mime").map(str::to_string);
            let up = upload(c, &file, name.as_deref(), mime.as_deref())?;
            let id = up["uploadID"].as_str().filter(|u| !u.is_empty()).ok_or_else(|| CliError::bug("Upload did not return an uploadID"))?;
            let duration = if c.cmd.id == "send voice" { c.int("duration").map(Value::from) } else { None };
            let size = (truthy(&up["width"]) && truthy(&up["height"])).then(|| json!({ "height": up["height"], "width": up["width"] }));
            Value::Object(obj([
                ("uploadID", json!(id)),
                ("type", json!(kind)),
                ("duration", duration.unwrap_or_else(|| up["duration"].clone())),
                ("fileName", up["fileName"].clone()),
                ("mimeType", mime.map_or_else(|| up["mimeType"].clone(), Value::from)),
                ("size", size.unwrap_or_default()),
            ]))
        }
    };
    let mentions = if c.cmd.id == "send text" { c.many("mention").to_vec() } else { Vec::new() };
    let no_preview = c.cmd.id == "send text" && c.on("no-preview");
    let body = obj([
        ("attachment", attachment),
        ("replyToMessageID", json!(c.str("reply-to"))),
        ("text", json!(text)),
        ("mentions", if mentions.is_empty() { Value::Null } else { json!(mentions) }),
        ("disableLinkPreview", if no_preview { json!(true) } else { Value::Null }),
    ]);
    let pending = c.call("POST", &chat_path(&chat, "/messages"), Some(Value::Object(body)))?;
    if !c.on("wait") {
        let mut result = pending.as_object().cloned().unwrap_or_default();
        result.extend([
            ("accepted".to_string(), json!(true)),
            ("state".to_string(), json!("accepted")),
            ("chatID".to_string(), json!(chat)),
            ("hint".to_string(), json!("Desktop accepted the send request. Pass --wait to wait for the final message or failure.")),
        ]);
        return c.print(&Value::Object(result));
    }
    let pending_id = js_string(&pending["pendingMessageID"]);
    let message = wait_for(c, &chat, &pending_id, c.int("wait-timeout").expect("--wait-timeout has a default"))?;
    c.print(&json!({ "accepted": true, "state": "resolved", "chatID": chat, "pendingMessageID": pending_id, "message": message }))
}

/// `send react` and `send unreact`.
fn react(c: &mut Ctx) -> Res {
    c.writable()?;
    let chat = chat_flag(c, "to")?;
    let id = c.str(ID).expect("--id is required").to_string();
    let key = c.str("reaction").expect("--reaction is required").to_string();
    let reply = match c.cmd.id {
        "send react" => {
            let body = obj([("reactionKey", json!(key)), ("transactionID", json!(c.str("transaction")))]);
            c.call("POST", &message_path(&chat, &id, "/reactions"), Some(Value::Object(body)))?
        }
        _ => c.call("DELETE", &message_path(&chat, &id, &format!("/reactions/{}", url_encode(&key))), None)?,
    };
    c.print(&reply)
}

fn presence(c: &mut Ctx) -> Res {
    c.writable()?;
    let (state, duration) = (c.str("state").expect("--state has a default").to_string(), c.int("duration"));
    if duration.is_some_and(|d| d <= 0) {
        return Err(CliError::bug("--duration must be a positive integer (seconds)"));
    }
    if duration.is_some() && state != "typing" {
        return Err(CliError::bug("--duration only applies when --state is typing"));
    }
    let chat = chat_flag(c, "chat")?;
    let path = chat_path(&chat, "/typing");
    c.call("POST", &path, Some(json!({ "state": state })))?;
    let Some(seconds) = duration else {
        return c.success(&format!("Sent {state} indicator"), None, json!({ "chatID": chat, "state": state }));
    };
    std::thread::sleep(std::time::Duration::from_secs(seconds as u64));
    c.call("POST", &path, Some(json!({ "state": "paused" })))?;
    let data = json!({ "chatID": chat, "state": "paused", "durationSeconds": seconds });
    c.success(&format!("Sent typing then paused after {seconds}s"), None, data)
}

/// `media download`: the file of an `mxc://` URL into a folder, or onto stdout.
fn media_download(c: &mut Ctx) -> Res {
    let url = c.arg(0).expect("a required arg").to_string();
    let client = c.client()?.clone();
    let bytes = c.block(client.call_raw("GET", &query_string("/v1/assets/serve", &[("url", &url)]), None))?;
    let out = c.str("out").expect("--out has a default").to_string();
    if out == "-" {
        use std::io::Write;
        return std::io::stdout().write_all(&bytes).map_err(|e| CliError::bug(e.to_string()));
    }
    c.writable()?;
    let after_scheme = url.split_once("://").map(|p| p.1).ok_or_else(|| CliError::bug(format!("\"{url}\" cannot be parsed as a URL.")))?;
    let path = after_scheme.split(['?', '#']).next().unwrap_or_default();
    let name = path.split_once('/').map(|p| p.1.trim_end_matches('/').rsplit('/').next().unwrap_or_default()).unwrap_or_default();
    let name = if name.is_empty() { "media" } else { name };
    let file = if out == "." { name.to_string() } else { format!("{}/{name}", out.trim_end_matches('/')) };
    std::fs::create_dir_all(&out).map_err(|e| CliError::system(format!("{e}, mkdir '{out}'")))?;
    std::fs::write(&file, &bytes).map_err(|e| CliError::system(format!("{e}, open '{file}'")))?;
    c.success("Downloaded media", Some(&file), json!({ "path": file, "bytes": bytes.len() }))
}
