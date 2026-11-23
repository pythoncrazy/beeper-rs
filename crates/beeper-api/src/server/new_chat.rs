//! Desktop's chat creation (`createChat` and `startChat`) and contacts (`searchContacts` and `listContacts`), which
//! ask the bridge of the account through the SDK's `PlatformAPI`. Without a Matrix client, the demo makes local chats
//! and finds the people of its chats. The bridge calls run without the store lock.

use super::{
    bad, check, create_chat_issues, custom_issue, error, error_with, invalid_input, missing, ok, search, store_error, string_issue,
    too_short, type_issue, view, zod_type, Issue, Reply, Req, Response, MIN_1,
};
use crate::http::StatusCode;
use beeper_core::matrix::{bridge_user, ChatBridge};
use beeper_core::models::{locale_cmp, CursorPage, User};
use beeper_core::store::{BeeperStore, Draft, SharedStore};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The routes of the module, or `None` for another route. Desktop's `createChat` with `mode: "start"` is
/// `startChat`, and beeper-rs's `mode: "merge"` stays with the other chat routes.
pub(super) fn route(store: &SharedStore, method: &str, segs: &[&str], r: &Req) -> Option<Reply> {
    match (method, segs) {
        ("POST", ["v1", "chats"]) => match r.str("mode") {
            Some("merge") => None,
            Some("start") => Some(start_chat(store, r)),
            _ => Some(create_chat(store, r)),
        },
        ("POST", ["v1", "chats", "start"]) => Some(start_chat(store, r)),
        ("GET", ["v1", "accounts", id, "contacts", rest @ ..]) => Some(contacts(store, id, rest, r)),
        _ => None,
    }
}

/// Desktop's `Dt`: the digits and `+` of a phone number.
fn phone_key(s: &str) -> Option<String> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    (!digits.is_empty()).then_some(digits)
}

/// Desktop's `yt`: an email in lower case.
fn email_key(s: &str) -> Option<String> {
    let e = search::js_trim(s).to_lowercase();
    (!e.is_empty()).then_some(e)
}

/// Desktop's `Tr`: a username without its `@`.
fn handle(s: &str) -> Option<&str> {
    let t = search::js_trim(s);
    Some(t.strip_prefix('@').unwrap_or(t)).filter(|t| !t.is_empty())
}

/// Desktop's `fr`: a username without its `@`, in lower case.
fn username_key(s: &str) -> Option<String> {
    handle(s).map(str::to_lowercase)
}

/// Desktop's `getResolveIdentifierCapabilities`: the identifiers that the bridge of an account takes.
struct Lookups {
    create_dm: bool,
    /// `lookup_phone` or `any_phone`.
    phone: bool,
    email: bool,
    username: bool,
}

impl Lookups {
    /// The demo takes every identifier.
    const DEMO: Lookups = Lookups { create_dm: true, phone: true, email: true, username: true };

    /// The `resolve_identifier` capabilities of the bridge of an account, or none when the bridge does not answer,
    /// like Desktop's for a bridge whose capabilities did not load.
    fn of(store: &SharedStore, account: &str) -> Self {
        let caps = tokio::runtime::Handle::current().block_on(ChatBridge::of(store, account).capabilities());
        let caps = &caps.unwrap_or_default()["resolve_identifier"];
        let on = |k: &str| caps[k] == true;
        Lookups {
            create_dm: on("create_dm"),
            phone: on("lookup_phone") || on("any_phone"),
            email: on("lookup_email"),
            username: on("lookup_username"),
        }
    }
}

/// The identifiers of Desktop's `StartChatInput.user`, normalized like Desktop's `Ft`.
struct Person {
    id: Option<String>,
    phone: Option<String>,
    email: Option<String>,
    username: Option<String>,
}

