//! Fixtures for the tests of all crates: empty temporary folders and demo stores.
use crate::config::CentralConfig;
use crate::store::{BeeperStore, SharedStore, StoreLock};
use serde_json::{json, Value};
use std::path::PathBuf;

/// An empty folder for the test `name`. The path holds a hash of this checkout, so that tests in several checkouts
/// do not delete the folders of each other, and the next run of the test deletes the folder of the last run.
pub fn temp_dir(name: &str) -> PathBuf {
    let checkout = env!("CARGO_MANIFEST_DIR").bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b.into()));
    let dir = std::env::temp_dir().join(format!("beeper-rs-test-{checkout:x}")).join(name);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "delete the test folder {dir:?}: {e}");
    }
    std::fs::create_dir_all(&dir).expect("create the test folder");
    dir
}

/// The config of a demo store in `temp_dir(name)`.
pub fn demo_config(name: &str) -> CentralConfig {
    CentralConfig { demo: true, ..CentralConfig::new(temp_dir(name), "127.0.0.1".into(), 23390, None) }
}

/// A demo store with the seed chats in `temp_dir(name)`.
pub fn demo_store(name: &str) -> SharedStore {
    StoreLock::shared(BeeperStore::from_config(demo_config(name)))
}

/// A signed-in store in `dir` with a Matrix client that synced the stub homeserver at `url`.
pub async fn stub_store(dir: &std::path::Path, url: &str) -> crate::store::SharedStore {
    crate::cloud::install_tls();
    let cfg = crate::config::CentralConfig::new(dir.into(), "127.0.0.1".into(), 23392, None);
    let mut st = crate::store::BeeperStore::from_config(cfg.clone());
    if !st.signed_in() {
        let s = &mut st.beeper_state;
        (s.homeserver, s.user_id, s.device_id, s.access_token) =
            (url.into(), "@me:x".into(), "DEV".into(), "token".into());
        st.persist_state_to_sqlite();
        // A signed-in start adds the note chat of the user.
        st = crate::store::BeeperStore::from_config(cfg);
    }
    let store = crate::store::StoreLock::shared(st);
    let client = crate::matrix::connect(&store).await.expect("connect to the stub homeserver");
    client.sync_once(matrix_sdk::config::SyncSettings::default()).await.expect("sync with the stub homeserver");
    store.write().matrix = Some(client);
    store
}

/// The `/sync` reply of the stub homeserver: one joined room `!room:x` of the user `@me:x`.
pub fn stub_sync() -> Value {
    let state = |kind: &str, key: &str, content: Value| {
        json!({ "type": kind, "state_key": key, "sender": "@me:x", "event_id": format!("${kind}"),
            "origin_server_ts": 1, "content": content })
    };
    let create = state("m.room.create", "", json!({ "creator": "@me:x", "room_version": "10" }));
    let member = state("m.room.member", "@me:x", json!({ "membership": "join", "displayname": "Me" }));
    let room = json!({ "state": { "events": [create, member] }, "timeline": { "events": [] } });
    json!({ "next_batch": "s1", "rooms": { "join": { "!room:x": room } } })
}

/// Message `i` of the stub room, whose Beeper `hs.order` is `i`.
pub fn stub_message(i: i64) -> Value {
    json!({ "type": "m.room.message", "sender": "@me:x", "event_id": format!("$e{i}"),
        "origin_server_ts": 1_700_000_000_000 + i * 1000, "content": { "msgtype": "m.text", "body": format!("m{i}") },
        "unsigned": { "com.beeper.hs.order": i } })
}

/// The `/messages` reply of the stub room, which has 70 messages: `limit` (30 by default) back from the token `tN` of
/// message N, or from the newest message.
pub fn stub_page(query: &str) -> Value {
    let arg = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(k));
    let to: i64 = arg("from=t").map_or(70, |t| t.parse().expect("a stub token"));
    let limit: i64 = arg("limit=").map_or(30, |n| n.parse().expect("a limit"));
    let end = (to - limit).max(0);
    let mut page =
        json!({ "start": format!("t{to}"), "chunk": (end + 1..=to).rev().map(stub_message).collect::<Vec<_>>() });
    if end > 0 {
        page["end"] = json!(format!("t{end}"));
    }
    page
}

