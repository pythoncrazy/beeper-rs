use crate::config::{
    base64_decode_standard, base64_encode_standard, base64_encode_url_nopad,
    network_spec_by_bridge, now_rfc3339_millis, sha1, sha256, url_decode, APP_BUNDLE_ID, APP_NAME,
    APP_VERSION, NETWORK_SPECS, OPENAPI_SPEC_JSON,
};
use crate::models::*;
use crate::platform::OsKind;
use crate::store::*;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpListener, TcpStream,
};

pub type SharedStore = Arc<RwLock<BeeperStore>>;
pub type Bytes = Vec<u8>;

pub struct State<T>(pub T);
pub struct AxumPath<T>(pub T);
pub struct Query<T>(pub T);
pub struct Json<T>(pub T);

impl<T> std::ops::Deref for Json<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCode(pub u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const NO_CONTENT: Self = Self(204);
    pub const PARTIAL_CONTENT: Self = Self(206);
    pub const FOUND: Self = Self(302);
    pub const BAD_REQUEST: Self = Self(400);
    pub const NOT_FOUND: Self = Self(404);
    pub const CONFLICT: Self = Self(409);

    #[inline]
    pub fn as_u16(self) -> u16 {
        self.0
    }

    #[inline]
    pub fn reason_phrase(self) -> &'static str {
        match self.0 {
            101 => "Switching Protocols",
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            206 => "Partial Content",
            302 => "Found",
            400 => "Bad Request",
            404 => "Not Found",
            409 => "Conflict",
            _ => "OK",
        }
    }
}

#[derive(Clone, Default)]
pub struct HeaderMap(pub HashMap<String, String>);

pub struct HeaderValue<'a>(&'a str);

impl<'a> HeaderValue<'a> {
    #[inline]
    pub fn to_str(&self) -> Result<&'a str, std::str::Utf8Error> {
        Ok(self.0)
    }
}

impl HeaderMap {
    #[inline]
    pub fn get(&self, key: &str) -> Option<HeaderValue<'_>> {
        self.0
            .get(&key.to_ascii_lowercase())
            .map(|s| HeaderValue(s.as_str()))
    }
}

