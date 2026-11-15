//! A client of the Desktop API over plain HTTP/1.1 and WebSocket, like the request layer of the Desktop API SDK.
use crate::codec::{base64_encode_url_nopad, sha256, url_encode};
use crate::http::{query_string, read_message, read_ws_frame, write_ws_frame, StatusCode};
use beeper_core::connect::random_id;
use serde_json::{json, Value};
use std::fmt::Write;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A client of one Desktop API server, with the access token that it sends.
#[derive(Clone)]
pub struct BeeperClient {
    /// The `host:port` of the server.
    pub host_port: String,
    /// The bearer token of the requests; empty sends no token.
    pub access_token: String,
    /// Prints each request and its status on stderr, like the `debug` log level of the SDK.
    pub debug: bool,
}

/// A failed request, with the text of the Desktop API SDK: `<status> <message>`, or `Connection error.`.
#[derive(Debug)]
pub struct ApiError(pub String);

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ApiError {}

fn json_body(body: Option<&Value>) -> Option<(String, Vec<u8>)> {
    body.map(|v| ("application/json".into(), serde_json::to_vec(v).expect("JSON values serialize")))
}

fn json_or_text(bytes: Vec<u8>) -> Value {
    match bytes.is_empty() {
        true => Value::Null,
        false => serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    }
}

