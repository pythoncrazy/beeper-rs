//! Desktop's routes: one table from which the server finds the route of a request, checks its token, and makes the
//! OpenAPI document of `/v1/spec`.

use super::{error, Response};
use crate::codec::url_decode;
use crate::http::StatusCode;
use beeper_core::connect::{READ, WRITE};
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

/// Who can call a route.
#[derive(Clone, Copy)]
pub(super) enum Auth {
    /// All clients.
    Open,
    /// Desktop's `requireAuth`: a token with these scopes. The server checks it before it reads the body.
    Token(&'static [&'static str]),
    /// The route checks the token itself.
    Own,
    /// Desktop's `requireAuth` of the Matrix proxy, which refuses with Matrix errors: `read` for GET, `write` for the
    /// other methods.
    Matrix,
}

use Auth::{Matrix, Open, Own};
const R: Auth = Auth::Token(&[READ]);
const W: Auth = Auth::Token(&[WRITE]);
const RW: Auth = Auth::Token(&[READ, WRITE]);

pub(super) struct Route {
    pub(super) method: &'static str,
    /// The path in the form of the OpenAPI document: `{name}` matches one segment, and a last `{*name}` matches all
    /// the segments that follow.
    pub(super) path: &'static str,
    /// The `operationId` of the route in the OpenAPI document, or `None` for a route that the document omits.
    pub(super) name: Option<&'static str>,
    pub(super) auth: Auth,
}

const fn op(method: &'static str, path: &'static str, name: &'static str, auth: Auth) -> Route {
    Route { method, path, name: Some(name), auth }
}

const fn other(method: &'static str, path: &'static str, auth: Auth) -> Route {
    Route { method, path, name: None, auth }
}