pub struct Response {
    pub status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub trait IntoResponse {
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    #[inline]
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for StatusCode {
    #[inline]
    fn into_response(self) -> Response {
        Response {
            status: self,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    #[inline]
    fn into_response(self) -> Response {
        Response {
            status: StatusCode::OK,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: serde_json::to_vec(&self.0).expect("serialize JSON response"),
        }
    }
}

impl<T: Serialize> IntoResponse for (StatusCode, Json<T>) {
    #[inline]
    fn into_response(self) -> Response {
        Response {
            status: self.0,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: serde_json::to_vec(&(self.1).0).expect("serialize JSON response"),
        }
    }
}

impl<const N: usize> IntoResponse for (StatusCode, [(&str, &str); N], Vec<u8>) {
    #[inline]
    fn into_response(self) -> Response {
        Response {
            status: self.0,
            headers: self
                .1
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: self.2,
        }
    }
}

impl<const N: usize> IntoResponse for (StatusCode, [(&str, &str); N], &str) {
    #[inline]
    fn into_response(self) -> Response {
        Response {
            status: self.0,
            headers: self
                .1
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: self.2.as_bytes().to_vec(),
        }
    }
}

pub struct Redirect;

impl Redirect {
    #[inline]
    pub fn to(uri: &str) -> Response {
        Response {
            status: StatusCode::FOUND,
            headers: vec![("location".to_string(), uri.to_string())],
            body: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct Router {
    pub store: SharedStore,
}

pub fn build_router(store: SharedStore) -> Router {
    Router { store }
}

#[inline]
fn parse_json_body(body: &[u8]) -> Json<Value> {
    let val = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body).expect("valid JSON request body")
    };
    Json(val)
}

#[inline]
fn parse_opt_json_body(body: &[u8]) -> Option<Json<Value>> {
    if body.is_empty() {
        None
    } else {
        serde_json::from_slice(body).ok().map(Json)
    }
}

impl Router {
    pub async fn dispatch(
        &self,
        method: &str,
        path: &str,
        query: HashMap<String, String>,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> Response {
        let store = self.store.clone();
        let segs: Vec<&str> = path
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        match (method, segs.as_slice()) {
            ("GET", ["v1", "info"]) => get_info(State(store)).await.into_response(),
            ("GET", ["v1", "openapi.json"]) => get_openapi_spec().await.into_response(),
            ("POST", ["oauth", "register"]) => oauth_register(State(store), parse_json_body(&body))
                .await
                .into_response(),
            ("GET", ["oauth", "authorize"]) => oauth_authorize(State(store), headers, Query(query))
                .await
                .into_response(),
            ("POST", ["oauth", "token"]) => oauth_token(State(store), headers, body)
                .await
                .into_response(),
            ("GET", ["v1", "app", "setup"]) => get_setup_status(State(store)).await.into_response(),
            ("POST", ["v1", "app", "setup", "start"]) => {
                post_setup_start(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "email"]) => {
                post_setup_email(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "response"]) => {
                post_setup_response(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "register"]) => {
                post_setup_register(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "verification", "recovery-key"]) => {
                post_recovery_key_verify(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "verification", "recovery-key", "reset"]) => {
                post_recovery_key_reset(State(store)).await.into_response()
            }
            (
                "POST",
                ["v1", "app", "setup", "verification", "recovery-key", "reset", "confirm"],
            ) => post_recovery_key_reset_confirm(State(store), parse_json_body(&body))
                .await
                .into_response(),
            ("GET", ["v1", "app", "setup", "verifications"]) => {
                get_verifications(State(store)).await.into_response()
            }
            ("POST", ["v1", "app", "setup", "verifications"]) => {
                post_verifications_start(State(store)).await.into_response()
            }
            ("POST", ["v1", "app", "setup", "verifications", "qr", "scan"]) => {
                post_verification_qr_scan(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "app", "setup", "verifications", id]) => {
                get_verification_by_id(State(store), AxumPath(url_decode(id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "verifications", id, "accept"]) => {
                post_verification_accept(State(store), AxumPath(url_decode(id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "verifications", id, "cancel"]) => {
                post_verification_cancel(State(store), AxumPath(url_decode(id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "verifications", id, "sas", "start"]) => {
                post_verification_sas_start(State(store), AxumPath(url_decode(id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "verifications", id, "sas", "confirm"]) => {
                post_verification_sas_confirm(State(store), AxumPath(url_decode(id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "app", "setup", "verifications", id, "qr", "confirm-scanned"]) => {
                post_verification_qr_confirm(State(store), AxumPath(url_decode(id)))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "accounts"]) => get_accounts(State(store)).await.into_response(),
            ("GET", ["v1", "accounts", account_id]) => {
                get_account_by_id(State(store), AxumPath(url_decode(account_id)))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "accounts", account_id, "contacts"]) => {
                search_contacts(State(store), AxumPath(url_decode(account_id)), Query(query))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "accounts", account_id, "contacts", "list"]) => {
                list_contacts(State(store), AxumPath(url_decode(account_id)))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "bridges"]) => get_bridges(State(store)).await.into_response(),
            ("GET", ["v1", "bridges", bridge_id]) => {
                get_bridge_by_id(State(store), AxumPath(url_decode(bridge_id)))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "bridges", bridge_id, "capabilities"]) => {
                get_bridge_capabilities(AxumPath(url_decode(bridge_id)))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "bridges", bridge_id, "login-flows"]) => {
                get_bridge_login_flows(AxumPath(url_decode(bridge_id)))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "bridges", bridge_id, "logins"]) => {
                get_bridge_logins(State(store), AxumPath(url_decode(bridge_id)))
                    .await
                    .into_response()
            }
            ("DELETE", ["v1", "bridges", bridge_id, "logins", login_id]) => delete_bridge_login(
                State(store),
                AxumPath((url_decode(bridge_id), url_decode(login_id))),
            )
            .await
            .into_response(),
            ("POST", ["v1", "bridges", bridge_id, "login-sessions"]) => post_bridge_login_session(
                State(store),
                AxumPath(url_decode(bridge_id)),
                parse_json_body(&body),
            )
            .await
            .into_response(),
            ("GET", ["v1", "bridges", bridge_id, "login-sessions", session_id]) => {
                get_bridge_login_session(
                    State(store),
                    AxumPath((url_decode(bridge_id), url_decode(session_id))),
                )
                .await
                .into_response()
            }
            ("DELETE", ["v1", "bridges", bridge_id, "login-sessions", session_id]) => {
                delete_bridge_login_session(
                    State(store),
                    AxumPath((url_decode(bridge_id), url_decode(session_id))),
                )
                .await
                .into_response()
            }
            (
                "POST",
                ["v1", "bridges", bridge_id, "login-sessions", session_id, "steps", step_id],
            ) => post_bridge_login_step(
                State(store),
                AxumPath((
                    url_decode(bridge_id),
                    url_decode(session_id),
                    url_decode(step_id),
                )),
                parse_json_body(&body),
            )
            .await
            .into_response(),
            ("GET", ["v1", "labels"]) => get_labels(State(store)).await.into_response(),
            ("GET", ["v1", "chats"]) => {
                list_chats(State(store), Query(query)).await.into_response()
            }
            ("POST", ["v1", "chats"]) => create_chat(State(store), parse_json_body(&body))
                .await
                .into_response(),
            ("POST", ["v1", "chats", "start"]) => start_chat(State(store), parse_json_body(&body))
                .await
                .into_response(),
            ("GET", ["v1", "chats", "search"]) => search_chats(State(store), Query(query))
                .await
                .into_response(),
            ("GET", ["v1", "chats", chat_id]) => {
                retrieve_chat(State(store), AxumPath(url_decode(chat_id)))
                    .await
                    .into_response()
            }
            ("PATCH", ["v1", "chats", chat_id]) => patch_chat_handler(
                State(store),
                AxumPath(url_decode(chat_id)),
                parse_json_body(&body),
            )
            .await
            .into_response(),
            ("POST", ["v1", "chats", chat_id, "archive"]) => archive_chat_handler(
                State(store),
                AxumPath(url_decode(chat_id)),
                parse_opt_json_body(&body),
            )
            .await
            .into_response(),
            ("POST", ["v1", "chats", chat_id, "read"]) => {
                read_chat_handler(State(store), AxumPath(url_decode(chat_id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "chats", chat_id, "unread"]) => {
                unread_chat_handler(State(store), AxumPath(url_decode(chat_id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "chats", chat_id, "notify-anyway"]) => {
                notify_anyway_handler(State(store), AxumPath(url_decode(chat_id)))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "chats", chat_id, "reminders"]) => create_reminder_handler(
                State(store),
                AxumPath(url_decode(chat_id)),
                parse_json_body(&body),
            )
            .await
            .into_response(),
            ("DELETE", ["v1", "chats", chat_id, "reminders"]) => {
                delete_reminder_handler(State(store), AxumPath(url_decode(chat_id)))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "chats", chat_id, "messages"]) => {
                list_messages(State(store), AxumPath(url_decode(chat_id)), Query(query))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "chats", chat_id, "messages"]) => send_message_handler(
                State(store),
                AxumPath(url_decode(chat_id)),
                parse_json_body(&body),
            )
            .await
            .into_response(),
            ("GET", ["v1", "chats", chat_id, "messages", message_id]) => get_message_handler(
                State(store),
                AxumPath((url_decode(chat_id), url_decode(message_id))),
            )
            .await
            .into_response(),
            ("PUT", ["v1", "chats", chat_id, "messages", message_id]) => edit_message_handler(
                State(store),
                AxumPath((url_decode(chat_id), url_decode(message_id))),
                parse_json_body(&body),
            )
            .await
            .into_response(),
            ("DELETE", ["v1", "chats", chat_id, "messages", message_id]) => delete_message_handler(
                State(store),
                AxumPath((url_decode(chat_id), url_decode(message_id))),
            )
            .await
            .into_response(),
            ("POST", ["v1", "chats", chat_id, "messages", message_id, "reactions"]) => {
                add_reaction_handler(
                    State(store),
                    AxumPath((url_decode(chat_id), url_decode(message_id))),
                    parse_json_body(&body),
                )
                .await
                .into_response()
            }
            (
                "DELETE",
                ["v1", "chats", chat_id, "messages", message_id, "reactions", reaction_key],
            ) => delete_reaction_handler(
                State(store),
                AxumPath((
                    url_decode(chat_id),
                    url_decode(message_id),
                    url_decode(reaction_key),
                )),
            )
            .await
            .into_response(),
            ("GET", ["v1", "messages", "search"]) => {
                search_messages_handler(State(store), Query(query))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "assets", "upload"]) => {
                upload_asset_multipart(State(store), headers, body)
                    .await
                    .into_response()
            }
            ("POST", ["v1", "assets", "upload", "base64"]) => {
                upload_asset_base64(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("POST", ["v1", "assets", "download"]) => {
                download_asset_handler(State(store), parse_json_body(&body))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "assets", "serve"]) => {
                serve_asset_handler(State(store), headers, Query(query))
                    .await
                    .into_response()
            }
            ("GET", ["v1", "search"]) => unified_search_handler(State(store), Query(query))
                .await
                .into_response(),
            ("POST", ["v1", "focus"]) => focus_app_handler(State(store), parse_json_body(&body))
                .await
                .into_response(),
            _ => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
        }
    }
}

pub async fn serve_http(listener: TcpListener, router: Router) {
    while let Ok((stream, _)) = listener.accept().await {
        let router_clone = router.clone();
        tokio::spawn(async move {
            handle_http_connection(stream, router_clone).await;
        });
    }
}

async fn handle_http_connection(mut stream: TcpStream, router: Router) {
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];
    loop {
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            match stream.read(&mut tmp).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
            }
        };

        let header_str =
            std::str::from_utf8(&buf[..header_end - 4]).expect("valid HTTP header utf8");
        let mut lines = header_str.split("\r\n");
        let req_line = match lines.next() {
            Some(l) if !l.is_empty() => l,
            _ => return,
        };
        let mut req_parts = req_line.split_whitespace();
        let method = req_parts.next().unwrap_or("GET").to_string();
        let target = req_parts.next().unwrap_or("/").to_string();

        let mut headers = HashMap::new();
        for line in lines {
            if let Some((k, v)) = line.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }
        let content_len: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        while buf.len() - header_end < content_len {
            match stream.read(&mut tmp).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
            }
        }

        let body = buf[header_end..header_end + content_len].to_vec();
        buf.drain(..header_end + content_len);

        let (raw_path, raw_query) = match target.split_once('?') {
            Some((p, q)) => (p, q),
            None => (target.as_str(), ""),
        };
        let mut query = HashMap::new();
        if !raw_query.is_empty() {
            for pair in raw_query.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    query.insert(url_decode(k), url_decode(v));
                }
            }
        }

        let is_ws = raw_path == "/v1/ws"
            && headers
                .get("upgrade")
                .map(|u| u.eq_ignore_ascii_case("websocket"))
                .unwrap_or(false);
        if is_ws {
            let ws_key = headers
                .get("sec-websocket-key")
                .expect("missing Sec-WebSocket-Key");
            let mut key_concat = String::with_capacity(ws_key.len() + 36);
            key_concat.push_str(ws_key);
            key_concat.push_str("258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
            let accept = base64_encode_standard(&sha1(key_concat.as_bytes()));
            let handshake = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            if stream.write_all(handshake.as_bytes()).await.is_err() {
                return;
            }
            let rx = router.store.read().unwrap().ws_tx.subscribe();
            let (reader, writer) = stream.into_split();
            handle_ws_client(reader, writer, rx).await;
            return;
        }

        let should_close = headers
            .get("connection")
            .map(|c| c.eq_ignore_ascii_case("close"))
            .unwrap_or(false);

        let resp = router
            .dispatch(&method, raw_path, query, HeaderMap(headers), body)
            .await;
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n",
            resp.status.0,
            resp.status.reason_phrase(),
            resp.body.len()
        );
        for (k, v) in &resp.headers {
            head.push_str(k);
            head.push_str(": ");
            head.push_str(v);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        if !resp.body.is_empty() && stream.write_all(&resp.body).await.is_err() {
            return;
        }
        if should_close {
            return;
        }
    }
}

async fn get_info(State(store): State<SharedStore>) -> Json<InfoResponse> {
    let st = store.read().unwrap();
    let base_url = st.config.base_url();
    Json(InfoResponse {
        app: InfoApp {
            bundle_id: APP_BUNDLE_ID.to_string(),
            name: APP_NAME.to_string(),
            version: APP_VERSION.to_string(),
        },
        endpoints: InfoEndpoints {
            mcp: "/v1/mcp".to_string(),
            oauth: InfoOAuthEndpoints {
                authorization_endpoint: format!("{base_url}/oauth/authorize"),
                introspection_endpoint: format!("{base_url}/oauth/introspect"),
                registration_endpoint: format!("{base_url}/oauth/register"),
                revocation_endpoint: format!("{base_url}/oauth/revoke"),
                token_endpoint: format!("{base_url}/oauth/token"),
                userinfo_endpoint: format!("{base_url}/oauth/userinfo"),
            },
            spec: "/v1/openapi.json".to_string(),
            ws_events: "/v1/ws".to_string(),
        },
        platform: InfoPlatform {
            arch: std::env::consts::ARCH.to_string(),
            os: OsKind::current().node_platform_str().to_string(),
            release: "7.0.14".to_string(),
        },
        server: InfoServer {
            base_url,
            hostname: st.config.host.clone(),
            mcp_enabled: true,
            port: st.config.port,
            remote_access: false,
            status: "ok".to_string(),
        },
    })
}

async fn get_openapi_spec() -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        OPENAPI_SPEC_JSON,
    )
        .into_response()
}

async fn oauth_register(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut st = store.write().unwrap();
    let client_id = st.next_id("beeper_cli_client");
    let client_name = body["client_name"]
        .as_str()
        .expect("client_name required")
        .to_string();
    let redirect_uris: Vec<String> = body["redirect_uris"]
        .as_array()
        .expect("redirect_uris required")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    st.oauth_clients.insert(
        client_id.clone(),
        OAuthClientEntry {
            client_id: client_id.clone(),
            client_name: client_name.clone(),
            redirect_uris: redirect_uris.clone(),
        },
    );
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": client_id,
            "client_name": client_name,
            "redirect_uris": redirect_uris,
            "token_endpoint_auth_method": "none"
        })),
    )
}

