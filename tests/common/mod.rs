//! The requests of the end-to-end test as methods of [`BeeperClient`]: they panic when the server does not answer,
//! and they give the status with the reply.
use beeper_api::client::{chat_path, message_path, BeeperClient};
use beeper_api::http::{query_string, StatusCode};
use beeper_core::codec::{base64_encode_standard, url_encode};
use beeper_core::models::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

pub trait TestClient {
    fn api(&self) -> &BeeperClient;

    async fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&Value>) -> (StatusCode, Vec<u8>) {
        let body = body.map(|v| ("application/json".into(), serde_json::to_vec(v).expect("JSON values serialize")));
        let sent = self.api().request_raw(method, path, headers, body).await;
        let (status, bytes) = sent.expect("send the HTTP request to the Beeper server");
        (StatusCode(status), bytes)
    }

    async fn json<T: DeserializeOwned>(&self, method: &str, path: &str, body: Option<Value>) -> T {
        let (status, bytes) = self.send(method, path, &[("accept", "application/json")], body.as_ref()).await;
        assert!(status.0 < 300, "{method} {path} failed with {}: {}", status.0, String::from_utf8_lossy(&bytes));
        serde_json::from_slice(&bytes).expect("parse JSON response")
    }

    /// Posts a JSON-RPC message to the MCP server, and returns the first event of the reply.
    async fn mcp(&self, body: Value) -> (StatusCode, Value) {
        self.mcp_at("/v0/mcp", body).await
    }

    /// Posts a JSON-RPC message to the MCP URL `path`, which can carry tool filters.
    async fn mcp_at(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let accept = [("accept", "application/json, text/event-stream")];
        let (status, bytes) = self.send("POST", path, &accept, Some(&body)).await;
        let text = String::from_utf8(bytes).expect("UTF-8 MCP reply");
        let data = text.strip_prefix("event: message\ndata: ").map_or(text.as_str(), |d| d.split('\n').next().unwrap());
        (status, if data.is_empty() { Value::Null } else { serde_json::from_str(data).expect("JSON-RPC reply") })
    }

    async fn info(&self) -> InfoResponse {
        self.json("GET", "/v1/info", None).await
    }

    /// Any request, for its status and JSON body.
    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, bytes) = self.send(method, path, &[], body.as_ref()).await;
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn setup_step(&self, step: &str, body: Value) -> (StatusCode, Value) {
        let (status, bytes) = self.send("POST", &format!("/v1/app/setup/{step}"), &[], Some(&body)).await;
        (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).expect("setup step JSON") })
    }

    async fn list_accounts(&self) -> Vec<Account> {
        self.json("GET", "/v1/accounts", None).await
    }

    async fn list_chats(&self) -> CursorPage<Value> {
        self.json("GET", "/v1/chats", None).await
    }

    async fn merge_chats(&self, title: &str, chat_ids: Vec<String>, default_chat_id: Option<String>) -> Value {
        let body = json!({ "mode": "merge", "title": title, "merge": { "chatIDs": chat_ids, "defaultChatID": default_chat_id } });
        self.json("POST", "/v1/chats", Some(body)).await
    }

    async fn patch_chat_draft(&self, chat_id: &str, draft_text: Option<&str>) -> (StatusCode, Value) {
        let body = json!({ "draft": draft_text.map(|t| json!({ "text": t })) });
        let (status, bytes) = self.send("PATCH", &chat_path(chat_id, ""), &[], Some(&body)).await;
        (status, serde_json::from_slice(&bytes).expect("parse patch response"))
    }

    async fn focus(&self, chat_id: Option<&str>, message_id: Option<&str>, draft_text: Option<&str>) -> StatusCode {
        let fields = [("chatID", chat_id), ("messageID", message_id), ("draftText", draft_text)];
        let body = defined(fields.map(|(k, v)| (k, v.map(|v| json!(v)))));
        self.send("POST", "/v1/focus", &[], Some(&body)).await.0
    }

    async fn list_messages(&self, chat_id: &str) -> CursorPage<Value> {
        self.json("GET", &chat_path(chat_id, "/messages"), None).await
    }

    async fn send_message(&self, chat_id: &str, text: Option<&str>, reply_to: Option<&str>, upload_id: Option<&str>) -> Value {
        let body = defined([
            ("text", text.map(|t| json!(t))),
            ("replyToMessageID", reply_to.map(|r| json!(r))),
            ("attachment", upload_id.map(|u| json!({ "uploadID": u }))),
        ]);
        self.json("POST", &chat_path(chat_id, "/messages"), Some(body)).await
    }

    async fn edit_message(&self, chat_id: &str, message_id: &str, text: &str) -> Value {
        self.json("PUT", &message_path(chat_id, message_id, ""), Some(json!({ "text": text }))).await
    }

    async fn delete_message(&self, chat_id: &str, message_id: &str) -> StatusCode {
        self.send("DELETE", &message_path(chat_id, message_id, ""), &[], None).await.0
    }

    async fn add_reaction(&self, chat_id: &str, message_id: &str, reaction_key: &str) -> Value {
        let body = json!({ "reactionKey": reaction_key });
        self.json("POST", &message_path(chat_id, message_id, "/reactions"), Some(body)).await
    }

    async fn remove_reaction(&self, chat_id: &str, message_id: &str, reaction_key: &str) -> Value {
        let path = message_path(chat_id, message_id, &format!("/reactions/{}", url_encode(reaction_key)));
        self.json("DELETE", &path, None).await
    }

    async fn upload_asset_base64(&self, file_name: &str, mime_type: &str, bytes: &[u8]) -> Value {
        let body = json!({ "content": base64_encode_standard(bytes), "fileName": file_name, "mimeType": mime_type });
        self.json("POST", "/v1/assets/upload/base64", Some(body)).await
    }

    async fn serve_asset_range(&self, url: &str, range: Option<&str>) -> (StatusCode, Vec<u8>) {
        let headers: Vec<(&str, &str)> = range.map(|r| ("range", r)).into_iter().collect();
        self.send("GET", &query_string("/v1/assets/serve", &[("url", url)]), &headers, None).await
    }

    async fn search_messages(&self, query: &str, media_types: Option<&str>) -> CursorPage<Value> {
        let params: Vec<(&str, &str)> =
            [("query", Some(query)), ("mediaTypes", media_types)].into_iter().filter_map(|(k, v)| Some((k, v?))).collect();
        self.json("GET", &query_string("/v1/messages/search", &params), None).await
    }

    async fn unified_search(&self, query: &str) -> Value {
        self.json("GET", &query_string("/v1/search", &[("query", query)]), None).await
    }

    /// Opens `/v1/ws` and sends `subscriptions.set`. The receiver gets every frame, `ready` first.
    async fn connect_ws(&self, chat_ids: Vec<String>) -> UnboundedReceiver<Value> {
        let mut ws = self.api().ws_open("/v1/ws").await.expect("open /v1/ws");
        let sub = json!({ "type": "subscriptions.set", "chatIDs": chat_ids }).to_string();
        ws.send(&sub).await.expect("send subscriptions.set");
        let (tx, rx) = unbounded_channel();
        tokio::spawn(async move {
            while let Ok(text) = ws.next().await {
                if tx.send(serde_json::from_str(&text).expect("parse WS frame")).is_err() {
                    break;
                }
            }
        });
        rx
    }
}

impl TestClient for BeeperClient {
    fn api(&self) -> &BeeperClient {
        self
    }
}

/// A JSON body of the set fields, like the Stainless client, which leaves out unset ones: Desktop's zod schemas
/// refuse `null`.
fn defined<const N: usize>(fields: [(&str, Option<Value>); N]) -> Value {
    Value::Object(fields.into_iter().filter_map(|(k, v)| Some((k.to_string(), v?))).collect())
}
