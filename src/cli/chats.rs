//! The `chats` topic, and the chat and account selectors of `lib/resolve.js`.

use super::render::Empty;
use super::{arg, config, js_string, obj, truthy, CliError, Cmd, Ctx, Flag, Res, AMBIGUOUS, NOT_FOUND};
use beeper_api::http::query_string;
use beeper_core::codec::url_encode;
use serde_json::{json, Value};

pub const CHAT: Flag = Flag::s("chat", "Chat selector (ID, local ID, title, or search text)").req();
pub const PICK: Flag = Flag::i("pick", "Pick the Nth result when the selector is ambiguous (1-indexed)");
pub const ACCOUNT: Flag = Flag::many("account", "Limit to Account ID, network, bridge, or account user");
const IDS: Flag = Flag::b("ids", "Print preferred chat selectors, using numeric local chat IDs when available");

pub const LIST_FLAGS: &[Flag] = &[
    ACCOUNT,
    IDS,
    Flag::i("limit", "Maximum chats to print").def("20"),
    Flag::no("archived", "Only archived chats (--no-archived to exclude)"),
    Flag::no("pinned", "Only pinned chats (--no-pinned to exclude)"),
    Flag::no("muted", "Only muted chats (--no-muted to exclude)"),
    Flag::no("unread", "Only chats with unread messages (--no-unread to exclude)"),
    Flag::no("low-priority", "Only Low Priority chats (--no-low-priority to exclude)"),
];

const fn on_chat(id: &'static str, summary: &'static str, run: super::Run) -> Cmd {
    Cmd { flags: &[CHAT, PICK], ..super::commands::cmd(id, summary, run) }
}