/// The issues of Desktop's zod schema `StartChatInput` for a request body.
fn start_chat_issues(b: &Value) -> Vec<Issue> {
    let field = |k: &'static str| string_issue(b.get(k), k == "messageText", json!([k]));
    let mut issues: Vec<Issue> = [field("accountID"), field("messageText")].into_iter().flatten().collect();
    match b.get("user") {
        Some(Value::Object(u)) => {
            let fields = ["id", "username", "phoneNumber", "email", "fullName"];
            let found: Vec<Issue> = fields.iter().filter_map(|k| string_issue(u.get(*k), true, json!(["user", k]))).collect();
            // zod runs the refinement of `user` only when its fields parse.
            let named = fields[..4].iter().any(|k| u.get(*k).and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
            if found.is_empty() && !named {
                issues.push(custom_issue("At least one of id, username, phoneNumber, or email is required.", json!(["user", "id"])));
            }
            issues.extend(found);
        }
        v => issues.push(type_issue(["code", "expected", "received"], "object", zod_type(v), json!(["user"]))),
    }
    if let Some(v) = b.get("allowInvite").filter(|v| !v.is_boolean()) {
        issues.push(type_issue(["code", "expected", "received"], "boolean", zod_type(Some(v)), json!(["allowInvite"])));
    }
    issues
}

/// Desktop's `matchesIdentifier`: two identifiers that are the same without case.
fn same(a: Option<&str>, b: Option<&String>) -> bool {
    a.zip(b).is_some_and(|(a, b)| !a.is_empty() && a.to_lowercase() == b.to_lowercase())
}

/// Desktop's `findExistingSingleChatID`: the direct chat of the account whose only other member, who did not leave,
/// has an identifier of the person.
fn existing_dm(st: &BeeperStore, account: &str, p: &Person) -> Option<String> {
    let mut dms = st.chats.iter().filter(|c| c.account_id == account && view::chat_type(c) == "single");
    let found = dms.find(|c| {
        let mut others = c.participants.items.iter().filter(|u| u.is_self != Some(true) && u.has_exited != Some(true));
        let (Some(u), None) = (others.next(), others.next()) else { return false };
        same(Some(&u.id), p.id.as_ref())
            || same(u.mxid.as_deref(), p.id.as_ref())
            || same(u.phone_number.as_deref().and_then(phone_key).as_deref(), p.phone.as_ref())
            || same(u.email.as_deref().and_then(email_key).as_deref(), p.email.as_ref())
            || same(u.username.as_deref().and_then(username_key).as_deref(), p.username.as_ref())
    });
    found.map(|c| c.id.clone())
}

/// Desktop's `CreateChatOutput`: the chat like `getChat`, then the deprecated `chatID` and, from `startChat`,
/// `status`.
fn answer(store: &SharedStore, id: &str, status: Option<&str>) -> Reply {
    let st = store.read();
    let c = &st.chats[st.resolve_chat_idx(id).ok_or_else(|| missing("Chat not found"))?];
    let mut chat = serde_json::to_value(view::chat(&st, c, Some(100), false)).expect("serialize a chat");
    chat["chatID"] = c.id.as_str().into();
    if let Some(status) = status {
        chat["status"] = status.into();
    }
    ok(&chat)
}

/// Desktop's `getCreatedChat` of a new room: the room once the sync brings it, which Desktop's `createThread` gives
/// with its first message. It waits like Desktop's `waitForCreatedSingleChatID`, then fails like `getChat`.
fn created(store: &SharedStore, room: &str, text: Option<String>, status: Option<&str>) -> Reply {
    let until = Instant::now() + Duration::from_secs(5);
    while store.read().resolve_chat_idx(room).is_none() {
        if Instant::now() > until {
            return Err(missing("Chat not found"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
        store.write().send(room, Draft { text: Some(text), ..Draft::default() }).map_err(store_error)?;
    }
    answer(store, room, status)
}

/// Desktop's `createChat`: a new direct chat or group of the account with the participants, which the bridge makes
/// (Desktop's `createThread`). Desktop names only groups, and answers a failure of the bridge as a failed tool.
fn create_chat(store: &SharedStore, r: &Req) -> Reply {
    let issues = create_chat_issues(&r.body);
    if !issues.is_empty() {
        return Err(invalid_input(issues));
    }
    let account = r.str("accountID").expect("a string accountID");
    let participants: Vec<String> = r.parse("participantIDs")?;
    let group = r.str("type") == Some("group");
    let (title, text) = (r.owned("title").filter(|_| group), r.owned("messageText"));
    if store.read().matrix.is_none() {
        let kind = if group { "group" } else { "single" };
        let made = store.write().create_or_start_chat(account, kind, title, participants, text, None, None);
        return answer(store, &made.0.id, None);
    }
    let (bridge, rt) = (ChatBridge::of(store, account), tokio::runtime::Handle::current());
    let made = match group {
        true => rt.block_on(bridge.create_group("group".into(), title.unwrap_or_default(), participants)),
        false => rt.block_on(bridge.create_dm(participants[0].clone())),
    };
    let room = made.map_err(|_| error_with(StatusCode(500), "Failed to execute tool: createChat", "TOOL_EXECUTION_ERROR", json!({})))?;
    created(store, &room, text, None)
}

/// Desktop's `mapStartChatError` of a failed `createThread`.
fn start_error(e: &str) -> Response {
    let t = e.to_lowercase();
    let (status, message, code) = if t.contains("invite") {
        (409, "Chat creation requires sending an invite", "INVITE_REQUIRED")
    } else if t.contains("message") && t.contains("require") {
        (400, "A message is required to start this chat", "MESSAGE_REQUIRED")
    } else if t.contains("cannot") && t.contains("message") {
        (403, "Cannot message this user on the selected account", "CANNOT_MESSAGE_USER")
    } else if t.contains("not found") || t.contains("unknown user") {
        (404, "User not found", "USER_NOT_FOUND")
    } else {
        (500, e, "CREATE_CHAT_FAILED")
    };
    error(StatusCode(status), message, code)
}

/// Desktop's `startChat`: the direct chat of the account with a person, which the bridge makes from the first
/// identifier that it takes (an ID, a phone number, an email, then a username) unless the account has the chat.
fn start_chat(store: &SharedStore, r: &Req) -> Reply {
    let issues = start_chat_issues(&r.body);
    if !issues.is_empty() {
        return Err(invalid_input(issues));
    }
    let account = r.str("accountID").expect("a string accountID");
    let user = &r.body["user"];
    let field = |k: &str| user[k].as_str();
    let p = Person {
        id: field("id").map(search::js_trim).filter(|s| !s.is_empty()).map(str::to_string),
        phone: field("phoneNumber").and_then(phone_key),
        email: field("email").and_then(email_key),
        username: field("username").and_then(username_key),
    };
    let real = store.read().matrix.is_some();
    let l = if real { Lookups::of(store, account) } else { Lookups::DEMO };
    let ids = [(l.create_dm, &p.id), (l.phone, &p.phone), (l.email, &p.email), (l.username, &p.username)];
    let first = ids
        .into_iter()
        .find_map(|(on, id)| id.as_ref().filter(|_| on))
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "No supported identifiers for this account", "UNSUPPORTED_IDENTIFIER"))?;
    let existing = existing_dm(&store.read(), account, &p);
    if let Some(id) = existing {
        return answer(store, &id, Some("existing"));
    }
    let text = r.owned("messageText");
    if !real {
        let name = field("fullName").map(str::to_string);
        let mut st = store.write();
        let id = st.create_or_start_chat(account, "single", name, vec![first.clone()], text, None, None).0.id;
        // The person of the demo chat has the identifiers of the request, which find the chat again.
        let idx = st.resolve_chat_idx(&id).expect("the new chat");
        st.update_chat(idx, |c| {
            let u = &mut c.participants.items[0];
            (u.phone_number, u.email) = (p.phone, p.email);
        });
        drop(st);
        return answer(store, &id, Some("created"));
    }
    let made = ChatBridge::of(store, account).create_dm(first.clone());
    let room = tokio::runtime::Handle::current().block_on(made).map_err(|e| start_error(&e))?;
    created(store, &room, text, Some("created"))
}

/// Desktop's `Oi`: an email address.
fn is_email(s: &str) -> bool {
    let plain = |p: &str| !p.is_empty() && !p.chars().any(|c| c == '@' || c.is_whitespace());
    let Some((local, domain)) = s.split_once('@') else { return false };
    plain(local) && plain(domain) && domain.char_indices().any(|(i, c)| c == '.' && i > 0 && i + 1 < domain.len())
}

/// Desktop's `hp`: a query that Desktop looks up as an identifier. That is a phone number that libphonenumber takes
/// without a region, which is a `+` and 8 to 15 digits here, a text with an `@` after its start, or an `@` handle.
fn identifier_like(q: &str) -> bool {
    let digits = q.chars().filter(char::is_ascii_digit).count();
    let phone = q.starts_with('+') && (8..=15).contains(&digits) && q.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c));
    let handle = q.as_bytes().windows(2).any(|w| w[0] == b'@' && w[1].is_ascii_alphanumeric());
    phone || (q.contains('@') && !q.starts_with('@')) || handle
}