async fn oauth_authorize(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let mut st = store.write().unwrap();
    let client_id = params.get("client_id").expect("missing client_id").clone();
    let redirect_uri = params
        .get("redirect_uri")
        .expect("missing redirect_uri")
        .clone();
    let code_challenge = params
        .get("code_challenge")
        .expect("missing code_challenge")
        .clone();
    let code_challenge_method = params
        .get("code_challenge_method")
        .expect("missing code_challenge_method")
        .clone();
    let state_param = params.get("state").cloned().unwrap_or_default();

    let code = st.next_id("auth_code");
    st.oauth_codes.insert(
        code.clone(),
        OAuthCodeEntry {
            code: code.clone(),
            client_id,
            redirect_uri: redirect_uri.clone(),
            code_challenge,
            code_challenge_method,
        },
    );

    if headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("application/json"))
        .unwrap_or(false)
    {
        return Json(json!({ "code": code, "state": state_param, "redirect_uri": redirect_uri }))
            .into_response();
    }
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    Redirect::to(&format!(
        "{redirect_uri}{sep}code={code}&state={state_param}"
    ))
    .into_response()
}

async fn oauth_token(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Response {
    let mut st = store.write().unwrap();
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let params: HashMap<String, String> = if content_type.contains("application/json") {
        serde_json::from_slice(&body_bytes).expect("valid oauth token JSON")
    } else {
        std::str::from_utf8(&body_bytes)
            .expect("valid utf8 form body")
            .split('&')
            .filter_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                Some((url_decode(k), url_decode(v)))
            })
            .collect()
    };

    let code = params.get("code").expect("missing code");
    let verifier = params.get("code_verifier").expect("missing code_verifier");
    let entry = st
        .oauth_codes
        .remove(code)
        .expect("valid authorization code");
    assert_eq!(
        entry.code_challenge_method, "S256",
        "only S256 PKCE supported"
    );

    let computed_challenge = base64_encode_url_nopad(&sha256(verifier.as_bytes()));
    assert_eq!(
        computed_challenge, entry.code_challenge,
        "PKCE challenge mismatch"
    );

    let token = st.next_id("bp_oauth_tok");
    st.valid_tokens.insert(token.clone());
    Json(json!({ "access_token": token, "token_type": "Bearer", "scope": "read write" }))
        .into_response()
}

async fn get_setup_status(State(store): State<SharedStore>) -> Json<SetupStatusResponse> {
    Json(store.read().unwrap().setup_status.clone())
}

