use crate::codec::{base64_encode_standard, base64_encode_url_nopad, sha256, url_encode};
use crate::http::{query_string, read_message, read_ws_frame, write_ws_frame, StatusCode};
use beeper_core::models::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::fmt::Write;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

#[derive(Clone)]
pub struct BeeperClient {
    pub host_port: String,
    pub access_token: String,
    /// Prints each request and its status on stderr, like the `debug` log level of the SDK.
    pub debug: bool,
}

/// A failed request, with the text of the Desktop API SDK: `<status> <message>`, or `Connection error.`.
#[derive(Debug)]
pub struct ApiError(pub String);

const REDIRECT_URI: &str = "http://127.0.0.1:0/callback";

fn chat_path(chat_id: &str, rest: &str) -> String {
    format!("/v1/chats/{}{rest}", url_encode(chat_id))
}

fn message_path(chat_id: &str, message_id: &str, rest: &str) -> String {
    chat_path(chat_id, &format!("/messages/{}{rest}", url_encode(message_id)))
}

impl BeeperClient {
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self::try_new(&base_url.into(), access_token).expect("base_url starts with http://")
    }

    /// A client of a base URL from the user. The client speaks plain HTTP only.
    pub fn try_new(base_url: &str, access_token: impl Into<String>) -> Result<Self, String> {
        let host =
            base_url.strip_prefix("http://").ok_or(format!("Only http:// base URLs are supported: {base_url}"))?;
        let host_port = host.split('/').next().expect("split gives one item").to_string();
        let host_port = if host_port.contains(':') { host_port } else { format!("{host_port}:80") };
        Ok(Self { host_port, access_token: access_token.into(), debug: false })
    }

    /// A request for the CLI: the body of a 2xx reply as JSON (`Null` when it is empty, a string when it is not
    /// JSON), else the SDK error text.
    pub async fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        let bytes = self.call_raw(method, path, body).await?;
        Ok(match bytes.is_empty() {
            true => Value::Null,
            false => serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        })
    }

    /// The status and the body of a request, or `Connection error.` when the server does not answer.
    pub async fn fetch(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Vec<u8>), ApiError> {
        let accept = [("accept", "application/json")];
        let (status, bytes) =
            self.try_send(method, path, &accept, body).await.map_err(|_| ApiError("Connection error.".into()))?;
        if self.debug {
            eprintln!("[beeper-api] {method} {path} -> {}", status.0);
        }
        Ok((status.0, bytes))
    }

    /// Like [`Self::call`], with the raw body of the reply.
    pub async fn call_raw(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Vec<u8>, ApiError> {
        let (status, bytes) = self.fetch(method, path, body).await?;
        if status < 400 {
            return Ok(bytes);
        }
        let error: Option<Value> = serde_json::from_slice(&bytes).ok();
        let msg = match &error {
            Some(Value::Object(e)) if e.get("message").is_some_and(|m| !m.is_null()) => match &e["message"] {
                Value::String(m) => m.clone(),
                m => m.to_string(),
            },
            Some(e) => e.to_string(),
            None if !bytes.is_empty() => String::from_utf8_lossy(&bytes).into_owned(),
            None => "status code (no body)".into(),
        };
        Err(ApiError(format!("{status} {msg}")))
    }

    pub async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> (StatusCode, Vec<u8>) {
        self.try_send(method, path, headers, body).await.expect("send the HTTP request to the Beeper server")
    }

    async fn try_send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> std::io::Result<(StatusCode, Vec<u8>)> {
        let body = body.map(|v| serde_json::to_vec(v).unwrap());
        let len = body.as_ref().map_or(0, Vec::len);
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nhost: {}\r\nconnection: close\r\ncontent-length: {len}\r\n",
            self.host_port
        );
        if !self.access_token.is_empty() {
            write!(head, "authorization: Bearer {}\r\n", self.access_token).unwrap();
        }
        if body.is_some() {
            head.push_str("content-type: application/json\r\n");
        }
        headers.iter().for_each(|(k, v)| write!(head, "{k}: {v}\r\n").unwrap());
        head.push_str("\r\n");
        let mut stream = TcpStream::connect(&self.host_port).await?;
        stream.write_all(&[head.into_bytes(), body.unwrap_or_default()].concat()).await?;
        let bad = |_| std::io::Error::new(std::io::ErrorKind::InvalidData, "a bad HTTP response");
        let resp = read_message(&mut stream, &mut Vec::new(), |_| usize::MAX).await.map_err(bad)?;
        let status = resp.start_line.split(' ').nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| bad(None))?;
        Ok((StatusCode(status), resp.body))
    }

    async fn json<T: DeserializeOwned>(&self, method: &str, path: &str, body: Option<Value>) -> T {
        let (status, bytes) = self.send(method, path, &[("accept", "application/json")], body.as_ref()).await;
        assert!(status.0 < 300, "{method} {path} failed with {}: {}", status.0, String::from_utf8_lossy(&bytes));
        serde_json::from_slice(&bytes).expect("parse JSON response")
    }

    /// Posts a JSON-RPC message to the MCP server, and returns the first event of the reply.
    pub async fn mcp(&self, body: Value) -> (StatusCode, Value) {
        self.mcp_at("/v0/mcp", body).await
    }

    /// Posts a JSON-RPC message to the MCP URL `path`, which can carry tool filters.
    pub async fn mcp_at(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let accept = [("accept", "application/json, text/event-stream")];
        let (status, bytes) = self.send("POST", path, &accept, Some(&body)).await;
        let text = String::from_utf8(bytes).expect("UTF-8 MCP reply");
        let data = text.strip_prefix("event: message\ndata: ").map_or(text.as_str(), |d| d.split('\n').next().unwrap());
        (status, if data.is_empty() { Value::Null } else { serde_json::from_str(data).expect("JSON-RPC reply") })
    }

    pub async fn info(&self) -> InfoResponse {
        self.json("GET", "/v1/info", None).await
    }

    /// Any request, for its status and JSON body.
    pub async fn request(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, bytes) = self.send(method, path, &[], body.as_ref()).await;
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// Desktop's OAuth flow with PKCE: the page of `/oauth/authorize` posts the request, which waits until the user
    /// approves it in the app.
    pub async fn oauth_pkce_login(&mut self, client_name: &str, code_verifier: &str) -> String {
        assert!(code_verifier.len() >= 16, "PKCE code_verifier too short");
        let body = json!({ "client_name": client_name, "redirect_uris": [REDIRECT_URI] });
        let reg: Value = self.json("POST", "/oauth/register", Some(body)).await;
        let client_id = reg["client_id"].as_str().expect("client_id");
        let challenge = base64_encode_url_nopad(&sha256(code_verifier.as_bytes()));
        let params = [
            ("client_id", client_id),
            ("redirect_uri", REDIRECT_URI),
            ("response_type", "code"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", "pkce_state_1"),
        ];
        let (status, page) = self.send("GET", &query_string("/oauth/authorize", &params), &[], None).await;
        let page = String::from_utf8(page).expect("UTF-8 authorize page");
        assert!(status == StatusCode::OK && page.contains("Continue in the app"), "authorize page: {page}");
        let body = json!({
            "mode": "oauth2",
            "clientInfo": { "name": client_name, "clientID": client_id },
            "scopes": ["read", "write"],
            "redirectUri": REDIRECT_URI,
            "scope": "read write",
            "state": "pkce_state_1",
            "codeChallenge": challenge,
            "codeChallengeMethod": "S256"
        });
        let auth: Value = self.json("POST", "/oauth/authorize/callback", Some(body)).await;
        assert_eq!(auth["state"], "pkce_state_1");
        let body = json!({
            "grant_type": "authorization_code",
            "client_id": client_id,
            "code": auth["code"],
            "redirect_uri": REDIRECT_URI,
            "code_verifier": code_verifier
        });
        let tok: Value = self.json("POST", "/oauth/token", Some(body)).await;
        self.access_token = tok["access_token"].as_str().expect("access_token").to_string();
        self.access_token.clone()
    }

    pub async fn setup_step(&self, step: &str, body: Value) -> (StatusCode, Value) {
        let (status, bytes) = self.send("POST", &format!("/v1/app/setup/{step}"), &[], Some(&body)).await;
        (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).expect("setup step JSON") })
    }

    pub async fn connect_bridge_session(&self, bridge_id: &str, flow_id: &str, mode: &str) -> BridgeLoginSession {
        let base = format!("/v1/bridges/{bridge_id}/login-sessions");
        let sess: BridgeLoginSession = self.json("POST", &base, Some(json!({ "flowID": flow_id }))).await;
        let path = format!("{base}/{}/steps/{}", sess.login_session_id, sess.step.step_id);
        let body = json!({ "mode": mode, "inputs": { "username": "+14155550199", "code": "994812" } });
        self.json("POST", &path, Some(body)).await
    }

    pub async fn list_accounts(&self) -> Vec<Account> {
        self.json("GET", "/v1/accounts", None).await
    }

    pub async fn list_chats(&self) -> CursorPage<Value> {
        self.json("GET", "/v1/chats", None).await
    }

    pub async fn merge_chats(&self, title: &str, chat_ids: Vec<String>, default_chat_id: Option<String>) -> Value {
        let body = json!({ "mode": "merge", "title": title, "merge": { "chatIDs": chat_ids, "defaultChatID": default_chat_id } });
        self.json("POST", "/v1/chats", Some(body)).await
    }

    pub async fn patch_chat_draft(&self, chat_id: &str, draft_text: Option<&str>) -> (StatusCode, Value) {
        let body = json!({ "draft": draft_text.map(|t| json!({ "text": t })) });
        let (status, bytes) = self.send("PATCH", &chat_path(chat_id, ""), &[], Some(&body)).await;
        (status, serde_json::from_slice(&bytes).expect("parse patch response"))
    }

    pub async fn focus(&self, chat_id: Option<&str>, message_id: Option<&str>, draft_text: Option<&str>) -> StatusCode {
        let body = json!({ "chatID": chat_id, "messageID": message_id, "draftText": draft_text });
        self.send("POST", "/v1/focus", &[], Some(&body)).await.0
    }

    pub async fn list_messages(&self, chat_id: &str) -> CursorPage<Value> {
        self.json("GET", &chat_path(chat_id, "/messages"), None).await
    }

    pub async fn send_message(
        &self,
        chat_id: &str,
        text: Option<&str>,
        reply_to: Option<&str>,
        upload_id: Option<&str>,
    ) -> Value {
        let body = json!({ "text": text, "replyToMessageID": reply_to, "attachment": upload_id.map(|u| json!({ "uploadID": u })) });
        self.json("POST", &chat_path(chat_id, "/messages"), Some(body)).await
    }

    pub async fn edit_message(&self, chat_id: &str, message_id: &str, text: &str) -> Value {
        self.json("PUT", &message_path(chat_id, message_id, ""), Some(json!({ "text": text }))).await
    }

    pub async fn delete_message(&self, chat_id: &str, message_id: &str) -> StatusCode {
        self.send("DELETE", &message_path(chat_id, message_id, ""), &[], None).await.0
    }

    pub async fn add_reaction(&self, chat_id: &str, message_id: &str, reaction_key: &str) -> Value {
        let body = json!({ "reactionKey": reaction_key });
        self.json("POST", &message_path(chat_id, message_id, "/reactions"), Some(body)).await
    }

    pub async fn remove_reaction(&self, chat_id: &str, message_id: &str, reaction_key: &str) -> Value {
        let path = message_path(chat_id, message_id, &format!("/reactions/{}", url_encode(reaction_key)));
        self.json("DELETE", &path, None).await
    }

    pub async fn upload_asset_base64(&self, file_name: &str, mime_type: &str, bytes: &[u8]) -> Value {
        let body = json!({ "content": base64_encode_standard(bytes), "fileName": file_name, "mimeType": mime_type });
        self.json("POST", "/v1/assets/upload/base64", Some(body)).await
    }

    pub async fn serve_asset_range(&self, url: &str, range: Option<&str>) -> (StatusCode, Vec<u8>) {
        let headers: Vec<(&str, &str)> = range.map(|r| ("range", r)).into_iter().collect();
        self.send("GET", &query_string("/v1/assets/serve", &[("url", url)]), &headers, None).await
    }

    pub async fn search_messages(&self, query: &str, media_types: Option<&str>) -> CursorPage<Value> {
        let params: Vec<(&str, &str)> = [("query", Some(query)), ("mediaTypes", media_types)]
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect();
        self.json("GET", &query_string("/v1/messages/search", &params), None).await
    }

    pub async fn unified_search(&self, query: &str) -> Value {
        self.json("GET", &query_string("/v1/search", &[("query", query)]), None).await
    }

    /// Opens `/v1/ws` and sends `subscriptions.set`. The receiver gets every frame, `ready` first.
    pub async fn connect_ws(&self, chat_ids: Vec<String>) -> UnboundedReceiver<Value> {
        let mut stream = TcpStream::connect(&self.host_port).await.expect("connect TCP to /v1/ws");
        let upgrade = format!(
            "GET /v1/ws HTTP/1.1\r\nhost: {}\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\nauthorization: Bearer {}\r\n\r\n",
            self.host_port, self.access_token
        );
        stream.write_all(upgrade.as_bytes()).await.expect("send WS upgrade request");
        let mut leftover = Vec::new();
        let resp = read_message(&mut stream, &mut leftover, |_| usize::MAX).await.expect("read WS upgrade response");
        assert!(
            resp.start_line.starts_with("HTTP/1.1 101"),
            "expected 101 Switching Protocols, got: {}",
            resp.start_line
        );
        let sub = json!({ "type": "subscriptions.set", "chatIDs": chat_ids }).to_string();
        write_ws_frame(&mut stream, 0x1, sub.as_bytes(), true).await.expect("send subscriptions.set");
        let (tx, rx) = unbounded_channel();
        tokio::spawn(async move {
            let mut reader = std::io::Cursor::new(leftover).chain(stream);
            while let Ok((0x1, payload)) = read_ws_frame(&mut reader, usize::MAX).await {
                if tx.send(serde_json::from_slice(&payload).expect("parse WS frame")).is_err() {
                    break;
                }
            }
        });
        rx
    }
}
