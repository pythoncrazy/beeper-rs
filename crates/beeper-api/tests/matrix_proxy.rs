//! Desktop's Matrix client-server proxy against a stub homeserver, which records the requests that reach it. No
//! test contacts a real homeserver.

use beeper_api::server::serve_http;
use beeper_core::store::SharedStore;
use beeper_core::testing::{demo_store, stub_store, stub_sync, temp_dir};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve(store: SharedStore) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_http(listener, store));
    port
}

/// One request with a token and an optional JSON body, and the status and body of the reply.
async fn call(port: u16, token: &str, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let body = body.map_or(String::new(), |b| format!("content-type: application/json\r\ncontent-length: {}\r\n\r\n{b}", b.len()));
    let body = if body.is_empty() { "\r\n".into() } else { body };
    let req = format!("{method} {path} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\nconnection: close\r\n{body}");
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    assert!(head.contains("\r\ncontent-type: application/json; charset=utf-8\r\n"), "{head}");
    (head[9..12].parse().unwrap(), body.to_string())
}

/// `call` with the reply as JSON.
async fn call_json(port: u16, token: &str, method: &str, path: &str, body: Option<&str>) -> (u16, Value) {
    let (status, body) = call(port, token, method, path, body).await;
    (status, serde_json::from_str(&body).unwrap())
}

fn matrix_error(errcode: &str, error: &str) -> Value {
    json!({ "errcode": errcode, "error": error })
}

/// Desktop's `requireAuth` of the proxy refuses with Matrix errors, a GET needs `read` and the others `write`, and the
/// routes answer 503 until the Matrix client starts.
#[tokio::test]
async fn the_proxy_needs_a_token_and_a_started_client() {
    let store = demo_store("matrix-proxy-auth");
    let token = store.read().beeper_state.access_token.clone();
    let read_only = store.write().connect.create(vec!["read".into()], 3600, None);
    let port = serve(store).await;
    let profile = "/_matrix/client/v3/profile/@a:x";
    let refused = matrix_error("M_UNKNOWN_TOKEN", "Unauthorized: Invalid or missing token");
    assert_eq!(call_json(port, "", "GET", profile, None).await, (401, refused));
    let write = "Insufficient permissions. Required scopes: write, missing: write";
    for (method, path) in [
        ("PUT", "/_matrix/client/v3/user/me/account_data/m.direct"),
        ("PUT", "/_matrix/client/v3/rooms/!r:x/account_data/m.tag"),
        ("POST", "/_matrix/client/v3/join/!r:x"),
        ("POST", "/_matrix/client/v3/rooms/!r:x/leave"),
        ("POST", "/_matrix/client/v3/createRoom"),
    ] {
        let reply = call_json(port, &read_only, method, path, Some("{}")).await;
        assert_eq!(reply, (403, matrix_error("M_FORBIDDEN", write)), "{method} {path}");
    }
    let not_ready = (503, matrix_error("M_UNKNOWN", "Matrix SDK is not ready"));
    for path in [
        profile,
        "/_matrix/client/v3/user/me/account_data/m.direct",
        "/_matrix/client/v3/user/me/rooms/!r:x/account_data/m.tag",
        "/_matrix/client/v3/rooms/!r:x/account_data/m.tag",
        "/_matrix/client/v3/rooms/!r:x/state",
        "/_matrix/client/v3/rooms/!r:x/state/m.room.member/@a:x",
        "/_matrix/client/v3/rooms/!r:x/event/$e",
    ] {
        assert_eq!(call_json(port, &read_only, "GET", path, None).await, not_ready, "{path}");
    }
    // Desktop checks the token and the client before it reads the body, even one over its 10 MB limit.
    let put = call_json(port, &token, "PUT", "/_matrix/client/v3/user/me/account_data/m.direct", Some("{")).await;
    assert_eq!(put, not_ready);
    for (t, status) in [("", "401"), (token.as_str(), "503")] {
        let head = format!("POST /_matrix/client/v3/createRoom HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {t}\r\ncontent-type: application/json\r\ncontent-length: 20000000\r\n\r\n");
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with(&format!("HTTP/1.1 {status} ")) && out.contains("\"errcode\":\"M_"), "{out}");
    }
    let other = call_json(port, &token, "DELETE", "/_matrix/client/v3/createRoom", None).await;
    assert_eq!(other, (404, json!({ "message": "Not found", "code": "not_found" })), "no route for other methods");
}

