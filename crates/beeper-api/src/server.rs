use crate::codec::{
    base64_decode, base64_encode_standard, base64_encode_url_nopad, sha1, sha256, url_decode, url_encode,
};
use crate::http::{parse_query, read_message, read_ws_frame, write_ws_frame, HttpMessage, StatusCode};
use beeper_core::cloud::{Cloud, CloudError, CodeResult};
use beeper_core::config::{
    network_spec, now_rfc3339_millis, APP_BUNDLE_ID, APP_NAME, APP_VERSION, NETWORK_SPECS, OPENAPI_SPEC_JSON,
};
use beeper_core::matrix::Outgoing;
use beeper_core::models::*;
use beeper_core::platform::{path_to_file_url, OsKind};
use beeper_core::store::{BeeperStore, InvertedIndex, OAuthClientEntry, OAuthCodeEntry, SharedStore, StoredAsset};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Response {
    status: StatusCode,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

type Reply = Result<Response, Response>;

fn json_with<T: Serialize + ?Sized>(status: StatusCode, value: &T) -> Response {
    let body = serde_json::to_vec(value).expect("serialize JSON response");
    Response { status, headers: vec![("content-type", "application/json".into())], body }
}

fn ok<T: Serialize + ?Sized>(value: &T) -> Reply {
    Ok(json_with(StatusCode::OK, value))
}

fn empty(status: StatusCode, headers: Vec<(&'static str, String)>) -> Reply {
    Ok(Response { status, headers, body: Vec::new() })
}

fn error(status: StatusCode, error: &str) -> Response {
    json_with(status, &json!({ "error": error }))
}

fn bad(e: &str) -> Response {
    error(StatusCode::BAD_REQUEST, e)
}

fn missing(e: &str) -> Response {
    error(StatusCode::NOT_FOUND, e)
}

fn store_error(e: &'static str) -> Response {
    error(if e == "existing_draft_not_empty" { StatusCode::CONFLICT } else { StatusCode::NOT_FOUND }, e)
}

fn page<T: Serialize>(items: Vec<T>, oldest_cursor: Option<String>, newest_cursor: Option<String>) -> Reply {
    ok(&CursorPage { items, has_more: false, oldest_cursor, newest_cursor })
}

struct Req {
    query: HashMap<String, String>,
    msg: HttpMessage,
    body: Value,
}

impl Req {
    fn q(&self, k: &str) -> Option<&str> {
        self.query.get(k).map(String::as_str)
    }

    fn need_q(&self, k: &str) -> Result<&str, Response> {
        self.q(k).filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(&format!("{k} is required")))
    }

    fn str(&self, k: &str) -> Option<&str> {
        self.body[k].as_str()
    }

    fn owned(&self, k: &str) -> Option<String> {
        self.str(k).map(str::to_string)
    }

    fn need(&self, k: &str) -> Result<&str, Response> {
        self.str(k).filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(&format!("{k} is required")))
    }

    fn parse<T: DeserializeOwned>(&self, k: &str) -> Result<T, Response> {
        serde_json::from_value(self.body[k].clone()).map_err(|e| bad(&format!("{k}: {e}")))
    }

    fn limit(&self, default: usize) -> Result<usize, Response> {
        self.q("limit").map_or(Ok(default), |v| v.parse().map_err(|_| bad("limit must be an integer")))
    }
}

fn parse_body(msg: &HttpMessage) -> Option<Value> {
    Some(match msg.header("content-type") {
        _ if msg.body.is_empty() => Value::Null,
        Some(ct) if ct.starts_with("application/json") => serde_json::from_slice(&msg.body).ok()?,
        Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
            json!(parse_query(std::str::from_utf8(&msg.body).ok()?)?)
        }
        _ => Value::Null,
    })
}

pub async fn serve_http(listener: TcpListener, store: SharedStore) {
    let cloud = {
        let cfg = &store.read().unwrap().config;
        Arc::new(Cloud::new(&cfg.api_url, &cfg.homeserver))
    };
    loop {
        let (stream, _) = listener.accept().await.expect("accept TCP connection");
        tokio::spawn(handle_connection(stream, store.clone(), cloud.clone()));
    }
}

async fn handle_connection(mut stream: TcpStream, store: SharedStore, cloud: Arc<Cloud>) {
    let mut buf = Vec::new();
    while let Some(msg) = read_message(&mut stream, &mut buf).await {
        if msg.start_line.starts_with("GET /v1/ws ") && msg.header_is("upgrade", "websocket") {
            return serve_ws(stream, buf, msg, store).await;
        }
        let close = msg.header_is("connection", "close");
        let (store, cloud) = (store.clone(), cloud.clone());
        let resp = tokio::task::spawn_blocking(move || respond(&store, &cloud, msg).unwrap_or_else(|e| e))
            .await
            .expect("request handler panicked");
        let mut head = format!("HTTP/1.1 {} \r\ncontent-length: {}\r\n", resp.status.0, resp.body.len());
        resp.headers.iter().for_each(|(k, v)| write!(head, "{k}: {v}\r\n").unwrap());
        head.push_str("\r\n");
        if stream.write_all(&[head.into_bytes(), resp.body].concat()).await.is_err() || close {
            return;
        }
    }
}