/// Desktop's `getExactLookupUsers`: the identifiers of a query that the bridge looks up, in Desktop's order.
fn exact_lookups(l: &Lookups, q: &str) -> Vec<String> {
    if !identifier_like(q) {
        return Vec::new();
    }
    let mut ids = Vec::new();
    ids.extend(phone_key(q).filter(|_| l.phone && q.chars().filter(char::is_ascii_digit).count() >= 7));
    ids.extend(email_key(q).filter(|_| l.email && is_email(q)));
    let username = username_key(q).filter(|_| l.username);
    let by_id = l.create_dm && username.is_none();
    ids.extend(username);
    if by_id {
        ids.push(q.to_string());
    }
    ids
}

/// The users that the bridge of an account finds for a query, with Desktop's scores: its `searchUsers` (200) and
/// the users of the exact lookups (300). A failed call finds no one, like Desktop's.
fn bridge_finds(store: &SharedStore, account: &str, q: &str) -> Vec<(User, i64)> {
    let (rt, l, bridge) = (tokio::runtime::Handle::current(), Lookups::of(store, account), ChatBridge::of(store, account));
    let lookups: Vec<_> = exact_lookups(&l, q).into_iter().map(|id| rt.spawn(bridge.clone().resolve_identifier(id))).collect();
    rt.block_on(async {
        let mut found: Vec<(User, i64)> = bridge.search_users(q.into()).await.unwrap_or_default().into_iter().map(|u| (u, 200)).collect();
        for lookup in lookups {
            if let Ok(Some(v)) = lookup.await.expect("a lookup task") {
                found.push((bridge_user(&v), 300));
            }
        }
        found
    })
}