/// `stub_store` with the chat of the stub room, before any of its messages loaded.
pub async fn stub_room_store(dir: &std::path::Path, url: &str) -> SharedStore {
    let store = stub_store(dir, url).await;
    let room = store.read().matrix.clone().unwrap().get_room(matrix_sdk::ruma::room_id!("!room:x")).expect("the room");
    crate::matrix::sync_room(&store, &room, Vec::new()).await;
    store
}

/// One sync of the store's Matrix client with the stub homeserver, which puts the joined rooms into the store like
/// the sync loop.
pub async fn sync_stub_rooms(store: &SharedStore) {
    let client = store.read().matrix.clone().expect("a Matrix client");
    client.sync_once(matrix_sdk::config::SyncSettings::default()).await.expect("sync with the stub homeserver");
    for room in client.joined_rooms() {
        crate::matrix::sync_room(store, &room, Vec::new()).await;
    }
}

/// The stub homeserver of `stub_room_store`, whose room has the 70 messages of `stub_page`.
pub async fn stub_homeserver() -> String {
    crate::bridge_stub::serve(|_, path, _| {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        match path {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => (200, stub_page(query)),
            _ => (404, json!({ "errcode": "M_UNRECOGNIZED", "error": "not in the stub" })),
        }
    })
    .await
}

/// The chats of `large_store`, its messages, labels and merged chats, and the logins of each demo network.
pub const LARGE_CHATS: usize = 5_000;
pub const LARGE_MESSAGES: usize = 200_000;
pub const LARGE_LABELS: usize = 40;
pub const LARGE_MERGED: usize = 50;
pub const LARGE_LOGINS: usize = 5;
/// The chat of `large_store` with the most messages, one of every `LARGE_OPEN_EVERY`.
pub const LARGE_OPEN: &str = "!large_0:beeper.com";
const LARGE_OPEN_EVERY: usize = 66;

/// A demo store of a large account in `temp_dir(name)`: `LARGE_CHATS` chats on `LARGE_LOGINS` logins of each demo
/// network, `LARGE_MESSAGES` messages, `LARGE_LABELS` labels, and `LARGE_MERGED` merged chats of people with direct
/// chats on two or three accounts. Most people have chats on several accounts, so there are many merge suggestions.
pub fn large_store(name: &str) -> SharedStore {
    sized_store(name, LARGE_CHATS, LARGE_MESSAGES)
}

/// `large_store` with `chats` chats and `messages` messages, and fewer merged chats when the chats are few.
pub fn sized_store(name: &str, chats: usize, messages: usize) -> SharedStore {
    let mut st = BeeperStore::from_config(demo_config(name));
    let logins = st.bridge_accounts.clone().into_iter().flat_map(|b| {
        (1..LARGE_LOGINS).map(move |n| crate::models::BridgeAccountInfo {
            account_id: format!("{}_{n}", b.account_id),
            login_id: format!("{}_{n}", b.login_id),
            remote_name: Some(format!("Login {n}")),
            ..b.clone()
        })
    });
    let logins: Vec<_> = logins.collect();
    st.bridge_accounts.extend(logins);
    st.rebuild_accounts_from_bridges();
    st.persist_state_to_sqlite();
    let accounts: Vec<String> = st.bridge_accounts.iter().map(|b| b.account_id.clone()).collect();
    let (chats, messages) = large_data(&accounts, chats, messages);
    st.add_all(chats, messages);
    widen_large(&mut st);
    StoreLock::shared(st)
}

/// The state of `large_store` that SQLite does not keep, also after a restart: its labels, and a plan with labels
/// and merged chats without limits.
pub fn widen_large(st: &mut BeeperStore) {
    let all = &mut st.features.flags["features"]["*"];
    (all["labels-v2"], all["max-merge-chats"]) = (json!(true), json!(-1));
    let label = |l: usize| {
        let id = format!("lbl_large_{l}");
        let rooms = st.chats.iter().filter(|c| c.label_ids.contains(&id)).map(|c| c.id.clone()).collect();
        crate::models::Label { name: format!("Label {l}"), color: Some((l % 8) as u8), rooms, id, ..Default::default() }
    };
    let labels: Vec<_> = (0..LARGE_LABELS).map(label).collect();
    st.labels.retain(|l| !l.id.starts_with("lbl_large_"));
    st.labels.extend(labels);
    st.apply_union_limit();
}