fn respond(store: &SharedStore, cloud: &Cloud, msg: HttpMessage) -> Reply {
    let mut parts = msg.start_line.split(' ');
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else { return Err(bad("malformed request line")) };
    let method = method.to_string();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let segs: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(url_decode)
        .collect::<Option<_>>()
        .ok_or_else(|| bad("bad path"))?;
    let query = parse_query(query).ok_or_else(|| bad("malformed query string"))?;
    let body = parse_body(&msg).ok_or_else(|| bad("malformed request body"))?;
    let req = Req { query, msg, body };
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    if let ("POST", ["v1", "app", "setup", step @ ("start" | "email" | "response" | "register")]) =
        (method.as_str(), &segs[..])
    {
        return sign_in_step(store, cloud, step, &req);
    }
    if let ("POST", ["v1", "app", "setup", "verifications", rest @ ..]) = (method.as_str(), &segs[..]) {
        if store.read().unwrap().matrix.is_some() {
            return matrix_verification(store, rest);
        }
    }
    let mut st = store.write().unwrap();
    let reply = dispatch(&mut st, &method, &segs, &req);
    for (chat, out) in std::mem::take(&mut st.outbox) {
        let s = store.clone();
        tokio::spawn(async move {
            if let Err(e) = beeper_core::matrix::push(s.clone(), chat, out).await {
                s.write().unwrap().matrix_error = Some(e);
            }
        });
    }
    reply
}

fn matrix_verification(store: &SharedStore, rest: &[&str]) -> Reply {
    let mut st = store.write().unwrap();
    match rest {
        [] => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            st.sas_confirm = Some(tx);
            let s = store.clone();
            tokio::runtime::Handle::current().spawn(async move {
                if let Err(e) = beeper_core::matrix::verify(s.clone(), rx).await {
                    s.write().unwrap().matrix_error = Some(e);
                }
            });
        }
        [_, "sas", "confirm"] | [_, "cancel"] => {
            let tx = st.sas_confirm.take().ok_or_else(|| missing("verification_not_found"))?;
            tx.send(rest[1] == "sas").ok();
        }
        [_, "accept"] | [_, "sas", "start"] => {}
        _ => return Err(missing("not_found")),
    }
    ok(&json!({ "session": st.setup_status.session, "verification": st.setup_status.session.verification }))
}

fn dispatch(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    match (method, path) {
        ("GET", ["v1", "info"]) => ok(&info(st)),
        ("GET", ["v1", "openapi.json"]) => Ok(Response {
            status: StatusCode::OK,
            headers: vec![("content-type", "application/json".into())],
            body: OPENAPI_SPEC_JSON.as_bytes().to_vec(),
        }),
        ("POST", ["oauth", "register"]) => oauth_register(st, r),
        ("GET", ["oauth", "authorize"]) => oauth_authorize(st, r),
        ("POST", ["oauth", "token"]) => oauth_token(st, r),
        ("GET", ["v1", "app", "setup"]) => ok(&st.setup_status),
        ("GET", ["v1", "app", "setup", "verifications"]) => {
            ok(&json!({ "session": st.setup_status.session, "verification": st.setup_status.session.verification }))
        }
        ("GET", ["v1", "app", "setup", "verifications", id]) => {
            let v = st.verifications.get(*id).ok_or_else(|| missing("verification_not_found"))?;
            ok(&json!({ "session": st.setup_status.session, "verification": v }))
        }
        ("POST", ["v1", "app", "setup", rest @ ..]) => setup(st, rest, r),
        ("GET", ["v1", "accounts"]) => ok(&st.accounts),
        ("GET", ["v1", "accounts", id, rest @ ..]) => account(st, id, rest, r),
        ("GET", ["v1", "labels"]) => ok(&st.labels),
        (_, ["v1", "bridges", rest @ ..]) => bridges(st, method, rest, r),
        (_, ["v1", "chats", rest @ ..]) => chats(st, method, rest, r),
        ("GET", ["v1", "messages", "search"]) => search_messages(st, r),
        ("POST", ["v1", "assets", "upload"]) => upload_multipart(st, r),
        ("POST", ["v1", "assets", "upload", "base64"]) => {
            let bytes = base64_decode(r.need("content")?).filter(|b| !b.is_empty());
            let bytes = bytes.ok_or_else(|| bad("content must be non-empty base64"))?;
            ok(&asset_json(st.store_uploaded_asset(r.need("fileName")?.into(), r.need("mimeType")?.into(), &bytes)))
        }
        ("POST", ["v1", "assets", "download"]) => {
            let (path, _) = st.resolve_asset_path(r.need("url")?).ok_or_else(|| missing("asset_not_found"))?;
            ok(&json!({ "srcURL": path_to_file_url(&path) }))
        }
        ("GET", ["v1", "assets", "serve"]) => serve_asset(st, r),
        ("GET", ["v1", "search"]) => unified_search(st, r),
        ("POST", ["v1", "focus"]) => focus(st, r),
        _ => Err(missing("not_found")),
    }
}