type Requests = Arc<Mutex<Vec<(String, String, Value)>>>;

/// A stub homeserver for `stub_store` that answers the requests of the proxy, and the requests that reach it, with
/// their decoded paths.
async fn homeserver() -> (String, Requests) {
    let requests = Requests::default();
    let seen = requests.clone();
    let url = beeper_core::bridge_stub::serve(move |method, path, body| {
        let path = beeper_core::codec::url_decode(path).unwrap();
        seen.lock().unwrap().push((method.into(), path.clone(), body));
        let (path, _) = path.split_once('?').unwrap_or((&path, ""));
        let member = json!({ "type": "m.room.member", "state_key": "@me:x", "sender": "@me:x", "event_id": "$m",
            "origin_server_ts": 1, "content": { "membership": "join", "displayname": "Me" } });
        let p = path.strip_prefix("/_matrix/client/v3").unwrap_or(path);
        match (method, p) {
            (_, "/_matrix/client/versions") => (200, json!({ "versions": ["v1.11"] })),
            ("GET", "/sync") => (200, stub_sync()),
            ("POST", "/keys/upload") => (200, json!({ "one_time_key_counts": {} })),
            ("GET", "/user/@me:x/account_data/m.direct") => (200, json!({ "@vic:x": ["!room:x"] })),
            ("GET", "/user/@me:x/rooms/!room:x/account_data/m.tag") => (200, json!({ "tags": { "u.work": {} } })),
            ("PUT", _) => (200, json!({})),
            ("GET", "/rooms/!room:x/state") => (200, json!([member])),
            ("GET", "/rooms/!room:x/state/m.room.member/@me:x") => (200, member),
            ("GET", "/rooms/!room:x/state/m.room.empty/") => (200, json!({ "type": "m.room.empty", "state_key": "" })),
            ("GET", "/rooms/!room:x/event/$e1") => (200, beeper_core::testing::stub_message(1)),
            ("GET", "/profile/@vic:x") => (200, json!({ "displayname": "Vic" })),
            ("GET", "/directory/room/#alias:x") => (200, json!({ "room_id": "!new:x", "servers": ["x"] })),
            ("POST", "/join/#alias:x") => (200, json!({ "room_id": "!new:x" })),
            ("POST", "/rooms/!other:x/join") => (200, json!({ "room_id": "!other:x" })),
            ("POST", "/rooms/!room:x/leave") => (200, json!({})),
            ("POST", "/createRoom") => (200, json!({ "room_id": "!made:x" })),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        }
    })
    .await;
    (url, requests)
}