pub const COMMANDS: &[Cmd] = &[
    on_chat("chats archive", "Archive a chat", patch),
    Cmd {
        flags: &[
            CHAT,
            PICK,
            Flag::s("file", "Image file to upload as the new avatar"),
            Flag::b("clear", "Clear the existing avatar instead of setting a new one"),
        ],
        ..on_chat("chats avatar", "Set a chat avatar", patch)
    },
    Cmd {
        flags: &[
            CHAT,
            PICK,
            Flag::s("description", "New chat description"),
            Flag::b("clear", "Clear the existing description instead of setting one"),
        ],
        ..on_chat("chats description", "Set a chat description", patch)
    },
    Cmd {
        flags: &[CHAT, PICK, Flag::s("seconds", "Timer in seconds, or \"off\" to disable").req()],
        examples: &[
            "beeper chats disappear --chat \"Mom\" --seconds 86400",
            "beeper chats disappear --chat \"Work Group\" --seconds off",
        ],
        ..on_chat("chats disappear", "Set disappearing-message expiry", patch)
    },
    Cmd {
        flags: &[
            CHAT,
            PICK,
            Flag::s("text", "Draft text. Omit and pass --clear to remove the draft."),
            Flag::s("file", "Attachment file to upload with the draft"),
            Flag::s("filename", "Override the displayed filename of the attachment"),
            Flag::s("mime", "Override MIME type detection for the attachment"),
            Flag::b("clear", "Clear the existing draft instead of setting one"),
        ],
        ..on_chat("chats draft", "Set or clear a chat draft", draft)
    },
    Cmd {
        flags: &[
            CHAT,
            PICK,
            Flag::s("message", "Scroll Desktop to this message ID after focusing"),
            Flag::s("draft", "Prefill the chat composer with this draft text"),
            Flag::s("attachment", "Prefill the chat composer with this attachment file path"),
        ],
        ..on_chat("chats focus", "Focus Beeper Desktop on a chat", focus)
    },
    Cmd { flags: LIST_FLAGS, ..super::commands::cmd("chats list", "List chats", list) },
    Cmd {
        flags: &[CHAT, PICK, Flag::s("message", "Mark read at (or unread starting from) this message ID")],
        ..on_chat("chats mark-read", "Mark a chat as read", post)
    },
    Cmd {
        flags: &[CHAT, PICK, Flag::s("message", "Mark read at (or unread starting from) this message ID")],
        ..on_chat("chats mark-unread", "Mark a chat as unread", post)
    },
    on_chat("chats mute", "Mute a chat", patch),
    on_chat("chats notify-anyway", "Send an iMessage Notify Anyway alert", post),
    on_chat("chats pin", "Pin a chat", patch),
    Cmd {
        flags: &[
            CHAT,
            PICK,
            Flag::s("level", "Destination: inbox (default mailbox) or low (Low Priority)")
                .req()
                .opts(&["inbox", "low"]),
        ],
        ..on_chat("chats priority", "Move a chat to the Inbox or Low Priority", patch)
    },
    Cmd {
        flags: &[
            CHAT,
            PICK,
            Flag::s("when", "ISO timestamp when the reminder should trigger").req(),
            Flag::b("dismiss-on-message", "Dismiss the reminder automatically when a new message arrives"),
        ],
        examples: &[
            "beeper chats remind --chat \"Mom\" --when 2024-12-25T09:00:00Z",
            "beeper chats remind --chat \"Work\" --when 2024-12-25T09:00:00Z --dismiss-on-message",
        ],
        ..on_chat("chats remind", "Set a chat reminder", remind)
    },
    Cmd {
        flags: &[CHAT, PICK, Flag::s("title", "New chat title").req()],
        ..on_chat("chats rename", "Rename a chat", patch)
    },
    Cmd {
        args: &[arg("query", "Search query (title, participant, or network)")],
        flags: &[ACCOUNT, IDS, Flag::i("limit", "Maximum chats to print").def("20")],
        ..super::commands::cmd("chats search", "Search chats", search)
    },
    Cmd {
        flags: &[CHAT, PICK, Flag::i("max-participants", "Limit number of participants returned in chat details")],
        ..on_chat("chats show", "Show chat details", show)
    },
    Cmd {
        args: &[arg("user", "User ID, phone number, email, or display name")],
        flags: &[
            Flag::s("account", "Account selector. Defaults to the single available account or the matrix account."),
            Flag::s("title", "Optional initial title for a new group chat"),
        ],
        ..super::commands::cmd("chats start", "Start a chat", start)
    },
    on_chat("chats unarchive", "Unarchive a chat", patch),
    on_chat("chats unmute", "Unmute a chat", patch),
    on_chat("chats unpin", "Unpin a chat", patch),
    on_chat("chats unremind", "Clear a chat reminder", unremind),
];

pub fn chat_path(id: &str, rest: &str) -> String {
    format!("/v1/chats/{}{rest}", url_encode(id))
}

/// `normalize` of `lib/resolve.js`.
fn normalize(v: &Value) -> String {
    let s = if v.is_null() { String::new() } else { js_string(v) };
    s.trim().to_lowercase().chars().filter(|c| !c.is_whitespace() && !"._-".contains(*c)).collect()
}

fn ambiguous(label: &str, choices: &[String]) -> CliError {
    let list: Vec<String> = choices.iter().enumerate().map(|(i, c)| format!("  {}. {c}", i + 1)).collect();
    CliError::abort(format!("Ambiguous {label}. Use an exact ID or --pick N:\n{}", list.join("\n")), AMBIGUOUS)
}

fn chat_input_id(chat: &Value) -> String {
    js_string(if truthy(&chat["localChatID"]) { &chat["localChatID"] } else { &chat["id"] })
}

fn format_chat(chat: &Value) -> String {
    let network = if truthy(&chat["network"]) { format!(" {}", js_string(&chat["network"])) } else { String::new() };
    let local = if truthy(&chat["localChatID"]) {
        format!(" local:{}", js_string(&chat["localChatID"]))
    } else {
        String::new()
    };
    let title = if chat["title"].is_null() { String::new() } else { js_string(&chat["title"]) };
    format!("{}{local}{network} {title}", js_string(&chat["id"])).trim().to_string()
}

