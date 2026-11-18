//! Desktop's chat creation and contact search on a signed-in account, against a stub homeserver with a stub WhatsApp
//! bridge, which record the requests that reach them. No test contacts a real homeserver or bridge.

use beeper_api::server::serve_http;
use beeper_core::models::{BridgeAccountInfo, BridgeMode};
use beeper_core::store::SharedStore;
use beeper_core::testing::{stub_store, stub_sync, sync_stub_rooms, temp_dir};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BRIDGE: &str = "/_matrix/client/unstable/com.beeper.bridge/whatsapp/_matrix/provision/v3/";

/// The joined room of a new chat with Sam, in the `/sync` reply of the stub homeserver.
fn room(id: &str) -> Value {
    let tag = &id[1..2];
    let state = |n: u8, kind: &str, key: &str, content: Value| {
        json!({ "type": kind, "state_key": key, "sender": "@me:x", "event_id": format!("${tag}{n}"),
            "origin_server_ts": 1, "content": content })
    };
    let events = [
        state(1, "m.room.create", "", json!({ "creator": "@me:x", "room_version": "10" })),
        state(2, "m.room.member", "@me:x", json!({ "membership": "join" })),
        state(3, "m.room.member", "@whatsapp_sam:x", json!({ "membership": "join", "displayname": "Sam Lee" })),
    ];
    json!({ "state": { "events": events }, "timeline": { "events": [] } })
}

/// The stub homeserver with the WhatsApp bridge, which knows Sam by the phone number `+15550100`. A new room joins
/// the next sync, which has a new token. The log has the bridge requests and the `createRoom` bodies.
async fn homeserver(log: Arc<Mutex<Vec<String>>>) -> String {
    let rooms = Arc::new(Mutex::new(Vec::<String>::new()));
    let sam =
        json!({ "id": "15550100", "name": "Sam Lee", "mxid": "@whatsapp_sam:x", "identifiers": ["tel:+15550100"] });
    beeper_core::bridge_stub::serve(move |method, path, body| {
        let path = path.split_once('?').map_or(path, |p| p.0);
        let made = |room: &str| rooms.lock().unwrap().push(room.into());
        let call = path.strip_prefix(BRIDGE);
        if let Some(call) = call {
            log.lock().unwrap().push(format!("{method} {call}"));
        }
        match (path, call) {
            ("/_matrix/client/versions", _) => (200, json!({ "versions": ["v1.11"] })),
            ("/_matrix/client/v3/keys/upload", _) => (200, json!({ "one_time_key_counts": {} })),
            ("/_matrix/client/v3/sync", _) => {
                let mut sync = stub_sync();
                let rooms = rooms.lock().unwrap();
                for r in rooms.iter() {
                    sync["rooms"]["join"][r] = room(r);
                }
                sync["next_batch"] = format!("s{}", rooms.len() + 1).into();
                (200, sync)
            }
            ("/_matrix/client/v3/createRoom", _) => {
                log.lock().unwrap().push(format!("createRoom {body}"));
                made("!group:x");
                (200, json!({ "room_id": "!group:x" }))
            }
            (_, Some("capabilities")) => {
                (200, json!({ "resolve_identifier": { "create_dm": true, "lookup_phone": true, "search": true } }))
            }
            (_, Some("search_users")) => (200, json!({ "results": [sam] })),
            (_, Some("resolve_identifier/%2B15550100")) => (200, sam.clone()),
            (_, Some("create_dm/%2B15550100")) => {
                made("!dm:x");
                (200, json!({ "dm_room_mxid": "!dm:x" }))
            }
            (_, Some(_)) => (404, json!({ "errcode": "M_NOT_FOUND", "error": "User not found on WhatsApp" })),
            _ => (404, json!({ "errcode": "M_UNRECOGNIZED", "error": "not in the stub" })),
        }
    })
    .await
}

/// A signed-in store with a WhatsApp login, its server, and a task that syncs the store like the sync loop.
async fn server(name: &str, log: Arc<Mutex<Vec<String>>>) -> (u16, tokio::task::JoinHandle<()>) {
    let store = stub_store(&temp_dir(name), &homeserver(log).await).await;
    let whatsapp = BridgeAccountInfo {
        account_id: "whatsapp".into(),
        bridge_id: "whatsapp".into(),
        bridge_type: "whatsapp".into(),
        is_running: true,
        login_id: "me".into(),
        mode: BridgeMode::Cloud,
        network_name: "WhatsApp".into(),
        state: "CONNECTED".into(),
        remote_name: None,
        remote_profile: None,
        remote_error: None,
        remote_error_code: None,
    };
    store.write().bridge_accounts.push(whatsapp);
    store.write().rebuild_accounts_from_bridges();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_http(listener, store.clone()));
    let syncing = tokio::spawn(sync_loop(store));
    (port, syncing)
}

