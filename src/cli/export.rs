//! `export` (`lib/export/index.js`): a resumable export of accounts, chats, messages, transcripts, and attachments.

use super::chats::{chat_path, resolve_accounts, resolve_chat, PICK};
use super::commands::cmd;
use super::{js_string, pretty, CliError, Cmd, Ctx, Flag, Res};
use beeper_api::http::query_string;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const COMMANDS: &[Cmd] = &[Cmd {
    description: "Creates a resumable Beeper Desktop export using the official Desktop API SDK.\nThe export directory contains accounts.json, chats.json, manifest.json, and one directory per chat with chat.json, messages.json, messages.markdown, messages.html, downloaded attachments, and checkpoint state for interrupted runs.",
    flags: &[
        Flag::many("account", "Limit to an account selector. Repeat to include more accounts."),
        Flag::many("chat", "Limit to a chat selector. Repeat to include more chats."),
        Flag::b("force", "Re-export chats even if checkpoint state says they are complete."),
        Flag::i("limit-chats", "Maximum chats to export. Intended for testing large exports."),
        Flag::i("limit-messages", "Maximum messages per chat. Intended for testing large exports."),
        Flag::i("max-participants", "Maximum participants to include in each chat.json.").def("500"),
        Flag::b("no-attachments", "Skip downloading message attachments."),
        Flag::s("out", "Export directory.").c('o').def("beeper-export"),
        PICK,
        Flag::b("quiet", "Suppress progress output."),
    ],
    ..cmd("export", "Export accounts, chats, messages, Markdown transcripts, and attachments", export)
}];

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> CliError + '_ {
    move |e| CliError::system(format!("{e}, open '{}'", path.display()))
}

/// `writeFileAtomic`: a temp file, then a rename.
fn write_atomic(path: &Path, content: &str) -> Res {
    std::fs::create_dir_all(path.parent().expect("an export file has a folder")).map_err(io(path))?;
    let tmp = PathBuf::from(format!("{}.tmp-{}", path.display(), std::process::id()));
    std::fs::write(&tmp, content).map_err(io(&tmp))?;
    std::fs::rename(&tmp, path).map_err(io(path))
}

fn write_json(path: &Path, value: &Value) -> Res {
    write_atomic(path, &format!("{}\n", pretty(value)))
}

fn read_jsonl(path: &Path) -> Res<Vec<Value>> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).map_err(|e| CliError::bug(e.to_string())))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(io(path)(e)),
    }
}

fn append(path: &Path, line: &Value) -> Res {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(io(path))?;
    writeln!(file, "{line}").map_err(io(path))
}

/// `safeSegment`: letters, digits, `.`, `_`, and `-`, at most 120.
fn safe_segment(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c.is_ascii_alphanumeric() || "._-".contains(c) {
            true => out.push(c),
            false if !out.ends_with('_') || out.is_empty() => out.push('_'),
            false => {}
        }
    }
    let out: String = out.trim_matches('_').chars().take(120).collect();
    if out.is_empty() {
        "item".into()
    } else {
        out
    }
}

fn basename(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or_default()
}

/// `safeFileName`: the base name without the characters that file systems reject, at most 160.
fn safe_file_name(value: &str) -> String {
    let mut out = String::new();
    for c in basename(value).chars() {
        match "/\\?%*:|\"<>".contains(c) {
            true if out.ends_with('\u{0}') => {}
            true => out.push('\u{0}'),
            false => out.push(c),
        }
    }
    let out = out.replace('\u{0}', "_");
    let units: Vec<u16> = out.trim().encode_utf16().take(160).collect();
    let out = String::from_utf16_lossy(&units);
    if out.is_empty() {
        "attachment".into()
    } else {
        out
    }
}

const MIME_EXTENSIONS: [(&str, &str); 5] = [
    ("image/jpeg", ".jpg"),
    ("image/png", ".png"),
    ("image/gif", ".gif"),
    ("video/mp4", ".mp4"),
    ("audio/mpeg", ".mp3"),
];

