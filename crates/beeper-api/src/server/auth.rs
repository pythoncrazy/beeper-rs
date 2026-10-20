//! Desktop's Beeper Connect auth on the HTTP server: the access token and scopes of a request, OAuth 2.1 with PKCE
//! and the approval of the user, and the OAuth discovery documents.

use super::{json_with, ok, Reply, Req, Response};
use crate::codec::base64_encode_standard;
use crate::http::{HttpMessage, StatusCode};
use beeper_core::config::{APP_BUNDLE_ID, APP_NAME, APP_VERSION};
use beeper_core::connect::{self, ClientInfo, Outcome, Token, READ, WRITE};
use beeper_core::store::{BeeperStore, SharedStore};
use serde_json::{json, Value};
use url::{Host, Url};

const PAGE: &str = include_str!("../../assets/page.html");
const AUTHORIZE: &str = include_str!("../../assets/authorize.html");
const SCOPES: [&str; 2] = [READ, WRITE];

/// Desktop's `{message, code}` error body.
pub(super) fn api_error(status: u16, message: &str, code: &str) -> Response {
    super::error(StatusCode(status), message, code)
}

/// Desktop's `extractAccessToken`: a bearer token, or the `x-beeper-access-token` header.
pub(super) fn access_token(msg: &HttpMessage) -> Option<&str> {
    match msg.header("authorization") {
        Some(a) => a.strip_prefix("Bearer "),
        None => msg.header("x-beeper-access-token"),
    }
}

/// The scope that a route needs, or `None` for the routes that are open to all.
pub(super) fn scope(method: &str, segs: &[&str]) -> Option<&'static str> {
    match (method, segs) {
        (_, [] | ["v1", "info" | "spec"] | [".well-known", ..] | ["register"] | ["v1", "app", "setup", ..]) => None,
        ("GET", ["oauth", "userinfo"]) => Some(READ),
        (_, ["oauth", ..]) => None,
        ("GET", _) | ("POST", ["v1", "focus"] | ["v1", "assets", "download"]) => Some(READ),
        _ => Some(WRITE),
    }
}

/// Desktop's `validateAndAuthorize`: the stored token of a request that has all `scopes`, or the status and
/// message of the refusal.
pub(super) fn authorize(st: &BeeperStore, token: Option<&str>, scopes: &[&str]) -> Result<Token, (u16, String)> {
    let token = token.filter(|t| !t.is_empty()).ok_or((401, "Unauthorized: Invalid or missing token".into()))?;
    let session = &st.beeper_state;
    let found = st.connect.validate(token, &session.access_token, &session.user_id).map_err(|m| (401, m.into()))?;
    let missing: Vec<&str> = scopes.iter().copied().filter(|s| !found.scopes.iter().any(|h| h == s)).collect();
    if !missing.is_empty() {
        let message = format!(
            "Insufficient permissions. Required scopes: {}, missing: {}",
            scopes.join(", "),
            missing.join(", ")
        );
        return Err((403, message));
    }
    Ok(found)
}

pub(super) fn refusal((status, message): (u16, String)) -> Response {
    api_error(status, &message, if status == 401 { "unauthorized" } else { "forbidden" })
}

/// The MCP refusal, which tells the client where to get a token.
pub(super) fn mcp_refusal(st: &BeeperStore, msg: &HttpMessage) -> Response {
    let base = st.config.base_url();
    let id = serde_json::from_slice::<Value>(&msg.body).ok().and_then(|b| b.get("id").cloned()).unwrap_or(Value::Null);
    let mut resp = json_with(
        StatusCode(401),
        &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": "Unauthorized" } }),
    );
    let challenge = format!(
        "Bearer realm=\"{base}/v0/mcp\", resource_metadata=\"{base}/.well-known/oauth-protected-resource/v0/mcp\""
    );
    resp.headers.push(("www-authenticate", challenge));
    resp
}