/// The routes in the order of Desktop's OpenAPI document, then the routes that the document omits.
pub(super) const ROUTES: &[Route] = &[
    op("GET", "/v1/accounts", "getAccounts", R),
    op("GET", "/v1/accounts/{accountID}", "getAccount", R),
    op("GET", "/v1/bridges", "listBridges", R),
    op("GET", "/v1/bridges/{bridgeID}", "getBridge", R),
    op("GET", "/v1/bridges/{bridgeID}/capabilities", "getBridgeCapabilities", R),
    op("GET", "/v1/bridges/{bridgeID}/login-flows", "listBridgeLoginFlows", R),
    op("GET", "/v1/bridges/{bridgeID}/logins", "listBridgeLogins", R),
    op("GET", "/v1/bridges/{bridgeID}/logins/{loginID}", "getBridgeLogin", R),
    op("POST", "/v1/bridges/{bridgeID}/logins/{loginID}/remove", "removeBridgeLogin", W),
    op("POST", "/v1/bridges/{bridgeID}/login-sessions", "createLoginSession", W),
    op("GET", "/v1/bridges/{bridgeID}/login-sessions/{loginSessionID}", "getLoginSession", R),
    op("POST", "/v1/bridges/{bridgeID}/login-sessions/{loginSessionID}/steps/{stepID}", "submitLoginStep", W),
    op("DELETE", "/v1/bridges/{bridgeID}/login-sessions/{loginSessionID}", "cancelLoginSession", W),
    op("GET", "/v1/messages/search", "searchMessages", R),
    op("GET", "/v1/chats/search", "searchChats", R),
    op("GET", "/v1/labels", "listLabels", R),
    op("GET", "/v1/accounts/{accountID}/contacts", "searchContacts", R),
    op("GET", "/v1/accounts/{accountID}/contacts/list", "listContacts", R),
    op("GET", "/v1/search", "search", R),
    op("POST", "/v1/focus", "focusApp", R),
    op("GET", "/v1/chats", "listChats", R),
    op("POST", "/v1/chats", "createChat", W),
    op("POST", "/v1/chats/start", "startChat", W),
    op("GET", "/v1/chats/{chatID}", "getChat", R),
    op("POST", "/v1/chats/{chatID}/archive", "archiveChat", W),
    op("PATCH", "/v1/chats/{chatID}", "patchChat", W),
    op("POST", "/v1/chats/{chatID}/reminders", "setChatReminder", W),
    op("DELETE", "/v1/chats/{chatID}/reminders", "clearChatReminder", W),
    op("POST", "/v1/chats/{chatID}/read", "markChatRead", W),
    op("POST", "/v1/chats/{chatID}/unread", "markChatUnread", W),
    op("POST", "/v1/chats/{chatID}/notify-anyway", "notifyAnyway", W),
    op("GET", "/v1/chats/{chatID}/messages", "listMessages", R),
    op("POST", "/v1/chats/{chatID}/messages", "sendMessage", W),
    op("GET", "/v1/chats/{chatID}/messages/{messageID}", "getMessage", R),
    op("PUT", "/v1/chats/{chatID}/messages/{messageID}", "editMessage", W),
    op("DELETE", "/v1/chats/{chatID}/messages/{messageID}", "deleteMessage", W),
    op("POST", "/v1/chats/{chatID}/messages/{messageID}/reactions", "addReaction", W),
    op("DELETE", "/v1/chats/{chatID}/messages/{messageID}/reactions/{reactionKey}", "removeReaction", W),
    op("POST", "/v1/assets/download", "downloadAsset", R),
    op("GET", "/v1/assets/serve", "serveAsset", Own),
    op("POST", "/v1/assets/upload", "uploadAsset", W),
    op("POST", "/v1/assets/upload/base64", "uploadAssetBase64", W),
    op("POST", "/v1/app/setup/start", "appSetupStart", Open),
    op("POST", "/v1/app/setup/email", "appSetupEmail", Open),
    op("POST", "/v1/app/setup/response", "appSetupResponse", Open),
    op("POST", "/v1/app/setup/register", "appSetupRegister", Open),
    op("GET", "/v1/app/setup", "appSetupRetrieve", Own),
    op("POST", "/v1/app/setup/verification/recovery-key", "appSetupVerificationRecoveryKey", RW),
    op("POST", "/v1/app/setup/verification/recovery-key/reset", "appSetupVerificationRecoveryKeyReset", RW),
    op(
        "POST",
        "/v1/app/setup/verification/recovery-key/reset/confirm",
        "appSetupVerificationRecoveryKeyResetConfirm",
        RW,
    ),
    op("GET", "/v1/app/setup/verifications", "appSetupVerificationsList", R),
    op("POST", "/v1/app/setup/verifications", "appSetupVerificationsCreate", RW),
    op("POST", "/v1/app/setup/verifications/qr/scan", "appSetupVerificationsQRScan", RW),
    op("GET", "/v1/app/setup/verifications/{verificationID}", "appSetupVerificationsGet", R),
    op("POST", "/v1/app/setup/verifications/{verificationID}/accept", "appSetupVerificationsAccept", RW),
    op("POST", "/v1/app/setup/verifications/{verificationID}/cancel", "appSetupVerificationsCancel", RW),
    op(
        "POST",
        "/v1/app/setup/verifications/{verificationID}/qr/confirm-scanned",
        "appSetupVerificationsQRConfirmScanned",
        RW,
    ),
    op("POST", "/v1/app/setup/verifications/{verificationID}/sas/start", "appSetupVerificationsSASStart", RW),
    op("POST", "/v1/app/setup/verifications/{verificationID}/sas/confirm", "appSetupVerificationsSASConfirm", RW),
    op("GET", "/v1/info", "getInfo", Open),
    op("GET", "/oauth/userinfo", "oauth_get_user_info", R),
    op("GET", "/oauth/authorize", "oauth_authorize", Open),
    op("POST", "/oauth/authorize/callback", "oauth_authorize_callback", Open),
    op("POST", "/oauth/introspect", "oauth_introspect_token", Open),
    op("POST", "/oauth/revoke", "oauth_revoke_token", Open),
    op("POST", "/oauth/token", "oauth_token", Open),
    op("POST", "/oauth/register", "oauth_register_client", Open),
    other("GET", "/", Open),
    other("GET", "/v1/spec", Open),
    other("GET", "/.well-known/oauth-protected-resource/{*suffix}", Open),
    other("GET", "/.well-known/oauth-authorization-server", Open),
    other("POST", "/register", Open),
    other("GET", "/v0/mcp", Own),
    other("POST", "/v0/mcp", Own),
    other("DELETE", "/v0/mcp", Own),
    other("OPTIONS", "/v0/mcp", Own),
    other("DELETE", "/v1/chats/{chatID}/messages/{messageID}/reactions", W),
    other("GET", "/focus", Open),
    other("GET", "/focus/{chatID}", Open),
    other("GET", "/focus/{chatID}/{messageID}", Open),
    other("GET", "/deeplink", Open),
    other("GET", "/deeplink/{*path}", Open),
    other("GET", "/_matrix/client/v3/user/{userID}/account_data/{eventType}", Matrix),
    other("PUT", "/_matrix/client/v3/user/{userID}/account_data/{eventType}", Matrix),
    other("GET", "/_matrix/client/v3/user/{userID}/rooms/{roomID}/account_data/{eventType}", Matrix),
    other("PUT", "/_matrix/client/v3/user/{userID}/rooms/{roomID}/account_data/{eventType}", Matrix),
    other("GET", "/_matrix/client/v3/rooms/{roomID}/account_data/{eventType}", Matrix),
    other("PUT", "/_matrix/client/v3/rooms/{roomID}/account_data/{eventType}", Matrix),
    other("GET", "/_matrix/client/v3/rooms/{roomID}/state/{*statePath}", Matrix),
    other("GET", "/_matrix/client/v3/rooms/{roomID}/event/{eventID}", Matrix),
    other("GET", "/_matrix/client/v3/profile/{userID}", Matrix),
    other("POST", "/_matrix/client/v3/join/{roomIDOrAlias}", Matrix),
    other("POST", "/_matrix/client/v3/rooms/{roomID}/leave", Matrix),
    other("POST", "/_matrix/client/v3/createRoom", Matrix),
];