/// The chats and messages of `sized_store` on the accounts `accounts`.
fn large_data(
    accounts: &[String],
    chats: usize,
    messages: usize,
) -> (Vec<crate::models::Chat>, Vec<crate::models::Message>) {
    let (count, people) = (chats, chats * 2 / 5);
    let name = |p: usize| format!("Person{p} Family{}", p % 97);
    let person = |p: usize, account: &str| {
        json!({ "id": format!("@person_{p}_{account}:beeper.com"), "fullName": name(p), "isSelf": false,
            "phoneNumber": format!("+1415{p:07}"), "email": format!("person{p}@example.com") })
    };
    let me = json!({ "id": "@alex:beeper.com", "fullName": "Alex", "isSelf": true });
    let base = 1_794_000_000_000_i64;
    let time = |n: usize| crate::config::rfc3339_from_millis(base + n as i64 * 1000);
    let chat = |i: usize| {
        let (account, p, group) = (&accounts[i % accounts.len()], i % people, i % 10 >= 7);
        let members: Vec<Value> = match group {
            true => (0..3 + i % 15).map(|k| person((p + k * 31) % people, account)).chain([me.clone()]).collect(),
            false => vec![person(p, account), me.clone()],
        };
        let labels: Vec<String> =
            i.is_multiple_of(3).then(|| format!("lbl_large_{}", i % LARGE_LABELS)).into_iter().collect();
        serde_json::from_value(json!({
            "id": format!("!large_{i}:beeper.com"), "accountID": account,
            "title": if group { format!("Group {i}") } else { name(p) }, "type": if group { "group" } else { "single" },
            "participants": { "hasMore": false, "total": members.len(), "items": members },
            "unreadCount": if i.is_multiple_of(5) { 1 + i % 7 } else { 0 }, "isArchived": i % 10 == 1, "isMuted": i % 10 == 2,
            "isLowPriority": i % 20 == 3, "isPinned": (1..12).contains(&i), "isRequest": i % 50 == 4,
            "labelIDs": labels, "lastActivity": time(0),
        }))
        .expect("a large chat")
    };
    let mut chats: Vec<crate::models::Chat> = (0..count).map(chat).collect();
    // Merged chat `m` joins the direct chats of person `10 m + 5` on two or three accounts.
    let unions: Vec<crate::models::Chat> = (0..LARGE_MERGED.min(people / 20))
        .map(|m| {
            let i = m * 10 + 5;
            let rooms = [i, i + people].into_iter().chain((m % 2 == 0).then_some(i + 2 * people));
            let rooms: Vec<&str> = rooms.map(|r| chats[r].id.as_str()).collect();
            serde_json::from_value(json!({
                "id": format!("!large_union_{m}:beeper.com"), "accountID": "hungryserv", "title": name(i),
                "type": "single", "participants": { "hasMore": false, "items": [me], "total": 1 }, "unreadCount": 0,
                "merge": { "chatIDs": rooms }, "createdAt": base + m as i64,
            }))
            .expect("a merged chat")
        })
        .collect();
    chats.extend(unions);
    let message = |n: usize| {
        let i = if n.is_multiple_of(LARGE_OPEN_EVERY) { 0 } else { 1 + n % (count - 1) };
        let c = &chats[i];
        let sender = &c.participants.items[n % (c.participants.items.len() - 1)];
        let mine = n.is_multiple_of(3);
        let text = match n % 11 {
            0 => format!("https://example.com/page/{n}"),
            1 => format!("**bold** message {n} with _formatting_"),
            _ => format!("message {n} in chat {i} about the plans for the week and other things"),
        };
        serde_json::from_value(json!({
            "id": format!("$large_{n}"), "accountID": c.account_id, "chatID": c.id,
            "senderID": if mine { "@alex:beeper.com" } else { &sender.id },
            "senderName": if mine { Some("Alex") } else { sender.full_name.as_deref() },
            "sortKey": crate::store::sort_key(10_000 + n as i64), "timestamp": time(n), "type": "TEXT", "text": text,
            "isSender": mine, "isUnread": false,
        }))
        .expect("a large message")
    };
    let messages = (0..messages).map(message).collect();
    (chats, messages)
}