async fn sync_loop(store: SharedStore) {
    loop {
        sync_stub_rooms(&store).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// One request with the token of the stub store and a JSON body, and the status and body of the reply.
async fn call(port: u16, method: &str, path: &str, body: Value) -> (u16, Value) {
    // Desktop's body parser refuses a JSON body that is not an object or an array, like `null`.
    let body = if body.is_null() { String::new() } else { body.to_string() };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer token\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    (head[9..12].parse().unwrap(), serde_json::from_str(body).unwrap())
}

/// Desktop's `startChat` asks the bridge for a direct chat with the first identifier that its capabilities take,
/// answers the synced chat, and maps the bridge's errors. `createChat` makes a Matrix group with `createRoom`, and a
/// failed bridge call fails the tool.
#[tokio::test(flavor = "multi_thread")]
async fn chats_are_made_by_the_bridge_like_desktop() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (port, syncing) = server("api-new-chat", log.clone()).await;
    let take = || std::mem::take(&mut *log.lock().unwrap());

    let sam = json!({ "accountID": "whatsapp", "user": { "email": "sam@x.com", "phoneNumber": "+1 555-0100" } });
    let (status, chat) = call(port, "POST", "/v1/chats/start", sam).await;
    let want = (200, &json!("!dm:x"), &json!("!dm:x"), &json!("created"));
    assert_eq!((status, &chat["id"], &chat["chatID"], &chat["status"]), want, "{chat}");
    assert_eq!(take(), ["GET capabilities", "POST create_dm/%2B15550100"], "the phone number, which WhatsApp takes");

    let email = json!({ "accountID": "whatsapp", "user": { "email": "sam@x.com" } });
    let unsupported =
        json!({ "message": "No supported identifiers for this account", "code": "UNSUPPORTED_IDENTIFIER" });
    assert_eq!(call(port, "POST", "/v1/chats/start", email).await, (400, unsupported));
    let nobody = json!({ "accountID": "whatsapp", "user": { "phoneNumber": "+15550199" }, "mode": "start" });
    let not_found = json!({ "message": "User not found", "code": "USER_NOT_FOUND" });
    assert_eq!(call(port, "POST", "/v1/chats", nobody).await, (404, not_found));
    take();

    let single = json!({ "accountID": "whatsapp", "type": "single", "participantIDs": ["+15550199"] });
    let failed =
        json!({ "message": "Failed to execute tool: createChat", "code": "TOOL_EXECUTION_ERROR", "details": {} });
    assert_eq!(call(port, "POST", "/v1/chats", single).await, (500, failed));
    assert_eq!(take(), ["POST create_dm/%2B15550199"]);
    let group = json!({ "accountID": "hungryserv", "type": "group", "participantIDs": ["@sam:x"], "title": "Team" });
    let (status, chat) = call(port, "POST", "/v1/chats", group).await;
    assert_eq!(
        (status, &chat["id"], &chat["chatID"], chat.get("status")),
        (200, &json!("!group:x"), &json!("!group:x"), None)
    );
    let encryption =
        json!({ "type": "m.room.encryption", "state_key": "", "content": { "algorithm": "m.megolm.v1.aes-sha2" } });
    let room = json!({ "invite": ["@sam:x"], "preset": "private_chat", "initial_state": [encryption], "name": "Team" });
    assert_eq!(take(), [format!("createRoom {room}")], "a Matrix group of Desktop's SDK");
    syncing.abort();
}

/// Desktop's `searchContacts` adds the users of the bridge's `searchUsers` and of the exact lookup of a phone number,
/// each once with the fields of all its sources, to the people of the account.
#[tokio::test(flavor = "multi_thread")]
async fn contacts_ask_the_bridge_like_desktop() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (port, syncing) = server("api-contacts", log.clone()).await;
    let (status, found) = call(port, "GET", "/v1/accounts/whatsapp/contacts?query=%2B15550100", Value::Null).await;
    let sam = json!({ "id": "@whatsapp_sam:x", "phoneNumber": "+15550100", "fullName": "Sam Lee" });
    assert_eq!((status, found), (200, json!({ "items": [sam] })));
    let mut asked = std::mem::take(&mut *log.lock().unwrap());
    asked.sort();
    let lookup = "GET resolve_identifier/%2B15550100";
    assert_eq!(asked, ["GET capabilities", lookup, lookup, "POST search_users"], "the phone number, then the ID");
    let (_, found) = call(port, "GET", "/v1/accounts/whatsapp/contacts?query=sam", Value::Null).await;
    assert_eq!(found, json!({ "items": [sam] }), "a name is no identifier");
    let asked = std::mem::take(&mut *log.lock().unwrap());
    assert_eq!(asked, ["GET capabilities", "POST search_users"]);
    syncing.abort();
}