fn levenshtein(a: &[u16], b: &[u16]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + usize::from(ca != cb)));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// `rankSuggestions`: the closest titles, by edit distance or by containment.
fn rank<'a>(query: &str, pool: &'a [Value], max: usize) -> Vec<(&'a Value, String)> {
    let q: Vec<u16> = query.trim().to_lowercase().encode_utf16().collect();
    let mut scored: Vec<(usize, &Value, String)> = pool
        .iter()
        .filter_map(|chat| {
            let label = js_string(&chat["title"]);
            let l: Vec<u16> = label.to_lowercase().encode_utf16().collect();
            let contains = l.windows(q.len().max(1)).any(|w| w == q.as_slice()) || q.is_empty();
            let dist = levenshtein(&q, &l).min(if contains { l.len().saturating_sub(q.len()) } else { usize::MAX });
            truthy(&chat["title"]).then_some((dist, chat, label))
        })
        .collect();
    scored.sort_by_key(|s| (s.0, s.2.encode_utf16().count()));
    let cutoff = 3.max((q.len() as f64 * 0.6).ceil() as usize);
    scored.into_iter().filter(|s| s.0 <= cutoff).take(max).map(|s| (s.1, s.2)).collect()
}

/// `confirmSuggestion`: yes after 10 s, and no when stdin or stderr is not a terminal.
fn confirm(prompt: &str) -> bool {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return false;
    }
    eprint!("{prompt} [Y/n] ");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = tx.send(std::io::stdin().read_line(&mut line).map(|_| line));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok(answer)) => ["", "y", "yes"].contains(&answer.trim().to_lowercase().as_str()),
        _ => true,
    }
}

/// `resolveChatID`: a Matrix ID as it is, else an exact chat, else the title search with `--pick`.
pub fn resolve_chat(c: &mut Ctx, input: &str, pick: Option<i64>, accounts: &[String]) -> Res<String> {
    if input.starts_with('!') {
        return Ok(input.to_string());
    }
    match c.get(&query_string(&chat_path(input, ""), &[("maxParticipantCount", "0")])) {
        Ok(chat) => return Ok(chat_input_id(&chat)),
        Err(e) if e.msg.to_lowercase().contains("not found") || e.msg.contains("404") => {}
        Err(e) => return Err(e),
    }
    let mut query: Vec<(&str, String)> = accounts.iter().map(|a| ("accountIDs", a.clone())).collect();
    query.extend([("query", input.to_string()), ("scope", "titles".to_string())]);
    let candidates = c.collect("/v1/chats/search", &query, 10, |_| true)?;
    let wanted = normalize(&json!(input));
    let exact: Vec<&Value> = candidates
        .iter()
        .filter(|ch| ["id", "localChatID", "title"].iter().any(|k| normalize(&ch[*k]) == wanted))
        .collect();
    let matches: Vec<&Value> = if exact.is_empty() { candidates.iter().collect() } else { exact };
    match (matches.as_slice(), pick) {
        ([], _) => suggest(c, input, accounts),
        ([one], _) => Ok(chat_input_id(one)),
        (all, Some(n)) => match usize::try_from(n - 1).ok().and_then(|i| all.get(i)) {
            Some(chat) => Ok(chat_input_id(chat)),
            None => Err(CliError::abort(format!("--pick {n} is outside the {} matching chats", all.len()), NOT_FOUND)),
        },
        (all, None) => {
            Err(ambiguous(&format!("chat \"{input}\""), &all.iter().map(|ch| format_chat(ch)).collect::<Vec<_>>()))
        }
    }
}