/// The segments of a path, without the first slash and an optional last slash.
fn segments(path: &str) -> Vec<&str> {
    let path = path.strip_prefix('/').expect("an absolute path");
    match path.strip_suffix('/').unwrap_or(path) {
        "" => Vec::new(),
        p => p.split('/').collect(),
    }
}

/// Express's match of a request path: the literal segments match without case and without percent-decoding, and
/// the parameters match non-empty segments, which it decodes like `decodeURIComponent`. The result has the segments
/// of the path with the literal segments of the route and the decoded parameters.
fn matches(route: &str, path: &[&str]) -> Option<Result<Vec<String>, String>> {
    let route = segments(route);
    let wild = route.last().is_some_and(|s| s.starts_with("{*"));
    let fixed = route.len() - wild as usize;
    if path.len() < fixed || (!wild && path.len() != fixed) {
        return None;
    }
    let literal = |(r, p): (&&str, &&str)| r.starts_with('{') || r.eq_ignore_ascii_case(p);
    if !route[..fixed].iter().zip(path).all(|(r, p)| literal((r, p)) && !p.is_empty()) {
        return None;
    }
    let decode = |(i, p): (usize, &&str)| match route.get(i).filter(|r| !r.starts_with('{')) {
        Some(r) => Ok(r.to_string()),
        None => url_decode(&p.replace('+', "%2B")).ok_or_else(|| p.to_string()),
    };
    Some(path.iter().enumerate().map(decode).collect())
}

/// The route of a request and the segments of its path, or Express's 404 for no route and 500 for a parameter with
/// bad percent-encoding.
pub(super) fn find(method: &str, path: &str) -> Result<(&'static Route, Vec<String>), Response> {
    let path = segments(path);
    let found = ROUTES.iter().filter(|r| r.method == method).find_map(|r| Some((r, matches(r.path, &path)?)));
    let (route, segs) = found.ok_or_else(super::no_route)?;
    let segs = segs.map_err(|p| error(StatusCode(500), &format!("Failed to decode param '{p}'"), "UNKNOWN_ERROR"))?;
    Ok((route, segs))
}

/// The OpenAPI document of Desktop 4.3.308. The `paths` give the operations of the routes, and the Matrix paths of
/// Desktop's `includeMatrixCompat`, which name their parameters like the Matrix spec rather than like the routes.
static OPENAPI: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("../../assets/openapi.json")).expect("OpenAPI JSON"));

/// The text of the document of [`spec`], with `\u0000` in place of the base URL.
static SPEC: LazyLock<String> = LazyLock::new(|| {
    let mut doc = OPENAPI.clone();
    let mut paths = Map::new();
    for route in ROUTES {
        let Some(name) = route.name else { continue };
        let method = route.method.to_ascii_lowercase();
        let operation = &OPENAPI["paths"][route.path][&method];
        assert_eq!(operation["operationId"], name, "the OpenAPI JSON has the operation of {method} {}", route.path);
        paths.entry(route.path).or_insert_with(|| json!({}))[method] = operation.clone();
    }
    // Desktop's `ge` adds the Matrix paths after the generated document.
    let all = OPENAPI["paths"].as_object().expect("OpenAPI paths");
    paths.extend(all.iter().filter(|(p, _)| p.starts_with("/_matrix/")).map(|(p, ops)| (p.clone(), ops.clone())));
    doc["paths"] = paths.into();
    let base = "\0";
    doc["servers"][0]["url"] = base.into();
    let flow = &mut doc["components"]["securitySchemes"]["oauth2"]["flows"]["authorizationCode"];
    flow["authorizationUrl"] = format!("{base}/oauth/authorize").into();
    flow["tokenUrl"] = format!("{base}/oauth/token").into();
    String::from_utf8(serde_json::to_vec_pretty(&doc).expect("serialize the OpenAPI document")).expect("UTF-8 JSON")
});

/// Desktop's `createOpenAPISpec` for a base URL, as `JSON.stringify(spec, null, 2)` writes it: the operations of
/// the routes of [`ROUTES`] in their order.
pub(super) fn spec(base: &str) -> Vec<u8> {
    let base = serde_json::to_string(base).expect("serialize a string");
    SPEC.replace("\\u0000", &base[1..base.len() - 1]).into_bytes()
}