/// `fileNameFromURL`: the last part of the URL path, else `attachment` with the extension of the MIME type.
fn file_name_from_url(url: &str, mime: Option<&str>) -> String {
    let path = url.split_once("://").map(|(_, rest)| rest.split(['?', '#']).next().unwrap_or_default());
    let name = path.and_then(|p| p.split_once('/')).map(|(_, p)| basename(p).to_string()).filter(|n| !n.is_empty());
    name.unwrap_or_else(|| {
        let ext = match mime {
            None => String::new(),
            Some(m) => MIME_EXTENSIONS.iter().find(|e| e.0 == m).map_or_else(
                || m.split('/').nth(1).filter(|s| !s.contains('+')).map_or(String::new(), |s| format!(".{s}")),
                |e| e.1.to_string(),
            ),
        };
        format!("attachment{ext}")
    })
}

/// `encodeURI` of JavaScript.
fn encode_uri(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b.is_ascii_alphanumeric() || b";,/?:@&=+$-_.!~*'()#".contains(&b) {
            true => (b as char).to_string(),
            false => format!("%{b:02X}"),
        })
        .collect()
}

fn escape_markdown(value: &str) -> String {
    value.chars().flat_map(|c| ("\\`*_{}[]()#+.!|-".contains(c).then_some('\\')).into_iter().chain([c])).collect()
}

fn escape_html(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn text_of(v: &Value) -> String {
    if v.is_null() {
        String::new()
    } else {
        js_string(v)
    }
}

/// `message.senderName || message.senderID || 'Unknown sender'`.
fn sender(m: &Value) -> String {
    [&m["senderName"], &m["senderID"]].into_iter().find(|v| super::truthy(v)).map_or("Unknown sender".into(), js_string)
}

fn title(chat: &Value) -> String {
    js_string(if super::truthy(&chat["title"]) { &chat["title"] } else { &chat["id"] })
}

/// The attachments of each message, from `attachments.jsonl`.
fn by_message<'a>(attachments: &'a [Value], id: &Value) -> Vec<&'a Value> {
    attachments.iter().filter(|a| a["messageID"] == *id).collect()
}

fn attachment_label(item: &Value) -> String {
    js_string(if super::truthy(&item["attachment"]["fileName"]) {
        &item["attachment"]["fileName"]
    } else {
        &item["kind"]
    })
}

fn render_markdown(chat: &Value, messages: &[Value], attachments: &[Value]) -> String {
    let mut lines = vec![
        format!("# {}", escape_markdown(&title(chat))),
        String::new(),
        format!("- Chat ID: `{}`", js_string(&chat["id"])),
        format!("- Account ID: `{}`", text_of(&chat["accountID"])),
        format!("- Network: {}", escape_markdown(&text_of(&chat["network"]))),
        format!("- Type: {}", text_of(&chat["type"])),
        format!("- Messages: {}", messages.len()),
        String::new(),
        "## Messages".into(),
        String::new(),
    ];
    for m in messages {
        lines.push(format!("### {} - {}", escape_markdown(&sender(m)), text_of(&m["timestamp"])));
        if super::truthy(&m["text"]) {
            lines.extend([String::new(), js_string(&m["text"])]);
        }
        for item in by_message(attachments, &m["id"]) {
            lines.extend([
                String::new(),
                format!("- [{}]({})", escape_markdown(&attachment_label(item)), encode_uri(&text_of(&item["path"]))),
            ]);
        }
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n").trim_end())
}

const HTML_STYLE: &str = r#"    :root { color-scheme: light dark; }
    body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; line-height: 1.45; margin: 0; color: CanvasText; background: Canvas; }
    main { max-width: 920px; margin: 0 auto; padding: 32px 20px; }
    h1 { font-size: 28px; margin: 0 0 8px; }
    .meta { color: color-mix(in srgb, CanvasText 70%, Canvas); display: grid; gap: 4px; margin: 0 0 28px; }
    .message { border-top: 1px solid color-mix(in srgb, CanvasText 18%, Canvas); padding: 16px 0; }
    header { display: flex; flex-wrap: wrap; gap: 8px 12px; align-items: baseline; margin-bottom: 8px; }
    time { color: color-mix(in srgb, CanvasText 62%, Canvas); font-size: 13px; }
    .body { overflow-wrap: anywhere; }
    .attachments { margin: 10px 0 0; padding-left: 22px; }
    a { color: LinkText; }"#;