/// The routes pass their requests to the homeserver through the Matrix client, and answer like Desktop: the content
/// as the homeserver sends it, `{}` for a change, the room of a join, and Matrix errors.
#[tokio::test(flavor = "multi_thread")]
async fn the_proxy_passes_requests_to_the_homeserver_like_desktop() {
    let (url, requests) = homeserver().await;
    let port = serve(stub_store(&temp_dir("matrix-proxy"), &url).await).await;
    requests.lock().unwrap().clear();
    let call = |method: &'static str, path: &str, body: Option<&'static str>| {
        let path = format!("/_matrix/client/v3{path}");
        async move { call_json(port, "token", method, &path, body).await }
    };
    let last = || requests.lock().unwrap().last().cloned().unwrap();

    assert_eq!(call("GET", "/user/me/account_data/m.direct", None).await, (200, json!({ "@vic:x": ["!room:x"] })));
    assert_eq!(call("GET", "/user/@me:x/account_data/m.direct", None).await.0, 200, "the user's own ID");
    let other = matrix_error("M_FORBIDDEN", "You can't access data for other users");
    assert_eq!(call("GET", "/user/@vic:x/account_data/m.direct", None).await, (403, other.clone()));
    assert_eq!(call("PUT", "/user/@vic:x/rooms/!room:x/account_data/m.tag", Some("{}")).await, (403, other));
    let missing = matrix_error("M_NOT_FOUND", "M_NOT_FOUND (HTTP 404): not in the stub");
    assert_eq!(call("GET", "/user/me/account_data/m.none", None).await, (404, missing.clone()));

    assert_eq!(call("PUT", "/user/me/account_data/com.x", Some(r#"{"b":1,"a":2}"#)).await, (200, json!({})));
    let (method, path, body) = last();
    assert_eq!((method.as_str(), path.as_str()), ("PUT", "/_matrix/client/v3/user/@me:x/account_data/com.x"));
    assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"b":1,"a":2}"#, "the body as it came");
    let not_json = (400, matrix_error("M_NOT_JSON", "Request body is not valid JSON"));
    assert_eq!(call("PUT", "/user/me/account_data/com.x", Some("{")).await, not_json);
    assert_eq!(call("PUT", "/rooms/%21room%3Ax/account_data/m.tag", None).await, (200, json!({})));
    assert_eq!(last().1, "/_matrix/client/v3/user/@me:x/rooms/!room:x/account_data/m.tag");
    assert_eq!(last().2, json!({}), "no body is {{}}");
    let tags = (200, json!({ "tags": { "u.work": {} } }));
    assert_eq!(call("GET", "/user/me/rooms/!room:x/account_data/m.tag", None).await, tags);
    assert_eq!(call("GET", "/rooms/!room:x/account_data/m.tag", None).await, tags);

    let (status, state) = call("GET", "/rooms/!room:x/state", None).await;
    assert_eq!((status, state[0]["type"].as_str()), (200, Some("m.room.member")));
    let content = json!({ "membership": "join", "displayname": "Me" });
    assert_eq!(call("GET", "/rooms/!room:x/state/m.room.member/%2540me%253Ax", None).await, (200, content));
    assert_eq!(last().1, "/_matrix/client/v3/rooms/!room:x/state/m.room.member/@me:x?format=event");
    let (_, event) = call("GET", "/rooms/!room:x/state/m.room.member/@me:x?format=event", None).await;
    assert_eq!(event["event_id"], "$m", "the whole event");
    let none = (404, matrix_error("M_NOT_FOUND", "Not found"));
    assert_eq!(call("GET", "/rooms/!room:x/state/m.room.empty", None).await, none, "an event without content");
    assert_eq!(call("GET", "/rooms/!room:x/state/m.room.topic/", None).await, (404, missing));

    let (status, event) = call("GET", "/rooms/!room:x/event/$e1", None).await;
    assert_eq!((status, &event["content"]["body"]), (200, &json!("m1")));
    let (status, unknown) = call("GET", "/rooms/!nope:x/event/$e1", None).await;
    assert_eq!((status, &unknown["errcode"]), (404, &json!("M_NOT_FOUND")), "{unknown}");
    assert_eq!(call("GET", "/profile/@vic:x", None).await, (200, json!({ "displayname": "Vic" })));

    let joined = (200, json!({ "room_id": "!new:x" }));
    assert_eq!(call("POST", "/join/%23alias%3Ax?server_name=x", None).await, joined);
    assert!(last().1.starts_with("/_matrix/client/v3/join/#alias:x?") && last().1.contains("server_name=x"), "{last:?}", last = last());
    let bad_json = (400, matrix_error("M_BAD_JSON", "Content could not be parsed as JSON"));
    assert_eq!(call("POST", "/join/!other:x", Some("[1]")).await, bad_json);
    assert_eq!(call("POST", "/join/!other:x", Some(r#"{"server_name":"y"}"#)).await, (200, json!({ "room_id": "!other:x" })));
    assert_eq!(call("POST", "/rooms/!room:x/leave", Some("{")).await, (200, json!({})), "leave reads no body");
    assert_eq!(last().1, "/_matrix/client/v3/rooms/!room:x/leave");

    let request = r#"{"name":"N","com.beeper.auto_join_invites":true}"#;
    assert_eq!(call("POST", "/createRoom", Some(request)).await, (200, json!({ "room_id": "!made:x" })));
    assert_eq!(serde_json::to_string(&last().2).unwrap(), request, "the request as it came");
}