/// `suggestChat`: offers the closest title, and gives up with exit 127 when the user says no.
fn suggest(c: &mut Ctx, input: &str, accounts: &[String]) -> Res<String> {
    let mut query: Vec<(&str, String)> = accounts.iter().map(|a| ("accountIDs", a.clone())).collect();
    query.push(("limit", "100".into()));
    let Ok(pool) = c.collect("/v1/chats", &query, 100, |_| true) else { return Ok(input.to_string()) };
    let ranked = rank(input, &pool, 3);
    let Some((top, label)) = ranked.first() else { return Ok(input.to_string()) };
    let net =
        |ch: &Value| if truthy(&ch["network"]) { format!(" ({})", js_string(&ch["network"])) } else { String::new() };
    eprintln!("No chat matches \"{input}\". Did you mean \"{label}\"{}?", net(top));
    ranked[1..].iter().for_each(|(ch, l)| eprintln!("  also: {l}{}", net(ch)));
    match confirm("use it?") {
        true => Ok(chat_input_id(top)),
        false => Err(CliError {
            class: Some("CLIError"),
            ..CliError::abort(format!("no chat selected for \"{input}\""), 127)
        }),
    }
}

/// The chat of the `--chat` (or `--to`) flag.
pub fn chat_flag(c: &mut Ctx, flag: &str) -> Res<String> {
    let input = c.str(flag).expect("the chat flag is required").to_string();
    let pick = c.int("pick");
    resolve_chat(c, &input, pick, &[])
}

fn accounts_of(list: Value) -> Vec<Value> {
    match list {
        Value::Array(items) => items,
        mut page => page["items"].as_array_mut().map(std::mem::take).unwrap_or_default(),
    }
}

const ACCOUNT_KEYS: [(&str, &str); 9] = [
    ("accountID", ""),
    ("network", ""),
    ("bridge", "type"),
    ("bridge", "id"),
    ("user", "id"),
    ("user", "username"),
    ("user", "displayName"),
    ("user", "name"),
    ("user", "email"),
];

fn account_field<'a>(a: &'a Value, (key, sub): (&str, &str)) -> &'a Value {
    if sub.is_empty() {
        &a[key]
    } else {
        &a[key][sub]
    }
}

/// `matchAccounts`: exact matches of any field, else the accounts whose ID, network, bridge, or name contains it.
fn match_accounts<'a>(accounts: &'a [Value], input: &str) -> Vec<&'a Value> {
    let wanted = normalize(&json!(input));
    let exact: Vec<&Value> =
        accounts.iter().filter(|a| ACCOUNT_KEYS.iter().any(|k| normalize(account_field(a, *k)) == wanted)).collect();
    if !exact.is_empty() {
        return exact;
    }
    let partial = [0, 1, 2, 3, 6, 7].map(|i| ACCOUNT_KEYS[i]);
    accounts.iter().filter(|a| partial.iter().any(|k| normalize(account_field(a, *k)).contains(&wanted))).collect()
}

fn format_account(a: &Value) -> String {
    let network = if truthy(&a["network"]) { format!(" {}", js_string(&a["network"])) } else { String::new() };
    let bridge =
        if truthy(&a["bridge"]["type"]) { format!(" {}", js_string(&a["bridge"]["type"])) } else { String::new() };
    let user = ["displayName", "name", "username", "id"].iter().map(|k| &a["user"][*k]).find(|v| truthy(v));
    let user = user.map_or(String::new(), |u| format!(" {}", js_string(u)));
    format!("{}{network}{bridge}{user}", js_string(&a["accountID"]))
}

/// `resolveAccountIDs`: the account IDs of `inputs`, or of the default account when there are none.
pub fn resolve_accounts(c: &mut Ctx, inputs: &[String], multiple: bool, default: bool) -> Res<Vec<String>> {
    let fallback = config()?.get("defaultAccount").and_then(Value::as_str).map(str::to_string);
    let inputs: Vec<String> = match (inputs.is_empty() && default, fallback) {
        (true, Some(account)) => vec![account],
        _ => inputs.to_vec(),
    };
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let accounts = accounts_of(c.get("/v1/accounts")?);
    let mut ids: Vec<String> = Vec::new();
    for input in &inputs {
        let matches = match_accounts(&accounts, input);
        match (matches.len(), multiple) {
            (0, _) => return Err(CliError::abort(format!("No account matches \"{input}\""), NOT_FOUND)),
            (2.., false) => {
                let choices: Vec<String> = matches.iter().map(|a| format_account(a)).collect();
                return Err(ambiguous(&format!("account \"{input}\""), &choices));
            }
            _ => ids.extend(matches.iter().map(|a| js_string(&a["accountID"]))),
        }
    }
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    Ok(ids)
}

