use crate::server::{read_ws_frame, write_ws_frame, StatusCode};
use beeper_core::config::{
    base64_encode_standard, base64_encode_url_nopad, copy_beeper_data_to_isolated_dir, is_original_beeper_dir, sha256,
    url_encode,
};
use beeper_core::models::*;
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub fn extract_state_from_index_db(data_dir: &Path) -> (BeeperStateRecord, Vec<BridgeAccountInfo>) {
    let working_dir = if is_original_beeper_dir(data_dir) {
        let isolated = std::env::temp_dir().join(format!("beeper_rs_isolated_extract_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&isolated);
        copy_beeper_data_to_isolated_dir(data_dir, &isolated);
        isolated
    } else {
        data_dir.to_path_buf()
    };

    let db_path = working_dir.join("index.db");
    assert!(db_path.exists(), "isolated index.db does not exist at {:?}", db_path);
    let conn = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open isolated index.db readonly");

    let state_raw: String = conn
        .query_row("SELECT value FROM key_values WHERE key = 'beeperState'", [], |r| r.get(0))
        .expect("query beeperState from index.db");
    let state = serde_json::from_str(&state_raw).expect("parse beeperState");

    let bridges_raw: String = conn
        .query_row("SELECT value FROM key_values WHERE key = 'bridgeAccounts'", [], |r| r.get(0))
        .expect("query bridgeAccounts from index.db");
    let bridges = serde_json::from_str(&bridges_raw).expect("parse bridgeAccounts");

    (state, bridges)
}

#[derive(Clone)]
pub struct BeeperClient {
    pub base_url: String,
    pub host_port: String,
    pub access_token: String,
}

impl BeeperClient {
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        assert!(!base.is_empty(), "base_url must not be empty");
        let stripped = base.strip_prefix("http://").or_else(|| base.strip_prefix("https://")).unwrap_or(&base);
        let host_port = stripped.split('/').next().unwrap_or(stripped).to_string();
        Self { base_url: base, host_port, access_token: access_token.into() }
    }

    async fn raw_request(
        &self,
        method: &str,
        path_and_query: &str,
        extra_headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> (StatusCode, Vec<u8>) {
        let mut stream = TcpStream::connect(&self.host_port).await.expect("connect to Beeper server");
        let body_bytes = body.unwrap_or(&[]);
        let mut req = format!(
            "{method} {path_and_query} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.host_port,
            body_bytes.len()
        );
        if !self.access_token.is_empty() {
            req.push_str(&format!("Authorization: Bearer {}\r\n", self.access_token));
        }
        if body.is_some() {
            req.push_str("Content-Type: application/json\r\n");
        }
        for (k, v) in extra_headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).await.expect("write HTTP request header");
        if !body_bytes.is_empty() {
            stream.write_all(body_bytes).await.expect("write HTTP request body");
        }

        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 4096];
        let hdr_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut tmp).await.expect("read HTTP response");
            assert!(n > 0, "unexpected EOF reading HTTP response header");
            buf.extend_from_slice(&tmp[..n]);
        };
        let hdr_str = std::str::from_utf8(&buf[..hdr_end - 4]).expect("HTTP response header utf8");
        let mut lines = hdr_str.lines();
        let status_line = lines.next().expect("HTTP status line");
        let status_code: u16 =
            status_line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).expect("HTTP status code");
        let mut content_len = 0usize;
        for line in lines {
            if let Some((k, v)) = line.split_once(':') {
                if k.trim().eq_ignore_ascii_case("content-length") {
                    content_len = v.trim().parse().unwrap_or(0);
                }
            }
        }
        while buf.len() - hdr_end < content_len {
            let n = stream.read(&mut tmp).await.expect("read HTTP response body");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        (StatusCode(status_code), buf[hdr_end..hdr_end + content_len].to_vec())
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        method: &str,
        path_and_query: &str,
        extra_headers: &[(&str, &str)],
        json_body: Option<&Value>,
    ) -> T {
        let body_vec = json_body.map(|v| serde_json::to_vec(v).expect("serialize request JSON"));
        let (_, resp_body) = self.raw_request(method, path_and_query, extra_headers, body_vec.as_deref()).await;
        serde_json::from_slice(&resp_body).expect("parse JSON response")
    }

    fn build_query_path(path: &str, params: &[(&str, &str)]) -> String {
        if params.is_empty() {
            return path.to_string();
        }
        let mut out = format!("{path}?");
        for (i, (k, v)) in params.iter().enumerate() {
            if i > 0 {
                out.push('&');
            }
            out.push_str(&url_encode(k));
            out.push('=');
            out.push_str(&url_encode(v));
        }
        out
    }

    pub async fn info(&self) -> InfoResponse {
        let (status, bytes) = self.raw_request("GET", "/v1/info", &[], None).await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_slice(&bytes).expect("parse InfoResponse")
    }

    pub async fn oauth_pkce_login(&mut self, client_name: &str, code_verifier: &str) -> String {
        assert!(code_verifier.len() >= 16, "PKCE code_verifier too short");
        let reg_resp: Value = self
            .request_json(
                "POST",
                "/oauth/register",
                &[],
                Some(&json!({
                    "client_name": client_name,
                    "redirect_uris": ["http://127.0.0.1:0/callback"]
                })),
            )
            .await;
        let client_id = reg_resp["client_id"].as_str().expect("client_id").to_string();

        let challenge = base64_encode_url_nopad(&sha256(code_verifier.as_bytes()));
        let auth_path = Self::build_query_path(
            "/oauth/authorize",
            &[
                ("client_id", client_id.as_str()),
                ("redirect_uri", "http://127.0.0.1:0/callback"),
                ("response_type", "code"),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", "pkce_state_1"),
            ],
        );
        let auth_resp: Value = self.request_json("GET", &auth_path, &[("Accept", "application/json")], None).await;
        let code = auth_resp["code"].as_str().expect("auth code").to_string();

        let tok_resp: Value = self
            .request_json(
                "POST",
                "/oauth/token",
                &[],
                Some(&json!({
                    "grant_type": "authorization_code",
                    "client_id": client_id,
                    "code": code,
                    "redirect_uri": "http://127.0.0.1:0/callback",
                    "code_verifier": code_verifier
                })),
            )
            .await;
        let token = tok_resp["access_token"].as_str().expect("access_token present").to_string();
        self.access_token = token.clone();
        token
    }

    pub async fn setup_email_login(&self, email: &str, code: &str) -> SetupStatusResponse {
        let _start: SetupStatusResponse = self
            .request_json("POST", "/v1/app/setup/start", &[], Some(&json!({ "reset": true, "env": "PRODUCTION" })))
            .await;

        let email_step: SetupStatusResponse = self
            .request_json("POST", "/v1/app/setup/email", &[], Some(&json!({ "email": email, "env": "PRODUCTION" })))
            .await;
        let login_token =
            email_step.setup_flow.as_ref().and_then(|f| f.login_token.clone()).expect("login_token from email step");

        self.request_json(
            "POST",
            "/v1/app/setup/response",
            &[],
            Some(&json!({
                "code": code,
                "email": email,
                "loginToken": login_token,
                "env": "PRODUCTION"
            })),
        )
        .await
    }

    pub async fn verify_recovery_key(&self, recovery_key: &str) -> Value {
        self.request_json(
            "POST",
            "/v1/app/setup/verification/recovery-key",
            &[],
            Some(&json!({ "recoveryKey": recovery_key })),
        )
        .await
    }

    pub async fn verify_device_sas(&self) -> Value {
        let started: Value = self.request_json("POST", "/v1/app/setup/verifications", &[], None).await;
        let vid = started["verification"]["verificationID"].as_str().expect("verificationID").to_string();

        let sas_started: Value =
            self.request_json("POST", &format!("/v1/app/setup/verifications/{vid}/sas/start"), &[], None).await;
        assert!(!sas_started["verification"]["sas"]["emojis"].as_array().expect("emojis array").is_empty());

        self.request_json("POST", &format!("/v1/app/setup/verifications/{vid}/sas/confirm"), &[], None).await
    }

    pub async fn connect_bridge_session(&self, bridge_id: &str, flow_id: &str, mode: &str) -> BridgeLoginSession {
        let sess: BridgeLoginSession = self
            .request_json(
                "POST",
                &format!("/v1/bridges/{bridge_id}/login-sessions"),
                &[],
                Some(&json!({ "flowID": flow_id })),
            )
            .await;

        self.request_json(
            "POST",
            &format!("/v1/bridges/{bridge_id}/login-sessions/{}/steps/{}", sess.login_session_id, sess.step.step_id),
            &[],
            Some(&json!({ "mode": mode, "inputs": { "username": "+14155550199", "code": "994812" } })),
        )
        .await
    }

    pub async fn list_accounts(&self) -> Vec<Account> {
        self.request_json("GET", "/v1/accounts", &[], None).await
    }

    pub async fn list_chats(&self) -> CursorPage<Chat> {
        self.request_json("GET", "/v1/chats", &[], None).await
    }

    pub async fn merge_chats(&self, title: &str, chat_ids: Vec<String>, default_chat_id: Option<String>) -> Value {
        self.request_json(
            "POST",
            "/v1/chats",
            &[],
            Some(&json!({
                "mode": "merge",
                "title": title,
                "merge": {
                    "chatIDs": chat_ids,
                    "defaultChatID": default_chat_id
                }
            })),
        )
        .await
    }

    pub async fn patch_chat_draft(&self, chat_id: &str, draft_text: Option<&str>) -> (StatusCode, Value) {
        let payload = match draft_text {
            Some(t) => json!({ "draft": { "text": t } }),
            None => json!({ "draft": null }),
        };
        let body = serde_json::to_vec(&payload).unwrap();
        let (status, resp_bytes) =
            self.raw_request("PATCH", &format!("/v1/chats/{}", url_encode(chat_id)), &[], Some(&body)).await;
        let val: Value = serde_json::from_slice(&resp_bytes).expect("parse patch response");
        (status, val)
    }

    pub async fn list_messages(&self, chat_id: &str) -> CursorPage<Message> {
        self.request_json("GET", &format!("/v1/chats/{}/messages", url_encode(chat_id)), &[], None).await
    }

    pub async fn send_message(
        &self,
        chat_id: &str,
        text: Option<&str>,
        reply_to: Option<&str>,
        upload_id: Option<&str>,
    ) -> Value {
        let mut body = json!({});
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        if let Some(r) = reply_to {
            body["replyToMessageID"] = json!(r);
        }
        if let Some(u) = upload_id {
            body["attachment"] = json!({ "uploadID": u });
        }
        self.request_json("POST", &format!("/v1/chats/{}/messages", url_encode(chat_id)), &[], Some(&body)).await
    }

    pub async fn edit_message(&self, chat_id: &str, message_id: &str, text: &str) -> Value {
        self.request_json(
            "PUT",
            &format!("/v1/chats/{}/messages/{}", url_encode(chat_id), url_encode(message_id)),
            &[],
            Some(&json!({ "text": text })),
        )
        .await
    }

    pub async fn delete_message(&self, chat_id: &str, message_id: &str) -> Value {
        self.request_json(
            "DELETE",
            &format!("/v1/chats/{}/messages/{}", url_encode(chat_id), url_encode(message_id)),
            &[],
            None,
        )
        .await
    }

    pub async fn add_reaction(&self, chat_id: &str, message_id: &str, reaction_key: &str) -> Value {
        self.request_json(
            "POST",
            &format!("/v1/chats/{}/messages/{}/reactions", url_encode(chat_id), url_encode(message_id)),
            &[],
            Some(&json!({ "reactionKey": reaction_key })),
        )
        .await
    }

    pub async fn remove_reaction(&self, chat_id: &str, message_id: &str, reaction_key: &str) -> Value {
        self.request_json(
            "DELETE",
            &format!(
                "/v1/chats/{}/messages/{}/reactions/{}",
                url_encode(chat_id),
                url_encode(message_id),
                url_encode(reaction_key)
            ),
            &[],
            None,
        )
        .await
    }

    pub async fn upload_asset_base64(&self, file_name: &str, mime_type: &str, bytes: &[u8]) -> Value {
        let b64 = base64_encode_standard(bytes);
        self.request_json(
            "POST",
            "/v1/assets/upload/base64",
            &[],
            Some(&json!({
                "content": b64,
                "fileName": file_name,
                "mimeType": mime_type
            })),
        )
        .await
    }

    pub async fn serve_asset_range(&self, url: &str, range: Option<&str>) -> (StatusCode, Vec<u8>) {
        let path = Self::build_query_path("/v1/assets/serve", &[("url", url)]);
        let mut hdrs = Vec::new();
        if let Some(r) = range {
            hdrs.push(("Range", r));
        }
        self.raw_request("GET", &path, &hdrs, None).await
    }

    pub async fn search_messages(&self, query: &str, media_types: Option<&str>) -> CursorPage<Message> {
        let mut q = vec![("query", query)];
        if let Some(mt) = media_types {
            q.push(("mediaTypes", mt));
        }
        let path = Self::build_query_path("/v1/messages/search", &q);
        self.request_json("GET", &path, &[], None).await
    }

    pub async fn unified_search(&self, query: &str) -> Value {
        let path = Self::build_query_path("/v1/search", &[("query", query)]);
        self.request_json("GET", &path, &[], None).await
    }

    pub async fn connect_ws(&self, chat_ids: Vec<String>) -> tokio::sync::mpsc::UnboundedReceiver<WsEvent> {
        let mut stream = TcpStream::connect(&self.host_port).await.expect("connect TCP to /v1/ws");
        let handshake = format!(
            "GET /v1/ws HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
            self.host_port
        );
        stream.write_all(handshake.as_bytes()).await.expect("send WS upgrade request");

        let mut hdr_buf = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        while !hdr_buf.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut one).await.expect("read WS upgrade header byte");
            hdr_buf.push(one[0]);
        }
        let hdr_str = std::str::from_utf8(&hdr_buf).expect("WS handshake utf8");
        assert!(hdr_str.starts_with("HTTP/1.1 101"), "expected 101 Switching Protocols, got: {hdr_str}");

        let sub_msg = json!({
            "type": "subscriptions.set",
            "chatIDs": chat_ids
        })
        .to_string();
        write_ws_frame(&mut stream, sub_msg.as_bytes(), true).await.expect("send subscriptions.set");

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            while let Some((opcode, payload)) = read_ws_frame(&mut reader).await {
                if opcode == 0x08 {
                    break;
                }
                if opcode == 0x01 {
                    if let Ok(txt) = std::str::from_utf8(&payload) {
                        if let Ok(ev) = serde_json::from_str::<WsEvent>(txt) {
                            let _ = tx.send(ev);
                        }
                    }
                }
            }
        });
        rx
    }
}
