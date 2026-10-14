use crate::models::*;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use futures_util::{SinkExt, StreamExt};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as TungsteniteMsg};

pub fn extract_state_from_index_db(data_dir: &Path) -> (BeeperStateRecord, Vec<BridgeAccountInfo>) {
    let db_path = data_dir.join("index.db");
    assert!(
        db_path.exists(),
        "BeeperTexts/index.db does not exist at {:?}",
        db_path
    );
    let conn = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open index.db readonly");

    let state_raw: String = conn
        .query_row(
            "SELECT value FROM key_values WHERE key = 'beeperState'",
            [],
            |r| r.get(0),
        )
        .expect("query beeperState from index.db");
    let state: BeeperStateRecord =
        serde_json::from_str(&state_raw).expect("parse beeperState JSON");

    let bridges_raw: String = conn
        .query_row(
            "SELECT value FROM key_values WHERE key = 'bridgeAccounts'",
            [],
            |r| r.get(0),
        )
        .expect("query bridgeAccounts from index.db");
    let bridges: Vec<BridgeAccountInfo> =
        serde_json::from_str(&bridges_raw).expect("parse bridgeAccounts JSON");

    (state, bridges)
}

#[derive(Clone)]
pub struct BeeperClient {
    pub base_url: String,
    pub access_token: String,
    http: reqwest::Client,
}

