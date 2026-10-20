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
}

const REDIRECT_URI: &str = "http://127.0.0.1:0/callback";

fn chat_path(chat_id: &str, rest: &str) -> String {
    format!("/v1/chats/{}{rest}", url_encode(chat_id))
}

fn message_path(chat_id: &str, message_id: &str, rest: &str) -> String {
    chat_path(chat_id, &format!("/messages/{}{rest}", url_encode(message_id)))
}

impl BeeperClient {
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let host = base_url.strip_prefix("http://").expect("base_url starts with http://");
        Self { host_port: host.split('/').next().unwrap().to_string(), access_token: access_token.into() }
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> (StatusCode, Vec<u8>) {
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
        let mut stream = TcpStream::connect(&self.host_port).await.expect("connect to Beeper server");
        stream.write_all(&[head.into_bytes(), body.unwrap_or_default()].concat()).await.expect("write HTTP request");
        let resp = read_message(&mut stream, &mut Vec::new()).await.expect("read HTTP response");
        let status = resp.start_line.split(' ').nth(1).and_then(|s| s.parse().ok()).expect("HTTP status code");
        (StatusCode(status), resp.body)
    }

    async fn json<T: DeserializeOwned>(&self, method: &str, path: &str, body: Option<Value>) -> T {
        let (status, bytes) = self.send(method, path, &[("accept", "application/json")], body.as_ref()).await;
        assert!(status.0 < 300, "{method} {path} failed with {}: {}", status.0, String::from_utf8_lossy(&bytes));
        serde_json::from_slice(&bytes).expect("parse JSON response")
    }

    /// Posts a JSON-RPC message to the MCP server, and returns the first event of the reply.
    pub async fn mcp(&self, body: Value) -> (StatusCode, Value) {
        let accept = [("accept", "application/json, text/event-stream")];
        let (status, bytes) = self.send("POST", "/v0/mcp", &accept, Some(&body)).await;
        let text = String::from_utf8(bytes).expect("UTF-8 MCP reply");
        let data = text.strip_prefix("event: message\ndata: ").map_or(text.as_str(), |d| d.split('\n').next().unwrap());
        (status, if data.is_empty() { Value::Null } else { serde_json::from_str(data).expect("JSON-RPC reply") })
    }

    pub async fn info(&self) -> InfoResponse {
        self.json("GET", "/v1/info", None).await
    }

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
        let auth: Value = self.json("GET", &query_string("/oauth/authorize", &params), None).await;
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

    pub async fn verify_recovery_key(&self, recovery_key: &str) -> Value {
        let body = json!({ "recoveryKey": recovery_key });
        self.json("POST", "/v1/app/setup/verification/recovery-key", Some(body)).await
    }

    pub async fn verify_device_sas(&self) -> Value {
        let started: Value = self.json("POST", "/v1/app/setup/verifications", None).await;
        let base =
            format!("/v1/app/setup/verifications/{}", started["verification"]["verificationID"].as_str().unwrap());
        let sas: Value = self.json("POST", &format!("{base}/sas/start"), None).await;
        assert!(!sas["verification"]["sas"]["emojis"].as_array().expect("emojis array").is_empty());
        self.json("POST", &format!("{base}/sas/confirm"), None).await
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

    pub async fn list_chats(&self) -> CursorPage<Chat> {
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

    pub async fn list_messages(&self, chat_id: &str) -> CursorPage<Message> {
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

    pub async fn delete_message(&self, chat_id: &str, message_id: &str) -> Value {
        self.json("DELETE", &message_path(chat_id, message_id, ""), None).await
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

    pub async fn search_messages(&self, query: &str, media_types: Option<&str>) -> CursorPage<Message> {
        let params: Vec<(&str, &str)> = [("query", Some(query)), ("mediaTypes", media_types)]
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect();
        self.json("GET", &query_string("/v1/messages/search", &params), None).await
    }

    pub async fn unified_search(&self, query: &str) -> Value {
        self.json("GET", &query_string("/v1/search", &[("query", query)]), None).await
    }

    pub async fn connect_ws(&self, chat_ids: Vec<String>) -> UnboundedReceiver<WsEvent> {
        let mut stream = TcpStream::connect(&self.host_port).await.expect("connect TCP to /v1/ws");
        let upgrade = format!(
            "GET /v1/ws HTTP/1.1\r\nhost: {}\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\n\r\n",
            self.host_port
        );
        stream.write_all(upgrade.as_bytes()).await.expect("send WS upgrade request");
        let mut leftover = Vec::new();
        let resp = read_message(&mut stream, &mut leftover).await.expect("read WS upgrade response");
        assert!(
            resp.start_line.starts_with("HTTP/1.1 101"),
            "expected 101 Switching Protocols, got: {}",
            resp.start_line
        );
        let sub = json!({ "type": "subscriptions.set", "chatIDs": chat_ids }).to_string();
        write_ws_frame(&mut stream, sub.as_bytes(), true).await.expect("send subscriptions.set");
        let (tx, rx) = unbounded_channel();
        tokio::spawn(async move {
            let mut reader = std::io::Cursor::new(leftover).chain(stream);
            while let Some((0x1, payload)) = read_ws_frame(&mut reader).await {
                if tx.send(serde_json::from_slice(&payload).expect("parse WS event")).is_err() {
                    break;
                }
            }
        });
        rx
    }
}
