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

/// The `/messages` reply of the stub room, which has 70 messages: 30 back from the token `tN` of message N, or from
/// the newest message.
pub fn stub_page(query: &str) -> Value {
    let from = query.split('&').find_map(|kv| kv.strip_prefix("from=t"));
    let to: i64 = from.map_or(70, |t| t.parse().expect("a stub token"));
    let end = (to - 30).max(0);
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