async fn post_setup_start(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<SetupStatusResponse> {
    let mut st = store.write().unwrap();
    let reset = body.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let env = body
        .get("env")
        .and_then(Value::as_str)
        .unwrap_or("PRODUCTION")
        .to_string();
    if reset {
        st.setup_status.session.state = "signing_in".to_string();
        st.setup_status.setup_flow = Some(SetupFlowStep {
            state: "email_required".to_string(),
            email: None,
            login_token: Some(format!("lt_{env}_01")),
            env: Some(env),
        });
    }
    Json(st.setup_status.clone())
}

async fn post_setup_email(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<SetupStatusResponse> {
    let mut st = store.write().unwrap();
    let email = body["email"].as_str().expect("email required").to_string();
    let env = body
        .get("env")
        .and_then(Value::as_str)
        .unwrap_or("PRODUCTION")
        .to_string();
    let login_token = st.next_id("login_tok");
    st.setup_status.session.state = "signing_in".to_string();
    st.setup_status.setup_flow = Some(SetupFlowStep {
        state: "email_code_required".to_string(),
        email: Some(email),
        login_token: Some(login_token),
        env: Some(env),
    });
    Json(st.setup_status.clone())
}

async fn post_setup_response(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<SetupStatusResponse> {
    let mut st = store.write().unwrap();
    let code = body["code"].as_str().expect("code required");
    assert!(!code.trim().is_empty(), "verification code cannot be empty");

    if code == "NEW_USER" {
        let email = body
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string);
        st.setup_status.setup_flow = Some(SetupFlowStep {
            state: "registration_required".to_string(),
            email,
            login_token: body
                .get("loginToken")
                .and_then(Value::as_str)
                .map(str::to_string),
            env: Some("PRODUCTION".to_string()),
        });
    } else {
        st.beeper_state.verified = false;
        st.setup_status.setup_flow = None;
        st.setup_status.session.state = "needs_verification".to_string();
        st.setup_status.session.e2ee.state = "needs_verification".to_string();
        st.setup_status.session.e2ee.verified = false;
        st.persist_state_to_sqlite();
    }
    Json(st.setup_status.clone())
}

async fn post_setup_register(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<SetupStatusResponse> {
    let mut st = store.write().unwrap();
    let username = body["username"].as_str().expect("username required");
    st.beeper_state.user_id = format!("@{username}:beeper.com");
    st.beeper_state.verified = true;
    st.setup_status.setup_flow = None;
    st.setup_status.session.state = "ready".to_string();
    st.setup_status.session.e2ee.state = "ready".to_string();
    st.setup_status.session.e2ee.verified = true;
    st.setup_status.session.matrix = Some(SetupMatrix {
        homeserver: st.beeper_state.homeserver.clone(),
        user_id: st.beeper_state.user_id.clone(),
        device_id: Some(st.beeper_state.device_id.clone()),
    });
    st.persist_state_to_sqlite();
    Json(st.setup_status.clone())
}

async fn post_recovery_key_verify(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let key = body["recoveryKey"].as_str().expect("recoveryKey required");
    assert!(!key.trim().is_empty(), "recoveryKey cannot be empty");
    st.beeper_state.secrets.recovery_code = key.to_string();
    st.beeper_state.verified = true;
    st.beeper_state.has_backed_up_code = true;
    st.setup_status.session.state = "ready".to_string();
    st.setup_status.session.e2ee.state = "ready".to_string();
    st.setup_status.session.e2ee.verified = true;
    st.setup_status.session.e2ee.has_backed_up_code = true;
    st.persist_state_to_sqlite();
    Json(json!({ "session": st.setup_status.session }))
}

async fn post_recovery_key_reset(State(store): State<SharedStore>) -> Json<Value> {
    let mut st = store.write().unwrap();
    let new_key = format!("EsTc Beeper Reset {}", st.next_id("Key"));
    st.beeper_state.secrets.recovery_code = new_key.clone();
    Json(json!({ "recoveryKey": new_key, "session": st.setup_status.session }))
}

async fn post_recovery_key_reset_confirm(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let key = body["recoveryKey"].as_str().expect("recoveryKey required");
    st.beeper_state.secrets.recovery_code = key.to_string();
    st.beeper_state.verified = true;
    st.beeper_state.has_backed_up_code = true;
    st.setup_status.session.state = "ready".to_string();
    st.setup_status.session.e2ee.state = "ready".to_string();
    st.setup_status.session.e2ee.verified = true;
    st.persist_state_to_sqlite();
    Json(json!({ "session": st.setup_status.session }))
}

async fn get_verifications(State(store): State<SharedStore>) -> Json<Value> {
    let st = store.read().unwrap();
    Json(
        json!({ "session": st.setup_status.session, "verification": st.setup_status.session.verification }),
    )
}

async fn post_verifications_start(State(store): State<SharedStore>) -> Json<Value> {
    let mut st = store.write().unwrap();
    let vid = st.next_id("verif");
    let vstate = VerificationState {
        verification_id: vid.clone(),
        direction: "outgoing".to_string(),
        methods: vec!["sas".to_string(), "qr_code".to_string()],
        state: "ready".to_string(),
        updated_at: now_rfc3339_millis(),
        sas: None,
        qr: Some(VerificationQr {
            data: Some(format!("BEEPER_QR_VERIFY:{vid}")),
        }),
        other_device: Some(VerificationOtherDevice {
            device_id: "IPHONE_BEEPER_01".to_string(),
            user_id: st.beeper_state.user_id.clone(),
        }),
    };
    st.verifications.insert(vid, vstate.clone());
    st.setup_status.session.state = "verifying".to_string();
    st.setup_status.session.e2ee.state = "verifying".to_string();
    st.setup_status.session.verification = Some(vstate.clone());
    Json(json!({ "session": st.setup_status.session, "verification": vstate }))
}

async fn get_verification_by_id(
    State(store): State<SharedStore>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let st = store.read().unwrap();
    Json(
        json!({ "session": st.setup_status.session, "verification": st.verifications.get(&id).cloned() }),
    )
}

async fn post_verification_accept(
    State(store): State<SharedStore>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let v = st.verifications.get_mut(&id).expect("verification exists");
    v.state = "ready".to_string();
    v.updated_at = now_rfc3339_millis();
    let v_clone = v.clone();
    st.setup_status.session.verification = Some(v_clone.clone());
    Json(json!({ "session": st.setup_status.session, "verification": v_clone }))
}

async fn post_verification_cancel(
    State(store): State<SharedStore>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let v = st.verifications.get_mut(&id).expect("verification exists");
    v.state = "cancelled".to_string();
    v.updated_at = now_rfc3339_millis();
    let v_clone = v.clone();
    st.setup_status.session.verification = None;
    Json(json!({ "session": st.setup_status.session, "verification": v_clone }))
}

async fn post_verification_sas_start(
    State(store): State<SharedStore>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let v = st.verifications.get_mut(&id).expect("verification exists");
    v.state = "sas_started".to_string();
    v.updated_at = now_rfc3339_millis();
    v.sas = Some(VerificationSas {
        decimals: Some("4821 9034 1172".to_string()),
        emojis: [
            ("🐶", "Dog"),
            ("🚀", "Rocket"),
            ("🔑", "Key"),
            ("⚡", "Zap"),
            ("🍕", "Pizza"),
            ("🎸", "Guitar"),
            ("🌈", "Rainbow"),
        ]
        .into_iter()
        .map(|(e, d)| VerificationSasEmoji {
            emoji: e.to_string(),
            description: d.to_string(),
        })
        .collect(),
    });
    let v_clone = v.clone();
    st.setup_status.session.verification = Some(v_clone.clone());
    Json(json!({ "session": st.setup_status.session, "verification": v_clone }))
}

fn finalize_verification(st: &mut BeeperStore, v_clone: VerificationState) -> Json<Value> {
    st.beeper_state.verified = true;
    st.setup_status.session.state = "ready".to_string();
    st.setup_status.session.e2ee.state = "ready".to_string();
    st.setup_status.session.e2ee.verified = true;
    st.setup_status.session.verification = Some(v_clone.clone());
    st.persist_state_to_sqlite();
    Json(json!({ "session": st.setup_status.session, "verification": v_clone }))
}

async fn post_verification_sas_confirm(
    State(store): State<SharedStore>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let v = st.verifications.get_mut(&id).expect("verification exists");
    v.state = "verified".to_string();
    v.updated_at = now_rfc3339_millis();
    let v_clone = v.clone();
    finalize_verification(&mut st, v_clone)
}

async fn post_verification_qr_scan(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let qr_data = body["data"].as_str().expect("data required");
    assert!(!qr_data.is_empty(), "QR scan data cannot be empty");
    let vid = st.next_id("verif_qr");
    let vstate = VerificationState {
        verification_id: vid.clone(),
        direction: "outgoing".to_string(),
        methods: vec!["qr_code".to_string()],
        state: "verified".to_string(),
        updated_at: now_rfc3339_millis(),
        sas: None,
        qr: Some(VerificationQr {
            data: Some(qr_data.to_string()),
        }),
        other_device: None,
    };
    st.verifications.insert(vid, vstate.clone());
    finalize_verification(&mut st, vstate)
}

async fn post_verification_qr_confirm(
    State(store): State<SharedStore>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let v = st.verifications.get_mut(&id).expect("verification exists");
    v.state = "verified".to_string();
    v.updated_at = now_rfc3339_millis();
    let v_clone = v.clone();
    finalize_verification(&mut st, v_clone)
}

async fn get_accounts(State(store): State<SharedStore>) -> Json<Vec<Account>> {
    Json(store.read().unwrap().accounts.clone())
}

async fn get_account_by_id(
    State(store): State<SharedStore>,
    AxumPath(account_id): AxumPath<String>,
) -> Response {
    let st = store.read().unwrap();
    match st.accounts.iter().find(|a| a.account_id == account_id) {
        Some(acc) => Json(acc.clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "account_not_found" })),
        )
            .into_response(),
    }
}

async fn search_contacts(
    State(store): State<SharedStore>,
    AxumPath(account_id): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let st = store.read().unwrap();
    let q = params
        .get("query")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    let items: Vec<User> = st
        .chats
        .iter()
        .filter(|c| c.account_id == account_id)
        .flat_map(|c| c.participants.items.iter())
        .filter(|p| {
            let hay = format!(
                "{} {} {} {}",
                p.full_name.as_deref().unwrap_or(""),
                p.username.as_deref().unwrap_or(""),
                p.email.as_deref().unwrap_or(""),
                p.phone_number.as_deref().unwrap_or("")
            )
            .to_lowercase();
            q.is_empty() || hay.contains(&q)
        })
        .cloned()
        .collect();
    Json(json!({ "items": items }))
}

async fn list_contacts(
    State(store): State<SharedStore>,
    AxumPath(account_id): AxumPath<String>,
) -> Json<CursorPage<User>> {
    let st = store.read().unwrap();
    let items: Vec<User> = st
        .chats
        .iter()
        .filter(|c| c.account_id == account_id)
        .flat_map(|c| c.participants.items.iter())
        .filter(|p| p.is_self != Some(true))
        .cloned()
        .collect();
    Json(CursorPage {
        items,
        has_more: false,
        oldest_cursor: None,
        newest_cursor: None,
    })
}

async fn get_labels(State(store): State<SharedStore>) -> Json<Vec<Label>> {
    Json(store.read().unwrap().labels.clone())
}

async fn get_bridges(State(store): State<SharedStore>) -> Json<Value> {
    let st = store.read().unwrap();
    let bridges: Vec<BridgeSummary> = NETWORK_SPECS
        .iter()
        .map(|spec| {
            let logins: Vec<BridgeAccountInfo> = st
                .bridge_accounts
                .iter()
                .filter(|b| b.bridge_id == spec.bridge_id)
                .cloned()
                .collect();
            BridgeSummary {
                bridge_id: spec.bridge_id.to_string(),
                bridge_type: spec.bridge_type.to_string(),
                is_running: !logins.is_empty(),
                logins,
            }
        })
        .collect();
    Json(json!({
        "bridges": bridges,
        "catalog": NETWORK_SPECS.iter().map(|s| json!({
            "bridgeID": s.bridge_id,
            "bridgeType": s.bridge_type,
            "displayName": s.display_name,
            "supportsOnDevice": s.supports_on_device,
            "supportsCloud": s.supports_cloud
        })).collect::<Vec<_>>()
    }))
}

