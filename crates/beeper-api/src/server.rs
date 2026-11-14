use crate::codec::{base64_decode, base64_encode_standard, sha1, url_decode};
use crate::http::{
    http_date, parse_query, read_message, read_ws_frame, weak_etag, write_ws_frame, HttpMessage, StatusCode,
};
use beeper_core::cloud::{Cloud, CloudError, CodeResult};
use beeper_core::config::{
    millis_from_iso, millis_from_rfc3339, network_spec, now_rfc3339_millis, APP_BUNDLE_ID, APP_NAME, APP_VERSION,
    NETWORK_SPECS, OPENAPI_SPEC_JSON,
};
use beeper_core::connect::{READ, WRITE};
use beeper_core::matrix::{self, ChatFlag, Outgoing};
use beeper_core::models::*;
use beeper_core::platform::{path_to_file_url, OsKind};
use beeper_core::store::{activity_key, tokenize, BeeperStore, SharedStore, StoredAsset};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::{BinaryHeap, HashMap};
use std::fmt::Write;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod auth;
mod mcp;
mod view;

struct Response {
    status: StatusCode,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

type Reply = Result<Response, Response>;

/// The `Content-Type` of `res.json` in Express.
const JSON: &str = "application/json; charset=utf-8";

fn json_with<T: Serialize + ?Sized>(status: StatusCode, value: &T) -> Response {
    let body = serde_json::to_vec(value).expect("serialize JSON response");
    Response { status, headers: vec![("content-type", JSON.into())], body }
}

fn ok<T: Serialize + ?Sized>(value: &T) -> Reply {
    Ok(json_with(StatusCode::OK, value))
}

fn empty(status: StatusCode, headers: Vec<(&'static str, String)>) -> Reply {
    Ok(Response { status, headers, body: Vec::new() })
}

/// Desktop's `BeeperConnectError` body, in its key order.
#[derive(Serialize)]
struct ApiError<'a> {
    message: &'a str,
    code: &'a str,
    #[serde(skip_serializing_if = "Value::is_null")]
    details: Value,
}

fn error(status: StatusCode, message: &str, code: &str) -> Response {
    error_with(status, message, code, Value::Null)
}

fn error_with(status: StatusCode, message: &str, code: &str, details: Value) -> Response {
    json_with(status, &ApiError { message, code, details })
}

#[derive(Serialize)]
struct Issues {
    issues: Vec<Issue>,
}

/// A zod issue, with its keys in the order of zod.
struct Issue(Vec<(&'static str, Value)>);

impl Serialize for Issue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(self.0.iter().map(|(k, v)| (k, v)))
    }
}

/// The type name of zod for a JSON value, or `undefined` for a missing value.
fn zod_type(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

/// The zod `invalid_type` issue, with `keys` in the order of the zod type that makes it.
fn type_issue(keys: [&'static str; 3], expected: &str, v: Option<&Value>, path: Value) -> Issue {
    let received = zod_type(v);
    let message = match received {
        "undefined" => "Required".to_string(),
        r => format!("Expected {expected}, received {r}"),
    };
    let value = |k| match k {
        "code" => json!("invalid_type"),
        "expected" => json!(expected),
        _ => json!(received),
    };
    Issue(keys.map(|k| (k, value(k))).into_iter().chain([("path", path), ("message", message.into())]).collect())
}

/// The zod issue of a string field, if the value is not a string.
fn string_issue(v: Option<&Value>, optional: bool, path: Value) -> Option<Issue> {
    let ok = matches!(v, Some(Value::String(_))) || (optional && v.is_none());
    (!ok).then(|| type_issue(["code", "expected", "received"], "string", v, path))
}

/// The issues of Desktop's zod schema `CreateChatInput` for a request body.
fn create_chat_issues(b: &Value) -> Vec<Issue> {
    let field = |k: &'static str, optional| string_issue(b.get(k), optional, json!([k]));
    let mut issues: Vec<Issue> =
        [field("accountID", false), field("messageText", true)].into_iter().flatten().collect();
    match b.get("type") {
        Some(Value::String(t)) if t == "single" || t == "group" => {}
        Some(Value::String(t)) => issues.push(Issue(vec![
            ("received", json!(t)),
            ("code", json!("invalid_enum_value")),
            ("options", json!(["single", "group"])),
            ("path", json!(["type"])),
            ("message", format!("Invalid enum value. Expected 'single' | 'group', received '{t}'").into()),
        ])),
        v => issues.push(type_issue(["expected", "received", "code"], "'single' | 'group'", v, json!(["type"]))),
    }
    let count = b.get("participantIDs").and_then(Value::as_array).map(Vec::len);
    match b.get("participantIDs") {
        Some(Value::Array(ids)) if ids.is_empty() => issues.push(Issue(vec![
            ("code", json!("too_small")),
            ("minimum", json!(1)),
            ("type", json!("array")),
            ("inclusive", json!(true)),
            ("exact", json!(false)),
            ("message", json!("Array must contain at least 1 element(s)")),
            ("path", json!(["participantIDs"])),
        ])),
        Some(Value::Array(ids)) => {
            let items = ids.iter().enumerate();
            issues.extend(items.filter_map(|(i, v)| string_issue(Some(v), false, json!(["participantIDs", i]))));
        }
        v => issues.push(type_issue(["code", "expected", "received"], "array", v, json!(["participantIDs"]))),
    }
    issues.extend(field("title", true));
    // zod runs the refinement when only the `too_small` issue, which does not abort, is present.
    if issues.len() == (count == Some(0)) as usize && b["type"] == "single" && count != Some(1) {
        issues.push(Issue(vec![
            ("code", json!("custom")),
            ("message", json!("'single' chat must have exactly one participantID.")),
            ("path", json!(["participantIDs"])),
        ]));
    }
    issues
}

fn bad(message: &str) -> Response {
    error(StatusCode::BAD_REQUEST, message, "VALIDATION_ERROR")
}

fn missing(message: &str) -> Response {
    error(StatusCode::NOT_FOUND, message, "NOT_FOUND")
}

/// The required-input check of Desktop's tools.
fn required(k: &str) -> Response {
    error(StatusCode::BAD_REQUEST, &format!("{k} is required"), "INVALID_INPUT")
}

/// Desktop's body for an error that is not a `BeeperConnectError`.
fn failed(message: &str) -> Response {
    error(StatusCode(500), message, "UNKNOWN_ERROR")
}

fn no_route() -> Response {
    error(StatusCode::NOT_FOUND, "Not found", "not_found")
}

fn store_error(e: &'static str) -> Response {
    match e.starts_with("Cannot overwrite") {
        true => error(StatusCode::CONFLICT, e, "DRAFT_NOT_EMPTY"),
        false => missing(e),
    }
}

fn page<T: Serialize>(items: Vec<T>, oldest_cursor: Option<String>, newest_cursor: Option<String>) -> Reply {
    ok(&CursorPage { items, has_more: false, oldest_cursor, newest_cursor })
}

/// The `limit` items of a page from an iterator, newest first, and whether the iterator has more items.
fn take_page<T>(items: impl Iterator<Item = T>, limit: usize) -> (Vec<T>, bool) {
    let mut items: Vec<T> = items.take(limit + 1).collect();
    let more = items.len() > limit;
    items.truncate(limit);
    (items, more)
}

/// The items of lists in ascending order, newest first: a merge from the ends of the lists.
fn newest_first<'a>(lists: Vec<&'a [usize]>, key: impl Fn(usize) -> &'a str + 'a) -> impl Iterator<Item = usize> + 'a {
    let mut heap: BinaryHeap<(&str, &[usize])> = lists.into_iter().filter_map(|l| Some((key(*l.last()?), l))).collect();
    std::iter::from_fn(move || {
        let (_, list) = heap.pop()?;
        let (&last, rest) = list.split_last().expect("the heap has no empty lists");
        if let Some(&next) = rest.last() {
            heap.push((key(next), rest));
        }
        Some(last)
    })
}

struct Req {
    query: serde_json::Map<String, Value>,
    msg: HttpMessage,
    body: Value,
}

impl Req {
    fn q(&self, k: &str) -> Option<&str> {
        self.query.get(k).and_then(Value::as_str)
    }

    /// All the values of an array parameter, which is a repeated key or a `key[]` key.
    fn qs(&self, k: &str) -> Vec<&str> {
        match self.query.get(k) {
            Some(Value::Array(values)) => values.iter().map(|v| v.as_str().expect("a string query value")).collect(),
            v => v.and_then(Value::as_str).into_iter().collect(),
        }
    }

    /// Desktop's boolean query parameter: `true` or `false`.
    fn flag(&self, k: &str, default: bool) -> Result<bool, Response> {
        match self.q(k) {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(_) => Err(bad(&format!("{k} must be true or false"))),
        }
    }

    /// An ISO 8601 date-time query parameter, in Unix milliseconds.
    fn date(&self, k: &str) -> Result<Option<i64>, Response> {
        let ms =
            self.q(k).map(|v| millis_from_iso(v).ok_or_else(|| bad(&format!("{k} must be an ISO 8601 date-time"))));
        ms.transpose()
    }

    /// Desktop's `direction`: `before` (the default) or `after`.
    fn after(&self) -> Result<bool, Response> {
        match self.q("direction") {
            None | Some("before") => Ok(false),
            Some("after") => Ok(true),
            Some(_) => Err(bad("direction must be before or after")),
        }
    }

    fn need_q(&self, k: &str) -> Result<&str, Response> {
        self.q(k).filter(|v| !v.trim().is_empty()).ok_or_else(|| required(k))
    }

    fn str(&self, k: &str) -> Option<&str> {
        self.body[k].as_str()
    }

    fn owned(&self, k: &str) -> Option<String> {
        self.str(k).map(str::to_string)
    }

    fn need(&self, k: &str) -> Result<&str, Response> {
        self.str(k).filter(|v| !v.trim().is_empty()).ok_or_else(|| required(k))
    }

    fn parse<T: DeserializeOwned>(&self, k: &str) -> Result<T, Response> {
        serde_json::from_value(self.body[k].clone()).map_err(|e| bad(&format!("{k}: {e}")))
    }

    fn limit(&self, default: usize, max: usize) -> Result<usize, Response> {
        let limit = self.q("limit").map_or(Ok(default), |v| v.parse().map_err(|_| bad("limit must be an integer")))?;
        match (1..=max).contains(&limit) {
            true => Ok(limit),
            false => Err(bad(&format!("limit must be from 1 to {max}"))),
        }
    }
}

fn parse_body(msg: &HttpMessage) -> Option<Value> {
    Some(match msg.header("content-type") {
        _ if msg.body.is_empty() => Value::Null,
        Some(ct) if ct.starts_with("application/json") => serde_json::from_slice(&msg.body).ok()?,
        Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
            Value::Object(parse_query(std::str::from_utf8(&msg.body).ok()?))
        }
        _ => Value::Null,
    })
}