/// Desktop's `normalizeUser` of a contact (`remoteContactToUser`) or a found user: its identifiers and profile in
/// Desktop's key order, or `None` without an identifier.
fn contact(u: &User) -> Option<Map<String, Value>> {
    let filled = |s: Option<&str>| s.map(search::js_trim).filter(|s| !s.is_empty()).map(str::to_string);
    let (phone, email) = (u.phone_number.as_deref().and_then(phone_key), u.email.as_deref().and_then(email_key));
    let username = u.username.as_deref().and_then(handle).map(str::to_string);
    let id = filled(Some(&u.id)).or_else(|| filled(u.mxid.as_deref()));
    let id = id.or_else(|| phone.clone()).or_else(|| email.clone()).or_else(|| username.clone())?;
    let image = u.img_url.clone().filter(|s| !search::js_trim(s).is_empty());
    let fields = [
        ("id", Some(id)),
        ("username", username),
        ("phoneNumber", phone),
        ("email", email),
        ("fullName", filled(u.full_name.as_deref())),
        ("imgURL", image),
    ];
    Some(fields.into_iter().filter_map(|(k, v)| Some((k.to_string(), Value::from(v?)))).collect())
}

/// Desktop's `scoreUserForQuery`: the score of a contact from the `base` of its source, with 300 more for a field that
/// is the query, 200 for one that starts with it and 100 for one that has it, or `None` when none has it.
fn score(c: &Map<String, Value>, q: &str, base: i64) -> Option<i64> {
    if q.is_empty() {
        return Some(base);
    }
    let q = q.to_lowercase();
    let fields = ["id", "fullName", "username", "email", "phoneNumber"].map(|k| c.get(k).and_then(Value::as_str));
    let fields: Vec<String> = fields.into_iter().flatten().map(str::to_lowercase).collect();
    let tests: [&dyn Fn(&String) -> bool; 3] = [&|f| *f == q, &|f| f.starts_with(&q), &|f| f.contains(&q)];
    let hit = tests.iter().position(|t| fields.iter().any(t))?;
    Some(base + 300 - 100 * hit as i64)
}