impl BeeperClient {
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        assert!(!base.is_empty(), "base_url must not be empty");
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("build reqwest client");
        Self {
            base_url: base,
            access_token: access_token.into(),
            http,
        }
    }

    pub async fn info(&self) -> InfoResponse {
        let resp = self
            .http
            .get(format!("{}/v1/info", self.base_url))
            .send()
            .await
            .expect("GET /v1/info");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        resp.json().await.expect("parse InfoResponse")
    }

    pub async fn oauth_pkce_login(&mut self, client_name: &str, code_verifier: &str) -> String {
        assert!(code_verifier.len() >= 16, "PKCE code_verifier too short");
        let reg_resp: Value = self
            .http
            .post(format!("{}/oauth/register", self.base_url))
            .json(&json!({
                "client_name": client_name,
                "redirect_uris": ["http://127.0.0.1:0/callback"]
            }))
            .send()
            .await
            .expect("POST /oauth/register")
            .json()
            .await
            .expect("parse client reg");
        let client_id = reg_resp["client_id"]
            .as_str()
            .expect("client_id")
            .to_string();

        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

        let auth_resp: Value = self
            .http
            .get(format!("{}/oauth/authorize", self.base_url))
            .header("Accept", "application/json")
            .query(&[
                ("client_id", client_id.as_str()),
                ("redirect_uri", "http://127.0.0.1:0/callback"),
                ("response_type", "code"),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", "pkce_state_1"),
            ])
            .send()
            .await
            .expect("GET /oauth/authorize")
            .json()
            .await
            .expect("parse auth code");
        let code = auth_resp["code"].as_str().expect("auth code").to_string();

        let tok_resp: Value = self
            .http
            .post(format!("{}/oauth/token", self.base_url))
            .json(&json!({
                "grant_type": "authorization_code",
                "client_id": client_id,
                "code": code,
                "redirect_uri": "http://127.0.0.1:0/callback",
                "code_verifier": code_verifier
            }))
            .send()
            .await
            .expect("POST /oauth/token")
            .json()
            .await
            .expect("parse token response");
        let token = tok_resp["access_token"]
            .as_str()
            .expect("access_token present")
            .to_string();
        self.access_token = token.clone();
        token
    }

    pub async fn setup_email_login(&self, email: &str, code: &str) -> SetupStatusResponse {
        let _start: SetupStatusResponse = self
            .http
            .post(format!("{}/v1/app/setup/start", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&json!({ "reset": true, "env": "PRODUCTION" }))
            .send()
            .await
            .expect("setup start")
            .json()
            .await
            .expect("parse setup start");

        let email_step: SetupStatusResponse = self
            .http
            .post(format!("{}/v1/app/setup/email", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&json!({ "email": email, "env": "PRODUCTION" }))
            .send()
            .await
            .expect("setup email")
            .json()
            .await
            .expect("parse setup email");
        let login_token = email_step
            .setup_flow
            .as_ref()
            .and_then(|f| f.login_token.clone())
            .expect("login_token from email step");

        self.http
            .post(format!("{}/v1/app/setup/response", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&json!({
                "code": code,
                "email": email,
                "loginToken": login_token,
                "env": "PRODUCTION"
            }))
            .send()
            .await
            .expect("setup response")
            .json()
            .await
            .expect("parse setup response")
    }

    pub async fn verify_recovery_key(&self, recovery_key: &str) -> Value {
        self.http
            .post(format!(
                "{}/v1/app/setup/verification/recovery-key",
                self.base_url
            ))
            .bearer_auth(&self.access_token)
            .json(&json!({ "recoveryKey": recovery_key }))
            .send()
            .await
            .expect("recovery key verify")
            .json()
            .await
            .expect("parse recovery key verify")
    }

    pub async fn verify_device_sas(&self) -> Value {
        let started: Value = self
            .http
            .post(format!("{}/v1/app/setup/verifications", self.base_url))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("start verification")
            .json()
            .await
            .expect("parse start verification");
        let vid = started["verification"]["verificationID"]
            .as_str()
            .expect("verificationID")
            .to_string();

        let sas_started: Value = self
            .http
            .post(format!(
                "{}/v1/app/setup/verifications/{vid}/sas/start",
                self.base_url
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("sas start")
            .json()
            .await
            .expect("parse sas start");
        assert!(!sas_started["verification"]["sas"]["emojis"]
            .as_array()
            .expect("emojis array")
            .is_empty());

        self.http
            .post(format!(
                "{}/v1/app/setup/verifications/{vid}/sas/confirm",
                self.base_url
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("sas confirm")
            .json()
            .await
            .expect("parse sas confirm")
    }

    pub async fn connect_bridge_session(
        &self,
        bridge_id: &str,
        flow_id: &str,
        mode: &str,
    ) -> BridgeLoginSession {
        let sess: BridgeLoginSession = self
            .http
            .post(format!(
                "{}/v1/bridges/{bridge_id}/login-sessions",
                self.base_url
            ))
            .bearer_auth(&self.access_token)
            .json(&json!({ "flowID": flow_id }))
            .send()
            .await
            .expect("start bridge session")
            .json()
            .await
            .expect("parse bridge session");

        self.http
            .post(format!(
                "{}/v1/bridges/{bridge_id}/login-sessions/{}/steps/{}",
                self.base_url, sess.login_session_id, sess.step.step_id
            ))
            .bearer_auth(&self.access_token)
            .json(&json!({ "mode": mode, "inputs": { "username": "+14155550199", "code": "994812" } }))
            .send()
            .await
            .expect("submit bridge step")
            .json()
            .await
            .expect("parse completed bridge session")
    }

    pub async fn list_accounts(&self) -> Vec<Account> {
        self.http
            .get(format!("{}/v1/accounts", self.base_url))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("GET /v1/accounts")
            .json()
            .await
            .expect("parse accounts")
    }

    pub async fn list_chats(&self) -> CursorPage<Chat> {
        self.http
            .get(format!("{}/v1/chats", self.base_url))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("GET /v1/chats")
            .json()
            .await
            .expect("parse chats page")
    }

    pub async fn merge_chats(
        &self,
        title: &str,
        chat_ids: Vec<String>,
        default_chat_id: Option<String>,
    ) -> Value {
        self.http
            .post(format!("{}/v1/chats", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&json!({
                "mode": "merge",
                "title": title,
                "merge": {
                    "chatIDs": chat_ids,
                    "defaultChatID": default_chat_id
                }
            }))
            .send()
            .await
            .expect("POST /v1/chats merge")
            .json()
            .await
            .expect("parse merge response")
    }

    pub async fn patch_chat_draft(
        &self,
        chat_id: &str,
        draft_text: Option<&str>,
    ) -> (reqwest::StatusCode, Value) {
        let payload = match draft_text {
            Some(t) => json!({ "draft": { "text": t } }),
            None => json!({ "draft": null }),
        };
        let resp = self
            .http
            .patch(format!(
                "{}/v1/chats/{}",
                self.base_url,
                urlencoding::encode(chat_id)
            ))
            .bearer_auth(&self.access_token)
            .json(&payload)
            .send()
            .await
            .expect("PATCH /v1/chats/:id");
        let status = resp.status();
        let val: Value = resp.json().await.expect("parse patch response");
        (status, val)
    }

    pub async fn list_messages(&self, chat_id: &str) -> CursorPage<Message> {
        self.http
            .get(format!(
                "{}/v1/chats/{}/messages",
                self.base_url,
                urlencoding::encode(chat_id)
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("GET /v1/chats/:id/messages")
            .json()
            .await
            .expect("parse messages page")
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
        self.http
            .post(format!(
                "{}/v1/chats/{}/messages",
                self.base_url,
                urlencoding::encode(chat_id)
            ))
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await
            .expect("POST /v1/chats/:id/messages")
            .json()
            .await
            .expect("parse send response")
    }

    pub async fn edit_message(&self, chat_id: &str, message_id: &str, text: &str) -> Value {
        self.http
            .put(format!(
                "{}/v1/chats/{}/messages/{}",
                self.base_url,
                urlencoding::encode(chat_id),
                urlencoding::encode(message_id)
            ))
            .bearer_auth(&self.access_token)
            .json(&json!({ "text": text }))
            .send()
            .await
            .expect("PUT /v1/chats/:id/messages/:mid")
            .json()
            .await
            .expect("parse edit response")
    }

    pub async fn delete_message(&self, chat_id: &str, message_id: &str) -> Value {
        self.http
            .delete(format!(
                "{}/v1/chats/{}/messages/{}",
                self.base_url,
                urlencoding::encode(chat_id),
                urlencoding::encode(message_id)
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("DELETE /v1/chats/:id/messages/:mid")
            .json()
            .await
            .expect("parse delete response")
    }

    pub async fn add_reaction(&self, chat_id: &str, message_id: &str, reaction_key: &str) -> Value {
        self.http
            .post(format!(
                "{}/v1/chats/{}/messages/{}/reactions",
                self.base_url,
                urlencoding::encode(chat_id),
                urlencoding::encode(message_id)
            ))
            .bearer_auth(&self.access_token)
            .json(&json!({ "reactionKey": reaction_key }))
            .send()
            .await
            .expect("POST reaction")
            .json()
            .await
            .expect("parse reaction add")
    }

    pub async fn remove_reaction(
        &self,
        chat_id: &str,
        message_id: &str,
        reaction_key: &str,
    ) -> Value {
        self.http
            .delete(format!(
                "{}/v1/chats/{}/messages/{}/reactions/{}",
                self.base_url,
                urlencoding::encode(chat_id),
                urlencoding::encode(message_id),
                urlencoding::encode(reaction_key)
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .expect("DELETE reaction")
            .json()
            .await
            .expect("parse reaction delete")
    }

    pub async fn upload_asset_base64(
        &self,
        file_name: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Value {
        let b64 = STANDARD.encode(bytes);
        self.http
            .post(format!("{}/v1/assets/upload/base64", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&json!({
                "content": b64,
                "fileName": file_name,
                "mimeType": mime_type
            }))
            .send()
            .await
            .expect("POST /v1/assets/upload/base64")
            .json()
            .await
            .expect("parse upload response")
    }

    pub async fn serve_asset_range(
        &self,
        url: &str,
        range: Option<&str>,
    ) -> (reqwest::StatusCode, Vec<u8>) {
        let mut req = self
            .http
            .get(format!("{}/v1/assets/serve", self.base_url))
            .bearer_auth(&self.access_token)
            .query(&[("url", url)]);
        if let Some(r) = range {
            req = req.header("Range", r);
        }
        let resp = req.send().await.expect("GET /v1/assets/serve");
        let status = resp.status();
        let bytes = resp.bytes().await.expect("asset bytes").to_vec();
        (status, bytes)
    }

    pub async fn search_messages(
        &self,
        query: &str,
        media_types: Option<&str>,
    ) -> CursorPage<Message> {
        let mut q = vec![("query", query)];
        if let Some(mt) = media_types {
            q.push(("mediaTypes", mt));
        }
        self.http
            .get(format!("{}/v1/messages/search", self.base_url))
            .bearer_auth(&self.access_token)
            .query(&q)
            .send()
            .await
            .expect("GET /v1/messages/search")
            .json()
            .await
            .expect("parse message search")
    }

    pub async fn unified_search(&self, query: &str) -> Value {
        self.http
            .get(format!("{}/v1/search", self.base_url))
            .bearer_auth(&self.access_token)
            .query(&[("query", query)])
            .send()
            .await
            .expect("GET /v1/search")
            .json()
            .await
            .expect("parse unified search")
    }

    pub async fn connect_ws(
        &self,
        chat_ids: Vec<String>,
    ) -> tokio::sync::mpsc::UnboundedReceiver<WsEvent> {
        let ws_url = self
            .base_url
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1)
            + "/v1/ws";
        let (ws_stream, _) = connect_async(&ws_url).await.expect("connect to /v1/ws");
        let (mut write, mut read) = ws_stream.split();

        let sub_msg = json!({
            "type": "subscriptions.set",
            "chatIDs": chat_ids
        })
        .to_string();
        write
            .send(TungsteniteMsg::Text(sub_msg))
            .await
            .expect("send subscriptions.set");

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _keep_write = write;
            while let Some(Ok(msg)) = read.next().await {
                if let TungsteniteMsg::Text(txt) = msg {
                    if let Ok(ev) = serde_json::from_str::<WsEvent>(&txt) {
                        let _ = tx.send(ev);
                    }
                }
            }
        });
        rx
    }
}