pub fn account_ids(c: &mut Ctx) -> Res<Vec<String>> {
    Ok(accounts_of(c.get("/v1/accounts")?)
        .iter()
        .map(|a| js_string(&a["accountID"]))
        .filter(|a| !a.is_empty())
        .collect())
}

// ─── commands ──────────────────────────────────────────────────────────────────

pub fn list(c: &mut Ctx) -> Res {
    let inputs = c.many("account").to_vec();
    let accounts = resolve_accounts(c, &inputs, true, true)?;
    let filters = ["archived", "pinned", "muted", "low-priority", "unread"].map(|f| (f, c.opt_bool(f)));
    let filtered = filters.iter().any(|f| f.1.is_some());
    let keep = |row: &Value| {
        filters.iter().all(|(name, want)| {
            let have = match *name {
                "unread" => row["unreadCount"].as_f64().unwrap_or(0.0) > 0.0 || truthy(&row["isMarkedUnread"]),
                "low-priority" => truthy(&row["isLowPriority"]),
                n => truthy(&row[format!("is{}{}", n[..1].to_uppercase(), &n[1..])]),
            };
            want.is_none_or(|w| w == have)
        })
    };
    let query: Vec<(&str, String)> = accounts.into_iter().map(|a| ("accountIDs", a)).collect();
    let limit = usize::try_from(c.int("limit").expect("--limit has a default")).unwrap_or(0);
    let items = c.collect("/v1/chats", &query, limit, keep)?;
    let subtitle =
        if filtered { "Try relaxing the filter flags." } else { "Connect an account or sync existing chats." };
    match c.on("ids") {
        true => c.print_ids(&items),
        false => c.print_list(&items, Empty::new("No chats matched", subtitle)),
    }
}

fn search(c: &mut Ctx) -> Res {
    let inputs = c.many("account").to_vec();
    let accounts = resolve_accounts(c, &inputs, true, true)?;
    let q = c.arg(0).expect("a required arg").to_string();
    let mut query = vec![("query", q.clone())];
    query.extend(accounts.into_iter().map(|a| ("accountIDs", a)));
    let limit = usize::try_from(c.int("limit").expect("--limit has a default")).unwrap_or(0);
    let items = c.collect("/v1/chats/search", &query, limit, |_| true)?;
    match c.on("ids") {
        true => c.print_ids(&items),
        false => c.print_list(&items, Empty::new("No chats matched", format!("Nothing found for \"{q}\"."))),
    }
}

fn show(c: &mut Ctx) -> Res {
    let id = chat_flag(c, "chat")?;
    let max = c.int("max-participants").map(|n| n.to_string());
    let path =
        query_string(&chat_path(&id, ""), &max.as_deref().map(|m| [("maxParticipantCount", m)]).unwrap_or_default());
    let chat = c.get(&path)?;
    c.print(&chat)
}

/// `userQueryFromInput`: the user fields that a selector can mean.
fn user_query(input: &str) -> Value {
    let s = input.trim();
    let matrix = s
        .strip_prefix('@')
        .and_then(|r| r.split_once(':'))
        .is_some_and(|(local, server)| !local.is_empty() && !server.is_empty());
    let phone =
        s.chars().count() >= 5 && s.trim_start_matches('+').chars().all(|c| c.is_ascii_digit() || " ().-".contains(c));
    match () {
        _ if matrix => json!({ "id": s, "username": s }),
        _ if s.contains('@') => json!({ "email": s, "username": s }),
        _ if phone && !s.trim_start_matches('+').is_empty() => json!({ "phoneNumber": s }),
        _ => json!({ "fullName": s, "username": s, "id": s }),
    }
}