async fn get_bridge_by_id(
    State(store): State<SharedStore>,
    AxumPath(bridge_id): AxumPath<String>,
) -> Json<BridgeSummary> {
    let st = store.read().unwrap();
    let spec = network_spec_by_bridge(&bridge_id);
    let logins: Vec<BridgeAccountInfo> = st
        .bridge_accounts
        .iter()
        .filter(|b| b.bridge_id == bridge_id)
        .cloned()
        .collect();
    Json(BridgeSummary {
        bridge_id: spec.bridge_id.to_string(),
        bridge_type: spec.bridge_type.to_string(),
        is_running: !logins.is_empty(),
        logins,
    })
}

async fn get_bridge_capabilities(AxumPath(bridge_id): AxumPath<String>) -> Json<Value> {
    let spec = network_spec_by_bridge(&bridge_id);
    Json(json!({
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
    }))
}

async fn get_bridge_login_flows(AxumPath(bridge_id): AxumPath<String>) -> Json<Value> {
    let spec = network_spec_by_bridge(&bridge_id);
    let mut flows = vec![
        BridgeLoginFlow {
            id: "qr_login".to_string(),
            name: format!("Scan {} QR Code", spec.display_name),
            description: Some("Link your account by scanning a QR code".to_string()),
        },
        BridgeLoginFlow {
            id: "user_input_login".to_string(),
            name: format!("{} Credentials / Phone", spec.display_name),
            description: Some("Sign in with phone number, username, or token".to_string()),
        },
        BridgeLoginFlow {
            id: "cookie_import".to_string(),
            name: format!("Import {} Browser Session", spec.display_name),
            description: Some("One-click browser cookie & session import".to_string()),
        },
    ];
    if spec.supports_on_device {
        flows.push(BridgeLoginFlow {
            id: "on_device_local".to_string(),
            name: format!("On-Device {} Bridge", spec.display_name),
            description: Some("Direct local Mautrix bridge connection".to_string()),
        });
    }
    Json(json!({ "bridgeID": spec.bridge_id, "bridgeType": spec.bridge_type, "flows": flows }))
}

async fn get_bridge_logins(
    State(store): State<SharedStore>,
    AxumPath(bridge_id): AxumPath<String>,
) -> Json<Value> {
    let st = store.read().unwrap();
    let spec = network_spec_by_bridge(&bridge_id);
    let logins: Vec<BridgeAccountInfo> = st
        .bridge_accounts
        .iter()
        .filter(|b| b.bridge_id == bridge_id)
        .cloned()
        .collect();
    Json(json!({ "bridgeID": spec.bridge_id, "bridgeType": spec.bridge_type, "logins": logins }))
}

async fn delete_bridge_login(
    State(store): State<SharedStore>,
    AxumPath((bridge_id, login_id)): AxumPath<(String, String)>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    let spec = network_spec_by_bridge(&bridge_id);
    st.bridge_accounts.retain(|b| {
        !(b.bridge_id == bridge_id && b.login_id.as_deref() == Some(login_id.as_str()))
    });
    st.persist_state_to_sqlite();
    let remaining: Vec<BridgeAccountInfo> = st
        .bridge_accounts
        .iter()
        .filter(|b| b.bridge_id == bridge_id)
        .cloned()
        .collect();
    Json(json!({
        "bridge": {
            "bridgeID": spec.bridge_id,
            "bridgeType": spec.bridge_type,
            "isRunning": !remaining.is_empty(),
            "logins": remaining
        }
    }))
}