fn render_html(chat: &Value, messages: &[Value], attachments: &[Value]) -> String {
    let rows: Vec<String> = messages
        .iter()
        .map(|m| {
            let items = by_message(attachments, &m["id"]);
            let links = items.iter().map(|i| {
                format!(
                    "<li><a href=\"{}\">{}</a></li>",
                    escape_html(&encode_uri(&text_of(&i["path"]))),
                    escape_html(&attachment_label(i))
                )
            });
            let list = if items.is_empty() {
                String::new()
            } else {
                format!("<ul class=\"attachments\">{}</ul>", links.collect::<String>())
            };
            let ts = text_of(&m["timestamp"]);
            let body = match super::truthy(&m["text"]) {
                true => {
                    format!("  <div class=\"body\">{}</div>", escape_html(&js_string(&m["text"])).replace('\n', "<br>"))
                }
                false => String::new(),
            };
            let header = format!(
                "  <header><strong>{}</strong><time datetime=\"{}\">{}</time></header>",
                escape_html(&sender(m)),
                escape_html(&encode_uri(&ts)),
                escape_html(&ts)
            );
            [String::from("<article class=\"message\">"), header, body, list, "</article>".into()]
                .into_iter()
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    let rows = rows
        .join("\n")
        .split('\n')
        .map(|l| if l.is_empty() { String::new() } else { format!("      {l}") })
        .collect::<Vec<_>>()
        .join("\n");
    let t = escape_html(&title(chat));
    let meta = |label: &str, value: String| format!("      <span>{label}: {value}</span>");
    [
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">".to_string(),
        format!("  <title>{t}</title>\n  <style>\n{HTML_STYLE}\n  </style>\n</head>\n<body>\n  <main>\n    <h1>{t}</h1>"),
        "    <section class=\"meta\" aria-label=\"Chat metadata\">".into(),
        meta("Chat ID", format!("<code>{}</code>", escape_html(&js_string(&chat["id"])))),
        meta("Account ID", format!("<code>{}</code>", escape_html(&text_of(&chat["accountID"])))),
        meta("Network", escape_html(&text_of(&chat["network"]))),
        meta("Type", escape_html(&text_of(&chat["type"]))),
        meta("Messages", messages.len().to_string()),
        "    </section>\n    <section class=\"messages\">".into(),
        rows,
        "    </section>\n  </main>\n</body>\n</html>\n".into(),
    ]
    .join("\n")
}

struct Export {
    out: PathBuf,
    quiet: bool,
    events: bool,
    attachments: bool,
    limit_messages: Option<usize>,
}

impl Export {
    fn progress(&self, message: &str) {
        if self.events {
            eprintln!("{}", json!({ "event": "export.progress", "data": { "message": message }, "ts": now() }));
        }
        if !self.quiet {
            eprintln!("{message}");
        }
    }

    /// `downloadURL`: copies or downloads one attachment into the chat folder, once.
    fn download(&self, c: &mut Ctx, dir: &Path, source: &str, message: &str, index: usize, name: &str) -> Res<String> {
        let target = format!("attachments/{}/{:02}-{}", safe_segment(message), index + 1, safe_file_name(name));
        let absolute = dir.join(&target);
        if absolute.exists() {
            return Ok(target);
        }
        std::fs::create_dir_all(absolute.parent().expect("a target has a folder")).map_err(io(&absolute))?;
        let local = source.strip_prefix("file://").or(source.starts_with('/').then_some(source));
        if let Some(from) = local {
            std::fs::copy(from, &absolute).map_err(io(Path::new(from)))?;
            return Ok(target);
        }
        let bytes = match (source.starts_with("mxc://") || source.starts_with("localmxc://"), source.split_once("://"))
        {
            (true, _) => {
                let client = c.client()?.clone();
                let path = query_string("/v1/assets/serve", &[("url", source)]);
                c.block(client.fetch("GET", &path, None))?
            }
            (false, Some(_)) => {
                let (base, path) = super::stream::split_url(source);
                let client = beeper_api::client::BeeperClient::try_new(&base, "").map_err(CliError::bug)?;
                c.block(client.fetch("GET", &path, None))?
            }
            (false, None) => return Err(CliError::bug("fetch() URL is invalid")),
        };
        match bytes {
            (200..=299, body) => std::fs::write(&absolute, body).map_err(io(&absolute))?,
            (status, _) => return Err(CliError::bug(format!("Failed to download {source}: HTTP {status}"))),
        }
        Ok(target)
    }

    /// `exportChatMessages`: the pages of a chat into `messages.partial.jsonl`, with the cursor in the state.
    fn messages(
        &self,
        c: &mut Ctx,
        chat: &Value,
        dir: &Path,
        state: &mut Value,
        state_path: &Path,
    ) -> Res<(Vec<Value>, Vec<Value>, u64)> {
        let id = js_string(&chat["id"]);
        let partial = dir.join("messages.partial.jsonl");
        let manifest = dir.join("attachments").join("attachments.jsonl");
        let mut existing = read_jsonl(&partial)?;
        let mut seen: std::collections::HashSet<String> = existing.iter().map(|m| js_string(&m["id"])).collect();
        let mut cursor = state["chats"][&id]["cursor"].as_str().map(str::to_string);
        let mut count = state["chats"][&id]["attachmentCount"].as_u64().unwrap_or(0);
        'pages: loop {
            let query: Vec<(&str, &str)> = cursor.as_deref().map(|c| ("cursor", c)).into_iter().collect();
            let page = c.get(&query_string(&chat_path(&id, "/messages"), &query))?;
            let items = page["items"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                break;
            }
            for m in items {
                let mid = js_string(&m["id"]);
                if seen.contains(&mid) {
                    continue;
                }
                if self.attachments {
                    for (index, a) in m["attachments"].as_array().into_iter().flatten().enumerate() {
                        let source = [&a["id"], &a["srcURL"]].into_iter().find(|v| super::truthy(v)).map(js_string);
                        let file_name = a["fileName"].as_str().filter(|n| !n.is_empty()).map(str::to_string);
                        let poster_name =
                            if a["fileName"].is_null() { index.to_string() } else { js_string(&a["fileName"]) };
                        let downloads = [
                            (source, file_name, "attachment", a["mimeType"].as_str()),
                            (
                                a["posterImg"].as_str().map(str::to_string),
                                Some(format!("poster-{poster_name}")),
                                "poster",
                                None,
                            ),
                        ];
                        for (url, name, kind, mime) in downloads {
                            let Some(url) = url.filter(|u| !u.is_empty()) else { continue };
                            let name = name.unwrap_or_else(|| file_name_from_url(&url, mime));
                            let path = self.download(c, dir, &url, &mid, index, &name)?;
                            let entry = json!({ "attachment": a, "index": index, "kind": kind, "messageID": mid, "path": path, "sourceURL": url });
                            append(&manifest, &entry)?;
                            count += 1;
                        }
                    }
                }
                append(&partial, &m)?;
                seen.insert(mid);
                existing.push(m);
                if self.limit_messages.is_some_and(|l| existing.len() >= l) {
                    break;
                }
            }
            cursor = page["oldestCursor"].as_str().filter(|c| !c.is_empty()).map(str::to_string);
            let entry = state["chats"][&id].as_object_mut().expect("the chat has a state");
            entry.insert("cursor".into(), json!(cursor));
            entry.insert("messageCount".into(), json!(existing.len()));
            entry.insert("attachmentCount".into(), json!(count));
            entry.insert("updatedAt".into(), json!(now()));
            write_json(state_path, state)?;
            let downloaded = if self.attachments { format!(", {count} attachments") } else { String::new() };
            self.progress(&format!("  {}: {} messages{downloaded}", label(chat), existing.len()));
            if self.limit_messages.is_some_and(|l| existing.len() >= l) || page["hasMore"] != true || cursor.is_none() {
                break 'pages;
            }
        }
        let key = |m: &Value| js_string(if super::truthy(&m["sortKey"]) { &m["sortKey"] } else { &m["timestamp"] });
        existing.sort_by_key(key);
        Ok((existing, read_jsonl(&manifest)?, count))
    }
}

/// `chatTitle`: the title, and the network in parentheses.
fn label(chat: &Value) -> String {
    let network =
        if super::truthy(&chat["network"]) { format!(" ({})", js_string(&chat["network"])) } else { String::new() };
    format!("{}{network}", title(chat))
}

fn export(c: &mut Ctx) -> Res {
    let inputs = c.many("account").to_vec();
    let accounts = resolve_accounts(c, &inputs, true, true)?;
    let pick = c.int("pick");
    let mut chat_ids = Vec::new();
    for input in c.many("chat").to_vec() {
        chat_ids.push(resolve_chat(c, &input, pick, &accounts)?);
    }
    let out = c.str("out").expect("--out has a default").to_string();
    let job = Export {
        out: PathBuf::from(&out),
        quiet: c.on("quiet"),
        events: c.on("events"),
        attachments: !c.on("no-attachments"),
        limit_messages: c.int("limit-messages").map(|n| usize::try_from(n).unwrap_or(0)),
    };
    let max_participants = c.int("max-participants").expect("--max-participants has a default").to_string();
    std::fs::create_dir_all(job.out.join("chats")).map_err(io(&job.out))?;
    let state_path = job.out.join(".beeper-export-state.json");
    let fresh = || json!({ "chats": {}, "completedChatIDs": [], "createdAt": now(), "exportVersion": 1 });
    let mut state = match (c.on("force"), std::fs::read_to_string(&state_path)) {
        (false, Ok(text)) => serde_json::from_str(&text).map_err(|e| CliError::bug(e.to_string()))?,
        _ => fresh(),
    };
    let started = state["createdAt"].clone();
    job.progress(&format!("Export directory: {out}"));
    let account_list = match c.get("/v1/accounts")? {
        Value::Array(items) => items,
        page => page["items"].as_array().cloned().unwrap_or_default(),
    };
    write_json(&job.out.join("accounts.json"), &json!(account_list))?;
    job.progress(&format!("Accounts: {}", account_list.len()));
    let chats = match chat_ids.is_empty() {
        false => chat_ids
            .iter()
            .map(|id| c.get(&query_string(&chat_path(id, ""), &[("maxParticipantCount", "0")])))
            .collect::<Res<Vec<_>>>()?,
        true => {
            let query: Vec<(&str, String)> = accounts.iter().map(|a| ("accountIDs", a.clone())).collect();
            let limit = c.int("limit-chats").filter(|n| *n > 0).map_or(usize::MAX, |n| n as usize);
            c.collect("/v1/chats", &query, limit, |_| true)?
        }
    };
    write_json(&job.out.join("chats.json"), &json!(chats))?;
    job.progress(&format!("Chats queued: {}", chats.len()));
    let (mut total_messages, mut total_attachments) = (0u64, 0u64);
    for (i, listed) in chats.iter().enumerate() {
        let id = js_string(&listed["id"]);
        let dir = job.out.join("chats").join(safe_segment(&id));
        let done = state["completedChatIDs"].as_array().is_some_and(|ids| ids.contains(&json!(id)));
        if !c.on("force") && state["chats"][&id]["complete"] == true && done {
            total_messages += state["chats"][&id]["messageCount"].as_u64().unwrap_or(0);
            total_attachments += state["chats"][&id]["attachmentCount"].as_u64().unwrap_or(0);
            job.progress(&format!("[{}/{}] {} already complete", i + 1, chats.len(), label(listed)));
            continue;
        }
        job.progress(&format!("[{}/{}] {} starting", i + 1, chats.len(), label(listed)));
        if c.on("force") && dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(io(&dir))?;
        }
        std::fs::create_dir_all(dir.join("attachments")).map_err(io(&dir))?;
        let chat = c.get(&query_string(&chat_path(&id, ""), &[("maxParticipantCount", &max_participants)]))?;
        write_json(&dir.join("chat.json"), &chat)?;
        let chats_state = state["chats"].as_object_mut().expect("the state has chats");
        let entry = chats_state.entry(id.clone()).or_insert_with(|| {
            json!({ "attachmentCount": 0, "complete": false, "cursor": null, "messageCount": 0, "startedAt": now(), "updatedAt": now() })
        });
        let entry = entry.as_object_mut().expect("a chat state is an object");
        entry.insert("complete".into(), json!(false));
        entry.shift_remove("error");
        write_json(&state_path, &state)?;
        match job.messages(c, &chat, &dir, &mut state, &state_path) {
            Ok((messages, attachments, count)) => {
                let entry = state["chats"][&id].as_object_mut().expect("a chat state is an object");
                let update = [
                    ("attachmentCount", json!(count)),
                    ("complete", json!(true)),
                    ("cursor", Value::Null),
                    ("messageCount", json!(messages.len())),
                    ("updatedAt", json!(now())),
                ];
                update.into_iter().for_each(|(k, v)| drop(entry.insert(k.into(), v)));
                let completed = state["completedChatIDs"].as_array_mut().expect("the state has completed chats");
                if !completed.contains(&json!(id)) {
                    completed.push(json!(id));
                }
                write_json(&dir.join("messages.json"), &json!(messages))?;
                write_atomic(&dir.join("messages.markdown"), &render_markdown(&chat, &messages, &attachments))?;
                write_atomic(&dir.join("messages.html"), &render_html(&chat, &messages, &attachments))?;
                let _ = std::fs::remove_file(dir.join("messages.partial.jsonl"));
                write_json(&state_path, &state)?;
                total_messages += messages.len() as u64;
                total_attachments += count;
                let summary = format!("{} messages, {count} attachments", messages.len());
                job.progress(&format!("[{}/{}] {} complete: {summary}", i + 1, chats.len(), label(&chat)));
            }
            Err(e) => {
                let entry = state["chats"][&id].as_object_mut().expect("a chat state is an object");
                entry.insert("complete".into(), json!(false));
                entry.insert("error".into(), json!(e.msg));
                entry.insert("updatedAt".into(), json!(now()));
                write_json(&state_path, &state)?;
                return Err(e);
            }
        }
    }
    let manifest = json!({
        "accounts": account_list,
        "attachmentCount": total_attachments,
        "chatCount": chats.len(),
        "completedAt": now(),
        "createdAt": started,
        "messageCount": total_messages,
        "version": 1,
    });
    write_json(&job.out.join("manifest.json"), &manifest)?;
    job.progress(&format!("Done: {} chats, {total_messages} messages, {total_attachments} attachments", chats.len()));
    println!("Exported {} chats, {total_messages} messages, {total_attachments} attachments to {out}", chats.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_escapes_follow_upstream() {
        assert_eq!(safe_segment("!wa_eric:beeper.com"), "wa_eric_beeper.com");
        assert_eq!(safe_segment("::"), "item");
        assert_eq!(safe_file_name("dir/a:b?.png"), "a_b_.png");
        assert_eq!(file_name_from_url("mxc://beeper.com/att_1", None), "att_1");
        assert_eq!(file_name_from_url("att_1", Some("image/png")), "attachment.png");
        assert_eq!(file_name_from_url("x", Some("application/vnd.a+json")), "attachment");
        assert_eq!(escape_markdown("a.b-c"), "a\\.b\\-c");
        assert_eq!(encode_uri("attachments/m 1/01-ü.png"), "attachments/m%201/01-%C3%BC.png");
    }
}