/// Desktop's `mergeCandidates`: each contact once by its ID, with its best score and the fields of its other sources
/// that the best one lacks, by score and then by name.
fn merge<'a>(candidates: impl Iterator<Item = (&'a User, i64)>, q: &str) -> Vec<Map<String, Value>> {
    let (mut merged, mut at) = (Vec::<(Map<String, Value>, i64)>::new(), HashMap::new());
    for (u, base) in candidates {
        let Some(c) = contact(u) else { continue };
        let Some(score) = score(&c, q, base) else { continue };
        let id = c["id"].as_str().expect("a contact ID").to_string();
        let Some(&i) = at.get(&id) else {
            at.insert(id, merged.len());
            merged.push((c, score));
            continue;
        };
        let (kept, best) = &mut merged[i];
        let (mut winner, loser) = if score > *best { (c, std::mem::take(kept)) } else { (std::mem::take(kept), c) };
        for (k, v) in loser {
            winner.entry(k).or_insert(v);
        }
        (*kept, *best) = (winner, (*best).max(score));
    }
    let name = |c: &Map<String, Value>| {
        let names = ["fullName", "username", "email", "phoneNumber", "id"].map(|k| c.get(k).and_then(Value::as_str));
        names.into_iter().flatten().next().unwrap_or_default().to_lowercase()
    };
    merged.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| locale_cmp(&name(a), &name(b))));
    merged.into_iter().map(|(c, _)| c).collect()
}

/// Desktop's `searchContacts` and `listContacts` (`resolveMergedContactsForAccount`): the people of the account's
/// chats, which stand in for the bridge's contact list, and for a query also the users that the bridge finds, merged
/// and ranked like Desktop. `listContacts` pages the people by contact ID without a query, and gives the first
/// `limit` contacts with one.
fn contacts(store: &SharedStore, id: &str, rest: &[&str], r: &Req) -> Reply {
    if store.read().accounts.iter().all(|a| a.account_id != id) {
        return Err(missing("Account not found"));
    }
    let list = rest == ["list"];
    let mut issues = Vec::new();
    let (cursor, after, limit) = match list {
        true => (
            check(&mut issues, r.string("cursor")).flatten(),
            check(&mut issues, r.direction()).unwrap_or_default(),
            r.page_size(50, MIN_1, 200).unwrap_or_else(|found| {
                issues.extend(found);
                0
            }),
        ),
        false => (None, false, 0),
    };
    let query = match check(&mut issues, r.string("query")) {
        Some(None) if !list => {
            issues.push(type_issue(["code", "expected", "received"], "string", "undefined", json!(["query"])));
            None
        }
        Some(Some("")) => {
            issues.push(too_short("query"));
            None
        }
        q => q.flatten(),
    };
    if !issues.is_empty() {
        return Err(invalid_input(issues));
    }
    let q = search::js_trim(query.unwrap_or_default());
    let real = store.read().matrix.is_some();
    let found = if real && !q.is_empty() { bridge_finds(store, id, q) } else { Vec::new() };
    let st = store.read();
    let people = st.chats.iter().filter(|c| c.account_id == id).flat_map(|c| &c.participants.items);
    let people = people.filter(|p| p.is_self != Some(true)).map(|u| (u, 100));
    let all = merge(people.chain(found.iter().map(|(u, s)| (u, *s))), q);
    if !list {
        return ok(&json!({ "items": all }));
    }
    if !q.is_empty() {
        let items: Vec<_> = all.into_iter().take(limit).collect();
        return ok(&CursorPage { items, has_more: false, oldest_cursor: None, newest_cursor: None });
    }
    // A cursor is the ID of a contact: `before` pages to the contacts after it, `after` to the ones before it.
    let at = cursor.map(|c| all.iter().position(|u| u["id"] == c).ok_or_else(|| bad("invalid cursor")));
    let (start, end) = match (at.transpose()?, after) {
        (None, _) => (0, limit.min(all.len())),
        (Some(i), false) => (i + 1, (i + 1 + limit).min(all.len())),
        (Some(i), true) => (i.saturating_sub(limit), i),
    };
    let has_more = if after { start > 0 } else { end < all.len() };
    let items = all[start..end].to_vec();
    let cursor_of = |u: Option<&Map<String, Value>>| u.and_then(|u| u["id"].as_str()).map(str::to_string);
    let (oldest_cursor, newest_cursor) = (cursor_of(items.last()), cursor_of(items.first()));
    ok(&CursorPage { items, has_more, oldest_cursor, newest_cursor })
}