async fn post_bridge_login_session(
    State(store): State<SharedStore>,
    AxumPath(bridge_id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Json<BridgeLoginSession> {
    let mut st = store.write().unwrap();
    let spec = network_spec_by_bridge(&bridge_id);
    let flow_id = body
        .get("flowID")
        .and_then(Value::as_str)
        .unwrap_or("user_input_login")
        .to_string();
    let session_id = st.next_id("lsess");

    let step = match flow_id.as_str() {
        "qr_login" => BridgeLoginStep {
            step_id: "step_qr_wait".to_string(),
            step_type: "display_and_wait".to_string(),
            instructions: Some(format!(
                "Scan this QR code in your {} mobile app",
                spec.display_name
            )),
            user_input: None,
            display_and_wait: Some(BridgeLoginStepDisplay {
                data: Some(format!("BEEPER_BRIDGE_QR:{}:{session_id}", spec.bridge_id)),
                message: Some("Waiting for mobile scan...".to_string()),
            }),
            cookies: None,
            complete: None,
        },
        "cookie_import" => BridgeLoginStep {
            step_id: "step_cookies".to_string(),
            step_type: "cookies".to_string(),
            instructions: Some(format!("Import session cookies for {}", spec.display_name)),
            user_input: None,
            display_and_wait: None,
            cookies: Some(BridgeLoginStepCookies {
                names: vec!["session_token".to_string(), "auth_id".to_string()],
                url: format!("https://{}.com", spec.bridge_id),
            }),
            complete: None,
        },
        _ => BridgeLoginStep {
            step_id: "step_credentials".to_string(),
            step_type: "user_input".to_string(),
            instructions: Some(format!("Enter {} login credentials", spec.display_name)),
            user_input: Some(BridgeLoginStepUserInput {
                fields: vec![
                    BridgeLoginField {
                        id: "username".to_string(),
                        name: "Phone or Username".to_string(),
                        field_type: "string".to_string(),
                    },
                    BridgeLoginField {
                        id: "code".to_string(),
                        name: "Verification Code / Password".to_string(),
                        field_type: "password".to_string(),
                    },
                ],
            }),
            display_and_wait: None,
            cookies: None,
            complete: None,
        },
    };

    let session = BridgeLoginSession {
        login_session_id: session_id.clone(),
        bridge_id: spec.bridge_id.to_string(),
        flow_id,
        step,
    };
    st.bridge_sessions.insert(session_id, session.clone());
    Json(session)
}

async fn get_bridge_login_session(
    State(store): State<SharedStore>,
    AxumPath((_bridge_id, session_id)): AxumPath<(String, String)>,
) -> Json<BridgeLoginSession> {
    Json(
        store
            .read()
            .unwrap()
            .bridge_sessions
            .get(&session_id)
            .expect("login session exists")
            .clone(),
    )
}

async fn delete_bridge_login_session(
    State(store): State<SharedStore>,
    AxumPath((_bridge_id, session_id)): AxumPath<(String, String)>,
) -> Json<Value> {
    store.write().unwrap().bridge_sessions.remove(&session_id);
    Json(json!({ "cancelled": true }))
}

async fn post_bridge_login_step(
    State(store): State<SharedStore>,
    AxumPath((bridge_id, session_id, _step_id)): AxumPath<(String, String, String)>,
    Json(body): Json<Value>,
) -> Json<BridgeLoginSession> {
    let mut st = store.write().unwrap();
    let spec = network_spec_by_bridge(&bridge_id);
    let login_id = st.next_id(&format!("login_{bridge_id}"));
    let mode = if body.get("mode").and_then(Value::as_str) == Some("cloud") {
        "cloud"
    } else {
        "on_device"
    };

    st.bridge_accounts.retain(|b| b.bridge_id != bridge_id);
    st.bridge_accounts.push(BridgeAccountInfo {
        account_id: format!("acc_{bridge_id}"),
        bridge_id: spec.bridge_id.to_string(),
        bridge_type: spec.bridge_type.to_string(),
        is_running: true,
        login_id: Some(login_id.clone()),
        mode: Some(mode.to_string()),
        network_name: spec.display_name.to_string(),
        state: "CONNECTED".to_string(),
    });
    st.rebuild_accounts_from_bridges();
    st.persist_state_to_sqlite();

    let completed = BridgeLoginSession {
        login_session_id: session_id.clone(),
        bridge_id: spec.bridge_id.to_string(),
        flow_id: "completed".to_string(),
        step: BridgeLoginStep {
            step_id: "complete".to_string(),
            step_type: "complete".to_string(),
            instructions: None,
            user_input: None,
            display_and_wait: None,
            cookies: None,
            complete: Some(BridgeLoginStepComplete {
                user_login_id: login_id,
            }),
        },
    };
    st.bridge_sessions.insert(session_id, completed.clone());
    Json(completed)
}

async fn list_chats(
    State(store): State<SharedStore>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<CursorPage<Chat>> {
    let st = store.read().unwrap();
    let account_filter = params.get("accountIDs");
    let mut items: Vec<Chat> = st
        .chats
        .iter()
        .filter(|c| {
            account_filter
                .map(|acc| c.account_id == *acc)
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    items.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    let oldest = items.last().map(|c| c.id.clone());
    let newest = items.first().map(|c| c.id.clone());
    Json(CursorPage {
        items,
        has_more: false,
        oldest_cursor: oldest,
        newest_cursor: newest,
    })
}

async fn create_chat(State(store): State<SharedStore>, Json(body): Json<Value>) -> Json<Value> {
    let mut st = store.write().unwrap();
    let mode = body.get("mode").and_then(Value::as_str).unwrap_or("create");
    if mode == "merge" {
        let merge_obj = body.get("merge").expect("merge object required");
        let chat_ids: Vec<String> = merge_obj["chatIDs"]
            .as_array()
            .expect("merge.chatIDs required")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let default_chat_id = merge_obj
            .get("defaultChatID")
            .and_then(Value::as_str)
            .map(str::to_string);
        let title = body
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (chat, status) = st.create_or_start_chat(
            "acc_merged",
            "merged",
            title,
            Vec::new(),
            None,
            Some(chat_ids),
            default_chat_id,
        );
        return Json(json!({ "chatID": chat.id, "status": status }));
    }

    let account_id = body
        .get("accountID")
        .and_then(Value::as_str)
        .unwrap_or("acc_whatsapp");
    let chat_type = body.get("type").and_then(Value::as_str).unwrap_or("single");
    let title = body
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string);
    let participant_ids: Vec<String> = body
        .get("participantIDs")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let message_text = body
        .get("messageText")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (chat, status) = st.create_or_start_chat(
        account_id,
        chat_type,
        title,
        participant_ids,
        message_text,
        None,
        None,
    );
    Json(json!({ "chatID": chat.id, "status": status }))
}

async fn start_chat(State(store): State<SharedStore>, Json(body): Json<Value>) -> Json<Value> {
    let mut st = store.write().unwrap();
    let account_id = body["accountID"].as_str().expect("accountID required");
    let user_obj = body.get("user").expect("user object required");
    let uid = user_obj
        .get("id")
        .or_else(|| user_obj.get("phoneNumber"))
        .or_else(|| user_obj.get("username"))
        .or_else(|| user_obj.get("email"))
        .and_then(Value::as_str)
        .expect("user identifier required")
        .to_string();
    let name = user_obj
        .get("fullName")
        .and_then(Value::as_str)
        .map(str::to_string);
    let msg_text = body
        .get("messageText")
        .and_then(Value::as_str)
        .map(str::to_string);

    if let Some(existing) = st.chats.iter().find(|c| {
        c.account_id == account_id
            && c.chat_type == "single"
            && c.participants.items.iter().any(|p| p.id == uid)
    }) {
        return Json(json!({ "chatID": existing.id, "status": "existing" }));
    }

    let (chat, status) =
        st.create_or_start_chat(account_id, "single", name, vec![uid], msg_text, None, None);
    Json(json!({ "chatID": chat.id, "status": status }))
}

async fn search_chats(
    State(store): State<SharedStore>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<CursorPage<Chat>> {
    let st = store.read().unwrap();
    let query = params.get("query").map(String::as_str).unwrap_or("");
    let inbox = params.get("inbox").map(String::as_str);
    let unread_only = params
        .get("unreadOnly")
        .map(|v| v == "true")
        .unwrap_or(false);
    let chat_type = params.get("type").map(String::as_str);
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);

    let candidate_indices: Vec<usize> = if query.trim().is_empty() {
        (0..st.chats.len()).collect()
    } else {
        st.chat_search_index
            .query_all_tokens(query)
            .into_iter()
            .map(|i| i as usize)
            .collect()
    };

    let mut items: Vec<Chat> = candidate_indices
        .into_iter()
        .filter_map(|i| st.chats.get(i).cloned())
        .filter(|c| {
            if unread_only && (c.flags & CHAT_UNREAD) == 0 {
                return false;
            }
            match inbox {
                Some("archive") if (c.flags & CHAT_ARCHIVED) == 0 => return false,
                Some("low-priority") if (c.flags & CHAT_LOW_PRIORITY) == 0 => return false,
                Some("primary") if (c.flags & (CHAT_ARCHIVED | CHAT_LOW_PRIORITY)) != 0 => {
                    return false
                }
                _ => {}
            }
            if let Some(t) = chat_type {
                if t != "any" && c.chat_type != t {
                    return false;
                }
            }
            true
        })
        .take(limit)
        .collect();

    items.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    let oldest = items.last().map(|c| c.id.clone());
    let newest = items.first().map(|c| c.id.clone());
    Json(CursorPage {
        items,
        has_more: false,
        oldest_cursor: oldest,
        newest_cursor: newest,
    })
}

async fn retrieve_chat(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
) -> Response {
    let st = store.read().unwrap();
    match st.resolve_chat_idx(&chat_id) {
        Some(idx) => Json(st.chats[idx].clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "chat_not_found" })),
        )
            .into_response(),
    }
}

async fn patch_chat_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut st = store.write().unwrap();
    let clear_draft = body.get("draft").map(Value::is_null).unwrap_or(false);
    let new_draft: Option<ChatDraft> = body
        .get("draft")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).expect("valid ChatDraft"));
    let new_merge: Option<Option<ChatMergeInfo>> = body.get("merge").map(|m_val| {
        if m_val.is_null() {
            None
        } else {
            Some(serde_json::from_value(m_val.clone()).expect("valid ChatMergeInfo"))
        }
    });

    match st.patch_chat(&chat_id, clear_draft, new_draft, new_merge) {
        Ok(chat) => Json(chat).into_response(),
        Err("existing_draft_not_empty") => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "existing_draft_not_empty" })),
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
    }
}

async fn archive_chat_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
    body: Option<Json<Value>>,
) -> Response {
    let mut st = store.write().unwrap();
    let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
    let archived = body
        .and_then(|b| b.get("archived").and_then(Value::as_bool))
        .unwrap_or(true);
    st.chats[idx].is_archived = Some(archived);
    st.chats[idx].sync_flags();
    let c = st.chats[idx].clone();
    st.emit_ws(WsEvent {
        event_type: "chat.upserted".to_string(),
        chat_id: Some(c.id.clone()),
        message_id: None,
        timestamp: now_rfc3339_millis(),
        chat: Some(c),
        message: None,
    });
    StatusCode::NO_CONTENT.into_response()
}

async fn read_chat_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
) -> Response {
    let mut st = store.write().unwrap();
    let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
    let canonical_id = st.chats[idx].id.clone();
    st.chats[idx].unread_count = 0;
    st.chats[idx].is_marked_unread = Some(false);
    st.chats[idx].sync_flags();
    if let Some(m_indices) = st.chat_msg_indices.get(&canonical_id).cloned() {
        for m_idx in m_indices {
            st.messages[m_idx].is_unread = Some(false);
            st.messages[m_idx].sync_flags();
        }
    }
    Json(json!({ "chatID": canonical_id, "success": true })).into_response()
}

async fn unread_chat_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
) -> Response {
    let mut st = store.write().unwrap();
    let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
    let canonical_id = st.chats[idx].id.clone();
    st.chats[idx].is_marked_unread = Some(true);
    st.chats[idx].unread_count = st.chats[idx].unread_count.max(1);
    st.chats[idx].sync_flags();
    Json(json!({ "chatID": canonical_id, "success": true })).into_response()
}

async fn notify_anyway_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
) -> Response {
    let mut st = store.write().unwrap();
    let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
    let canonical_id = st.chats[idx].id.clone();
    st.chats[idx].flags |= CHAT_NOTIFY_ANYWAY;
    Json(json!({ "chatID": canonical_id, "success": true })).into_response()
}

async fn create_reminder_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut st = store.write().unwrap();
    let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
    let reminder: ChatReminder =
        serde_json::from_value(body["reminder"].clone()).expect("valid reminder");
    st.chats[idx].reminder = Some(reminder);
    StatusCode::NO_CONTENT.into_response()
}

async fn delete_reminder_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
) -> Response {
    let mut st = store.write().unwrap();
    let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
    st.chats[idx].reminder = None;
    StatusCode::NO_CONTENT.into_response()
}

