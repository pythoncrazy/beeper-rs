//! The HTTP/1.1 transport of the Desktop API server, with raw requests on a TCP connection.

use beeper_api::server::serve_http;
use beeper_core::config::CentralConfig;
use beeper_core::store::{BeeperStore, StoreLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

const BAD_REQUEST: &str = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n";

/// Starts a demo server, and returns its port and access token.
async fn server(name: &str) -> (u16, String) {
    let dir = std::env::temp_dir().join(format!("beeper-rs-http-{name}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("remove the stale test dir");
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = CentralConfig::new(dir, "127.0.0.1".into(), port, None);
    cfg.demo = true;
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
    let both = found.iter().filter(|(_, x)| x == chats[0] || x == chats[1]).count();
    let (c, d) = (url_encode(chats[0]), url_encode(chats[1]));
    assert_eq!(get(format!("/v1/messages/search?limit=20&chatIDs={c}&chatIDs={d}")).await.len(), both);
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
    assert_eq!(count, 67);
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
    assert_eq!(body(&download), json!({ "error": "URL must be mxc:// or localmxc://" }));

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
    let kept = send("DELETE", &format!("{chat}/{id}?forEveryone=false"), "", "").await;
    assert!(kept.starts_with("HTTP/1.1 400 "), "{kept}");
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