pub async fn serve_http(listener: TcpListener, store: SharedStore) {
    let cloud = {
        let cfg = &store.read().config;
        Arc::new(Cloud::new(&cfg.api_url, &cfg.homeserver))
    };
    loop {
        let (stream, _) = listener.accept().await.expect("accept TCP connection");
        tokio::spawn(handle_connection(stream, store.clone(), cloud.clone()));
    }
}

/// The limits of Desktop's body parsers: 64 KB for OAuth and setup, 100 MB for JSON uploads, 500 MB for multipart
/// uploads, and 10 MB for the other routes.
fn body_limit(head: &HttpMessage) -> usize {
    match head.path().split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>()[..] {
        ["oauth", ..] | ["register"] | ["v1", "app", "setup", ..] => 64 << 10,
        ["v1", "assets", "upload"] if head.header("content-type").is_some_and(|t| t.contains("application/json")) => {
            100 << 20
        }
        ["v1", "assets", "upload"] => 500 << 20,
        _ => 10 << 20,
    }
}

/// The origin of a request that Desktop's CORS allows: an `http:` or `https:` origin, on all routes except
/// `/focus` and `/deeplink`.
fn cors_origin(head: &HttpMessage) -> Option<String> {
    let origin = head.header("origin")?;
    let blocked = ["/focus", "/deeplink"]
        .iter()
        .any(|p| head.path().strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')));
    let web = url::Url::parse(origin).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
    (web && !blocked).then(|| origin.into())
}

const EXPOSE_HEADERS: (&str, &str) = ("Access-Control-Expose-Headers", "WWW-Authenticate,X-Beeper-Desktop-Version");

/// The answer of the `cors` package to a preflight request.
fn preflight(origin: String) -> Response {
    let headers = [
        ("Access-Control-Allow-Methods", "GET,POST,PUT,PATCH,DELETE,OPTIONS"),
        (
            "Access-Control-Allow-Headers",
            "Authorization,Content-Type,MCP-Protocol-Version,X-Beeper-Access-Token,X-Stainless-MCP",
        ),
        ("Access-Control-Max-Age", "600"),
        EXPOSE_HEADERS,
        ("Content-Length", "0"),
    ];
    let mut cors =
        vec![("X-Powered-By", "Express".into()), ("Access-Control-Allow-Origin", origin), ("Vary", "Origin".into())];
    cors.extend(headers.map(|(k, v)| (k, v.into())));
    Response { status: StatusCode::NO_CONTENT, headers: cors, body: Vec::new() }
}

/// Adds the headers of the CORS and version middleware of Desktop, and the `ETag` of Express, which answers a
/// conditional GET of a fresh body with 304.
fn express(resp: Response, origin: Option<String>, if_none_match: Option<&str>) -> Response {
    let Response { mut status, headers: route, mut body } = resp;
    let mut headers = vec![("X-Powered-By", "Express".into())];
    if let Some(origin) = origin {
        headers.extend([("Access-Control-Allow-Origin", origin), ("Vary", "Origin".into())]);
        headers.push((EXPOSE_HEADERS.0, EXPOSE_HEADERS.1.into()));
    }
    headers.push(("X-Beeper-Desktop-Version", APP_VERSION.into()));
    let sent = route
        .iter()
        .any(|(k, v)| *k == "content-type" && ["application/json", "text/html"].iter().any(|t| v.starts_with(t)));
    headers.extend(route);
    let tag = sent.then(|| weak_etag(&body));
    let fresh = tag.as_ref().is_some_and(|tag| if_none_match.is_some_and(|m| m.split(',').any(|t| t.trim() == tag)));
    if fresh && status.0 / 100 == 2 {
        status = StatusCode::NOT_MODIFIED;
        headers.retain(|(k, _)| *k != "content-type");
        body.clear();
    }
    if ![StatusCode::NO_CONTENT, StatusCode::NOT_MODIFIED].contains(&status) {
        headers.push(("Content-Length", body.len().to_string()));
    }
    headers.extend(tag.map(|tag| ("ETag", tag)));
    Response { status, headers, body }
}

/// The `keepAliveTimeout` of Desktop's server.
const KEEP_ALIVE: std::time::Duration = std::time::Duration::from_secs(360);

/// Node keeps an HTTP/1.1 connection open unless the client asks to close it, and an HTTP/1.0 connection only when
/// the client asks to keep it open.
fn keep_alive(msg: &HttpMessage) -> bool {
    let asks =
        |t: &str| msg.header("connection").is_some_and(|c| c.split(',').any(|v| v.trim().eq_ignore_ascii_case(t)));
    if msg.start_line.ends_with(" HTTP/1.0") {
        asks("keep-alive")
    } else {
        !asks("close")
    }
}

/// The checks of Node's HTTP parser on a request line, and the `Host` header that HTTP/1.1 needs.
fn valid_request(msg: &HttpMessage) -> bool {
    let [method, target, version] = msg.start_line.split(' ').collect::<Vec<_>>()[..] else { return false };
    let digit = |s: &str| s.len() == 1 && s.bytes().all(|b| b.is_ascii_digit());
    let version_ok =
        version.strip_prefix("HTTP/").and_then(|v| v.split_once('.')).is_some_and(|(a, b)| digit(a) && digit(b));
    let method_ok = !method.is_empty() && method.bytes().all(|b| b.is_ascii_uppercase() || b == b'-');
    method_ok && !target.is_empty() && version_ok && (version != "HTTP/1.1" || msg.header("host").is_some())
}

/// Desktop's PAS gives these paths to the API server, and ends all other requests with an empty 200.
fn api_path(msg: &HttpMessage) -> bool {
    const ROOTS: [&str; 8] = [".well-known", "oauth", "register", "v0", "v1", "_matrix", "deeplink", "focus"];
    let target = msg.start_line.split(' ').nth(1).expect("a valid request line");
    let Some(rest) = target.strip_prefix('/') else { return false };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    match &rest[..end] {
        "" => !rest.starts_with('/'),
        root => ROOTS.contains(&root),
    }
}

async fn write_response(stream: &mut TcpStream, resp: Response, keep_alive: bool, with_body: bool) -> bool {
    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status.0, resp.status.reason());
    resp.headers.iter().for_each(|(k, v)| write!(head, "{k}: {v}\r\n").unwrap());
    let connection = if keep_alive { "keep-alive\r\nKeep-Alive: timeout=360" } else { "close" };
    write!(head, "Date: {}\r\nConnection: {connection}\r\n\r\n", http_date(beeper_core::config::now_unix_secs()))
        .unwrap();
    let body = if with_body { resp.body } else { Vec::new() };
    stream.write_all(&[head.into_bytes(), body].concat()).await.is_ok()
}

/// Closes a connection after the server refuses a request: the server discards the rest of the request until the
/// client closes, so that the client gets the response and not a reset.
async fn linger<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(writer: &mut W, reader: &mut R) {
    writer.shutdown().await.ok();
    tokio::time::timeout(LINGER, tokio::io::copy(reader, &mut tokio::io::sink())).await.ok();
}

/// The `closeTimeout` of the `ws` package.
const LINGER: std::time::Duration = std::time::Duration::from_secs(30);

async fn handle_connection(mut stream: TcpStream, store: SharedStore, cloud: Arc<Cloud>) {
    let mut buf = Vec::new();
    loop {
        let mut origin = None;
        let limit = |head: &HttpMessage| {
            origin = cors_origin(head);
            body_limit(head)
        };
        let Ok(read) = tokio::time::timeout(KEEP_ALIVE, read_message(&mut stream, &mut buf, limit)).await else {
            return;
        };
        let read = read.and_then(|msg| if valid_request(&msg) { Ok(msg) } else { Err(Some(StatusCode::BAD_REQUEST)) });
        let mut msg = match read {
            Ok(msg) => msg,
            Err(None) => return,
            Err(Some(StatusCode::PAYLOAD_TOO_LARGE)) => {
                let resp = error(StatusCode::PAYLOAD_TOO_LARGE, "request entity too large", "UNKNOWN_ERROR");
                write_response(&mut stream, express(resp, origin, None), false, true).await;
                let (mut reader, mut writer) = stream.split();
                linger(&mut writer, &mut reader).await;
                return;
            }
            Err(Some(status)) => {
                let head = format!("HTTP/1.1 {} {}\r\nConnection: close\r\n\r\n", status.0, status.reason());
                stream.write_all(head.as_bytes()).await.ok();
                let (mut reader, mut writer) = stream.split();
                linger(&mut writer, &mut reader).await;
                return;
            }
        };
        let keep_alive = keep_alive(&msg);
        if !api_path(&msg) {
            let empty =
                Response { status: StatusCode::OK, headers: vec![("Content-Length", "0".into())], body: Vec::new() };
            if !write_response(&mut stream, empty, keep_alive, true).await || !keep_alive {
                return;
            }
            continue;
        }
        if msg.start_line.starts_with("GET /v1/ws ") && msg.header_is("upgrade", "websocket") {
            let refused = auth::authorize(&store.read(), auth::access_token(&msg), &[beeper_core::connect::READ]).err();
            if let Some((status, message)) = refused {
                let head = format!("HTTP/1.1 {status} {message}\r\nConnection: close\r\n\r\n");
                stream.write_all(head.as_bytes()).await.ok();
                return;
            }
            return serve_ws(stream, buf, msg, store).await;
        }
        // Express answers HEAD with the response of GET, without the body.
        let head_only = msg.method() == "HEAD";
        if head_only {
            msg.start_line.replace_range(..4, "GET");
        }
        let resp = match origin {
            Some(origin) if msg.method() == "OPTIONS" => preflight(origin),
            origin => {
                let if_none_match = (msg.method() == "GET").then(|| msg.header("if-none-match")).flatten();
                let if_none_match = if_none_match.map(String::from);
                let (store, cloud) = (store.clone(), cloud.clone());
                let resp = tokio::task::spawn_blocking(move || respond(&store, &cloud, msg).unwrap_or_else(|e| e))
                    .await
                    .expect("request handler panicked");
                express(resp, origin, if_none_match.as_deref())
            }
        };
        if !write_response(&mut stream, resp, keep_alive, !head_only).await || !keep_alive {
            return;
        }
    }
}