async fn list_messages(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let st = store.read().unwrap();
    let c_idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
    let canonical_id = &st.chats[c_idx].id;
    let cursor = params.get("cursor").cloned();
    let direction = params
        .get("direction")
        .map(String::as_str)
        .unwrap_or("before");

    let mut target_chat_ids = vec![canonical_id.clone()];
    if let Some(ref merge) = st.chats[c_idx].merge {
        for mid in &merge.chat_ids {
            if let Some(mc_id) = st.resolve_canonical_chat_id(mid) {
                target_chat_ids.push(mc_id);
            }
        }
    }

    let mut items: Vec<Message> = target_chat_ids
        .iter()
        .filter_map(|cid| st.chat_msg_indices.get(cid))
        .flatten()
        .map(|&m_idx| &st.messages[m_idx])
        .filter(|m| (m.flags & MSG_DELETED) == 0)
        .cloned()
        .collect();
    items.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));

    if let Some(ref cur) = cursor {
        items.retain(|m| {
            if direction == "after" {
                m.sort_key > *cur
            } else {
                m.sort_key < *cur
            }
        });
    }

    let oldest = items.first().map(|m| m.sort_key.clone());
    let newest = items.last().map(|m| m.sort_key.clone());
    Json(CursorPage {
        items,
        has_more: false,
        oldest_cursor: oldest,
        newest_cursor: newest,
    })
    .into_response()
}