fn info(st: &BeeperStore) -> InfoResponse {
    let base_url = st.config.base_url();
    let oauth = |p: &str| format!("{base_url}/oauth/{p}");
    InfoResponse {
        app: InfoApp { bundle_id: APP_BUNDLE_ID.into(), name: APP_NAME.into(), version: APP_VERSION.into() },
        endpoints: InfoEndpoints {
            mcp: "/v1/mcp".into(),
            oauth: InfoOAuthEndpoints {
                authorization_endpoint: oauth("authorize"),
                introspection_endpoint: oauth("introspect"),
                registration_endpoint: oauth("register"),
                revocation_endpoint: oauth("revoke"),
                token_endpoint: oauth("token"),
                userinfo_endpoint: oauth("userinfo"),
            },
            spec: "/v1/openapi.json".into(),
            ws_events: "/v1/ws".into(),
        },
        platform: InfoPlatform {
            arch: std::env::consts::ARCH.into(),
            os: OsKind::current().node_platform_str().into(),
            release: "7.0.14".into(),
        },
        server: InfoServer {
            hostname: st.config.host.clone(),
            mcp_enabled: true,
            port: st.config.port,
            remote_access: false,
            status: "ok".into(),
            base_url,
        },
    }
}

fn oauth_register(st: &mut BeeperStore, r: &Req) -> Reply {
    let client_name = r.need("client_name")?.to_string();
    let redirect_uris: Vec<String> = r.parse("redirect_uris")?;
    let client_id = st.next_id("beeper_cli_client");
    let entry = OAuthClientEntry { client_id: client_id.clone(), client_name, redirect_uris };
    let resp = json!({
        "client_id": client_id,
        "client_name": entry.client_name,
        "redirect_uris": entry.redirect_uris,
        "token_endpoint_auth_method": "none"
    });
    st.oauth_clients.insert(client_id, entry);
    Ok(json_with(StatusCode::CREATED, &resp))
}

fn oauth_authorize(st: &mut BeeperStore, r: &Req) -> Reply {
    let (client_id, redirect_uri) = (r.need_q("client_id")?, r.need_q("redirect_uri")?);
    if !st.oauth_clients.get(client_id).is_some_and(|c| c.redirect_uris.iter().any(|u| u == redirect_uri)) {
        return Err(bad("invalid_client"));
    }
    if r.need_q("code_challenge_method")? != "S256" {
        return Err(bad("only S256 PKCE is supported"));
    }
    let state = r.q("state").unwrap_or_default();
    let code = st.next_id("auth_code");
    st.oauth_codes.insert(
        code.clone(),
        OAuthCodeEntry {
            code: code.clone(),
            client_id: client_id.into(),
            redirect_uri: redirect_uri.into(),
            code_challenge: r.need_q("code_challenge")?.into(),
            code_challenge_method: "S256".into(),
        },
    );
    if r.msg.header("accept").is_some_and(|a| a.contains("application/json")) {
        return ok(&json!({ "code": code, "state": state, "redirect_uri": redirect_uri }));
    }
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    empty(StatusCode::FOUND, vec![("location", format!("{redirect_uri}{sep}code={code}&state={}", url_encode(state)))])
}

fn oauth_token(st: &mut BeeperStore, r: &Req) -> Reply {
    let entry = st.oauth_codes.remove(r.need("code")?).ok_or_else(|| bad("invalid_grant"))?;
    let challenge = base64_encode_url_nopad(&sha256(r.need("code_verifier")?.as_bytes()));
    if entry.client_id != r.need("client_id")? || challenge != entry.code_challenge {
        return Err(bad("invalid_grant"));
    }
    let token = st.next_id("bp_oauth_tok");
    st.valid_tokens.insert(token.clone());
    ok(&json!({ "access_token": token, "token_type": "Bearer", "scope": "read write" }))
}

fn refresh_setup(st: &mut BeeperStore) {
    st.persist_state_to_sqlite();
    st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state);
}