fn loopback(u: &Url) -> bool {
    match u.host() {
        Some(Host::Domain(d)) => d == "localhost",
        Some(Host::Ipv4(ip)) => ip.octets()[0] == 127,
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Desktop's `isSameOriginRequest`: no `Origin`, the origin of the server, or a loopback origin on its port.
fn same_origin(st: &BeeperStore, msg: &HttpMessage) -> Result<(), Response> {
    let Some(origin) = msg.header("origin") else { return Ok(()) };
    let base = Url::parse(&st.config.base_url()).expect("parse the base URL");
    let same = Url::parse(origin).is_ok_and(|o| {
        o.origin() == base.origin()
            || (o.scheme() == base.scheme() && o.port() == base.port() && loopback(&o) && loopback(&base))
    });
    same.then_some(()).ok_or_else(|| api_error(403, "This endpoint must be called same-origin", "invalid_request"))
}

/// The same-origin check of the setup routes, against the `Host` of the request.
pub(super) fn setup_origin(msg: &HttpMessage) -> Result<(), Response> {
    let Some(origin) = msg.header("origin") else { return Ok(()) };
    let host = msg.header("host").map(|h| format!("http://{h}"));
    let same = Url::parse(origin).is_ok_and(|o| Some(o.origin().ascii_serialization()) == host);
    same.then_some(()).ok_or_else(|| api_error(403, "Setup requests must be same-origin", "invalid_request"))
}

/// A loopback HTTP(S) redirect, or a private-use URI scheme.
fn redirect_ok(uri: &str) -> bool {
    let Ok(u) = Url::parse(uri) else { return false };
    match u.scheme() {
        "about" | "blob" | "data" | "file" | "javascript" => false,
        "http" | "https" => loopback(&u),
        s => !s.starts_with("web+"),
    }
}

/// A registered redirect matches the same URI, or the same loopback URI on another port.
fn redirect_matches(registered: &str, uri: &str) -> bool {
    registered == uri
        || Url::parse(registered).ok().zip(Url::parse(uri).ok()).is_some_and(|(r, u)| {
            r.scheme() == u.scheme()
                && r.path() == u.path()
                && r.query() == u.query()
                && loopback(&r)
                && r.host() == u.host()
        })
}

struct Valid {
    client_id: String,
    client_name: String,
    client_uri: Option<String>,
    scopes: Vec<String>,
}

/// Desktop's `validateOAuthAuthorizationRequest`.
fn validate(
    st: &BeeperStore,
    client_id: &str,
    redirect: &str,
    scope: &str,
    response_type: &str,
    challenge: &str,
    method: &str,
) -> Result<Valid, &'static str> {
    if response_type != "code" {
        return Err("Invalid or missing response_type parameter. Only \"code\" is supported.");
    }
    if client_id.is_empty() {
        return Err("Missing client_id parameter.");
    }
    if redirect.is_empty() {
        return Err("Missing redirect_uri parameter.");
    }
    if !redirect_ok(redirect) {
        return Err("Invalid redirect_uri. Use a loopback HTTP/HTTPS or private-use URI scheme redirect.");
    }
    if challenge.is_empty() {
        return Err("PKCE code_challenge is required for public clients.");
    }
    if !method.is_empty() && method != "S256" {
        return Err("Only S256 code_challenge_method is supported.");
    }
    let mut scopes: Vec<String> = scope.split(' ').filter(|s| !s.is_empty()).map(String::from).collect();
    if !scopes.iter().any(|s| s == READ) {
        scopes.insert(0, READ.into());
    }
    if scopes.iter().any(|s| !SCOPES.contains(&s.as_str())) {
        return Err("Requested scope is not supported.");
    }
    let Some(c) = st.connect.clients.get(client_id) else {
        let client_name =
            if redirect.starts_with("cursor://") { format!("Cursor ({client_id})") } else { client_id.into() };
        return Ok(Valid { client_id: client_id.into(), client_name, client_uri: None, scopes });
    };
    if !c.redirect_uris.iter().any(|r| redirect_matches(r, redirect)) {
        return Err("Invalid redirect_uri. Must match one of the registered redirect URIs.");
    }
    if scopes.iter().any(|s| !c.scope.split(' ').any(|r| r == s)) {
        return Err("Requested scope exceeds registered scope for this client.");
    }
    Ok(Valid {
        client_id: client_id.into(),
        client_name: c.client_name.clone(),
        client_uri: c.client_uri.clone(),
        scopes,
    })
}

/// Fills the `{{key}}` holes of a template in one pass, so that a value is never filled again.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let end = start + rest[start..].find("}}").expect("a closed template hole");
        let key = &rest[start + 2..end];
        let value = values.iter().find(|(k, _)| *k == key).unwrap_or_else(|| panic!("no value for {{{{{key}}}}}")).1;
        out.push_str(&rest[..start]);
        out.push_str(value);
        rest = &rest[end + 2..];
    }
    out + rest
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// A JavaScript literal that is safe in a `<script>`.
fn js(v: Option<&str>) -> String {
    serde_json::to_string(&v)
        .unwrap()
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn html(status: u16, title: &str, body: &str, mut headers: Vec<(&'static str, String)>) -> Reply {
    headers.push(("content-type", "text/html; charset=utf-8".into()));
    let page = fill(PAGE, &[("title", &escape_html(title)), ("body", body)]);
    Ok(Response { status: StatusCode(status), headers, body: page.into_bytes() })
}

/// Drops the `null` members, like `JSON.stringify` drops `undefined`.
fn compact(mut v: Value) -> Value {
    v.as_object_mut().expect("a JSON object").retain(|_, m| !m.is_null());
    v
}

fn secs(ms: i64) -> i64 {
    ms / 1000
}

pub(super) fn route(store: &SharedStore, method: &str, segs: &[&str], r: &Req, token: Option<&Token>) -> Option<Reply> {
    let reply = match (method, segs) {
        ("GET", [".well-known", "oauth-protected-resource", rest @ ..]) => {
            let suffix = format!("/{}", rest.join("/"));
            if !["/", "/v1", "/v0/mcp"].contains(&suffix.as_str()) {
                return Some(Ok(json_with(StatusCode::NOT_FOUND, &json!({ "error": "Not Found" }))));
            }
            let base = store.read().config.base_url();
            let resource = if suffix == "/" { "/v1" } else { &suffix };
            ok(&json!({
                "resource": format!("{base}{resource}"),
                "authorization_servers": [base],
                "bearer_methods_supported": ["header"],
                "scopes_supported": SCOPES
            }))
        }
        ("GET", [".well-known", "oauth-authorization-server"]) => {
            let base = store.read().config.base_url();
            let mut resp = json_with(
                StatusCode::OK,
                &json!({
                    "issuer": base,
                    "authorization_endpoint": format!("{base}/oauth/authorize"),
                    "token_endpoint": format!("{base}/oauth/token"),
                    "revocation_endpoint": format!("{base}/oauth/revoke"),
                    "userinfo_endpoint": format!("{base}/oauth/userinfo"),
                    "registration_endpoint": format!("{base}/oauth/register"),
                    "grant_types_supported": ["authorization_code"],
                    "token_endpoint_auth_methods_supported": ["none"],
                    "response_types_supported": ["code"],
                    "scopes_supported": SCOPES,
                    "code_challenge_methods_supported": ["S256"],
                    "service_documentation": format!("{base}/v1/spec")
                }),
            );
            resp.headers.push(("cache-control", "no-cache".into()));
            Ok(resp)
        }
        ("GET", ["oauth", "authorize"]) => authorize_page(&store.read(), r),
        ("GET", ["oauth", "userinfo"]) => {
            let t = token.expect("userinfo needs a token");
            let client = t.client_info.as_ref().and_then(|c| c.client_id.clone());
            ok(&compact(json!({
                "sub": t.id,
                "aud": client,
                "scope": t.scopes.join(" "),
                "exp": t.expires_at.map(secs),
                "iat": secs(t.created_at),
                "client_id": client,
                "token_use": "access"
            })))
        }
        ("POST", ["oauth", rest @ ..] | rest @ ["register"]) => {
            if let Err(e) = same_origin(&store.read(), &r.msg) {
                return Some(Err(e));
            }
            match rest {
                ["authorize", "callback"] => callback(store, r),
                ["token"] => exchange(&mut store.write(), r),
                ["revoke"] => {
                    if let Some(t) = r.str("token") {
                        let mut st = store.write();
                        st.connect.revoke_access(t);
                        st.persist_connect();
                    }
                    ok(&json!({}))
                }
                ["introspect"] => introspect(&store.read(), r),
                ["register"] => register(&mut store.write(), r),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(reply)
}

fn authorize_page(st: &BeeperStore, r: &Req) -> Reply {
    let q = |k| r.q(k).unwrap_or_default();
    let or = |k, default| Some(q(k)).filter(|v| !v.is_empty()).unwrap_or(default);
    let (client_id, redirect, challenge) = (q("client_id"), q("redirect_uri"), q("code_challenge"));
    let (scope, method) = (or("scope", "read write"), or("code_challenge_method", "S256"));
    let v = match validate(st, client_id, redirect, scope, q("response_type"), challenge, method) {
        Ok(v) => v,
        Err(m) => return html(400, "Invalid Request", &format!("<h1>Invalid Request</h1><p>{m}</p>"), Vec::new()),
    };
    let nonce = base64_encode_standard(&connect::random::<16>());
    let nonempty = |k| Some(q(k)).filter(|v| !v.is_empty());
    let body = fill(
        AUTHORIZE,
        &[
            ("nonce", &nonce),
            ("redirect_uri", &js(Some(redirect))),
            ("state", &js(nonempty("state"))),
            ("client_name", &js(Some(&v.client_name))),
            ("scope", &js(Some(scope))),
            ("code_challenge", &js(Some(challenge))),
            ("code_challenge_method", &js(Some(method))),
            ("resource", &js(nonempty("resource"))),
            ("client_uri", &js(v.client_uri.as_deref())),
            ("client_id", &js(Some(client_id))),
        ],
    );
    let csp = format!("default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline'; connect-src 'self' http://localhost:* http://127.0.0.1:* http://[::1]:* https://localhost:* https://127.0.0.1:* https://[::1]:*; frame-ancestors 'none'; base-uri 'none'; form-action 'none'");
    html(
        200,
        &format!("Beeper Client API Authorization - {}", v.client_name),
        &body,
        vec![("content-security-policy", csp)],
    )
}

/// The page asks for a token, which waits for the "Approve connection" dialog, and gets an authorization code.
fn callback(store: &SharedStore, r: &Req) -> Reply {
    let (b, state) = (&r.body, r.body["state"].clone());
    let s = |k: &str| b[k].as_str().unwrap_or_default();
    let info = &b["clientInfo"];
    let (redirect, challenge) = (s("redirectUri"), s("codeChallenge"));
    let scope = Some(s("scope")).filter(|v| !v.is_empty()).unwrap_or(READ);
    let client_id = info["clientID"].as_str().unwrap_or_default();
    let v = match validate(&store.read(), client_id, redirect, scope, "code", challenge, s("codeChallengeMethod")) {
        Ok(v) => v,
        Err(m) => {
            let body = compact(json!({ "error": "invalid_request", "error_description": m, "state": state }));
            return Ok(json_with(StatusCode::BAD_REQUEST, &body));
        }
    };
    let client = ClientInfo {
        name: Some(v.client_name),
        version: info["version"].as_str().map(String::from),
        client_id: Some(v.client_id.clone()),
        client_uri: v.client_uri,
        user_agent: r.msg.header("user-agent").map(String::from),
        remote_address: None,
    };
    let (id, rx) = {
        let mut st = store.write();
        let (id, new, rx) = st.connect.request(client, v.scopes, redirect);
        if new {
            st.connect_tx.send((id.clone(), None)).ok();
        }
        (id, rx)
    };
    let answer = tokio::runtime::Handle::current().block_on(tokio::time::timeout(connect::APPROVAL, rx));
    let mut st = store.write();
    let body = match answer {
        Ok(Ok(Some(token))) => {
            let resource = Some(s("resource")).filter(|v| !v.is_empty()).map(String::from);
            json!({ "code": st.connect.store_code(token, v.client_id, challenge.into(), resource), "state": state })
        }
        Ok(Ok(None)) => json!({ "error": "access_denied", "state": state }),
        _ => {
            if st.connect.expire(&id) {
                st.connect_tx.send((id, Some(Outcome::Timeout))).ok();
            }
            json!({ "error": "server_error", "state": state })
        }
    };
    ok(&compact(body))
}

/// Desktop's `handleOAuth2TokenRequest`.
fn exchange(st: &mut BeeperStore, r: &Req) -> Reply {
    let given = |k| r.str(k).filter(|v| !v.is_empty());
    let fail = |error: &str, description: &str| {
        Ok(json_with(StatusCode::BAD_REQUEST, &json!({ "error": error, "error_description": description })))
    };
    if r.str("grant_type") != Some("authorization_code") {
        return fail("unsupported_grant_type", "Only authorization_code grant type is supported.");
    }
    let Some(code) = given("code") else { return fail("invalid_request", "Authorization code is required") };
    let Some(verifier) = given("code_verifier") else { return fail("invalid_request", "code_verifier is required") };
    let resource = r.str("resource").map(str::trim).filter(|v| !v.is_empty());
    match st.connect.exchange(code, verifier, r.str("client_id"), resource) {
        Ok((token, t)) => ok(&compact(json!({
            "access_token": token,
            "token_type": "Bearer",
            "expires_in": t.expires_at.map(|e| secs(e - connect::now_ms()).max(0)),
            "scope": t.scopes.join(" ")
        }))),
        Err((error, description)) => fail(error, description),
    }
}

fn introspect(st: &BeeperStore, r: &Req) -> Reply {
    let Some(token) = r.str("token").filter(|t| !t.is_empty()) else {
        let body = json!({ "error": "invalid_request", "error_description": "Token parameter is required" });
        return Ok(json_with(StatusCode::BAD_REQUEST, &body));
    };
    let Ok(t) = authorize(st, Some(token), &[]) else { return ok(&json!({ "active": false })) };
    let c = t.client_info.unwrap_or_default();
    ok(&compact(json!({
        "active": true,
        "scope": t.scopes.join(" "),
        "token_type": "Bearer",
        "exp": t.expires_at.map(secs),
        "iat": secs(t.created_at),
        "nbf": secs(t.created_at),
        "sub": t.id,
        "aud": c.client_id,
        "client_id": c.client_id,
        "iss": st.config.base_url(),
        "app": { "version": APP_VERSION, "name": APP_NAME, "bundle_id": APP_BUNDLE_ID },
        "client": compact(json!({
            "id": c.client_id,
            "name": c.name,
            "user_agent": c.user_agent,
            "remote_address": c.remote_address
        }))
    })))
}

/// Desktop's `registerOAuth2Client`, for dynamic client registration.
fn register(st: &mut BeeperStore, r: &Req) -> Reply {
    let b = &r.body;
    let fail = |m: &str| {
        Ok(json_with(StatusCode::BAD_REQUEST, &json!({ "error": "invalid_client_metadata", "error_description": m })))
    };
    let or = |k: &str, default: &'static str| b[k].as_str().filter(|v| !v.is_empty()).unwrap_or(default).to_string();
    let uris: Option<Vec<String>> = b["redirect_uris"]
        .as_array()
        .and_then(|a| a.iter().map(|u| u.as_str().filter(|u| redirect_ok(u)).map(String::from)).collect());
    let Some(uris) = uris.filter(|u| !u.is_empty()) else {
        return fail("redirect_uris must contain at least one loopback HTTP/HTTPS or private-use URI scheme redirect");
    };
    let method = or("token_endpoint_auth_method", "none");
    if method != "none" {
        return fail("Only public clients with token_endpoint_auth_method \"none\" are supported");
    }
    let has = |k: &str, v: &str| b[k].as_array().is_none_or(|a| a.iter().any(|x| x == v));
    if !has("grant_types", "authorization_code") {
        return fail("authorization_code grant type is required");
    }
    if !has("response_types", "code") {
        return fail("code response type is required");
    }
    let scope = or("scope", "read write");
    if scope.split(' ').any(|s| !s.is_empty() && !SCOPES.contains(&s)) {
        return fail("Only read and write scopes are supported");
    }
    let client = connect::Client {
        client_id: connect::random_id(),
        client_name: or("client_name", "MCP Client"),
        client_uri: b["client_uri"].as_str().map(String::from),
        redirect_uris: uris,
        scope,
        created_at: connect::now_ms(),
    };
    let base = st.config.base_url();
    let body = compact(json!({
        "client_id": client.client_id,
        "client_name": client.client_name,
        "client_uri": client.client_uri,
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "redirect_uris": client.redirect_uris,
        "scope": client.scope,
        "token_endpoint_auth_method": method,
        "client_id_issued_at": secs(client.created_at),
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token")
    }));
    st.connect.clients.insert(client.client_id.clone(), client);
    st.persist_connect();
    Ok(json_with(StatusCode::CREATED, &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_like_desktop() {
        assert!(
            redirect_ok("http://127.0.0.1:3000/cb") && redirect_ok("http://[::1]/cb") && redirect_ok("cursor://x/cb")
        );
        assert!(
            !redirect_ok("https://example.com/cb") && !redirect_ok("javascript:alert(1)") && !redirect_ok("web+x:y")
        );
        assert!(redirect_matches("http://127.0.0.1:1/cb", "http://127.0.0.1:2/cb"));
        assert!(!redirect_matches("http://127.0.0.1/cb", "http://127.0.0.1/other"));
        assert_eq!(fill("{{a}}-{{b}}", &[("a", "{{b}}"), ("b", "x")]), "{{b}}-x", "a value is not filled again");
        assert_eq!(js(Some("</script>")), "\"\\u003c/script>\"");
    }
}