async fn send_message_handler(
    State(store): State<SharedStore>,
    AxumPath(chat_id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut st = store.write().unwrap();
    let text = body.get("text").and_then(Value::as_str).map(str::to_string);
    let reply_to = body
        .get("replyToMessageID")
        .and_then(Value::as_str)
        .map(str::to_string);

    let attachment = body.get("attachment").map(|att_obj| {
        let upload_id = att_obj["uploadID"]
            .as_str()
            .expect("attachment.uploadID required");
        let stored = st
            .assets
            .get(upload_id)
            .cloned()
            .expect("uploaded asset exists");
        let mime = att_obj
            .get("mimeType")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or(stored.mime_type.clone());
        let att_type = if mime.starts_with("image/") {
            "img"
        } else if mime.starts_with("video/") {
            "video"
        } else if mime.starts_with("audio/") {
            "audio"
        } else {
            "unknown"
        };
        Attachment {
            attachment_type: att_type.to_string(),
            id: Some(upload_id.to_string()),
            duration: att_obj.get("duration").and_then(Value::as_f64),
            file_name: att_obj
                .get("fileName")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(Some(stored.file_name)),
            file_size: Some(stored.file_size),
            is_gif: att_obj.get("isGif").and_then(Value::as_bool),
            is_sticker: att_obj.get("isSticker").and_then(Value::as_bool),
            is_voice_note: att_obj.get("isVoiceNote").and_then(Value::as_bool),
            mime_type: Some(mime),
            poster_img: None,
            size: None,
            src_url: Some(stored.mxc_url),
            transcript: None,
        }
    });

    let (resolved_chat_id, pending_id, _) = st
        .send_message(&chat_id, text, reply_to, attachment)
        .expect("send message");
    (
        StatusCode::CREATED,
        Json(json!({ "chatID": resolved_chat_id, "pendingMessageID": pending_id })),
    )
        .into_response()
}

async fn get_message_handler(
    State(store): State<SharedStore>,
    AxumPath((_chat_id, message_id)): AxumPath<(String, String)>,
) -> Response {
    let st = store.read().unwrap();
    let &m_idx = st.msg_idx_by_id.get(&message_id).expect("message exists");
    Json(st.messages[m_idx].clone()).into_response()
}

async fn edit_message_handler(
    State(store): State<SharedStore>,
    AxumPath((chat_id, message_id)): AxumPath<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let mut st = store.write().unwrap();
    let new_text = body["text"].as_str().expect("text required").to_string();
    let (cid, updated) = st
        .edit_message(&chat_id, &message_id, new_text)
        .expect("edit message");
    Json(json!({ "chatID": cid, "messageID": updated.id, "success": true })).into_response()
}

async fn delete_message_handler(
    State(store): State<SharedStore>,
    AxumPath((chat_id, message_id)): AxumPath<(String, String)>,
) -> Response {
    let mut st = store.write().unwrap();
    let cid = st
        .delete_message(&chat_id, &message_id)
        .expect("delete message");
    Json(json!({ "chatID": cid, "messageID": message_id, "success": true })).into_response()
}

async fn add_reaction_handler(
    State(store): State<SharedStore>,
    AxumPath((chat_id, message_id)): AxumPath<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let mut st = store.write().unwrap();
    let reaction_key = body["reactionKey"].as_str().expect("reactionKey required");
    let tx_id = body
        .get("transactionID")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (cid, tx) = st
        .add_reaction(&chat_id, &message_id, reaction_key, tx_id)
        .expect("add reaction");
    Json(json!({
        "chatID": cid,
        "messageID": message_id,
        "reactionKey": reaction_key,
        "transactionID": tx,
        "success": true
    }))
    .into_response()
}

async fn delete_reaction_handler(
    State(store): State<SharedStore>,
    AxumPath((chat_id, message_id, reaction_key)): AxumPath<(String, String, String)>,
) -> Response {
    let mut st = store.write().unwrap();
    let decoded_key = url_decode(&reaction_key);
    let cid = st
        .remove_reaction(&chat_id, &message_id, &decoded_key)
        .expect("remove reaction");
    Json(json!({
        "chatID": cid,
        "messageID": message_id,
        "reactionKey": decoded_key,
        "success": true
    }))
    .into_response()
}

async fn search_messages_handler(
    State(store): State<SharedStore>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<CursorPage<Message>> {
    let st = store.read().unwrap();
    let query = params.get("query").map(String::as_str).unwrap_or("");
    let sender_filter = params.get("sender").map(String::as_str);
    let chat_id_filter = params.get("chatIDs").map(String::as_str);
    let media_filter = params.get("mediaTypes").map(String::as_str);
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(20);

    let candidate_indices: Vec<usize> = if query.trim().is_empty() {
        (0..st.messages.len()).collect()
    } else {
        st.msg_search_index
            .query_all_tokens(query)
            .into_iter()
            .map(|i| i as usize)
            .collect()
    };

    let mut items: Vec<Message> = candidate_indices
        .into_iter()
        .filter_map(|i| st.messages.get(i).cloned())
        .filter(|m| {
            if (m.flags & MSG_DELETED) != 0 {
                return false;
            }
            if let Some(cid) = chat_id_filter {
                if m.chat_id != cid {
                    return false;
                }
            }
            match sender_filter {
                Some("me") if m.is_sender != Some(true) => return false,
                Some("others") if m.is_sender == Some(true) => return false,
                Some(specific)
                    if specific != "me" && specific != "others" && m.sender_id != specific =>
                {
                    return false
                }
                _ => {}
            }
            match media_filter {
                Some("any") if m.attachments.is_empty() => return false,
                Some("image") if !m.attachments.iter().any(|a| a.attachment_type == "img") => {
                    return false
                }
                Some("video") if !m.attachments.iter().any(|a| a.attachment_type == "video") => {
                    return false
                }
                Some("audio") if !m.attachments.iter().any(|a| a.attachment_type == "audio") => {
                    return false
                }
                Some("link")
                    if !m
                        .text
                        .as_deref()
                        .map(|t| t.contains("http://") || t.contains("https://"))
                        .unwrap_or(false) =>
                {
                    return false
                }
                Some("file") if !m.attachments.iter().any(|a| a.attachment_type == "unknown") => {
                    return false
                }
                _ => {}
            }
            true
        })
        .take(limit)
        .collect();

    items.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
    let oldest = items.last().map(|m| m.sort_key.clone());
    let newest = items.first().map(|m| m.sort_key.clone());
    Json(CursorPage {
        items,
        has_more: false,
        oldest_cursor: oldest,
        newest_cursor: newest,
    })
}

fn asset_json_response(asset: StoredAsset) -> Response {
    Json(json!({
        "uploadID": asset.upload_id,
        "fileName": asset.file_name,
        "mimeType": asset.mime_type,
        "fileSize": asset.file_size,
        "srcURL": asset.mxc_url,
        "width": asset.width,
        "height": asset.height
    }))
    .into_response()
}

async fn upload_asset_multipart(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    let mut file_name = "upload.bin".to_string();
    let mut mime_type = "application/octet-stream".to_string();
    let mut file_bytes: Vec<u8> = body.to_vec();

    if let Some((_, boundary)) = ct.split_once("boundary=") {
        let delim = format!("--{}", boundary.trim().trim_matches('"'));
        if let Ok(raw_str) = std::str::from_utf8(&body) {
            for part in raw_str.split(&delim) {
                if let Some((part_hdrs, part_body)) = part.split_once("\r\n\r\n") {
                    let clean_body = part_body
                        .trim_end_matches("--\r\n")
                        .trim_end_matches("\r\n");
                    if part_hdrs.contains("name=\"file\"") {
                        if let Some((_, fn_part)) = part_hdrs.split_once("filename=\"") {
                            if let Some((fname, _)) = fn_part.split_once('"') {
                                file_name = fname.to_string();
                            }
                        }
                        if let Some((_, ct_part)) = part_hdrs.split_once("Content-Type:") {
                            if let Some(ct_line) = ct_part.lines().next() {
                                mime_type = ct_line.trim().to_string();
                            }
                        }
                        file_bytes = clean_body.as_bytes().to_vec();
                    } else if part_hdrs.contains("name=\"fileName\"") {
                        file_name = clean_body.to_string();
                    } else if part_hdrs.contains("name=\"mimeType\"") {
                        mime_type = clean_body.to_string();
                    }
                }
            }
        }
    }
    let asset = store
        .write()
        .unwrap()
        .store_uploaded_asset(file_name, mime_type, &file_bytes);
    asset_json_response(asset)
}

async fn upload_asset_base64(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Response {
    let content_b64 = body["content"].as_str().expect("content base64 required");
    let file_name = body
        .get("fileName")
        .and_then(Value::as_str)
        .unwrap_or("upload.bin")
        .to_string();
    let mime_type = body
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = base64_decode_standard(content_b64);
    let asset = store
        .write()
        .unwrap()
        .store_uploaded_asset(file_name, mime_type, &bytes);
    asset_json_response(asset)
}

async fn download_asset_handler(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Response {
    let url = body["url"].as_str().expect("url required");
    let (path, _) = store
        .read()
        .unwrap()
        .resolve_asset_path(url)
        .expect("asset exists");
    Json(json!({ "srcURL": crate::platform::path_to_file_url(&path) })).into_response()
}

async fn serve_asset_handler(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let url = params.get("url").expect("url query parameter required");
    let (path, mime) = store
        .read()
        .unwrap()
        .resolve_asset_path(url)
        .expect("asset exists");
    let bytes = std::fs::read(&path).expect("read asset file");
    let total = bytes.len();

    if let Some(range_hdr) = headers.get("range").and_then(|v| v.to_str().ok()) {
        let spec = range_hdr.strip_prefix("bytes=").expect("bytes= prefix");
        let (s_str, e_str) = spec.split_once('-').expect("range dash");
        let start: usize = s_str.parse().unwrap_or(0);
        let end: usize = if e_str.is_empty() {
            total.saturating_sub(1)
        } else {
            e_str.parse::<usize>().unwrap().min(total.saturating_sub(1))
        };
        assert!(start <= end && end < total, "invalid Range bounds");
        let content_range = format!("bytes {start}-{end}/{total}");
        return (
            StatusCode::PARTIAL_CONTENT,
            [
                ("content-type", mime.as_str()),
                ("accept-ranges", "bytes"),
                ("content-range", content_range.as_str()),
            ],
            bytes[start..=end].to_vec(),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        [("content-type", mime.as_str()), ("accept-ranges", "bytes")],
        bytes,
    )
        .into_response()
}

async fn unified_search_handler(
    State(store): State<SharedStore>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let st = store.read().unwrap();
    let query = params.get("query").map(String::as_str).unwrap_or("");
    assert!(!query.trim().is_empty(), "search query must not be empty");

    let mut chats = Vec::new();
    let mut in_groups = Vec::new();
    for idx in st.chat_search_index.query_all_tokens(query) {
        let c = &st.chats[idx as usize];
        if c.chat_type == "group" {
            in_groups.push(c.clone());
        } else {
            chats.push(c.clone());
        }
    }

    let mut msg_items = Vec::new();
    let mut referenced_chats: HashMap<String, Chat> = HashMap::new();
    for idx in st.msg_search_index.query_all_tokens(query) {
        let m = &st.messages[idx as usize];
        if (m.flags & MSG_DELETED) == 0 {
            msg_items.push(m.clone());
            if let Some(&c_idx) = st.chat_idx_by_id.get(&m.chat_id) {
                referenced_chats.insert(m.chat_id.clone(), st.chats[c_idx].clone());
            }
        }
    }

    Json(json!({
        "results": {
            "chats": chats,
            "in_groups": in_groups,
            "messages": {
                "chats": referenced_chats,
                "hasMore": false,
                "items": msg_items,
                "newestCursor": msg_items.first().map(|m| m.sort_key.clone()),
                "oldestCursor": msg_items.last().map(|m| m.sort_key.clone())
            }
        }
    }))
}

async fn focus_app_handler(
    State(store): State<SharedStore>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut st = store.write().unwrap();
    if let Some(cid) = body
        .get("chatID")
        .and_then(Value::as_str)
        .and_then(|id| st.resolve_canonical_chat_id(id))
    {
        st.focused_chat_id = Some(cid.clone());
        let draft_text = body
            .get("draftText")
            .and_then(Value::as_str)
            .map(str::to_string);
        let draft_attachment = body
            .get("draftAttachmentPath")
            .and_then(Value::as_str)
            .map(str::to_string);
        if draft_text.is_some() || draft_attachment.is_some() {
            let idx = st.resolve_chat_idx(&cid).unwrap();
            let attachments = draft_attachment
                .into_iter()
                .map(|p| ChatDraftAttachment {
                    upload_id: p.clone(),
                    file_name: Some(p),
                    mime_type: None,
                })
                .collect();
            st.chats[idx].draft = Some(ChatDraft {
                text: draft_text,
                attachments,
                reply_to_message_id: None,
            });
        }
    }
    if let Some(mid) = body
        .get("messageID")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        st.focused_message_id = Some(mid);
    }
    Json(json!({ "success": true }))
}

pub async fn read_ws_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Option<(u8, Vec<u8>)> {
    let mut hdr = [0u8; 2];
    reader.read_exact(&mut hdr).await.ok()?;
    let opcode = hdr[0] & 0x0F;
    let masked = (hdr[1] & 0x80) != 0;
    let mut len = (hdr[1] & 0x7F) as usize;
    if len == 126 {
        let mut ext = [0u8; 2];
        reader.read_exact(&mut ext).await.ok()?;
        len = u16::from_be_bytes(ext) as usize;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        reader.read_exact(&mut ext).await.ok()?;
        len = u64::from_be_bytes(ext) as usize;
    }
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask).await.ok()?;
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await.ok()?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }
    Some((opcode, payload))
}

pub async fn write_ws_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    payload: &[u8],
    masked: bool,
) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(14 + payload.len());
    frame.push(0x81);
    let mask_bit = if masked { 0x80 } else { 0x00 };
    if payload.len() < 126 {
        frame.push(mask_bit | (payload.len() as u8));
    } else if payload.len() <= 65535 {
        frame.push(mask_bit | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(mask_bit | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    if masked {
        let mask_key = [0x37u8, 0xFA, 0x21, 0x3D];
        frame.extend_from_slice(&mask_key);
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask_key[i & 3]);
        }
    } else {
        frame.extend_from_slice(payload);
    }
    writer.write_all(&frame).await
}

async fn handle_ws_client(
    mut reader: OwnedReadHalf,
    mut writer: OwnedWriteHalf,
    mut rx: tokio::sync::broadcast::Receiver<WsEvent>,
) {
    let subscribed_chats: Arc<RwLock<HashSet<String>>> =
        Arc::new(RwLock::new(HashSet::from(["*".to_string()])));
    let sub_clone = subscribed_chats.clone();

    let recv_task = tokio::spawn(async move {
        while let Some((opcode, payload)) = read_ws_frame(&mut reader).await {
            if opcode == 0x08 {
                break;
            }
            if opcode == 0x01 {
                if let Ok(txt) = std::str::from_utf8(&payload) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(txt) {
                        if parsed.get("type").and_then(Value::as_str) == Some("subscriptions.set") {
                            if let Some(arr) = parsed.get("chatIDs").and_then(Value::as_array) {
                                let mut set = sub_clone.write().unwrap();
                                set.clear();
                                set.extend(
                                    arr.iter().filter_map(|v| v.as_str().map(str::to_string)),
                                );
                            }
                        }
                    }
                }
            }
        }
    });

    while let Ok(event) = rx.recv().await {
        let should_send = {
            let subs = subscribed_chats.read().unwrap();
            subs.contains("*")
                || event
                    .chat_id
                    .as_ref()
                    .map(|cid| subs.contains(cid))
                    .unwrap_or(false)
        };
        if should_send {
            let payload = serde_json::to_string(&event).unwrap();
            if write_ws_frame(&mut writer, payload.as_bytes(), false)
                .await
                .is_err()
            {
                break;
            }
        }
    }
    recv_task.abort();
}
