//! The HTTP/1.1 transport of the Desktop API server, with raw requests on a TCP connection.

use beeper_api::server::serve_http;
use beeper_core::config::CentralConfig;
use beeper_core::store::{BeeperStore, StoreLock};
use beeper_core::testing::demo_config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

const BAD_REQUEST: &str = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n";

/// Starts a demo server, and returns its port and access token.
async fn server(name: &str) -> (u16, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = CentralConfig { port, ..demo_config(&format!("http-{name}")) };
    let store = StoreLock::shared(BeeperStore::from_config(cfg));
    let token = store.read().beeper_state.access_token.clone();
    tokio::spawn(serve_http(listener, store));
    (port, token)
}

/// Sends `request`, and returns all the bytes that the server writes until it closes the connection.
async fn raw(port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(request).await.unwrap();
    let mut out = Vec::new();
    timeout(Duration::from_secs(10), stream.read_to_end(&mut out)).await.expect("the server closes").unwrap();
    out
}

async fn text(port: u16, request: &str) -> String {
    String::from_utf8(raw(port, request.as_bytes()).await).expect("a UTF-8 response")
}

fn post(path: &str, headers: &str, body: &str) -> String {
    format!("POST {path} HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\n{headers}\r\n{body}")
}

#[tokio::test]
async fn chunked_bodies_are_decoded_and_conflicts_are_refused() {
    let (port, _) = server("chunked").await;
    let info = "GET /v1/info HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n";
    let chunks = "4;ext=1\r\n{\"to\r\n9\r\nken\":\"x\"}\r\n0\r\ntrailer: 1\r\n\r\n";
    let out = text(port, &(post("/oauth/introspect", "transfer-encoding: chunked\r\n", chunks) + info)).await;
    let (first, second) = out.split_once("\r\n\r\n{\"active\":false}").expect("the decoded token is introspected");
    assert!(first.starts_with("HTTP/1.1 200 ") && second.contains("\"bundle_id\""), "{out}");

    for headers in [
        "transfer-encoding: chunked\r\ncontent-length: 4\r\n",
        "transfer-encoding: gzip\r\n",
        "content-length: 2\r\ncontent-length: 2\r\n",
        "content-length: +2\r\n",
        "content-length: abc\r\n",
    ] {
        assert_eq!(text(port, &post("/oauth/revoke", headers, "{}")).await, BAD_REQUEST, "{headers}");
    }
    let bad_size = post("/oauth/revoke", "transfer-encoding: chunked\r\n", "zz\r\n{}\r\n0\r\n\r\n");
    assert_eq!(text(port, &bad_size).await, BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_info_and_the_spec_answer_while_the_chats_load() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = CentralConfig { port, ..demo_config("http-loading") };
    let store = StoreLock::shared(BeeperStore::open(cfg));
    let token = store.read().beeper_state.access_token.clone();
    let server = serve_http(listener, store.clone());
    let (locked, load) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
    let loader = std::thread::spawn(move || {
        let mut st = store.write();
        locked.0.send(()).unwrap();
        load.1.recv().unwrap();
        st.load_chats();
    });
    locked.1.recv().unwrap();
    tokio::spawn(server);
    for path in ["/v1/info", "/v1/spec"] {
        let out = text(port, &format!("GET {path} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")).await;
        assert!(out.starts_with("HTTP/1.1 200 "), "{path}: {out}");
    }
    let chats =
        format!("GET /v1/chats HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\nconnection: close\r\n\r\n");
    let chats = tokio::spawn(async move { text(port, &chats).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!chats.is_finished(), "the chats wait for the load");
    load.0.send(()).unwrap();
    let out = chats.await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200 ") && out.contains("\"chatID\""), "{out}");
    loader.join().unwrap();
}

#[tokio::test]
async fn bodies_over_the_limit_of_the_route_get_413() {
    let (port, token) = server("limits").await;
    let too_large = "\r\n\r\n{\"message\":\"request entity too large\",\"code\":\"UNKNOWN_ERROR\"}";
    let auth = format!("authorization: Bearer {token}\r\n");
    for (path, ct, len) in [
        ("/oauth/token", "application/json", 64 * 1024 + 1),
        ("/v1/app/setup/start", "application/json", 64 * 1024 + 1),
        ("/v1/chats", "application/json", 10 * 1024 * 1024 + 1),
        ("/v1/assets/upload", "application/json", 100 * 1024 * 1024 + 1),
        ("/v1/assets/upload", "multipart/form-data; boundary=b", 500 * 1024 * 1024 + 1),
    ] {
        let head =
            format!("POST {path} HTTP/1.1\r\nhost: x\r\n{auth}content-type: {ct}\r\ncontent-length: {len}\r\n\r\n");
        let out = text(port, &head).await;
        let closes = out.contains("\r\nConnection: close\r\n");
        assert!(out.starts_with("HTTP/1.1 413 ") && closes && out.ends_with(too_large), "{path}: {out}");
    }
    // Desktop checks the route and the token before it reads the body.
    for (path, refusal) in [("/v1/chats", "401 Unauthorized"), ("/v1/nope", "404 Not Found")] {
        let head = format!(
            "POST {path} HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: 20000000\r\n\r\n"
        );
        let out = text(port, &head).await;
        assert!(out.starts_with(&format!("HTTP/1.1 {refusal}\r\n")), "{path}: {out}");
    }
    let big_chunk = post("/oauth/revoke", "transfer-encoding: chunked\r\n", "10001\r\n");
    assert!(text(port, &big_chunk).await.starts_with("HTTP/1.1 413 "));
    let body = format!("{{\"token\":\"{}\"}}", "x".repeat(60_000));
    let out = text(
        port,
        &post("/oauth/introspect", &format!("content-length: {}\r\nconnection: close\r\n", body.len()), &body),
    )
    .await;
    assert!(out.ends_with("{\"active\":false}"), "a body under the limit is read: {out}");
}

#[tokio::test]
async fn websocket_frames_over_64_kb_close_with_1009() {
    let (port, token) = server("ws").await;
    let ready = br#"{"type":"ready","version":1,"chatIDs":[]}"#;
    for (len, close) in [(64 * 1024, false), (64 * 1024 + 1, true)] {
        let upgrade = format!("GET /v1/ws HTTP/1.1\r\nhost: x\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\nauthorization: Bearer {token}\r\n\r\n");
        let mut frame = vec![0x81, 0x80 | 127];
        frame.extend((len as u64).to_be_bytes());
        frame.extend([0; 4]);
        frame.extend(vec![b'x'; len]);
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(&[upgrade.into_bytes(), frame].concat()).await.unwrap();
        let mut out = Vec::new();
        // The server answers a frame that fits with an error frame and keeps the connection.
        let read = timeout(Duration::from_secs(2), stream.read_to_end(&mut out)).await;
        assert_eq!(read.is_ok(), close, "a {len} byte frame");
        let end = out.windows(4).position(|w| w == b"\r\n\r\n").expect("a handshake") + 4;
        assert!(out.starts_with(b"HTTP/1.1 101 "));
        assert!(out[end..].starts_with(&[&[0x81, ready.len() as u8][..], ready].concat()), "the ready frame first");
        assert_eq!(out.ends_with(&[0x88, 0x02, 0x03, 0xF1]), close, "a {len} byte frame");
    }
}

#[tokio::test]
async fn websocket_drops_events_that_repeat_within_250_ms() {
    use beeper_api::http::{read_ws_frame, write_ws_frame};
    let (port, token) = server("ws-repeat").await;
    let upgrade = format!("GET /v1/ws HTTP/1.1\r\nhost: x\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\nauthorization: Bearer {token}\r\n\r\n");
    let mut ws = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    ws.write_all(upgrade.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(ws.read_u8().await.unwrap());
    }
    let (_, ready) = timeout(Duration::from_secs(5), read_ws_frame(&mut ws, 1 << 20)).await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&ready).contains("\"ready\""));
    write_ws_frame(&mut ws, 0x1, br#"{"type":"subscriptions.set","chatIDs":["*"]}"#, true).await.unwrap();
    let mute = |on: bool| {
        let body = format!("{{\"isMuted\":{on}}}");
        let request = format!("PATCH /v1/chats/!wa_family:beeper.com HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        async move { text(port, &request).await }
    };
    let (_, ack) = timeout(Duration::from_secs(5), read_ws_frame(&mut ws, 1 << 20)).await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&ack).contains("subscriptions.updated"));
    mute(true).await;
    mute(true).await;
    // A chat event carries no chat, so the same event shows again after the window.
    tokio::time::sleep(Duration::from_millis(300)).await;
    mute(false).await;
    let mut frames = Vec::new();
    while let Ok(Ok((_, payload))) = timeout(Duration::from_millis(500), read_ws_frame(&mut ws, 1 << 20)).await {
        let v: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        frames.push((v["seq"].as_u64().unwrap(), v["type"].as_str().unwrap().to_string()));
    }
    let seqs: Vec<u64> = frames.iter().map(|f| f.0).collect();
    assert_eq!(seqs, (1..=frames.len() as u64).collect::<Vec<_>>(), "seq counts only the sent events");
    assert!(
        frames.len() == 2 && frames.iter().all(|f| f.1 == "chat.upserted"),
        "the repeated mute is dropped: {frames:?}"
    );
}

#[tokio::test]
async fn repeated_and_bracket_query_keys_are_arrays() {
    use beeper_api::codec::url_encode;
    use serde_json::{json, Value};
    let query = beeper_api::http::parse_query("a=1&a=2&b%5B%5D=3&b[]=4&c[]=5&d=x&e=%zz+1");
    assert_eq!(Value::Object(query), json!({ "a": ["1", "2"], "b": ["3", "4"], "c": ["5"], "d": "x", "e": "%zz 1" }));
    let (port, token) = server("query").await;
    let get = |target: String| {
        let request =
            format!("GET {target} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\nconnection: close\r\n\r\n");
        async move {
            let out = text(port, &request).await;
            let body: Value = serde_json::from_str(out.split_once("\r\n\r\n").unwrap().1).expect("a JSON body");
            body["items"]
                .as_array()
                .expect("items")
                .iter()
                .map(|i| (i["accountID"].clone(), i["chatID"].clone()))
                .collect::<Vec<_>>()
        }
    };
    let all = get("/v1/chats".into()).await;
    let mut accounts: Vec<&str> = all.iter().map(|(a, _)| a.as_str().unwrap()).collect();
    accounts.sort();
    accounts.dedup();
    assert!(accounts.len() >= 2, "the demo has chats on several accounts");
    let (a, b) = (url_encode(accounts[0]), url_encode(accounts[1]));
    let (a, b) = (a.as_str(), b.as_str());
    let both = all.iter().filter(|(x, _)| x == accounts[0] || x == accounts[1]).count();
    for form in [format!("accountIDs={a}&accountIDs={b}"), format!("accountIDs[]={a}&accountIDs[]={b}")] {
        assert_eq!(get(format!("/v1/chats?{form}")).await.len(), both, "{form}");
    }
    assert_eq!(
        get(format!("/v1/chats?accountIDs[]={a}")).await.len(),
        all.iter().filter(|(x, _)| x == accounts[0]).count()
    );

    let found = get("/v1/messages/search?limit=20".into()).await;
    let mut chats: Vec<&str> = found.iter().map(|(_, c)| c.as_str().unwrap()).collect();
    chats.sort();
    chats.dedup();
    let (c, d) = (url_encode(chats[0]), url_encode(chats[1]));
    let mut both: Vec<String> = get(format!("/v1/messages/search?limit=20&chatIDs={c}&chatIDs={d}"))
        .await
        .into_iter()
        .map(|(_, chat)| chat.as_str().unwrap().into())
        .collect();
    both.sort();
    both.dedup();
    assert_eq!(both, chats[..2], "the messages of both chats");
}