fn start(c: &mut Ctx) -> Res {
    c.writable()?;
    let account = match c.str("account").map(str::to_string) {
        Some(input) => {
            let ids = resolve_accounts(c, std::slice::from_ref(&input), false, true)?;
            ids.into_iter()
                .next()
                .ok_or_else(|| CliError::abort(format!("No account matches \"{input}\""), NOT_FOUND))?
        }
        None => match account_ids(c)? {
            ids if ids.iter().any(|a| a == "matrix") => "matrix".into(),
            ids if ids.len() == 1 => ids[0].clone(),
            _ => return Err(CliError::bug("Use --account to choose which account should start the chat.")),
        },
    };
    let user = user_query(c.arg(0).expect("a required arg"));
    let body = obj([("accountID", json!(account)), ("user", user), ("title", json!(c.str("title")))]);
    let chat = c.call("POST", "/v1/chats/start", Some(Value::Object(body)))?;
    c.print(&chat)
}

/// The `PATCH /v1/chats/{id}` body of each chat setting command.
fn patch_body(c: &Ctx) -> Res<Value> {
    let need = |flag: &str, msg: &str| match c.on("clear") || c.str(flag).is_some() {
        true => Ok(()),
        false => Err(CliError::bug(msg)),
    };
    Ok(match c.cmd.id {
        "chats archive" => json!({ "isArchived": true }),
        "chats unarchive" => json!({ "isArchived": false }),
        "chats pin" => json!({ "isPinned": true }),
        "chats unpin" => json!({ "isPinned": false }),
        "chats mute" => json!({ "isMuted": true }),
        "chats unmute" => json!({ "isMuted": false }),
        "chats rename" => json!({ "title": c.str("title") }),
        "chats priority" if c.str("level") == Some("inbox") => json!({ "isArchived": false, "isLowPriority": false }),
        "chats priority" => json!({ "isLowPriority": true }),
        "chats description" => {
            need("description", "Provide --description or --clear")?;
            json!({ "description": if c.on("clear") { None } else { c.str("description") } })
        }
        "chats avatar" => {
            need("file", "Provide --file or --clear")?;
            json!({ "imgURL": if c.on("clear") { None } else { c.str("file") } })
        }
        "chats disappear" => {
            let seconds = c.str("seconds").expect("--seconds is required");
            let expiry = match seconds.to_lowercase() == "off" {
                true => None,
                false => Some(
                    seconds
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| CliError::bug("--seconds must be a positive integer or \"off\""))?,
                ),
            };
            json!({ "messageExpirySeconds": expiry })
        }
        id => unreachable!("{id} has no chat setting"),
    })
}

fn patch(c: &mut Ctx) -> Res {
    c.writable()?;
    let body = patch_body(c)?;
    let id = chat_flag(c, "chat")?;
    let chat = c.call("PATCH", &chat_path(&id, ""), Some(body))?;
    c.print(&chat)
}

/// The commands that post to a chat route: mark-read, mark-unread, notify-anyway.
fn post(c: &mut Ctx) -> Res {
    c.writable()?;
    let id = chat_flag(c, "chat")?;
    let (route, body) = match c.cmd.id {
        "chats mark-read" => ("/read", Some(Value::Object(obj([("messageID", json!(c.str("message")))])))),
        "chats mark-unread" => ("/unread", Some(Value::Object(obj([("messageID", json!(c.str("message")))])))),
        _ => ("/notify-anyway", None),
    };
    let reply = c.call("POST", &chat_path(&id, route), body)?;
    c.print(&reply)
}