fn respond(store: &SharedStore, cloud: &Cloud, msg: HttpMessage) -> Reply {
    let mut parts = msg.start_line.split(' ');
    let (method, target) = (parts.next().expect("a method"), parts.next().expect("a request target"));
    let method = method.to_string();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let segs: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(url_decode)
        .collect::<Option<_>>()
        .ok_or_else(|| bad("bad path"))?;
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    if segs == ["v0", "mcp"] {
        let token = auth::access_token(&msg);
        if method != "OPTIONS" && auth::authorize(&store.read(), token, &[]).is_err() {
            return Err(auth::mcp_refusal(&store.read(), &msg));
        }
        return mcp::handle(store, cloud, &method, &msg, token);
    }
    let query = parse_query(query);
    let body = parse_body(&msg).ok_or_else(|| bad("malformed request body"))?;
    let req = Req { query, msg, body };
    let in_query = if segs == ["v1", "assets", "serve"] { req.q("dangerouslyUseTokenInQuery") } else { None };
    let granted = match auth::scope(&method, &segs) {
        Some(scope) => {
            let token = auth::access_token(&req.msg).or(in_query);
            Some(auth::authorize(&store.read(), token, &[scope]).map_err(auth::refusal)?)
        }
        None => None,
    };
    if let Some(reply) = auth::route(store, &method, &segs, &req, granted.as_ref()) {
        return reply;
    }
    if let ("POST", ["v1", "app", "setup", step @ ("start" | "email" | "response" | "register")]) =
        (method.as_str(), &segs[..])
    {
        auth::setup_origin(&req.msg)?;
        return sign_in_step(store, cloud, step, &req);
    }
    if let ("GET" | "POST", ["v1", "app", "setup", rest @ ..]) = (method.as_str(), &segs[..]) {
        return e2ee_setup(store, &method, rest, &req);
    }
    if method == "GET" {
        return get(store, &segs, &req);
    }
    let mut st = store.write();
    let reply = dispatch(&mut st, &method, &segs, &req);
    if st.start_flush() {
        let s = store.clone();
        tokio::spawn(async move {
            if let Err(e) = beeper_core::matrix::flush(s.clone()).await {
                s.write().matrix_error = Some(e);
            }
        });
    }
    reply
}

/// Desktop's `{verification, session}` body of the verification routes.
#[derive(Serialize)]
struct Progress<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<&'a AppVerification>,
    session: AppState,
}

/// The progress body. A route that `requires` a verification fails without one.
fn progress(store: &SharedStore, requires: bool) -> Reply {
    let st = store.read();
    let verification = st.app_verification();
    if requires && verification.is_none() {
        return Err(verification_not_found());
    }
    ok(&Progress { verification, session: st.app_state() })
}

/// Desktop's app setup state and its encrypted messaging setup routes, which act on the Matrix client of a
/// signed-in account. Each route checks the token first, like Desktop's `requireAuth`.
fn e2ee_setup(store: &SharedStore, method: &str, path: &[&str], r: &Req) -> Reply {
    let scopes: &[&str] = if method == "GET" { &[READ] } else { &[READ, WRITE] };
    let authorized = || auth::authorize(&store.read(), auth::access_token(&r.msg), scopes).map_err(auth::refusal);
    let signed_in = || {
        authorized()?;
        match store.read().signed_in() {
            true => Ok(()),
            false => Err(error(StatusCode::CONFLICT, "Sign in to Beeper Desktop first", "needs_login")),
        }
    };
    let not_ready = || {
        signed_in()?;
        match store.read().beeper_state.app_state() {
            "ready" => Err(error(StatusCode::CONFLICT, "Beeper Desktop is already set up", "already_ready")),
            _ => Ok(()),
        }
    };
    let rt = tokio::runtime::Handle::current();
    match (method, path) {
        ("GET", []) => {
            if store.read().signed_in() {
                authorized()?;
            }
            ok(&store.read().app_state())
        }
        ("POST", ["verification", "recovery-key"]) => {
            not_ready()?;
            let key = r.need("recoveryKey")?.to_string();
            rt.block_on(matrix::recover(store.clone(), key)).map_err(|e| {
                match e.to_lowercase().contains("invalid recovery key") {
                    true => error(StatusCode::BAD_REQUEST, "The recovery key is invalid", "invalid_recovery_code"),
                    false => failed(&e),
                }
            })?;
            progress(store, false)
        }
        ("POST", ["verification", "recovery-key", "reset"] | ["verification", "recovery-key", "reset", "confirm"]) => {
            not_ready()?;
            Err(failed("Reset of the recovery key is not supported"))
        }
        ("GET", ["verifications"]) => {
            signed_in()?;
            ok(&json!({ "items": store.read().app_verification().into_iter().collect::<Vec<_>>() }))
        }
        ("POST", ["verifications"]) => {
            signed_in()?;
            let purpose = match r.str("purpose") {
                None | Some("device") => "device",
                Some("login") => "login",
                Some(_) => return Err(bad("purpose must be login or device")),
            };
            if r.str("userID").is_some_and(|u| u != store.read().beeper_state.user_id) {
                return Err(failed("Verification of other users is not supported"));
            }
            if let Some(v) = store.read().app_verification().filter(|v| !matches!(v.state, "done" | "cancelled")) {
                let details = json!({ "verificationID": v.id, "state": v.state });
                return Err(error_with(
                    StatusCode::CONFLICT,
                    "A verification is already in progress",
                    "verification_in_progress",
                    details,
                ));
            }
            rt.block_on(matrix::start_verification(store, purpose)).map_err(|e| failed(&e))?;
            progress(store, true)
        }
        ("GET", ["verifications", id]) => {
            signed_in()?;
            let found = store.read().app_verification().is_some_and(|v| v.id == *id);
            match found {
                true => progress(store, true),
                false => Err(verification_not_found()),
            }
        }
        ("POST", ["verifications", "qr", "scan"]) => {
            signed_in()?;
            Err(failed("QR code verification is not supported"))
        }
        ("POST", ["verifications", id, action @ ..]) => {
            let action = match action {
                ["accept"] => "accept",
                ["cancel"] => "cancel",
                ["sas", "start"] => "sas.start",
                ["sas", "confirm"] => "sas.confirm",
                ["qr", "confirm-scanned"] => "qr.confirmScanned",
                _ => return Err(no_route()),
            };
            signed_in()?;
            available(&store.read(), id, action)?;
            match action {
                "accept" => rt.block_on(matrix::accept_verification(store, id)).map_err(|e| failed(&e))?,
                "cancel" => rt.block_on(matrix::cancel_verification(store, id)).map_err(|e| failed(&e))?,
                "sas.confirm" => {
                    let tx = store.write().sas_confirm.take().ok_or_else(verification_not_found)?;
                    tx.send(true).ok();
                }
                // beeper-rs starts the emoji comparison when the request is ready.
                "sas.start" => {}
                _ => unreachable!("beeper-rs has no QR code verification that allows {action}"),
            }
            progress(store, true)
        }
        _ => Err(no_route()),
    }
}

/// Desktop's check that the current verification has the `id` and allows the `action`.
fn available(st: &BeeperStore, id: &str, action: &str) -> Result<(), Response> {
    let v = st.app_verification();
    if v.is_some_and(|v| v.id == id && v.available_actions.contains(&action)) {
        return Ok(());
    }
    let actions = v.map_or(&Vec::new(), |v| &v.available_actions).clone();
    let mut details = json!({ "action": action, "verificationID": id, "availableActions": actions });
    if let Some(v) = v {
        details["state"] = v.state.into();
    }
    let message = "Verification action is not available in the current app state";
    Err(error_with(StatusCode::CONFLICT, message, "verification_action_not_available", details))
}

fn verification_not_found() -> Response {
    error(StatusCode::NOT_FOUND, "Verification not found", "verification_not_found")
}

/// The GET routes only read the store, so they share its lock with each other and with the UI.
fn get(store: &SharedStore, path: &[&str], r: &Req) -> Reply {
    if path == ["v1", "assets", "serve"] {
        let file = asset_file(&store.read(), r)?;
        return serve_asset(file, r);
    }
    let st = &*store.read();
    match path {
        ["v1", "info"] => ok(&info(st)),
        ["v1", "spec"] => Ok(Response {
            status: StatusCode::OK,
            headers: vec![("content-type", JSON.into())],
            body: OPENAPI_SPEC_JSON.as_bytes().to_vec(),
        }),
        [] => empty(StatusCode::FOUND, vec![("location", "/v1/info".into())]),
        ["v1", "accounts"] => ok(&st.accounts),
        ["v1", "accounts", id, rest @ ..] => account(st, id, rest, r),
        ["v1", "labels"] => ok(&st.labels),
        ["v1", "bridges", rest @ ..] => bridge_get(st, rest),
        ["v1", "chats"] => list_chats(st, r),
        ["v1", "chats", "search"] => search_chats(st, r),
        ["v1", "chats", id, rest @ ..] => {
            let idx = st.resolve_chat_idx(id).ok_or_else(|| missing("Chat not found"))?;
            match rest {
                [] => {
                    let max = r.q("maxParticipantCount").map_or(Ok(100), |v| match v.parse::<i64>() {
                        Ok(n @ -1..=500) => Ok(n),
                        _ => Err(bad("maxParticipantCount must be an integer from -1 to 500")),
                    })?;
                    ok(&view::chat(st, &st.chats[idx], usize::try_from(max).ok(), false))
                }
                ["messages"] => list_messages(st, idx, r),
                ["messages", mid] => {
                    ok(&view::message(&st.messages[st.message_in_chat(id, mid).map_err(store_error)?.1]))
                }
                _ => Err(no_route()),
            }
        }
        ["v1", "messages", "search"] => search_messages(st, r),
        ["v1", "search"] => unified_search(st, r),
        _ => Err(no_route()),
    }
}

fn dispatch(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    match (method, path) {
        (_, ["v1", "bridges", rest @ ..]) => bridges(st, method, rest, r),
        (_, ["v1", "chats", rest @ ..]) => chats(st, method, rest, r),
        ("POST", ["v1", "assets", "upload"]) => upload_multipart(st, r),
        ("POST", ["v1", "assets", "upload", "base64"]) => {
            let bytes = base64_decode(r.need("content")?).filter(|b| !b.is_empty());
            let bytes = bytes.ok_or_else(|| bad("content must be non-empty base64"))?;
            ok(&asset_json(st.store_uploaded_asset(r.need("fileName")?.into(), r.need("mimeType")?.into(), &bytes)))
        }
        ("POST", ["v1", "assets", "download"]) => {
            let url = r.need("url")?;
            let failed = || json_with(StatusCode::OK, &json!({ "error": asset_failure(url).1 }));
            let (path, _) = st.resolve_asset_path(url).ok_or_else(failed)?;
            ok(&json!({ "srcURL": path_to_file_url(&path) }))
        }
        ("POST", ["v1", "focus"]) => focus(st, r),
        _ => Err(no_route()),
    }
}