fn verified(st: &mut BeeperStore, v: Option<VerificationState>) -> Reply {
    st.beeper_state.verified = true;
    refresh_setup(st);
    st.setup_status.session.verification = v.clone();
    ok(&json!({ "session": st.setup_status.session, "verification": v }))
}

fn new_verification(id: String, methods: &[&str], state: &str, qr: String) -> VerificationState {
    VerificationState {
        verification_id: id,
        direction: "outgoing".into(),
        methods: methods.iter().map(|m| m.to_string()).collect(),
        state: state.into(),
        updated_at: now_rfc3339_millis(),
        sas: None,
        qr: Some(VerificationQr { data: Some(qr) }),
        other_device: None,
    }
}

fn sign_in_step(store: &SharedStore, cloud: &Cloud, step: &str, r: &Req) -> Reply {
    let already = || error(StatusCode::CONFLICT, "already_logged_in");
    if store.read().unwrap().signed_in() {
        return Err(already());
    }
    let upstream = |e: CloudError| match e.status {
        401 => error(StatusCode::UNAUTHORIZED, "invalid_sign_in_code"),
        _ => error(StatusCode::BAD_GATEWAY, &e.message),
    };
    let jwt = match step {
        "start" => {
            let (request, methods) = cloud.start_login().map_err(upstream)?;
            if !methods.iter().any(|m| m == "email") {
                return Err(error(StatusCode::CONFLICT, "email_sign_in_not_supported"));
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
    let mut st = store.write().unwrap();
    if st.signed_in() {
        return Err(already());
    }
    st.sign_in(session, cloud.homeserver.clone(), &profile, accounts);
    let s = &st.beeper_state;
    let matrix = json!({ "userID": s.user_id, "deviceID": s.device_id, "accessToken": s.access_token, "homeserver": s.homeserver });
    ok(&json!({ "matrix": matrix, "session": st.setup_status.session }))
}

fn setup(st: &mut BeeperStore, path: &[&str], r: &Req) -> Reply {
    match path {
        ["verification", "recovery-key"] | ["verification", "recovery-key", "reset", "confirm"] => {
            st.beeper_state.secrets.recovery_code = r.need("recoveryKey")?.into();
            st.beeper_state.has_backed_up_code = true;
            verified(st, None)
        }
        ["verification", "recovery-key", "reset"] => {
            let key = format!("EsTc Beeper Reset {}", st.next_id("Key"));
            st.beeper_state.secrets.recovery_code = key.clone();
            ok(&json!({ "recoveryKey": key, "session": st.setup_status.session }))
        }
        ["verifications"] => {
            let id = st.next_id("verif");
            let v = VerificationState {
                other_device: Some(VerificationOtherDevice {
                    device_id: "IPHONE_BEEPER_01".into(),
                    user_id: st.beeper_state.user_id.clone(),
                }),
                ..new_verification(id.clone(), &["sas", "qr_code"], "ready", format!("BEEPER_QR_VERIFY:{id}"))
            };
            st.verifications.insert(id, v.clone());
            st.setup_status.session.state = "verifying".into();
            st.setup_status.session.e2ee.state = "verifying".into();
            st.setup_status.session.verification = Some(v.clone());
            ok(&json!({ "session": st.setup_status.session, "verification": v }))
        }
        ["verifications", "qr", "scan"] => {
            let v = new_verification(st.next_id("verif_qr"), &["qr_code"], "verified", r.need("data")?.into());
            st.verifications.insert(v.verification_id.clone(), v.clone());
            verified(st, Some(v))
        }
        ["verifications", id, action @ ..] => {
            let state = match action {
                ["accept"] => "ready",
                ["cancel"] => "cancelled",
                ["sas", "start"] => "sas_started",
                ["sas", "confirm"] | ["qr", "confirm-scanned"] => "verified",
                _ => return Err(missing("not_found")),
            };
            let v = st.verifications.get_mut(*id).ok_or_else(|| missing("verification_not_found"))?;
            v.state = state.into();
            v.updated_at = now_rfc3339_millis();
            if state == "sas_started" {
                let emojis = [("🐶", "Dog"), ("🚀", "Rocket"), ("🔑", "Key"), ("⚡", "Zap"), ("🍕", "Pizza")]
                    .into_iter()
                    .chain([("🎸", "Guitar"), ("🌈", "Rainbow")])
                    .map(|(e, d)| VerificationSasEmoji { emoji: e.into(), description: d.into() })
                    .collect();
                v.sas = Some(VerificationSas { decimals: Some("4821 9034 1172".into()), emojis });
            }
            let v = v.clone();
            if state == "verified" {
                return verified(st, Some(v));
            }
            st.setup_status.session.verification = (state != "cancelled").then(|| v.clone());
            ok(&json!({ "session": st.setup_status.session, "verification": v }))
        }
        _ => Err(missing("not_found")),
    }
}

fn account(st: &BeeperStore, id: &str, rest: &[&str], r: &Req) -> Reply {
    let account = st.accounts.iter().find(|a| a.account_id == id).ok_or_else(|| missing("account_not_found"))?;
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
        _ => Err(missing("not_found")),
    }
}

fn bridge_summary(st: &BeeperStore, spec: &NetworkSpec) -> BridgeSummary {
    let logins: Vec<_> = st.bridge_accounts.iter().filter(|b| b.bridge_id == spec.bridge_id).cloned().collect();
    BridgeSummary {
        bridge_id: spec.bridge_id.into(),
        bridge_type: spec.bridge_type.into(),
        is_running: !logins.is_empty(),
        logins,
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

fn bridges(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    let [id, rest @ ..] = path else {
        let catalog: Vec<Value> = NETWORK_SPECS
            .iter()
            .map(|s| {
                json!({
                    "bridgeID": s.bridge_id,
                    "bridgeType": s.bridge_type,
                    "displayName": s.display_name,
                    "supportsOnDevice": s.supports_on_device,
                    "supportsCloud": s.supports_cloud
                })
            })
            .collect();
        let bridges: Vec<_> = NETWORK_SPECS.iter().map(|s| bridge_summary(st, s)).collect();
        return ok(&json!({ "bridges": bridges, "catalog": catalog }));
    };
    let spec = network_spec(id).ok_or_else(|| missing("bridge_not_found"))?;
    let name = spec.display_name;
    let session_exists = |st: &BeeperStore, sid: &str| st.bridge_sessions.get(sid).is_some_and(|s| s.bridge_id == *id);
    match (method, rest) {
        ("GET", []) => ok(&bridge_summary(st, spec)),
        ("GET", ["capabilities"]) => ok(&json!({
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
        ("GET", ["login-flows"]) => {
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
            ok(&json!({ "bridgeID": spec.bridge_id, "bridgeType": spec.bridge_type, "flows": flows }))
        }
        ("GET", ["logins"]) => ok(
            &json!({ "bridgeID": spec.bridge_id, "bridgeType": spec.bridge_type, "logins": bridge_summary(st, spec).logins }),
        ),
        ("DELETE", ["logins", login_id]) => {
            let pos = st.bridge_accounts.iter().position(|b| b.bridge_id == *id && b.login_id == *login_id);
            st.bridge_accounts.remove(pos.ok_or_else(|| missing("login_not_found"))?);
            st.rebuild_accounts_from_bridges();
            st.persist_state_to_sqlite();
            ok(&json!({ "bridge": bridge_summary(st, spec) }))
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
        ("GET", ["login-sessions", sid]) if session_exists(st, sid) => ok(&st.bridge_sessions[*sid]),
        ("DELETE", ["login-sessions", sid]) if session_exists(st, sid) => {
            st.bridge_sessions.remove(*sid);
            ok(&json!({ "cancelled": true }))
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
        (_, ["login-sessions", ..]) => Err(missing("login_session_not_found")),
        _ => Err(missing("not_found")),
    }
}

fn candidates(index: &InvertedIndex, query: Option<&str>, len: usize) -> Vec<usize> {
    match query.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => index.query_all_tokens(q).into_iter().map(|i| i as usize).collect(),
        None => (0..len).collect(),
    }
}

fn chats(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    match (method, path) {
        ("GET", []) => {
            let account = r.q("accountIDs");
            let mut items: Vec<&Chat> = st.chats.iter().filter(|c| account.is_none_or(|a| c.account_id == a)).collect();
            items.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
            let (oldest, newest) = (items.last().map(|c| c.id.clone()), items.first().map(|c| c.id.clone()));
            page(items, oldest, newest)
        }
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
        ("GET", ["search"]) => search_chats(st, r),
        (_, [id, rest @ ..]) => {
            let idx = st.resolve_chat_idx(id).ok_or_else(|| missing("chat_not_found"))?;
            chat(st, method, idx, id, rest, r)
        }
        _ => Err(missing("not_found")),
    }
}

fn create_chat(st: &mut BeeperStore, r: &Req) -> Reply {
    let (chat, status) = match r.str("mode").unwrap_or("create") {
        "merge" => {
            let merge = &r.body["merge"];
            let chat_ids: Vec<String> = serde_json::from_value(merge["chatIDs"].clone())
                .ok()
                .filter(|ids: &Vec<String>| !ids.is_empty())
                .ok_or_else(|| bad("merge.chatIDs is required"))?;
            let default_chat_id = merge["defaultChatID"].as_str().map(str::to_string);
            let lead = st.resolve_chat_idx(default_chat_id.as_deref().unwrap_or(&chat_ids[0]));
            let account = st.chats[lead.ok_or_else(|| missing("chat_not_found"))?].account_id.clone();
            st.create_or_start_chat(
                &account,
                "merged",
                r.owned("title"),
                Vec::new(),
                None,
                Some(chat_ids),
                default_chat_id,
            )
        }
        "create" => {
            let participants: Option<Vec<String>> = r.parse("participantIDs")?;
            let account_id = r.need("accountID")?;
            let chat_type = r.str("type").unwrap_or("single");
            let (title, text) = (r.owned("title"), r.owned("messageText"));
            st.create_or_start_chat(account_id, chat_type, title, participants.unwrap_or_default(), text, None, None)
        }
        _ => return Err(bad("mode must be create or merge")),
    };
    ok(&json!({ "chatID": chat.id, "status": status }))
}

fn search_chats(st: &BeeperStore, r: &Req) -> Reply {
    let (mut mask, mut want) = match r.q("inbox") {
        None => (0, 0),
        Some("archive") => (CHAT_ARCHIVED, CHAT_ARCHIVED),
        Some("low-priority") => (CHAT_LOW_PRIORITY, CHAT_LOW_PRIORITY),
        Some("primary") => (CHAT_ARCHIVED | CHAT_LOW_PRIORITY, 0),
        Some(_) => return Err(bad("inbox must be primary, low-priority, or archive")),
    };
    if r.q("unreadOnly") == Some("true") {
        (mask, want) = (mask | CHAT_UNREAD, want | CHAT_UNREAD);
    }
    let chat_type = r.q("type").filter(|&t| t != "any");
    let mut items: Vec<&Chat> = candidates(&st.chat_search_index, r.q("query"), st.chats.len())
        .into_iter()
        .map(|i| &st.chats[i])
        .filter(|c| c.flags & mask == want && chat_type.is_none_or(|t| c.chat_type == t))
        .collect();
    items.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    items.truncate(r.limit(50)?);
    let (oldest, newest) = (items.last().map(|c| c.id.clone()), items.first().map(|c| c.id.clone()));
    page(items, oldest, newest)
}

fn chat(st: &mut BeeperStore, method: &str, idx: usize, id: &str, rest: &[&str], r: &Req) -> Reply {
    let success = |st: &BeeperStore| ok(&json!({ "chatID": st.chats[idx].id, "success": true }));
    match (method, rest) {
        ("GET", []) => ok(&st.chats[idx]),
        ("PATCH", []) => {
            let draft = r.body.get("draft");
            let new_draft = draft.filter(|d| !d.is_null()).map(|_| r.parse("draft")).transpose()?;
            let new_merge = r.body.get("merge").map(|_| r.parse("merge")).transpose()?;
            ok(&st.patch_chat(id, draft.is_some_and(Value::is_null), new_draft, new_merge).map_err(store_error)?)
        }
        ("POST", ["archive"]) => {
            let archived = r.body["archived"].as_bool().unwrap_or(true);
            st.update_chat(idx, |c| c.is_archived = Some(archived));
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("POST", ["read"]) => {
            st.update_chat(idx, |c| {
                c.unread_count = 0;
                c.is_marked_unread = Some(false);
            });
            let msgs = st.chat_msg_indices.get(&st.chats[idx].id).into_iter().flatten();
            let unread: Vec<usize> = msgs.copied().filter(|&m| st.messages[m].is_unread == Some(true)).collect();
            for m in unread {
                st.update_message(m, |m| m.is_unread = Some(false));
            }
            success(st)
        }
        ("POST", ["unread"]) => {
            st.update_chat(idx, |c| {
                c.is_marked_unread = Some(true);
                c.unread_count = c.unread_count.max(1);
            });
            success(st)
        }
        ("POST", ["notify-anyway"]) => {
            st.update_chat(idx, |c| c.flags |= CHAT_NOTIFY_ANYWAY);
            success(st)
        }
        ("POST", ["reminders"]) => {
            let reminder: ChatReminder = r.parse("reminder")?;
            st.update_chat(idx, |c| c.reminder = Some(reminder));
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("DELETE", ["reminders"]) => {
            st.update_chat(idx, |c| c.reminder = None);
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("GET", ["messages"]) => list_messages(st, idx, r),
        ("POST", ["messages"]) => send_message(st, id, r),
        ("GET", ["messages", mid]) => {
            ok(&st.messages[*st.msg_idx_by_id.get(*mid).ok_or_else(|| missing("message_not_found"))?])
        }
        ("PUT", ["messages", mid]) => {
            let text = r.need("text")?;
            let (cid, msg) = st.edit_message(id, mid, text.into()).map_err(store_error)?;
            st.queue(&cid, Outgoing::Edit { target: msg.id.clone(), text: text.into() });
            ok(&json!({ "chatID": cid, "messageID": msg.id, "success": true }))
        }
        ("DELETE", ["messages", mid]) => {
            let cid = st.delete_message(id, mid).map_err(store_error)?;
            st.queue(&cid, Outgoing::Redact { target: (*mid).into() });
            ok(&json!({ "chatID": cid, "messageID": mid, "success": true }))
        }
        ("POST", ["messages", mid, "reactions"]) => {
            let key = r.need("reactionKey")?;
            let (cid, tx) = st.add_reaction(id, mid, key, r.owned("transactionID")).map_err(store_error)?;
            st.queue(&cid, Outgoing::React { txn: tx.clone(), target: (*mid).into(), key: key.into() });
            ok(&json!({ "chatID": cid, "messageID": mid, "reactionKey": key, "transactionID": tx, "success": true }))
        }
        ("DELETE", ["messages", mid, "reactions", key]) => {
            let mine = st.messages[st.msg_idx_by_id[*mid]]
                .reactions
                .iter()
                .find(|x| x.reaction_key == *key && st.is_me(&x.participant_id));
            let target = mine.ok_or_else(|| missing("reaction_not_found"))?.id.clone();
            let cid = st.remove_reaction(id, mid, key).map_err(store_error)?;
            st.queue(&cid, Outgoing::Redact { target });
            ok(&json!({ "chatID": cid, "messageID": mid, "reactionKey": key, "success": true }))
        }
        _ => Err(missing("not_found")),
    }
}

fn list_messages(st: &BeeperStore, idx: usize, r: &Req) -> Reply {
    let after = match r.q("direction") {
        None | Some("before") => false,
        Some("after") => true,
        Some(_) => return Err(bad("direction must be before or after")),
    };
    let chat = &st.chats[idx];
    let members = chat.merge.iter().flat_map(|m| &m.chat_ids);
    let member_ids = members.map(|id| &st.chats[st.resolve_chat_idx(id).expect("merged member chat exists")].id);
    let mut items: Vec<&Message> = std::iter::once(&chat.id)
        .chain(member_ids)
        .filter_map(|cid| st.chat_msg_indices.get(cid))
        .flatten()
        .map(|&m| &st.messages[m])
        .filter(|m| {
            r.q("cursor").is_none_or(|cur| if after { m.sort_key.as_str() > cur } else { m.sort_key.as_str() < cur })
        })
        .collect();
    items.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
    let (oldest, newest) = (items.first().map(|m| m.sort_key.clone()), items.last().map(|m| m.sort_key.clone()));
    page(items, oldest, newest)
}

fn send_message(st: &mut BeeperStore, id: &str, r: &Req) -> Reply {
    let attachment = match &r.body["attachment"] {
        Value::Null => None,
        a => {
            let upload_id = a["uploadID"].as_str().ok_or_else(|| bad("attachment.uploadID is required"))?;
            let stored = st.assets.get(upload_id).ok_or_else(|| missing("upload_not_found"))?.clone();
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
        st.queue(&chat_id, Outgoing::Send { txn: pending.clone(), text, reply_to });
    }
    Ok(json_with(StatusCode::CREATED, &json!({ "chatID": chat_id, "pendingMessageID": pending })))
}

fn search_messages(st: &BeeperStore, r: &Req) -> Reply {
    let media: fn(&Message) -> bool = match r.q("mediaTypes") {
        None => |_| true,
        Some("any") => |m| !m.attachments.is_empty(),
        Some("image") => |m| m.attachments.iter().any(|a| a.attachment_type == "img"),
        Some("video") => |m| m.attachments.iter().any(|a| a.attachment_type == "video"),
        Some("audio") => |m| m.attachments.iter().any(|a| a.attachment_type == "audio"),
        Some("file") => |m| m.attachments.iter().any(|a| a.attachment_type == "unknown"),
        Some("link") => |m| m.text.as_deref().is_some_and(|t| t.contains("http://") || t.contains("https://")),
        Some(_) => return Err(bad("unknown mediaTypes")),
    };
    let (sender, chat_id) = (r.q("sender"), r.q("chatIDs"));
    let mut items: Vec<&Message> = candidates(&st.msg_search_index, r.q("query"), st.messages.len())
        .into_iter()
        .map(|i| &st.messages[i])
        .filter(|m| {
            m.flags & MSG_DELETED == 0
                && chat_id.is_none_or(|c| m.chat_id == c)
                && match sender {
                    None => true,
                    Some("me") => m.is_sender == Some(true),
                    Some("others") => m.is_sender != Some(true),
                    Some(s) => m.sender_id == s,
                }
                && media(m)
        })
        .collect();
    items.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
    items.truncate(r.limit(20)?);
    let (oldest, newest) = (items.last().map(|m| m.sort_key.clone()), items.first().map(|m| m.sort_key.clone()));
    page(items, oldest, newest)
}

fn unified_search(st: &BeeperStore, r: &Req) -> Reply {
    let q = r.need_q("query")?;
    let (in_groups, chats): (Vec<&Chat>, Vec<&Chat>) = st
        .chat_search_index
        .query_all_tokens(q)
        .into_iter()
        .map(|i| &st.chats[i as usize])
        .partition(|c| c.chat_type == "group");
    let messages: Vec<&Message> =
        st.msg_search_index.query_all_tokens(q).into_iter().map(|i| &st.messages[i as usize]).collect();
    let referenced: HashMap<&str, &Chat> =
        messages.iter().map(|m| (m.chat_id.as_str(), &st.chats[st.chat_idx_by_id[&m.chat_id]])).collect();
    ok(&json!({
        "results": {
            "chats": chats,
            "in_groups": in_groups,
            "messages": {
                "chats": referenced,
                "hasMore": false,
                "items": messages,
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

fn serve_asset(st: &BeeperStore, r: &Req) -> Reply {
    let (path, mime) = st.resolve_asset_path(r.need_q("url")?).ok_or_else(|| missing("asset_not_found"))?;
    let bytes = std::fs::read(&path).expect("read asset file");
    let mut headers = vec![("content-type", mime), ("accept-ranges", "bytes".into())];
    let Some(range) = r.msg.header("range") else {
        return Ok(Response { status: StatusCode::OK, headers, body: bytes });
    };
    let total = bytes.len();
    let (start, end) =
        parse_range(range, total).ok_or_else(|| error(StatusCode::RANGE_NOT_SATISFIABLE, "bad_range"))?;
    headers.push(("content-range", format!("bytes {start}-{end}/{total}")));
    Ok(Response { status: StatusCode::PARTIAL_CONTENT, headers, body: bytes[start..=end].to_vec() })
}

fn focus(st: &mut BeeperStore, r: &Req) -> Reply {
    let idx = match (r.str("chatID"), r.str("messageID")) {
        (Some(id), _) => Some(st.resolve_chat_idx(id).ok_or_else(|| missing("chat_not_found"))?),
        (None, Some(mid)) => {
            let &m_idx = st.msg_idx_by_id.get(mid).ok_or_else(|| missing("message_not_found"))?;
            Some(st.chat_idx_by_id[&st.messages[m_idx].chat_id])
        }
        (None, None) => None,
    };
    let (text, path) = (r.owned("draftText"), r.owned("draftAttachmentPath"));
    if text.is_some() || path.is_some() {
        let idx = idx.ok_or_else(|| bad("draft needs chatID"))?;
        let attachments =
            path.into_iter().map(|p| ChatDraftAttachment { upload_id: p.clone(), file_name: Some(p), mime_type: None });
        let draft = ChatDraft { text, attachments: attachments.collect(), reply_to_message_id: None };
        st.update_chat(idx, |c| c.draft = Some(draft));
    }
    st.focus_tx.send(idx.map(|i| st.chats[i].id.clone())).ok();
    ok(&json!({ "success": true }))
}

async fn serve_ws(mut stream: TcpStream, leftover: Vec<u8>, msg: HttpMessage, store: SharedStore) {
    let Some(key) = msg.header("sec-websocket-key") else { return };
    let accept = base64_encode_standard(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
    );
    if stream.write_all(handshake.as_bytes()).await.is_err() {
        return;
    }
    let mut events = store.read().unwrap().ws_tx.subscribe();
    let (reader, mut writer) = stream.into_split();
    let subs = Arc::new(RwLock::new(HashSet::from(["*".to_string()])));
    let reader_subs = subs.clone();
    let reader_task = tokio::spawn(async move {
        let mut reader = std::io::Cursor::new(leftover).chain(reader);
        while let Some((opcode, payload)) = read_ws_frame(&mut reader).await {
            match (opcode, serde_json::from_slice::<Value>(&payload)) {
                (0x1, Ok(v)) if v["type"] == "subscriptions.set" && v["chatIDs"].is_array() => {
                    let ids = v["chatIDs"].as_array().unwrap().iter().filter_map(Value::as_str).map(String::from);
                    *reader_subs.write().unwrap() = ids.collect();
                }
                (0x9 | 0xA, _) => {}
                _ => break,
            }
        }
    });
    while let Ok(ev) = events.recv().await {
        let wanted = {
            let s = subs.read().unwrap();
            s.contains("*") || ev.chat_id.as_ref().is_some_and(|c| s.contains(c))
        };
        if wanted && write_ws_frame(&mut writer, &serde_json::to_vec(&ev).unwrap(), false).await.is_err() {
            break;
        }
    }
    reader_task.abort();
}