/// Desktop's `listChats`: `limit` rows past an opaque cursor in both directions, `hasMore` for a full page of rows,
/// `accountIDs`, and the zod issues of a bad query.
#[tokio::test]
async fn list_chats_pages_and_validates_like_desktop() {
    use serde_json::{json, Value};
    let (port, token) = server("list-chats").await;
    let get = |target: String| {
        let request =
            format!("GET {target} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\nconnection: close\r\n\r\n");
        async move {
            let out = text(port, &request).await;
            let (head, body) = out.split_once("\r\n\r\n").unwrap();
            (head[9..12].parse::<u16>().unwrap(), serde_json::from_str::<Value>(body).expect("a JSON body"))
        }
    };
    let page = |query: String| async move {
        let (status, body) = get(format!("/v1/chats?{query}")).await;
        assert_eq!(status, 200, "{query}: {body}");
        let ids: Vec<String> =
            body["items"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().into()).collect();
        let cursor = |k: &str| body[k].as_str().map(str::to_string);
        (ids, body["hasMore"].as_bool().unwrap(), cursor("oldestCursor"), cursor("newestCursor"))
    };
    let enc = beeper_api::codec::url_encode;

    let (all, more, ..) = page("limit=200".into()).await;
    assert!(all.len() > 6 && !more, "{} chats", all.len());
    assert_eq!(page(String::new()).await.0, all[..all.len().min(25)], "limit defaults to 25");
    let (two, more, _, newest) = page("limit=2".into()).await;
    assert_eq!((two, more), (all[..2].to_vec(), true));
    assert!(page(format!("limit={}", all.len())).await.1, "a full page of rows has more");

    let (mut seen, mut cursor) = (Vec::new(), None::<String>);
    loop {
        let query = cursor.as_ref().map_or("limit=2".into(), |c| format!("limit=2&cursor={}", enc(c)));
        let (ids, more, oldest, _) = page(query).await;
        seen.extend(ids);
        cursor = oldest;
        if !more {
            break;
        }
    }
    assert_eq!(seen, all, "paging `before` with the oldest cursor walks all the chats");

    let (_, _, third, _) = page("limit=3".into()).await;
    let third = third.unwrap();
    let (ms, _) = third.split_once(':').expect("a <timestamp>:<chat> cursor");
    assert_eq!(page(format!("limit=2&cursor={}", enc(&third))).await.0, all[3..5]);
    let (ids, more, _, newest_after) = page(format!("limit=2&direction=after&cursor={}", enc(&third))).await;
    assert_eq!((ids, more, newest_after), (all[..2].to_vec(), true, newest.clone()), "`after` takes the newest rows");
    let (ids, more, ..) = page(format!("limit=3&direction=after&cursor={}", enc(&third))).await;
    assert_eq!((ids, more), (all[..2].to_vec(), false));
    let same = |c: String| async move { page(format!("limit=200&cursor={}", enc(&c))).await };
    assert_eq!(same(format!("{third}|abc")).await, same(third.clone()).await, "the iMessage part is ignored");
    assert_eq!(same("|abc".into()).await, page("limit=200".into()).await, "a cursor without its Beeper part");

    let (older, ..) = page(format!("limit=200&cursor={ms}")).await;
    let (newer, ..) = page(format!("limit=200&direction=after&cursor={ms}")).await;
    assert!(all.ends_with(&older) && all.starts_with(&newer) && !older.contains(&all[2]) && !newer.contains(&all[2]));

    let empty = json!({ "items": [], "hasMore": false, "oldestCursor": "nope", "newestCursor": "nope" });
    assert_eq!(get("/v1/chats?cursor=nope".into()).await, (200, empty), "an unreadable cursor is an empty page");

    let account = get("/v1/chats?limit=200".into()).await.1["items"][0]["accountID"].as_str().unwrap().to_string();
    let (status, body) = get(format!("/v1/chats?limit=200&accountIDs[]={}", enc(&account))).await;
    let items = body["items"].as_array().unwrap();
    assert!(status == 200 && !items.is_empty() && items.iter().all(|c| c["accountID"] == account.as_str()));
    assert!(items.len() < all.len());
    assert_eq!(page("limit=200&accountIDs=nope".into()).await.0, Vec::<String>::new());
    assert_eq!(page("limit=200&accountIDs=%20".into()).await.0, all, "blank account IDs are dropped");

    let invalid = |issues: Value| json!({ "message": "Invalid input", "code": "VALIDATION_ERROR", "details": { "issues": issues } });
    let nan = json!({ "code": "invalid_type", "expected": "number", "received": "nan", "path": ["limit"], "message": "Expected number, received nan" });
    let float = json!({ "code": "invalid_type", "expected": "integer", "received": "float", "message": "Expected integer, received float", "path": ["limit"] });
    let bound = |code: &str, k: &str, v: i64, inclusive: bool, message: &str| json!({ "code": code, k: v, "type": "number", "inclusive": inclusive, "exact": false, "message": message, "path": ["limit"] });
    let small = bound("too_small", "minimum", 1, true, "Number must be greater than or equal to 1");
    let big = bound("too_big", "maximum", 200, true, "Number must be less than or equal to 200");
    let cases = [
        ("/v1/chats?limit=abc", invalid(json!([nan]))),
        ("/v1/chats?limit=0", invalid(json!([small]))),
        ("/v1/chats?limit=", invalid(json!([small]))),
        ("/v1/chats?limit=201", invalid(json!([big]))),
        ("/v1/chats?limit=1.5", invalid(json!([float]))),
        ("/v1/chats?limit=-Infinity", invalid(json!([float, small]))),
        (
            "/v1/chats?cursor=a&cursor=b&direction=up&limit=0",
            invalid(json!([
                { "code": "invalid_type", "expected": "string", "received": "array", "path": ["cursor"], "message": "Expected string, received array" },
                { "received": "up", "code": "invalid_enum_value", "options": ["after", "before"], "path": ["direction"], "message": "Invalid enum value. Expected 'after' | 'before', received 'up'" },
                small,
            ])),
        ),
        (
            "/v1/messages/search?limit=0",
            invalid(json!([bound("too_small", "minimum", 0, false, "Number must be greater than 0")])),
        ),
    ];
    for (target, want) in cases {
        let (status, body) = get(target.into()).await;
        assert_eq!((status, body.to_string()), (400, want.to_string()), "{target}");
    }
    assert_eq!(page("limit=%205%20".into()).await.0.len(), 5, "`Number` trims the limit");
}

/// The head of a response without the `Date` header, and the body.
fn without_date(out: &str) -> (String, &str) {
    let (head, body) = out.split_once("\r\n\r\n").expect("a response head");
    let date = head.lines().find_map(|l| l.strip_prefix("Date: ")).expect("a Date header");
    assert!(date.ends_with(" GMT") && date.len() == 29, "an IMF-fixdate: {date}");
    (head.lines().filter(|l| !l.starts_with("Date: ")).collect::<Vec<_>>().join("\r\n"), body)
}