fn info(st: &BeeperStore) -> InfoResponse {
    let base_url = st.config.base_url();
    let oauth = |p: &str| format!("{base_url}/oauth/{p}");
    InfoResponse {
        app: InfoApp { bundle_id: APP_BUNDLE_ID.into(), name: APP_NAME.into(), version: APP_VERSION.into() },
        endpoints: InfoEndpoints {
            mcp: format!("{base_url}/v0/mcp"),
            oauth: InfoOAuthEndpoints {
                authorization_endpoint: oauth("authorize"),
                introspection_endpoint: oauth("introspect"),
                registration_endpoint: oauth("register"),
                revocation_endpoint: oauth("revoke"),
                token_endpoint: oauth("token"),
                userinfo_endpoint: oauth("userinfo"),
            },
            spec: format!("{base_url}/v1/spec"),
            ws_events: format!("{base_url}/v1/ws"),
        },
        platform: InfoPlatform {
            // Node's `process.arch`.
            arch: match std::env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                "x86" => "ia32",
                other => other,
            }
            .into(),
            os: OsKind::current().node_platform_str().into(),
            release: "7.0.14".into(),
        },
        server: InfoServer {
            hostname: st.config.host.clone(),
            mcp_enabled: true,
            port: st.config.port,
            remote_access: st.config.remote,
            status: "running".into(),
            base_url,
        },
    }
}

fn sign_in_step(store: &SharedStore, cloud: &Cloud, step: &str, r: &Req) -> Reply {
    let already = || error(StatusCode::CONFLICT, "Beeper Desktop is already signed in", "already_logged_in");
    if store.read().signed_in() {
        return Err(already());
    }
    let upstream = |e: CloudError| match e.status {
        401 => error(StatusCode::UNAUTHORIZED, &e.message, "invalid_sign_in_code"),
        _ => failed(&e.message),
    };
    let jwt = match step {
        "start" => {
            let (request, methods) = cloud.start_login().map_err(upstream)?;
            if !methods.iter().any(|m| m == "email") {
                return Err(error(
                    StatusCode::CONFLICT,
                    "Email sign-in is not available right now",
                    "email_sign_in_not_supported",
                ));
            }
            return ok(&json!({ "setupRequestID": request, "signInMethods": methods }));
        }
        "email" => {
            cloud.send_login_email(r.need("setupRequestID")?, r.need("email")?).map_err(upstream)?;
            return empty(StatusCode::NO_CONTENT, Vec::new());
        }
        "response" => match cloud.send_login_code(r.need("setupRequestID")?, r.need("response")?).map_err(upstream)? {
            CodeResult::Token(jwt) => jwt,
            CodeResult::Registration { lead_token, username_suggestions } => {
                return ok(&json!({
                    "registrationRequired": true,
                    "setupRequestID": r.need("setupRequestID")?,
                    "leadToken": lead_token,
                    "usernameSuggestions": username_suggestions,
                    "copy": {
                        "title": "Choose your username",
                        "usernamePlaceholder": "Username",
                        "submit": "Continue",
                        "terms": "By continuing, you agree to the Terms of Use and acknowledge the Privacy Policy."
                    }
                }));
            }
        },
        "register" => {
            if r.body["acceptTerms"] != true {
                return Err(bad("acceptTerms must be true"));
            }
            let (request, lead) = (r.need("setupRequestID")?, r.need("leadToken")?);
            cloud.register(request, lead, r.need("username")?).map_err(upstream)?
        }
        other => unreachable!("sign-in step {other}"),
    };
    let (session, profile, accounts) = cloud.finish(&jwt).map_err(upstream)?;
    let mut st = store.write();
    if st.signed_in() {
        return Err(already());
    }
    st.sign_in(session, cloud.homeserver.clone(), &profile, accounts);
    let s = &st.beeper_state;
    let matrix = json!({ "userID": s.user_id, "deviceID": s.device_id, "accessToken": s.access_token, "homeserver": s.homeserver });
    ok(&json!({ "matrix": matrix, "session": st.app_state() }))
}

fn account(st: &BeeperStore, id: &str, rest: &[&str], r: &Req) -> Reply {
    let account = st.accounts.iter().find(|a| a.account_id == id).ok_or_else(|| missing("Account not found"))?;
    let users = st.chats.iter().filter(|c| c.account_id == id).flat_map(|c| &c.participants.items);
    match rest {
        [] => ok(account),
        ["contacts"] => {
            let q = r.q("query").unwrap_or_default().to_lowercase();
            let fields = |p: &User| [p.full_name.clone(), p.username.clone(), p.email.clone(), p.phone_number.clone()];
            let hit = |p: &&User| q.is_empty() || fields(p).iter().flatten().any(|f| f.to_lowercase().contains(&q));
            ok(&json!({ "items": users.filter(hit).collect::<Vec<_>>() }))
        }
        ["contacts", "list"] => page(users.filter(|p| p.is_self != Some(true)).collect(), None, None),
        _ => Err(no_route()),
    }
}

fn login_step(id: &str, step_type: &str, instructions: Option<String>) -> BridgeLoginStep {
    BridgeLoginStep {
        step_id: id.into(),
        step_type: step_type.into(),
        instructions,
        user_input: None,
        display_and_wait: None,
        cookies: None,
        complete: None,
    }
}

fn bridge_get(st: &BeeperStore, path: &[&str]) -> Reply {
    let [id, rest @ ..] = path else {
        return ok(&json!({ "items": NETWORK_SPECS.iter().map(|s| view::bridge(st, s)).collect::<Vec<_>>() }));
    };
    let spec = network_spec(id).ok_or_else(|| missing("Bridge not found"))?;
    let name = spec.display_name;
    match rest {
        [] => ok(&view::bridge(st, spec)),
        ["capabilities"] => ok(&json!({
            "bridgeID": spec.bridge_id,
            "bridgeType": spec.bridge_type,
            "capabilities": {
                "reactions": true,
                "replies": true,
                "edits": true,
                "deletes": true,
                "typingNotifications": true,
                "readReceipts": true,
                "polls": spec.supports_polls,
                "e2ee": spec.supports_e2ee,
                "smsMms": spec.supports_sms_mms,
                "onDevice": spec.supports_on_device,
                "cloud": spec.supports_cloud
            }
        })),
        ["login-flows"] => {
            let flows: Vec<BridgeLoginFlow> = [
                ("qr_login", format!("Scan {name} QR Code"), "Link your account by scanning a QR code"),
                (
                    "user_input_login",
                    format!("{name} Credentials / Phone"),
                    "Sign in with phone number, username, or token",
                ),
                (
                    "cookie_import",
                    format!("Import {name} Browser Session"),
                    "One-click browser cookie & session import",
                ),
                ("on_device_local", format!("On-Device {name} Bridge"), "Direct local Mautrix bridge connection"),
            ]
            .into_iter()
            .take(if spec.supports_on_device { 4 } else { 3 })
            .map(|(id, name, d)| BridgeLoginFlow { id: id.into(), name, description: Some(d.into()) })
            .collect();
            ok(&json!({ "items": flows }))
        }
        ["logins"] => ok(&json!({ "items": view::logins(st, spec.bridge_id) })),
        ["logins", login_id] => {
            let login = view::logins(st, spec.bridge_id).into_iter().find(|l| l.login_id == *login_id);
            ok(&login.ok_or_else(|| missing("Bridge login not found"))?)
        }
        ["login-sessions", sid] => {
            let session = st.bridge_sessions.get(*sid).filter(|s| s.bridge_id == *id);
            ok(session.ok_or_else(|| missing("Bridge login session not found"))?)
        }
        _ => Err(no_route()),
    }
}

fn bridges(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    let [id, rest @ ..] = path else { return Err(no_route()) };
    let spec = network_spec(id).ok_or_else(|| missing("Bridge not found"))?;
    let name = spec.display_name;
    let session_exists = |st: &BeeperStore, sid: &str| st.bridge_sessions.get(sid).is_some_and(|s| s.bridge_id == *id);
    match (method, rest) {
        ("DELETE", ["logins", login_id]) => {
            let pos = st.bridge_accounts.iter().position(|b| b.bridge_id == *id && b.login_id == *login_id);
            st.bridge_accounts.remove(pos.ok_or_else(|| missing("Bridge login not found"))?);
            st.rebuild_accounts_from_bridges();
            st.persist_state_to_sqlite();
            ok(&json!({ "bridge": view::bridge(st, spec) }))
        }
        ("POST", ["login-sessions"]) => {
            let session_id = st.next_id("lsess");
            let flow_id = r.need("flowID")?;
            let step = match flow_id {
                "qr_login" => BridgeLoginStep {
                    display_and_wait: Some(BridgeLoginStepDisplay {
                        data: Some(format!("BEEPER_BRIDGE_QR:{id}:{session_id}")),
                        message: Some("Waiting for mobile scan...".into()),
                    }),
                    ..login_step(
                        "step_qr_wait",
                        "display_and_wait",
                        Some(format!("Scan this QR code in your {name} mobile app")),
                    )
                },
                "cookie_import" => BridgeLoginStep {
                    cookies: Some(BridgeLoginStepCookies {
                        names: vec!["session_token".into(), "auth_id".into()],
                        url: format!("https://{id}.com"),
                    }),
                    ..login_step("step_cookies", "cookies", Some(format!("Import session cookies for {name}")))
                },
                "user_input_login" | "on_device_local" if flow_id == "user_input_login" || spec.supports_on_device => {
                    let fields = [
                        ("username", "Phone or Username", "string"),
                        ("code", "Verification Code / Password", "password"),
                    ]
                    .map(|(id, name, t)| BridgeLoginField { id: id.into(), name: name.into(), field_type: t.into() })
                    .to_vec();
                    BridgeLoginStep {
                        user_input: Some(BridgeLoginStepUserInput { fields }),
                        ..login_step("step_credentials", "user_input", Some(format!("Enter {name} login credentials")))
                    }
                }
                _ => return Err(bad("unknown flowID")),
            };
            let session = BridgeLoginSession {
                login_session_id: session_id.clone(),
                bridge_id: spec.bridge_id.into(),
                flow_id: flow_id.into(),
                step,
            };
            st.bridge_sessions.insert(session_id, session.clone());
            ok(&session)
        }
        ("DELETE", ["login-sessions", sid]) if session_exists(st, sid) => {
            st.bridge_sessions.remove(*sid);
            ok(&json!({ "bridgeID": spec.bridge_id, "loginSessionID": sid, "status": "cancelled" }))
        }
        ("POST", ["login-sessions", sid, "steps", _]) if session_exists(st, sid) => {
            let mode: BridgeMode = r.parse("mode")?;
            let login_id = st.next_id(&format!("login_{id}"));
            st.bridge_accounts.retain(|b| b.bridge_id != *id);
            st.bridge_accounts.push(BridgeAccountInfo {
                account_id: format!("acc_{id}"),
                bridge_id: spec.bridge_id.into(),
                bridge_type: spec.bridge_type.into(),
                is_running: true,
                login_id: login_id.clone(),
                mode,
                network_name: name.into(),
                state: "CONNECTED".into(),
                remote_name: None,
                remote_profile: None,
                remote_error: None,
            });
            st.rebuild_accounts_from_bridges();
            st.persist_state_to_sqlite();
            let session = BridgeLoginSession {
                login_session_id: sid.to_string(),
                bridge_id: spec.bridge_id.into(),
                flow_id: "completed".into(),
                step: BridgeLoginStep {
                    complete: Some(BridgeLoginStepComplete { user_login_id: login_id }),
                    ..login_step("complete", "complete", None)
                },
            };
            st.bridge_sessions.insert(sid.to_string(), session.clone());
            ok(&session)
        }
        (_, ["login-sessions", ..]) => Err(missing("Bridge login session not found")),
        _ => Err(no_route()),
    }
}