/// The body of a 2xx or 3xx reply, else the error text of the SDK's `APIError`.
fn checked(status: u16, bytes: Vec<u8>) -> Result<Vec<u8>, ApiError> {
    if status < 400 {
        return Ok(bytes);
    }
    let msg = match serde_json::from_slice::<Value>(&bytes).ok() {
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

const REDIRECT_URI: &str = "http://127.0.0.1:0/callback";

/// The path of a chat, then `rest`: `/v1/chats/<chat_id><rest>`.
pub fn chat_path(chat_id: &str, rest: &str) -> String {
    format!("/v1/chats/{}{rest}", url_encode(chat_id))
}

/// The path of a message, then `rest`: `/v1/chats/<chat_id>/messages/<message_id><rest>`.
pub fn message_path(chat_id: &str, message_id: &str, rest: &str) -> String {
    chat_path(chat_id, &format!("/messages/{}{rest}", url_encode(message_id)))
}

impl BeeperClient {
    /// A client of `base_url`. It panics when the URL does not start with `http://`; see [`Self::try_new`].
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
        Ok(json_or_text(self.call_raw(method, path, body).await?))
    }

    /// The status and the body of a request, or `Connection error.` when the server does not answer.
    pub async fn fetch(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Vec<u8>), ApiError> {
        self.request_raw(method, path, &[("accept", "application/json")], json_body(body)).await
    }

    /// Like [`Self::fetch`], with the headers, and the content type and the bytes of the body.
    pub async fn request_raw(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<(String, Vec<u8>)>,
    ) -> Result<(u16, Vec<u8>), ApiError> {
        let (status, bytes) =
            self.try_send(method, path, headers, body).await.map_err(|_| ApiError("Connection error.".into()))?;
        if self.debug {
            eprintln!("[beeper-api] {method} {path} -> {}", status.0);
        }
        Ok((status.0, bytes))
    }

    /// Like [`Self::call`], with the raw body of the reply.
    pub async fn call_raw(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Vec<u8>, ApiError> {
        let (status, bytes) = self.fetch(method, path, body).await?;
        checked(status, bytes)
    }

    /// The `assets.upload` of the SDK: a multipart form with the file, then the `fields` (`fileName`, `mimeType`).
    pub async fn upload(&self, name: &str, bytes: &[u8], fields: &[(&str, &str)]) -> Result<Value, ApiError> {
        const BOUNDARY: &str = "----beeper-rs-upload";
        let name = name.replace('"', "%22");
        let head = format!("--{BOUNDARY}\r\ncontent-disposition: form-data; name=\"file\"; filename=\"{name}\"\r\ncontent-type: application/octet-stream\r\n\r\n");
        let mut body = [head.as_bytes(), bytes].concat();
        for (k, v) in fields {
            body.extend(
                format!("\r\n--{BOUNDARY}\r\ncontent-disposition: form-data; name=\"{k}\"\r\n\r\n{v}").as_bytes(),
            );
        }
        body.extend(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let form = (format!("multipart/form-data; boundary={BOUNDARY}"), body);
        let (status, bytes) =
            self.request_raw("POST", "/v1/assets/upload", &[("accept", "application/json")], Some(form)).await?;
        Ok(json_or_text(checked(status, bytes)?))
    }

    /// Sends one request with its content type and body, and reads the reply.
    async fn try_send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<(String, Vec<u8>)>,
    ) -> std::io::Result<(StatusCode, Vec<u8>)> {
        let len = body.as_ref().map_or(0, |b| b.1.len());
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nhost: {}\r\nconnection: close\r\ncontent-length: {len}\r\n",
            self.host_port
        );
        if !self.access_token.is_empty() {
            write!(head, "authorization: Bearer {}\r\n", self.access_token).unwrap();
        }
        if let Some((kind, _)) = &body {
            write!(head, "content-type: {kind}\r\n").unwrap();
        }
        headers.iter().for_each(|(k, v)| write!(head, "{k}: {v}\r\n").unwrap());
        head.push_str("\r\n");
        let mut stream = TcpStream::connect(&self.host_port).await?;
        stream.write_all(&[head.into_bytes(), body.map(|b| b.1).unwrap_or_default()].concat()).await?;
        let bad = |_| std::io::Error::new(std::io::ErrorKind::InvalidData, "a bad HTTP response");
        let resp = read_message(&mut stream, &mut Vec::new(), |_| usize::MAX).await.map_err(bad)?;
        let status = resp.start_line.split(' ').nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| bad(None))?;
        Ok((StatusCode(status), resp.body))
    }

    /// Desktop's OAuth flow with PKCE: it registers `client_name`, opens `/oauth/authorize`, and posts the request of
    /// the page, which waits until the user approves it in the app. The client then uses the new access token.
    pub async fn oauth_pkce_login(&mut self, client_name: &str, code_verifier: &str) -> Result<String, ApiError> {
        assert!(code_verifier.len() >= 16, "PKCE code_verifier too short");
        let text = |v: &Value, key: &str| match &v[key] {
            Value::String(s) => Ok(s.clone()),
            _ => Err(ApiError(format!("The OAuth reply has no {key}: {v}"))),
        };
        let body = json!({ "client_name": client_name, "redirect_uris": [REDIRECT_URI] });
        let client_id = text(&self.call("POST", "/oauth/register", Some(&body)).await?, "client_id")?;
        let (challenge, state) = (base64_encode_url_nopad(&sha256(code_verifier.as_bytes())), random_id());
        let params = [
            ("client_id", client_id.as_str()),
            ("redirect_uri", REDIRECT_URI),
            ("response_type", "code"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ];
        self.call_raw("GET", &query_string("/oauth/authorize", &params), None).await?;
        let body = json!({
            "mode": "oauth2",
            "clientInfo": { "name": client_name, "clientID": client_id },
            "scopes": ["read", "write"],
            "redirectUri": REDIRECT_URI,
            "scope": "read write",
            "state": state,
            "codeChallenge": challenge,
            "codeChallengeMethod": "S256"
        });
        let auth = self.call("POST", "/oauth/authorize/callback", Some(&body)).await?;
        if text(&auth, "state")? != state {
            return Err(ApiError(format!("The OAuth reply has an other state: {auth}")));
        }
        let body = json!({
            "grant_type": "authorization_code",
            "client_id": client_id,
            "code": text(&auth, "code")?,
            "redirect_uri": REDIRECT_URI,
            "code_verifier": code_verifier
        });
        self.access_token = text(&self.call("POST", "/oauth/token", Some(&body)).await?, "access_token")?;
        Ok(self.access_token.clone())
    }

    /// Opens a WebSocket at `path` with the bearer token, or gives the reason when the server refuses it.
    pub async fn ws_open(&self, path: &str) -> Result<WsConn, String> {
        let failed = |_: std::io::Error| "WebSocket connection failed".to_string();
        let mut stream = TcpStream::connect(&self.host_port).await.map_err(failed)?;
        let mut upgrade = format!(
            "GET {path} HTTP/1.1\r\nhost: {}\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\n",
            self.host_port
        );
        if !self.access_token.is_empty() {
            write!(upgrade, "authorization: Bearer {}\r\n", self.access_token).unwrap();
        }
        stream.write_all(format!("{upgrade}\r\n").as_bytes()).await.map_err(failed)?;
        let mut leftover = Vec::new();
        let resp = read_message(&mut stream, &mut leftover, |_| usize::MAX)
            .await
            .map_err(|_| "WebSocket connection failed".to_string())?;
        let status = resp.start_line.split(' ').nth(1).unwrap_or_default();
        if status != "101" {
            return Err(format!("Unexpected server response: {status}"));
        }
        let (read, write) = stream.into_split();
        Ok(WsConn { read: std::io::Cursor::new(leftover).chain(read), write })
    }
}

/// A WebSocket connection of the client.
pub struct WsConn {
    read: tokio::io::Chain<std::io::Cursor<Vec<u8>>, tokio::net::tcp::OwnedReadHalf>,
    write: tokio::net::tcp::OwnedWriteHalf,
}

impl WsConn {
    /// Sends a text frame.
    pub async fn send(&mut self, text: &str) -> std::io::Result<()> {
        write_ws_frame(&mut self.write, 0x1, text.as_bytes(), true).await
    }

    /// The next text frame, or the close code and reason (1006 when the connection ends without a close frame).
    /// It answers a ping.
    pub async fn next(&mut self) -> Result<String, (u16, String)> {
        loop {
            match read_ws_frame(&mut self.read, usize::MAX).await {
                Ok((0x1, text)) => return Ok(String::from_utf8_lossy(&text).into_owned()),
                Ok((0x8, close)) => {
                    let code = close.get(..2).map_or(1005, |c| u16::from_be_bytes([c[0], c[1]]));
                    return Err((code, String::from_utf8_lossy(close.get(2..).unwrap_or_default()).into_owned()));
                }
                Ok((0x9, ping)) => drop(write_ws_frame(&mut self.write, 0xA, &ping, true).await),
                Ok(_) => {}
                Err(_) => return Err((1006, String::new())),
            }
        }
    }

    /// Sends a close frame with `code`.
    pub async fn close(&mut self, code: u16) -> std::io::Result<()> {
        write_ws_frame(&mut self.write, 0x8, &code.to_be_bytes(), true).await
    }
}