#[tokio::test]
async fn cors_and_default_headers_match_desktop() {
    use beeper_api::http::{http_date, weak_etag};
    assert_eq!(http_date(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
    assert_eq!(
        weak_etag(b"{\"message\":\"Not found\",\"code\":\"not_found\"}"),
        "W/\"2a-ffPysxnJrBrRRKnu/lKR4M9M4Rc\""
    );
    let (port, _) = server("cors").await;
    let request = |method: &str, path: &str, headers: &str| {
        format!("{method} {path} HTTP/1.1\r\nhost: x\r\n{headers}connection: close\r\n\r\n")
    };

    let out = text(port, &request("OPTIONS", "/v1/accounts", "origin: http://example.com\r\n")).await;
    let preflight = "HTTP/1.1 204 No Content\r\nX-Powered-By: Express\r\nAccess-Control-Allow-Origin: http://example.com\r\nVary: Origin\r\nAccess-Control-Allow-Methods: GET,POST,PUT,PATCH,DELETE,OPTIONS\r\nAccess-Control-Allow-Headers: Authorization,Content-Type,MCP-Protocol-Version,X-Beeper-Access-Token,X-Stainless-MCP\r\nAccess-Control-Max-Age: 600\r\nAccess-Control-Expose-Headers: WWW-Authenticate,X-Beeper-Desktop-Version\r\nContent-Length: 0\r\nConnection: close";
    assert_eq!(without_date(&out), (preflight.into(), ""));

    let out = text(port, &request("GET", "/v1/info", "origin: https://example.com\r\n")).await;
    let (head, body) = without_date(&out);
    let version = beeper_core::config::APP_VERSION;
    let cors = format!("HTTP/1.1 200 OK\r\nX-Powered-By: Express\r\nAccess-Control-Allow-Origin: https://example.com\r\nVary: Origin\r\nAccess-Control-Expose-Headers: WWW-Authenticate,X-Beeper-Desktop-Version\r\nX-Beeper-Desktop-Version: {version}\r\n");
    let etag = format!("ETag: {}", weak_etag(body.as_bytes()));
    assert!(head.starts_with(&cors) && head.contains(&etag), "{head}");

    for (path, origin) in [("/v1/info", "chrome-extension://abc"), ("/focus", "http://example.com")] {
        let out = text(port, &request("OPTIONS", path, &format!("origin: {origin}\r\n"))).await;
        assert!(!out.contains("Access-Control") && out.contains("X-Beeper-Desktop-Version"), "{path} {origin}: {out}");
    }

    let out = text(port, &request("GET", "/v1/info", &format!("if-none-match: {}\r\n", &etag[6..]))).await;
    let (head, body) = without_date(&out);
    assert!(head.starts_with("HTTP/1.1 304 ") && !head.contains("Content-Length") && body.is_empty(), "{out}");
}

#[tokio::test]
async fn the_transport_behaves_like_node() {
    let (port, _) = server("node").await;
    let get = "GET /v1/info?x=%zz HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n";
    let out = text(port, get).await;
    let (head, body) = without_date(&out);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n") && head.ends_with("\r\nConnection: close"), "{head}");
    assert!(head.contains("\r\ncontent-type: application/json; charset=utf-8\r\n"), "{head}");
    let out = text(port, &get.replacen("GET", "HEAD", 1)).await;
    let (head_of_head, empty) = without_date(&out);
    assert_eq!((head_of_head, empty), (head.clone(), ""), "HEAD gives the head of GET");
    assert!(head.contains(&format!("\r\nContent-Length: {}\r\n", body.len())));

    let alive = "GET /v1/info HTTP/1.0\r\nconnection: keep-alive\r\n\r\n";
    let out = text(port, &format!("{alive}GET /v1/info HTTP/1.0\r\n\r\n")).await;
    let (first, second) = out.split_once("}HTTP/1.1 200 OK\r\n").expect("two responses");
    assert!(first.contains("\r\nConnection: keep-alive\r\nKeep-Alive: timeout=360\r\n"), "{first}");
    assert!(second.contains("\r\nConnection: close\r\n"), "HTTP/1.0 closes by default: {second}");

    for target in ["/nope", "//v1/info", "/V1/INFO", "http://x/v1/info"] {
        let out = text(port, &format!("GET {target} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")).await;
        assert_eq!(without_date(&out), ("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close".into(), ""));
    }

    for request in [
        "GARBAGE\r\n\r\n".to_string(),
        "GET /v1/info HTTP/1.1\r\n\r\n".into(),
        "GET /v1/info HTTP/1.1\r\nhost: x\r\nfoo\r\n\r\n".into(),
        "GET /v1/info HTTP/1.1\r\nhost x : y\r\n\r\n".into(),
        "GET /v1/info HTTP/1.1\nhost: x\n\n".into(),
        "GET /v1/info HTTPS/1.1\r\nhost: x\r\n\r\n".into(),
    ] {
        assert_eq!(text(port, &request).await, BAD_REQUEST, "{request:?}");
    }
    let big = format!("GET /v1/info HTTP/1.1\r\nhost: x\r\nx-big: {}\r\n\r\n", "a".repeat(16 * 1024));
    assert_eq!(text(port, &big).await, "HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let body = "{\"token\":\"x\"}";
    let expect = format!("expect: 100-continue\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
    stream.write_all(post("/oauth/introspect", &expect, "").as_bytes()).await.unwrap();
    let mut continued = [0; 25];
    stream.read_exact(&mut continued).await.unwrap();
    assert_eq!(&continued, b"HTTP/1.1 100 Continue\r\n\r\n");
    stream.write_all(body.as_bytes()).await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200 OK\r\n") && out.ends_with("{\"active\":false}"), "{out}");
}

#[tokio::test]
async fn routes_match_like_express() {
    let (port, token) = server("routes").await;
    let get = |method: &str, target: &str, headers: &str| {
        let request = format!("{method} {target} HTTP/1.1\r\nhost: x\r\n{headers}connection: close\r\n\r\n");
        async move { without_date(&text(port, &request).await).0 }
    };
    let auth = format!("authorization: Bearer {token}\r\n");
    let not_found = "\r\n\r\n{\"message\":\"Not found\",\"code\":\"not_found\"}";
    for (method, target) in [
        ("GET", "/v1/%69nfo"),
        ("GET", "/v1/info%2F"),
        ("GET", "/v1//info"),
        ("GET", "/v1/nope"),
        ("PUT", "/v1/accounts"),
        ("GET", "/v1/ws"),
    ] {
        let out = text(port, &format!("{method} {target} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")).await;
        assert!(out.starts_with("HTTP/1.1 404 ") && out.ends_with(not_found), "{method} {target}: {out}");
    }
    for target in ["/v1/info/", "/v1/INFO", "/v1/accounts/"] {
        assert!(get("GET", target, &auth).await.starts_with("HTTP/1.1 200 "), "{target}");
    }
    let out = text(port, &format!("GET /v1/chats/%zz HTTP/1.1\r\nhost: x\r\n{auth}connection: close\r\n\r\n")).await;
    assert!(out.ends_with("{\"message\":\"Failed to decode param '%zz'\",\"code\":\"UNKNOWN_ERROR\"}"), "{out}");
    let out = text(port, &format!("GET /v1/chats/a%2Fb HTTP/1.1\r\nhost: x\r\n{auth}connection: close\r\n\r\n")).await;
    assert!(
        out.ends_with("{\"message\":\"Chat not found\",\"code\":\"NOT_FOUND\"}"),
        "a parameter can hold a slash: {out}"
    );

    let redirect = "HTTP/1.1 302 Found\r\nX-Powered-By: Express\r\nX-Beeper-Desktop-Version: ";
    for (accept, kind, body) in [
        ("", Some("text/plain"), "Found. Redirecting to /v1/info"),
        ("accept: */*\r\n", Some("text/plain"), "Found. Redirecting to /v1/info"),
        ("accept: text/html,text/plain;q=0.9\r\n", Some("text/html"), "<p>Found. Redirecting to /v1/info</p>"),
        ("accept: application/json\r\n", None, ""),
    ] {
        let out = text(port, &format!("GET / HTTP/1.1\r\nhost: x\r\n{accept}connection: close\r\n\r\n")).await;
        let (head, got) = without_date(&out);
        let kind = kind.map_or(String::new(), |k| format!("\r\ncontent-type: {k}; charset=utf-8"));
        let tail = format!(
            "\r\nlocation: /v1/info\r\nvary: Accept{kind}\r\nContent-Length: {}\r\nConnection: close",
            body.len()
        );
        assert!(head.starts_with(redirect) && head.ends_with(&tail) && got == body, "{accept}: {out}");
    }

    let spec = |host: String| async move {
        let out = text(port, &format!("GET /v1/spec HTTP/1.1\r\nhost: {host}\r\nconnection: close\r\n\r\n")).await;
        let (head, body) = out.split_once("\r\n\r\n").expect("a response head");
        assert!(head.contains("\r\ncache-control: public, max-age=3600\r\n"), "{head}");
        assert!(body.starts_with("{\n  \"openapi\": \"3.1.0\",\n  \"info\": {\n    \"title\": \"Beeper Client API\",\n    \"version\": \"5.0.0\""));
        serde_json::from_str::<serde_json::Value>(body).expect("a JSON document")
    };
    for (host, base) in [
        (format!("127.0.0.1:{port}"), format!("http://127.0.0.1:{port}")),
        (format!("localhost:{port}"), format!("http://localhost:{port}")),
        ("example.com".into(), format!("http://127.0.0.1:{port}")),
    ] {
        let doc = spec(host).await;
        assert_eq!(doc["servers"][0]["url"], base.as_str());
        let flow = &doc["components"]["securitySchemes"]["oauth2"]["flows"]["authorizationCode"];
        assert_eq!(flow["tokenUrl"], format!("{base}/oauth/token"));
    }
    // The document lists the routes of the server and no other routes.
    let doc = spec("x".into()).await;
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for (method, op) in methods.as_object().unwrap() {
            let target = path.replace('{', "x").replace('}', "");
            let method = method.to_uppercase();
            let out = text(port, &format!("{method} {target} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")).await;
            assert!(!out.ends_with(not_found), "{method} {path}: {}", op["operationId"]);
        }
    }
    let count = doc["paths"].as_object().unwrap().values().map(|m| m.as_object().unwrap().len()).sum::<usize>();
    assert_eq!(count, 78, "the 67 operations of the routes and the 11 of the Matrix proxy");
    let tags: Vec<&str> = doc["tags"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(tags[tags.len() - 2..], ["Matrix", "OAuth"]);
}

#[tokio::test]
async fn assets_upload_and_serve_like_desktop() {
    use serde_json::{json, Value};
    let (port, token) = server("assets").await;
    let send = |method: &str, target: &str, headers: &str, body: &str| {
        let request = format!(
            "{method} {target} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        async move { String::from_utf8_lossy(&raw(port, request.as_bytes()).await).into_owned() }
    };
    let body = |out: &str| serde_json::from_str::<Value>(out.split_once("\r\n\r\n").unwrap().1).unwrap();
    let json = "content-type: application/json\r\n";
    // A PNG of 2 x 3 pixels without a name and a type.
    let png =
        beeper_api::codec::base64_encode_standard(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x03\x08\x02\0\0\0");
    let up = body(&send("POST", "/v1/assets/upload/base64", json, &json!({ "content": png }).to_string()).await);
    let keys: Vec<&str> = up.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["uploadID", "srcURL", "fileName", "mimeType", "fileSize", "width", "height"]);
    assert_eq!(
        (&up["fileName"], &up["mimeType"], &up["width"], &up["height"]),
        (&json!("file.png"), &json!("image/png"), &json!(2), &json!(3))
    );
    let src = up["srcURL"].as_str().unwrap();
    assert!(src.starts_with("file://") && src.ends_with("file.png"), "{src}");
    for name in [".env", "con", "LPT1", "a~b"] {
        let out =
            send("POST", "/v1/assets/upload/base64", json, &json!({ "content": png, "fileName": name }).to_string())
                .await;
        assert_eq!(body(&out)["code"], "INVALID_FILENAME", "{name}");
    }
    let multipart = "content-type: multipart/form-data; boundary=b\r\n";
    let field = "--b\r\ncontent-disposition: form-data; name=\"fileName\"\r\n\r\nx.txt\r\n--b--\r\n";
    assert_eq!(body(&send("POST", "/v1/assets/upload", multipart, field).await)["code"], "MISSING_FILE");
    let download = send("POST", "/v1/assets/download", json, &json!({ "url": src }).to_string()).await;
    // `DownloadAssetInput` refines the URL before the handler sees it.
    let refined = json!({ "code": "custom", "message": "URL must start with mxc:// or localmxc://", "path": ["url"] });
    assert_eq!(body(&download), serde_json::from_str::<serde_json::Value>(&invalid(json!([refined]))).unwrap());

    let serve = format!("/v1/assets/serve?url={}", beeper_api::codec::url_encode(src));
    let out = send("GET", &serve, "", "").await;
    let (head, _) = without_date(&out);
    let etag = head.lines().find_map(|l| l.strip_prefix("ETag: ")).expect("an ETag").to_string();
    assert!(etag.starts_with("\"29-") && head.contains("\r\nCache-Control: private, max-age=3600\r\n"), "{head}");
    assert!(
        head.contains("\r\nContent-Type: image/png\r\nAccept-Ranges: bytes\r\n") && head.contains("Last-Modified: ")
    );
    let fresh = send("GET", &serve, &format!("if-none-match: {etag}\r\n"), "").await;
    assert!(fresh.starts_with("HTTP/1.1 304 ") && fresh.ends_with("\r\n\r\n"), "{fresh}");
    let ranged = send("GET", &serve, "range: bytes=1-3,5-6\r\n", "").await;
    assert!(
        ranged.starts_with("HTTP/1.1 206 ")
            && ranged.contains("Content-Range: bytes 1-3/29\r\n")
            && ranged.ends_with("PNG")
    );
    let refused = send("GET", &serve, "range: bytes=40-50\r\n", "").await;
    assert!(
        refused.starts_with("HTTP/1.1 416 ")
            && refused.contains("Content-Range: bytes */29\r\n")
            && refused.ends_with("\r\n\r\n"),
        "{refused}"
    );

    let message = json!({ "text": "pic", "attachment": { "uploadID": up["uploadID"] } }).to_string();
    let sent = body(&send("POST", "/v1/chats/!wa_eric:beeper.com/messages", json, &message).await);
    let id = sent["pendingMessageID"].as_str().unwrap();
    let shown = body(&send("GET", &format!("/v1/chats/!wa_eric:beeper.com/messages/{id}"), "", "").await);
    let attachment = &shown["attachments"][0];
    assert!(
        attachment["id"].as_str().unwrap().starts_with("mxc://") && attachment["id"] == attachment["srcURL"],
        "{attachment}"
    );

    // The attachment `type` of the request marks stickers, voice notes and GIFs.
    let chat = "/v1/chats/!wa_eric:beeper.com/messages";
    let mut kinds = Vec::new();
    for kind in ["sticker", "voice-note", "gif"] {
        let message = json!({ "attachment": { "uploadID": up["uploadID"], "type": kind } }).to_string();
        let id = body(&send("POST", chat, json, &message).await)["pendingMessageID"].as_str().unwrap().to_string();
        let shown = body(&send("GET", &format!("{chat}/{id}"), "", "").await);
        let a = &shown["attachments"][0];
        kinds.push((shown["type"].clone(), a["isSticker"].clone(), a["isVoiceNote"].clone(), a["isGif"].clone()));
        // Desktop edits no sticker.
        let edit = body(&send("PUT", &format!("{chat}/{id}"), json, r#"{"text":"x"}"#).await);
        assert_eq!(edit["code"] == "EDIT_NOT_ALLOWED", kind == "sticker", "{kind}: {edit}");
    }
    let (t, f) = (json!(true), json!(false));
    assert_eq!(
        kinds,
        [
            (json!("STICKER"), t.clone(), f.clone(), f.clone()),
            (json!("VOICE"), f.clone(), t.clone(), f.clone()),
            (json!("IMAGE"), f.clone(), f.clone(), t.clone()),
        ]
    );

    // A sent message has the user of the account as its sender, and an edit returns the message.
    let id =
        body(&send("POST", chat, json, r#"{"text":"hello"}"#).await)["pendingMessageID"].as_str().unwrap().to_string();
    let edited = body(&send("PUT", &format!("{chat}/{id}"), json, r#"{"text":"hello again"}"#).await);
    let accounts = body(&send("GET", "/v1/accounts", "", "").await);
    let me =
        accounts.as_array().unwrap().iter().find(|a| a["accountID"] == "acc_whatsapp").unwrap()["user"]["id"].clone();
    assert_eq!(
        (&edited["id"], &edited["messageID"], &edited["success"], &edited["text"], &edited["senderID"]),
        (&json!(id), &json!(id), &json!(true), &json!("hello again"), &me)
    );
    assert!(edited["editedTimestamp"].is_string(), "{edited}");
    // Desktop answers 204 to `forEveryone=false`, and the message stays.
    let kept = send("DELETE", &format!("{chat}/{id}?forEveryone=false"), "", "").await;
    assert!(kept.starts_with("HTTP/1.1 204 "), "{kept}");
    let message = body(&send("GET", &format!("{chat}/{id}"), "", "").await);
    assert_eq!(
        (&message["text"], &message["isDeleted"]),
        (&json!("hello again"), &serde_json::Value::Null),
        "{message}"
    );
}

#[tokio::test]
async fn contacts_and_focus_follow_desktop() {
    use serde_json::{json, Value};
    let (port, token) = server("contacts").await;
    let call = |method: &str, target: String, body: &str| {
        let request = format!(
            "{method} {target} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        async move {
            let out = text(port, &request).await;
            (out[9..12].to_string(), serde_json::from_str::<Value>(out.split_once("\r\n\r\n").unwrap().1).unwrap())
        }
    };
    let list = "/v1/accounts/acc_telegram/contacts/list";
    let (_, all) = call("GET", format!("{list}?limit=200"), "").await;
    let ids: Vec<&str> = all["items"].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap()).collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert!(ids.len() > 2 && ids.len() == unique.len() && all["hasMore"] == false, "each contact once: {ids:?}");
    assert!(all["items"].as_array().unwrap().iter().all(|u| u["isSelf"] != true));
    let (mut paged, mut cursor) = (Vec::new(), None::<String>);
    loop {
        let after = cursor.as_ref().map_or(String::new(), |c| format!("&cursor={}", beeper_api::codec::url_encode(c)));
        let (_, page) = call("GET", format!("{list}?limit=2{after}"), "").await;
        paged.extend(page["items"].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap().to_string()));
        cursor = page["oldestCursor"].as_str().map(str::to_string);
        if page["hasMore"] == false {
            break;
        }
    }
    assert_eq!(paged, ids);
    let name = all["items"][0]["fullName"].as_str().unwrap().to_lowercase();
    let (_, found) = call("GET", format!("{list}?query={}", beeper_api::codec::url_encode(&name)), "").await;
    assert!(found["items"].as_array().unwrap().iter().any(|u| u["id"] == ids[0]));
    assert_eq!(call("GET", "/v1/accounts/acc_telegram/contacts".into(), "").await.0, "400");

    let (status, failed) = call("POST", "/v1/focus".into(), r#"{"draftText":"hi"}"#).await;
    let want = json!({ "message": "Failed to execute tool: focusApp", "code": "TOOL_EXECUTION_ERROR", "details": {} });
    assert_eq!((status.as_str(), failed), ("500", want));
    let (status, _) = call("POST", "/v1/focus".into(), r#"{"messageID":"msg_1001"}"#).await;
    assert_eq!(status, "200", "a message without a chat only focuses the app");
    let (_, chat) = call("GET", "/v1/chats/!tg_rust:beeper.com".into(), "").await;
    assert_eq!(chat["isReadOnly"], false);
}

/// Desktop's `createChat` and `startChat` answer the chat like `getChat`, with `chatID` and, from `startChat`,
/// `status`. `startChat` finds a direct chat again by any identifier of the person, `createChat` with
/// `mode: "start"` is `startChat`, and both check their input like Desktop's zod schemas.
#[tokio::test]
async fn chats_are_created_and_started_like_desktop() {
    use serde_json::{json, Value};
    let (port, token) = server("create-chat").await;
    let post = |target: &str, body: Value| {
        let (token, target) = (token.clone(), target.to_string());
        async move { call(port, &token, "POST", &target, &body.to_string()).await }
    };
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();

    let (status, dm) = post(
        "/v1/chats",
        json!({ "accountID": "acc_whatsapp", "type": "single", "participantIDs": ["@pat:x"], "title": "Ignored" }),
    )
    .await;
    assert_eq!((status, &dm["chatID"], &dm["title"], &dm["type"]), (200, &dm["id"], &json!("pat"), &json!("single")));
    assert_eq!(keys(&dm).last().unwrap(), "chatID", "{dm}");
    let (_, shown) = call(port, &token, "GET", &format!("/v1/chats/{}", dm["id"].as_str().unwrap()), "").await;
    let mut like_get = shown.clone();
    like_get["chatID"] = dm["id"].clone();
    assert_eq!(dm, like_get, "the chat of getChat, then chatID");
    let (_, group) = post(
        "/v1/chats",
        json!({ "accountID": "acc_whatsapp", "type": "group", "participantIDs": ["@a:x", "@b:x"], "title": "Team" }),
    )
    .await;
    assert_eq!((&group["title"], &group["type"], group.get("status")), (&json!("Team"), &json!("group"), None));

    let start =
        json!({ "accountID": "acc_whatsapp", "user": { "phoneNumber": "+1 (555) 010-0999", "fullName": "Pat" } });
    let (status, started) = post("/v1/chats/start", start).await;
    assert_eq!((status, &started["status"], &started["title"]), (200, &json!("created"), &json!("Pat")));
    assert_eq!(keys(&started)[keys(&started).len() - 2..], ["chatID", "status"]);
    let again = json!({ "accountID": "acc_whatsapp", "user": { "phoneNumber": "+15550100999" } });
    let (_, found) = post("/v1/chats/start", again.clone()).await;
    assert_eq!((&found["id"], &found["status"]), (&started["id"], &json!("existing")), "the same phone number");
    let mut start_mode = again;
    start_mode["mode"] = json!("start");
    let (_, found) = post("/v1/chats", start_mode).await;
    assert_eq!((&found["id"], &found["status"]), (&started["id"], &json!("existing")), "mode start");

    let user_issue = |path: Value, message: &str| json!({ "code": "custom", "message": message, "path": path });
    let required = |path: &str, kind: &str| json!({ "code": "invalid_type", "expected": kind, "received": "undefined", "path": [path], "message": "Required" });
    let cases = [
        (json!({}), json!([required("accountID", "string"), required("user", "object")])),
        (
            json!({ "accountID": "acc_whatsapp", "user": { "fullName": "Pat", "id": "" }, "allowInvite": "yes" }),
            json!([
                user_issue(json!(["user", "id"]), "At least one of id, username, phoneNumber, or email is required."),
                zod::boolean("allowInvite", "string"),
            ]),
        ),
        (
            json!({ "accountID": "acc_whatsapp", "user": { "email": 7 } }),
            json!([{ "code": "invalid_type", "expected": "string", "received": "number", "path": ["user", "email"],
                "message": "Expected string, received number" }]),
        ),
    ];
    for (body, issues) in cases {
        let (status, refused) = post("/v1/chats/start", body.clone()).await;
        assert_eq!((status, refused.to_string()), (400, invalid(issues)), "{body}");
    }
    for (query, issue) in [("", required("query", "string")), ("?query=", zod::small("query", 1, "string"))] {
        let (status, refused) =
            call(port, &token, "GET", &format!("/v1/accounts/acc_telegram/contacts{query}"), "").await;
        assert_eq!((status, refused.to_string()), (400, invalid(json!([issue]))), "{query}");
    }
}

#[tokio::test]
async fn json_keys_keep_the_order_of_desktop() {
    let (port, _) = server("order").await;
    let get = "GET /.well-known/oauth-authorization-server HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n";
    let base = format!("http://127.0.0.1:{port}");
    let expected = format!(
        "{{\"issuer\":\"{base}\",\"authorization_endpoint\":\"{base}/oauth/authorize\",\"token_endpoint\":\"{base}/oauth/token\",\"revocation_endpoint\":\"{base}/oauth/revoke\",\"userinfo_endpoint\":\"{base}/oauth/userinfo\",\"registration_endpoint\":\"{base}/oauth/register\",\"grant_types_supported\":[\"authorization_code\"],\"token_endpoint_auth_methods_supported\":[\"none\"],\"response_types_supported\":[\"code\"],\"scopes_supported\":[\"read\",\"write\"],\"code_challenge_methods_supported\":[\"S256\"],\"service_documentation\":\"{base}/v1/spec\"}}"
    );
    assert_eq!(text(port, get).await.split_once("\r\n\r\n").unwrap().1, expected);
}

/// Desktop's `fetchMessagesPage`: a page back in time that the store cannot fill waits for older messages from the
/// homeserver, and the pages have more until the start of the room.
#[tokio::test(flavor = "multi_thread")]
async fn message_pages_load_back_from_the_homeserver_like_desktop() {
    use beeper_core::testing::{stub_homeserver, stub_room_store, temp_dir};
    let store = stub_room_store(&temp_dir("http-backfill"), &stub_homeserver().await).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_http(listener, store));
    let (mut texts, mut cursor, mut more) = (Vec::new(), None::<String>, Vec::new());
    loop {
        let after = cursor.as_ref().map_or(String::new(), |c| format!("?cursor={c}"));
        let get = format!("GET /v1/chats/!room:x/messages{after} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer token\r\nconnection: close\r\n\r\n");
        let out = text(port, &get).await;
        let page: serde_json::Value = serde_json::from_str(out.split_once("\r\n\r\n").unwrap().1).unwrap();
        texts.extend(page["items"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap().to_string()));
        cursor = page["oldestCursor"].as_str().map(str::to_string);
        more.push(page["hasMore"] == true);
        if page["hasMore"] == false {
            break;
        }
    }
    assert_eq!(texts, (1..=70).rev().map(|i| format!("m{i}")).collect::<Vec<_>>(), "every message once, newest first");
    assert_eq!(more, [true, true, true, false], "full pages of 20 until the start of the room");
}

/// Desktop's `sp`: a message carries its `sendStatus` (with the time of `ep`), its `mentions`, and reactions that say
/// whether their key is an emoji, in Desktop's key order.
#[tokio::test]
async fn messages_carry_send_status_mentions_and_emoji_like_desktop() {
    use beeper_core::models::{Reaction, SendStatus};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = CentralConfig { port, ..demo_config("http-message-fields") };
    let store = StoreLock::shared(BeeperStore::from_config(cfg));
    let (token, chat, mid) = {
        let mut st = store.write();
        let i = st.messages.iter().position(|m| m.is_sender == Some(true) && m.chat_id.starts_with('!')).unwrap();
        let reaction = |key: &str, img: Option<&str>| Reaction {
            id: key.into(),
            participant_id: "@x:beeper.com".into(),
            reaction_key: key.into(),
            animated: None,
            img_url: img.map(String::from),
            sort_key: String::new(),
        };
        st.update_message(i, |m| {
            m.mentions = Some(vec!["@x:beeper.com".into()]);
            m.send_status = Some(SendStatus {
                status: "SUCCESS".into(),
                reason: None,
                message: None,
                ts: Some(1_700_000_000_000),
                delivered_to_users: vec!["@x:beeper.com".into()],
                internal_error: None,
            });
            m.reactions = vec![reaction("👍", None), reaction("party", Some("mxc://x/party"))];
        });
        let m = &st.messages[i];
        (st.beeper_state.access_token.clone(), m.chat_id.clone(), m.id.clone())
    };
    tokio::spawn(serve_http(listener, store));
    let path =
        format!("/v1/chats/{}/messages/{}", beeper_api::codec::url_encode(&chat), beeper_api::codec::url_encode(&mid));
    let get = format!("GET {path} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\nconnection: close\r\n\r\n");
    let out = text(port, &get).await;
    let body = out.split_once("\r\n\r\n").unwrap().1;
    let m: serde_json::Value = serde_json::from_str(body).unwrap();
    let status = serde_json::json!({ "status": "SUCCESS", "timestamp": "2023-11-14T22:13:20.000Z", "deliveredToUsers": ["@x:beeper.com"] });
    assert_eq!(m["sendStatus"], status, "{body}");
    assert_eq!(m["mentions"], serde_json::json!(["@x:beeper.com"]));
    assert_eq!((m["reactions"][0]["emoji"].as_bool(), m["reactions"][1]["emoji"].as_bool()), (Some(true), Some(false)));
    let keys: Vec<&str> = ["\"isSender\"", "\"sendStatus\"", "\"linkedMessageID\"", "\"mentions\"", "\"reactions\""]
        .into_iter()
        .filter(|k| body.contains(k))
        .collect();
    let at = |k: &str| body.find(k).unwrap();
    assert!(keys.windows(2).all(|w| at(w[0]) < at(w[1])), "Desktop's key order: {body}");
    assert!(body.contains(r#"{"id":"👍","participantID":"@x:beeper.com","emoji":true,"reactionKey":"👍"}"#), "{body}");
}

/// Desktop's `sendAppStateSnapshot` and `processAppStateEvent`: an `app.state` subscription gets the setup state at
/// once, and then only when it changes, and the store's own `setup.updated` never shows.
#[tokio::test]
async fn websocket_sends_the_app_state_like_desktop() {
    use beeper_api::http::{read_ws_frame, write_ws_frame};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = CentralConfig { port, ..demo_config("http-app-state") };
    let store = StoreLock::shared(BeeperStore::from_config(cfg));
    let token = store.read().beeper_state.access_token.clone();
    tokio::spawn(serve_http(listener, store.clone()));
    let upgrade = format!("GET /v1/ws HTTP/1.1\r\nhost: x\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\nauthorization: Bearer {token}\r\n\r\n");
    let mut ws = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    ws.write_all(upgrade.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(ws.read_u8().await.unwrap());
    }
    async fn next(ws: &mut TcpStream) -> Option<serde_json::Value> {
        let frame = timeout(Duration::from_millis(500), read_ws_frame(ws, 1 << 20)).await.ok()?.ok()?;
        Some(serde_json::from_slice(&frame.1).unwrap())
    }
    assert_eq!(next(&mut ws).await.unwrap()["type"], "ready");
    write_ws_frame(&mut ws, 0x1, br#"{"type":"subscriptions.set","chatIDs":["*"],"app":{"state":true}}"#, true)
        .await
        .unwrap();
    let snapshot = next(&mut ws).await.unwrap();
    assert_eq!((snapshot["type"].as_str(), snapshot["seq"].as_u64()), (Some("app.state.updated"), Some(1)));
    assert_eq!(next(&mut ws).await.unwrap()["type"], "subscriptions.updated", "after the snapshot, like Desktop");
    assert_eq!(snapshot["appState"]["e2ee"]["verified"], store.read().beeper_state.verified);
    store.read().emit_ws("setup.updated", "", None, None, None);
    assert!(next(&mut ws).await.is_none(), "an unchanged state sends nothing");
    {
        let mut st = store.write();
        st.beeper_state.key_backup = !st.beeper_state.key_backup;
        st.emit_ws("setup.updated", "", None, None, None);
    }
    let changed = next(&mut ws).await.unwrap();
    assert_eq!((changed["type"].as_str(), changed["seq"].as_u64()), (Some("app.state.updated"), Some(2)));
    let keys: Vec<&String> = changed.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["type", "seq", "ts", "appState"], "Desktop's key order");
}

/// Sends a request with the token and a JSON body, and returns the status and the JSON of the response.
async fn call(port: u16, token: &str, method: &str, target: &str, body: &str) -> (u16, serde_json::Value) {
    let request = format!(
        "{method} {target} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let out = text(port, &request).await;
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    (head[9..12].parse().unwrap(), serde_json::from_str(body).expect("a JSON body"))
}

/// Desktop's `Invalid input` answer with zod's issues.
fn invalid(issues: serde_json::Value) -> String {
    serde_json::json!({ "message": "Invalid input", "code": "VALIDATION_ERROR", "details": { "issues": issues } })
        .to_string()
}

/// zod's issues of a bad enum value, a bad boolean `received`, a bad `datetime()` and a value below `minimum`.
mod zod {
    use serde_json::{json, Value};

    pub fn choice(path: &str, options: &[&str], received: &str) -> Value {
        let expected = options.iter().map(|o| format!("'{o}'")).collect::<Vec<_>>().join(" | ");
        let message = format!("Invalid enum value. Expected {expected}, received '{received}'");
        json!({ "received": received, "code": "invalid_enum_value", "options": options, "path": [path], "message": message })
    }

    pub fn boolean(path: &str, received: &str) -> Value {
        let message = format!("Expected boolean, received {received}");
        json!({ "code": "invalid_type", "expected": "boolean", "received": received, "path": [path], "message": message })
    }

    pub fn string(path: &str) -> Value {
        let message = "Expected string, received array";
        json!({ "code": "invalid_type", "expected": "string", "received": "array", "path": [path], "message": message })
    }

    pub fn datetime(path: &str) -> Value {
        json!({ "code": "invalid_string", "validation": "datetime", "message": "Invalid datetime", "path": [path] })
    }

    pub fn small(path: &str, minimum: i64, kind: &str) -> Value {
        let message = match kind {
            "string" => format!("String must contain at least {minimum} character(s)"),
            _ => format!("Number must be greater than or equal to {minimum}"),
        };
        json!({ "code": "too_small", "minimum": minimum, "type": kind, "inclusive": true, "exact": false, "message": message, "path": [path] })
    }

    pub fn big(maximum: i64) -> Value {
        let message = format!("Number must be less than or equal to {maximum}");
        json!({ "code": "too_big", "maximum": maximum, "type": "number", "inclusive": true, "exact": false, "message": message, "path": ["limit"] })
    }
}

/// The IDs of the items of a page.
fn ids(page: &serde_json::Value) -> Vec<String> {
    page["items"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_string()).collect()
}

/// Desktop's `searchChats`: the matches of the title and the network by the words in the title, the matches of the
/// participants and all the chats newest first, the filters, the base64 JSON cursors in both directions, and zod's
/// issues in the order of the schema.
#[tokio::test]
async fn search_chats_filters_orders_and_pages_like_desktop() {
    use beeper_api::codec::{base64_decode, url_encode};
    use serde_json::{json, Value};
    let (port, token) = server("search-chats").await;
    let get = |query: String| {
        let token = token.clone();
        async move { call(port, &token, "GET", &format!("/v1/chats/search?{query}"), "").await }
    };
    let found = |query: &str| {
        let page = get(query.into());
        let query = query.to_string();
        async move {
            let (status, page) = page.await;
            assert_eq!(status, 200, "{query}: {page}");
            ids(&page)
        }
    };
    let patch = |id: &str, body: &str| {
        let (token, target, body) = (token.clone(), format!("/v1/chats/{id}"), body.to_string());
        async move { assert_eq!(call(port, &token, "PATCH", &target, &body).await.0, 200, "{target} {body}") }
    };
    let key = |cursor: &Value| -> Value {
        serde_json::from_slice(&base64_decode(cursor.as_str().unwrap()).unwrap()).unwrap()
    };
    let ms = |c: &Value| beeper_core::config::millis_from_rfc3339(c["lastActivity"].as_str().unwrap()).unwrap();

    let (_, listed) = call(port, &token, "GET", "/v1/chats?limit=200", "").await;
    let (status, page) = get(String::new()).await;
    let keys: Vec<&str> = page.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!((status, keys), (200, vec!["items", "hasMore", "oldestCursor", "newestCursor"]));
    let all = ids(&listed);
    assert_eq!((ids(&page), &page["hasMore"]), (all.clone(), &json!(false)), "all the chats, newest first");
    let first = &page["items"][0];
    assert!(first.get("preview").is_none(), "no preview: {first}");
    assert_eq!(key(&page["newestCursor"]), json!({ "ts": ms(first), "id": first["id"] }));
    assert_eq!(found("limit=3").await, all[..3]);

    let (mut seen, mut cursor) = (Vec::new(), None::<String>);
    loop {
        let (_, page) = get(cursor.map_or("limit=4".into(), |c| format!("limit=4&cursor={}", url_encode(&c)))).await;
        seen.extend(ids(&page));
        cursor = page["oldestCursor"].as_str().map(str::to_string);
        if page["hasMore"] == false {
            break;
        }
    }
    assert_eq!(seen, all, "paging `before` walks all the chats");
    let third = url_encode(get("limit=3".into()).await.1["oldestCursor"].as_str().unwrap());
    assert_eq!(found(&format!("limit=2&cursor={third}")).await, all[3..5]);
    let (_, newer) = get(format!("limit=1&direction=after&cursor={third}")).await;
    assert_eq!((ids(&newer), &newer["hasMore"]), (all[..1].to_vec(), &json!(true)), "`after` takes the newest chats");
    let none = json!({ "items": [], "hasMore": false, "oldestCursor": null, "newestCursor": null });
    assert_eq!(get("cursor=W10%3D".into()).await.1, none, "the keys of a cursor `[]` compare as NaN");
    assert_eq!(found("cursor=nope").await, all, "an unreadable cursor is no cursor");

    patch("!tg_pebble:beeper.com", r#"{"title":"Pebble Rust"}"#).await;
    let rust = ["!tg_rust:beeper.com", "!tg_weekly:beeper.com", "!tg_pebble:beeper.com"];
    assert_eq!(found("query=Rust").await, rust, "one title hit each, newest first");
    assert_eq!(found("query=rust%20telegram").await, rust, "the network matches, and only the title counts");
    let (_, page) = get("query=telegram%20pebble%20rust".into()).await;
    assert_eq!((ids(&page), key(&page["newestCursor"])["score"].clone()), (vec![rust[2].to_string()], json!(2)));
    assert_eq!(found("query=rust%20pebble%20OR%20weekly").await, Vec::<String>::new(), "every word must match");
    assert_eq!(found("query=desing").await, ["!sl_design:beeper.com"], "a fuzzy match");
    assert_eq!(found("query=%23desktop").await, ["!sl_infra:beeper.com"], "a query with a symbol is a phrase");
    assert_eq!(found("query=%20eric%20").await, ["!wa_eric:beeper.com"]);
    let people = found("query=migicovsky&scope=participants").await;
    assert_eq!(people, ["!wa_eric:beeper.com", "!sl_infra:beeper.com"], "by time");
    assert_eq!(found("query=eric&scope=participants").await, all, "`er.c` of `alex@beeper.com` is a fuzzy match");
    assert_eq!(found("query=rust&accountIDs=acc_signal").await, Vec::<String>::new());

    let only = |pick: &dyn Fn(&Value) -> bool| -> Vec<String> {
        let items = listed["items"].as_array().unwrap().iter();
        items.filter(|c| pick(c)).map(|c| c["id"].as_str().unwrap().into()).collect()
    };
    assert_eq!(found("type=group").await, only(&|c| c["type"] == "group"));
    assert_eq!(found("type=single").await, only(&|c| c["type"] == "single"));
    assert_eq!(found("type=any").await, all);
    assert_eq!(found("unreadOnly=true").await, only(&|c| c["unreadCount"].as_u64() > Some(0)));
    assert_eq!(found("labelID=lbl_work").await, ["!tg_rust:beeper.com", "!sl_infra:beeper.com"]);
    assert_eq!(found("includeMuted=false").await, only(&|c| c["isMuted"] == false));
    let slack_or_signal = only(&|c| c["accountID"] == "acc_slack" || c["accountID"] == "acc_signal");
    assert_eq!(found("accountIDs[]=acc_slack&accountIDs[]=acc_signal").await, slack_or_signal);
    assert_eq!(found("accountIDs=").await, Vec::<String>::new(), "no account has a blank ID");
    let at = beeper_core::config::millis_from_rfc3339("2026-10-14T07:45:00.000Z").unwrap();
    assert_eq!(found("lastActivityBefore=2026-10-14T07:45Z").await, only(&|c| ms(c) < at));
    assert_eq!(found("lastActivityAfter=2026-10-14T07:45:00.0001Z").await, only(&|c| ms(c) > at));
    for (flag, inbox) in [("isArchived", "archive"), ("isLowPriority", "low-priority")] {
        patch("!wa_sam:beeper.com", &format!(r#"{{"{flag}":true}}"#)).await;
        assert_eq!(found(&format!("inbox={inbox}")).await, ["!wa_sam:beeper.com"]);
        assert!(!found("inbox=primary").await.contains(&"!wa_sam:beeper.com".to_string()));
        patch("!wa_sam:beeper.com", &format!(r#"{{"{flag}":false}}"#)).await;
    }
    assert!(found("inbox=primary").await.contains(&"!wa_sam:beeper.com".to_string()));

    let merge =
        r#"{"mode":"merge","title":"Friends","merge":{"chatIDs":["!sg_lena:beeper.com","!ig_kai:beeper.com"]}}"#;
    let (_, merged) = call(port, &token, "POST", "/v1/chats", merge).await;
    let merged = merged["chatID"].as_str().unwrap();
    let (members, now) = (["!sg_lena:beeper.com", "!ig_kai:beeper.com"], found("limit=200").await);
    // Desktop's `sortTimestamp` of a merged chat is the newest activity of its member chats.
    assert!(now.contains(&merged.to_string()) && members.iter().all(|m| !now.contains(&m.to_string())), "{now:?}");
    assert_eq!(found("query=fischer").await, [merged], "a member matches as its merged chat");
    assert_eq!(found("query=fischer&accountIDs=acc_signal").await, [merged]);

    let cases = [
        ("query=", json!([zod::small("query", 1, "string")])),
        ("query=%20%0A", json!([zod::small("query", 1, "string")])),
        ("limit=0", json!([zod::small("limit", 1, "number")])),
        ("limit=201", json!([zod::big(200)])),
        ("includeMuted=1", json!([zod::boolean("includeMuted", "string")])),
        ("lastActivityAfter=2026-10-14T07:45:00%2B02:00", json!([zod::datetime("lastActivityAfter")])),
        ("lastActivityBefore=2026-02-29T00:00:00Z", json!([zod::datetime("lastActivityBefore")])),
        (
            "includeMuted=no&scope=people&query=&type=dm&limit=0&unreadOnly=1&unreadOnly=0&labelID=a&labelID=b&inbox=all&direction=up&cursor=a&cursor=b&lastActivityBefore=x",
            json!([
                zod::string("cursor"),
                zod::choice("direction", &["after", "before"], "up"),
                zod::choice("inbox", &["primary", "low-priority", "archive"], "all"),
                zod::string("labelID"),
                zod::boolean("unreadOnly", "array"),
                zod::small("limit", 1, "number"),
                zod::choice("type", &["single", "group", "any"], "dm"),
                zod::small("query", 1, "string"),
                zod::choice("scope", &["titles", "participants"], "people"),
                zod::datetime("lastActivityBefore"),
                zod::boolean("includeMuted", "string"),
            ]),
        ),
    ];
    for (query, issues) in cases {
        let (status, body) = get(query.into()).await;
        assert_eq!((status, body.to_string()), (400, invalid(issues)), "{query}");
    }
}

/// Desktop's `searchMessages`: the newest messages by time past a `<time>:<row>` cursor in both directions, then in
/// sort order, with their chats, the filters and zod's issues, and Desktop's `search` over both searches.
#[tokio::test]
async fn search_messages_filters_orders_and_pages_like_desktop() {
    use beeper_api::codec::url_encode;
    use serde_json::{json, Value};
    let (port, token) = server("search-messages").await;
    let get = |path: &str, query: String| {
        let (token, target) = (token.clone(), format!("{path}?{query}"));
        async move { call(port, &token, "GET", &target, "").await }
    };
    let search = |query: &str| {
        let page = get("/v1/messages/search", query.into());
        let query = query.to_string();
        async move {
            let (status, page) = page.await;
            assert_eq!(status, 200, "{query}: {page}");
            page
        }
    };
    let keys = |page: &Value| -> Vec<String> {
        page["items"].as_array().unwrap().iter().map(|m| m["sortKey"].as_str().unwrap()[12..].to_string()).collect()
    };
    let found = |query: &str| {
        let page = search(query);
        async move { keys(&page.await) }
    };
    let time = |at: &str| beeper_core::config::millis_from_rfc3339(at).unwrap().to_string();

    let page = search("limit=7").await;
    let fields: Vec<&str> = page.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(fields, ["items", "chats", "hasMore", "oldestCursor", "newestCursor"]);
    assert_eq!(
        keys(&page),
        ["1004", "1003", "1002", "1001", "0999", "0998", "0997"],
        "the newest by time, by sort key"
    );
    let chats: Vec<&str> = page["chats"].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(chats, ["!tg_rust:beeper.com", "!im_sarah:beeper.com", "!wa_eric:beeper.com"]);
    assert_eq!(page["chats"]["!tg_rust:beeper.com"]["participants"]["items"].as_array().unwrap().len(), 3);
    let cursor = |k: &str| page[k].as_str().unwrap().split_once(':').unwrap().0.to_string();
    assert_eq!(
        (cursor("oldestCursor"), cursor("newestCursor")),
        (time("2026-10-14T08:10:00Z"), time("2026-10-14T08:20:00Z"))
    );
    let oldest = url_encode(page["oldestCursor"].as_str().unwrap());
    assert_eq!(found(&format!("limit=2&cursor={oldest}")).await, ["1006", "1005"]);
    let newer = search(&format!("limit=3&direction=after&cursor={oldest}")).await;
    assert_eq!((keys(&newer), &newer["hasMore"]), (vec!["1004".into(), "1003".into(), "1001".into()], &json!(true)));
    let none = json!({ "items": [], "chats": {}, "hasMore": false, "oldestCursor": null, "newestCursor": null });
    let newest = url_encode(page["newestCursor"].as_str().unwrap());
    assert_eq!(search(&format!("direction=after&cursor={newest}")).await, none);

    let (mut seen, mut cursor) = (Vec::new(), None::<String>);
    loop {
        let page = search(&cursor.map_or("limit=6".into(), |c| format!("limit=6&cursor={}", url_encode(&c)))).await;
        let times = page["items"].as_array().unwrap().iter().map(|m| m["timestamp"].as_str().unwrap().to_string());
        seen.extend(times.collect::<Vec<_>>());
        cursor = page["oldestCursor"].as_str().map(str::to_string);
        if page["hasMore"] == false {
            break;
        }
    }
    assert_eq!(seen.len(), 31, "paging `before` walks all the messages");
    assert!(seen.chunks(6).zip(seen.chunks(6).skip(1)).all(|(a, b)| a.iter().min() > b.iter().max()));
    let first = search("limit=1").await;
    let piped = search("limit=1&cursor=%7Cimsg").await;
    assert_eq!(piped["newestCursor"], format!("{}|imsg", first["newestCursor"].as_str().unwrap()), "the iMessage part");
    assert_eq!(keys(&search("limit=1&cursor=nope").await), keys(&first), "an unreadable cursor is no cursor");
    assert_eq!(search("").await["items"].as_array().unwrap().len(), 20);

    assert_eq!(found("query=ship").await, ["1011", "1002"], "a prefix of a word, in sort order");
    assert_eq!(found("query=Ship%20i").await, ["1011"]);
    let (status, blank) = get("/v1/messages/search", "query=%20".into()).await;
    let failed =
        json!({ "message": "Failed to execute tool: searchMessages", "code": "TOOL_EXECUTION_ERROR", "details": {} });
    assert_eq!((status, blank), (500, failed), "Desktop's full-text query of a blank query fails");
    assert_eq!(found("chatIDs=!sl_infra:beeper.com").await, ["1011", "1010", "1009", "1008", "1007"]);
    assert_eq!(found("chatIDs[]=!sl_design:beeper.com&chatIDs[]=!tg_pebble:beeper.com").await, ["2007", "2005"]);
    assert_eq!(get("/v1/messages/search", "chatIDs=nope".into()).await.0, 404);
    assert_eq!(found("accountIDs=acc_slack").await, ["2005", "1011", "1010", "1009", "1008", "1007"]);
    assert_eq!(found("chatType=group").await.len(), 11);
    assert_eq!(found("chatType=single&limit=1").await, ["1001"]);
    assert_eq!(found("mediaTypes=image").await, ["2010", "2009", "2008", "2002"]);
    assert_eq!(found("mediaTypes=file").await, ["1003", "0998"]);
    assert_eq!(found("mediaTypes=link").await, ["0904", "0902", "0900"]);
    assert_eq!(found("mediaTypes=any").await.len(), 7);
    assert_eq!(found("mediaTypes=image&mediaTypes=file").await.len(), 6);
    assert_eq!(found("sender=me").await.len(), 7);
    assert_eq!(search("sender=others").await["hasMore"], true);
    assert_eq!(found("sender=@devon:beeper.com").await, ["1010", "1009", "1007"]);
    assert_eq!(found("dateAfter=2026-10-14T10:00:00%2B02:00").await.len(), 7);
    assert_eq!(found("dateAfter=2026-10-14T08:10:00Z").await.len(), 6, "strictly after");
    assert_eq!(found("dateBefore=2026-10-06T19:04Z").await, ["2007", "0900"]);
    assert_eq!(found("includeMuted=false&limit=7").await, ["1005", "1004", "1003", "1001", "0999", "0998", "0997"]);
    let low = r#"{"isLowPriority":true}"#;
    assert_eq!(call(port, &token, "PATCH", "/v1/chats/!sl_infra:beeper.com", low).await.0, 200);
    assert_eq!(found("chatIDs=!sl_infra:beeper.com").await, Vec::<String>::new(), "low priority is out by default");
    assert_eq!(found("chatIDs=!sl_infra:beeper.com&excludeLowPriority=false").await.len(), 5);

    let (_, unified) = get("/v1/search", "query=rust".into()).await;
    let section = |path: &str, query: &str| get(path, query.into());
    let (titles, people, messages) = (
        section("/v1/chats/search", "query=rust&limit=30").await.1,
        section("/v1/chats/search", "query=rust&scope=participants&limit=30").await.1,
        section("/v1/messages/search", "query=rust").await.1,
    );
    let want = json!({ "results": { "chats": titles["items"], "in_groups": people["items"], "messages": messages } });
    assert_eq!(unified, want, "Desktop's `search` is both searches");

    let custom = json!({ "code": "custom", "message": "'dateAfter' must be earlier than 'dateBefore' when both are provided.", "path": ["dateAfter"] });
    let reversed = "dateAfter=2026-02-01T00:00:00Z&dateBefore=2026-01-01T00:00:00Z";
    let float = json!({ "code": "invalid_type", "expected": "integer", "received": "float", "message": "Expected integer, received float", "path": ["limit"] });
    let media = ["any", "video", "image", "link", "file"];
    let cases = [
        ("limit=21".to_string(), json!([zod::big(20)])),
        (reversed.into(), json!([custom])),
        ("dateAfter=2026-01-01T00:00:00Z&dateBefore=2026-01-01T00:00:00Z".into(), json!([custom])),
        (format!("limit=1.5&{reversed}"), json!([float, custom])),
        ("dateAfter=2026-02-01T00:00:00Z&dateBefore=x".into(), json!([zod::datetime("dateBefore")])),
        ("dateAfter=2026-02-30T00:00:00Z".into(), json!([zod::datetime("dateAfter")])),
        (format!("chatType=dm&{reversed}"), json!([zod::choice("chatType", &["group", "single"], "dm")])),
        ("mediaTypes=any&mediaTypes=audio".into(), {
            let mut issue = zod::choice("mediaTypes", &media, "audio");
            issue["path"] = json!(["mediaTypes", 1]);
            json!([issue])
        }),
        ("dateAfter=2026-01-01".into(), json!([zod::datetime("dateAfter")])),
        (
            "includeMuted=1&excludeLowPriority=true&excludeLowPriority=false&sender=a&sender=b&query=a&query=b".into(),
            json!([
                zod::string("query"),
                zod::string("sender"),
                zod::boolean("excludeLowPriority", "array"),
                zod::boolean("includeMuted", "string"),
            ]),
        ),
    ];
    for (query, issues) in cases {
        let (status, body) = get("/v1/messages/search", query.clone()).await;
        assert_eq!((status, body.to_string()), (400, invalid(issues)), "{query}");
    }
}

/// Desktop's Stainless MCP routes read the MCP URL before the MCP SDK's transport checks the request, answer with
/// `res.json` and no `id`, and the transport writes its errors with `writeHead`, without a `Content-Type`.
#[tokio::test]
async fn mcp_errors_come_from_the_layer_of_desktop() {
    let (port, token) = server("mcp-errors").await;
    let request = |method: &str, target: &str, headers: &str| {
        format!("{method} {target} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\n{headers}content-length: 2\r\nconnection: close\r\n\r\n{{}}")
    };
    let json = "content-type: application/json; charset=utf-8\r\n";
    let filters = text(port, &request("POST", "/v0/mcp?operation=x", "")).await;
    let message = "Invalid request: Validation error: Invalid enum value. Expected 'read' | 'write', received 'x' at \\\"operation[0]\\\"";
    assert!(filters.starts_with("HTTP/1.1 400 ") && filters.contains(json), "{filters}");
    assert!(
        filters.ends_with(&format!(r#"{{"jsonrpc":"2.0","error":{{"code":-32000,"message":"{message}"}}}}"#)),
        "{filters}"
    );
    let get = text(port, &request("GET", "/v0/mcp", "")).await;
    assert!(get.ends_with(r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not supported"}}"#), "{get}");
    let accept = text(port, &request("POST", "/v0/mcp", "content-type: application/json\r\n")).await;
    assert!(accept.starts_with("HTTP/1.1 406 ") && !accept.to_lowercase().contains("content-type:"), "{accept}");
    assert!(accept.ends_with(r#""id":null}"#), "{accept}");

    // The Stainless client of a tool refuses a missing path parameter, so `get_chat` without a chat is no chat list.
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_chat","arguments":{}}}"#;
    let headers = format!("content-type: application/json\r\naccept: application/json, text/event-stream\r\ncontent-length: {}\r\n", call.len());
    let out = text(port, &format!("POST /v0/mcp HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\n{headers}connection: close\r\n\r\n{call}")).await;
    let message = r"Path parameters result in path with invalid segments:\nValue of type Undefined is not a valid path parameter\n/v1/chats/undefined\n          ^^^^^^^^^";
    assert!(out.contains(&format!(r#""error":{{"code":-32603,"message":"{message}"}}"#)), "{out}");
}

/// Desktop's `listBridges` leaves out the Beeper and iMessage bridges and names each network by its ID, and its
/// login routes read the bridge accounts, so an unknown bridge has no logins rather than a 404.
#[tokio::test]
async fn bridges_list_like_desktop() {
    let (port, token) = server("bridges").await;
    let (status, list) = call(port, &token, "GET", "/v1/bridges", "").await;
    assert_eq!(status, 200);
    let items = list["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|b| b["id"].as_str().unwrap()).collect();
    assert!(!ids.contains(&"hungryserv") && !ids.contains(&"imessage"), "{ids:?}");
    let whatsapp = items.iter().find(|b| b["id"] == "whatsapp").unwrap();
    assert_eq!((&whatsapp["displayName"], &whatsapp["network"]), (&"WhatsApp".into(), &"whatsapp".into()));
    let slack = items.iter().find(|b| b["id"] == "slackgo").unwrap();
    assert_eq!(slack["network"], "slack");
    let not_found = serde_json::json!({ "message": "Bridge not found", "code": "NOT_FOUND" });
    assert_eq!(call(port, &token, "GET", "/v1/bridges/hungryserv", "").await, (404, not_found));
    let none = call(port, &token, "GET", "/v1/bridges/nope/logins", "").await;
    assert_eq!(none, (200, serde_json::json!({ "items": [] })));
}

/// Desktop's setup routes check their bodies with zod's `.strict()` objects and answer the first issue, its body
/// parser answers V8's message for malformed JSON, and the chat routes but `getChat` name a chat they do not find.
#[tokio::test]
async fn bodies_and_unknown_chats_get_the_errors_of_desktop() {
    use serde_json::json;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let out_port = listener.local_addr().unwrap().port();
    let dir = beeper_core::testing::temp_dir("http-setup-bodies");
    // No request reaches the cloud: each body fails its schema first.
    let cfg = CentralConfig {
        api_url: "http://127.0.0.1:9".into(),
        ..CentralConfig::new(dir, "127.0.0.1".into(), out_port, None)
    };
    tokio::spawn(serve_http(listener, StoreLock::shared(BeeperStore::from_config(cfg))));
    let refused = |message: &str, issues: serde_json::Value| json!({ "message": message, "code": "VALIDATION_ERROR", "details": { "issues": issues } });
    let required = |k: &str| json!({ "code": "invalid_type", "expected": "string", "received": "undefined", "path": [k], "message": "Required" });
    let email =
        json!({ "validation": "email", "code": "invalid_string", "message": "Invalid email", "path": ["email"] });
    let extra = json!({ "code": "unrecognized_keys", "keys": ["x", "y"], "path": [], "message": "Unrecognized key(s) in object: 'x', 'y'" });
    let terms = json!({ "code": "invalid_literal", "expected": true, "path": ["acceptTerms"], "message": "Invalid literal value, expected true" });
    let no_body = json!({ "code": "invalid_type", "expected": "object", "received": "undefined", "path": [], "message": "Required" });
    let cases = [
        (
            "/v1/app/setup/email",
            r#"{"setupRequestID":"r","email":"notanemail"}"#,
            refused("Invalid email", json!([email])),
        ),
        (
            "/v1/app/setup/email",
            r#"{"email":"a@b.co","x":1,"y":2}"#,
            refused("Required", json!([required("setupRequestID"), extra])),
        ),
        ("/v1/app/setup/response", r#"{"setupRequestID":"r"}"#, refused("Required", json!([required("response")]))),
        (
            "/v1/app/setup/register",
            r#"{"setupRequestID":"r","leadToken":"l","username":"u"}"#,
            refused("Invalid literal value, expected true", json!([terms])),
        ),
    ];
    for (path, body, expected) in cases {
        assert_eq!(call(out_port, "", "POST", path, body).await, (400, expected), "{path} {body}");
    }
    let out = text(out_port, "POST /v1/app/setup/email HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n").await;
    let expected = refused("Required", json!([no_body])).to_string();
    assert!(out.starts_with("HTTP/1.1 400 ") && out.ends_with(&expected), "a request without a body: {out}");

    let (port, token) = server("setup-bodies").await;
    let purpose = json!({ "received": "x", "code": "invalid_enum_value", "options": ["login", "device"], "path": ["purpose"], "message": "Invalid enum value. Expected 'login' | 'device', received 'x'" });
    let expected = (400, refused("Invalid enum value. Expected 'login' | 'device', received 'x'", json!([purpose])));
    assert_eq!(call(port, &token, "POST", "/v1/app/setup/verifications", r#"{"purpose":"x"}"#).await, expected);
    let malformed = |message: &str| (400, json!({ "message": message, "code": "UNKNOWN_ERROR" }));
    let start = call(port, &token, "POST", "/v1/chats/start", "{x").await;
    assert_eq!(start, malformed("Expected property name or '}' in JSON at position 1 (line 1 column 2)"));
    let read = call(port, &token, "POST", "/v1/chats/!wa_eric:beeper.com/read", "\"text\"").await;
    assert_eq!(
        read,
        malformed("Unexpected token '\"', \"\"text\"\" is not valid JSON"),
        "a body must be an object or an array"
    );
    let mcp = call(port, &token, "POST", "/v0/mcp", "[1,]").await;
    assert_eq!(mcp, malformed("Unexpected token ']', \"[1,]\" is not valid JSON"), "before the MCP transport");
    let no_chat = |message: &str| (404, json!({ "message": message, "code": "NOT_FOUND" }));
    assert_eq!(call(port, &token, "GET", "/v1/chats/!nope", "").await, no_chat("Chat not found"));
    for (method, target) in [
        ("GET", "/v1/chats/!nope/messages"),
        ("POST", "/v1/chats/!nope/read"),
        ("PATCH", "/v1/chats/!nope"),
        ("GET", "/v1/messages/search?chatIDs=!nope"),
    ] {
        assert_eq!(
            call(port, &token, method, target, "{}").await,
            no_chat("Chat not found: !nope"),
            "{method} {target}"
        );
    }
}

/// Desktop's tool routes check `{...query, ...params, ...body}` with their zod input schemas before they look for
/// the chat, and answer the issues.
#[tokio::test]
async fn tool_inputs_are_checked_like_desktop() {
    use serde_json::json;
    let (port, token) = server("tool-inputs").await;
    let required = |path: serde_json::Value, kind: &str| json!({ "code": "invalid_type", "expected": kind, "received": "undefined", "path": path, "message": "Required" });
    let short = |path: serde_json::Value| json!({ "code": "too_small", "minimum": 1, "type": "string", "inclusive": true, "exact": false, "message": "String must contain at least 1 character(s)", "path": path });
    let wrong = |path: serde_json::Value, kind: &str, got: &str| json!({ "code": "invalid_type", "expected": kind, "received": got, "path": path, "message": format!("Expected {kind}, received {got}") });
    let custom = |path: serde_json::Value, message: &str| json!({ "code": "custom", "message": message, "path": path });
    let chat = "/v1/chats/!wa_eric:beeper.com";
    let cases = [
        ("PUT", "/v1/chats/!nope/messages/msg_1001".to_string(), r#"{"text":""}"#, json!([short(json!(["text"]))])),
        (
            "POST",
            format!("{chat}/messages/msg_1001/reactions"),
            "{}",
            json!([required(json!(["reactionKey"]), "string")]),
        ),
        ("POST", format!("{chat}/reminders"), "{}", json!([required(json!(["reminder"]), "object")])),
        (
            "POST",
            format!("{chat}/reminders"),
            r#"{"reminder":{"remindAt":"tomorrow","dismissOnIncomingMessage":1}}"#,
            json!([
                { "code": "invalid_string", "validation": "datetime", "message": "Invalid datetime", "path": ["reminder", "remindAt"] },
                wrong(json!(["reminder", "dismissOnIncomingMessage"]), "boolean", "number"),
            ]),
        ),
        (
            "POST",
            format!("{chat}/archive"),
            r#"{"archived":"yes"}"#,
            json!([wrong(json!(["archived"]), "boolean", "string")]),
        ),
        (
            "POST",
            format!("{chat}/messages"),
            "{}",
            json!([custom(json!([]), "Either text or attachment is required.")]),
        ),
        (
            "POST",
            format!("{chat}/messages"),
            r#"{"text":5,"attachment":{"uploadID":1,"size":{"width":1}}}"#,
            json!([
                wrong(json!(["text"]), "string", "number"),
                wrong(json!(["attachment", "uploadID"]), "string", "number"),
                required(json!(["attachment", "size", "height"]), "number"),
            ]),
        ),
        (
            "PATCH",
            chat.to_string(),
            r#"{"title":null,"isPinned":"yes","messageExpirySeconds":-1.5}"#,
            json!([
                wrong(json!(["isPinned"]), "boolean", "string"),
                { "code": "invalid_type", "expected": "integer", "received": "float", "message": "Expected integer, received float", "path": ["messageExpirySeconds"] },
                { "code": "too_small", "minimum": 0, "type": "number", "inclusive": true, "exact": false, "message": "Number must be greater than or equal to 0", "path": ["messageExpirySeconds"] },
            ]),
        ),
        (
            "DELETE",
            format!("{chat}/messages/msg_1001?forEveryone=yes"),
            "",
            json!([wrong(json!(["forEveryone"]), "boolean", "string")]),
        ),
        (
            "POST",
            "/v1/assets/download".to_string(),
            r#"{"url":""}"#,
            json!([short(json!(["url"])), custom(json!(["url"]), "URL must start with mxc:// or localmxc://")]),
        ),
        (
            "POST",
            "/v1/focus".to_string(),
            r#"{"draftText":1}"#,
            json!([wrong(json!(["draftText"]), "string", "number")]),
        ),
        (
            "GET",
            "/v1/bridges/bad!id".to_string(),
            "",
            json!([{ "validation": "regex", "code": "invalid_string", "message": "Bridge ID may only contain letters, numbers, underscores, and hyphens", "path": ["bridgeID"] }]),
        ),
        (
            "GET",
            format!("{chat}?maxParticipantCount=x"),
            "",
            json!([wrong(json!(["maxParticipantCount"]), "number", "nan")]),
        ),
        ("GET", "/v1/search?query=".to_string(), "", json!([short(json!(["query"]))])),
    ];
    for (method, target, body, issues) in cases {
        assert_eq!(
            call(port, &token, method, &target, body).await,
            (400, serde_json::from_str::<serde_json::Value>(&invalid(issues)).unwrap()),
            "{method} {target} {body}"
        );
    }
    let reminder = r#"{"reminder":{"remindAtMs":1900000000000}}"#;
    let request = format!(
        "POST {chat}/reminders HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reminder}",
        reminder.len()
    );
    assert!(text(port, &request).await.starts_with("HTTP/1.1 204 "), "the older remindAtMs");
    let edited = call(port, &token, "PUT", &format!("{chat}/messages/msg_1001"), r#"{"text":" "}"#).await;
    assert_eq!((edited.0, &edited.1["text"]), (200, &json!(" ")), "zod's min(1) takes a blank text");
    let (_, added) =
        call(port, &token, "POST", &format!("{chat}/messages/msg_1001/reactions"), r#"{"reactionKey":"👍"}"#).await;
    let keys: Vec<&String> = added.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["success", "chatID", "messageID", "reactionKey", "transactionID"], "Desktop's key order");
    let removed = call(port, &token, "DELETE", &format!("{chat}/messages/msg_1001/reactions?reactionKey="), "").await;
    assert_eq!(removed, (400, json!({ "message": "reactionKey is required", "code": "INVALID_INPUT" })));
}