/// One page of chats from keys in `chat_order` order, newest first, with Desktop's chat cursors
/// `<last activity ms>:<chat>`. Merged member chats show only in their merged chat, like Desktop.
fn chat_page<'a>(
    st: &'a BeeperStore,
    r: &Req,
    keys: impl Iterator<Item = (i64, &'a str)>,
    keep: impl Fn(&Chat) -> bool,
    limit: usize,
) -> Reply {
    let chats = keys.map(|k| (k, &st.chats[st.chat_idx_by_id[k.1]]));
    let (chats, has_more) = take_page(chats.filter(|(_, c)| c.merged_into_chat_id.is_none() && keep(c)), limit);
    let cursor = |(ms, id): (i64, &str)| Some(format!("{ms}:{id}"));
    let input = || r.q("cursor").map(str::to_string);
    let oldest_cursor = chats.last().map_or_else(input, |&(k, _)| cursor(k));
    let newest_cursor = chats.first().map_or_else(input, |&(k, _)| cursor(k));
    let items = chats.into_iter().map(|(_, c)| view::chat(st, c, Some(20), true)).collect();
    ok(&CursorPage { items, has_more, oldest_cursor, newest_cursor })
}

/// The `chat_order` key of a chat cursor.
fn chat_cursor(r: &Req) -> Result<Option<(i64, String)>, Response> {
    let key = |c: &str| c.split_once(':').and_then(|(ms, id)| Some((ms.parse().ok()?, id.to_string())));
    r.q("cursor").map(|c| key(c).ok_or_else(|| bad("invalid cursor"))).transpose()
}

/// The `chat_order` keys newest first, past the cursor. Both directions take the newest chats past the cursor, like
/// Desktop's `ORDER BY timestamp DESC LIMIT`.
fn chat_keys<'a>(
    st: &'a BeeperStore,
    cursor: Option<&(i64, String)>,
    after: bool,
) -> impl Iterator<Item = (i64, &'a str)> {
    let keys = match cursor {
        None => st.chat_order.range::<(i64, String), _>(..),
        Some(c) if after => st.chat_order.range((Excluded(c), Unbounded)),
        Some(c) => st.chat_order.range(..c),
    };
    keys.rev().map(|(ms, id)| (*ms, id.as_str()))
}

/// Desktop's `listChats`: the chats by last activity, newest first.
fn list_chats(st: &BeeperStore, r: &Req) -> Reply {
    let (after, limit, accounts, cursor) = (r.after()?, r.limit(25, 200)?, r.qs("accountIDs"), chat_cursor(r)?);
    let keep = |c: &Chat| accounts.is_empty() || accounts.contains(&c.account_id.as_str());
    chat_page(st, r, chat_keys(st, cursor.as_ref(), after), keep, limit)
}

fn chats(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    match (method, path) {
        ("POST", []) => create_chat(st, r),
        ("POST", ["start"]) => {
            let account_id = r.need("accountID")?;
            let user = &r.body["user"];
            let uid = ["id", "phoneNumber", "username", "email"].iter().find_map(|k| user[k].as_str());
            let uid = uid.ok_or_else(|| bad("user identifier is required"))?;
            let existing = st.chats.iter().find(|c| {
                c.account_id == account_id
                    && c.chat_type == "single"
                    && c.participants.items.iter().any(|p| p.id == uid)
            });
            if let Some(c) = existing {
                return ok(&json!({ "chatID": c.id, "status": "existing" }));
            }
            let name = user["fullName"].as_str().map(str::to_string);
            let (chat, status) = st.create_or_start_chat(
                account_id,
                "single",
                name,
                vec![uid.into()],
                r.owned("messageText"),
                None,
                None,
            );
            ok(&json!({ "chatID": chat.id, "status": status }))
        }
        (_, [id, rest @ ..]) => {
            let idx = st.resolve_chat_idx(id).ok_or_else(|| missing("Chat not found"))?;
            chat(st, method, idx, id, rest, r)
        }
        _ => Err(no_route()),
    }
}

fn create_chat(st: &mut BeeperStore, r: &Req) -> Reply {
    let (chat, status) = match r.str("mode").unwrap_or("create") {
        "merge" => {
            let merge = &r.body["merge"];
            let chat_ids: Vec<String> =
                serde_json::from_value(merge["chatIDs"].clone()).map_err(|_| bad("merge.chatIDs is required"))?;
            let default_chat_id = merge["defaultChatID"].as_str().map(str::to_string);
            check_merge(st, &chat_ids, default_chat_id.as_deref())?;
            let lead = default_chat_id.as_deref().unwrap_or(&chat_ids[0]);
            let account = st.chats[st.resolve_chat_idx(lead).expect("check_merge found the chat")].account_id.clone();
            let title = r.need("title")?.into();
            st.create_or_start_chat(&account, "merged", Some(title), Vec::new(), None, Some(chat_ids), default_chat_id)
        }
        "create" => {
            let issues = create_chat_issues(&r.body);
            if !issues.is_empty() {
                let details = json!(Issues { issues });
                let body = ApiError { message: "Invalid input", code: "VALIDATION_ERROR", details };
                return Err(json_with(StatusCode::BAD_REQUEST, &body));
            }
            let participants = r.parse("participantIDs")?;
            let (account_id, chat_type) = (r.need("accountID")?, r.need("type")?);
            let (title, text) = (r.owned("title"), r.owned("messageText"));
            st.create_or_start_chat(account_id, chat_type, title, participants, text, None, None)
        }
        _ => return Err(bad("mode must be create or merge")),
    };
    ok(&json!({ "chatID": chat.id, "status": status }))
}

/// The member chats of a merged chat exist.
fn check_merge(st: &BeeperStore, chat_ids: &[String], default_chat_id: Option<&str>) -> Result<(), Response> {
    if chat_ids.is_empty() {
        return Err(bad("merge.chatIDs is required"));
    }
    let mut ids = chat_ids.iter().map(String::as_str).chain(default_chat_id);
    ids.all(|id| st.resolve_chat_idx(id).is_some()).then_some(()).ok_or_else(|| missing("Chat not found"))
}

/// Whether each word starts a word of the title and the network of the chat, or with `people` of the names of its
/// participants: Desktop's `searchTitlesOrNetworkDetailed` and `searchParticipantsDetailed`.
fn chat_hit(c: &Chat, people: bool, words: &[String]) -> bool {
    let fields: Vec<&String> = match people {
        true => c.participants.items.iter().flat_map(|p| [&p.full_name, &p.custom_nickname]).flatten().collect(),
        false => [Some(&c.title), c.network_name.as_ref()].into_iter().flatten().collect(),
    };
    let tokens = tokenize(&fields.into_iter().map(String::as_str).collect::<Vec<_>>().join(" "));
    words.iter().all(|w| tokens.iter().any(|t| t.starts_with(w.as_str())))
}

/// Desktop's `searchChats`: the chats by last activity, newest first. The default scope `titles` matches the title
/// and the network, and the scope `participants` matches the names of the participants.
fn search_chats(st: &BeeperStore, r: &Req) -> Reply {
    let (mut mask, mut want) = match r.q("inbox") {
        None => (0, 0),
        Some("archive") => (CHAT_ARCHIVED, CHAT_ARCHIVED),
        Some("low-priority") => (CHAT_LOW_PRIORITY, CHAT_LOW_PRIORITY),
        Some("primary") => (CHAT_ARCHIVED | CHAT_LOW_PRIORITY, 0),
        Some(_) => return Err(bad("inbox must be primary, low-priority, or archive")),
    };
    if r.flag("unreadOnly", false)? {
        (mask, want) = (mask | CHAT_UNREAD, want | CHAT_UNREAD);
    }
    if !r.flag("includeMuted", true)? {
        mask |= CHAT_MUTED;
    }
    let people = match r.q("scope") {
        None | Some("titles") => false,
        Some("participants") => true,
        Some(_) => return Err(bad("scope must be titles or participants")),
    };
    let chat_type = r.q("type").filter(|&t| t != "any");
    let (after, limit, accounts, label) = (r.after()?, r.limit(50, 200)?, r.qs("accountIDs"), r.q("labelID"));
    let (active_before, active_after, cursor) =
        (r.date("lastActivityBefore")?, r.date("lastActivityAfter")?, chat_cursor(r)?);
    let keep = |c: &Chat| {
        let ms = || activity_key(c).0;
        c.flags & mask == want
            && chat_type.is_none_or(|t| c.chat_type == t)
            && (accounts.is_empty() || accounts.contains(&c.account_id.as_str()))
            && label.is_none_or(|l| c.label_ids.iter().any(|id| id == l))
            && active_before.is_none_or(|b| ms() < b)
            && active_after.is_none_or(|a| ms() > a)
    };
    let Some(query) = r.q("query").map(str::trim).filter(|q| !q.is_empty()) else {
        return chat_page(st, r, chat_keys(st, cursor.as_ref(), after), keep, limit);
    };
    let words = tokenize(query);
    let hit = |c: &Chat| chat_hit(c, people, &words);
    let past =
        |k: (i64, &str)| cursor.as_ref().is_none_or(|c| k != (c.0, c.1.as_str()) && (k > (c.0, c.1.as_str())) == after);
    let mut keys: Vec<(i64, &str)> = st
        .chat_search_index
        .query_all_tokens(query)
        .into_iter()
        .map(|i| &st.chats[i as usize])
        .filter(|c| hit(c))
        .map(|c| (activity_key(c).0, c.id.as_str()))
        .filter(|&k| past(k))
        .collect();
    keys.sort_unstable_by(|a, b| b.cmp(a));
    chat_page(st, r, keys.into_iter(), keep, limit)
}