/// `assets.upload`: the uploaded file, with its `uploadID`.
pub fn upload(c: &mut Ctx, path: &str, file_name: Option<&str>, mime: Option<&str>) -> Res<Value> {
    let bytes = std::fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => CliError::system(format!("ENOENT: no such file or directory, open '{path}'")),
        _ => CliError::system(format!("{e}, open '{path}'")),
    })?;
    let name = std::path::Path::new(path).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let fields: Vec<(&str, &str)> =
        [("fileName", file_name), ("mimeType", mime)].into_iter().filter_map(|(k, v)| Some((k, v?))).collect();
    let client = c.client()?.clone();
    Ok(c.block(client.upload(&name, &bytes, &fields))?)
}

fn draft(c: &mut Ctx) -> Res {
    c.writable()?;
    let (text, file, clear) = (c.str("text").map(str::to_string), c.str("file").map(str::to_string), c.on("clear"));
    if !clear && text.is_none() {
        return Err(CliError::bug("Provide --text TEXT (and optionally --file PATH) or --clear."));
    }
    if clear && (text.is_some() || file.is_some()) {
        return Err(CliError::bug("--clear cannot be combined with --text or --file."));
    }
    let id = chat_flag(c, "chat")?;
    let draft = match (clear, file) {
        (true, _) => Value::Null,
        (false, None) => json!({ "text": text }),
        (false, Some(file)) => {
            let (name, mime) = (c.str("filename").map(str::to_string), c.str("mime").map(str::to_string));
            let up = upload(c, &file, name.as_deref(), mime.as_deref())?;
            let attachments = up["uploadID"].as_str().filter(|u| !u.is_empty()).map(|u| json!({ u: up }));
            Value::Object(obj([("text", json!(text)), ("attachments", attachments.unwrap_or_default())]))
        }
    };
    let chat = c.call("PATCH", &chat_path(&id, ""), Some(json!({ "draft": draft })))?;
    c.print(&chat)
}

fn focus(c: &mut Ctx) -> Res {
    c.writable()?;
    let id = chat_flag(c, "chat")?;
    let body = obj([
        ("chatID", json!(id)),
        ("messageID", json!(c.str("message"))),
        ("draftText", json!(c.str("draft"))),
        ("draftAttachmentPath", json!(c.str("attachment"))),
    ]);
    let reply = c.call("POST", "/v1/focus", Some(Value::Object(body)))?;
    c.print(&reply)
}

fn remind(c: &mut Ctx) -> Res {
    c.writable()?;
    let id = chat_flag(c, "chat")?;
    let when = c.str("when").expect("--when is required").to_string();
    let dismiss = c.on("dismiss-on-message").then_some(true);
    let reminder = obj([("remindAt", json!(when)), ("dismissOnIncomingMessage", json!(dismiss))]);
    c.call("POST", &chat_path(&id, "/reminders"), Some(json!({ "reminder": reminder })))?;
    c.success("Reminder set", Some(&when), json!({ "chatID": id, "remindAt": when }))
}

fn unremind(c: &mut Ctx) -> Res {
    c.writable()?;
    let id = chat_flag(c, "chat")?;
    c.call("DELETE", &chat_path(&id, "/reminders"), None)?;
    c.success("Reminder cleared", None, json!({ "chatID": id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_queries_follow_upstream() {
        assert_eq!(user_query("@eric:beeper.com"), json!({ "id": "@eric:beeper.com", "username": "@eric:beeper.com" }));
        assert_eq!(
            user_query("eric@example.com"),
            json!({ "email": "eric@example.com", "username": "eric@example.com" })
        );
        assert_eq!(user_query("+1 (415) 555-0199"), json!({ "phoneNumber": "+1 (415) 555-0199" }));
        assert_eq!(user_query(" Eric "), json!({ "fullName": "Eric", "username": "Eric", "id": "Eric" }));
    }

    #[test]
    fn suggestions_rank_by_distance_then_length() {
        let pool = [json!({ "title": "Family" }), json!({ "title": "Famiy" }), json!({ "title": "Family Group" })];
        let ranked: Vec<String> = rank("famly", &pool, 3).into_iter().map(|r| r.1).collect();
        assert_eq!(ranked, ["Famiy", "Family"]);
    }
}