fn chat(st: &mut BeeperStore, method: &str, idx: usize, id: &str, rest: &[&str], r: &Req) -> Reply {
    match (method, rest) {
        ("PATCH", []) => {
            let draft = r.body.get("draft");
            let new_draft = draft.filter(|d| !d.is_null()).map(|d| draft_input(st, d)).transpose()?;
            let new_merge: Option<Option<ChatMergeInfo>> = r.body.get("merge").map(|_| r.parse("merge")).transpose()?;
            if let Some(Some(m)) = &new_merge {
                check_merge(st, &m.chat_ids, m.default_chat_id.as_deref())?;
            }
            let fields = ChatFields::parse(&r.body)?;
            if fields.expiry.is_some() && st.chats[idx].timers.is_none() {
                return Err(bad("messageExpirySeconds: the chat has no disappearing messages"));
            }
            st.patch_chat(id, draft.is_some_and(Value::is_null), new_draft, new_merge).map_err(store_error)?;
            fields.apply(st, idx);
            chat_reply(st, idx)
        }
        ("POST", ["archive"]) => {
            let archived = r.body["archived"].as_bool().unwrap_or(true);
            st.set_chat_flag(idx, ChatFlag::Archived, Some(archived));
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("POST", ["read"]) => {
            if let Some(ev) = st.mark_chat_read(id, true) {
                let cid = st.chats[idx].id.clone();
                st.queue(&cid, Outgoing::Read(ev));
            }
            chat_reply(st, idx)
        }
        ("POST", ["unread"]) => {
            st.set_chat_flag(idx, ChatFlag::MarkedUnread, Some(true));
            chat_reply(st, idx)
        }
        ("POST", ["notify-anyway"]) => {
            st.update_chat(idx, |c| c.flags |= CHAT_NOTIFY_ANYWAY);
            chat_reply(st, idx)
        }
        ("POST", ["reminders"]) => {
            let reminder = &r.body["reminder"];
            // Desktop's `Sp` also takes the older `remindAtMs`.
            let at = match (&reminder["remindAt"], &reminder["remindAtMs"]) {
                (Value::String(at), _) => millis_from_iso(at),
                (Value::Null, ms) => ms.as_i64(),
                _ => None,
            };
            let invalid =
                || error(StatusCode::BAD_REQUEST, "reminder.remindAt must be a valid ISO timestamp", "INVALID_INPUT");
            let dismiss = match &reminder["dismissOnIncomingMessage"] {
                Value::Null => false,
                v => v.as_bool().ok_or_else(|| bad("reminder.dismissOnIncomingMessage must be a boolean"))?,
            };
            let reminder = ChatReminder { remind_at_ms: at.ok_or_else(invalid)?, dismiss_on_incoming_message: dismiss };
            st.set_reminder(idx, Some(reminder));
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("DELETE", ["reminders"]) => {
            st.set_reminder(idx, None);
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("POST", ["messages"]) => send_message(st, id, r),
        ("PUT", ["messages", mid]) => {
            let text = r.need("text")?;
            let (cid, msg) = st.edit_message(id, mid, text.into()).map_err(store_error)?;
            st.queue(&cid, Outgoing::Edit { target: msg.id.clone(), text: text.into(), mentions: Vec::new() });
            ok(&json!({ "chatID": cid, "messageID": msg.id, "success": true }))
        }
        ("DELETE", ["messages", mid]) => {
            let cid = st.delete_message(id, mid).map_err(store_error)?;
            st.queue(&cid, Outgoing::Redact { target: (*mid).into() });
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("POST", ["messages", mid, "reactions"]) => {
            let key = r.need("reactionKey")?;
            let (cid, tx) = st.add_reaction(id, mid, key, r.owned("transactionID")).map_err(store_error)?;
            st.queue(&cid, Outgoing::React { txn: tx.clone(), target: (*mid).into(), key: key.into() });
            ok(&json!({ "chatID": cid, "messageID": mid, "reactionKey": key, "transactionID": tx, "success": true }))
        }
        ("DELETE", ["messages", mid, "reactions", key]) => {
            let (cid, target) = st.remove_reaction(id, mid, key).map_err(store_error)?;
            st.queue(&cid, Outgoing::Redact { target });
            ok(&json!({ "chatID": cid, "messageID": mid, "reactionKey": key, "success": true }))
        }
        _ => Err(no_route()),
    }
}

/// Desktop's page size of `getRoomMessages`.
const MESSAGE_PAGE: usize = 20;

/// Desktop's `listMessages`: one page of the chat and its merged member chats, newest first. The cursor is a sort
/// key, and both directions take the newest messages past it, like Desktop's `ORDER BY hsOrder DESC LIMIT 20`.
fn list_messages(st: &BeeperStore, idx: usize, r: &Req) -> Reply {
    let (after, cursor) = (r.after()?, r.q("cursor"));
    let chat = &st.chats[idx];
    let members = chat.merge.iter().flat_map(|m| &m.chat_ids);
    let member_ids = members.map(|id| &st.chats[st.resolve_chat_idx(id).expect("merged member chat exists")].id);
    let key = |m: usize| st.messages[m].sort_key.as_str();
    let lists = std::iter::once(&chat.id).chain(member_ids).map(|cid| {
        let list = st.chat_msg_indices[cid].as_slice();
        match cursor {
            None => list,
            Some(c) if after => &list[list.partition_point(|&m| key(m) <= c)..],
            Some(c) => &list[..list.partition_point(|&m| key(m) < c)],
        }
    });
    let (items, has_more) = take_page(newest_first(lists.collect(), key).map(|m| &st.messages[m]), MESSAGE_PAGE);
    let (oldest, newest) = (items.last().map(|m| m.sort_key.clone()), items.first().map(|m| m.sort_key.clone()));
    let items = items.into_iter().map(view::message).collect();
    ok(&CursorPage { items, has_more, oldest_cursor: oldest, newest_cursor: newest })
}

fn upload_not_found(upload_id: &str) -> Response {
    let message = format!("Upload not found: {upload_id}. File may have expired.");
    error(StatusCode::NOT_FOUND, &message, "UPLOAD_NOT_FOUND")
}

/// Desktop's reply of the chat routes that change a chat: the chat with at most 100 participants.
fn chat_reply(st: &BeeperStore, idx: usize) -> Reply {
    ok(&view::chat(st, &st.chats[idx], Some(100), false))
}

/// Desktop's `ChatDraftInput`: the text, and the attachments keyed by upload ID.
fn draft_input(st: &BeeperStore, v: &Value) -> Result<ChatDraft, Response> {
    let text = match &v["text"] {
        Value::Null => None,
        Value::String(t) => Some(t.clone()),
        _ => return Err(bad("draft.text must be a string")),
    };
    let ids = match &v["attachments"] {
        Value::Null => Vec::new(),
        Value::Object(m) => m.keys().collect(),
        _ => return Err(bad("draft.attachments must be an object keyed by upload ID")),
    };
    let attachment = |id: &String| {
        let a = st.assets.get(id).ok_or_else(|| upload_not_found(id))?;
        let (file_name, mime_type) = (Some(a.file_name.clone()), Some(a.mime_type.clone()));
        Ok(ChatDraftAttachment { upload_id: id.clone(), file_name, mime_type })
    };
    let attachments = ids.into_iter().map(attachment).collect::<Result<_, Response>>()?;
    Ok(ChatDraft { text, attachments, ..ChatDraft::default() })
}

/// The fields of Desktop's `patchChat` besides `draft`, checked before the chat changes.
struct ChatFields {
    title: Option<String>,
    description: Option<String>,
    avatar: Option<Option<(String, Vec<u8>, String)>>,
    flags: Vec<(ChatFlag, bool)>,
    expiry: Option<u64>,
}

impl ChatFields {
    fn parse(b: &Value) -> Result<Self, Response> {
        let text = |k: &str| match b.get(k) {
            None => Ok(None),
            Some(Value::String(t)) => Ok(Some(t.clone())),
            Some(Value::Null) if k == "description" => Ok(Some(String::new())),
            Some(_) => Err(bad(&format!("{k} must be a string"))),
        };
        let avatar = match b.get("imgURL") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(url)) => Some(Some(avatar_file(url)?)),
            Some(_) => return Err(bad("imgURL must be a string")),
        };
        let mut flags = Vec::new();
        // The pin goes first, because the store does not archive a pinned chat.
        for (k, flag) in [
            ("isPinned", ChatFlag::Pinned),
            ("isArchived", ChatFlag::Archived),
            ("isMuted", ChatFlag::Muted),
            ("isLowPriority", ChatFlag::LowPriority),
        ] {
            match b.get(k) {
                None => {}
                Some(Value::Bool(on)) => flags.push((flag, *on)),
                Some(_) => return Err(bad(&format!("{k} must be a boolean"))),
            }
        }
        let expiry = match b.get("messageExpirySeconds") {
            None => None,
            Some(Value::Null) => Some(0),
            Some(v) => Some(v.as_u64().ok_or_else(|| bad("messageExpirySeconds must be a whole number or null"))?),
        };
        Ok(Self { title: text("title")?, description: text("description")?, avatar, flags, expiry })
    }

    fn apply(self, st: &mut BeeperStore, idx: usize) {
        let id = st.chats[idx].id.clone();
        if self.title.is_some() || self.description.is_some() || self.avatar.is_some() {
            st.update_group(&id, self.title, self.description, self.avatar);
        }
        for (flag, on) in self.flags {
            st.set_chat_flag(idx, flag, Some(on));
        }
        if let Some(seconds) = self.expiry {
            st.set_disappearing(&id, seconds * 1000);
        }
    }
}

/// The file URL, the bytes, and the MIME type of a new chat avatar from an absolute path or a `file://` URL.
fn avatar_file(url: &str) -> Result<(String, Vec<u8>, String), Response> {
    let path = match url.starts_with("file://") {
        true => url::Url::parse(url).ok().and_then(|u| u.to_file_path().ok()),
        false => Some(std::path::PathBuf::from(url)).filter(|p| p.is_absolute()),
    };
    let path = path.ok_or_else(|| bad("imgURL must be an absolute path or a file URL"))?;
    let mime = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => return Err(bad("imgURL must be a PNG, JPEG, GIF, or WebP image")),
    };
    let bytes = std::fs::read(&path).map_err(|e| bad(&format!("imgURL: {e}")))?;
    Ok((path_to_file_url(&path), bytes, mime.into()))
}

fn send_message(st: &mut BeeperStore, id: &str, r: &Req) -> Reply {
    let attachment = match &r.body["attachment"] {
        Value::Null => None,
        a => {
            let upload_id = a["uploadID"]
                .as_str()
                .ok_or_else(|| error(StatusCode::BAD_REQUEST, "uploadID is required", "MISSING_UPLOAD_ID"))?;
            let stored = st.assets.get(upload_id).ok_or_else(|| upload_not_found(upload_id))?;
            let stored = stored.clone();
            let mime = a["mimeType"].as_str().map_or(stored.mime_type, str::to_string);
            let kind = match mime.split('/').next() {
                Some("image") => "img",
                Some("video") => "video",
                Some("audio") => "audio",
                _ => "unknown",
            };
            Some(Attachment {
                attachment_type: kind.into(),
                id: Some(upload_id.into()),
                duration: a["duration"].as_f64(),
                file_name: Some(a["fileName"].as_str().map_or(stored.file_name, str::to_string)),
                file_size: Some(stored.file_size),
                is_gif: a["isGif"].as_bool(),
                is_sticker: a["isSticker"].as_bool(),
                is_voice_note: a["isVoiceNote"].as_bool(),
                mime_type: Some(mime),
                poster_img: None,
                size: None,
                src_url: Some(stored.mxc_url),
                transcript: None,
            })
        }
    };
    let (text, reply_to) = (r.owned("text"), r.owned("replyToMessageID"));
    if !st.config.demo && (attachment.is_some() || text.is_none()) {
        return Err(bad("only text messages can be sent to Matrix"));
    }
    let (chat_id, pending, _) = st.send_message(id, text.clone(), reply_to.clone(), attachment).map_err(store_error)?;
    if let Some(text) = text {
        st.queue(
            &chat_id,
            Outgoing::Send { txn: pending.clone(), text, reply_to, preview: None, mentions: Vec::new() },
        );
    }
    ok(&json!({ "chatID": chat_id, "pendingMessageID": pending }))
}

/// Desktop's `searchMessages`: the newest messages that match the query and the filters, at most 20 for each page,
/// with the chats of the messages. The cursor is a sort key.
fn search_messages(st: &BeeperStore, r: &Req) -> Reply {
    let media = r.qs("mediaTypes");
    if let Some(m) = media.iter().find(|m| !["any", "video", "image", "link", "file"].contains(m)) {
        return Err(bad(&format!("unknown mediaTypes value {m}")));
    }
    let chat_type = r.q("chatType");
    if chat_type.is_some_and(|t| t != "group" && t != "single") {
        return Err(bad("chatType must be group or single"));
    }
    let (after, cursor, limit, sender) = (r.after()?, r.q("cursor"), r.limit(20, 20)?, r.q("sender"));
    let (date_after, date_before) = (r.date("dateAfter")?, r.date("dateBefore")?);
    if date_after.zip(date_before).is_some_and(|(a, b)| a >= b) {
        return Err(bad("'dateAfter' must be earlier than 'dateBefore' when both are provided."));
    }
    let (exclude_low, include_muted, accounts) =
        (r.flag("excludeLowPriority", true)?, r.flag("includeMuted", true)?, r.qs("accountIDs"));
    let mut rooms = Vec::new();
    for id in r.qs("chatIDs") {
        let chat = &st.chats[st.resolve_chat_idx(id).ok_or_else(|| missing("Chat not found"))?];
        match &chat.merge {
            Some(m) => {
                rooms.extend(m.chat_ids.iter().map(|id| st.resolve_canonical_chat_id(id).expect("member exists")))
            }
            None => rooms.push(chat.id.clone()),
        }
    }
    let kind = |m: &Message, k: &str| m.attachments.iter().any(|a| a.attachment_type == k);
    // The message filters read only the message, and the chat filters also read its chat.
    let message_ok = |&m: &usize| {
        let m = &st.messages[m];
        let ms = || millis_from_rfc3339(&m.timestamp).expect("RFC 3339 message time");
        m.flags & MSG_DELETED == 0
            && !matches!(m.message_type.as_deref(), Some("HIDDEN" | "REACTION"))
            && match sender {
                None => true,
                Some("me") => m.is_sender == Some(true),
                Some("others") => m.is_sender != Some(true),
                Some(s) => m.sender_id == s,
            }
            && date_after.is_none_or(|a| ms() > a)
            && date_before.is_none_or(|b| ms() < b)
            && (media.is_empty()
                || (media.contains(&"any") && !m.attachments.is_empty())
                || (media.contains(&"image") && kind(m, "img"))
                || (media.contains(&"video") && kind(m, "video"))
                || (media.contains(&"file") && kind(m, "unknown"))
                || (media.contains(&"link") && !m.links.is_empty()))
    };
    let chat_ok = |&m: &usize| {
        let c = &st.chats[st.chat_idx_by_id[&st.messages[m].chat_id]];
        (accounts.is_empty() || accounts.contains(&c.account_id.as_str()))
            && chat_type.is_none_or(|t| c.chat_type == t)
            && !(exclude_low && c.flags & CHAT_LOW_PRIORITY != 0)
            && (include_muted || c.flags & CHAT_MUTED == 0)
    };
    let key = |m: usize| st.messages[m].sort_key.as_str();
    let past = |k: &str| cursor.is_none_or(|c| if after { k > c } else { k < c });
    let hits: Box<dyn Iterator<Item = usize>> =
        match (r.q("query").map(str::trim).filter(|q| !q.is_empty()), &rooms[..]) {
            (None, [_, ..]) => {
                let lists = rooms.iter().map(|cid| {
                    let list = st.chat_msg_indices[cid].as_slice();
                    match cursor {
                        None => list,
                        Some(c) if after => &list[list.partition_point(|&m| key(m) <= c)..],
                        Some(c) => &list[..list.partition_point(|&m| key(m) < c)],
                    }
                });
                Box::new(newest_first(lists.collect(), key).filter(message_ok).filter(chat_ok))
            }
            (query, _) => {
                // A text query has few hits, so the filters run after the sort. Without one, the message filters run
                // first, because they drop most messages and need no chat.
                let room = |m: usize| rooms.is_empty() || rooms.contains(&st.messages[m].chat_id);
                let mut hits: Vec<usize> = match query {
                    Some(q) => st.search_messages(q),
                    None => (0..st.messages.len()).filter(message_ok).collect(),
                };
                hits.retain(|&m| past(key(m)) && room(m));
                hits.sort_unstable_by(|&a, &b| key(b).cmp(key(a)));
                Box::new(hits.into_iter().filter(message_ok).filter(chat_ok))
            }
        };
    let (items, has_more) = take_page(hits.map(|m| &st.messages[m]), limit);
    let chat = |id: &str| view::chat(st, &st.chats[st.chat_idx_by_id[id]], None, false);
    let chats: HashMap<&str, view::ApiChat> = items.iter().map(|m| (m.chat_id.as_str(), chat(&m.chat_id))).collect();
    let (oldest, newest) = (items.last().map(|m| &m.sort_key), items.first().map(|m| &m.sort_key));
    let items: Vec<_> = items.into_iter().map(view::message).collect();
    ok(&json!({ "items": items, "chats": chats, "hasMore": has_more, "oldestCursor": oldest, "newestCursor": newest }))
}

/// Desktop's `search`: at most 30 chats that match by title, 30 groups that match by participant, and the first page of
/// 20 messages of `searchMessages`, each newest first.
fn unified_search(st: &BeeperStore, r: &Req) -> Reply {
    let q = r.need_q("query")?;
    let words = tokenize(q);
    let mut found: Vec<&Chat> =
        st.chat_search_index.query_all_tokens(q).into_iter().map(|i| &st.chats[i as usize]).collect();
    found.sort_unstable_by_key(|c| std::cmp::Reverse(activity_key(c)));
    let titles = found.iter().copied().filter(|c| chat_hit(c, false, &words));
    let chats: Vec<_> = titles.take(30).map(|c| view::chat(st, c, Some(20), false)).collect();
    let groups = found.iter().copied().filter(|c| c.chat_type == "group" && chat_hit(c, true, &words));
    let in_groups: Vec<_> = groups.take(30).map(|c| view::chat(st, c, Some(20), false)).collect();
    let mut hits: Vec<&Message> = st.search_messages(q).into_iter().map(|i| &st.messages[i]).collect();
    hits.sort_unstable_by(|a, b| b.sort_key.cmp(&a.sort_key));
    let shown = |m: &&&Message| {
        let c = &st.chats[st.chat_idx_by_id[&m.chat_id]];
        m.flags & MSG_DELETED == 0 && c.flags & CHAT_LOW_PRIORITY == 0
    };
    let (messages, has_more) = take_page(hits.iter().filter(shown), 20);
    let chat = |id: &str| view::chat(st, &st.chats[st.chat_idx_by_id[id]], None, false);
    let referenced: HashMap<&str, view::ApiChat> =
        messages.iter().map(|m| (m.chat_id.as_str(), chat(&m.chat_id))).collect();
    ok(&json!({
        "results": {
            "chats": chats,
            "in_groups": in_groups,
            "messages": {
                "chats": referenced,
                "hasMore": has_more,
                "items": messages.iter().map(|m| view::message(m)).collect::<Vec<_>>(),
                "newestCursor": messages.first().map(|m| &m.sort_key),
                "oldestCursor": messages.last().map(|m| &m.sort_key)
            }
        }
    }))
}

fn asset_json(asset: StoredAsset) -> Value {
    json!({
        "uploadID": asset.upload_id,
        "fileName": asset.file_name,
        "mimeType": asset.mime_type,
        "fileSize": asset.file_size,
        "srcURL": asset.mxc_url,
        "width": asset.width,
        "height": asset.height
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn upload_multipart(st: &mut BeeperStore, r: &Req) -> Reply {
    let boundary = r.msg.header("content-type").and_then(|ct| ct.split_once("boundary="));
    let delim = format!("\r\n--{}", boundary.ok_or_else(|| bad("multipart boundary is required"))?.1.trim_matches('"'));
    let body = [b"\r\n", r.msg.body.as_slice()].concat();
    let mut rest = body.as_slice();
    let (mut file, mut fields) = (None, HashMap::new());
    while let Some(start) = find(rest, delim.as_bytes()) {
        rest = &rest[start + delim.len()..];
        let part = &rest[..find(rest, delim.as_bytes()).unwrap_or(rest.len())];
        let Some(head_end) = find(part, b"\r\n\r\n") else { continue };
        let head = std::str::from_utf8(&part[..head_end]).map_err(|_| bad("multipart headers must be UTF-8"))?;
        let content = &part[head_end + 4..];
        let attr = |key: &str| head.split(&format!("{key}=\"")).nth(1).and_then(|s| s.split('"').next());
        let content_type = head.lines().find_map(|l| {
            l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-type")).map(|(_, v)| v.trim())
        });
        match attr("name") {
            Some("file") => file = Some((attr("filename"), content_type, content)),
            Some(name) => {
                fields.insert(name, std::str::from_utf8(content).map_err(|_| bad("multipart fields must be UTF-8"))?);
            }
            None => return Err(bad("multipart part needs a name")),
        }
    }
    let (file_name, mime, content) = file.ok_or_else(|| bad("file part is required"))?;
    let file_name = fields.get("fileName").copied().or(file_name).ok_or_else(|| bad("fileName is required"))?;
    let mime = fields.get("mimeType").copied().or(mime).ok_or_else(|| bad("mimeType is required"))?;
    if content.is_empty() {
        return Err(bad("file part is empty"));
    }
    ok(&asset_json(st.store_uploaded_asset(file_name.into(), mime.into(), content)))
}

fn parse_range(header: &str, total: usize) -> Option<(usize, usize)> {
    let (s, e) = header.strip_prefix("bytes=")?.split_once('-')?;
    let last = total.checked_sub(1)?;
    let (start, end) = match (s, e) {
        ("", n) => (total.saturating_sub(n.parse().ok()?), last),
        (s, "") => (s.parse().ok()?, last),
        (s, e) => (s.parse().ok()?, e.parse::<usize>().ok()?.min(last)),
    };
    (start <= end).then_some((start, end))
}

/// Desktop's failures of `resolveMatrixAsset` and of file URLs.
fn asset_failure(url: &str) -> (StatusCode, &'static str) {
    match url {
        u if u.starts_with("file://") => (StatusCode::NOT_FOUND, "File not found"),
        u if u.starts_with("mxc://") || u.starts_with("localmxc://") => {
            (StatusCode::BAD_GATEWAY, "Failed to download asset: Not found")
        }
        _ => (StatusCode::BAD_REQUEST, "URL must be mxc:// or localmxc://"),
    }
}

/// The folders of the data folder from which Desktop serves `file://` URLs.
const ATTACHMENT_DIRS: &[&str] = &[
    "media",
    "api-uploads",
    "report_a_problem_attachments",
    "draft_attachments",
    "app_icons",
    "user_avatar",
    "quick_reply_attachments",
    "temp_avatar_uploads",
    "transcoded_cache",
    "transcribed_notes",
    "linkpreviews",
];

/// Desktop's `serveAsset` check of a `file://` URL: the real path is in an attachment folder of the data folder.
fn attachment_file(st: &BeeperStore, url: &str) -> Result<std::path::PathBuf, Response> {
    let fail = |status, message| error(status, message, "SERVE_ASSET_ERROR");
    let path = url::Url::parse(url).ok().and_then(|u| u.to_file_path().ok());
    let path = path.ok_or_else(|| fail(StatusCode::BAD_REQUEST, "Malformed file URL"))?;
    let (Ok(real), Ok(data)) = (path.canonicalize(), st.config.data_dir.canonicalize()) else {
        return Err(fail(StatusCode::NOT_FOUND, "File not found"));
    };
    match real.strip_prefix(&data).ok().and_then(|rel| rel.components().next()) {
        Some(std::path::Component::Normal(dir)) if ATTACHMENT_DIRS.iter().any(|d| dir == *d) => Ok(real),
        _ => Err(fail(StatusCode(403), "Access denied: Path must be within allowed attachment directories")),
    }
}

fn asset_file(st: &BeeperStore, r: &Req) -> Result<(std::path::PathBuf, String), Response> {
    let url = r.need_q("url")?;
    match url.starts_with("file://") {
        true => Ok((attachment_file(st, url)?, "application/octet-stream".into())),
        false => st.resolve_asset_path(url).ok_or_else(|| {
            let (status, message) = asset_failure(url);
            error(status, message, "SERVE_ASSET_ERROR")
        }),
    }
}

/// Reads the file after the caller releases the store lock.
fn serve_asset((path, mime): (std::path::PathBuf, String), r: &Req) -> Reply {
    let bytes = std::fs::read(&path).map_err(|e| {
        // Windows reports "Access is denied" for a directory.
        let message = match path.is_dir() {
            true => "EISDIR: illegal operation on a directory, read".into(),
            false => e.to_string(),
        };
        error(StatusCode(500), &message, "UNKNOWN_ERROR")
    })?;
    let mut headers = vec![("content-type", mime), ("accept-ranges", "bytes".into())];
    let Some(range) = r.msg.header("range") else {
        return Ok(Response { status: StatusCode::OK, headers, body: bytes });
    };
    let total = bytes.len();
    let (start, end) = parse_range(range, total)
        .ok_or_else(|| error(StatusCode::RANGE_NOT_SATISFIABLE, "Range Not Satisfiable", "UNKNOWN_ERROR"))?;
    headers.push(("content-range", format!("bytes {start}-{end}/{total}")));
    Ok(Response { status: StatusCode::PARTIAL_CONTENT, headers, body: bytes[start..=end].to_vec() })
}

fn focus(st: &mut BeeperStore, r: &Req) -> Reply {
    let idx = match (r.str("chatID"), r.str("messageID")) {
        (Some(id), _) => Some(st.resolve_chat_idx(id).ok_or_else(|| missing("Chat not found"))?),
        (None, Some(mid)) => {
            let &m_idx = st.msg_idx_by_id.get(mid).ok_or_else(|| missing("Message not found"))?;
            Some(st.chat_idx_by_id[&st.messages[m_idx].chat_id])
        }
        (None, None) => None,
    };
    let (text, path) = (r.owned("draftText"), r.owned("draftAttachmentPath"));
    if text.is_some() || path.is_some() {
        let idx = idx.ok_or_else(|| bad("draft needs chatID"))?;
        let attachments =
            path.into_iter().map(|p| ChatDraftAttachment { upload_id: p.clone(), file_name: Some(p), mime_type: None });
        let draft = ChatDraft { text, attachments: attachments.collect(), ..ChatDraft::default() };
        st.update_chat(idx, |c| c.draft = Some(draft));
    }
    st.focus_tx.send(idx.map(|i| st.chats[i].id.clone())).ok();
    ok(&json!({ "success": true }))
}

/// Desktop's event frame `{type, seq, ts, chatID, ids, entries?}`. A message event carries the message in `entries`.
#[derive(Serialize)]
struct WsFrame<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    seq: u64,
    ts: String,
    #[serde(rename = "chatID")]
    chat_id: &'a str,
    ids: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entries: Option<Vec<view::ApiMessage<'a>>>,
}

impl<'a> WsFrame<'a> {
    fn new(ev: &'a WsEvent, seq: u64) -> Self {
        let chat_id = ev.chat_id.as_deref().expect("store events have a chat");
        WsFrame {
            kind: &ev.event_type,
            seq,
            ts: now_rfc3339_millis(),
            chat_id,
            ids: vec![ev.message_id.as_deref().unwrap_or(chat_id)],
            entries: ev.message.as_ref().map(|m| vec![view::message(m)]),
        }
    }
}

/// Desktop's `handleClientMessage`: applies a `subscriptions.set` command, and returns its `subscriptions.updated`
/// acknowledgement or an `error` frame. The chat IDs are sorted and unique, and `*` stands for all chats.
fn subscribe(subs: &RwLock<Vec<String>>, payload: &[u8]) -> Value {
    let error = |code: &str, message: &str, request: Option<&Value>| {
        let mut e = json!({ "type": "error", "code": code, "message": message });
        if let Some(r) = request {
            e["requestID"] = r.clone();
        }
        e
    };
    let Ok(v) = serde_json::from_slice::<Value>(payload) else {
        return error("INVALID_PAYLOAD", "Invalid JSON payload", None);
    };
    let Some(command) = v.as_object().filter(|o| o.contains_key("type")) else {
        return error("INVALID_PAYLOAD", "Payload must be an object with a type field", None);
    };
    if v["type"] != "subscriptions.set" {
        let kind = v["type"].as_str().map_or_else(|| v["type"].to_string(), str::to_string);
        return error("INVALID_COMMAND", &format!("Unsupported command type: {kind}"), None);
    }
    let request = v.get("requestID").filter(|r| r.is_string());
    let invalid = |message: &str| error("INVALID_PAYLOAD", message, request);
    if let Some(k) = command.keys().find(|k| !["type", "requestID", "chatIDs", "app"].contains(&k.as_str())) {
        return invalid(&format!("Unrecognized key(s) in object: '{k}'"));
    }
    if v.get("requestID").is_some_and(|r| !r.is_string()) {
        return invalid("requestID must be a string");
    }
    let ids: Option<Vec<String>> = match &v["chatIDs"] {
        Value::Null => Some(Vec::new()),
        ids => serde_json::from_value(ids.clone()).ok(),
    };
    let Some(mut ids) = ids else { return invalid("chatIDs must be an array of strings") };
    if ids.contains(&"*".to_string()) && ids.len() > 1 {
        return invalid("chatIDs cannot combine '*' with specific IDs");
    }
    let app = match &v["app"] {
        Value::Null => false,
        Value::Object(a) if a.len() == 1 && a["state"].is_boolean() => a["state"] == true,
        _ => return invalid("app must be an object with a boolean state"),
    };
    ids.sort_unstable();
    ids.dedup();
    let mut ack = json!({ "type": "subscriptions.updated", "chatIDs": ids, "app": { "state": app } });
    if let Some(r) = request {
        ack["requestID"] = r.clone();
    }
    *subs.write().unwrap() = ids;
    ack
}

/// Desktop's `maxPayload` of the WebSocket server.
const WS_MAX_PAYLOAD: usize = 64 * 1024;

async fn serve_ws(mut stream: TcpStream, leftover: Vec<u8>, msg: HttpMessage, store: SharedStore) {
    let Some(key) = msg.header("sec-websocket-key") else { return };
    let accept = base64_encode_standard(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
    );
    if stream.write_all(handshake.as_bytes()).await.is_err() {
        return;
    }
    let mut events = store.read().ws_tx.subscribe();
    let (reader, mut writer) = stream.into_split();
    let subs = Arc::new(RwLock::new(Vec::<String>::new()));
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let frame = |v: &Value| serde_json::to_vec(v).expect("serialize a WebSocket frame");
    out_tx.send(frame(&json!({ "type": "ready", "version": 1, "chatIDs": [] }))).expect("the writer runs");
    let reader_subs = subs.clone();
    let mut reader_task = tokio::spawn(async move {
        let mut reader = std::io::Cursor::new(leftover).chain(reader);
        loop {
            let (opcode, payload) = match read_ws_frame(&mut reader, WS_MAX_PAYLOAD).await {
                Ok(frame) => frame,
                Err(close) => return (close, reader),
            };
            match opcode {
                0x1 => {
                    let reply = subscribe(&reader_subs, &payload);
                    out_tx.send(frame(&reply)).ok();
                }
                0x9 | 0xA => {}
                _ => return (None, reader),
            }
        }
    });
    let send_events = async {
        let mut seq = 0;
        loop {
            let bytes = tokio::select! {
                ev = events.recv() => {
                    let Ok(ev) = ev else { break };
                    let chat = ev.chat_id.as_deref().expect("store events have a chat");
                    let wanted = {
                        let s = subs.read().unwrap();
                        s.first().is_some_and(|f| f == "*") || s.iter().any(|c| c == chat)
                    };
                    if !wanted {
                        continue;
                    }
                    seq += 1;
                    serde_json::to_vec(&WsFrame::new(&ev, seq)).expect("serialize a WebSocket event")
                }
                Some(bytes) = out_rx.recv() => bytes,
            };
            if write_ws_frame(&mut writer, 0x1, &bytes, false).await.is_err() {
                break;
            }
        }
    };
    let close = tokio::select! {
        r = &mut reader_task => Some(r.expect("WS reader task panicked")),
        () = send_events => None,
    };
    reader_task.abort();
    if let Some((Some(code), mut reader)) = close {
        write_ws_frame(&mut writer, 0x8, &code.to_be_bytes(), false).await.ok();
        linger(&mut writer, &mut reader).await;
    }
}
